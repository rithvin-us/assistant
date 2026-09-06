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
#[serde(rename_all = "camelCase", tag = "state")]
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
        .timeout(Duration::from_secs(5))
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

/// Reports whether the offline cache is available.
#[tauri::command]
fn local_cache_ready(state: tauri::State<'_, Shell>) -> bool {
    state.local.is_some()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
                http: reqwest::Client::new(),
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
