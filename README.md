<p align="center">
  <img src="logo.png" alt="Personal Assistant Logo" width="160" />
</p>

# personal-assistant

A voice-first personal assistant. Rust backend, Tauri 2 + React mobile app, PostgreSQL.

**Status: Milestone 0 — repository bootstrap.** There is no AI in this
repository. No model provider, no Gmail, no Calendar, no memory engine, no voice.
What exists is the structure those things will be built into, plus a server and
an app that actually run and actually talk to each other.

## What works today

- Rust workspace of 7 crates, building clean under `clippy -D warnings`.
- Axum server with `GET /v1/health` and `WS /v1/conversation/{id}/stream`.
- Tauri 2 desktop app with a Material UI shell that probes the server and reports
  what it finds.
- Local SQLite cache opened at startup.
- 8 tests, including integration tests that run the real router over real HTTP
  and a real WebSocket.
- CI running format, clippy, tests, typecheck, lint and build.

## Layout

```
apps/mobile/                React + TypeScript + Vite + Material UI
apps/mobile/src-tauri/      Tauri shell: network, local SQLite, device
services/assistant-server/  Axum HTTP + WebSocket
crates/
  assistant-protocol/       wire types shared by server and shell
  assistant-core/           orchestration seams, event bus
  assistant-models/         ModelProvider trait (no providers)
  assistant-tools/          ToolSpec, RiskLevel, permission types
  assistant-auth/           Principal, TokenVerifier, dev placeholder
  assistant-memory/         memory domain, MemoryStore trait
migrations/                 plain SQL, applied with sqlx-cli
docs/                       ARCHITECTURE, DECISIONS, DEVELOPMENT
```

## Quick start

```powershell
Copy-Item .env.example .env
Copy-Item apps/mobile/.env.example apps/mobile/.env
# set DEV_AUTH_TOKEN and VITE_DEV_AUTH_TOKEN to the same value

pnpm install --dir apps/mobile

cargo run -p assistant-server            # terminal 1
pnpm --dir apps/mobile tauri dev         # terminal 2
```

Home shows **Degraded** when the server is up but has no database — that is the
expected state until you set `DATABASE_URL`.

Full setup, Android instructions and the check commands are in
[`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

## Design

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — layers, dependency rules,
  request paths, what is deliberately absent.
- [`docs/DECISIONS.md`](docs/DECISIONS.md) — ADRs. Read ADR-0003 (providers),
  ADR-0005 (permissions) and ADR-0009 (dev auth) before extending anything.
- [`CLAUDE.md`](CLAUDE.md) — working rules for AI assistants in this repository.

## Security

`DevTokenVerifier` is a static shared bearer token with no expiry and no
revocation. It exists so the auth boundary is real code rather than a `TODO`. It
is **not** a security control — do not expose a build that uses it beyond a
trusted local network. See ADR-0009.

Secrets live in `.env` files, which are git-ignored; CI fails if one is ever
tracked. OAuth client secrets and provider API keys belong on the server and must
never be shipped in the mobile bundle.
