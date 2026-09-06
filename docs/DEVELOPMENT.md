# Development

## Prerequisites

| Tool | Version used | Notes |
|---|---|---|
| Rust | 1.96.0 (stable, MSVC) | Install via **rustup**, not the standalone MSI — Android targets need `rustup target add`. |
| Node.js | 24.15.0 | |
| pnpm | 9.15.9 | `npm i -g pnpm` |
| Visual Studio Build Tools | 2022 or 2026, C++ workload | Required for the MSVC linker. |
| WebView2 Runtime | any recent | Preinstalled on Windows 11. |

For **Android** builds, additionally:

| Tool | Notes |
|---|---|
| JDK 17+ | Temurin 17. JDK 8 will not work. |
| Android SDK + NDK | `ANDROID_HOME` and `NDK_HOME` must be set. |
| Rust Android targets | `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` |

### Windows note: antivirus and `target/`

Real-time antivirus scanning can hold `.o` files open in `target/` and make
`cargo` fail with `The process cannot access the file because it is being used by
another process (os error 32)`. It is intermittent and a rebuild usually
succeeds. If it happens often, exclude the `target` directory from real-time
scanning (requires an elevated shell):

```powershell
Add-MpPreference -ExclusionPath "E:\w\personal AI assistant\target"
```

## First-time setup

```powershell
git clone <repo> && cd personal-assistant

# Secrets. Both files are git-ignored.
Copy-Item .env.example .env
Copy-Item apps/mobile/.env.example apps/mobile/.env
# Set DEV_AUTH_TOKEN in .env and VITE_DEV_AUTH_TOKEN in apps/mobile/.env
# to the same value.

pnpm install --dir apps/mobile
cargo build --workspace
```

## Running

Two terminals.

```powershell
# 1. the server
cargo run -p assistant-server
#    or: pwsh scripts/dev-server.ps1

# 2. the app (desktop)
pnpm --dir apps/mobile tauri dev
```

Home should show the server as **Degraded** (up, but no database configured) with
a latency figure. "Unreachable" means the server is not running, or
`VITE_SERVER_BASE_URL` points somewhere the device cannot reach.

### Frontend only, no Tauri

```powershell
pnpm --dir apps/mobile dev     # http://localhost:1420
```

The page renders, but every server call fails: `invoke` only exists inside a
Tauri webview. Use this for pure styling work.

### Android

Prerequisites above must be installed first.

```powershell
pnpm --dir apps/mobile tauri android init     # once
pnpm --dir apps/mobile tauri android dev
```

Set `VITE_SERVER_BASE_URL` in `apps/mobile/.env` to something the device can
reach — `http://10.0.2.2:8787` from the emulator, or your machine's LAN IP from a
physical device. The server already binds `0.0.0.0` for this reason.

## Database

The server runs without one and reports itself degraded, which is enough for
frontend work. To connect a real database, set `DATABASE_URL` in `.env` to the
Supabase Postgres connection string, then:

```powershell
cargo install sqlx-cli --no-default-features --features postgres,rustls
$env:DATABASE_URL = "<connection string>"
pwsh scripts/migrate.ps1
```

Migrations are never applied on boot — see ADR-0006.

## Checks

These are exactly what CI runs.

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

pnpm --dir apps/mobile typecheck
pnpm --dir apps/mobile lint
pnpm --dir apps/mobile build
```

## Adding things

**A server endpoint.** Add the request/response types to `assistant-protocol`,
mirror them in `apps/mobile/src/api/types.ts`, add the handler under
`services/assistant-server/src/routes/`, register it in `routes/mod.rs` (public
or behind the auth layer), and add an integration test to
`services/assistant-server/tests/api.rs`.

**A frontend server call.** Add a `#[tauri::command]` in
`apps/mobile/src-tauri/src/lib.rs`, register it in `invoke_handler`, and add a
wrapper to `apps/mobile/src/api/bridge.ts`. React components call the wrapper —
never `fetch` — see ADR-0008.

**A dependency.** Declare the version in `[workspace.dependencies]` in the root
`Cargo.toml` and reference it as `foo.workspace = true` in the member crate, so
versions cannot diverge across the workspace.

**A decision that affects security, the database, auth, model providers, mobile
architecture, cost, permissions or retention.** Write the ADR in
`docs/DECISIONS.md` first.

## Secrets

`.env` and `apps/mobile/.env` are git-ignored; only `.env.example` files are
committed, and CI fails the build if a `.env` is ever tracked.

`VITE_*` variables are inlined into the JavaScript bundle at build time. Only
development placeholders belong there. Real OAuth client secrets and provider API
keys live on the server and never reach the device — see ADR-0008 and ADR-0009.
