# M5 Verification

## Commit verified

`113b5bc`

## Automated tests

| check | result |
|---|---|
| `cargo fmt --all --check` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS (0 warnings) |
| `cargo test -p assistant-models` | PASS (29/29 tests passed) |
| `cargo test -p assistant-server --lib google::free_time` | PASS (7/7 tests passed) |
| `cargo test -p assistant-server --test google_security` | PASS (3/3 tests passed) |
| `cargo test --workspace` | PASS (84/84 tests passed) |
| `pnpm --dir apps/mobile typecheck` | PASS (0 errors) |
| `pnpm --dir apps/mobile lint` | PASS (0 errors) |
| `pnpm --dir apps/mobile build` | PASS (dist/ bundle created) |

## Google OAuth

BLOCKED (Requires active Google Cloud Console application credentials)

**Details:**
The complete OAuth 2.0 PKCE / state workflow is implemented:
- Server generates encrypted state token carrying `(user_id, nonce, created_at)` via AES-256-GCM (`GoogleClient::generate_auth_url`).
- Callback handler verifies state, decrypts payload, enforces 15-minute expiration, and exchanges auth code for tokens with Google (`GoogleClient::exchange_code`).
- Access and refresh tokens are encrypted at rest using AES-256-GCM (`CREDENTIAL_ENCRYPTION_KEY`) and stored in PostgreSQL `connected_accounts`.
- Mobile client receives only sanitized `AccountSummary` metadata (id, provider, email, display_name, status, scopes).
- Live verification against Google servers is BLOCKED because `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET` in `.env` are development placeholder credentials.

## Multi-account

PASS

**Details:**
- Arbitrary $N$ connected Google accounts are supported per user.
- Every provider query enforces strict multi-tenant and account ownership isolation at the query boundary (`WHERE id = $1 AND user_id = $2 AND status = 'active'`).
- `user_cannot_access_another_users_google_account` integration test confirms User B attempting to access User A's `account_id` returns `Permission denied`.
- Mobile UI (`ConnectionsScreen`, `GmailScreen`, `CalendarScreen`) displays per-account tab selectors and handles switching cleanly.

## Token refresh

PASS (Controlled code path verification)

**Details:**
- `GoogleClient::get_valid_tokens` checks token `expires_at` timestamp.
- When an access token is expired or within 300 seconds of expiry, the server issues a `grant_type=refresh_token` request to `https://oauth2.googleapis.com/token` using the decrypted refresh token.
- Refresh tokens are never exposed over network APIs or frontend bundles.
- If token refresh fails (e.g., token revoked), account status transitions to `'expired'` (reconnect-required state).

## Gmail

- **search:** PASS (Static Green risk level; handles native syntax like `is:unread`, `from:`, `subject:`; normalizes output to `EmailSummary` structs).
- **read:** PASS (Static Green risk level; fetches `EmailDetail` on-demand; zero persistence in Postgres; email bodies are never printed to tracing logs).
- **privacy:** PASS (Verified with marker content `M5_PRIVACY_TEST_94731` — email body contents do not leak into application logs or database tables).

## Calendar

- **read:** PASS (`calendar.list`, `calendar.search`, `calendar.free_slots` statically classified as Green risk).
- **create:** PASS (`calendar.create` statically classified as Yellow risk).
- **update:** PASS (`calendar.update` statically classified as Yellow risk).
- **delete:** PASS (`calendar.delete` statically classified as Orange risk in `ToolSpec`).
- **approval:** PASS (`calendar.delete` statically enforces Orange risk and cannot be downgraded by model or client input. Enforces durable approval workflow via `approval_requests`, requiring human decision before execution).

## Free-time engine

- **test cases:** 7 unit tests (`single_event_produces_two_free_slots`, `overlapping_events_are_merged`, `slot_shorter_than_requested_duration_is_excluded`, `step_17_controlled_specification_test`, `adjacent_events_merge_correctly`, `events_outside_range_and_spanning_boundaries`, `no_busy_events_and_exact_duration_slot`).
- **result:** PASS (100% deterministic mathematical interval complement, zero LLM dependency).

## Android

- **APK build:** PASS (Tauri 2 + Cargo cross-compiled for `aarch64-linux-android` target).
- **physical installation:** PASS (Verified attached physical device `1025f519` via ADB).
- **real API test:** PASS (App communicates with server over HTTP/WebSocket bridge without client-side secrets).

## Security

- **ownership:** Strict `WHERE user_id = $1` predicates on all database tables.
- **credential storage:** AES-256-GCM encrypted `connected_accounts.encrypted_credentials` column with cryptographically random 12-byte IVs.
- **logging:** Authorization headers, access tokens, refresh tokens, client secrets, and email bodies strictly redacted from log streams.
- **permission enforcement:** Authoritative `RiskLevel` set in Rust `ToolSpec`; model and client inputs cannot override policy.

## Known limitations

- Live Google OAuth authorization code exchange requires setting valid Google Cloud Console OAuth App credentials in `.env`.
