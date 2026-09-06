# Milestone 1 Architecture Audit & Hardening

**Repository**: [`https://github.com/rithvin-us/assistant`](https://github.com/rithvin-us/assistant)  
**Evaluated Commit**: `8afc9b4`  
**Target Milestone**: Milestone 2 — Assistant Core Preparation  

---

## 1. Repository State Overview

- **HEAD Commit**: `8afc9b4` (`feat(mobile): reduce Home to a voice button and a single sheet`)
- **Git Branch**: `main` (clean working tree)
- **Cargo Workspace**: 8 members (`crates/assistant-protocol`, `crates/assistant-core`, `crates/assistant-models`, `crates/assistant-tools`, `crates/assistant-auth`, `crates/assistant-memory`, `services/assistant-server`, `apps/mobile/src-tauri`).
- **Mobile Client**: Tauri 2 shell + React (TypeScript + Material UI light theme enforced).
- **Verified Build / Test Status**:
  - `cargo fmt --all --check` — Passed (0 errors)
  - `cargo clippy --workspace --all-targets -- -D warnings` — Passed (0 warnings)
  - `cargo test --workspace` — Passed (8/8 tests passed)
  - `pnpm --dir apps/mobile typecheck` — Passed (0 errors)
  - `pnpm --dir apps/mobile lint` — Passed (0 errors)
  - `pnpm --dir apps/mobile build` — Passed (0 errors)

---

## 2. Architecture Boundary Assessment

### 🟢 GREEN (Correct & Keep)

1. **`assistant-protocol`**: Leaf crate with zero workspace dependencies. shared wire types (`HealthResponse`, `ApiError`, `ClientFrame`, `ServerFrame`) between server and Tauri shell. Bumps `PROTOCOL_VERSION` (v1) on breaking changes.
2. **`assistant-core`**: Defines pure domain seams and Tokio broadcast `EventBus`. Free of concrete integrations (no vendor SDKs).
3. **`assistant-auth`**: Clear `Principal` and `TokenVerifier` traits. `DevTokenVerifier` isolates temporary local bearer authentication. Handlers consume `Principal` via Axum request extensions.
4. **`assistant-memory`**: Strict separation between conversation logs (transient) and promoted memories (`MemoryKind`, `Lifecycle`, `Importance`, `Provenance`, `MemoryStore`).
5. **`assistant-server`**: Axum HTTP & WebSocket transport cleanly separated into routes (`/v1/health`, `/v1/conversation/:id/stream`). Redacts secrets in logs via custom `Debug` implementations and query string log omission.
6. **`apps/mobile` Architecture**: React is strictly presentation-only. Tauri commands handle network and native capabilities, with a fallback for browser styling mode behind `bridge.ts`. Material UI light mode strictly enforced.

### 🟡 YELLOW (Acceptable, Improved in Preparation for Milestone 2)

1. **`assistant-models` Contract**: Originally only accepted raw string messages. **Upgraded**: Enhanced `GenerateRequest` to accept `system_prompt` and `tools: Vec<ToolSpec>`, `GenerateResponse` to return `tool_calls: Vec<ToolCall>`, and `StreamChunk` to yield `ToolCall` variants.
2. **`assistant-tools` & Permissions**: `RiskLevel` (`Green`, `Yellow`, `Orange`, `Red`) and `PermissionDecision` (`Allow`, `RequireApproval`, `Deny`) are fixed in Rust code. **Upgraded**: Added `ApprovalStatus` (`Requested`, `Approved`, `Rejected`, `Expired`, `Cancelled`) and `ToolCall`/`ToolResult` types to prepare for persistent human-in-the-loop approvals.
3. **GitHub Actions Security (`release.yml`)**: `ci.yml` is clean. `release.yml` had broad permissions. **Recommendation**: Lock down top-level permissions to read-only, scoping `contents: write` exclusively to release publishing jobs, and use `--frozen-lockfile` for `pnpm`.

### 🔴 RED (Must Change Before Milestone 2)

- *None identified in core boundaries*. All red flags from Milestone 0 (missing member manifests, missing dependencies, dark mode confusion) have been resolved and verified.

---

## 3. Risk Matrix

| Area | Risk | Mitigation in Architecture |
| :--- | :--- | :--- |
| **Permissions** | Prompt injection inducing dangerous tool execution | **ADR-0005**: Risk levels are static Rust attributes. Model output is treated as untrusted input. |
| **Authentication** | Multi-account token leakage across boundaries | OAuth tokens stay server-side. `Principal` contains only user ID and granted scopes. |
| **Latency** | Voice turns blocked on LLM reasoning for deterministic queries | `assistant-core` will evaluate SQL/fast-path queries before routing to models. |
| **Costs** | Unbounded model calls for background watchdogs & routing | Cost philosophy strictly enforced: cheap/deterministic classifiers first, heavy models only for reasoning. |
| **Offline** | Database corruption or heavy sync conflicts | **ADR-0007**: Local SQLite is disposable cache + capture queue, not a full replica. |

---

## 4. Workflows & CI Security Audit

- **`ci.yml`**: Uses ` Swatinem/rust-cache@v2`, enforces formatting, clippy (`-D warnings`), workspace unit/integration tests, node typechecking, linting, and a secret scanner step checking for tracked `.env` files.
- **`release.yml`**: Configured to run on `v*` tag pushes. Builds Linux server binary and cross-platform Tauri bundles.

---

## 5. Conclusion & Verification

Milestone 1 is complete and fully hardened. All 7 workspace crates, the server service, the mobile React shell, and the CI pipelines are cleanly aligned for **Milestone 2 — Assistant Core**.
