//! Authentication seam.
//!
//! Milestone 0 ships exactly one verifier: [`DevTokenVerifier`], a shared static
//! bearer token. It exists so the extraction point is real code rather than a
//! `TODO`, and so wiring Supabase JWT verification later touches one file
//! instead of every handler. It is not a security control.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: Uuid,
    /// Scopes granted to this caller. Per-connected-account scopes will be
    /// resolved separately when integrations land.
    pub scopes: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("no credentials were presented")]
    Missing,
    #[error("credentials were rejected")]
    Invalid,
}

#[async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Verifies a bearer token. Implementations must never log the token.
    async fn verify(&self, token: &str) -> Result<Principal, AuthError>;
}

/// Development-only verifier: accepts one constant token and maps it to one
/// fixed local user.
///
/// # Warning
///
/// This performs a constant comparison against a shared secret read from the
/// environment. It has no expiry, no revocation and no per-user identity. Do not
/// deploy a build that uses it outside local development.
pub struct DevTokenVerifier {
    token: String,
    principal: Principal,
}

impl DevTokenVerifier {
    /// The stable, obviously-fake user id used by every development session.
    pub const DEV_USER_ID: Uuid = Uuid::from_u128(0xDEAD_BEEF_0000_4000_8000_0000_0000_0001);

    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            principal: Principal {
                user_id: Self::DEV_USER_ID,
                scopes: vec!["dev".to_string()],
            },
        }
    }
}

#[async_trait]
impl TokenVerifier for DevTokenVerifier {
    async fn verify(&self, token: &str) -> Result<Principal, AuthError> {
        if token.is_empty() {
            return Err(AuthError::Missing);
        }
        if token == self.token {
            Ok(self.principal.clone())
        } else {
            Err(AuthError::Invalid)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn correct_token_yields_the_dev_principal() {
        let verifier = DevTokenVerifier::new("secret");
        let principal = verifier.verify("secret").await.expect("accepted");
        assert_eq!(principal.user_id, DevTokenVerifier::DEV_USER_ID);
    }

    #[tokio::test]
    async fn wrong_and_empty_tokens_are_rejected() {
        let verifier = DevTokenVerifier::new("secret");
        assert_eq!(verifier.verify("nope").await, Err(AuthError::Invalid));
        assert_eq!(verifier.verify("").await, Err(AuthError::Missing));
    }
}
