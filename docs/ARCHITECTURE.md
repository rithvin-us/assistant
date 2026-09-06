# Architecture

This describes the shape of the system, not its current feature set. Most of what
is named here does not exist yet; the point of Milestone 0 is that when it does
exist, there is an obvious place for it to go.

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
live in `assistant-tools`; Claude and Gemini will be *implementations of
`ModelProvider`* that live in `assistant-models`. The core depends on the traits.
This is what allows a provider to be swapped for cost or latency reasons without
touching orchestration — see ADR-0003.

## Why each crate exists

| Crate | Reason it is separate |
|---|---|
| `assistant-protocol` | Shared by the server and the Tauri shell. Must not pull server dependencies into the mobile binary. |
| `assistant-core` | The one place allowed to know about orchestration. Kept free of integrations so that rule is compiler-enforced. |
| `assistant-models` | Provider implementations will be feature-gated. Separating them keeps unused vendor SDKs out of the build. |
| `assistant-tools` | The risk/permission vocabulary must be usable by policy code that has no business depending on orchestration. |
| `assistant-auth` | Auth is consumed by the server middleware and, later, by the desktop agent's connection handshake. |
| `assistant-memory` | Memory has its own lifecycle and retention rules; keeping it separate stops conversation logging from quietly becoming memory. |

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
        ← ServerFrame::Ready / AssistantDelta* / TurnEnd
```

Assistant output is a stream of deltas terminated by `TurnEnd`. That shape is
chosen so that adding streamed audio, tool-call frames and barge-in later needs
new enum variants, not a new transport.

## Permission flow (designed, not built)

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
ADR-0005 explains why.

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

## What is deliberately absent

No Redis, no Kafka, no Neo4j, no Elasticsearch, no Temporal, no Kubernetes. Each
would be justifiable in a system with more traffic or more operators; here each
would be cost and operational surface with no corresponding benefit. Postgres
covers relational data, job scheduling and — via pgvector — semantic retrieval.
