//! Configuration, resolved once at startup from the environment.
//!
//! Everything the process needs is read here and nowhere else, so there is a
//! single place to audit for secret handling. `Debug` is implemented by hand to
//! keep secrets out of logs.

use std::{fmt, net::SocketAddr, path::PathBuf, time::Duration};

use assistant_models::anthropic::{AnthropicConfig, Effort};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set")]
    Missing(&'static str),
    #[error("environment variable {name} is invalid: {reason}")]
    Invalid { name: &'static str, reason: String },
}

pub struct Config {
    /// Address the HTTP server binds to. Defaults to `0.0.0.0:8787` so a phone
    /// or emulator on the same network can reach the dev server.
    pub bind_addr: SocketAddr,
    /// Postgres connection string (Supabase in deployment). When absent the
    /// server still starts and reports itself degraded, so the frontend can be
    /// developed without a database.
    pub database_url: Option<String>,
    /// Shared bearer token for the development auth placeholder.
    pub dev_auth_token: String,
    /// Origins allowed by CORS during development.
    pub allowed_origins: Vec<String>,
    /// `RUST_LOG`-style filter.
    pub log_filter: String,
    /// Hard ceiling on rounds of tool execution within one turn.
    ///
    /// The server owns this, not the core and certainly not the model: it is the
    /// only thing standing between a confused model and an unbounded spend.
    pub max_tool_rounds: usize,

    /// The Anthropic credential.
    ///
    /// Server-only, and read here and nowhere else. `None` is a supported
    /// deployment: the deterministic path still answers and a turn that needs a
    /// model fails with `no_model_provider` rather than a fabricated reply.
    pub anthropic_api_key: Option<String>,
    /// Model identifier. Named once, here.
    pub model: String,
    pub model_max_output_tokens: u32,
    pub model_timeout: Duration,
    /// Reasoning effort. `None` omits the field entirely, for a model that does
    /// not accept it.
    pub model_effort: Option<Effort>,
    /// How many past messages may be replayed to the model. The character bound
    /// that accompanies it is a core default; this is the knob an operator has
    /// a reason to turn.
    pub context_max_messages: usize,
}

impl Config {
    /// Loads `.env` if present, then reads the environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Missing .env is normal in deployment; a malformed one is not, but
        // dotenvy reports both the same way, so only the value reads below can
        // fail hard.
        let _ = dotenvy::dotenv();

        let bind_addr = env_or("ASSISTANT_BIND_ADDR", "0.0.0.0:8787")
            .parse()
            .map_err(|e| ConfigError::Invalid {
                name: "ASSISTANT_BIND_ADDR",
                reason: format!("{e}"),
            })?;

        let dev_auth_token =
            std::env::var("DEV_AUTH_TOKEN").map_err(|_| ConfigError::Missing("DEV_AUTH_TOKEN"))?;
        if dev_auth_token.trim().is_empty() {
            return Err(ConfigError::Invalid {
                name: "DEV_AUTH_TOKEN",
                reason: "must not be empty".into(),
            });
        }

        let allowed_origins = env_or("ASSISTANT_ALLOWED_ORIGINS", "http://localhost:1420")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()),
            dev_auth_token,
            allowed_origins,
            log_filter: env_or("RUST_LOG", "assistant_server=debug,tower_http=debug,info"),
            max_tool_rounds: env_or("ASSISTANT_MAX_TOOL_ROUNDS", "4")
                .parse()
                .map_err(|e| ConfigError::Invalid {
                    name: "ASSISTANT_MAX_TOOL_ROUNDS",
                    reason: format!("{e}"),
                })?,

            // Read, never printed. The only thing logged about it is whether it
            // is present -- see `Debug` below and the startup log line.
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            model: env_or("ASSISTANT_MODEL", AnthropicConfig::DEFAULT_MODEL),
            model_max_output_tokens: parse_env("ASSISTANT_MODEL_MAX_OUTPUT_TOKENS", "4096")?,
            model_timeout: Duration::from_millis(parse_env("ASSISTANT_MODEL_TIMEOUT_MS", "60000")?),
            model_effort: {
                let raw = env_or("ASSISTANT_MODEL_EFFORT", "low");
                if raw.eq_ignore_ascii_case("none") {
                    None
                } else {
                    Some(Effort::parse(&raw).ok_or(ConfigError::Invalid {
                        name: "ASSISTANT_MODEL_EFFORT",
                        reason: "expected one of low, medium, high, xhigh, max, none".into(),
                    })?)
                }
            },
            context_max_messages: parse_env("ASSISTANT_CONTEXT_MAX_MESSAGES", "40")?,
        })
    }

    /// Builds the provider configuration, when this deployment has a credential.
    ///
    /// Assembling it here keeps every environment read in one file: the
    /// provider itself reads nothing.
    pub fn anthropic(&self) -> Option<AnthropicConfig> {
        let key = self.anthropic_api_key.as_ref()?;
        Some(
            AnthropicConfig::new(key)
                .with_model(&self.model)
                .with_max_output_tokens(self.model_max_output_tokens)
                .with_timeout(self.model_timeout)
                .with_effort(self.model_effort),
        )
    }

    /// Path to the SQL migrations directory, relative to the workspace root.
    pub fn migrations_dir() -> PathBuf {
        PathBuf::from("migrations")
    }
}

fn parse_env<T>(name: &'static str, default: &str) -> Result<T, ConfigError>
where
    T: std::str::FromStr,
    T::Err: fmt::Display,
{
    env_or(name, default)
        .parse()
        .map_err(|e| ConfigError::Invalid {
            name,
            reason: format!("{e}"),
        })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Redacts every secret. Adding a secret field to `Config` without adding it
/// here as `<redacted>` is the one mistake this impl exists to prevent.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "<redacted>"),
            )
            .field("dev_auth_token", &"<redacted>")
            .field("allowed_origins", &self.allowed_origins)
            .field("log_filter", &self.log_filter)
            .field("max_tool_rounds", &self.max_tool_rounds)
            // Presence, never the value: whether a provider is configured is
            // operationally important and is not itself a secret.
            .field(
                "anthropic_api_key",
                &self.anthropic_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("model", &self.model)
            .field("model_max_output_tokens", &self.model_max_output_tokens)
            .field("model_timeout", &self.model_timeout)
            .field("model_effort", &self.model_effort)
            .field("context_max_messages", &self.context_max_messages)
            .finish()
    }
}
