# Working rules for this repository

Read `docs/ARCHITECTURE.md` and `docs/DECISIONS.md` before changing structure.

## Commands

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p assistant-server

pnpm --dir apps/mobile typecheck
pnpm --dir apps/mobile lint
pnpm --dir apps/mobile build
pnpm --dir apps/mobile tauri dev
```

`.env` at the repository root is required by the server (`DEV_AUTH_TOKEN`).

## Non-negotiable rules

**Never claim something works without running it.** "Should work", "the build
should pass" and similar are not acceptable. Run the command, read the output,
report what it actually said — including when it failed.

**Never fabricate an assistant response.** If a model provider is not wired up,
the correct behaviour is to say so explicitly, as the conversation WebSocket does
today. A canned reply that looks like inference is a lie about project state.

**Never let a model decide a permission level.** `RiskLevel` is set in Rust in a
`ToolSpec` and evaluated by deterministic code. Model output is untrusted input.
See ADR-0005.

**Never put a secret in `apps/mobile`.** `VITE_*` variables are inlined into the
shipped bundle. OAuth client secrets and provider API keys live on the server.

**Never log tokens, keys, passwords or message bodies.** `Config` has a
hand-written `Debug` that redacts secrets — if you add a secret field, add it
there as `<redacted>`. HTTP request logging omits query strings on purpose,
because the WebSocket accepts `?access_token=`.

**Never introduce infrastructure without a demonstrated need.** No Redis, Kafka,
Neo4j, Elasticsearch, Temporal or Kubernetes. Postgres covers relational data,
jobs and vector search.

**Never use a model for something deterministic.** "What are my tasks today?" is
a SQL query. Routing it through inference costs money and latency and can be
wrong.

## Architecture rules

`assistant-core` must not depend on a concrete integration or a concrete model
provider. Integrations implement traits from `assistant-tools`; providers
implement `ModelProvider` from `assistant-models`. The core depends on the traits.

`assistant-protocol` is a leaf crate. It must not gain a dependency on any other
crate in this workspace — the Tauri shell shares it.

React is presentation only. Network calls, credentials, retries and timeouts live
in `#[tauri::command]` functions in `apps/mobile/src-tauri/src/lib.rs`, exposed
through `apps/mobile/src/api/bridge.ts`. No `fetch` in a component. See ADR-0008.

Declare dependency versions once, in `[workspace.dependencies]` in the root
`Cargo.toml`; member crates use `foo.workspace = true`.

Migrations are plain SQL in `migrations/`, applied deliberately with
`scripts/migrate.ps1`. Never apply migrations at startup.

## Changing the wire protocol

`crates/assistant-protocol/src/lib.rs` and `apps/mobile/src/api/types.ts` must
change together. Bump `PROTOCOL_VERSION` in both for a breaking change; the app
already surfaces a mismatch to the user rather than misparsing.

## Recording decisions

Any change affecting security, database design, authentication, model providers,
mobile architecture, cloud cost, permissions or data retention needs an ADR in
`docs/DECISIONS.md` first — Decision, Context, Options, Chosen approach, Reason,
Consequences. Do not change these silently.

## Scope

Build what was asked. Do not implement future milestones opportunistically. A
placeholder screen that says "not built yet" is better than mock data that makes
an unbuilt feature look finished.
