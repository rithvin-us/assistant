//! Entry point.
//!
//! Responsibilities are limited to: install tracing, resolve config, connect
//! optional storage, bind, and shut down cleanly. Everything else lives in the
//! library so it is testable.

use std::sync::Arc;

use assistant_core::{DomainEvent, EventBus};
use assistant_models::ModelProvider;
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
                tracing::error!(error = %error, "DATABASE_URL is set but unreachable; continuing degraded");
                None
            }
        },
        None => {
            tracing::warn!("DATABASE_URL is not set; running without persistence");
            None
        }
    };

    let events = EventBus::default();

    // The durable stores exist only when a database does. Without one the
    // server still runs: turns still stop at an approval, but the action is not
    // persisted and the client is told so rather than handed an unusable id,
    // and conversations do not survive a restart.
    let store = pool.clone().map(|pool| {
        Arc::new(assistant_server::store::PostgresActionStore::new(pool))
            as Arc<dyn assistant_core::actions::ActionStore>
    });
    if store.is_none() {
        tracing::warn!("no durable action store; approvals cannot be resumed");
    }

    let conversations = pool.clone().map(|pool| {
        Arc::new(assistant_server::conversations::PostgresConversationStore::new(pool))
            as Arc<dyn assistant_core::conversation::ConversationStore>
    });
    if conversations.is_none() {
        tracing::warn!("no conversation store; the assistant will not remember anything");
    }

    // The provider is constructed only when a credential is configured. A
    // deployment without one still answers on the deterministic path, and a
    // turn that needs a model fails with a clear `no_model_provider` rather
    // than a fabricated reply.
    //
    // Only the *presence* of the credential is logged, never any part of it.
    let model = match config.openai() {
        Some(provider_config) => {
            match assistant_models::openai::OpenAIModelProvider::new(provider_config) {
                Ok(provider) => {
                    tracing::info!(
                        provider = provider.name(),
                        model = %config.model,
                        "model provider configured"
                    );
                    Some(Arc::new(provider) as Arc<dyn assistant_models::ModelProvider>)
                }
                Err(error) => {
                    tracing::error!(
                        code = error.code(),
                        "OPENAI_API_KEY is set but the provider could not be built"
                    );
                    None
                }
            }
        }
        None => {
            tracing::warn!("OPENAI_API_KEY is not set; running without a model provider");
            None
        }
    };

    // Tools stay empty on purpose. Registering a placeholder would advertise a
    // capability that does not exist; real tools arrive with the integration
    // milestones that implement them.
    let router = app(
        &config,
        pool,
        events.clone(),
        assistant_server::orchestration::Dependencies {
            model,
            store,
            conversations,
            ..Default::default()
        },
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
        tracing::error!(error = %error, "failed to install ctrl-c handler");
    }
    tracing::info!("shutdown signal received");
}
