//! Configuration, resolved once at startup from the environment.
//!
//! Everything the process needs is read here and nowhere else, so there is a
//! single place to audit for secret handling. `Debug` is implemented by hand to
//! keep secrets out of logs.

use std::{fmt, net::SocketAddr, path::PathBuf, time::Duration};

use assistant_models::openai::OpenAIConfig;

/// Google's OpenAI-compatible chat endpoint. Named once so the completions path
/// and the transcription path cannot drift apart on which host they mean.
pub const GEMINI_OPENAI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

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
    /// The Gemini API credential (Google AI Studio).
    pub gemini_api_key: Option<String>,
    /// Custom base URL for OpenAI-compatible endpoints (e.g. Gemini OpenAI compatible REST endpoint).
    pub openai_base_url: Option<String>,
    /// Model used for audio transcription (e.g. whisper-1 or gemini-2.0-flash).
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
    /// Permits the development-only security defaults: `DevTokenVerifier` and
    /// the hardcoded credential encryption key. Without it, a configuration
    /// that would reach either one refuses to start rather than quietly serving
    /// an open API. See ADR-0039.
    pub allow_dev_auth: bool,
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

        let allow_dev_auth = std::env::var("ASSISTANT_ALLOW_DEV_AUTH")
            .ok()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"));

        let supabase_project_ref = std::env::var("SUPABASE_PROJECT_REF")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        // Refuse to start rather than fall back to a development default that
        // nobody asked for (ADR-0039). This runs before the listener binds.
        Self::validate_security(
            supabase_project_ref.as_deref(),
            credential_encryption_key.as_deref(),
            allow_dev_auth,
        )?;

        Ok(Self {
            bind_addr,
            database_url: std::env::var("DATABASE_URL").ok().filter(|s| !s.is_empty()),
            dev_auth_token,
            supabase_project_ref,
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
            gemini_api_key: std::env::var("GEMINI_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            openai_base_url: std::env::var("OPENAI_BASE_URL")
                .ok()
                .map(|url| url.trim().to_string())
                .filter(|url| !url.is_empty()),
            openai_transcription_model: env_or("OPENAI_TRANSCRIPTION_MODEL", "gemini-3.6-flash"),
            openai_transcription_language: std::env::var("OPENAI_TRANSCRIPTION_LANGUAGE")
                .ok()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty()),
            model: env_or("ASSISTANT_MODEL", "gemini-3.6-flash"),
            model_max_output_tokens: parse_env("ASSISTANT_MODEL_MAX_OUTPUT_TOKENS", "4096")?,
            model_timeout: Duration::from_millis(parse_env("ASSISTANT_MODEL_TIMEOUT_MS", "15000")?),
            context_max_messages: parse_env("ASSISTANT_CONTEXT_MAX_MESSAGES", "40")?,

            google_client_id,
            google_client_secret,
            google_redirect_uri,
            credential_encryption_key,
            allow_dev_auth,
            document_storage_dir: PathBuf::from(env_or(
                "ASSISTANT_DOCUMENT_STORAGE_DIR",
                "./data/documents",
            )),
            cartesia_api_key: std::env::var("CARTESIA_API_KEY")
                .ok()
                .map(|key| key.trim().to_string())
                .filter(|key| !key.is_empty()),
            cartesia_stt_model: env_or("CARTESIA_STT_MODEL", assistant_voice::DEFAULT_STT_MODEL),
            cartesia_tts_model: env_or("CARTESIA_TTS_MODEL", assistant_voice::DEFAULT_TTS_MODEL),
            cartesia_tts_voice_id: env_or(
                "CARTESIA_TTS_VOICE_ID",
                "a0e99841-438c-4a64-b679-ae501e7d6091",
            ),
        })
    }

    /// Rejects a configuration that would silently run on a development-only
    /// security default.
    ///
    /// Pure, and separate from `from_env`, so it can be unit-tested without
    /// mutating process-wide environment variables -- which these tests avoid
    /// because they run concurrently in one process. See ADR-0039.
    pub fn validate_security(
        supabase_project_ref: Option<&str>,
        credential_encryption_key: Option<&str>,
        allow_dev_auth: bool,
    ) -> Result<(), ConfigError> {
        // A malformed key is always an error. Accepting it silently is worse
        // than accepting none: the operator sets the variable, sees a running
        // server, and believes it took effect.
        if let Some(key) = credential_encryption_key
            && parse_encryption_key(key).is_none()
        {
            return Err(ConfigError::Invalid {
                name: "CREDENTIAL_ENCRYPTION_KEY",
                reason: "must be 64 hex characters or exactly 32 bytes".into(),
            });
        }

        if allow_dev_auth {
            return Ok(());
        }

        if supabase_project_ref.is_none() {
            return Err(ConfigError::Missing("SUPABASE_PROJECT_REF"));
        }
        if credential_encryption_key.is_none() {
            return Err(ConfigError::Missing("CREDENTIAL_ENCRYPTION_KEY"));
        }
        Ok(())
    }

    /// Resolves the 32-byte credential encryption key.
    /// If CREDENTIAL_ENCRYPTION_KEY is provided (hex or 32-byte ascii), it decodes it.
    /// If absent, falls back to a deterministic development key with a warning.
    pub fn resolved_encryption_key(&self) -> [u8; 32] {
        if let Some(bytes) = self
            .credential_encryption_key
            .as_deref()
            .and_then(parse_encryption_key)
        {
            return bytes;
        }
        // Unreachable unless `allow_dev_auth` is set: `validate_security`
        // refuses to build a `Config` that would land here (ADR-0039).
        tracing::warn!(
            "CREDENTIAL_ENCRYPTION_KEY is unset or invalid; using development fallback key"
        );
        *b"dev_credential_key_32_bytes_ok!!"
    }

    /// True when this deployment talks to Google's OpenAI-compatible endpoint
    /// rather than OpenAI's own.
    ///
    /// An explicit `OPENAI_BASE_URL` is the operator's choice and wins; failing
    /// that, a Gemini credential or a `gemini-*` model name selects Google.
    pub fn targets_gemini(&self) -> bool {
        match self.openai_base_url.as_deref() {
            Some(url) => url.contains("generativelanguage.googleapis.com"),
            None => self.gemini_api_key.is_some() || self.model.starts_with("gemini"),
        }
    }

    /// Builds the provider configuration, when this deployment has a credential.
    ///
    /// The credential and the endpoint are chosen together. They used to be
    /// chosen independently -- the key preferred `OPENAI_API_KEY` while the base
    /// URL was switched to Google whenever a Gemini key or model was present --
    /// so a deployment holding both keys sent the OpenAI credential to Google
    /// and every turn failed with `provider_invalid_request` /
    /// `400 Please pass a valid API key`.
    pub fn openai(&self) -> Option<OpenAIConfig> {
        let targets_gemini = self.targets_gemini();

        // Google's endpoint only accepts a Google credential, and OpenAI's only
        // accepts an OpenAI one. Neither is a usable fallback for the other, so
        // a missing key here is no provider at all rather than a call that is
        // certain to be rejected.
        let key = if targets_gemini {
            self.gemini_api_key.as_ref()?
        } else {
            self.openai_api_key.as_ref()?
        };

        let mut cfg = OpenAIConfig::new(key);
        if let Some(ref base_url) = self.openai_base_url {
            cfg.base_url = base_url.clone();
        } else if targets_gemini {
            cfg.base_url = GEMINI_OPENAI_BASE_URL.to_string();
        }
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

/// Accepts 64 hex characters or exactly 32 raw bytes. The single definition of
/// what a valid `CREDENTIAL_ENCRYPTION_KEY` is, so startup validation and key
/// resolution cannot disagree.
fn parse_encryption_key(key: &str) -> Option<[u8; 32]> {
    if let Some(bytes) = decode_hex_32(key) {
        return Some(bytes);
    }
    if key.len() == 32 {
        let mut out = [0u8; 32];
        out.copy_from_slice(key.as_bytes());
        return Some(out);
    }
    None
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
            .field("allow_dev_auth", &self.allow_dev_auth)
            .field("document_storage_dir", &self.document_storage_dir)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX_KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const RAW_KEY: &str = "dev_credential_key_32_bytes_ok!!";

    /// The M13 regression. A deployment that sets neither `SUPABASE_PROJECT_REF`
    /// nor the opt-in used to boot on `DevTokenVerifier`, which authenticates
    /// every caller as one fixed user. It must now refuse (ADR-0039).
    #[test]
    fn a_missing_supabase_project_is_refused_without_the_opt_in() {
        let error = Config::validate_security(None, Some(HEX_KEY), false)
            .expect_err("a missing project ref must not silently select the dev verifier");
        assert!(matches!(
            error,
            ConfigError::Missing("SUPABASE_PROJECT_REF")
        ));
    }

    /// The other half: falling back to a key that is a constant in public source
    /// would let anyone forge the OAuth `state` the unauthenticated callback
    /// trusts.
    #[test]
    fn a_missing_encryption_key_is_refused_without_the_opt_in() {
        let error = Config::validate_security(Some("abcdefg"), None, false)
            .expect_err("a missing key must not silently select the development fallback");
        assert!(matches!(
            error,
            ConfigError::Missing("CREDENTIAL_ENCRYPTION_KEY")
        ));
    }

    /// A set-but-unparseable key is an error even under the opt-in: the operator
    /// believes the value took effect, and it did not.
    #[test]
    fn a_malformed_encryption_key_is_refused_even_with_the_opt_in() {
        for allow_dev_auth in [false, true] {
            let error = Config::validate_security(Some("proj"), Some("too-short"), allow_dev_auth)
                .expect_err("a malformed key must never fall through to the fallback");
            assert!(matches!(
                error,
                ConfigError::Invalid {
                    name: "CREDENTIAL_ENCRYPTION_KEY",
                    ..
                }
            ));
        }
    }

    #[test]
    fn the_opt_in_permits_the_development_defaults() {
        Config::validate_security(None, None, true).expect("offline development stays possible");
    }

    #[test]
    fn a_fully_configured_deployment_passes() {
        Config::validate_security(Some("proj"), Some(HEX_KEY), false).expect("valid");
        Config::validate_security(Some("proj"), Some(RAW_KEY), false).expect("valid");
    }

    #[test]
    fn both_key_encodings_parse_and_hex_wins_on_length() {
        assert_eq!(parse_encryption_key(HEX_KEY).expect("hex")[0], 0x01);
        assert_eq!(
            parse_encryption_key(RAW_KEY).expect("raw"),
            *b"dev_credential_key_32_bytes_ok!!"
        );
        assert!(parse_encryption_key("short").is_none());
        assert!(parse_encryption_key(&"z".repeat(64)).is_none());
    }
}
