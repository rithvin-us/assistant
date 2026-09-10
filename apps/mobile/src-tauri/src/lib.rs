//! Tauri shell.
//!
//! The React layer owns presentation only. Anything that touches the network,
//! the filesystem or the device lives here, behind `#[tauri::command]`, so the
//! same logic serves desktop and Android and can later be reused by native
//! Kotlin/Swift surfaces.

mod conversation;
mod local_db;

use std::time::Duration;

use std::sync::Arc;

use assistant_protocol::HealthResponse;
use serde::Serialize;
use sqlx::sqlite::SqlitePool;
use tauri::Manager;

/// Handles owned by the shell for the lifetime of the process.
struct Shell {
    http: reqwest::Client,
    /// `None` when the local cache could not be opened; the app still runs, it
    /// simply has no offline buffer.
    local: Option<SqlitePool>,
}

/// Result of a connectivity probe, shaped for direct rendering.
#[derive(Debug, Serialize)]
// `rename_all` renames the variants; the fields need `rename_all_fields`.
// Without it `latency_ms` reaches a UI that reads `latencyMs` and the sheet
// renders "undefined ms".
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "state"
)]
enum ProbeResult {
    Reachable {
        health: HealthResponse,
        latency_ms: u64,
    },
    Unreachable {
        reason: String,
    },
}

/// Probes the development server.
///
/// Returns `Ok` with an `Unreachable` variant for network failures rather than
/// `Err`: not being able to reach the server is an expected state the UI must
/// render, not an exception.
#[tauri::command]
async fn probe_server(
    state: tauri::State<'_, Shell>,
    base_url: String,
    token: String,
) -> Result<ProbeResult, String> {
    let started = std::time::Instant::now();

    let response = state
        .http
        .get(format!("{}/v1/health", base_url.trim_end_matches('/')))
        .bearer_auth(&token)
        .timeout(Duration::from_secs(30))
        .send()
        .await;

    let response = match response {
        Ok(response) => response,
        // The error is rendered to the user, so it must not contain the token.
        // `reqwest::Error`'s Display does not include headers.
        Err(error) => {
            return Ok(ProbeResult::Unreachable {
                reason: error.to_string(),
            });
        }
    };

    if !response.status().is_success() {
        return Ok(ProbeResult::Unreachable {
            reason: format!("server returned HTTP {}", response.status()),
        });
    }

    match response.json::<HealthResponse>().await {
        Ok(health) => Ok(ProbeResult::Reachable {
            health,
            latency_ms: started.elapsed().as_millis() as u64,
        }),
        Err(error) => Ok(ProbeResult::Unreachable {
            reason: format!("unreadable health response: {error}"),
        }),
    }
}

/// Builds the shell's HTTP client.
///
/// The trust anchors are the bundled Mozilla root set rather than the platform
/// verifier reqwest 0.13 selects by default. That verifier requires JNI
/// initialisation with the Android `Context`; a Tauri shell never performs it,
/// and uninitialised it panics inside the connector task:
///
/// ```text
/// thread 'tokio-rt-worker' panicked at rustls-platform-verifier-0.6.2/src/android.rs:94:10:
/// Expect rustls-platform-verifier to be initialized
/// ```
///
/// A panicked command never answers its IPC call, so the UI waits on a promise
/// that cannot settle -- the connection dot stays on "checking" forever instead
/// of reporting a failure. Plain HTTP was unaffected, which is why this stayed
/// hidden while the server was addressed over the LAN. `tokio_tungstenite`
/// already trusts this same set for the conversation socket. See ADR-0031.
fn http_client() -> reqwest::Client {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("aws-lc-rs supports the default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth();

    reqwest::Client::builder()
        .tls_backend_preconfigured(tls)
        .build()
        .expect("a client with a preconfigured rustls backend always builds")
}

/// Reports whether the offline cache is available.
#[tauri::command]
fn local_cache_ready(state: tauri::State<'_, Shell>) -> bool {
    state.local.is_some()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Install aws-lc-rs as the process-wide rustls crypto provider.
    //
    // `tokio_tungstenite` builds its TLS config with
    // `rustls::ClientConfig::builder()`, which calls
    // `CryptoProvider::get_default()`. Without this call the provider is
    // `None` and every `wss://` connection logs
    //   "Call CryptoProvider::install_default() before this point"
    // and then fails — so the conversation WebSocket never opens and the
    // AI turn appears to hang forever. The `http_client()` below already
    // passes the provider in explicitly; this covers tokio_tungstenite.
    // Installing twice is harmless: the second call returns `Err` and we
    // ignore it.
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "assistant_mobile_lib=debug,info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("assistant-cache.sqlite");

            // Blocking here is intentional: the cache must exist before the
            // first command runs, and setup is off the UI thread.
            let local = tauri::async_runtime::block_on(local_db::open(&path));
            let local = match local {
                Ok(pool) => Some(pool),
                Err(error) => {
                    tracing::error!(%error, "local cache unavailable; continuing without it");
                    None
                }
            };

            app.manage(Shell {
                http: http_client(),
                local,
            });
            app.manage(Arc::new(conversation::Connection::default()));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_server,
            local_cache_ready,
            conversation::conversation_open,
            conversation::conversation_send,
            conversation::conversation_close,
            conversation::conversation_is_open,
        ])
        .run(tauri::generate_context!())
        .expect("error while running the Tauri application");
}
