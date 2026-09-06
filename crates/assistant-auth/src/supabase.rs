//! Supabase Auth token verification. See ADR-0024.
//!
//! The project's GoTrue instance signs access tokens with ES256 and publishes
//! the matching public key at its JWKS endpoint. Verification therefore needs
//! no secret: this server holds only a public key, and a leak of its entire
//! configuration still mints nobody a token.
//!
//! What is trusted from a verified token is deliberately small. `sub` becomes
//! the [`Principal`]'s user id, and that is all the authority a token carries.
//! Scopes and roles inside the JWT are *not* read as tool permissions --
//! `RiskLevel` is a static property of a `ToolSpec` and is evaluated by
//! deterministic code (ADR-0005). A token says who is calling, never what they
//! may do.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
use serde::Deserialize;
use tokio::{
    sync::RwLock,
    time::{Instant, timeout},
};
use uuid::Uuid;

use crate::{AuthError, Principal, TokenVerifier};

/// How long a successfully fetched key set is reused before it is considered
/// stale. Supabase rotates rarely; an unknown `kid` forces a refetch anyway, so
/// this only bounds how long a *withdrawn* key stays accepted.
const JWKS_TTL: Duration = Duration::from_secs(15 * 60);

/// The floor between two refetches. Without it, a stream of tokens bearing
/// unknown `kid`s would drive one outbound request each -- an amplifier
/// reachable by anyone who can send this server a string.
const JWKS_MIN_REFETCH_INTERVAL: Duration = Duration::from_secs(30);

/// Ceiling on a single JWKS fetch, so a hung endpoint cannot hold request
/// tasks open indefinitely.
const JWKS_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// The audience GoTrue stamps on an end-user access token.
const EXPECTED_AUDIENCE: &str = "authenticated";

/// Tolerance for clock skew between this server and GoTrue when checking
/// `exp`. Replaces `jsonwebtoken`'s 60-second default, which is long enough to
/// matter for a short-lived access token.
const CLOCK_SKEW_LEEWAY: Duration = Duration::from_secs(5);

/// Claims this server reads. Everything else in the token is ignored.
#[derive(Debug, Deserialize)]
struct Claims {
    /// The GoTrue user id. A UUID, and the only identity input.
    sub: String,
}

/// Where the public key set comes from.
///
/// `Static` exists so the verifier's own tests exercise real signature
/// checking without a network round trip.
pub enum JwksSource {
    Remote {
        url: String,
        client: reqwest::Client,
    },
    Static(JwkSet),
}

struct Cached {
    keys: JwkSet,
    fetched_at: Instant,
}

/// Verifies a Supabase-issued ES256 access token.
pub struct SupabaseJwtVerifier {
    source: JwksSource,
    issuer: String,
    cache: RwLock<Option<Cached>>,
    last_attempt: RwLock<Option<Instant>>,
}

impl SupabaseJwtVerifier {
    /// Builds a verifier for a Supabase project reference.
    ///
    /// The issuer and JWKS URL are derived from the reference rather than
    /// configured separately, because a mismatch between them is a silent
    /// failure: tokens from one project verified against another project's keys
    /// simply never validate, and the error looks like a bad token.
    pub fn for_project(project_ref: &str, client: reqwest::Client) -> Self {
        Self {
            source: JwksSource::Remote {
                url: format!("https://{project_ref}.supabase.co/auth/v1/.well-known/jwks.json"),
                client,
            },
            issuer: format!("https://{project_ref}.supabase.co/auth/v1"),
            cache: RwLock::new(None),
            last_attempt: RwLock::new(None),
        }
    }

    /// Builds a verifier over a fixed key set. Tests and offline use only.
    pub fn with_static_keys(issuer: impl Into<String>, keys: JwkSet) -> Self {
        Self {
            source: JwksSource::Static(keys),
            issuer: issuer.into(),
            cache: RwLock::new(None),
            last_attempt: RwLock::new(None),
        }
    }

    /// Returns the decoding key for `kid`, refetching at most once if it is
    /// unknown and the rate limit allows.
    async fn decoding_key(&self, kid: &str) -> Result<DecodingKey, AuthError> {
        if let JwksSource::Static(keys) = &self.source {
            return key_from_set(keys, kid);
        }

        // Fast path: a warm, unexpired cache that already knows this `kid`.
        {
            let cache = self.cache.read().await;
            if let Some(cached) = cache.as_ref()
                && cached.fetched_at.elapsed() < JWKS_TTL
                && let Ok(key) = key_from_set(&cached.keys, kid)
            {
                return Ok(key);
            }
        }

        // An unknown or stale `kid`. Refetch, but never more often than the
        // floor -- otherwise this path is an outbound-request amplifier.
        {
            let mut last = self.last_attempt.write().await;
            if let Some(at) = *last
                && at.elapsed() < JWKS_MIN_REFETCH_INTERVAL
            {
                return Err(AuthError::Invalid);
            }
            *last = Some(Instant::now());
        }

        let keys = self.fetch().await?;
        let key = key_from_set(&keys, kid)?;
        *self.cache.write().await = Some(Cached {
            keys,
            fetched_at: Instant::now(),
        });
        Ok(key)
    }

    async fn fetch(&self) -> Result<JwkSet, AuthError> {
        let JwksSource::Remote { url, client } = &self.source else {
            return Err(AuthError::Invalid);
        };

        let response = timeout(JWKS_FETCH_TIMEOUT, client.get(url).send())
            .await
            .map_err(|_| {
                tracing::warn!("timed out fetching the Supabase key set");
                AuthError::Invalid
            })?
            .map_err(|error| {
                // The URL is safe to log; it is public and holds no secret.
                tracing::warn!(%error, "could not fetch the Supabase key set");
                AuthError::Invalid
            })?;

        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Supabase key set request failed");
            return Err(AuthError::Invalid);
        }

        response.json::<JwkSet>().await.map_err(|error| {
            tracing::warn!(%error, "Supabase key set was not valid JWKS JSON");
            AuthError::Invalid
        })
    }
}

fn key_from_set(keys: &JwkSet, kid: &str) -> Result<DecodingKey, AuthError> {
    let jwk = keys.find(kid).ok_or(AuthError::Invalid)?;
    DecodingKey::from_jwk(jwk).map_err(|_| AuthError::Invalid)
}

#[async_trait]
impl TokenVerifier for SupabaseJwtVerifier {
    async fn verify(&self, token: &str) -> Result<Principal, AuthError> {
        if token.is_empty() {
            return Err(AuthError::Missing);
        }

        // The header is unauthenticated input; it selects a key and nothing
        // else. `alg` is not taken from it -- `Validation` below fixes ES256,
        // which is what stops the classic "alg: none" and
        // algorithm-confusion substitutions.
        let header = jsonwebtoken::decode_header(token).map_err(|_| AuthError::Invalid)?;
        let kid = header.kid.ok_or(AuthError::Invalid)?;
        let key = self.decoding_key(&kid).await?;

        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[EXPECTED_AUDIENCE]);
        // `exp` is required rather than merely checked when present, so a token
        // minted without one is rejected instead of living forever.
        validation.set_required_spec_claims(&["exp", "sub", "aud", "iss"]);
        // `jsonwebtoken` defaults to 60 seconds of leeway, which keeps an
        // expired token working for a minute after it expires. That is a
        // generous window to inherit silently from a library default, so the
        // tolerance is stated here instead: enough for ordinary clock skew
        // between this server and GoTrue, and no more.
        validation.leeway = CLOCK_SKEW_LEEWAY.as_secs();

        let data = jsonwebtoken::decode::<Claims>(token, &key, &validation)
            .map_err(|_| AuthError::Invalid)?;

        let user_id = Uuid::parse_str(&data.claims.sub).map_err(|_| AuthError::Invalid)?;

        Ok(Principal {
            user_id,
            // Deliberately empty. Tool authority is resolved from the registry
            // and the policy, never from claims the token carries (ADR-0005).
            scopes: Vec::new(),
        })
    }
}

/// Convenience for callers that hold the verifier behind the trait object.
impl From<SupabaseJwtVerifier> for Arc<dyn TokenVerifier> {
    fn from(verifier: SupabaseJwtVerifier) -> Self {
        Arc::new(verifier)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    const ISSUER: &str = "https://test-project.supabase.co/auth/v1";
    const KID: &str = "test-key-1";

    /// Throwaway P-256 key, generated for this test file and used nowhere else.
    const TEST_PRIVATE_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgVOQqob91G75xk4Xm\n\
GVz+HmkOQT9VHgeXV3D+r+5mlJShRANCAASRStAqV7WKIoohfXLaCnb2Bf8CTyq+\n\
ZyucmYn5hpdZEqHPfjTQ9ICwCXpIPfoRWo3AL+ChSymmQgH5p6TDnus9\n\
-----END PRIVATE KEY-----\n";

    const TEST_X: &str = "kUrQKle1iiKKIX1y2gp29gX_Ak8qvmcrnJmJ-YaXWRI";
    const TEST_Y: &str = "oc9-NND0gLAJekg9-hFajcAv4KFLKaZCAfmnpMOe6z0";

    fn key_set() -> JwkSet {
        serde_json::from_value(serde_json::json!({
            "keys": [{
                "kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig",
                "kid": KID, "x": TEST_X, "y": TEST_Y
            }]
        }))
        .expect("a well-formed key set")
    }

    fn verifier() -> SupabaseJwtVerifier {
        SupabaseJwtVerifier::with_static_keys(ISSUER, key_set())
    }

    fn sign(claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(KID.to_string());
        encode(
            &header,
            &claims,
            &EncodingKey::from_ec_pem(TEST_PRIVATE_PEM.as_bytes()).expect("a usable private key"),
        )
        .expect("signed")
    }

    fn expiry(offset_secs: i64) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("a clock after 1970")
            .as_secs() as i64;
        now + offset_secs
    }

    fn valid_claims(sub: &str) -> serde_json::Value {
        serde_json::json!({
            "sub": sub, "iss": ISSUER, "aud": EXPECTED_AUDIENCE, "exp": expiry(600)
        })
    }

    #[tokio::test]
    async fn a_valid_token_yields_the_subject_as_the_user_id() {
        let user_id = Uuid::new_v4();
        let token = sign(valid_claims(&user_id.to_string()));

        let principal = verifier().verify(&token).await.expect("accepted");

        assert_eq!(principal.user_id, user_id);
    }

    #[tokio::test]
    async fn claims_never_grant_scopes() {
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": ISSUER,
            "aud": EXPECTED_AUDIENCE,
            "exp": expiry(600),
            // A token that tries to award itself authority.
            "scopes": ["gmail.send", "admin"],
            "role": "service_role",
        }));

        let principal = verifier().verify(&token).await.expect("accepted");

        assert!(
            principal.scopes.is_empty(),
            "a token must not be able to grant itself tool scopes"
        );
    }

    #[tokio::test]
    async fn an_expired_token_is_rejected() {
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": ISSUER, "aud": EXPECTED_AUDIENCE,
            "exp": expiry(-600),
        }));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn expiry_leeway_is_seconds_rather_than_the_library_default_minute() {
        // Guards the explicit `leeway`. With `jsonwebtoken`'s default of 60s
        // this token -- expired half a minute ago -- would still be accepted.
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": ISSUER, "aud": EXPECTED_AUDIENCE,
            "exp": expiry(-30),
        }));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn a_token_from_another_issuer_is_rejected() {
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": "https://someone-else.supabase.co/auth/v1",
            "aud": EXPECTED_AUDIENCE,
            "exp": expiry(600),
        }));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn an_anon_audience_token_is_rejected() {
        // GoTrue issues `anon` tokens too. They authenticate nobody.
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": ISSUER, "aud": "anon", "exp": expiry(600),
        }));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn a_token_with_no_expiry_is_rejected() {
        let token = sign(serde_json::json!({
            "sub": Uuid::new_v4().to_string(),
            "iss": ISSUER, "aud": EXPECTED_AUDIENCE,
        }));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn a_token_signed_by_an_unknown_key_is_rejected() {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("some-other-kid".to_string());
        let token = encode(
            &header,
            &valid_claims(&Uuid::new_v4().to_string()),
            &EncodingKey::from_ec_pem(TEST_PRIVATE_PEM.as_bytes()).expect("key"),
        )
        .expect("signed");

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn an_unsigned_alg_none_token_is_rejected() {
        // The classic downgrade: a well-formed header asking for no signature.
        let token = format!(
            "{}.{}.",
            base64_url(br#"{"alg":"none","typ":"JWT","kid":"test-key-1"}"#),
            base64_url(br#"{"sub":"x","iss":"i","aud":"authenticated","exp":9999999999}"#),
        );

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn a_subject_that_is_not_a_uuid_is_rejected() {
        let token = sign(valid_claims("not-a-uuid"));

        assert_eq!(verifier().verify(&token).await, Err(AuthError::Invalid));
    }

    #[tokio::test]
    async fn an_empty_token_reports_missing_rather_than_invalid() {
        assert_eq!(verifier().verify("").await, Err(AuthError::Missing));
    }

    #[tokio::test]
    async fn garbage_is_rejected_without_panicking() {
        for candidate in ["....", "a.b.c", "not a jwt at all", "..", "ey.ey.ey"] {
            assert!(
                verifier().verify(candidate).await.is_err(),
                "{candidate:?} must not be accepted"
            );
        }
    }

    fn base64_url(bytes: &[u8]) -> String {
        use std::fmt::Write;
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                chunk.get(1).copied().unwrap_or(0),
                chunk.get(2).copied().unwrap_or(0),
            ];
            let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
            let take = chunk.len() + 1;
            for i in 0..take {
                let _ = write!(
                    out,
                    "{}",
                    ALPHABET[((n >> (18 - 6 * i)) & 0x3F) as usize] as char
                );
            }
        }
        out
    }
}
