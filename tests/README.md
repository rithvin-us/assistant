# Tests

Rust tests live next to what they test:

- **Unit tests** — `#[cfg(test)]` modules inside each crate.
- **Integration tests** — `services/assistant-server/tests/api.rs`. These build
  the real router, bind an ephemeral port, and talk to it over real HTTP and a
  real WebSocket. Nothing is mocked; the database is simply absent, which is a
  state the server is designed to handle.

This directory is reserved for cross-cutting tests that span the server and the
Tauri shell — end-to-end checks that neither side can own alone. There are none
yet, and an empty file is preferable to a placeholder test that asserts nothing.

Run everything with:

```powershell
cargo test --workspace
```
