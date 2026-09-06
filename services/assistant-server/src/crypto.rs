//! AES-256-GCM credential encryption at rest. See ADR-0025.
//!
//! Encrypts sensitive OAuth refresh tokens and credentials before persisting to PostgreSQL.
//! Payload format: `[12-byte Nonce][Ciphertext + 16-byte GCM Tag]`.

use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng},
};
use thiserror::Error;

pub const NONCE_LEN: usize = 12;
pub const TAG_LEN: usize = 16;
pub const MIN_PAYLOAD_LEN: usize = NONCE_LEN + TAG_LEN;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("payload too short (expected at least {MIN_PAYLOAD_LEN} bytes, got {0})")]
    PayloadTooShort(usize),
    #[error("decryption failed: authentication tag mismatch or corrupted ciphertext")]
    DecryptionFailed,
    #[error("encryption failed")]
    EncryptionFailed,
}

/// Encrypts plaintext bytes using AES-256-GCM with a freshly generated random nonce.
pub fn encrypt_credentials(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new(key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|_| CryptoError::EncryptionFailed)?;

    let mut payload = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    payload.extend_from_slice(&nonce);
    payload.extend_from_slice(&ciphertext);

    Ok(payload)
}

/// Decrypts a payload containing `[12-byte nonce][ciphertext + tag]` using AES-256-GCM.
pub fn decrypt_credentials(key: &[u8; 32], payload: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if payload.len() < MIN_PAYLOAD_LEN {
        return Err(CryptoError::PayloadTooShort(payload.len()));
    }

    let (nonce_bytes, ciphertext) = payload.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new(key.into());
    #[allow(deprecated)]
    let nonce = Nonce::from_slice(nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::DecryptionFailed)?;

    Ok(plaintext)
}

pub fn encrypt(plaintext: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    encrypt_credentials(key, plaintext)
}

pub fn decrypt(payload: &[u8], key: &[u8; 32]) -> Result<Vec<u8>, CryptoError> {
    decrypt_credentials(key, payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encryption_decryption() {
        let key = [0x42u8; 32];
        let secret = b"super_secret_oauth_refresh_token_12345";

        let encrypted = encrypt_credentials(&key, secret).expect("encryption succeeds");
        assert_ne!(encrypted, secret);
        assert!(encrypted.len() >= secret.len() + MIN_PAYLOAD_LEN);

        let decrypted = decrypt_credentials(&key, &encrypted).expect("decryption succeeds");
        assert_eq!(decrypted, secret);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let key_a = [0x01u8; 32];
        let key_b = [0x02u8; 32];
        let secret = b"sensitive_token";

        let encrypted = encrypt_credentials(&key_a, secret).unwrap();
        let result = decrypt_credentials(&key_b, &encrypted);

        assert_eq!(result, Err(CryptoError::DecryptionFailed));
    }

    #[test]
    fn tampering_detected() {
        let key = [0x07u8; 32];
        let secret = b"tamper_proof_payload";

        let mut encrypted = encrypt_credentials(&key, secret).unwrap();
        // Flip one byte in ciphertext
        let last_idx = encrypted.len() - 1;
        encrypted[last_idx] ^= 0xFF;

        let result = decrypt_credentials(&key, &encrypted);
        assert_eq!(result, Err(CryptoError::DecryptionFailed));
    }

    #[test]
    fn payload_too_short() {
        let key = [0x11u8; 32];
        let short = vec![0u8; 10];
        assert_eq!(
            decrypt_credentials(&key, &short),
            Err(CryptoError::PayloadTooShort(10))
        );
    }
}
