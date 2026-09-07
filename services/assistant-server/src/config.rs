//! Configuration, resolved once at startup from the environment.
//!
//! Everything the process needs is read here and nowhere else, so there is a
//! single place to audit for secret handling. `Debug` is implemented by hand to
//! keep secrets out of logs.

use std::{fmt, net::SocketAddr, path::PathBuf, time::Duration};

use assistant_models::openai::OpenAIConfig;

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
    /// Shared bearer token for the development auth placeholder. Used only
    /// when no Supabase project is configured.
    pub dev_auth_token: String,
    /// Supabase project reference. When set, callers are authenticated by a
    /// Supabase-issued ES256 JWT and the development token is not accepted.
    /// See ADR-0024.
    pub supabase_project_ref: Option<String>,
    /// Origins allowed by CORS during development.
    pub allowed_origins: Vec<String>,
    /// `RUST_LOG`-style filter.
    pub log_filter: String,
    /// Hard ceiling on rounds of tool execution within one turn.
    pub max_tool_rounds: usize,

    /// The OpenAI credential.
    pub openai_api_key: Option<String>,
    /// Model used for audio transcription (e.g. whisper-1).
    pub openai_transcription_model: String,
    /// Language hint for audio transcription (e.g. en).
    pub openai_transcription_language: Option<String>,
    /// Model identifier for completions. Named once, here.
    pub model: String,
    pub model_max_output_tokens: u32,
    pub model_timeout: Duration,
    /// How many past messages may be replayed to the model.
    pub context_max_messages: usize,

    /// Google OAuth Client ID.
    pub google_client_id: Option<String>,
    /// Google OAuth Client Secret.
    pub google_client_secret: Option<String>,
    /// Google OAuth Redirect URI.
    pub google_redirect_uri: Option<String>,
    /// 32-byte AES-GCM encryption key for credentials at rest.
    pub credential_encryption_key: Option<String>,
    /// Directory the local document storage backend writes into (M8).
    pub document_storage_dir: PathBuf,

    /// Cartesia API key for STT and TTS (M10). SERVER ONLY.
    pub cartesia_api_key: Option<String>,
    pub cartesia_stt_model: String,
    pub cartesia_tts_model: String,
    pub cartesia_tts_voice_id: String,
}

impl Config {
    /// Loads `.env` if present, then reads the environment.
    pub fn from_env() -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();

        let port = env_or("PORT", "8787");
        let default_bind = format!("0.0.0.0:{port}");
        let bind_addr = env_or("ASSISTANT_BIND_ADDR", &default_bind)
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
                reason: "cannot be empty".into(),
            });
        }

        let allowed_origins = env_or("ASSISTANT_ALLOWED_ORIGINS", "http://localhost:1420")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let google_client_id = std::env::var("GOOGLE_CLIENT_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let google_client_secret = std::env::var("GOOGLE_CLIENT_SECRET")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let google_redirect_uri = std::env::var("GOOGLE_REDIRECT_URI")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let credential_encryption_key = std::env::var("CREDENTIAL_ENCRYPTION_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()),
            dev_auth_token,
            supabase_project_ref: std::env::var("SUPABASE_PROJECT_REF")
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()),
            allowed_origins,
            log_filter: env_or("RUST_LOG", "assistant_server=debug,tower_http=debug,info"),
            max_tool_rounds: env_or("ASSISTANT_MAX_TOOL_ROUNDS", "4")
                .parse()
                .map_err(|e| ConfigError::Invalid {
                    name: "ASSISTANT_MAX_TOOL_ROUNDS",
                    reason: format!("{e}"),
                })?,

            openai_api_key: std::env::var("OPENAI_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            openai_transcription_model: env_or("OPENAI_TRANSCRIPTION_MODEL", "whisper-1"),
            openai_transcription_language: std::env::var("OPENAI_TRANSCRIPTION_LANGUAGE")
                .ok()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty()),
            model: env_or("ASSISTANT_MODEL", OpenAIConfig::DEFAULT_MODEL),
            model_max_output_tokens: parse_env("ASSISTANT_MODEL_MAX_OUTPUT_TOKENS", "4096")?,
            model_timeout: Duration::from_millis(parse_env("ASSISTANT_MODEL_TIMEOUT_MS", "60000")?),
            context_max_messages: parse_env("ASSISTANT_CONTEXT_MAX_MESSAGES", "40")?,

            google_client_id,
            google_client_secret,
            google_redirect_uri,
            credential_encryption_key,
            document_storage_dir: PathBuf::from(env_or(
                "ASSISTANT_DOCUMENT_STORAGE_DIR",
                "./data/documents",
            )),
            cartesia_api_key: std::env::var("CARTESIA_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            cartesia_stt_model: env_or("CARTESIA_STT_MODEL", "ink-en-us"),
            cartesia_tts_model: env_or("CARTESIA_TTS_MODEL", "sonic-english"),
            cartesia_tts_voice_id: env_or(
                "CARTESIA_TTS_VOICE_ID",
                "a0e99841-438c-4a64-b679-ae501e7d6091",
            ),
        })
    }

    /// Resolves the 32-byte credential encryption key.
    /// If CREDENTIAL_ENCRYPTION_KEY is provided (hex or 32-byte ascii), it decodes it.
    /// If absent, falls back to a deterministic development key with a warning.
    pub fn resolved_encryption_key(&self) -> [u8; 32] {
        if let Some(ref key_str) = self.credential_encryption_key {
            if let Some(bytes) = decode_hex_32(key_str) {
                return bytes;
            }
            if key_str.len() == 32 {
                let mut key = [0u8; 32];
                key.copy_from_slice(key_str.as_bytes());
                return key;
            }
        }
        tracing::warn!(
            "CREDENTIAL_ENCRYPTION_KEY is unset or invalid; using development fallback key"
        );
        *b"dev_credential_key_32_bytes_ok!!"
    }

    /// Builds the provider configuration, when this deployment has a credential.
    pub fn openai(&self) -> Option<OpenAIConfig> {
        let key = self.openai_api_key.as_ref()?;
        let mut cfg = OpenAIConfig::new(key);
        cfg.model = self.model.clone();
        cfg.max_output_tokens = self.model_max_output_tokens;
        cfg.timeout = self.model_timeout;
        Some(cfg)
    }

    /// Path to the SQL migrations directory, relative to the workspace root.
    pub fn migrations_dir() -> PathBuf {
        PathBuf::from("migrations")
    }
}

fn decode_hex_32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
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

/// Redacts every secret.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("bind_addr", &self.bind_addr)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "<redacted>"),
            )
            .field("dev_auth_token", &"<redacted>")
            .field("supabase_project_ref", &self.supabase_project_ref)
            .field("allowed_origins", &self.allowed_origins)
            .field("log_filter", &self.log_filter)
            .field("max_tool_rounds", &self.max_tool_rounds)
            .field(
                "openai_api_key",
                &self.openai_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field(
                "openai_transcription_model",
                &self.openai_transcription_model,
            )
            .field(
                "openai_transcription_language",
                &self.openai_transcription_language,
            )
            .field("model", &self.model)
            .field("model_max_output_tokens", &self.model_max_output_tokens)
            .field("model_timeout", &self.model_timeout)
            .field("context_max_messages", &self.context_max_messages)
            .field("google_client_id", &self.google_client_id)
            .field(
                "google_client_secret",
                &self.google_client_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("google_redirect_uri", &self.google_redirect_uri)
            .field(
                "credential_encryption_key",
                &self
                    .credential_encryption_key
                    .as_ref()
                    .map(|_| "<redacted>"),
            )
            .field("document_storage_dir", &self.document_storage_dir)
            .finish()
    }
}
