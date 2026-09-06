# Milestone 4 — Real model provider and contextual conversation

Before this milestone the assistant answered `no_model_provider` to anything the
deterministic fast path did not claim, and forgot every turn the moment it
ended. This milestone makes it an assistant: a real Anthropic provider behind
the existing `ModelProvider` seam, and conversation history in Postgres.

What did **not** change: the orchestrator's shape, the permission seam, the
approval architecture, the wire protocol (`PROTOCOL_VERSION` is still 3), the
transport, and the platform. No new infrastructure was introduced.

---

## The provider

`assistant-models::anthropic`, behind a non-default `anthropic` cargo feature.

```
assistant-core            depends on assistant-models with NO features
      |                   -> cannot link reqwest, cannot name an Anthropic type
      v
ModelProvider  (trait, vendor-neutral)
      |
      v
AnthropicModelProvider    reqwest -> POST /v1/messages
      |- config.rs        model, output cap, timeout, effort, streaming
      |- wire.rs          request/response translation
      +- sse.rs           streaming decode
```

A small explicit `reqwest` client rather than an SDK: the surface used is one
endpoint, one streaming format and one error shape, `reqwest` is already the
project's HTTP dependency, and it is already compiled for Android by the Tauri
shell. See ADR-0018.

**Request headers.** `x-api-key`, `anthropic-version: 2023-06-01`,
`content-type`. Never `Authorization`.

**Translation, both directions, entirely inside the provider:**

| Repository type | Anthropic wire |
|---|---|
| `Message { role: System }` | folded into the top-level `system` field |
| `Message { role: User }` | `{"role":"user","content":[{"type":"text"}]}` |
| `Message::assistant_tool_calls` | assistant message with `text` + `tool_use` blocks |
| `Message::tool_result` | `tool_result` block inside a **user** message |
| `ToolSpec` | `{name, description, input_schema}` — and nothing else |
| `tool_use` block | `ToolCall` |
| content blocks | `GenerateResponse` / `StreamChunk` |

Two of those are load-bearing rather than cosmetic. The API has no `system` role
inside `messages`, so system instructions are hoisted; and it rejects
consecutive same-role messages, so a run of tool results coalesces into one user
message.

`risk`, `required_scopes` and `timeout_ms` deliberately do not cross into a tool
declaration. They are the server's business, and telling the model about them
would invite it to argue about them (ADR-0005). There is a test asserting they
do not appear in the serialised request.

**Model configuration** lives on `AnthropicConfig` and comes from the server's
`Config`. The model name appears there and nowhere else. `temperature` defaults
to `None` because current Claude models reject it outright; the field stays for
a model that accepts one. `effort` defaults to `low` — a conversational turn is
latency-critical and rarely needs deep reasoning, and turning depth down is
better than turning thinking off, which makes current models occasionally
narrate a tool call instead of emitting one.

**The mock provider is untouched.** `cargo test -p assistant-models` and
`cargo test --workspace` need no API key and make no network call.

---

## Streaming

One path, end to end. There is no second streaming architecture; the M2
abstractions carry it.

```
Anthropic SSE
  -> SseDecoder                    (provider; a pure state machine over bytes)
  -> StreamChunk::{Text,ToolCall,Done}
  -> Orchestrator::one_model_pass
  -> TurnEvent::AssistantDelta
  -> orchestration::to_frame
  -> ServerFrame::AssistantDelta
  -> WebSocket  /v1/conversation/{id}/stream
  -> Tauri shell (conversation.rs) -> emits "conversation://frame"
  -> React ConversationSheet       (appends to the message the delta names)
```

Nothing waits for the full response. `assistant-core` never sees an SSE event.

**Tool-call assembly.** A streamed tool call arrives as a `content_block_start`
naming it, then `input_json_delta` fragments that are only valid JSON once
concatenated, then a `content_block_stop`. The `ToolCall` is emitted at the
stop, never before — a half-parsed argument object must never reach the
permission engine.

**Truncation is not completion.** A stream that ends without `message_stop`
yields an error, not a `Done`. Presenting a cut-off answer as a finished one is
the failure mode this exists to prevent.

**Cancellation.** Dropping the chunk stream drops the HTTP body, which closes
the connection and stops the provider generating. The orchestrator's
`CancellationToken` already does that, and the WebSocket handler already cancels
when the client goes away — so a user closing the app stops the spend, with no
new mechanism.

---

## Conversation persistence

```
ContextProvider            (assistant-core trait — the read path)
  ^
StoredContextProvider      (assistant-core — applies the context window)
  |
ConversationStore          (assistant-core trait)
  ^
PostgresConversationStore  (assistant-server — owns sqlx)
  |
Postgres
```

The core defines what must be stored and what safety it needs; the server owns
the SQL. Same rule as `ActionStore` in M3.

The orchestrator holds the store for **writes** and a `StoredContextProvider`
over the same store for **reads**. Reading through a `ContextProvider` keeps the
core's existing seam; writing is the orchestrator's job because what to persist
depends on how the turn went, and a context provider is not told that.

### Turn lifecycle

1. authenticate (WebSocket bearer / `?access_token=`)
2. normalise input
3. `ensure` the conversation — creates it on first use, scoped to the principal
4. read history through the `ContextProvider` (bounded)
5. persist the **user** message
6. route: deterministic handler, or model
7. stream the answer
8. per tool round: persist the assistant turn that proposed the calls, then each
   tool result
9. persist the **assistant** message — before the turn is reported complete
10. emit `TurnEnd`

History is read at step 4 and the user message written at step 5, not the other
way round. The current turn already reaches the model from `input`; if it were
also in the history handed to the provider it would be sent twice. This is why
there is no duplicate-user-message problem to solve.

Step 9 happens before completion so that a turn a client saw finish is a turn
the next one can see. If the write fails, the turn fails — the client has the
text and an error, which is honest; silently dropping it would mean the next
turn cannot see an answer the user was shown.

**A failed turn writes no assistant message.** Not an empty one, not a partial
one. Deltas are streamed to the client and accumulated server-side; only a
completed provider stream produces a row.

### Schema — `migrations/0003_conversation_history.sql`

`conversations` already existed from `0001_init.sql` and is reused unchanged, as
is the conversation id the WebSocket already carries. There is no second
conversation identifier.

```
messages
  id               uuid  primary key
  conversation_id  uuid  references conversations (id) on delete cascade
  turn_id          uuid                       -- correlates one turn's messages
  seq              bigint generated always as identity   -- ordering
  role             text  check (user | assistant | tool)
  content          text
  tool_calls       jsonb default '[]'         -- structured, assistant only
  tool_call_id     text                       -- tool only
  created_at       timestamptz

  check: a tool message names the call it answers, and nothing else does
  check: only an assistant turn may carry tool calls

  index (conversation_id, seq desc)   -- the recency read
  index (turn_id, seq)                -- turn reconstruction
```

Ordering is `seq`, not `created_at`: two messages written in the same
millisecond still have an order, and a conversation replayed in the wrong order
is a different conversation.

The second CHECK is a security constraint, not tidiness. A user-supplied tool
call is exactly the shape an injection attempt would take, so the database
refuses it rather than relying on Rust to have remembered.

### Ownership

Enforced in SQL. `ensure` selects by `(id, user_id)` after an
`on conflict do nothing` insert, so an id owned by somebody else finds nothing
and is reported as not-found — never adopted. `append` selects its
`conversation_id` *from* `conversations` filtered by owner, so an unowned
conversation inserts zero rows. `history` joins `conversations` on `user_id`.

There is no code path that reads a row and checks the owner afterwards, because
that is the shape of check people forget to write.

### Context window

Two independent bounds, spent from the newest end: `ASSISTANT_CONTEXT_MAX_MESSAGES`
(40) and a character budget (24,000). Either alone is escapable. Tool results
from *earlier* turns are stored but not replayed — within the turn that produced
it a tool result is already in the message list, and replaying a stale one
invites the model to treat last week's inbox as current. See ADR-0022.

### This is not memory

"My name is Alex" is a fact in a conversation, not a durable fact about the
user. Nothing here scores importance, embeds, retrieves semantically or extracts
long-term facts. That is Milestone 8, with its own schema, retention policy and
correction surface. See ADR-0021.

---

## System prompt

`services/assistant-server/src/prompt.rs`, a constant. No `ClientFrame` has a
field that could reach it, and the provider folds it into the API's `system`
field rather than into `messages`. Two tests cover this: one asserts the server
always supplies its own prompt and that user text reading like an instruction
stays a user message; one asserts the prompt is short enough that somebody will
reread it before changing it.

It establishes only what stops the model claiming untrue things about the system
it runs in — be concise, be truthful about what did and did not happen, use the
tools you are given, do not decide your own permissions, do not invent personal
details, and say that Gmail/Calendar/files are not connected rather than
pretending to check. It is not the assistant's personality; writing one now
would mean rewriting it once the assistant does something.

---

## Errors

`ModelError` carries structured variants, each with a stable `code()` and a
vendor-neutral `user_message()`:

| Variant | Code |
|---|---|
| `AuthFailed` | `provider_auth_failed` |
| `RateLimited` | `provider_rate_limited` |
| `Timeout` | `provider_timeout` |
| `InvalidRequest` | `provider_invalid_request` |
| `Unavailable`, `Transport` | `provider_unavailable` |
| `MalformedResponse`, `Rejected` | `provider_error` |
| `Refused` | `provider_refused` |
| `UnsupportedCapability` | `provider_unsupported_capability` |

`CoreError::code()` delegates, so a client can tell "busy, try again" from "this
deployment is misconfigured" without the transport learning the provider's
taxonomy.

What the user sees comes from `user_message()`, which names no vendor and quotes
nothing the provider returned — a provider error body is untrusted text that
could contain account identifiers. The detail stays in the error source chain
for the log. The mobile app maps codes to shorter phrasing again in
`friendlyError`.

Errors are built from a status line and a parsed error body, never from the
request that was sent, so no code path can put the API key into one.

**Retries** are narrow: bounded transport retries only, never with tool results
attached, never mid-stream, and never for a rate limit. See ADR-0019.

---

## API key security

`ANTHROPIC_API_KEY` is read in `Config::from_env` and nowhere else. It reaches
`AnthropicConfig` as a private field and is used to set one header.

- Redacted in `Config`'s hand-written `Debug` and in `AnthropicConfig`'s.
- Sent as `x-api-key`, never `Authorization`.
- No tracing span in the provider records a header.
- Not in any protocol frame, database row, or error message.
- No `VITE_ANTHROPIC_API_KEY` exists, and must not: `VITE_*` variables are
  inlined into the shipped Android bundle.
- The phone talks to this server; the server talks to the provider. Never
  phone → Anthropic.

Only the credential's *presence* is logged, at startup.

See ADR-0020.

---

## Mobile path

```
Android APK (Tauri 2)
  React ConversationSheet          presentation only
    -> src/api/conversation.ts     invokes commands, parses frames once
      -> Tauri command (Rust)      src-tauri/src/conversation.rs
        -> WebSocket               ws://<server>/v1/conversation/{id}/stream
          -> Axum handler          transport glue
            -> Orchestrator
              -> AnthropicModelProvider
                -> Anthropic
```

The socket is opened by the shell, not the webview — the same reason as the HTTP
client (ADR-0008). Frames are forwarded verbatim as the JSON text they arrived
as, so `src/api/types.ts` stays the single mirror of the wire contract.

Connection errors are replaced by a short phrase rather than passed through: the
socket URL carries the access token in its query string, and a transport error
can quote the URL.

The UI is the minimum that makes the assistant usable. A dropped stream keeps
the text that arrived and marks it "Stopped early" rather than presenting it as
finished. A send is refused while a turn is in flight — a double tap would be
two model calls and two interleaved answers. Tool, approval and lifecycle frames
are ignored: showing them would make this a protocol inspector.

---

## Cost and latency

- One model request per conversational turn, plus one per tool round.
- `ASSISTANT_MAX_TOOL_ROUNDS` (4) is unchanged and unreadable by the model.
- The deterministic fast path still answers status questions with zero model
  invocations; two tests assert the call count is zero.
- One `reqwest::Client` for the process, so TLS and connection setup are not
  paid per turn.
- History is bounded at the SQL query.
- No background, proactive or autonomous model calls. No summarisation pass. No
  embeddings.

`one_model_pass` records, at debug level: provider, model, time to open the
stream, time to first token, total duration, and token usage. No message
content, no prompt, no tool argument values.

---

## Testing

| Suite | What it proves | Needs |
|---|---|---|
| `assistant-models` unit (5) | mock provider still deterministic | nothing |
| `assistant-models/tests/anthropic.rs` (21) | request construction, SSE decode, tool-call assembly, status→error mapping, retry rules, key redaction | nothing |
| `assistant-core` unit (42) | normalisation, permissions, context window selection | nothing |
| `assistant-core/tests/orchestrator.rs` (34) | tool loop, streaming, persistence ordering, failed turns, fast path | nothing |
| `assistant-server/tests/api.rs` (19) | the whole path over a real WebSocket | nothing |
| `assistant-server/tests/conversations.rs` (10) | ordering, ownership, constraints, restart survival | `DATABASE_URL` |
| `assistant-server/tests/durable_actions.rs` (24) | M3 approvals, unchanged | `DATABASE_URL` |
| `assistant-models/tests/live_anthropic.rs` (1) | one real streamed request | `ANTHROPIC_API_KEY`, `--ignored` |

The provider tests run against a local socket speaking canned HTTP, written one
SSE event per TCP write — so a decoder that only works when the whole body
arrives at once fails there. No API key, no network, no account.

Database tests skip rather than fail without `DATABASE_URL`, matching the
existing M3 philosophy. The live test is `#[ignore]`d so `cargo test
--workspace` compiles it and never runs it: CI must not depend on an external AI
API, and a suite that silently spends money is one people stop running.

### Security regressions covered

- API key absent from `Debug` output of both config types.
- No `Authorization` header on any provider request.
- Provider error detail (`sk-...`, org ids, quota text) never reaches the client
  frame.
- Client text cannot become a system prompt or a system message.
- Tool `risk`, `required_scopes` and `timeout_ms` never reach the model.
- A conversation owned by another principal is indistinguishable from a
  nonexistent one, for read and for write.
- The database refuses a user message carrying tool calls.
- The request logger still omits query strings.

---

## Known limitations

- **Authentication is still `DevTokenVerifier`** — a static shared bearer token
  with no expiry and no revocation (ADR-0009). It is not a security control. Do
  not expose a build using it beyond a trusted local network. Real
  authentication is a later milestone.
- **A turn that fails mid-stream leaves the question with no answer row.** There
  is no failure-state column on a message. That is the honest record; a richer
  one can be added when something reads it.
- **No conversation list.** The app generates a conversation id per sheet
  session. History persists under that id and survives a server restart, but the
  app does not yet offer a way to reopen an earlier conversation.
- **No tools are registered.** The tool path is fully wired and tested with
  deterministic doubles; no real integration exists to offer the model yet.
- **Reasoning effort is not exposed per turn.** One deployment-wide setting.

---

## Deferred, deliberately

| Not built | Milestone |
|---|---|
| Tasks, reminders, notes | M5 |
| Gmail, Calendar, Drive, Google OAuth | later |
| Long-term memory, importance scoring, embeddings, pgvector, semantic retrieval | M8 |
| Speech recognition, TTS, barge-in | M9 / M11 |
| Production authentication | later |
| Conversation summarisation / compaction | when the window is demonstrably too small |
| Model routing across providers | when a second provider exists |
