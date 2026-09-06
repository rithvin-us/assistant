# Development

## Prerequisites

| Tool | Version verified | Notes |
|---|---|---|
| Rust | 1.98.1 (stable, MSVC) | Managed via **rustup** (`C:\Users\rithv\.cargo\bin\rustup.exe`). Pinned via `rust-toolchain.toml`. |
| Node.js | 24.15.0 | Modern ESM support. |
| pnpm | 9.15.9 | Workspace package manager (`npm i -g pnpm`). |
| Visual Studio Build Tools | 2026 (Professional), C++ workload | Required for the MSVC linker. |
| WebView2 Runtime | 152.0.4191.62 | Preinstalled on Windows 11. |

For **Android** builds, additionally:

| Tool | Version / Location | Notes |
|---|---|---|
| JDK 21 | OpenJDK 21.0.10 | Bundled Android Studio JBR at `C:\Program Files\Android\Android Studio\jbr`. `JAVA_HOME` points here. |
| Android SDK | API 35 / 36 | `C:\Users\rithv\AppData\Local\Android\Sdk`. `ANDROID_HOME` points here. |
| Android NDK | `27.1.12297006` | `C:\Users\rithv\AppData\Local\Android\Sdk\ndk\27.1.12297006`. `NDK_HOME` points here. |
| Android Target Triples | `aarch64`, `armv7`, `i686`, `x86_64` | `rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android` |
| ADB | `1.0.41` (v37.0.0) | `C:\Users\rithv\AppData\Local\Android\Sdk\platform-tools\adb.exe` |

### Windows note: antivirus and `target/`

Real-time antivirus scanning can hold `.o` files open in `target/` and make `cargo` fail with `The process cannot access the file because it is being used by another process (os error 32)`. It is intermittent and a rebuild usually succeeds. If it happens often, exclude the `target` directory from real-time scanning (requires an elevated shell):

```powershell
Add-MpPreference -ExclusionPath "E:\w\personal AI assistant\target"
```

---

## Environment Setup

### 1. Root Server Environment (`.env`)

Copy `.env.example` to `.env` at root:

```powershell
Copy-Item .env.example .env
```

Configured variables:
```env
ASSISTANT_BIND_ADDR=0.0.0.0:8787
DEV_AUTH_TOKEN=local-dev-token
ASSISTANT_ALLOWED_ORIGINS=http://localhost:1420
ASSISTANT_MAX_TOOL_ROUNDS=4
RUST_LOG=assistant_server=debug,tower_http=debug,info

# Database (Supabase Session Pooler on port 5432)
DATABASE_URL=postgresql://postgres.<ref>:<password>@aws-0-<region>.pooler.supabase.com:5432/postgres

# Model provider. SERVER ONLY -- read by Config::from_env and nowhere else.
# Leave unset to run without a provider: the deterministic path still answers
# and a turn that needs a model fails with `no_model_provider`.
# ANTHROPIC_API_KEY=sk-ant-api03-...
ASSISTANT_MODEL=claude-opus-5
ASSISTANT_MODEL_MAX_OUTPUT_TOKENS=4096
ASSISTANT_MODEL_TIMEOUT_MS=60000
ASSISTANT_MODEL_EFFORT=low
ASSISTANT_CONTEXT_MAX_MESSAGES=40
```

Every variable above is documented in `.env.example`. The four
`ASSISTANT_MODEL_*` values and `ASSISTANT_CONTEXT_MAX_MESSAGES` have working
defaults; only `DEV_AUTH_TOKEN` is required.

> **Never add `VITE_ANTHROPIC_API_KEY`.** Vite inlines `VITE_*` variables into
> the shipped Android bundle, so a provider key there ships to every device. The
> phone talks to this server; this server talks to the provider. See ADR-0020.

### 2. Mobile Client Environment (`apps/mobile/.env`)

Copy `apps/mobile/.env.example` to `apps/mobile/.env`:

```powershell
Copy-Item apps/mobile/.env.example apps/mobile/.env
```

Configured variables:
```env
VITE_SERVER_BASE_URL=http://127.0.0.1:8787
VITE_DEV_AUTH_TOKEN=local-dev-token
```

> [!IMPORTANT]
> Secrets (`DATABASE_URL`, `ANTHROPIC_API_KEY`) belong strictly in the root `.env` server environment. Never put database credentials or provider secrets in `apps/mobile/.env` or `VITE_*` variables.

---

## Running the Application

Two terminals:

```powershell
# Terminal 1: Rust Axum Backend Server
cargo run -p assistant-server

# Terminal 2: React + Tauri Desktop App
pnpm --dir apps/mobile tauri dev
```

### Mobile / Android Development

Initialize Android project once:

```powershell
# 1. Initialize Android Studio Gradle project
pnpm --dir apps/mobile tauri android init

# 2. Run Android live dev server (emulator or connected physical device)
pnpm --dir apps/mobile tauri android dev
```

Build installable APK:

```powershell
pnpm --dir apps/mobile tauri android build -- --apk
```

Output APK location:
`apps/mobile/src-tauri/gen/android/app/build/outputs/apk/debug/app-debug.apk`

Install on connected Android device via ADB:

```powershell
adb devices
adb install apps/mobile/src-tauri/gen/android/app/build/outputs/apk/debug/app-debug.apk
```

---

## Database & Migrations

The server runs in **Degraded** mode without a database. To connect Supabase PostgreSQL, set `DATABASE_URL` in `.env` and run migrations:

```powershell
# 1. Install sqlx-cli (once)
cargo install sqlx-cli --no-default-features --features postgres,rustls

# 2. Apply migrations
$env:DATABASE_URL = "postgresql://postgres.<ref>:<password>@aws-0-<region>.pooler.supabase.com:5432/postgres"
pwsh scripts/migrate.ps1
```

### Supabase Connection String Rules

- **Use Session Pooler (port 5432)** for persistent client connections over IPv4 networks (`postgres.<ref>@aws-N-<region>.pooler.supabase.com:5432`). Do not use transaction pooler mode on port 6543 (transaction mode breaks prepared statements).

---

## Database-backed tests

Both suites skip, rather than fail, when `DATABASE_URL` is absent, so
`cargo test --workspace` stays runnable without credentials.

```powershell
$env:DATABASE_URL = "postgresql://postgres.<ref>:<password>@aws-0-<region>.pooler.supabase.com:5432/postgres"

# Approvals, executions and audit (M3).
cargo test -p assistant-server --test durable_actions

# Conversation history: ordering, ownership, schema constraints, and survival
# across a restart (M4).
cargo test -p assistant-server --test conversations
```

---

## Live Anthropic smoke test

One real, billable request. `#[ignore]`d so `cargo test --workspace` compiles it
and never runs it — CI must not depend on an external AI API.

```powershell
$env:ANTHROPIC_API_KEY = "sk-ant-api03-..."   # or set it in .env
cargo test -p assistant-models --test live_anthropic -- --ignored --nocapture
```

It sends one trivial prompt with a 64-token output cap, asserts the response
streamed and answered the prompt, and asserts no tool call occurred on a turn
that offered no tools. It prints token counts and the answer; it prints no part
of the credential.

Every other test — including all 21 Anthropic provider tests — runs against a
local socket speaking canned HTTP and needs no key, no network and no account.

---

## Project Verification Checks

These commands are run in CI:

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

pnpm --dir apps/mobile typecheck
pnpm --dir apps/mobile lint
pnpm --dir apps/mobile build
```

`cargo test --workspace` needs no database and no API key. The suites that do
are listed above and skip without their credential.
