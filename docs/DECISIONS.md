# Architecture Decision Records

Every decision that touches security, database design, authentication, model
providers, mobile architecture, cloud cost, permissions or data retention is
recorded here before it is implemented. Records are append-only; a reversal is a
new record that supersedes an old one.

---

## ADR-0001 — Modular monolith in one Cargo workspace

**Context.** The system will eventually span assistant orchestration, model
providers, tools, permissions, memory, scheduling, notifications, documents,
integrations and audit. That is a lot of surface for one person to operate.

**Options.**
1. Microservices, one per bounded context.
2. A single crate with modules.
3. A modular monolith: one deployable binary, many crates with an enforced
   dependency direction.

**Chosen.** Option 3.

**Reason.** Boundaries are what matter, and Cargo enforces them for free: a crate
cannot use what it does not depend on. Microservices would add network hops,
deployment surface and cost for a single-user product. One flat crate would let
`assistant-core` reach into a Gmail client six months from now with nothing to
stop it.

**Consequences.** One process to deploy, one place to read a stack trace.
Splitting a crate out into its own service later is mechanical, because the
dependency graph is already acyclic. Cross-crate refactors cost more than moving
code between modules would.

---

## ADR-0002 — `apps/mobile/src-tauri` is a workspace member

**Context.** The Tauri shell is Rust and wants to share `assistant-protocol` with
the server. It could equally be its own separate workspace.

**Options.**
1. Exclude it; separate lockfile and `target/`.
2. Include it as a workspace member.

**Chosen.** Option 2.

**Reason.** One lockfile means the shell and the server cannot drift onto
different versions of a shared dependency, and `cargo clippy --workspace` covers
the shell. Cargo only honours `[profile.*]` at the workspace root, so the release
tuning Tauri generates was moved into the root `Cargo.toml`.

**Consequences.** `cargo build --workspace` compiles the Tauri shell too, so CI
on Linux needs the system webview development packages. If Android
cross-compilation ever conflicts with the shared `target/`, splitting the shell
back out is the fallback.

---

## ADR-0003 — The core depends on model interfaces, never on providers

**Context.** Claude, Gemini and later local models each have different strengths.
Any of them could be replaced, repriced or deprecated.

**Options.**
1. Call a provider SDK directly from orchestration code.
2. Define a `ModelProvider` trait; implement it per provider behind features.

**Chosen.** Option 2. `assistant-models` contains the trait and no provider.

**Reason.** Provider choice is a routing decision that should follow the
workload — a cheap model for classification, a strong model for planning, a
realtime model for voice. Hard-coding a vendor at call sites makes that
impossible to change without touching every call site, and turns cost control
into a rewrite.

**Consequences.** A provider-specific feature must either be expressible as a
`Capability` or be unavailable. That is the intended trade: a feature no
abstraction can express is a feature that locks the product to one vendor.

---

## ADR-0004 — In-process event bus, not a broker

**Context.** The system is event-driven (`EmailReceived`, `DeadlineDetected`,
`ApprovalRequested`, …).

**Options.**
1. A message broker (Redis, NATS, Kafka).
2. A Tokio broadcast channel in-process.
3. A Postgres table polled as a queue.

**Chosen.** Option 2 now, with option 3 as the durability layer once durability
is actually required.

**Reason.** Every producer and consumer currently lives in one process. A broker
would add infrastructure, cost and an operational failure mode to solve a problem
that does not exist yet.

**Consequences.** Events are lost on restart, so nothing that must survive a
restart may rely on the bus alone. When durable events are needed, a subscriber
writes them to Postgres and the bus interface does not change.

---

## ADR-0005 — Risk level is a static property of a tool declaration

**Context.** Tools will range from reading a calendar to sending mail from the
user's account. Something has to decide what needs approval.

**Options.**
1. The model states how risky its own call is.
2. Risk is fixed in the tool's declaration; policy code maps it to a decision.

**Chosen.** Option 2. `RiskLevel` lives in `ToolSpec` and is set in Rust.

**Reason.** Model output is untrusted input. A model that could label its own call
`Green` could be argued into sending email without approval — whether by a prompt
injection in a document it read, or by ordinary error. Deterministic policy over a
fixed declaration cannot be talked out of anything.

**Consequences.** Adding a tool means making an explicit risk judgement in code
and in review. Risk cannot vary with arguments; a tool whose danger depends on its
input must be split into separate tools at separate levels.

---

## ADR-0006 — Plain SQL migrations, applied explicitly

**Context.** Postgres via Supabase is the source of truth. Schema has to evolve.

**Options.**
1. `supabase/migrations` and the Supabase CLI.
2. Plain `.sql` files under `migrations/`, applied with `sqlx migrate`.
3. Both.

**Chosen.** Option 2.

**Reason.** Plain SQL applied by sqlx-cli works against Supabase and against any
other Postgres, which keeps a later move off Supabase a hosting change rather than
a rewrite. It also avoids making Docker and the Supabase CLI hard prerequisites
for building the project.

**Consequences.** Supabase-specific features (row-level security policies, edge
functions) must be written as SQL in the same migration files rather than managed
by Supabase tooling. Migrations are never applied automatically at startup: a
process that alters schema on boot can corrupt data during a rolling restart, so
`scripts/migrate.ps1` is a deliberate, separate act.

---

## ADR-0007 — Local SQLite is a cache, not a replica

**Context.** The app must stay useful offline: quick capture, today's tasks, local
reminders.

**Options.**
1. Mirror the cloud schema locally and sync bidirectionally.
2. Keep a small local store holding only pending captures and cached reads.

**Chosen.** Option 2.

**Reason.** Bidirectional sync of a full schema means conflict resolution, which is
a large and subtle body of work that would dominate the project and produce
user-visible data anomalies whenever it is wrong. Almost all real offline value
comes from queuing writes and caching a handful of reads.

**Consequences.** Offline reads are limited to what was cached. The local database
is disposable — the recovery for a corrupt or outdated local schema is to delete
the file and re-sync, which is why it is created inline rather than migrated.

---

## ADR-0008 — Network calls go through the Tauri shell, not the webview

**Context.** The frontend needs to reach the server from desktop and from Android.

**Options.**
1. `fetch` from the webview.
2. Rust `#[tauri::command]` functions that the webview invokes.

**Chosen.** Option 2.

**Reason.** One request path on both platforms, no CORS configuration, no Android
cleartext-HTTP exemption, and — most importantly — credentials and retry/timeout
policy live in Rust rather than in JavaScript shipped to the device. It also keeps
business logic out of React, which is the stated goal for the mobile layer.

**Consequences.** Every new server call needs a Rust command as well as a
TypeScript wrapper. Streaming will need Tauri events rather than a returned value:
the conversation WebSocket terminates in Rust and forwards frames to the webview.

**Amendment (Milestone 0).** `bridge.ts` also carries a `fetch` fallback used
only when the page is not running inside a Tauri webview, so `pnpm dev` in a
plain browser is usable for styling work. It is a development convenience, not a
second supported path: it needs the server's CORS origin list to include
whatever port Vite is serving, and it has no access to the local SQLite cache.
Shipped builds always take the Tauri command path. Components still must not call
`fetch` directly -- the fallback lives behind the same `bridge.ts` functions.

---

## ADR-0009 — Development authentication is a static bearer token

**Context.** Milestone 0 needs an auth boundary to exist without building real
authentication.

**Options.**
1. No authentication at all.
2. A shared static bearer token behind a `TokenVerifier` trait.
3. Supabase JWT verification now.

**Chosen.** Option 2.

**Reason.** The extraction point has to be real code, or every handler written
before real auth arrives will need changing afterwards. A static token buys that
for a few lines. Real authentication is a later milestone with its own design.

**Consequences.** `DevTokenVerifier` has no expiry, no revocation and no per-user
identity. It is not a security control, and a build using it must not be exposed
beyond a trusted local network. Replacing it means implementing `TokenVerifier`
once; no handler changes.

---

## ADR-0010 — The release workflow is kept, scoped down, and marked premature

**Context.** `.github/workflows/release.yml` arrived during Milestone 1. It runs
on `v*` tag pushes and `workflow_dispatch`, builds the server binary and Tauri
bundles on three platforms, and creates a draft GitHub release via
`tauri-apps/tauri-action`. The project has no tags and has never cut a release.

**Options.**
1. Delete it as scope that is not needed yet.
2. Keep it as-is.
3. Keep it, but reduce its privileges to the minimum it actually needs.

**Chosen.** Option 3.

**Reason.** It is genuinely premature -- nothing releases yet -- but deleting
working infrastructure to satisfy a scope rule is a worse trade than leaving it
dormant, and it costs nothing while no tag is pushed. What did need fixing was
privilege: the workflow declared `contents: write` at the top level, so every
job inherited a write-scoped `GITHUB_TOKEN` while running repository-controlled
build code (`cargo build`, `pnpm install` lifecycle scripts, the Tauri build).
Only the job that publishes a release needs that. `pnpm install` was also
unpinned, so a release artefact would not have been reproducible from the
committed lockfile.

**Consequences.** Top-level permission is now `contents: read`; `contents: write`
is re-granted only on the `build-tauri` job that creates the release. The
frontend install uses `--frozen-lockfile`.

Residual, accepted for now: the workflow still executes repository code, which is
inherent to building anything. It is not reachable from a fork pull request --
there is no `pull_request_target` trigger, and only accounts with write access
can push a tag or dispatch a workflow. It also drifts from `ci.yml` (Node 20 vs
24, `checkout@v4` vs `v5`, `pnpm/action-setup@v3` vs `v4`); that is cosmetic
while no release is being cut and should be reconciled before the first real tag.

Release infrastructure was deliberately kept out of the Assistant Core work: this
change is a separate commit.

---

## ADR-0011 — Milestone 2 orchestration keeps no persistent state

> Superseded in part. The approval and audit half was answered by ADR-0014; the
> conversation half by ADR-0021, once a real model provider made an assistant
> that forgets the previous sentence untenable. The reasoning below still stands
> for why neither was built earlier.

**Context.** The orchestrator now runs turns, evaluates permissions and stops for
approval. Three things could plausibly need a database: conversation identity,
approval requests, and an audit trail of tool executions.

**Options.**
1. Add `conversations`, `approvals` and `tool_executions` tables now.
2. Keep the implemented behaviour in memory and add tables when a feature
   actually requires them to survive a restart.

**Chosen.** Option 2.

**Reason.** Nothing implemented in this milestone needs to outlive the process.
A conversation id is supplied by the client and used only to scope in-memory
history. An approval stops the turn and reports that to the caller; there is no
resume path yet, so a persisted `ApprovalRequest` would be a row nothing reads.
Tool executions are traced, and the durable audit trail should be designed
alongside the retention policy rather than accreted from whatever the first
feature happened to need. Writing tables ahead of the code that uses them
produces schema that is wrong in ways nobody discovers until migration time.

**Consequences.** Conversation history is lost on restart, and `assistant-core`
carries an explicit `InMemoryContextProvider` that says so in its own
documentation. An approval cannot currently be granted and resumed -- the turn
stops and the user must ask again. Both are Milestone 3 work, and both arrive
with a migration at that point.

The `migrations/` directory is unchanged by this milestone.

---

## ADR-0012 — Wire protocol v2 carries tool and approval frames

**Context.** With a real tool loop, a client can now observe things the v1
protocol had no way to express: that the assistant asked for a tool, that a tool
finished, and -- most importantly -- that the turn stopped because something
needs human approval.

**Options.**
1. Leave the protocol at v1 and express approval as a generic `Error` frame.
2. Add explicit `ToolProposed`, `ApprovalRequired` and `ToolCompleted` frames and
   bump `PROTOCOL_VERSION` to 2.

**Chosen.** Option 2.

**Reason.** Approval is the one place where the product deliberately stops and
asks the user a question. Encoding that as an error would force the client to
pattern-match on an error string to decide whether to render an approval sheet,
which is exactly the kind of implicit contract that breaks silently. The frames
also carry the tool's authoritative `RiskLevel`, so the UI can size the prompt to
the actual risk instead of treating every approval identically.

**Consequences.** `PROTOCOL_VERSION` is 2 in both
`crates/assistant-protocol/src/lib.rs` and `apps/mobile/src/api/types.ts`; they
must continue to change together. A v1 client talking to a v2 server sees the
mismatch on `/v1/health` and reports it rather than misparsing.

The wire `RiskLevel` is for display only. Every permission decision is made
server-side against the tool registry, and a client cannot influence it -- the
same rule as ADR-0005, restated at the transport boundary.

---

## ADR-0013 — Parallel tool execution only for distinct read-only calls

**Context.** A model can return several tool calls at once. Running them
concurrently is a real latency win for a voice product, and a real correctness
hazard.

**Options.**
1. Always sequential.
2. Always concurrent.
3. Concurrent only when the batch is provably safe.

**Chosen.** Option 3, with a deliberately narrow definition of "provably safe":
every call in the batch is classified `Green` (read-only, no observable side
effect) *and* no tool appears more than once. Anything else runs sequentially.

**Reason.** Two writes in one batch may depend on each other, and the model's
ordering is the only signal about that -- a signal that is untrusted and often
wrong. Two calls to the same tool may contend on the same resource even when the
tool is nominally read-only. `Green` is already defined as having no observable
side effect, so a batch of distinct `Green` calls cannot interfere. Everything
outside that is ordered.

**Consequences.** Batches containing a single write are as slow as sequential
execution, which is the intended trade. If a future tool is `Green` but
internally stateful, that tool is misclassified and the classification is the bug
to fix, not this rule. The behaviour is pinned by two tests -- one asserting
read-only calls overlap, one asserting writes do not.

---

## ADR-0014 — Approvals, executions and audit are durable in Postgres

**Context.** Milestone 2 stopped a turn at `RequireApproval` and lost the action.
Nothing survived a restart, so an approval could never actually be answered.

**Options.**
1. Keep approvals in process memory.
2. Persist approvals, executions and audit in Postgres.
3. Introduce a workflow engine (Temporal) or a queue (Redis, RabbitMQ).

**Chosen.** Option 2. Three tables, plain SQL, applied by sqlx-cli.

**Reason.** An approval is a question put to a human, and humans answer minutes
later, from a phone, after the server has been redeployed. In-memory state loses
the action exactly when the product most needs it. A workflow engine would solve
a distribution problem this system does not have while adding infrastructure,
cost and an operational failure mode.

Postgres also supplies the two properties this feature actually needs and that
application code cannot provide safely on its own: row-level locking, which makes
a double-tapped Approve button run one action rather than two, and transactions,
which stop a half-answered approval existing.

**Consequences.** Approvals only work where a database is configured. Without
`DATABASE_URL` the turn still stops, but the client is sent `approval_id: null`
and told the action was not persisted, rather than being handed an id that would
fail on use. `/v1/health` already reports that deployment as degraded.

The state machines are explicit (`ExecutionStatus`, `ApprovalStatus`) with a
transition table in code and `CHECK` constraints in SQL, so `Cancelled → Running`
— a rejected action executing anyway — is unrepresentable in both.

---

## ADR-0015 — An approval authorises one persisted action, and is re-validated

**Context.** Once an action is durable, a client has to be able to say "yes" to
it. What the client is allowed to say determines the whole security posture.

**Options.**
1. The client sends the tool name and arguments to approve.
2. The client sends an approval id; the server loads the persisted action.
3. The client approves a tool generally ("always allow `gmail.send`").

**Chosen.** Option 2.

**Reason.** Option 1 makes the client an input to authorisation — it could
approve one action and execute another, and the server would have no way to tell.
Option 3 converts a single decision into a standing grant, which is precisely the
authority a prompt injection would try to obtain. Under option 2 the only thing a
client contributes is a uuid; everything about what that uuid means comes from
the database.

Ownership is enforced inside the query, and a mismatch reports "not found" rather
than "forbidden", so a caller cannot probe for the existence of other users'
approvals. The schema carries `unique (execution_id)`, so one approval can never
cover several actions.

Approval is not a freeze on the world. Before the tool runs, the coordinator
re-resolves the `ToolSpec` from the live registry and re-evaluates policy. An
action whose tool has since been unregistered, blocklisted, or whose user has lost
a scope, does not execute — it is cancelled and audited.

**Consequences.** A client cannot approve something the server has not already
decided to ask about. An approval granted before a policy change may refuse to
run afterwards, which is the intended direction to fail in.

Both paths converge on `ToolExecutor::run_authorized`, the only function in the
codebase that calls `Tool::execute`. Approval changes whether that is reached,
never how execution happens.

---

## ADR-0016 — What durable action records may contain

**Context.** Resuming an approved action requires storing its arguments. Tool
arguments are also the most likely place for sensitive content to appear — an
email body, a document, a recipient list.

**Options.**
1. Store arguments and results everywhere, for maximum forensic detail.
2. Store nothing, and accept that approvals cannot be resumed.
3. Store arguments only where resume requires them; keep audit argument-free.

**Chosen.** Option 3.

**Reason.** `tool_executions.arguments` holds the validated arguments because
resume means running exactly those — that is the whole mechanism, and dropping
them would mean asking the client to re-supply them, which ADR-0015 forbids.

`audit_events` stores no arguments at all. Its `summary` names the tool and the
*keys* of its arguments, never their values: a calendar title is unremarkable and
an email body is not, and a generic audit writer cannot tell them apart, so it
records neither. A test asserts that known secret-shaped values do not survive
into a summary.

Credentials must never appear in either table. Tools receive credentials from the
server at call time and must not accept them as arguments; that obligation is
stated in the `Tool` contract and is a review requirement for every tool added
later.

**Consequences.** The audit trail answers who did what, when, under which
decision and with what outcome — not what was written in the email. Anyone
needing the latter must read the execution row deliberately, which is a separate,
auditable act.

Rows in `tool_executions` therefore inherit the sensitivity of the most sensitive
tool in the registry. When a tool with genuinely sensitive arguments arrives,
encryption at rest for that column, or a per-tool redaction hook, is the next
step — and is deliberately not built ahead of the tool that needs it.

---

## ADR-0017 — Retention is deliberate, not automatic

**Context.** Approvals, executions and audit events accumulate. Something has to
say what happens to them.

**Options.**
1. Delete rows on a schedule from inside the application.
2. Define retention boundaries now, implement cleanup when there is volume.
3. Keep everything forever.

**Chosen.** Option 2. No cleanup code ships in this milestone.

**Reason.** An application that silently deletes its own audit trail is an
application whose audit trail cannot be trusted — the rows most worth removing
are exactly the ones someone would want removed. Volume is currently zero, so
automatic deletion would be code with no purpose and a standing hazard.

**Intended boundaries**, for whoever implements cleanup later:

| Record | Boundary | Rationale |
|---|---|---|
| `approval_requests` | resolved or expired for 90 days | Answered questions have no ongoing use; the audit row preserves the decision. |
| `tool_executions` | terminal for 90 days | Holds the most sensitive column (`arguments`), so it should be the shortest-lived. |
| `audit_events` | retained; archived, never silently deleted | This is the record of what the assistant did on the user's behalf. |

**Consequences.** Cleanup, when built, must be an explicit operation — a script
or an admin action, not a background sweep — and must not remove audit events as
a side effect of removing executions. `audit_events` deliberately has no foreign
key to `tool_executions` for that reason: deleting an execution cannot cascade
away its audit record.

Approval *expiry* is a different thing and is implemented: it is enforced on the
write path in `claim_approval`, so an approval cannot become executable again
because a sweeper failed to run.

---

## ADR-0018 — The Anthropic provider is a small explicit client, behind a feature

**Context.** Milestone 4 needs a real `ModelProvider`. Two things had to be
decided: what the provider is written with, and where it lives so that ADR-0003
("the core depends on model interfaces, never on providers") still holds once a
provider actually exists.

**Options.**
1. Depend on an official Anthropic SDK from `assistant-models`.
2. Write a small `reqwest` client for the surface this application uses.
3. Put the provider in a new crate, `assistant-anthropic`.

**Chosen.** Option 2, in `assistant-models::anthropic`, behind a non-default
`anthropic` cargo feature.

**Reason.** The surface needed is one endpoint, one streaming format and one
error shape. `reqwest` is already the project's HTTP dependency, declared once
in `[workspace.dependencies]`, and is already compiled for Android by the Tauri
shell — so the provider costs no new dependency and no new cross-compilation
risk. An SDK would add a large dependency to buy a wrapper over `POST
/v1/messages`.

A separate crate was rejected because the feature flag already provides the
property that matters. `assistant-core` depends on `assistant-models` with no
features, so it does not link `reqwest`, cannot name `AnthropicModelProvider`,
and cannot see an Anthropic type. A crate boundary would restate that at the
cost of another manifest.

The boundary is enforced by what crosses it. `wire.rs` and `sse.rs` translate in
both directions; nothing above the provider sees a content block, an SSE event
or an HTTP status, and nothing inside it sees a `TurnEvent`, a `Principal` or a
`RiskLevel`.

**Consequences.** When the API adds a feature this application wants, the change
is in this module and nowhere else. When the API adds something it does *not*
want, nothing happens: responses are parsed leniently, so a new content-block
type — a thinking block, a server-tool result — is ignored rather than treated
as a malformed response.

The cost is that this build owns its API compatibility. `anthropic-version:
2023-06-01` is sent on every request so a server-side change cannot silently
alter the response shape, and the provider tests replay recorded wire bytes
rather than trusting a library to be right.

Tool declarations are generated from the registry's `ToolSpec`s, and carry only
`name`, `description` and `input_schema`. `risk`, `required_scopes` and
`timeout_ms` deliberately do not cross: they are the server's business, and
telling the model about them would invite it to argue about them (ADR-0005).

---

## ADR-0019 — A model call is retried only for transport failures, never with tool results attached

**Context.** Network calls fail. The reflex is a retry policy, but a model call
is not an idempotent GET: it costs money every time, and in a turn that has
already executed tools it can lead to consequential work being done twice.

**Options.**
1. No retries. A transport failure fails the turn.
2. Retry any retryable-looking failure with backoff, as a general HTTP client
   would.
3. Retry only where a retry provably repeats nothing.

**Chosen.** Option 3: at most `ASSISTANT_MODEL_TIMEOUT`-bounded attempts —
one retry by default — for timeouts, connection failures and 5xx, and only when
the request carries no tool results. Rate limits (429) are surfaced, not
retried. Once a stream has delivered bytes to the caller, nothing is retried.

**Reason.** A request with no tool results is a pure question: if it failed at
the transport, nothing happened and nothing was billed, so resending is free of
consequence. A request carrying tool results belongs to a turn that has already
run something. Re-asking the model there is not itself dangerous, but the
model's next move could be to request the same write again, and the honest
answer is to fail cleanly and let the user decide.

Retrying mid-stream is refused for a different reason: the user has already seen
text. Replaying the request would duplicate it on screen.

429 is not retried automatically because the provider's own `retry-after` can be
long, and silently holding a turn open for it is worse than telling the user the
assistant is busy. The value is carried on the error for a future caller that
has a reason to use it.

**Consequences.** A flaky connection costs one extra request at most. A rate
limit is visible to the user immediately. Nothing above the provider retries a
model call at all — the orchestrator has no retry path, because by the time it
sees a failure the turn may already have side effects.

---

## ADR-0020 — The provider credential is read once, in the server's configuration layer

**Context.** `ANTHROPIC_API_KEY` is the first credential in this system that
buys something on the user's behalf. Where it is read, and what can observe it,
had to be decided before a provider existed to use it.

**Options.**
1. Read the environment inside the provider, where it is used.
2. Read it in the server's `Config`, and pass it to the provider as a value.
3. Read it in the server and pass it to the mobile client for direct calls.

**Chosen.** Option 2.

**Reason.** `Config` already exists as the one place the process reads its
environment, and it already has a hand-written `Debug` whose entire purpose is
that adding a secret without redacting it is a visible omission. Reading the
environment in the provider would create a second such place, and
`assistant-models` has no `Debug` discipline of its own to inherit.

Option 3 is the one that matters to rule out explicitly. If the phone called the
provider directly, the key would ship in the APK — `VITE_*` variables are
inlined into the bundle — and permission evaluation, audit and the tool registry
would all be on the wrong side of the boundary. The phone talks to this server;
this server talks to the provider.

**Consequences.** The key exists in exactly three places at runtime: the
process environment, `Config`, and the private field of `AnthropicConfig`. It is
redacted in both `Debug` impls. It is sent as `x-api-key` and never as
`Authorization`, so the header-scrubbing the request logger already does for
query strings is not the only thing standing between it and a log line. No
tracing span in the provider records a header, and provider errors are
constructed from a status line and a parsed error body — never from the request
that was sent — so there is no code path by which the key can reach an error
message.

A deployment with no key is supported and reported, not fatal: the deterministic
path still answers and a turn that needs a model fails with `no_model_provider`
rather than a fabricated reply.

---

## ADR-0021 — Conversation history is durable, and is not memory

**Context.** ADR-0011 recorded that Milestone 2 kept no persistent conversation
state, because nothing then implemented needed to outlive the process. A real
model provider changes that: an assistant that forgets the previous sentence is
not an assistant. This record supersedes ADR-0011's conclusion about
conversation identity.

**Options.**
1. Keep history in memory, keyed by conversation id.
2. Persist conversations and messages in Postgres.
3. Persist, and additionally extract durable facts about the user as they are
   mentioned.

**Chosen.** Option 2. Migration 0003 adds `messages`; `conversations` already
existed from 0001 and is reused, as is the conversation id the WebSocket
already carries.

**Reason.** Option 1 fails the requirement it exists to meet: a restart, a
deploy or a crash silently erases the conversation, and the user finds out by
being asked their name again. It is also the option that quietly becomes the
source of truth for something that has to be right.

Option 3 is Milestone 8, and conflating it with this is the mistake this record
is written to prevent. "My name is Alex" said in a conversation is a fact *in
that conversation*. Promoting it to a durable fact about the user is a different
decision, with different consequences when it is wrong, and it needs its own
retention policy, its own review surface and its own way to be corrected. No
importance scoring, embedding, semantic retrieval or extraction exists here.

**Storage boundary.** A message row holds exactly what was said, because
replaying the conversation is the point of the table. That makes it the most
sensitive table in the database — and it is the reason `audit_events`
(ADR-0016) holds none of it: audit rows are written by generic code that cannot
tell a calendar title from an email body, whereas these rows are only ever read
back to the one principal who owns them.

Roles are `user`, `assistant` and `tool`, not flattened into prose. A tool
result keeps the id of the call it answers, and tool calls are a structured
field beside `content` rather than English encoded into it, so reconstruction
never depends on parsing. Two CHECK constraints hold what the application would
otherwise have to remember: a tool result must name its call, and only an
assistant turn may carry tool calls — a user-supplied tool call is the shape an
injection attempt would take.

**Ownership** is enforced in SQL, not after the read. Every statement selects
the conversation by `(id, user_id)` or joins on it, so there is no code path
that fetches a row and checks the owner afterwards — the shape of check people
forget to write. A conversation belonging to somebody else is indistinguishable
from one that does not exist, and an id that exists under another owner is never
silently re-parented.

**Consequences.** History survives a restart, which is tested against a real
database by dropping the pool and the orchestrator and reading back through new
ones. A failed turn persists the question and no answer: an answer that was
never delivered must not appear in history as though it had been. There is no
failure-state column on a message, so a turn that fails mid-stream leaves the
question with nothing after it — that is the honest record, and a richer one can
be added when something reads it.

---

## ADR-0022 — Context is a bounded window of recent turns

**Context.** With durable history, something has to decide how much of it is
sent to the model each turn. Every message replayed is paid for on every
subsequent turn, so this is a cost decision before it is a quality one.

**Options.**
1. Send the whole conversation.
2. Send a bounded window of recent messages.
3. Summarise older history with a second model call.

**Chosen.** Option 2: the most recent messages, subject to two independent
bounds — a message count (`ASSISTANT_CONTEXT_MAX_MESSAGES`, 40 by default) and a
character budget — with the budget spent from the newest end.

**Reason.** Two bounds because either alone is escapable: a message count says
nothing about a conversation of enormous messages, and a character budget alone
would happily replay a thousand tiny ones. The bound is applied in the SQL as
well as in the window, so a long conversation is never fully read just to be
sliced in Rust.

Option 3 is rejected for this milestone: it makes a conversational turn cost two
model calls, and a summary is a lossy artefact that has to be stored,
invalidated and shown to somebody when it is wrong. It is not obviously needed
before the window is demonstrably too small.

**What is not replayed.** Tool results from *earlier* turns. Within the turn
that produced it, a tool result is in the message list the orchestrator builds;
replaying a stale one invites the model to treat last week's inbox as current.
The record still exists in the store — it simply is not context.

The system prompt is not history either. It travels in
`GenerateRequest::system_prompt` and is assembled per request from a server
constant, so it cannot be trimmed away by the window and cannot be supplied by a
client.

**Consequences.** A long conversation loses its oldest turns rather than getting
slower and more expensive without limit. Something said forty messages ago is
forgotten, which is the correct behaviour for a conversation log and the wrong
behaviour for memory — which is exactly why memory is a separate system
(ADR-0021).

---

## ADR-0023 — The `public` schema is closed to PostgREST, by RLS and by grant

**Decision.** Every table in `public` has row level security enabled with an
empty policy set, and `anon` and `authenticated` hold no grants on the schema —
including by default, for tables that do not exist yet. `assistant-server`
remains the only way into this data.

**Context.** The database is a Supabase project. Supabase publishes `public`
through PostgREST and GraphQL and grants `anon` and `authenticated` full DML on
every table created there (`arwdDxtm`), via `alter default privileges` that
applies to future tables as well. Neither role holds BYPASSRLS.

Every table shipped through Milestone 4 was created with RLS off, which is the
Postgres default and which Supabase does not change. The live database was
therefore reachable — read, write, and `TRUNCATE` — by anyone holding the
project's anon key, against `users`, `conversations`, `messages`,
`tool_executions`, `approval_requests` and `audit_events`. The anon key is
designed to be distributed to clients; it is not a secret and cannot be treated
as one.

Two tables make this worse than a data-exposure finding. `approval_requests`
and `audit_events` are the durable half of the permission architecture: ADR-0015
says an approval authorises exactly one persisted action and is re-validated
against the record before it runs. A caller who can `INSERT` into
`approval_requests` manufactures that record, and a caller who can `DELETE` from
`audit_events` erases the evidence. An open `public` schema is not a
confidentiality bug here, it is an authorisation bypass of ADR-0014 through
ADR-0016.

**Options.**

1. Write per-table RLS policies keyed on `auth.uid()`, the Supabase-native
   approach.
2. Enable RLS with no policies, and revoke the PostgREST role grants.
3. Move the tables out of `public` into a schema PostgREST does not expose.
4. Leave it, and rely on the anon key not leaking.

**Chosen approach: option 2.**

**Reason.** Option 1 solves a problem this project does not have. PostgREST is
not a client here and ADR-0008 already says the webview makes no network calls
of its own; there is no browser holding a Supabase JWT for `auth.uid()` to
return. Writing policies would state the ownership rule a second time, in a
second language, where it could drift from the `user_id` predicates the server
already writes — and it would quietly bless PostgREST as a supported entry
point, which would put a second execution path next to
`ToolExecutor::run_authorized`.

Option 2 matches how the system actually works. `assistant-server` connects as
`postgres`, which owns these tables and holds BYPASSRLS, so an empty policy set
costs the server nothing and denies everyone else everything. The revoked grants
are belt to that braces: a table added later where someone forgets `enable row
level security` is still unreachable, because the roles that could reach it have
no privileges on the schema.

Option 3 is a larger change for the same result, and fights the tooling —
`sqlx` and the Supabase dashboard both assume `public`. Option 4 makes a
published key the only control, which is what the current state already is.

`force row level security` is not used. It would apply the empty policy set to
the table owner as well, and the owner is the server.

**Consequences.** The Supabase dashboard's table editor still works, because it
connects as a privileged role rather than as `anon`. Any future client that
wants PostgREST access does not get it by default; it needs its own ADR, and
that ADR has to explain how it avoids becoming a second execution path. The
security advisor's "RLS Disabled in Public" findings go quiet, which means the
next one it raises will be a real one rather than noise in a list of seven.

---

## ADR-0024 — Authentication is a Supabase-issued ES256 JWT, verified against JWKS

**Decision.** `assistant-server` authenticates callers by verifying a JWT issued
by the project's Supabase Auth (GoTrue) instance, using the ES256 public key
published at the project's JWKS endpoint. `DevTokenVerifier` stays, but only
behind an explicit configuration choice for offline work. Supersedes ADR-0009.

**Context.** ADR-0009 shipped a static shared bearer token as an honest
placeholder, on the stated condition that it never leave local development. It
has since left: the token reaches the Android app through `VITE_DEV_AUTH_TOKEN`,
and Vite inlines `VITE_*` at build time, so it is a string in the shipped
bundle — recoverable with `unzip` and `strings`. That breaks the rule in
CLAUDE.md that no secret lives in `apps/mobile`.

It is also the reason `user_id` scoping is currently decorative. Every caller
presenting the one token maps to one hardcoded `DevTokenVerifier::DEV_USER_ID`,
so the `where user_id = $1` predicates in every productivity query are all
comparing against the same constant. Two devices are one account. The rows are
scoped correctly against an identity that does not vary.

The database is already a Supabase project. Its GoTrue instance is provisioned,
the `auth` schema exists, and the project's JWKS endpoint serves a single
`kty=EC, alg=ES256, use=sig` key — this project signs asymmetrically rather than
with the legacy shared HS256 secret.

**Options.**

1. Verify Supabase's ES256 JWT against the published JWKS.
2. Verify Supabase's JWT with the legacy HS256 shared secret.
3. Issue our own JWTs from an email/password table inside `assistant-auth`.
4. Keep the static token and stop shipping it in the bundle some other way.

**Chosen approach: option 1.**

**Reason.** Option 1 asks the server to hold no secret at all. Verification
needs only a public key, so there is nothing in the server's configuration that
leaking would let an attacker forge a token with — which is a materially
different failure mode from option 2, where the signing secret and the
verification secret are the same string, and any leak anywhere mints valid
tokens for every user. Asymmetric signing also makes key rotation Supabase's
problem: a new `kid` appears in the JWKS and the server picks it up, with no
coordinated secret change.

Option 3 means owning password hashing, reset-email delivery, lockout and
rate-limiting. Each is a well-understood problem and each is a way to be wrong
quietly. Supabase Auth is already paid for and already running; adding a second
identity system next to it would violate the rule against infrastructure without
a demonstrated need.

Option 4 does not address that every caller is still one user.

**How it works.** `SupabaseJwtVerifier` implements the existing `TokenVerifier`
trait, so `AppState` continues to hold `Arc<dyn TokenVerifier>` and no handler
changes. Verification checks the ES256 signature against the JWKS key matching
the token's `kid`, and rejects on `exp`, on an `iss` that is not the project's
auth issuer, and on an `aud` that is not `authenticated`. The `sub` claim is a
UUID and becomes `Principal::user_id`; nothing else in the token is trusted to
carry authority.

The JWKS is fetched once and cached. A `kid` that is absent from the cache
triggers exactly one refetch, rate-limited, so a token bearing an unknown `kid`
cannot be used to drive unbounded outbound requests.

**What does not change.** `PermissionPolicy::evaluate` still takes a `ToolSpec`
and a `Principal` and nothing else — the claims in a JWT are input to *who* the
caller is, never to *what* they may do. Scopes in the token are deliberately not
read as tool permissions; risk stays a static property of a `ToolSpec`
(ADR-0005). RLS remains policy-free and PostgREST remains closed (ADR-0023):
Supabase issues the identity, but it still is not a client of this database.

**Consequences.** The app gains a sign-in screen, and the token it receives is
short-lived and per-device, so no credential is compiled into the bundle. The
existing rows owned by `DEV_USER_ID` belong to no real account and are not
migrated. `users.id` must line up with GoTrue's `auth.users.id` for the foreign
keys to mean anything, so `ensure_user` becomes an insert of a known subject
rather than of an invented one. Offline development still works, because
`DevTokenVerifier` remains selectable — but it is now selected explicitly rather
than by default, so shipping it takes a deliberate act.

---

## ADR-0025 — Credential Encryption at Rest using AES-256-GCM

**Decision.** OAuth refresh tokens and credentials stored in `connected_accounts`
are encrypted using authenticated AES-256-GCM encryption with a 256-bit server
master key (`CREDENTIAL_ENCRYPTION_KEY`). Each row stores a random 96-bit nonce
alongside the ciphertext and authentication tag.

**Context.** Connected Google accounts grant persistent access to sensitive user
data (email, calendar). Raw refresh tokens in PostgreSQL create catastrophic risk
if database dumps or backups are compromised.

**Options.**
1. Store plaintext tokens in PostgreSQL.
2. Encrypt tokens with AES-256-GCM using a server-side symmetric key.
3. Use an external cloud KMS (e.g. AWS KMS, GCP KMS).

**Chosen approach: option 2.**

**Reason.** Option 1 violates basic credential security principles. Option 3
introduces third-party cloud infrastructure dependencies and paid monthly costs
for a personal assistant system. Option 2 provides standard authenticated
encryption using audited pure-Rust crypto (`aes-gcm`), keeping the master key
strictly in server environment variables and never persisting it to disk or
exposing it over network APIs.

**Consequences.** Database backups do not contain usable OAuth credentials. If
the server master key is rotated or lost, connected accounts enter an
expired/reconnect-required state and must be re-authorized by the user.

---

## ADR-0026 — Multi-Account Scoping and Provider-Neutral Integrations

**Decision.** Google integrations support an arbitrary number of connected
accounts ($N$). Every tool call and API request explicitly requires an
`account_id: Uuid`. Database queries enforce ownership with
`WHERE id = $1 AND user_id = $2`. Capability traits (`GmailProvider`,
`CalendarProvider`) live in `assistant-tools`, while concrete Google HTTP clients
live in `assistant-server`. No Google SDK types leak into `assistant-core`.

**Context.** Users often have multiple Google accounts (personal, college, work).
A tool must never accidentally access the wrong account or leak credentials
between users.

**Options.**
1. Global single Google account per user.
2. Multi-account support with explicit account parameters and database-level ownership.

**Chosen approach: option 2.**

**Reason.** Enforcing ownership at the database query level ensures that even if
an attacker modifies an account ID in an API request or prompt, the query returns
zero rows, making another user's account indistinguishable from a non-existent
one. Trait interfaces ensure `assistant-core` remains decoupled from concrete
Google API schemas.

---

## ADR-0027 — Deterministic Free-Time Interval Arithmetic

**Decision.** Available schedule slot computation is implemented as a
deterministic mathematical interval complement over existing calendar events
within a time window `[start, end]`, accounting for minimum event duration,
meeting buffers, and study/work hours. No LLM inference is used.

**Context.** Users need to find available time slots for scheduling tasks. Routing
this question through an LLM is non-deterministic, high latency, costly, and
prone to arithmetic hallucinations.

**Options.**
1. Send calendar events to an LLM and prompt it to find free time.
2. Compute free time intervals deterministically in Rust.

**Chosen approach: option 2.**

**Reason.** Interval complement arithmetic is completely deterministic. It executes
in sub-millisecond time and produces 100% mathematically correct available
slots.


## ADR-0028 — Projects and labels are rows, not repeated strings

**Decision.** `tasks.project text` and `notes.tags text[]` are replaced by
`projects` and `labels` tables owned by a user, referenced by id, with
`task_labels` and `note_labels` as join tables. Names are unique per user,
case-insensitively, via a `lower(name)` unique index. One project per user
carries `is_inbox = true` and is the fallback every task lands in. Labels are
shared between tasks and notes. `updated_at` is written by a database trigger
rather than by each UPDATE statement. Migration `0007`.

**Context.** The productivity layer stored a project as a name repeated on every
task row and a tag as an element of a `text[]` on the note. Neither could be
renamed without rewriting every row that mentioned it, neither could carry a
colour or an ordering, and 'Work', 'work' and 'Work ' were three distinct
values. A label could not be shared between a note and a task, so the same word
meant two unrelated things depending on which screen created it. Separately,
every UPDATE in `routes/productivity.rs` hand-wrote `updated_at = now()`: one
place per statement to forget, and any writer other than that file left the
timestamp stale.

**Options.**
1. Leave the strings and deduplicate in the client.
2. Normalize projects only, keep `notes.tags` as an array.
3. Normalize both into owned rows with join tables, and move `updated_at` into a
   trigger.

**Chosen approach: option 3.**

**Reason.** Option 1 puts the uniqueness rule in the one place that cannot
enforce it — a rename in one client cannot reach rows another client wrote.
Option 2 keeps the harder half of the problem: a tag is exactly the thing a user
most wants to rename and to reuse across item types, and `tags @> '{x}'` cannot
share an index with the equality lookup a join table gets for free. Option 3
makes the database the place a name exists once, which is what makes renaming a
single UPDATE and makes "everything labelled @home" one indexed join.

`on delete restrict` on `tasks.project_id` is deliberate. Deleting a project
that still holds tasks is a decision about those tasks, not a side effect of
tidying a sidebar; the route reassigns them to Inbox and then deletes, so the
reassignment is visible in the code rather than implied by a cascade.

`is_inbox` is a column rather than a match on the literal name `'Inbox'` because
the user is free to rename that project, and a fallback that stops working when
renamed is a bug that only appears for users who customise.

**Consequences.** `PROTOCOL_VERSION` goes to 4: `TaskItem` gains `project_id`
and `labels`, and `NoteItem`'s `tags` becomes a resolved list of label names
rather than a stored array. New endpoints `/v1/projects` and `/v1/labels` manage
the rows. Clients written against version 3 are rejected by the version check
rather than silently misreading a task with no `project` field.

## ADR-0029 — An unsent local write is queued, never silently accepted

**Decision.** `apps/mobile/src/api/productivity.ts` no longer treats
`localStorage` as an alternative database. A write made while the server is
unreachable is appended to a durable outbox, replayed in order on the next
reachable probe, and surfaced to the user as pending until the server has
acknowledged it. `localStorage` holds a cache of server state plus the outbox,
and is never the system of record.

**Context.** The offline fallback wrote the item to `localStorage` and returned
it as if the write had succeeded. Nothing ever replayed it. A task created
while the server was down existed only in that browser profile's storage: it
never reached Postgres, so it did not survive reinstalling the app, clearing
site data, or opening the app anywhere else — and the UI reported success either
way. The Supabase `tasks`, `reminders`, `notes` and `ideas` tables held zero
rows while the app showed a populated list.

**Options.**
1. Keep the silent fallback.
2. Remove the fallback and fail the write outright when the server is down.
3. Keep writing locally, but as a replayed queue with pending state in the UI.

**Chosen approach: option 3.**

**Reason.** Option 1 is the same class of mistake as a canned assistant reply
that looks like inference: it reports a state the system is not in. Durability
is the entire reason the row goes to Postgres, and an interface that cannot
distinguish "saved" from "saved nowhere" has removed the user's ability to
notice. Option 2 is honest but throws away work for a network blip on a device
that is offline by nature. Option 3 keeps the capture and keeps the truth: the
item is visible immediately, marked pending, and becomes durable the moment the
server answers.

**Consequences.** Replay is idempotent by client-generated `id`: the create
endpoint accepts the client's UUID so a retry after an ambiguous failure
updates the same row instead of creating a second one. Queue entries carry an
attempt count; an entry rejected with a 4xx is dead-lettered rather than
retried forever, because a request the server has judged invalid will not become
valid by being sent again.

## ADR-0030 — The deployed image pins the workspace toolchain, and the blueprint carries no secret

**Decision.** `Dockerfile` builds on a Rust image that satisfies the workspace's
`rust-version`, and `render.yaml` declares every secret as `sync: false` rather
than carrying a value.

**Context.** The Render deploy never produced a running service. The builder
stage was `rust:1.80-slim`, while the workspace root declares
`edition = "2024"`, `resolver = "3"` and `rust-version = "1.90"`. Cargo 1.80.1
cannot parse that manifest at all:

```
error: failed to parse manifest at `/w/Cargo.toml`
Caused by:
  feature `edition2024` is required
```

The failure is at manifest parse, before any crate is compiled, so no amount of
environment configuration on Render could have made the deploy succeed.

Two further problems were visible in the same files. `render.yaml` carried a
literal `DEV_AUTH_TOKEN` and a literal 64-character hex
`CREDENTIAL_ENCRYPTION_KEY`. The first is the bearer token for every protected
route; the second is the AES-GCM key that ADR-0025 uses to encrypt Google
refresh tokens at rest. Both were committed, so the encryption at rest was
decorative: an attacker with the repository and the database has the plaintext.
Separately, there was no `.dockerignore`, so a 143 MB `node_modules` tree was
uploaded as build context on every deploy.

**Options.**
1. Relax the workspace to an older edition so `rust:1.80` can build it.
2. Pin the builder image to the version the workspace already requires.
3. Install a toolchain inside the image with `rustup` at build time.

**Chosen approach: option 2**, with `.dockerignore` added and both secrets moved
to `sync: false`.

**Reason.** Option 1 inverts the dependency: the deployment target does not get
to dictate the language edition the codebase is written in. Option 3 adds a
network download and a second source of truth for the toolchain to every build,
for no benefit over choosing the right base image. Option 2 makes the failure
impossible to reintroduce silently — a workspace `rust-version` bump past the
pinned image fails the build with the same clear message, in CI, rather than on
Render.

`rust-toolchain.toml` is deliberately still not copied into the image. It pins
`channel = "stable"` and four Android targets for local mobile work; honouring
it in the container would download a floating toolchain plus four unused
standard libraries on every deploy.

**Consequences.** The release profile is overridden for the container build
only, via `CARGO_PROFILE_RELEASE_LTO=false` and
`CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16`. The workspace profile's fat LTO and
single codegen unit exist for the shipped mobile binary (ADR-0002) and are the
usual cause of an out-of-memory build on a small hosted builder; the server
binary does not need them. `cargo build` is `--locked`, so a deploy fails rather
than resolving a dependency the lockfile does not name.

Because `CREDENTIAL_ENCRYPTION_KEY` is no longer the committed value, any Google
credential encrypted under the old key cannot be decrypted and the account must
be reconnected. `DEV_AUTH_TOKEN` and `CREDENTIAL_ENCRYPTION_KEY` must now be set
in the Render dashboard before the first deploy; the service will not start
without the first, and will fall back to the development key with a warning
without the second.

## ADR-0031 — The mobile shell trusts a bundled root store, not the platform verifier

**Decision.** The Tauri shell's `reqwest` client is built with an explicit
`rustls` configuration over the bundled Mozilla root set (`webpki-roots`),
rather than the platform verifier `reqwest` selects by default.

**Context.** The app reported "Demo Mode" against a healthy server. The
connection dot never left `checking`, because `probe_server` never returned:

```
thread 'tokio-rt-worker' panicked at rustls-platform-verifier-0.6.2/src/android.rs:94:10:
Expect rustls-platform-verifier to be initialized
```

`reqwest` 0.13's `rustls` feature pulls in `rustls-platform-verifier` and makes
it the default certificate verifier. On Android that crate must first be handed
the app's `Context` over JNI; a Tauri shell never does this, and uninitialised
it panics rather than returning an error. The panic kills the task running the
command, so the IPC call is never answered and the `invoke` promise cannot
settle. `loadConnection` awaits it forever.

Two things kept this hidden. Plain HTTP never reaches certificate
verification, so while `VITE_SERVER_BASE_URL` pointed at a LAN address the
probe failed quickly and honestly; only the move to an `https://` origin
exposed it. And the failure mode is a hang, not an error, so the UI had nothing
to display.

The workspace already listed `webpki-roots` in `reqwest`'s feature list. That
feature does not exist in `reqwest` 0.13: `webpki-roots` is an optional
*dependency*, so naming it compiles the crate in and nothing more — no code in
`reqwest` references it. The intent was already bundled roots; it simply had no
effect.

**Options.**
1. Initialise `rustls-platform-verifier` over JNI from the Tauri shell.
2. Configure `rustls` explicitly with the bundled Mozilla roots.
3. Use the OS trust store via `rustls-native-certs`.

**Chosen approach: option 2.**

**Reason.** Option 1 keeps the OS trust store but requires the shell to reach
into JNI for a `Context` during startup and to keep that working across Tauri
upgrades — a platform-specific initialisation path whose failure mode is a
panic in a background task, which is exactly what just cost a day. Option 3
reads `/system/etc/security/cacerts`, whose layout Android has been moving into
an APEX module, so it is not dependable across versions. Option 2 is the same
trust decision the conversation socket already makes: `tokio-tungstenite` is
configured with `rustls-tls-webpki-roots`. Both transports now trust the same
anchors on every platform, and the shell has no JNI dependency.

**Consequences.** The root store ships with the app and changes only when the
app is rebuilt, so a root rotation requires a release; for a client that talks
to one known host this is acceptable. Enterprise or user-installed CAs in the
Android trust store are deliberately not honoured, which also means a device
with a user-installed interception certificate cannot silently read this
traffic. `rustls` and `webpki-roots` are declared in `[workspace.dependencies]`
and the provider is pinned to `aws-lc-rs`, matching what `reqwest` enables, so
the preconfigured config is the type `reqwest` expects.

`ProbeResult` also carried `#[serde(rename_all = "camelCase")]`, which renames
enum *variants* and not their fields. The shell sent `latency_ms` to a UI
reading `latencyMs`, so the sheet rendered "undefined ms"; the enum now also
sets `rename_all_fields`.

## ADR-0032 — Classroom is read-only, student-scope, and consent is not incremental

**Decision.** Milestone 6 requests four new Google scopes — three Classroom
`.readonly` scopes in their `.me` form, and `drive.readonly` — as one consent
screen, and implements no Classroom write operation of any kind.

**Context.** The assistant needs to understand a student's academic
obligations. Google Classroom exposes that through `courses.list`,
`courses.courseWork.list` and `courses.announcements.list`. Each has a
`.readonly` scope, and the coursework scopes come in two forms: `.me`, which
reads the signed-in student's own work, and `.students`, which reads the work
of students in courses the caller teaches or administers.

The existing OAuth flow requests every scope at once with `prompt=consent`, and
stores what Google actually granted in `connected_accounts.scopes`.

**Options.**
1. Request the `.students` scopes as well, to support teachers later.
2. Request only the `.me` read scopes.
3. Add Classroom write scopes now, for submitting assignments.

**Chosen approach: option 2.**

**Reason.** Option 1 asks a student to grant the ability to read other
students' grades in order to see their own timetable, which is a worse consent
prompt for a capability nothing uses. Option 3 is out of scope and carries a
consequence this application should not be able to cause: submitting an
assignment is not undoable by the person who did it. The scopes granted are the
ceiling on what a bug can do, so the ceiling is set at reading.

**Consequences.** Teacher and administrator capabilities are unavailable, and a
course where the user is the teacher rather than a student will not appear:
`courses.list` is called with `studentId=me`. That is a real limitation and is
documented in `docs/MILESTONE-6.md` rather than worked around.

Consent is not incremental. An account connected before this milestone holds
only the Gmail and Calendar scopes, and every Classroom or Drive call against it
returns 403 until the user reconnects it. Rather than let that surface as an
unexplained error, `AccountSummary::scopes` is compared against what each
feature needs and the UI offers "Reconnect" on the specific account. Re-running
consent does not create a second account: the upsert key is
`(user_id, provider, provider_account_id)`.

Google's error bodies quote the request back, which for these APIs can include a
course name, a file name or a search query. `api_error` maps the HTTP status to
a sentence and drops the body, keeping the four cases a user can act on —
reconnect, ask an administrator, wait, retry — distinguishable without
forwarding Google's text into a phone screen or a log.

A Workspace for Education domain can block unverified third-party applications
from Classroom data entirely, in which case a school account returns 403
regardless of the scopes granted. The 403 message says so.

## ADR-0033 — Drive is read-only, is never mirrored, and refuses before it downloads

**Decision.** Drive access uses `drive.readonly`; no Drive data is stored in
Postgres; and a file is checked for type and size before any content request is
made, with a ceiling of 512 KiB.

**Context.** The milestone needs to find a file by name across My Drive and open
small text documents. Google offers two relevant scopes. `drive.file` is
non-sensitive but grants access only to files the user has individually picked
through Google's own file picker, so it cannot answer "search my Drive" at all.
`drive.readonly` can, and is a Google *restricted* scope: a published
application using it must pass OAuth verification and, if it stores user data on
a server, a third-party security assessment.

**Options.**
1. Use `drive.file` and drop search.
2. Use `drive.readonly`.
3. Use `drive.metadata.readonly` for search and never read contents.

**Chosen approach: option 2.**

**Reason.** Option 1 does not implement the requirement; a picker-scoped
integration cannot find a file the user has not already pointed at. Option 3 is
also restricted, so it pays the same verification cost while removing the
ability to open a document. Option 2 is the only one that does the job, and the
verification cost is a distribution problem rather than a technical one.

**Consequences.** Until verification is completed the application must stay in
Google's testing mode, where restricted scopes work for a limited number of
explicitly listed test users and every other user sees an unverified-app
warning. This is a real constraint on distribution and is recorded in
`docs/MILESTONE-6.md`.

Nothing from Drive is written to Postgres. A user's Drive is not this
application's data to keep, and mirroring it would turn a read-only integration
into a second copy of their files with its own breach surface. Metadata is
fetched on demand and cached on the device, which is enough for the offline
requirement.

`read_small_file` fetches metadata first and refuses on type before size, so a
900 MB video is rejected without a byte being requested. A file that reports no
size and is not an exportable Google document is refused rather than streamed
blindly, because an unknown length is exactly the case a limit exists for.
Refusals name the file and say why. Above `MAX_INLINE_CHARS` the response is cut
and `truncated` is set, so the UI can say it is showing only the beginning
rather than presenting a partial file as whole.

PDFs are searchable and their metadata is returned, but they are not read as
text. Extracting a PDF needs OCR and page-level handling that belongs to the
later document-intelligence milestone; answering from a partial extraction would
be exactly the confident wrong answer this project's rules forbid.

## ADR-0034 — Imported coursework is a task with provenance, synced on demand

**Decision.** Classroom coursework becomes a row in `tasks` carrying its origin.
Identity is `(user_id, external_provider, external_id)` and is enforced by a
unique index. Sync is triggered explicitly, never on a timer, and decides field
by field whether the provider or the user owns a value.

**Context.** An assignment is an obligation the user has, and the application
already has a table for those. Copying coursework into a parallel "academic
items" table would mean the Tasks screen, the free-time engine and the scheduler
each had to learn about a second kind of deadline, and a student would see the
same assignment twice.

But an imported task is not entirely the user's: Classroom owns the title and
the due date, and moves them. The user owns their notes, their priority, and any
rename they make. A sync has to serve both.

**Options.**
1. A separate academic-items table, joined at read time.
2. A task row per assignment, overwriting from Classroom on every sync.
3. A task row per assignment, with the provider's last-known values recorded
   alongside the current ones.

**Chosen approach: option 3.**

**Reason.** Option 1 duplicates the concept of a deadline and pushes the
duplication into every consumer. Option 2 is simple until the first time a user
renames an assignment to something meaningful to them and a background refresh
silently discards it. Option 3 makes the question answerable: `source_title` and
`source_due_at` record what Classroom last sent, so a field that still matches
is provider-owned and a field that differs has been edited by the user. Without
those columns the only available policies are "always overwrite the user" and
"never update the deadline", and a wrong deadline is not acceptable for a value
the scheduler acts on.

Title and due date are decided independently, so renaming an assignment does not
freeze its deadline.

**Consequences.** Running a sync twice cannot create a second task: the unique
index is partial, on `external_id is not null`, so manually created tasks are
unaffected. A row created before this milestone has no `source_title` and is
treated as user-owned, which is the safe direction — the alternative would let
the first sync overwrite a title someone had been maintaining by hand.

Coursework that disappears from Classroom leaves its task alone. There is no
delete path in sync at all. The task belongs to the user, and a teacher tidying
a course is not consent to destroy the user's copy of the work.

`external_account_id` is `on delete set null`. Disconnecting a Google account
stops future requests, but no task, note or reminder is removed; the imported
rows keep their `source` so the UI can still say where they came from.

Sync is explicit. Polling Classroom on a schedule would spend a shared quota
re-reading data that changes a few times a term, so the refresh is a button.
`academic_sync_state` records when each resource last synced and what failed,
which is what lets the UI show the cache's age rather than implying a live read.
No Redis, no queue, no scheduler.

Deduplication across sources is deliberately limited to exact identity —
provider plus external id. A Gmail message mentioning the same assignment is not
matched to it. Doing that well needs semantic comparison, which belongs to a
later milestone; guessing at it here would silently merge two obligations that
happened to share a word.

## ADR-0035 — Long-term memory is a durable domain, not a running transcript

**Decision.** Memory lives in a first-class `memories` table, owned per user and
scoped by `user_id` in every statement. The model may **propose** a memory
(`MemoryProposal`) but never write one: proposals are validated by
deterministic Rust and only accepted values reach the store as `NewMemory`.
Retrieval on the assistant path is bounded (at most `MAX_CONTEXT_MEMORIES = 8`
ranked rows are injected as facts). Ranking, secret rejection, and lifecycle
transitions are all deterministic.

**Context.** Milestone 2 left `assistant-memory` as a placeholder trait so that
conversation logging could not silently become memory. M7 fills it, and the
project has two failure modes to avoid at that boundary. First, letting the
model be the memory writer: a language model asked to "remember X" and given
a table would happily invent, exaggerate, or contradict itself, and the
resulting rows would look like ground truth to the next turn. Second,
inflating architecture: a memory subsystem tempts embeddings, a vector store,
a knowledge graph, semantic dedup, and a scheduler, none of which the current
product needs. The right shape is a small typed domain with clear seams.

**Options.**
1. Let the model call a `remember(...)` tool that writes directly.
2. Extract memory from every conversation with a background worker feeding
   pgvector + embeddings.
3. A typed domain layer where the model proposes and deterministic code
   decides, backed by a plain Postgres table with Postgres text search.

**Chosen approach: option 3.**

**Reason.** Option 1 makes model output authoritative and undoes ADR-0005's
"model output is untrusted input" rule for a different resource. Option 2 is
what a milestone brief specifically asks not to build — vectors, background
extraction, embeddings, complex ranking — and the shape it forces (a large
unowned index of extracted claims) is exactly the "AI memory experiment"
mode the project rejects. Option 3 keeps memory boring where boring is
correct: rows in a table with a `user_id`, a `kind`, a `content`, an
`importance`, a `confidence`, a bounded `MemorySource` for provenance, an
`expires_at` for temporaries, and a `lifecycle` of `active | archived |
superseded`. The store is a trait (`assistant_memory::MemoryStore`) and the
core depends only on the trait — same pattern as `ConversationStore` and
`ActionStore`.

Provenance is mandatory and enumerated. `explicit_user_input` is the "the
user typed this" case; the others (`conversation`, `task`, `note`, `idea`,
`project`, `document`, `external_source`) name where the memory came from
so the UI can answer *why does the assistant remember this?* Free-form
provenance would let a proposer invent a category and defeat the audit.

`Importance` (1..=5) and `Confidence` (0.0..=1.0) are separate values. They
mean different things: importance is how much the user cares, confidence is
how sure the system is that the content is true. Collapsing them into one
"score" would hide that distinction; keeping them apart makes each queryable
and each editable in the UI.

Retrieval is deterministic and bounded. `MemoryContextProvider` wraps the
existing conversation-history provider and, per turn, asks the store for a
short list of candidates scoped to the principal, ranks them with
`assistant_memory::rank` against the raw request text (text overlap,
importance, confidence, 24-hour recency decay, prior usage), and injects at
most `MAX_CONTEXT_MEMORIES` into the turn context as facts. The model never
sees the whole memory set; the store enforces an upper bound (`MAX_SEARCH_LIMIT`)
on the API too, so a client cannot pull the entire table with one call.

Access tracking is explicit. `last_accessed_at` and `access_count` are moved
only when the retrieval layer declares the memory "used" (returned in a
turn's context) — not on every incidental read. That makes the counter mean
something the ranker can trust; a scheme that bumped it on every SELECT
would produce noise the moment the UI listed archives.

Explicit memory bypasses the model. "Remember that I prefer concise answers."
is parsed by `assistant_memory::parse_explicit_memory` on the deterministic
router; the handler constructs a `NewMemory` with `explicit_user_input`
provenance and hands it to the store. The turn skips the model entirely, so
the user's phrasing is preserved verbatim and the outcome is testable in
Rust.

Secret-like content is refused before persistence. A small heuristic
(`assistant_memory::looks_like_secret`) rejects obvious API keys, bearer
tokens, PEM blobs, and long high-entropy alphanumeric runs. It is not a
security control — a determined user can still paste a password — but it
catches the common accidental case, and it fires in one place (the domain
crate) so the API, the deterministic handler, and the store cannot drift.

Conflict handling is `supersede`, not merge. A newer memory replaces an
older one on request (either from the API's `supersedes` field or a store
call), and the older row is marked `superseded_by = <new id>` with
`lifecycle = 'superseded'`. Both `alice.old` and `bob.new` must belong to
the same user; the store refuses cross-owner supersession with `NotFound`.
Nothing here attempts semantic contradiction detection; that belongs to a
later milestone.

Temporary memories carry an `expires_at` and are archived on the retrieval
path, not by a scheduler. A partial index on `(expires_at) where kind =
'temporary' and lifecycle = 'active'` keeps the sweep cheap. This preserves
the M7 rule "no new infrastructure": Postgres does the work `cron` would.

**Consequences.** Adding a new memory kind or source is a migration (the
database `check` constraint enumerates them) and a small enum change, not a
config knob. The bounded retrieval keeps every model call's context cost
predictable, so a growing memory set does not silently slow every turn. The
supersede rule preserves history — an old preference is still queryable
under the archived filter, and the UI can show what replaced it — so a
future contradiction detector can operate on real chains instead of
guessing what came before. And because the writer is always deterministic
Rust, swapping model providers (Claude, OpenAI, Gemini) does not change
what the memory database is willing to accept.

Explicitly not built as part of M7: vector embeddings, a semantic search
index, autonomous background extraction, cross-conversation summarisation,
psychological profiling, and any scheduler or queue. Postgres text search
(`tsvector` + a GIN index + `websearch_to_tsquery`) is enough for the
baseline; embeddings can arrive when a real use case demands them, without
touching this ADR's boundaries.

## ADR-0036 — Document / PDF intelligence: deterministic first, providers behind traits

**Decision.** Documents are a first-class domain in their own crate
(`assistant-documents`). Ingestion produces a metadata row, bytes go to a
pluggable object store, extraction is deterministic (via `pdf-extract`), OCR
and visual/multimodal verification are provider-neutral traits invoked only
when the deterministic pass leaves a page unreadable, and every page carries
its own extraction method so provenance survives to the API and the UI.

**Context.** The M6 Drive integration deliberately refused to read PDFs
because doing so requires OCR and page-level handling. M8 supplies that
handling, but the same failure modes the memory milestone had to avoid still
apply: a language model given a "read this file and remember what it says"
tool would invent content, silently drop pages, and treat every extraction as
ground truth. On the other end, running a multimodal model over every page of
every PDF is a real bill of money and latency, and it makes retrieval slow
enough that the fast deterministic paths that already exist stop feeling fast.

The right shape is the same shape memory took: a small typed domain layer,
storage behind a trait, providers behind narrower traits, and no path that
lets a model turn document content into stored data without the application
deciding.

**Options.**
1. Extract with a multimodal model on every page, index vectors, retrieve
   with embeddings.
2. Extract text on the client with `pdf.js`, send the flattened text to the
   server, store as one blob.
3. Deterministic server-side extraction with `pdf-extract`, per-page rows in
   Postgres, OCR/vision traits called only for pages that need them, page-
   level full-text search with `tsvector`.

**Chosen approach: option 3.**

**Reason.** Option 1 is what a milestone brief specifically forbids: expensive
model calls for basic metadata and text extraction, vector infrastructure the
project has no other use for, and a pipeline where every page depends on a
provider being up. Option 2 solves the wrong problem: PDF parsing works fine
in Rust, the mobile bundle is not the right place to hold a PDF parser, and
"send the flattened text" throws away the page boundaries retrieval needs.

Option 3 keeps the same rules the rest of the project runs on:

* **Determinism first.** `pdf-extract` returns per-page text and a page count
  from raw bytes with no network call. A page whose native text is under 40
  printable characters is flagged as `needs_ocr` -- a small, testable
  heuristic, not a "should we try OCR" model prompt.

* **Storage behind a trait.** `DocumentStorage` has a `LocalFilesystemStorage`
  backend today, mapping `user_id/document_id` to files under a configurable
  root. A Supabase Storage or S3 backend is a straightforward implementation
  against the same trait; the pipeline does not know which backend is
  configured. Bytes never live in an ordinary Postgres row.

* **OCR and vision behind traits.** `OcrProvider` and `DocumentVisionProvider`
  are narrow: give them a rendered page, they return text and a confidence.
  The default `NullOcrProvider` returns `NotAvailable`; the pipeline records
  the page as "OCR needed" rather than pretending to have read it. A
  Tesseract or cloud-OCR backend plugs in without touching the domain crate.
  Same for a model-vendor's vision API.

* **Provenance is mandatory.** Every `DocumentPage` records how it was
  extracted (`native_text | ocr | visual_verification | none`) and, for
  non-native methods, a confidence. Every `DocumentProvenance` names the
  document, filename, source, page and method. The `DocumentMemoryProposal`
  and `DeadlineCandidate` types both carry a `DocumentProvenance`, so a
  memory or a task derived from a document is traceable back to `filename
  page 7, OCR`.

* **Bounded retrieval, page-level search.** Pages are stored per row with a
  `tsvector` maintained by trigger. Search returns page-level hits with
  snippets; there is a hard `MAX_CONTEXT_PAGES` (4) and `MAX_CONTEXT_CHARS`
  (8000) budget for anything the assistant injects into a turn's context. No
  code path can dump a whole 200-page PDF at the model.

* **The processing state machine is linear and honest.** `Uploaded →
  Extracting → [Ocr] → [Verifying] → Indexed`, with `Failed` as a terminal
  side branch that always sets `processing_error`. Reprocess restarts from
  the current bytes.

* **Explicit-only ingest.** `POST /v1/documents` accepts bytes with a
  `Content-Type` and `X-Filename` header. `POST /v1/documents/from-drive`
  takes an account/file pair, uses the existing M6 Drive client to fetch
  bytes (behind size and MIME checks), and runs the same pipeline. There is
  no autonomous "sweep the user's Drive for PDFs" path -- ADR-0032 and
  ADR-0033 already said Drive is user-triggered, and M8 keeps that.

**Consequences.** Same-user, same-bytes re-uploads dedup on `(user_id,
content_hash)`; the `on conflict` clause refreshes `storage_key` so a
re-upload after a storage GC still resolves. Deletion of a document cascades
to its pages via `on delete cascade`, and the storage object is deleted
best-effort. The `MAX_DOCUMENT_BYTES` limit (25 MiB) is enforced twice for
Drive ingest -- once against the reported metadata size (before any
download), once against the actual body -- so a provider that lies about
size cannot spend memory.

Explicitly not built as part of M8: a background job queue, vector
embeddings, semantic search, per-document access sharing, PDF form
extraction, and automatic memory/task creation. Deadline candidates and
memory proposals are produced deterministically; the application (or a later
milestone with an explicit "review candidates" UI) is what decides whether
they become durable rows. Real OCR and real multimodal verification plug in
behind the existing traits when a provider is configured; the M8 deployment
default is honest "no OCR available, page recorded as unreadable" rather
than fabricated content.

Storage backend note. The `LocalFilesystemStorage` backend is sufficient for
a single-node deployment (including the current Render container) and for
local development. A `SupabaseStorage` HTTP-backed implementation of the
same trait is the intended production backend and is not implemented in
M8; the migration path is a new impl of `DocumentStorage` plus a
`document_storage_backend` config value. No schema change is needed for that
migration because the storage_key is opaque to Postgres.

## ADR-0037 — The model credential is chosen with the endpoint, and provider round-trip state is carried opaquely

**Decision.** Three changes, all on the model-provider seam:

1. `Config::openai()` picks the API key *and* the base URL together. If the
   deployment targets Google's OpenAI-compatible endpoint it uses
   `GEMINI_API_KEY`; otherwise it uses `OPENAI_API_KEY`. Neither key is a
   fallback for the other, and a missing key means no provider rather than a
   call that is certain to be rejected. `Config::targets_gemini()` is the single
   predicate; `routes::transcribe` uses the same rule.
2. `assistant_tools::ToolCall` gains
   `provider_metadata: Option<serde_json::Value>` — opaque state the provider
   attached to a call and requires back on the next round. The OpenAI provider
   reads it from `tool_calls[].extra_content` (streaming and non-streaming) and
   echoes it verbatim on the follow-up request. `ToolCall::new` constructs a
   call without it, which is what the orchestrator and the approval-resume path
   do.
3. The system prompt no longer asserts which integrations exist. The tool list
   sent with the turn is the source of truth.

**Context.** No voice or text turn that needed a tool could complete. Three
faults in series, each masking the next:

* The key and the base URL were chosen from different sources: the key preferred
  `OPENAI_API_KEY`, while the base URL was switched to
  `generativelanguage.googleapis.com` whenever a Gemini key or a `gemini-*`
  model was configured. A deployment holding both keys sent an OpenAI `sk-`
  credential to Google, which answered
  `400 Please pass a valid API key` — surfaced as `provider_invalid_request`.
* With the right key, the model refused to call the calendar tool it had been
  handed, because the system prompt still said "Integrations such as email,
  calendar and files are not connected yet". That sentence was true when it was
  written and has been false since the Google tools were registered in M6. A
  constant cannot know what a deployment wired up.
* With the right key and an honest prompt, the first tool round succeeded and
  the second model call failed:
  `400 Function call is missing a thought_signature in functionCall parts`.
  Gemini 3 returns a signed blob with each function call and rejects the
  follow-up round unless it is echoed. The provider was dropping it.

**Options.**

* *Key/endpoint*: (a) keep the `or` fallback and document it; (b) pair them.
  (a) leaves a configuration that is silently guaranteed to fail.
* *Prompt*: (a) rebuild the prompt at startup from the registry; (b) stop
  claiming anything about integrations and point at the tool list. (a) makes the
  prompt a runtime value the tests cannot pin, for no gain — the model already
  receives the tool list.
* *Signature*: (a) pin a model that does not require it; (b) drop to
  non-streaming; (c) carry the blob. (a) is a countdown — `gemini-2.5-flash`
  already returns 404 for new users and `gemini-2.0-flash` is retired. (b) was
  measured and fails identically: the requirement is the echo, not the
  transport.

**Chosen approach.** Pair the credential with the endpoint; ground the prompt in
the tool list; carry the blob.

**Reason.** Each fault was a case of one half of a pair being changed without
the other. The credential belongs to the endpoint. The claim about integrations
belongs to the registry. The provider's round-trip state belongs to the call it
came from.

**Consequences.**

* A deployment that sets `ASSISTANT_MODEL=gemini-*` without `GEMINI_API_KEY`
  now runs with no model provider and says so at startup, instead of failing
  every turn at the provider.
* `provider_metadata` widens `ToolCall`, which is a shared type. It does not
  widen the permission seam: `PermissionPolicy::evaluate` takes a `ToolSpec` and
  a `Principal`, and the field is on neither. It is never read, never matched
  against, never written to an audit record, and goes nowhere except back to the
  provider that issued it. ADR-0005 is unaffected.
* Anthropic sets it to `None`; the field is skipped on the wire when absent, so
  an OpenAI request is byte-identical to what it was before.
* Retired model ids remain an operational hazard: `gemini-2.0-flash` and
  `gemini-2.5-flash` both now 404. The model id stays configuration, not code.
