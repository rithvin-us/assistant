# Milestone 13 — System Hardening and Production Readiness

**Status: RED — partial audit.**

M13 is a gate before system-wide wake word, autonomous computer access, a PC
agent, or Claude Code control. It is not green. One critical authentication
defect was confirmed against the live deployment and fixed in code; most gate
criteria were not evaluated in this pass and are marked NOT VERIFIED below.

Nothing here is asserted without evidence. Where a check was not performed, it
says so rather than assuming a result.

---

## 1. Repository state

The audit began on `main` at `166f85f`, with both M12 branches
(`claude/m12-voice-completion-reliability`, `fix/m12-voice-pipeline-tool-turns`)
already merged into it — confirmed with `git branch --merged main`.

M12 advanced during the audit. The working checkout moved to
`claude/m12-finalize-voice` at `c699262` ("populate principal scopes from
connected Google accounts and enforce multi-account isolation"). Both defects
found at `166f85f` were re-confirmed present at `c699262`.

M13 work is therefore isolated in a git worktree on
`claude/m13-system-hardening`, rebased onto `c699262`. The primary checkout was
not switched, so parallel M12 work was not disturbed, and the M12 branch was not
modified. When M12 is finalised this branch must be rebased onto the final M12
commit and every verification below re-run.

## 2. M12 baseline

`c699262`. Treated as a moving snapshot, not a final state.

## 3. Findings by severity

### CRITICAL — Production authentication bypass (fixed in code; operational steps outstanding)

The deployed service accepted the bearer token `local-dev-token`. Confirmed with
two read-only requests:

    Bearer local-dev-token   -> 200  []
    Bearer bogus-token-xyz   -> 401  {"code":"unauthorized",...}

Chain:

1. `render.yaml` provisions `DEV_AUTH_TOKEN` and never sets
   `SUPABASE_PROJECT_REF`.
2. `services/assistant-server/src/lib.rs` selected `DevTokenVerifier` whenever
   `supabase_project_ref` was `None`, logging only a `warn!`. Fail-open.
3. `DevTokenVerifier` does a plain string comparison — no signature, no expiry,
   no revocation — and maps every caller to one constant `DEV_USER_ID`.
4. `apps/mobile/src/api/bridge.ts` defines `DEV_TOKEN` as
   `import.meta.env.VITE_DEV_AUTH_TOKEN ?? "local-dev-token"`. The variable was
   unset at build time, so the guessable fallback literal is compiled into the
   shipped bundle — confirmed present in the built `apps/mobile/dist` asset.

Impact: anyone who guessed or extracted a 15-character string had full API
access as the sole account. Every `where user_id = $1` predicate was comparing
against one hardcoded constant, so user scoping was decorative. This is the
condition ADR-0024 was written to end, and it violates the CLAUDE.md rule that
no secret lives in `apps/mobile`.

### CRITICAL — Hardcoded credential encryption key reachable by fallback (fixed in code)

`Config::resolved_encryption_key` returned a literal constant that appears in
public source whenever `CREDENTIAL_ENCRYPTION_KEY` was unset **or failed to
parse**. `render.yaml` marks that variable `sync: false`, so it is unset until
an operator fills it in.

That key seals the OAuth `state` parameter validated by the *unauthenticated*
`/v1/auth/google/callback` route, and encrypts Google refresh tokens at rest
(ADR-0025). With the fallback active, a `state` naming any `user_id` can be
forged, and stored refresh tokens are encrypted under a public constant.

Silently accepting a malformed value was the sharper edge: the operator sets the
variable, sees a running server, and believes it took effect.

### HIGH — ENVIRONMENT — Production is running with no database

`GET /v1/health` returns `{"status":"degraded",...}`. `AppState::is_healthy` is
`self.db.is_some()`, and `main.rs` turns an absent or unreachable `DATABASE_URL`
into `None` and boots anyway, logging at `info!` — the comment says "log
loudly", the code does not.

Consequence: every persistence route returns 500 in production. Tasks,
reminders, notes, memories, documents, conversations, approvals and durable
actions do not work on the deployed service.

This is not a fabricated-success defect: the missing pool maps to
`AppError::Internal("database unavailable")`, so the failure is honest. Not
fixed in this pass.

### MEDIUM — Health endpoint has no readiness semantics

`is_healthy` proves only that a pool object was constructed at boot. A database
that becomes unreachable afterwards still reports healthy. There is no
liveness/readiness split and no dependency probe. Not fixed in this pass.

### MEDIUM — Database unavailability is mapped to 500, not 503

`AppError::Internal` classifies a dependency outage as an internal error, so a
client cannot distinguish "this server is broken" from "a dependency is down and
a retry may succeed". Not fixed in this pass.

## 4. Fixes applied

One commit on `claude/m13-system-hardening`, recorded as ADR-0039.

- `Config::validate_security(supabase_project_ref, credential_encryption_key,
  allow_dev_auth)` is a pure function called from `from_env`, so a bad
  configuration aborts startup before the listener binds.
- Both development defaults now require `ASSISTANT_ALLOW_DEV_AUTH=true`.
- A malformed `CREDENTIAL_ENCRYPTION_KEY` is refused even under the opt-in.
- `parse_encryption_key` is the single definition of a valid key, so startup
  validation and key resolution cannot disagree.
- `.env.example` documents the opt-in; the four integration harnesses set
  `allow_dev_auth: true` explicitly.

It is pure specifically so it is unit-testable without mutating process-wide
environment variables, which the existing tests avoid because they run
concurrently in one process.

Six regression tests in `config::tests`, all passing:
`a_missing_supabase_project_is_refused_without_the_opt_in`,
`a_missing_encryption_key_is_refused_without_the_opt_in`,
`a_malformed_encryption_key_is_refused_even_with_the_opt_in`,
`the_opt_in_permits_the_development_defaults`,
`a_fully_configured_deployment_passes`,
`both_key_encodings_parse_and_hex_wins_on_length`.

### Operational steps NOT performed by this commit

These require action on the deployed service and were deliberately not taken:

1. Rotate `DEV_AUTH_TOKEN` — the current value is public.
2. Set `SUPABASE_PROJECT_REF` on Render, so real JWT verification is used.
3. Set `CREDENTIAL_ENCRYPTION_KEY` to a real 64-hex-character value.
4. Re-encrypt or re-consent any Google credential stored under the fallback key.
5. Rebuild and reship the mobile app so no bearer token is inlined at all.
6. Set `DATABASE_URL` so the service stops running without persistence.

After steps 2 and 3 the server will refuse to start until both are present. That
is the intended behaviour.

## 5. Verification actually performed

| Check | Command | Result |
|---|---|---|
| Formatting | `cargo fmt --all --check` | exit 0 |
| Lints | `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| Tests | `cargo test --workspace` | 354 passed, 0 failed, 2 ignored (348 before the 6 new tests) |
| Mobile types | `pnpm --dir apps/mobile typecheck` | exit 0 |
| Mobile lints | `pnpm --dir apps/mobile lint` | exit 0 |
| Mobile build | `pnpm --dir apps/mobile build` | exit 0 |

Caveat on the test count: the database-backed integration tests skip rather than
fail when `DATABASE_URL` is absent, and `cargo test` captures the message that
says so. The suite passing is therefore **not** evidence that the cross-user
isolation, durable-action or memory tests executed. Re-run with `DATABASE_URL`
set before trusting them.

### Other properties positively confirmed

- Unauthenticated REST and WebSocket requests are refused; the socket is
  authenticated before upgrade, asserted as HTTP 401 on the handshake.
- The Supabase JWT path checks signature, issuer, audience and a required
  `exp`; `alg: none` and unknown-`kid` tokens are rejected; JWKS failures fail
  closed; JWKS refetch is rate-limited. Ten unit tests in
  `crates/assistant-auth/src/supabase.rs`.
- Token claims never grant scopes — `verify` returns an empty scope vector
  unconditionally, asserted by `claims_never_grant_scopes`. ADR-0005 holds at
  this seam.
- `PROTOCOL_VERSION` is 10 in `crates/assistant-protocol/src/lib.rs`, 10 in
  `apps/mobile/src/api/types.ts`, and 10 as reported by the live server.
- The built mobile bundle contains no JWT, API key, OAuth client id or client
  secret — only the dev bearer token described above, and the server URL.
- No `.env`, key, certificate or credential file is tracked by git; `.gitignore`
  covers `.env` and `.env.*` with an `!.env.example` exception.
- RLS is enabled on every table across migrations 0001 to 0011, and the
  `alter default privileges ... revoke` in 0005 covers tables created later
  (ADR-0023).
- Requests to persistence routes without a database return an honest error
  rather than a fabricated empty success.

## 6. NOT VERIFIED

None of the following was evaluated. They are gate criteria and remain open.

- **Authorization / ownership** — whether every user-owned resource query scopes
  by the authenticated principal, and whether any by-id fetch is an IDOR.
- **Google account isolation** — whether an access token is loaded on
  `(user_id, account_id)` together or on `account_id` alone.
- **Scope propagation** — `c699262` claims to fix this; the claim was not
  independently verified.
- **Tool registry, risk levels, approvals, double-approval races, and
  idempotency of consequential writes.**
- **Model trust boundary** beyond the token-scopes seam noted above.
- **SSRF** on server-side URL fetches.
- **Document security** — upload limits, MIME validation, path traversal,
  decompression bombs.
- **Memory security** — secret detection, cross-user vector search scoping.
- **Voice and WebSocket** — frame limits, stale turn ids, cross-user socket
  state, provider failure honesty.
- **Rate limits and request limits** beyond the per-principal voice limiter.
- **Provider and database failure-injection matrix.**
- **Server restart and recovery** of pending approvals and durable actions.
- **Mobile cache isolation across a user switch** — the most important untested
  item after authentication.
- **Offline honesty.**
- **Remote database schema** — cannot be confirmed from the repository. The
  applied migration list must be read from the production database directly.
- **Backups** — no backup configuration exists in the repository. Whether the
  hosting provider takes any is undocumented and unverified.
- **Physical Android regression** — no device test was performed.
- **Production vertical slice** — only unauthenticated probes and one
  authenticated read were performed, against a server with no database.

## 7. Gate criteria

Authentication: fixed in code, unverified in deployment. Everything else in the
M13 green criteria list is either NOT VERIFIED or RED.

**M13 remains RED.** It must not be treated as passed, and no wake word, PC
agent, autonomous computer access or Claude Code control should be built on top
of it until the operational steps in section 4 are complete and the NOT VERIFIED
list in section 6 has been worked through.
