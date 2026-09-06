<p align="center">
  <img src="logo.png" alt="Personal Assistant Logo" width="160" />
</p>

# personal-assistant

A voice-first personal assistant. Rust backend, Tauri 2 + React mobile app, PostgreSQL.

**Status: Milestone 5 complete — Google Ecosystem + Personal Schedule Foundation.**

## What works today

- **Multi-Account Google Connectivity:** Connect arbitrary ($N$) Google accounts (Personal, College, Work) via server-side OAuth2 with AES-256-GCM encrypted tokens at rest.
- **Gmail Search and Read:** Search Gmail with native syntax (`is:unread`, `from:`, `subject:`) and read email messages with privacy-first sanitization.
- **Google Calendar Management:** Read, search, create, and update calendar events. Destructive deletions are statically classified as Orange risk and require explicit human approval via durable actions.
- **Deterministic Free-Time Calculation:** Pure mathematical interval arithmetic computes open calendar slots for task scheduling without requiring an LLM.
- **Task to Calendar Foundation:** Tasks support estimated duration and due dates to bridge directly into personal schedule planning.
- **Pure Light Theme Mobile Screens:** Dedicated Connections, Gmail, and Calendar screens alongside Tasks, Notes, and the conversational assistant.
- **A real Anthropic model provider** behind the vendor-neutral `ModelProvider` trait.
- **Conversations persist in PostgreSQL** scoped strictly to the authenticated user.
- **Durable approvals, executions, and audit trail** (M3), preserving authoritative security.

**Not built yet:** Google Classroom, Google Drive, WhatsApp, long-term memory embeddings, automatic email importance classification, and local wake-word voice. See [`docs/MILESTONE-5.md`](docs/MILESTONE-5.md) for full details.

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
