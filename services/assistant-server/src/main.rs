//! Entry point.
//!
//! Responsibilities are limited to: install tracing, resolve config, connect
//! optional storage, bind, and shut down cleanly. Everything else lives in the
//! library so it is testable.

use assistant_core::{DomainEvent, EventBus};
use assistant_server::{app, config::Config, db};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::from_env()?;

    tracing_subscriber::registry()
        .with(EnvFilter::new(&config.log_filter))
        .with(fmt::layer().with_target(true))
        .init();

    tracing::info!(config = ?config, "starting assistant-server");

    let pool = match &config.database_url {
        Some(url) => match db::connect(url).await {
            Ok(pool) => {
                tracing::info!("connected to postgres");
                Some(pool)
            }
            // A database that is configured but unreachable is a real problem,
            // yet refusing to boot would also block frontend work. Log loudly,
            // report degraded via /v1/health, continue.
            Err(error) => {
                tracing::error!(%error, "DATABASE_URL is set but unreachable; continuing degraded");
                None
            }
        },
        None => {
            tracing::warn!("DATABASE_URL is not set; running without persistence");
            None
        }
    };

    let events = EventBus::default();
    // A default build wires no model provider and no tools. The deterministic
    // path still answers, and a turn that needs a model fails with a clear
    // `no_model_provider` rather than a fabricated reply. Real providers and
    // tools arrive with the milestones that implement them.
    let router = app(
        &config,
        pool,
        events.clone(),
        assistant_server::orchestration::Dependencies::default(),
    );

    let listener = tokio::net::TcpListener::bind(config.bind_addr).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");

    // Published only after the socket is bound, so no subscriber can observe
    // "started" before the port actually accepts connections.
    events.publish(DomainEvent::ServerStarted {
        version: env!("CARGO_PKG_VERSION").to_string(),
    });

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("shut down cleanly");
    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to install ctrl-c handler");
    }
    tracing::info!("shutdown signal received");
}
