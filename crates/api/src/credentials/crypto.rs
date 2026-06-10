//! AES-256-GCM envelope encryption for venue credentials (C-093).
//!
//! Flow:
//!   1. Generate a random 256-bit DEK.
//!   2. Encrypt plaintext credential bytes with AES-256-GCM (random 96-bit nonce).
//!   3. Wrap the DEK with the operator KEK (from env `CRED_KEK`).
//!   4. Persist `{ciphertext, nonce, wrapped_dek, key_version}`.
//!
//! The plaintext type deliberately does **not** derive `Debug`/`Display` in
//! any way that exposes credential bytes (C-093).

use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use thiserror::Error;

/// Errors from credential encryption/decryption.
#[derive(Debug, Error)]
pub enum CredentialError {
    #[error("encryption failed")]
    Encrypt,
    #[error("decryption failed")]
    Decrypt,
    #[error("invalid KEK length — must be 32 bytes")]
    InvalidKekLength,
    #[error("invalid DEK length — must be 32 bytes")]
    InvalidDekLength,
    #[error("env var `CRED_KEK` not set or not valid hex")]
    MissingKek,
}

/// Plaintext credential bytes.
///
/// `Debug` is intentionally redacted — never prints credential content.
pub struct PlaintextCredential {
    pub bytes: Vec<u8>,
}

impl PlaintextCredential {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl std::fmt::Debug for PlaintextCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlaintextCredential([REDACTED])")
    }
}

/// All fields persisted to the `venue_credentials` table.
#[derive(Debug, Clone)]
pub struct EncryptedCredential {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub wrapped_dek: Vec<u8>,
    pub key_version: i32,
}

/// AES-256-GCM envelope encryption service.
pub struct CredentialCrypto {
    /// Operator key-encryption key (32 bytes).
    kek: Vec<u8>,
    key_version: i32,
}

impl CredentialCrypto {
    /// Construct from an explicit 32-byte KEK (used in tests).
    pub fn new(kek: Vec<u8>, key_version: i32) -> Result<Self, CredentialError> {
        if kek.len() != 32 {
            return Err(CredentialError::InvalidKekLength);
        }
        Ok(Self { kek, key_version })
    }

    /// Construct from the `CRED_KEK` environment variable (hex-encoded 32 bytes).
    pub fn from_env() -> Result<Self, CredentialError> {
        let hex = std::env::var("CRED_KEK").map_err(|_| CredentialError::MissingKek)?;
        let kek = hex::decode(hex.trim()).map_err(|_| CredentialError::MissingKek)?;
        Self::new(kek, 1)
    }

    /// Encrypt `plaintext` with a freshly-generated DEK.  The DEK is then
    /// wrapped with the KEK so only the operator can unwrap it.
    pub fn encrypt(
        &self,
        plaintext: &PlaintextCredential,
    ) -> Result<EncryptedCredential, CredentialError> {
        // Generate fresh DEK.
        let dek = Aes256Gcm::generate_key(OsRng);

        // Encrypt plaintext with DEK.
        let cipher = Aes256Gcm::new(&dek);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext.bytes.as_slice())
            .map_err(|_| CredentialError::Encrypt)?;

        // Wrap DEK with KEK.
        let kek_key = Key::<Aes256Gcm>::from_slice(&self.kek);
        let kek_cipher = Aes256Gcm::new(kek_key);
        let dek_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let mut wrapped_dek = kek_cipher
            .encrypt(&dek_nonce, dek.as_slice())
            .map_err(|_| CredentialError::Encrypt)?;
        // Prepend DEK nonce to wrapped_dek for storage in a single field.
        let mut wrapped = dek_nonce.to_vec();
        wrapped.append(&mut wrapped_dek);

        Ok(EncryptedCredential {
            ciphertext,
            nonce: nonce.to_vec(),
            wrapped_dek: wrapped,
            key_version: self.key_version,
        })
    }

    /// Decrypt an `EncryptedCredential` back to plaintext bytes.
    pub fn decrypt(
        &self,
        enc: &EncryptedCredential,
    ) -> Result<PlaintextCredential, CredentialError> {
        // Unwrap DEK using KEK.
        // wrapped_dek = [12-byte nonce | ciphertext_of_dek]
        if enc.wrapped_dek.len() < 12 {
            return Err(CredentialError::Decrypt);
        }
        let (dek_nonce_bytes, dek_ciphertext) = enc.wrapped_dek.split_at(12);
        let dek_nonce = Nonce::from_slice(dek_nonce_bytes);

        let kek_key = Key::<Aes256Gcm>::from_slice(&self.kek);
        let kek_cipher = Aes256Gcm::new(kek_key);
        let dek_bytes = kek_cipher
            .decrypt(dek_nonce, dek_ciphertext)
            .map_err(|_| CredentialError::Decrypt)?;

        if dek_bytes.len() != 32 {
            return Err(CredentialError::InvalidDekLength);
        }

        // Decrypt ciphertext using DEK.
        let dek_key = Key::<Aes256Gcm>::from_slice(&dek_bytes);
        let cipher = Aes256Gcm::new(dek_key);
        let nonce = Nonce::from_slice(&enc.nonce);
        let plaintext = cipher
            .decrypt(nonce, enc.ciphertext.as_slice())
            .map_err(|_| CredentialError::Decrypt)?;

        Ok(PlaintextCredential::new(plaintext))
    }

    /// Encrypt, then verify, then persist — the save is gated on the verifier.
    ///
    /// `verifier` receives the plaintext and should return `Ok(())` if a live
    /// venue health-check passes.  If it returns `Err`, no credential is stored.
    pub fn verify_then_store<F>(
        &self,
        plaintext: &PlaintextCredential,
        verifier: F,
    ) -> Result<EncryptedCredential, CredentialError>
    where
        F: FnOnce(&[u8]) -> Result<(), CredentialError>,
    {
        verifier(&plaintext.bytes).map_err(|_| CredentialError::Encrypt)?;
        self.encrypt(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_kek() -> Vec<u8> {
        vec![0x42u8; 32]
    }

    fn crypto() -> CredentialCrypto {
        CredentialCrypto::new(test_kek(), 1).unwrap()
    }

    #[test]
    fn encrypt_decrypt_round_trips() {
        let original = b"api_key=secret123&api_secret=verysecret";
        let plaintext = PlaintextCredential::new(original.to_vec());
        let crypto = crypto();
        let enc = crypto.encrypt(&plaintext).unwrap();
        let dec = crypto.decrypt(&enc).unwrap();
        assert_eq!(dec.bytes, original);
    }

    #[test]
    fn two_encryptions_produce_different_ciphertext() {
        let plaintext = PlaintextCredential::new(b"same plaintext".to_vec());
        let crypto = crypto();
        let enc1 = crypto.encrypt(&plaintext).unwrap();
        let enc2 = crypto.encrypt(&plaintext).unwrap();
        assert_ne!(enc1.ciphertext, enc2.ciphertext, "random nonce must differ");
    }

    #[test]
    fn plaintext_debug_contains_no_credential_bytes() {
        let creds = PlaintextCredential::new(b"super_secret_key".to_vec());
        let debug_output = format!("{creds:?}");
        assert!(
            !debug_output.contains("super_secret_key"),
            "Debug output must not expose credential bytes: {debug_output}"
        );
        assert!(debug_output.contains("REDACTED"));
    }
}
