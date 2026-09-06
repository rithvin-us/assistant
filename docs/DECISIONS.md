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
