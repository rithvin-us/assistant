//! Entry point.
//!
//! Responsibilities are limited to: install tracing, resolve config, connect
//! optional storage, bind, and shut down cleanly. Everything else lives in the
//! library so it is testable.

use std::sync::Arc;

use assistant_core::{DomainEvent, EventBus, ToolRegistry};
use assistant_models::ModelProvider;
use assistant_server::{app, config::Config, db};
use assistant_tools::{
    AcademicAssignmentsTool, AcademicDeadlinesTool, AcademicSyncTool, CalendarCreateTool,
    CalendarDeleteTool, CalendarListTool, CalendarSearchTool, CalendarUpdateTool,
    ClassroomAnnouncementsTool, ClassroomCoursesTool, ClassroomCourseworkTool, DriveListTool,
    DriveMetadataTool, DriveReadFileTool, DriveSearchTool, GmailReadTool, GmailSearchTool,
};
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
                tracing::info!(error = %error, "DATABASE_URL unreachable; running in memory mode");
                None
            }
        },
        None => {
            tracing::info!("DATABASE_URL not set; running in memory mode");
            None
        }
    };

    let events = EventBus::default();

    let store = pool.clone().map(|pool| {
        Arc::new(assistant_server::store::PostgresActionStore::new(pool))
            as Arc<dyn assistant_core::actions::ActionStore>
    });

    let conversations = pool.clone().map(|pool| {
        Arc::new(assistant_server::conversations::PostgresConversationStore::new(pool))
            as Arc<dyn assistant_core::conversation::ConversationStore>
    });

    let memory = pool.clone().map(|pool| {
        Arc::new(assistant_server::memory_store::PostgresMemoryStore::new(
            pool,
        )) as Arc<dyn assistant_memory::MemoryStore>
    });

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

    let mut tool_registry = ToolRegistry::new();
    if let Some(ref pool_ref) = pool {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        let google_client = Arc::new(assistant_server::google::GoogleClient::new(
            pool_ref.clone(),
            http,
            config.google_client_id.clone(),
            config.google_client_secret.clone(),
            config.resolved_encryption_key(),
        ));
        tool_registry.register(Arc::new(GmailSearchTool::new(google_client.clone())));
        tool_registry.register(Arc::new(GmailReadTool::new(google_client.clone())));
        tool_registry.register(Arc::new(CalendarListTool::new(google_client.clone())));
        tool_registry.register(Arc::new(CalendarSearchTool::new(google_client.clone())));
        tool_registry.register(Arc::new(CalendarCreateTool::new(google_client.clone())));
        tool_registry.register(Arc::new(CalendarUpdateTool::new(google_client.clone())));
        tool_registry.register(Arc::new(CalendarDeleteTool::new(google_client.clone())));

        // Milestone 6. `GoogleClient` implements the Classroom and Drive
        // traits too, so the same authenticated, credential-decrypting client
        // serves all four integrations.
        let classroom: Arc<dyn assistant_tools::ClassroomProvider> = google_client.clone();
        let drive: Arc<dyn assistant_tools::DriveProvider> = google_client.clone();

        tool_registry.register(Arc::new(ClassroomCoursesTool::new(classroom.clone())));
        tool_registry.register(Arc::new(ClassroomCourseworkTool::new(classroom.clone())));
        tool_registry.register(Arc::new(ClassroomAnnouncementsTool::new(classroom.clone())));
        tool_registry.register(Arc::new(DriveSearchTool::new(drive.clone())));
        tool_registry.register(Arc::new(DriveListTool::new(drive.clone())));
        tool_registry.register(Arc::new(DriveMetadataTool::new(drive.clone())));
        tool_registry.register(Arc::new(DriveReadFileTool::new(drive)));

        // The academic tools read the database as well as Google, so they get
        // the service that owns both rather than the raw client.
        let academic: Arc<dyn assistant_tools::AcademicProvider> = Arc::new(
            assistant_server::academic::AcademicService::new(pool_ref.clone(), classroom),
        );
        tool_registry.register(Arc::new(AcademicDeadlinesTool::new(academic.clone())));
        tool_registry.register(Arc::new(AcademicAssignmentsTool::new(academic.clone())));
        tool_registry.register(Arc::new(AcademicSyncTool::new(academic)));

        tracing::info!(
            "registered Google tools (gmail.*, calendar.*, classroom.*, drive.*, academic.*)"
        );
    }

    let router = app(
        &config,
        pool,
        events.clone(),
        assistant_server::orchestration::Dependencies {
            model,
            tools: Arc::new(tool_registry),
            store,
            conversations,
            memory,
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
