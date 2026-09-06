# Milestone 3 — Durable Actions, Approval & Audit

Milestone 2 stopped a turn at an approval and lost the action. This milestone
makes the action durable, so a user can answer minutes later, from a phone, after
the server has restarted — and the answer runs the action that was actually
proposed.

Baseline commit `56f5b54`. Everything below is implemented and tested against a
real PostgreSQL.

## The lifecycle

```
model proposes ToolCall
   │
   ├─ registry → authoritative ToolSpec        (risk, scopes, timeout)
   ├─ policy   → PermissionDecision
   │
   ├─ Allow           → execute now
   ├─ Deny            → stop, audit
   └─ RequireApproval
          │
          ├─ persist ToolExecution   (AwaitingApproval, with validated arguments)
          ├─ persist ApprovalRequest (Requested, expires_at)
          └─ emit ApprovalRequired{approval_id, risk, summary}
                 │
        …server may restart here…
                 │
          user answers: {"type":"approve_action","approval_id":"…"}
                 │
          ├─ claim_approval  (SELECT … FOR UPDATE — exactly one winner)
          ├─ re-resolve ToolSpec from the live registry
          ├─ re-evaluate policy
          ├─ ToolExecutor::run_authorized
          └─ complete_execution + audit event, one transaction
```

## Schema

`migrations/0002_durable_actions.sql`. Three tables, no workflow engine.

| Table | Holds | Notable constraint |
|---|---|---|
| `tool_executions` | the action and how far it got, including the validated `arguments` replayed on resume | `status` CHECK over the six execution states |
| `approval_requests` | the question put to the user | `unique (execution_id)` — one approval can never cover several actions; a CHECK that an answered approval names when and by whom |
| `audit_events` | append-only record of consequential actions | no FK to `tool_executions`, so deleting an execution cannot cascade away its audit row |

## State machines

Encoded once in `ExecutionStatus::can_transition_to` and
`ApprovalTransitions::can_transition_to`, mirrored by SQL `CHECK` constraints.

**Execution:**

```
Proposed ──► AwaitingApproval ──► Running ──► Succeeded
   │                │                │    └──► Failed
   │                │                └───────► Cancelled
   ├────────────────┴──► Cancelled
   └──► Running
```

`Running` is reachable **only** from `Proposed` or `AwaitingApproval`, so
`Cancelled → Running` — a rejected action executing anyway — is not
representable. Terminal states permit no transition at all.

**Approval:** `Requested` → `Approved` | `Rejected` | `Expired` | `Cancelled`.
Every state except `Requested` is final, so an approval is answered exactly once.

## Idempotency

A double-tapped Approve button is a duplicate request, not a fault.
`claim_approval` takes a row lock inside a transaction; a second caller blocks,
then sees the resolved status and receives `AlreadyResolved` — with no means to
execute. There is no read-then-write pair for a caller to get wrong, because
`claim_approval` is the only way to answer an approval.

Verified with eight simultaneous claims of one id: exactly one `Claimed`, seven
`AlreadyResolved`.

## Transactions

Two matter, and both exist to prevent a contradictory half-state:

- **`record_proposal`** — execution and approval together. An approval
  referencing a missing execution would be unanswerable.
- **`complete_execution`** — outcome and audit event together. If the audit
  insert fails, the outcome rolls back too: retrying is recoverable, a silently
  unaudited consequential action is not.

`claim_approval` is itself one transaction, so "approved but not runnable" is not
a state the database can hold.

## Expiration

`ApprovalPolicy::ttl`, default 15 minutes, passed in rather than scattered as a
literal. Enforced on the **write** path inside `claim_approval`, so an approval
cannot become executable again because a cleanup job failed to run. An expired
approval marks itself `Expired`, cancels its execution, writes an audit event,
and does not execute.

## Security boundaries

The client sends an approval **id and nothing else**. It cannot name a tool,
supply arguments, assert a risk level, or claim to be another user — the server
loads the persisted action and that record is authoritative (ADR-0015).

- **Ownership** is a `WHERE` clause, not a check afterwards. A mismatch reports
  "not found", indistinguishable from a non-existent id, so a caller cannot probe
  for other users' approvals.
- **Approval is not a freeze on the world.** The `ToolSpec` is re-resolved from
  the live registry and policy re-evaluated before execution. A tool since
  unregistered, blocklisted, or whose scope the user has lost, does not run.
- **One execution path.** `ToolExecutor::run_authorized` is the only function
  that calls `Tool::execute`. Both the in-turn and resumed paths converge on it.
- **Audit stores no argument values** — only the tool and its argument *keys*
  (ADR-0016).

## Restart behaviour

The restart tests build a store on its own pool, write the approval, **close that
pool**, then build an independent store and read the approval back, answer it,
and execute it. Nothing survives in-process; only the row in Postgres does.

## Observability

`turn_id`, `approval_id`, `execution_id`, `tool_name`, authoritative `risk` and
outcome appear as structured span fields. Never logged: tokens, keys, passwords,
argument values, message bodies.

The Milestone 2 leak — the WebSocket `?access_token=` reaching `tower_http`'s
request log through the full URI — remains fixed, with its regression test
retained in `tests/api.rs`.

## Configuration

No new environment variables. `DATABASE_URL` (server-only, already documented)
now also gates durable approvals: without it the server runs, the turn still
stops at an approval, and the client is told `approval_id: null` rather than
handed an id that would fail.

Nothing was added to `apps/mobile/.env`; `VITE_*` values ship to the device and
no part of this milestone belongs there.

## Testing

`services/assistant-server/tests/durable_actions.rs` — 17 tests against a real
PostgreSQL. They **skip, not fail**, when `DATABASE_URL` is absent, so
`cargo test --workspace` stays runnable without credentials.

They run against the real store on purpose: the properties under test — one
winner under concurrent claims, an expired approval refusing to execute — are
properties of the database, and an in-memory fake would pass while proving
nothing.

Two harness details worth knowing, both discovered by running them:

- A `sqlx` pool is bound to the runtime that created it, so a pool in a `static`
  dies when the first `#[tokio::test]`'s runtime ends. Each test opens its own.
- A hosted Supabase session pooler admits a fixed number of clients (15 for the
  project used here) and refuses the rest with `EMAXCONNSESSION`. A semaphore
  caps concurrent database-holding tests so the suite fails on assertions rather
  than on connection admission.

## Not in this milestone

No Gmail, Calendar, Drive, Classroom or Google OAuth. No real model provider. No
memory engine, embeddings or pgvector. No voice, browser automation, desktop
agent or Claude Code. No notification delivery — the approval reaches the client
over the existing socket, and push is a later milestone. No retention cleanup
(ADR-0017). No JSON Schema validation of arguments beyond "must be an object".
No mobile approval UI: the protocol carries everything it needs, and the screen
is deliberately unchanged.
