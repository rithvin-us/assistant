<p align="center">
  <img src="logo.png" alt="Personal Assistant Logo" width="160" />
</p>

# personal-assistant

A voice-first personal assistant. Rust backend, Tauri 2 + React mobile app, PostgreSQL.

**Status: Milestone 1 Complete — Prepared for Milestone 2 (Assistant Core).**
The repository structure, 7 Rust workspace crates, Axum HTTP/WS server, Tauri 2 mobile client, Material UI light theme, and security boundaries are audited, hardened, and verified.

## What works today

- Rust workspace of 7 crates, building clean under `clippy -D warnings`.
- Axum server with `GET /v1/health` and `WS /v1/conversation/{id}/stream`.
- Tauri 2 app with Material UI light theme shell that probes the server and reports status.
- Local SQLite cache opened at startup.
- 9 tests (unit + integration tests) passing 100% over real HTTP and WebSocket transports.
- Complete documentation audit in [`docs/MILESTONE-1-AUDIT.md`](docs/MILESTONE-1-AUDIT.md) and domain specification in [`docs/DOMAIN.md`](docs/DOMAIN.md).
- CI running format, clippy, tests, typecheck, lint, build, and secret scanning.

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
- [`docs/MILESTONE-3.md`](docs/MILESTONE-3.md) — durable actions: lifecycle,
  schema, state machines, idempotency, transactions, expiry, security boundaries.
- [`docs/MILESTONE-2.md`](docs/MILESTONE-2.md) — the orchestrator: lifecycle,
  injection, execution modes, tool loop, permission and approval boundaries,
  streaming, cancellation, error model, testing strategy.
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
