# Milestone 2 — Assistant Core

The execution spine. One user turn goes in; an answer, an approval request, or a
structured failure comes out. Every future capability — Gmail, Calendar, voice,
the desktop agent, Claude Code — plugs into this without changing it.

Baseline commit `05ec231`. What follows describes what exists and is tested, not
what is planned.

## The pipeline

```
TurnRequest
   │
   ├─ normalize ─────────────► InvalidInput if empty
   │
   ├─ ContextProvider::assemble
   │
   ├─ plan ──────────────────► Deterministic | Model | ModelWithTools
   │                                │
   │      ┌─────────────────────────┘
   │      │
   │  Deterministic ─► handler ─► answer            (no model invocation)
   │      │
   │  Model ─► stream ─► AssistantDelta*
   │              │
   │         tool calls?
   │              │
   │         registry ─► authoritative ToolSpec
   │              │
   │         PermissionPolicy
   │           ├─ Allow           ─► execute ─► result ─► back to model
   │           ├─ RequireApproval ─► stop, ApprovalRequired
   │           └─ Deny            ─► stop, PermissionDenied
   │              │
   │         bounded by max_tool_rounds
   │
   └─► TurnOutcome | CoreError
```

## Dependency injection

`Orchestrator::builder()` takes every dependency explicitly. There is no global
state, no `lazy_static`, no environment read anywhere in `assistant-core`.

| Seam | Trait | Milestone 2 implementation |
|---|---|---|
| Model | `assistant_models::ModelProvider` | none in production; `MockModelProvider` in tests |
| Tools | `assistant_tools::Tool` via `ToolRegistry` | none in production |
| Permissions | `PermissionPolicy` | `RiskBasedPolicy` |
| Context | `ContextProvider` | `EmptyContextProvider`, `InMemoryContextProvider` |
| Fast path | `DeterministicHandler` via `DeterministicRouter` | `AssistantStatusHandler` |
| Events | `EventBus` | in-process broadcast (ADR-0004) |

Defaults are inert. A caller opts into each capability, so nothing is silently
enabled — a build with no provider is a supported state, not a broken one.

## Execution modes

`ExecutionMode` is chosen by code, before any model is contacted:

- **`Deterministic`** — a registered handler matched. No model invocation at all.
- **`Model`** — the model answers; no tools are offered.
- **`ModelWithTools`** — the model answers and may propose tool calls.

The model is never asked whether it should have been used. By the time it could
answer that question, the round trip and the money are already spent.

## The deterministic fast path

`AssistantStatusHandler` answers status and health questions from real process
state: the injected provider's name and the actual contents of the tool
registry. No fake task or calendar store was created to give the fast path
something to do.

Matching is exact-substring on normalised lowercase text, and deliberately
narrow. A handler that guesses hijacks turns the model should have answered; a
missed match only costs one model call.

The invariant is asserted twice — once in the core, once across the whole stack
through the WebSocket — using the mock provider's call counter, which is the only
way to prove a model was *not* invoked.

## Permission boundary

```rust
fn evaluate(&self, spec: &ToolSpec, principal: &Principal) -> PermissionDecision
```

The signature is the security property. `ToolSpec` can only be produced by the
registry; `Principal` can only be produced by the auth layer. There is no
parameter through which model output can reach a decision, so no amount of prompt
injection changes what this returns.

Rules, in order: blocklist → missing scope (deny, not prompt — asking a user to
approve something their credentials cannot do is theatre) → risk at or above
`Orange` (require approval) → allow.

Three tests pin this: a tool declared `Green` and one declared `Red` under the
same *name* get different decisions; a call whose arguments assert
`{"risk":"green","requires_approval":false}` on a `Red` tool is still held; and
the tool's own call counter proves it never ran.

## Approval boundary

`RequireApproval` is modelled as a `CoreError` variant, not a boolean. No code
path can continue past it by forgetting to check a flag.

The whole batch is authorised before any of it runs. If one call in a batch needs
approval, nothing in that batch has already taken effect by the time the user is
asked.

There is no resume path yet. The turn stops, the client is told, and the user can
ask again — ADR-0011 explains why a persisted `ApprovalRequest` would currently
be a row nothing reads.

## Tool loop

Bounded by `OrchestratorConfig::max_tool_rounds`, which the server reads from
`ASSISTANT_MAX_TOOL_ROUNDS` (default 4). A turn makes at most
`max_tool_rounds + 1` model calls. The limit is checked *before* any tool runs,
so exceeding it costs nothing beyond the model call that proposed the calls. The
model cannot read or raise it.

Concurrency is conservative — parallel only for batches where every call is
`Green` and no tool repeats. See ADR-0013.

A failing tool is a structured `ToolResult` fed back to the model, not a failed
turn. Tools fail routinely and models usually recover from being told so.

## Streaming

`assistant-core` emits `TurnEvent`s and knows nothing about WebSockets.
`services/assistant-server/src/orchestration.rs` translates them into
`ServerFrame`s. The Axum handler is glue: read frame, build `TurnRequest`, forward
events, write frames.

| Core event | Wire frame |
|---|---|
| `Started`, `AssistantStarted`, `ToolStarted` | *(not forwarded)* |
| `AssistantDelta` | `AssistantDelta` |
| `ToolProposed` | `ToolProposed` |
| `ApprovalRequired` | `ApprovalRequired` |
| `ToolCompleted` | `ToolCompleted` |
| `Completed` | `TurnEnd` |
| `Failed` | `Error` (carrying the stable code) |

Protocol is now v2 — see ADR-0012.

## Cancellation

`tokio_util::sync::CancellationToken`. The socket owns a token; each turn gets a
child. Cancelling stops the model stream and any running tool at the next await
point, and the turn ends with `Cancelled`.

This is the barge-in seam. A future voice client interrupting playback cancels
the turn instead of waiting for it to finish. Nothing more elaborate was built:
a clean boundary is what was needed, not a framework.

## Error model

`CoreError` carries `code()`, a stable discriminant the transport maps to the
wire without matching the enum in three places.

`InvalidInput` · `ContextError` · `ModelError` · `NoModelProvider` ·
`UnknownTool` · `ToolValidationError` · `PermissionDenied` · `ApprovalRequired` ·
`ToolExecutionError` · `Cancelled` · `IterationLimitExceeded` · `Internal`

`Display` is user-safe. Provider internals stay in the `#[source]` chain for
logs. A test asserts that a provider error mentioning an API key does not leak it
into the message, and that the detail is still reachable for logging.

`is_expected_stop()` distinguishes turns that stopped deliberately (approval,
cancellation) from turns that broke.

## Observability

Every turn runs inside a `turn` span carrying `turn_id`, `conversation_id`,
`user_id`, `mode`, `handler` and `rounds`. Tool execution adds `tool`, `call_id`
and the authoritative `risk`, plus the permission decision.

Never recorded: user message text (only its character count), tool arguments,
tool output, tokens, keys. `Config`'s hand-written `Debug` redacts secrets, and
HTTP request logging omits query strings because the WebSocket accepts
`?access_token=`.

## Testing strategy

Deterministic doubles, no network, no randomness, no sleeping on wall-clock time
except where a timeout is the thing under test.

- `MockModelProvider` (`assistant-models`, `mock` feature) — scripted responses:
  text, streamed text, one tool call, several, a provider error, a malformed
  call. Counts invocations and records every request, which is what makes
  "the model was not called" and "the tool result went back to the model"
  assertable.
- `assistant-core`'s `testing` feature — `EchoTool` (counts invocations, so a
  denied tool can be proven not to have run), `FailingTool`, `SlowTool`,
  `TracingTool` + `CallLog` (logs enter/exit, so concurrency is observable).

Both features are off by default; neither reaches a release binary.

**78 tests pass, 0 fail** (baseline was 9).

## Explicitly not in this milestone

No Anthropic or Gemini provider. No Gmail, Calendar, Drive, Classroom or Google
OAuth. No memory engine, embeddings, pgvector or watchdogs. No voice. No browser
automation, desktop agent or Claude Code. No approval persistence or resume. No
JSON Schema validation of tool arguments beyond "must be an object". No mobile UI
changes.

The seams for all of them exist. None of them is faked.
