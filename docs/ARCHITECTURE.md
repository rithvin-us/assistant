# Architecture

This describes the shape of the system. As of Milestone 2 the orchestration spine
is implemented and tested; the integrations, providers, memory engine and voice
are still seams. Where something does not exist yet, the point is that there is an
obvious place for it to go — and that nothing has been faked in the meantime.

## Layers

```
┌───────────────────────────────────────────────────────────────┐
│ apps/mobile                React + TypeScript + Material UI   │
│                            presentation only                  │
├───────────────────────────────────────────────────────────────┤
│ apps/mobile/src-tauri      Tauri 2 shell (Rust)               │
│                            network, local SQLite, device      │
├───────────────────────────────────────────────────────────────┤
│                       ⇅  HTTP + WebSocket                     │
├───────────────────────────────────────────────────────────────┤
│ services/assistant-server  Axum: routing, auth, transport     │
├───────────────────────────────────────────────────────────────┤
│ crates/assistant-core      orchestration, events              │
│ crates/assistant-models    ModelProvider trait                │
│ crates/assistant-tools     ToolSpec, RiskLevel, permissions   │
│ crates/assistant-auth      Principal, TokenVerifier           │
│ crates/assistant-memory    memory domain + MemoryStore        │
│ crates/assistant-documents document domain + PDF pipeline     │
│ crates/assistant-protocol  wire types (leaf, no deps on above)│
├───────────────────────────────────────────────────────────────┤
│ PostgreSQL (Supabase)      source of truth                    │
│ SQLite (on device)         cache + offline capture            │
└───────────────────────────────────────────────────────────────┘
```

## Dependency rule

Dependencies point downward and inward. `assistant-protocol` is a leaf: it
depends on nothing in this repository, which is what makes it safe for both the
server and the Tauri shell to share.

`assistant-core` must never depend on a concrete integration or a concrete model
provider. Gmail, Calendar and Drive will be *implementations of tool traits* that
live in `assistant-tools`; providers are *implementations of `ModelProvider`*
that live in `assistant-models`. The core depends on the traits. This is what
allows a provider to be swapped for cost or latency reasons without touching
orchestration — see ADR-0003.

This is now load-bearing rather than aspirational: `assistant-models::anthropic`
exists, behind a non-default `anthropic` cargo feature. `assistant-core` depends
on `assistant-models` with no features, so it does not link `reqwest`, cannot
name `AnthropicModelProvider`, and cannot see an Anthropic type. The server
enables the feature; the core cannot. See ADR-0018.

## Why each crate exists

| Crate | Reason it is separate |
|---|---|
| `assistant-protocol` | Shared by the server and the Tauri shell. Must not pull server dependencies into the mobile binary. |
| `assistant-core` | Owns the orchestrator: normalisation, context, routing, the bounded tool loop, permission evaluation, streaming. Kept free of integrations and providers so that rule is compiler-enforced. |
| `assistant-models` | Provider implementations are feature-gated. The vendor-neutral vocabulary is always compiled; `anthropic` is not, so a build that does not use it does not link it. |
| `assistant-tools` | The risk/permission vocabulary must be usable by policy code that has no business depending on orchestration. |
| `assistant-auth` | Auth is consumed by the server middleware and, later, by the desktop agent's connection handshake. |
| `assistant-memory` | Memory has its own lifecycle and retention rules; keeping it separate stops conversation logging from quietly becoming memory. |
| `assistant-documents` | Documents have their own lifecycle (upload → extract → OCR → verify → index), object storage, and per-page provenance; the crate exposes traits for `DocumentStore`, `DocumentStorage`, `OcrProvider` and `DocumentVisionProvider` so the pipeline stays deterministic-first and swappable. See ADR-0036. |

Durable storage follows the same rule as model providers: `assistant-core`
declares the trait and the safety it needs; the `sqlx` implementation lives in
`assistant-server`, which already owns the pool. The core does not know Postgres
exists. Two traits follow this shape — `ActionStore` (approvals, executions,
audit; ADR-0014) and `ConversationStore` (conversation history; ADR-0021).

For conversations the trait also carries a *security* obligation, not only a
storage one: every method takes the authenticated principal, and an
implementation must scope its SQL by it rather than filtering after the read. A
conversation belonging to another principal must be indistinguishable from one
that does not exist.

## Request paths

**Deterministic read** — "what are my tasks today?"

```
webview → tauri command → HTTP GET → handler → Postgres → response
```

No model is involved. This path exists precisely so that common questions do not
pay for inference. See the latency notes below.

**Conversation** — text now, voice later

```
webview → tauri command → WS /v1/conversation/{id}/stream
        → ClientFrame::UserText
        → TurnRequest → Orchestrator
        ← TurnEvent stream, translated to frames:
          Ready / AssistantDelta* / ToolProposed / ApprovalRequired /
          ToolCompleted / TurnEnd, or Error
```

Assistant output is a stream of deltas terminated by `TurnEnd`. That shape was
chosen so adding streamed audio and barge-in later needs new enum variants, not a
new transport.

With a real provider, that stream runs the whole way through without buffering:

```
Anthropic SSE → SseDecoder → StreamChunk → TurnEvent::AssistantDelta
             → ServerFrame::AssistantDelta → WebSocket → Tauri shell → React
```

`assistant-core` never sees an SSE event, and the shell forwards frames verbatim
so `apps/mobile/src/api/types.ts` stays the single mirror of the wire contract.
Dropping the chunk stream drops the HTTP body, so cancelling a turn stops the
provider generating — the existing `CancellationToken` is the whole mechanism.

The Axum handler is transport glue only: it builds a `TurnRequest`, forwards
`TurnEvent`s, and writes frames. `assistant-core` has never heard of a WebSocket;
the translation lives in `services/assistant-server/src/orchestration.rs`. See
`docs/MILESTONE-2.md`.

## Orchestration

```
TurnRequest -> normalize -> context -> plan
                                        |
                    Deterministic ------+------ Model / ModelWithTools
                          |                            |
                    handler answers            stream deltas
                    (no model call)                    |
                                                 tool calls?
                                                       |
                                          registry -> ToolSpec -> policy
                                                       |
                                       Allow -> execute -> feed back -> model
```

Every dependency is injected through a trait; the orchestrator constructs no
providers, reads no environment variables and holds no global state. Execution
mode is chosen by code *before* any model is contacted — the model is never asked
whether it should have been used, because by then the round trip is already paid
for.

The tool loop is bounded by `max_tool_rounds` (server config, default 4). A turn
makes at most `max_tool_rounds + 1` model calls, and the limit is checked before
any tool runs.

Cancellation is a `CancellationToken` owned by the socket, with a child per turn.
That is the barge-in seam for voice.

## Permission flow

```
model proposes a tool call
   → ToolSpec looked up (name, RiskLevel, required_scopes)
   → deterministic policy evaluates → PermissionDecision
       Allow           → execute, audit
       RequireApproval → persist approval request → notify device
                       → Material UI approval sheet → user decides
                       → execute or discard, audit either way
       Deny            → refuse, audit
```

The model never supplies the risk level and never sees the decision function.
ADR-0005 explains why; the mechanism is that `PermissionPolicy::evaluate` takes a
`ToolSpec` (producible only by the registry) and a `Principal` (producible only by
the auth layer), so there is no parameter through which model output can reach a
decision.

Everything except approval persistence is implemented and tested as of
Milestone 2. An approval currently stops the turn and reports it; there is no
resume path yet — see ADR-0011.

## Latency

Voice latency is a product requirement, so the architecture commits to three
things up front:

- **A streaming transport from day one.** The WebSocket exists before any model
  does, so no later change has to retrofit streaming onto a request/response API.
- **A deterministic fast path.** Lookups that a SQL query can answer must not
  route through a model.
- **Connection reuse.** One `reqwest::Client` per process on both sides; one
  socket per conversation rather than one per turn.

## Observability

`tracing` throughout, with `TraceLayer` on the HTTP surface. Spans should make it
possible to answer: what happened, which model was called, which tool ran, how
long it took, whether the user approved it, whether it succeeded.

Never logged: OAuth tokens, API keys, passwords, message bodies. Two concrete
guards exist already — `Config`'s hand-written `Debug` redacts every secret, and
HTTP request logging omits query strings so a WebSocket `access_token` parameter
cannot reach the logs.

## Google Ecosystem & Personal Schedule (Milestone 5)

Google integration is implemented strictly behind provider-neutral interfaces (`GmailProvider`, `CalendarProvider` in `assistant-tools`) ensuring `assistant-core` never links Google SDKs.

- **Multi-Account Ownership:** An arbitrary number of Google accounts are supported, each isolated by `(id, user_id)` at the database query layer (ADR-0026).
- **Encrypted Credential Storage:** OAuth tokens are encrypted at rest using AES-256-GCM (ADR-0025) and refreshed server-side.
- **Risk Level Enforcement:** Read operations are Green; Calendar create/update are Yellow; Calendar deletion is Orange (ADR-0005, ADR-0009), requiring explicit human approval.
- **Deterministic Free-Time Engine:** Interval arithmetic computes available schedule slots without calling an LLM (ADR-0027).

## What is deliberately absent

No Redis, no Kafka, no Neo4j, no Elasticsearch, no Temporal, no Kubernetes. Each
would be justifiable in a system with more traffic or more operators; here each
would be cost and operational surface with no corresponding benefit. Postgres
covers relational data, job scheduling and — via pgvector — semantic retrieval.

Also absent, and not an oversight: a memory system. Conversation history is
durable (ADR-0021), and it is a log of what was said in one conversation.
Nothing promotes it to a durable fact about the user, scores importance, embeds
or retrieves semantically. That is a separate system with its own retention and
correction rules, and conflating the two is the mistake `assistant-memory`
exists as a separate crate to prevent.

## Academic Intelligence (Milestone 6)

Classroom and Drive follow the shape Gmail and Calendar established:
`ClassroomProvider`, `DriveProvider` and `AcademicProvider` are traits in
`assistant-tools`; the HTTP clients are `google/classroom.rs` and
`google/drive.rs`, the only files in the server that know Google's JSON.
`assistant-core` still links no Google type.

`academic.rs` is the single implementation of the academic capabilities. The
mobile screens reach it through `routes/academic.rs`, and the `classroom.*`,
`drive.*` and `academic.*` tools reach the same functions through the traits,
so the manual path and the model path cannot diverge. Nothing in it calls a
model: counting overdue assignments and importing coursework are arithmetic and
an upsert.

Coursework is imported into `tasks` rather than a parallel table, keyed by
`(user_id, external_provider, external_id)`. `source_title` and `source_due_at`
record what the provider last sent, which is what lets a sync distinguish a
provider change from a user edit. See ADR-0034.

Drive is read-only and is never mirrored into Postgres; file metadata is cached
on the device. Content reads check type and size before downloading. See
ADR-0033.

Full detail: `docs/MILESTONE-6.md`.

