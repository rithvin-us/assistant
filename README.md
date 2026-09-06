<p align="center">
  <img src="logo.png" alt="Personal Assistant Logo" width="160" />
</p>

# personal-assistant

A voice-first personal assistant. Rust backend, Tauri 2 + React mobile app, PostgreSQL.

**Status: Milestone 4 complete — a real assistant that holds a conversation.**

## What works today

- **A real Anthropic model provider** behind the vendor-neutral `ModelProvider`
  trait, feature-gated so `assistant-core` cannot link or name it. Responses
  stream token by token from the API all the way to the phone.
- **Conversations persist in PostgreSQL.** Tell it your name, ask for it two
  turns later, restart the server, ask again — the history is read back from the
  database, scoped to the authenticated user in the SQL itself.
- **The Android APK connects to the server** over an authenticated WebSocket and
  renders the answer as it arrives. No provider credential is in the bundle: the
  phone talks to this server, and this server talks to the provider.
- Durable approvals, executions and an audit trail (M3), with the permission
  seam the model cannot reach (ADR-0005).
- A deterministic fast path that answers what code already knows without calling
  a model at all.
- CI running format, clippy, tests, typecheck, lint, build and secret scanning —
  and depending on no external AI API.

**Not built yet:** Gmail, Calendar, Drive, tasks, reminders, long-term memory,
speech recognition and text-to-speech. Authentication is still a development
bearer token. See [`docs/MILESTONE-4.md`](docs/MILESTONE-4.md) for the full list
of what is deferred and why.

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
expected state until you set `DATABASE_URL` and run `pwsh scripts/migrate.ps1`.

Without `ANTHROPIC_API_KEY` the server still runs: the deterministic path
answers, and anything that needs a model returns a clear "no model provider is
configured" rather than a fabricated reply.

Full setup, Android instructions and the check commands are in
[`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

## Design

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — layers, dependency rules,
  request paths, what is deliberately absent.
- [`docs/MILESTONE-4.md`](docs/MILESTONE-4.md) — the Anthropic provider,
  streaming end to end, conversation persistence, the context-window policy,
  API-key security, testing, and known limitations.
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
never be shipped in the mobile bundle — `VITE_*` variables are inlined into the
shipped Android bundle, so there is deliberately no `VITE_ANTHROPIC_API_KEY` and
must never be one. `ANTHROPIC_API_KEY` is read in one place, redacted in every
`Debug`, and never reaches a protocol frame, a database row or an error message.
See ADR-0020.
