# Milestone 13 — System Hardening and Production Readiness

**Status: RED.**

M13 is a gate before system-wide wake word, autonomous computer access, a PC
agent, or Claude Code control. It does not pass. The audit is now broad — every
area in the M13 brief was examined — but it found defects that must be closed,
and several gate criteria can only be verified against the deployed environment
and a physical device, neither of which was exercised.

Nothing here is asserted without evidence. Where a check was not performed, it
says so rather than assuming a result.

---

## 1. Repository state

The audit began on `main` at `166f85f`, with both M12 branches
(`claude/m12-voice-completion-reliability`, `fix/m12-voice-pipeline-tool-turns`)
already merged into it — confirmed with `git branch --merged main`.

M12 advanced mid-audit. The working checkout moved to
`claude/m12-finalize-voice` at `c699262` ("populate principal scopes from
connected Google accounts and enforce multi-account isolation"). Every defect
found at `166f85f` was re-confirmed present at `c699262`.

M13 work is isolated in a git worktree on `claude/m13-system-hardening`, rebased
onto `c699262`. The primary checkout was never switched, so parallel M12 work
was not disturbed and the M12 branch was not modified.

## 2. M12 baseline

`c699262`. A moving snapshot, not a final state. `c699262` claims to fix scope
propagation; that claim was verified and is **substantially true** for
`expand_google_scopes` and `get_access_token_with_required_scope`, but the
`Principal.scopes` half is implemented as a union across accounts — see
finding H5.

## 3. Findings

Severity reflects exploitability in the system as configured, not worst
imaginable case.

### Fixed in this milestone

**C1 — CRITICAL — Production authentication bypass.** The deployed service
accepted the bearer token `local-dev-token`. Confirmed with two read-only
requests:

    Bearer local-dev-token   -> 200  []
    Bearer bogus-token-xyz   -> 401  {"code":"unauthorized",...}

`render.yaml` provisions `DEV_AUTH_TOKEN` and never sets
`SUPABASE_PROJECT_REF`; `lib.rs` selected `DevTokenVerifier` whenever that was
`None`, logging only a `warn!`. `DevTokenVerifier` does a plain string
comparison — no signature, no expiry, no revocation — and maps every caller to
one constant `DEV_USER_ID`. `apps/mobile/src/api/bridge.ts` defines `DEV_TOKEN`
as `import.meta.env.VITE_DEV_AUTH_TOKEN ?? "local-dev-token"`, and the variable
was unset at build time, so the guessable fallback is compiled into the shipped
bundle — confirmed present in the built `apps/mobile/dist` asset. Every
`where user_id = $1` predicate was comparing against one hardcoded constant, so
user scoping was decorative. Fixed by ADR-0039.

**C2 — CRITICAL — Hardcoded credential encryption key reachable by fallback.**
`Config::resolved_encryption_key` returned a constant that appears in public
source whenever `CREDENTIAL_ENCRYPTION_KEY` was unset **or failed to parse**.
That key seals the OAuth `state` validated by the *unauthenticated*
`/v1/auth/google/callback` route and encrypts Google refresh tokens at rest
(ADR-0025), so the fallback lets a `state` naming any `user_id` be forged.
Silently accepting a malformed value was the sharper edge: the operator sets the
variable, sees a running server, and believes it took effect. Fixed by ADR-0039.

**H1 — HIGH — Google account revocation was inoperative.** Found independently
by two audit passes. `0006` constrained `connected_accounts.status` to
`('active','expired','revoked')`, but `disconnect_account` writes
`'disconnected'` and the two refresh-failure paths write `'error'`. Every
disconnect returned `23514`, and both refresh paths discarded the result with
`let _ =`, so they failed silently. An account the user had revoked stayed
`'active'`, its credentials kept being used, and its scopes kept entering the
`Principal`. Fixed by migration `0012` plus loud logging — **but see section 6:
that migration has not been applied anywhere.**

**H2 — HIGH — `?access_token=` accepted on every route.** `auth.rs` read the
query credential unconditionally while its own doc comment claimed it was
"accepted only on the WebSocket upgrade path". A credential in a URL reaches
proxy access logs, `Referer` headers and browser history. Now requires an
`Upgrade: websocket` header.

**M1 — MEDIUM — Model-supplied ids interpolated into Google URL paths raw.**
Gmail `message_id` and Calendar `event_id` reach these URLs from model output
and were interpolated unencoded, while every query parameter in the same file
was encoded. A crafted id could add path segments and aim the caller's OAuth
bearer at an endpoint the tool never declared. `url_encode_path` now encodes
them.

**M2 — MEDIUM — Reminder could be attached to another user's task.**
`reminders.task_id` was bound from client input and its foreign key references
`tasks(id)` with no owner predicate. The accept/reject difference was an oracle
for which task ids exist. `verify_task_ownership` now proves ownership on create
and update; "not yours" and "does not exist" both answer `NotFound`.

**M3 — MEDIUM — Readiness did not ask the database.** `is_healthy` was
`db.is_some()`, proving only that a pool was constructed at boot. A database
that fell over afterwards still reported healthy. Now `is_ready` runs
`select 1` with a 2s timeout, and a new public `/v1/ready` answers 200/503.

**M4 — MEDIUM — Database outage reported as 500 `internal`.** That tells a
client "this server is broken" when the truth is "a dependency is down, retry
may succeed". Now `AppError::DependencyUnavailable` → 503
`dependency_unavailable`.

### Open — not fixed in this milestone

**H3 — HIGH — WebSocket voice path bypasses the voice rate limiter.** The WS
path calls Cartesia STT and TTS directly and never touches
`state.voice_rate_limiter`; the 30/min cap guards only `/v1/voice/*`. One socket
can bill unbounded paid provider calls.

**H4 — HIGH — Undisclosed provider substitution.** On any Cartesia TTS failure —
and unconditionally when `cartesia_api_key` is `None` — the assistant's reply
text is sent to `translate.google.com/translate_tts`. On any non-2xx Cartesia
STT status, the provider reads `OPENAI_API_KEY` from the environment directly
and POSTs the user's audio to OpenAI. In both cases a third party the deployment
never configured for that role receives user content, and nothing in the
response indicates a different provider was used.

**H5 — MEDIUM/HIGH — `Principal.scopes` is a union across all of a user's
accounts.** The policy gate is therefore account-blind: a tool call targeting
account B passes `RiskBasedPolicy` on account A's scopes. No data crosses,
because `get_access_token_with_required_scope` re-checks the target account's
own scopes — but the gate itself is bypassable, and defence-in-depth is the
point of having a gate.

**H6 — HIGH (latent) — Planning tools trust a model-supplied `_user_id`.** The
five tools in `planning_tools.rs` implement only `execute`, so the default trait
impl drops the authenticated user and `user_id(args)` reads `_user_id` straight
out of model-controlled arguments. Not exploitable today: `main.rs` registers no
planning tool. It becomes a cross-user read the moment one is registered, and
argument schema validation is still "later work" (`executor.rs`).

**H7 — HIGH — Unbounded, blocking PDF extraction.** No page cap, no time limit,
no `spawn_blocking`; it runs inline in the axum handler. A 25 MiB
object-stream-bomb stalls a tokio worker and writes unbounded `document_pages`
rows.

**M5 — MEDIUM — WebSocket frame and message sizes are library defaults**
(64 MiB message, 16 MiB frame). A 64 MiB text frame is fully buffered per socket
before the 14 MB audio check rejects it. No per-connection or per-user WS rate
limit. `ClientFrame::UserText` length is never checked.

**M6 — MEDIUM — No Origin check on the WebSocket upgrade,** and `CorsLayer` does
not apply to the handshake. Exploitation still needs the token, so this is
depth, not immediate takeover.

**M7 — MEDIUM — `/v1/audio/transcribe` extracts no `Principal`,** applies no
rate limit and gets no body-limit override, so metered provider calls are
unattributed and unbounded per user.

**M8 — MEDIUM — Gmail search silently drops messages.** The per-message metadata
fetch matches `Ok(r) if success => r, _ => continue`, so a 401 or rate-limit
mid-loop yields a short or empty list reported as success.

**M9 — MEDIUM — Model-visible account facts, no per-turn account pinning.**
Every connected `account_id` and its scopes are injected into model-visible
facts, and `account_id` is an ordinary tool argument. Text injected via an email
or Drive document read in-turn can steer the model to a different account of the
same user; `calendar.create`/`update` are Yellow, so no approval gates the
write.

**M10 — MEDIUM — No idempotency on calendar create/update or memory insert.** No
client-generated id, no dedupe constraint. A retry after a timeout duplicates
the event or the memory. (Classroom sync, task and reminder create *are*
idempotent — see section 5.)

**M11 — MEDIUM — Uploaded MIME is taken verbatim from `Content-Type`,** never
sniffed, and `text/html` is on the allow-list. Whether this is exploitable
depends on whether any surface renders page content unescaped — NOT VERIFIED.

**M12 — MEDIUM — Mobile has no logout or account switch at all,** and no cache
key is namespaced by user. See section 8.

**L1 — LOW — `exchange_oauth` accepts a code plus an attacker-chosen
`redirect_uri` with no `state` check** and binds the result to the caller's
principal — the classic OAuth code-injection shape. The state-checked path is
`oauth_callback`.

**L2 — LOW — `AppError` still has five arms.** Authorization, conflict,
rate-limit, timeout, provider-failure and cancellation remain
indistinguishable on non-voice HTTP routes. M4 added the dependency case; the
rest of §32 of the brief is unmet.

**L3 — LOW — Unclamped caller-supplied `limit`** on academic/Drive routes is
forwarded to the upstream provider; productivity list routes have no
`limit`/`offset` at all.

**L4 — LOW — Orphan risk in `0002`:** `principal_id` on `tool_executions`,
`approval_requests` and `audit_events` has no foreign key to `users`, so
deleting a user leaves them dangling. `0011`'s `planning_preferences` has no FK
either, and its `auth.uid()` RLS policy is dead code under the ADR-0023 model.

**L5 — LOW — Flaky rate-limit test. FIXED.**
`voice_security::repeated_requests_are_rate_limited_per_principal` drove 40
requests *sequentially* against a 30-per-60s window, and each of the first 30 was
allowed through to a real TTS call — which, with no Cartesia key configured,
means a live request to Google's translate endpoint. Under full-suite parallel
load those 30 took longer than the window, so it reopened before the 31st
arrived and the limit never tripped. The test failed for a reason unrelated to
what it tested. Observed failing at 173s, passing isolated at 15s. Now sent
concurrently, which bounds wall time by the slowest single request rather than
the sum of thirty: three consecutive runs at 10.9s, 1.3s and 0.97s, all passing.
Test-only change; no production behaviour was altered.

**L6 — LOW — Shared `reqwest::Client` sets only `connect_timeout`** — no total
or read timeout, default 10-redirect policy. A slow or redirect-looping upstream
holds a request task open indefinitely.

**ENVIRONMENT — Production is running with no database.** `/v1/health` returns
`degraded`. Every persistence route returns 503. This is honest failure, not
fabricated success, but the deployed assistant cannot store anything.

## 4. Fixes applied

Three commits on `claude/m13-system-hardening`:

1. `fix(auth): refuse to start on a development security default` — ADR-0039.
   `Config::validate_security` is a pure function called from `from_env`, so a
   configuration that would reach `DevTokenVerifier` or the fallback encryption
   key aborts startup before the listener binds. Both now require
   `ASSISTANT_ALLOW_DEV_AUTH=true`; a malformed key is refused even under the
   opt-in. 6 regression tests.
2. `fix(server): make readiness ask the database, and report an outage as 503` —
   M3, M4, plus a configured-but-unreachable `DATABASE_URL` now logging at
   `error!` rather than `info!`. 5 regression tests.
3. `fix(security): restrict the query credential, encode Google path ids, repair
   account revocation` — H1, H2, M1, M2, with migration `0012`. 8 regression
   tests.

## 5. Verification actually performed

| Check | Result |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo test --workspace` | **367 passed, 0 failed, 2 ignored** (348 at the M12 baseline) |
| `pnpm --dir apps/mobile typecheck` | exit 0 |
| `pnpm --dir apps/mobile lint` | exit 0 |
| `pnpm --dir apps/mobile build` | exit 0 |

Caveat: the database-backed integration tests skip rather than fail when
`DATABASE_URL` is absent, and `cargo test` captures the message saying so. A
passing suite is **not** evidence that the cross-user isolation, durable-action
or memory tests executed. Re-run with `DATABASE_URL` set before trusting them.

### Properties positively confirmed

- **Ownership.** `Principal` comes only from `Extension<Principal>`; no handler
  reads a user id from a body, query or path. Every by-id read, update and
  delete on tasks, notes, ideas, projects, labels, reminders, memories,
  documents, document pages, conversations and classroom data carries
  `and user_id = $n`, and zero rows maps to `NotFound`. Client-supplied ids on
  create are guarded by `on conflict (id) do update ... where user_id =
  excluded.user_id`.
- **Google token load** is on `(account_id, user_id)` together —
  `WHERE id = $1 AND user_id = $2 AND provider = 'google'`. There is no
  `account_id`-alone load anywhere.
- **Effective scopes at execution** are read from the target account's own row.
- **Approvals.** No double-approval race: `claim_approval` is one transaction —
  `select ... for update`, terminal-state check, expiry check, conditional
  update — and resume re-resolves the spec from the live registry and re-runs
  `executor.decide`. TTL 15 minutes. The client sends only an approval id.
- **Invariants.** Risk level is read only from `spec.risk`;
  `PermissionPolicy::evaluate` takes only `ToolSpec` + `Principal`;
  `max_tool_rounds` is server config only; audit `summarize` records argument
  keys, never values. Risk, scopes, principal and user id are never deserialized
  from tool arguments or request bodies. JWT claims never grant scopes.
- **Idempotency where it exists.** Classroom sync uses
  `on conflict (user_id, account_id, external_id) do update`; coursework-to-task
  uses `do nothing`; task and reminder create accept a client id with an owner
  guard.
- **No fabricated success.** No path produces a canned assistant reply presented
  as inference. Missing STT returns `stt_unconfigured`; missing TTS returns
  `tts_unconfigured`; an empty answer is not spoken; a socket close with no text
  rejects rather than inventing one. The `FakeSpeechToTextProvider` that returns
  a canned utterance has no server construction site.
- **No SSRF surface.** No tool exposes a URL argument to the model; no fetch
  takes its host from a request body, document content or a redirect. Every
  outbound host is hardcoded or operator-configured.
- **Memory.** `looks_like_secret` covers 17 prefixes plus an entropy rule and is
  on *every* write path; a hit returns 400 and persists nothing. There is no
  vector search to leak across users — search is `tsvector` + `ilike`, scoped by
  `user_id`. Context injection is capped at 8 memories. The model cannot write
  memory: the only non-REST write path parses raw user text.
- **Documents.** The client filename is never used to build a path; the path is
  `root/user_uuid/doc_uuid` and the storage key is server-generated;
  `parse_local_key` rejects `..` and `/`. 25 MiB cap with a body-limit backstop.
  Every route re-verifies ownership before touching the object store.
- **No content in logs.** Zero `tracing::` calls in the memory crate, memory
  store, memory routes or context provider. Document logging carries ids only.
  `TraceLayer` records `uri.path()` only, keeping `?access_token=` out of logs.
- **Offline honesty.** Queued writes are marked `pending: true` and
  dead-lettered creates are removed from cache — labelled queued, not fabricated
  success.
- **WS turn state** is a per-connection local, not a shared map: no cross-user
  socket state. Malformed JSON answers `bad_frame` and keeps the socket open.
  Disconnect cancels the turn.
- **Secrets.** No `.env`, key or certificate is tracked by git. The built mobile
  bundle contains no JWT, API key, OAuth client id or client secret — only the
  dev token of C1 and the server URL. `PROTOCOL_VERSION` is 10 in the Rust
  crate, 10 in `types.ts`, and 10 as reported by the live server.
- **RLS** is enabled on every table across `0001`–`0011`, and `0005`'s
  `alter default privileges ... revoke` covers tables created later (ADR-0023).

## 6. Operational steps NOT performed

These require action on the deployed service and were deliberately not taken:

1. **Rotate `DEV_AUTH_TOKEN`** — the current value is public.
2. Set `SUPABASE_PROJECT_REF` on Render, so real JWT verification is used.
3. Set `CREDENTIAL_ENCRYPTION_KEY` to a real 64-hex-character value.
4. Re-encrypt or re-consent any Google credential stored under the fallback key.
5. Rebuild and reship the mobile app so no bearer token is inlined at all.
6. Set `DATABASE_URL` so the service stops running without persistence.
7. **Apply migration `0012` with `scripts/migrate.ps1`.** Until it runs, H1
   remains live in that environment: account disconnection still fails.

After steps 2 and 3 the server will refuse to start until both are present.
That is the intended behaviour, and it means **the next deploy of this branch
will not boot until they are set.** `render.yaml`'s `healthCheckPath` is
deliberately left at `/v1/health` rather than `/v1/ready`, so a database outage
does not by itself fail a deploy.

## 6b. Deployed environment audit (Render)

Read from the service's own boot logs, which print the resolved `Config` with
secrets redacted, plus live probes. Service `assistant-server`
(`srv-daepksf40ujc7387pg60`), region singapore, free plan, **autoDeploy on
`main`**, `healthCheckPath: /v1/health`.

| Variable | State at audit | Verdict |
|---|---|---|
| `SUPABASE_PROJECT_REF` | `None` on all three boots | caused C1 |
| `DATABASE_URL` | set, connection failing | no persistence |
| `CREDENTIAL_ENCRYPTION_KEY` | `Some` | set, so C2's fallback was **not** active in production |
| `GOOGLE_REDIRECT_URI` | `.../api/google/oauth/callback` | 404s; no such route |
| `OPENAI_API_KEY` | `None` | fine, Gemini is the configured provider |
| `ASSISTANT_ALLOWED_ORIGINS` | localhost + tauri schemes | fine |
| `DEV_AUTH_TOKEN` | was `local-dev-token` | public value; rotated |

**C2 correction.** `credential_encryption_key` is `Some` in every boot log, so
the hardcoded fallback key was not in use in production. The defect in the code
was real and is fixed, but the deployed blast radius was smaller than first
reported. Whether the configured value actually *parses* cannot be told from a
redacted log: if it were malformed, the old code would have silently used the
fallback anyway. After ADR-0039 a malformed value refuses to boot, which settles
the question on the next deploy.

**The database is misconfigured, and failed two different ways within two
hours** — the value was changed between boots:

    13:08  prepared statement "sqlx_s_1" already exists
    14:38  Network is unreachable (os error 101)

These are the two standard Supabase-on-Render failures. The first is the
transaction pooler on port 6543: PgBouncer in transaction mode cannot serve
sqlx's prepared statements. The second is the direct connection
(`db.<ref>.supabase.co:5432`), which is IPv6-only, against a Render free
instance that has no IPv6 egress. The working configuration is the **session
pooler** — `aws-1-<region>.pooler.supabase.com:5432`, user
`postgres.<project_ref>` — which is reachable over IPv4 and, being session mode,
supports prepared statements. There are no Render Postgres instances in the
account, so Supabase is the only candidate target.

**`GOOGLE_REDIRECT_URI` pointed at a route that does not exist.** Live check:
`/api/google/oauth/callback` returns 404, `/v1/auth/google/callback` returns 200.
Google account connection could not have completed. Corrected on the service;
the same URI must also be updated in the Google Cloud console's authorised
redirect URIs, which is not something this repository controls.

**Actions taken on the deployed service during M13:** `DEV_AUTH_TOKEN` rotated
to a 64-character random value, `ASSISTANT_ALLOW_DEV_AUTH=true` set explicitly,
`GOOGLE_REDIRECT_URI` corrected. Verified afterwards: `local-dev-token` now
returns 401 where it previously returned 200. The bypass is closed.

**Deliberate choice recorded:** production stays on the development verifier for
now, because the mobile app contains no sign-in code at all — switching on
Supabase JWT verification would leave the app unable to authenticate. Dev auth
with a rotated, non-guessable secret closes the exposure without bricking the
client. It does **not** restore per-user identity: every caller is still one
fixed user. Real authentication needs a login screen, and that is a milestone,
not a configuration change.

## 6c. Production remediation and verification (completed)

The deployed service was brought to a working, non-bypassable state and each
step verified against it rather than assumed.

**Changes made to `srv-daepksf40ujc7387pg60`:**

| Change | Evidence it took effect |
|---|---|
| `DEV_AUTH_TOKEN` rotated to a 64-character random value | `local-dev-token` -> **401** (was 200) |
| `ASSISTANT_ALLOW_DEV_AUTH=true` set explicitly | boot log now prints `allow_dev_auth: true` |
| `GOOGLE_REDIRECT_URI` corrected to `/v1/auth/google/callback` | boot log shows the new value; the old path returned 404 |
| `DATABASE_URL` repointed at the Supabase **session pooler** (`aws-0-ap-southeast-1.pooler.supabase.com:5432`) | boot log: `connected to postgres` |
| Migration `0012` applied with `sqlx migrate run` | `sqlx migrate info` reports `12/installed` |

**Remote schema verified directly**, closing the largest item that the previous
revision of this document listed as unverifiable from the repository. Queried
through the project's own Supabase instance (`uiumoirlfkkucoqpaura`):

- `public._sqlx_migrations` held 11 rows, versions 1-11, all `success = true`,
  matching `migrations/` exactly. No drift, nothing missing, nothing applied
  out of band.
- All 24 expected tables exist, and `rls_enabled` is true on every one of them,
  which is what ADR-0023 requires.
- Before `0012`, the live constraint read
  `CHECK (status = ANY (ARRAY['active','expired','revoked']))` — H1 confirmed at
  runtime in production, not merely inferred from source. After `0012` it reads
  `CHECK (status = ANY (ARRAY['active','expired','revoked','disconnected','error']))`.
  Google account disconnection works from this point on.
- `connected_accounts` holds 3 rows, all `active`, all with scopes.

**Production vertical slice — the part that could be run:**

| Step | Result |
|---|---|
| `/v1/health` | `{"status":"ok"}` (was `degraded`) |
| `/v1/ready` (new) | 200 — the readiness probe genuinely reaches the database |
| Authenticated read: tasks, memories, documents, google accounts, planning | all 200 with real rows |
| Durable low-risk write | task created, read back, deleted (204). Test row removed. |
| H2 fix live | `/v1/tasks?access_token=<token>` -> **401**; same token in the header -> 200 |
| M4 fix live | before the database came up, `/v1/tasks` returned `503 dependency_unavailable` rather than a 500 |

**What this does not establish.** Every row above is owned by
`deadbeef-0000-4000-8000-000000000001` — `DevTokenVerifier::DEV_USER_ID`. The
server is reachable only with a strong secret now, but it still has exactly one
user, so none of this exercises per-user authorization in production. That
requires real authentication, which requires a sign-in screen in the app.

Two operational items remain outstanding and are **not** closed by the above:
the Supabase database password was transmitted in plaintext during this session
and should be rotated, and the `GOOGLE_REDIRECT_URI` correction must be mirrored
in the Google Cloud console's authorised redirect URIs before account
connection will succeed.

## 7. NOT VERIFIED

- ~~**Remote database schema.**~~ **RESOLVED** — see section 6c. Migrations
  1-12 are confirmed applied, all 24 tables exist, RLS is on for every one.
- **Backups.** No backup configuration exists anywhere in the repository.
  Whether the hosting provider takes any is undocumented and unverified. No
  restore test was performed. Disaster recovery must not be claimed to exist.
- **Physical Android regression.** No device test was performed. App launch,
  authentication, voice, tool call, Google read, task/reminder, memory,
  document, planning, account switch, offline, reconnect and app restart are all
  **NOT VERIFIED**.
- **Production vertical slice.** Partially run — see section 6c. Authenticated
  read, durable low-risk write and persistence are verified end to end against
  the production server and database. Still NOT VERIFIED: the authenticated
  voice request, the approval-required operation, and anything requiring the
  Android client.
- **Whether the mobile app renders document page content as HTML** — decides the
  real severity of M11.
- **Runtime behaviour of the PDF bomb (H7)** — argued from absent bounds in
  code, not measured.
- **The `0012` constraint failures** were proven from schema versus SQL text,
  not observed at runtime.
- **Failure-injection matrix.** Provider and database failures were traced
  through code, not injected.
- **Concurrent-write and transaction-failure testing** against a real database.
- **Dependency advisory scan.** No `cargo audit` was run.

## 8. Mobile user-switch isolation

**Verdict: LEAKS — but vacuously, because there is no user switch to perform.**

There is no logout, login or account-switch anywhere in `apps/mobile/src` or
`src-tauri/src`. Persistence sites are `localStorage` keys
`assistant_cache_{tasks,reminders,notes,ideas,projects,labels}`,
`assistant_outbox`, `assistant_outbox_dead`, legacy `assistant_local_*`,
`ASSISTANT_SERVER_URL`, and a Tauri SQLite `assistant-cache.sqlite`. No key is
namespaced by user id and nothing clears any cache, so once C1 is fixed and real
per-user sign-in exists, **switching users on one device would show the previous
user's cached data**. This must be closed before multi-user use.

Socket teardown is correct: `Connection::open` closes first, and the server
cancels the turn on disconnect.

ADR-0008 note: no `fetch` or `new WebSocket` appears in any component or screen.
But 40+ `fetch` calls live in `apps/mobile/src/api/*.ts`, bypassing the Tauri
bridge that `bridge.ts` describes as the only outward path. Whether that
satisfies ADR-0008 is a judgement call the ADR does not currently settle.

## 9. Gate criteria

Authentication: fixed in code, **unverified in deployment**. Authorization,
approvals, model trust boundary, SSRF, memory and document ownership: verified
and holding. Everything else is open or NOT VERIFIED.

**M13 remains RED.** It must not be treated as passed. No wake word, PC agent,
autonomous computer access or Claude Code control should be built on top of it
until at minimum: the operational steps in section 6 are complete, H3–H7 are
closed, and the physical-device and production-slice verification in section 7
has actually been run.
