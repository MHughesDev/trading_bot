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
///
/// Every field is something the card prints. There is no field that could carry
/// the key, and `the_status_row_has_no_field_that_could_hold_a_key` holds that.
#[derive(Debug, serde::Serialize)]
pub struct LlmCredentialStatus {
    pub provider: String,
    pub configured: bool,
    pub key_last4: Option<String>,
    pub base_url: Option<String>,
    pub updated_at: Option<String>,
    /// When the stored key last answered a real provider call.
    pub verified_at: Option<String>,
    /// When it was last checked at all, whatever the answer.
    pub last_checked_at: Option<String>,
    /// Why the last check failed, scrubbed of the key. `None` when it passed.
    pub last_error: Option<String>,
    /// The model the desk starts this provider on.
    pub default_model: Option<String>,
    /// Whether this is the user's default provider.
    pub is_default: bool,
}

/// Every provider the status list reports, in the order the settings page draws
/// them: the two hosted providers a user brings a key for, then the local ones.
pub const STATUS_PROVIDERS: &[&str] = &["anthropic", "openai", "vllm", "ollama"];

/// The last four characters of a key, for display. `None` under eight, because
/// the last four of a very short key are most of the key.
#[must_use]
pub fn last4(api_key: &str) -> Option<String> {
    let chars: Vec<char> = api_key.chars().collect();
    (chars.len() >= 8).then(|| chars[chars.len() - 4..].iter().collect())
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
/// Row shape: (provider, key_last4, base_url, updated_at, verified_at,
/// last_checked_at, last_error, default_model, is_default).
type StatusRow = (
    String,
    Option<String>,
    Option<String>,
    chrono::DateTime<chrono::Utc>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
    Option<String>,
    Option<String>,
    bool,
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
    ///
    /// Callers verify the key against the provider first (the route does), so a
    /// save is also a verification: `verified_at` is stamped and any earlier
    /// failure is cleared. The user's first connected provider becomes their
    /// default, so a user who connects one key can start a conversation without
    /// a second trip to settings; a replaced key keeps whatever default it had.
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
        let key_last4 = last4(api_key);

        sqlx::query(
            "INSERT INTO llm_credentials
                 (user_id, provider, ciphertext, nonce, wrapped_dek, key_version, base_url, key_last4,
                  verified_at, last_checked_at, last_error, is_default)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now(), NULL,
                     NOT EXISTS (SELECT 1 FROM llm_credentials d WHERE d.user_id = $1 AND d.is_default))
             ON CONFLICT (user_id, provider) DO UPDATE SET
                 ciphertext = EXCLUDED.ciphertext,
                 nonce = EXCLUDED.nonce,
                 wrapped_dek = EXCLUDED.wrapped_dek,
                 key_version = EXCLUDED.key_version,
                 base_url = EXCLUDED.base_url,
                 key_last4 = EXCLUDED.key_last4,
                 verified_at = now(),
                 last_checked_at = now(),
                 last_error = NULL,
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

    /// Record the outcome of re-checking a stored key. `Err(reason)` must already
    /// be scrubbed of the key; it is truncated to the column's limit here.
    ///
    /// Returns `false` when the user has no credential for `provider`.
    pub async fn record_check(
        &self,
        user_id: Uuid,
        provider: &str,
        outcome: Result<(), &str>,
    ) -> Result<bool, LlmStoreError> {
        let reason: Option<String> = outcome.err().map(|r| r.chars().take(500).collect());
        let result = sqlx::query(
            "UPDATE llm_credentials SET
                 last_checked_at = now(),
                 verified_at = CASE WHEN $3::text IS NULL THEN now() ELSE verified_at END,
                 last_error = $3
             WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .bind(reason)
        .execute(&self.pg)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Make `provider` the user's default, starting on `model` (or keeping the
    /// model it had when `None`). One default per user: the others are cleared in
    /// the same transaction, and the partial unique index refuses a second.
    ///
    /// Returns `false` when the user has no credential for `provider`, so a
    /// default can never point at a provider nobody connected.
    pub async fn set_default(
        &self,
        user_id: Uuid,
        provider: &str,
        model: Option<&str>,
    ) -> Result<bool, LlmStoreError> {
        let mut tx = self.pg.begin().await?;
        let exists: Option<(i32,)> = sqlx::query_as(
            "SELECT 1 FROM llm_credentials WHERE user_id = $1 AND provider = $2 FOR UPDATE",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&mut *tx)
        .await?;
        if exists.is_none() {
            return Ok(false);
        }
        sqlx::query(
            "UPDATE llm_credentials SET is_default = false
              WHERE user_id = $1 AND provider <> $2 AND is_default",
        )
        .bind(user_id)
        .bind(provider)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE llm_credentials
                SET is_default = true, default_model = COALESCE($3, default_model)
              WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .bind(model)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
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
            "SELECT provider, key_last4, base_url, updated_at, verified_at,
                    last_checked_at, last_error, default_model, is_default
                 FROM llm_credentials WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await?;

        Ok(STATUS_PROVIDERS
            .iter()
            .map(|provider| match rows.iter().find(|r| r.0 == *provider) {
                Some((
                    _,
                    last4,
                    base_url,
                    updated_at,
                    verified_at,
                    last_checked_at,
                    last_error,
                    default_model,
                    is_default,
                )) => LlmCredentialStatus {
                    provider: (*provider).to_string(),
                    configured: true,
                    key_last4: last4.clone(),
                    base_url: base_url.clone(),
                    updated_at: Some(updated_at.to_rfc3339()),
                    verified_at: verified_at.map(|t| t.to_rfc3339()),
                    last_checked_at: last_checked_at.map(|t| t.to_rfc3339()),
                    last_error: last_error.clone(),
                    default_model: default_model.clone(),
                    is_default: *is_default,
                },
                None => LlmCredentialStatus {
                    provider: (*provider).to_string(),
                    configured: false,
                    key_last4: None,
                    base_url: None,
                    updated_at: None,
                    verified_at: None,
                    last_checked_at: None,
                    last_error: None,
                    default_model: None,
                    is_default: false,
                },
            })
            .collect())
    }

    /// Remove a credential. When it was the default, the default moves to the
    /// most recently verified provider the user still has, so "the default" never
    /// names a provider that is gone.
    pub async fn delete(&self, user_id: Uuid, provider: &str) -> Result<bool, LlmStoreError> {
        let mut tx = self.pg.begin().await?;
        let removed: Option<(bool,)> = sqlx::query_as(
            "DELETE FROM llm_credentials WHERE user_id = $1 AND provider = $2 RETURNING is_default",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((was_default,)) = removed else {
            return Ok(false);
        };
        if was_default {
            sqlx::query(
                "UPDATE llm_credentials SET is_default = true
                  WHERE id = (SELECT id FROM llm_credentials WHERE user_id = $1
                               ORDER BY verified_at DESC NULLS LAST, updated_at DESC LIMIT 1)",
            )
            .bind(user_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A masked key is the last four of a long key, and nothing for a short one:
    /// the last four of a five-character key are most of the key.
    #[test]
    fn a_short_key_is_never_shown_masked() {
        assert_eq!(last4("sk-ant-api03-abcdWXYZ").as_deref(), Some("WXYZ"));
        assert_eq!(last4("short"), None);
        assert_eq!(last4(""), None);
    }

    /// The status row is what the client receives. It has no field that could
    /// carry the key; adding one fails here before it can fail in a browser.
    #[test]
    fn the_status_row_has_no_field_that_could_hold_a_key() {
        let row = LlmCredentialStatus {
            provider: "openai".into(),
            configured: true,
            key_last4: Some("abcd".into()),
            base_url: None,
            updated_at: None,
            verified_at: None,
            last_checked_at: None,
            last_error: None,
            default_model: None,
            is_default: true,
        };
        let v = serde_json::to_value(&row).expect("serialises");
        let mut keys: Vec<&str> = v
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "base_url",
                "configured",
                "default_model",
                "is_default",
                "key_last4",
                "last_checked_at",
                "last_error",
                "provider",
                "updated_at",
                "verified_at"
            ]
        );
    }
}
