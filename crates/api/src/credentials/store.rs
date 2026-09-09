//! Encrypted per-user LLM credential storage (`llm_credentials` table).
//!
//! API keys are envelope-encrypted with [`CredentialCrypto`]; the key is never
//! returned by any read path except [`LlmCredentialStore::load`], which is
//! used only to construct an outbound provider client.

use std::sync::Arc;

use sqlx::PgPool;
use uuid::Uuid;

use super::crypto::{CredentialCrypto, CredentialError, EncryptedCredential, PlaintextCredential};

/// One provider's stored (decrypted) credential.
pub struct LlmCredential {
    /// Empty for keyless providers (Ollama).
    pub api_key: String,
    pub base_url: Option<String>,
}

/// Display-safe status row (no secret material).
#[derive(Debug, serde::Serialize)]
pub struct LlmCredentialStatus {
    pub provider: String,
    pub configured: bool,
    pub key_last4: Option<String>,
    pub base_url: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum LlmStoreError {
    #[error("credential crypto unavailable — CRED_KEK not configured")]
    CryptoUnavailable,
    #[error(transparent)]
    Crypto(#[from] CredentialError),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Row shape: (ciphertext, nonce, wrapped_dek, key_version, base_url).
type EncryptedRow = (Vec<u8>, Vec<u8>, Vec<u8>, i32, Option<String>);
/// Row shape: (provider, key_last4, base_url, updated_at).
type StatusRow = (
    String,
    Option<String>,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
);

/// Store + crypto pair used by the LLM routes and the agent driver.
#[derive(Clone)]
pub struct LlmCredentialStore {
    pg: PgPool,
    crypto: Option<Arc<CredentialCrypto>>,
}

impl LlmCredentialStore {
    pub fn new(pg: PgPool, crypto: Option<Arc<CredentialCrypto>>) -> Self {
        Self { pg, crypto }
    }

    pub fn crypto_available(&self) -> bool {
        self.crypto.is_some()
    }

    fn crypto(&self) -> Result<&CredentialCrypto, LlmStoreError> {
        self.crypto
            .as_deref()
            .ok_or(LlmStoreError::CryptoUnavailable)
    }

    /// Encrypt and upsert a provider credential.
    pub async fn save(
        &self,
        user_id: Uuid,
        provider: &str,
        api_key: &str,
        base_url: Option<&str>,
    ) -> Result<(), LlmStoreError> {
        let enc = self
            .crypto()?
            .encrypt(&PlaintextCredential::new(api_key.as_bytes().to_vec()))?;
        let key_last4: Option<String> = if api_key.len() >= 4 {
            Some(
                api_key
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect(),
            )
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO llm_credentials
                 (user_id, provider, ciphertext, nonce, wrapped_dek, key_version, base_url, key_last4)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (user_id, provider) DO UPDATE SET
                 ciphertext = EXCLUDED.ciphertext,
                 nonce = EXCLUDED.nonce,
                 wrapped_dek = EXCLUDED.wrapped_dek,
                 key_version = EXCLUDED.key_version,
                 base_url = EXCLUDED.base_url,
                 key_last4 = EXCLUDED.key_last4,
                 updated_at = now()",
        )
        .bind(user_id)
        .bind(provider)
        .bind(&enc.ciphertext)
        .bind(&enc.nonce)
        .bind(&enc.wrapped_dek)
        .bind(enc.key_version)
        .bind(base_url)
        .bind(key_last4)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Load and decrypt one provider credential.
    pub async fn load(
        &self,
        user_id: Uuid,
        provider: &str,
    ) -> Result<Option<LlmCredential>, LlmStoreError> {
        let row: Option<EncryptedRow> = sqlx::query_as(
            "SELECT ciphertext, nonce, wrapped_dek, key_version, base_url
             FROM llm_credentials WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.pg)
        .await?;

        let Some((ciphertext, nonce, wrapped_dek, key_version, base_url)) = row else {
            return Ok(None);
        };
        let plaintext = self.crypto()?.decrypt(&EncryptedCredential {
            ciphertext,
            nonce,
            wrapped_dek,
            key_version,
        })?;
        Ok(Some(LlmCredential {
            api_key: String::from_utf8_lossy(&plaintext.bytes).into_owned(),
            base_url,
        }))
    }

    /// Status for every provider (configured or not), never exposing keys.
    pub async fn status(&self, user_id: Uuid) -> Result<Vec<LlmCredentialStatus>, LlmStoreError> {
        let rows: Vec<StatusRow> = sqlx::query_as(
            "SELECT provider, key_last4, base_url, updated_at
                 FROM llm_credentials WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await?;

        Ok(["openai", "anthropic", "ollama"]
            .iter()
            .map(
                |provider| match rows.iter().find(|(p, _, _, _)| p == provider) {
                    Some((_, last4, base_url, updated_at)) => LlmCredentialStatus {
                        provider: (*provider).to_string(),
                        configured: true,
                        key_last4: last4.clone(),
                        base_url: base_url.clone(),
                        updated_at: Some(updated_at.to_rfc3339()),
                    },
                    None => LlmCredentialStatus {
                        provider: (*provider).to_string(),
                        configured: false,
                        key_last4: None,
                        base_url: None,
                        updated_at: None,
                    },
                },
            )
            .collect())
    }

    pub async fn delete(&self, user_id: Uuid, provider: &str) -> Result<bool, LlmStoreError> {
        let result =
            sqlx::query("DELETE FROM llm_credentials WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(provider)
                .execute(&self.pg)
                .await?;
        Ok(result.rows_affected() > 0)
    }
}
