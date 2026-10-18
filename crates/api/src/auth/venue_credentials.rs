//! `/auth/venue-credentials` — per-user, envelope-encrypted venue API keys (Set E 1.2).
//!
//! The SPA has called these routes since the auth UX shipped (`SignUpPage`,
//! `Settings → VenueCredentials`); none existed, so every save 404'd. They now
//! store each venue's fields as one AES-256-GCM envelope
//! ([`crate::credentials::CredentialCrypto`]) in `venue_credentials`
//! (columns completed by migration 0150).
//!
//! # What never leaves the server
//!
//! A read returns the venue, the **names** of the stored fields, the last four
//! characters of the first field and when it was saved. No route decrypts.
//! [`VenueCredentialStore::load`] is the only decrypting path, for a live
//! adapter to build its client (Set E 3.x).
//!
//! # Refusals
//!
//! * No `CRED_KEK` → 503. Storing a credential the platform cannot encrypt is
//!   not a degraded mode; it is a plaintext secret in a database.
//! * `POST /auth/venue-credentials/verify` → 501. Verifying means calling the
//!   venue, which is the adapter work of Set E 3.x; answering "verified" without
//!   calling anything is the fabricated-success shape this repo keeps removing.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use super::session::BearerToken;
use crate::credentials::{CredentialCrypto, EncryptedCredential, PlaintextCredential};
use crate::state::AppState;

/// Venues a credential may be stored for. A slug outside this list is refused
/// rather than stored under a name no adapter will ever look up.
pub const KNOWN_VENUES: &[&str] = &[
    "alpaca", "coinbase", "kraken", "oanda", "tradier", "tradovate", "kalshi",
];

const MAX_FIELDS: usize = 8;
const MAX_FIELD_NAME: usize = 32;
const MAX_FIELD_VALUE: usize = 4096;

/// Display-safe status for one stored venue credential.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VenueCredentialStatus {
    pub venue: String,
    pub field_names: Vec<String>,
    pub key_last4: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum VenueCredentialError {
    #[error("credential crypto unavailable — CRED_KEK not configured")]
    CryptoUnavailable,
    #[error("{0}")]
    Invalid(String),
    #[error("credential crypto failed")]
    Crypto,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Validate and group a request body into `venue → {field → value}`.
///
/// Two shapes are accepted, because the SPA sends both:
/// * `{"venue": "tradier", "api_key": "…"}` — one venue, plain field names
///   (`Settings → VenueCredentials`);
/// * `{"alpaca_api_key": "…", "coinbase_api_secret": "…"}` — venue-prefixed
///   fields, possibly several venues (`SignUpPage`).
///
/// # Errors
/// [`VenueCredentialError::Invalid`] naming what was wrong.
pub fn parse_body(
    body: &Value,
) -> Result<BTreeMap<String, BTreeMap<String, String>>, VenueCredentialError> {
    let invalid = |m: String| VenueCredentialError::Invalid(m);
    let obj = body
        .as_object()
        .ok_or_else(|| invalid("body must be a JSON object".into()))?;

    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let explicit = obj.get("venue").and_then(Value::as_str);
    for (key, value) in obj {
        if key == "venue" {
            continue;
        }
        let value = value
            .as_str()
            .ok_or_else(|| invalid(format!("field {key:?} must be a string")))?;
        if value.is_empty() {
            continue;
        }
        let (venue, field) = match explicit {
            Some(v) => (v.to_owned(), key.clone()),
            None => {
                let (v, f) = key.split_once('_').ok_or_else(|| {
                    invalid(format!(
                        "field {key:?} names no venue; send \"venue\" or prefix the field with one"
                    ))
                })?;
                (v.to_owned(), f.to_owned())
            }
        };
        out.entry(venue).or_default().insert(field, value.to_owned());
    }

    if out.is_empty() {
        return Err(invalid("no credential fields were sent".into()));
    }
    for (venue, fields) in &out {
        if !KNOWN_VENUES.contains(&venue.as_str()) {
            return Err(invalid(format!(
                "unknown venue {venue:?}; expected one of {KNOWN_VENUES:?}"
            )));
        }
        if fields.len() > MAX_FIELDS {
            return Err(invalid(format!("too many fields for {venue}")));
        }
        for (name, value) in fields {
            let name_ok = !name.is_empty()
                && name.len() <= MAX_FIELD_NAME
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !name_ok {
                return Err(invalid(format!("field name {name:?} is not [a-z0-9_]{{1,32}}")));
            }
            if value.len() > MAX_FIELD_VALUE {
                return Err(invalid(format!("field {name:?} is too long")));
            }
        }
    }
    Ok(out)
}

fn last4(s: &str) -> Option<String> {
    let chars: Vec<char> = s.chars().collect();
    (chars.len() >= 8).then(|| chars[chars.len() - 4..].iter().collect())
}

/// Store + crypto for venue credentials.
#[derive(Clone)]
pub struct VenueCredentialStore {
    pg: PgPool,
    crypto: Option<Arc<CredentialCrypto>>,
}

type EncryptedRow = (Vec<u8>, Vec<u8>, Vec<u8>, i32);
type StatusRow = (String, Vec<String>, Option<String>, chrono::DateTime<chrono::Utc>);

impl VenueCredentialStore {
    #[must_use]
    pub fn new(pg: PgPool, crypto: Option<Arc<CredentialCrypto>>) -> Self {
        Self { pg, crypto }
    }

    fn crypto(&self) -> Result<&CredentialCrypto, VenueCredentialError> {
        self.crypto
            .as_deref()
            .ok_or(VenueCredentialError::CryptoUnavailable)
    }

    /// Encrypt and upsert one venue's fields for `user_id`.
    ///
    /// # Errors
    /// No KEK, a crypto failure or a database error.
    pub async fn save(
        &self,
        user_id: Uuid,
        venue: &str,
        fields: &BTreeMap<String, String>,
    ) -> Result<(), VenueCredentialError> {
        let crypto = self.crypto()?;
        let bytes = serde_json::to_vec(fields).map_err(|_| VenueCredentialError::Crypto)?;
        let enc = crypto
            .encrypt(&PlaintextCredential::new(bytes))
            .map_err(|_| VenueCredentialError::Crypto)?;
        let names: Vec<String> = fields.keys().cloned().collect();
        let key_last4 = fields.values().next().and_then(|v| last4(v));
        sqlx::query(
            "INSERT INTO venue_credentials \
               (user_id, venue_id, ciphertext, nonce, wrapped_dek, envelope_key_version, \
                field_names, key_last4) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (user_id, venue_id) DO UPDATE SET \
               ciphertext = EXCLUDED.ciphertext, nonce = EXCLUDED.nonce, \
               wrapped_dek = EXCLUDED.wrapped_dek, \
               envelope_key_version = EXCLUDED.envelope_key_version, \
               field_names = EXCLUDED.field_names, key_last4 = EXCLUDED.key_last4, \
               updated_at = now()",
        )
        .bind(user_id)
        .bind(venue)
        .bind(&enc.ciphertext)
        .bind(&enc.nonce)
        .bind(&enc.wrapped_dek)
        .bind(enc.key_version)
        .bind(&names)
        .bind(key_last4)
        .execute(&self.pg)
        .await?;
        Ok(())
    }

    /// Decrypt one venue's fields. The only decrypting path; for adapters.
    ///
    /// # Errors
    /// No KEK, a crypto failure or a database error.
    pub async fn load(
        &self,
        user_id: Uuid,
        venue: &str,
    ) -> Result<Option<BTreeMap<String, String>>, VenueCredentialError> {
        let row: Option<EncryptedRow> = sqlx::query_as(
            "SELECT ciphertext, nonce, wrapped_dek, envelope_key_version \
               FROM venue_credentials WHERE user_id = $1 AND venue_id = $2",
        )
        .bind(user_id)
        .bind(venue)
        .fetch_optional(&self.pg)
        .await?;
        let Some((ciphertext, nonce, wrapped_dek, key_version)) = row else {
            return Ok(None);
        };
        let plain = self
            .crypto()?
            .decrypt(&EncryptedCredential {
                ciphertext,
                nonce,
                wrapped_dek,
                key_version,
            })
            .map_err(|_| VenueCredentialError::Crypto)?;
        serde_json::from_slice(&plain.bytes)
            .map(Some)
            .map_err(|_| VenueCredentialError::Crypto)
    }

    /// Display-safe status of everything `user_id` has stored.
    ///
    /// # Errors
    /// A database error.
    pub async fn status(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<VenueCredentialStatus>, VenueCredentialError> {
        let rows: Vec<StatusRow> = sqlx::query_as(
            "SELECT venue_id, field_names, key_last4, updated_at \
               FROM venue_credentials WHERE user_id = $1 ORDER BY venue_id",
        )
        .bind(user_id)
        .fetch_all(&self.pg)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(venue, field_names, key_last4, updated_at)| VenueCredentialStatus {
                venue,
                field_names,
                key_last4,
                updated_at: updated_at.to_rfc3339(),
            })
            .collect())
    }

    /// Remove one venue's credential for `user_id`. `true` when a row went.
    ///
    /// Not named `delete`: the route handler below is, and the source scans that
    /// resolve a handler by name (`every_mounted_route_refuses_an_anonymous_caller`)
    /// would find this method first and read its signature, which takes no token.
    ///
    /// # Errors
    /// A database error.
    pub async fn remove(&self, user_id: Uuid, venue: &str) -> Result<bool, VenueCredentialError> {
        let r = sqlx::query("DELETE FROM venue_credentials WHERE user_id = $1 AND venue_id = $2")
            .bind(user_id)
            .bind(venue)
            .execute(&self.pg)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}

fn error_response(e: &VenueCredentialError) -> Response {
    match e {
        VenueCredentialError::CryptoUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "credential_crypto_unavailable",
                "detail": "CRED_KEK is not configured, so a credential cannot be encrypted. \
                           Nothing was stored.",
            })),
        )
            .into_response(),
        VenueCredentialError::Invalid(m) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid", "detail": m })),
        )
            .into_response(),
        VenueCredentialError::Crypto => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "credential_crypto_failed" })),
        )
            .into_response(),
        VenueCredentialError::Db(err) => {
            tracing::error!(error = %err, "venue credentials: db error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "database error" })),
            )
                .into_response()
        }
    }
}

fn store(state: &AppState) -> VenueCredentialStore {
    VenueCredentialStore::new(state.pg.clone(), state.cred_crypto.clone())
}

/// `GET /auth/venue-credentials` — what the caller has stored, never the values.
pub async fn list(State(state): State<AppState>, token: BearerToken) -> Response {
    match store(&state).status(token.user_id()).await {
        Ok(rows) => Json(json!({ "credentials": rows })).into_response(),
        Err(e) => error_response(&e),
    }
}

/// `PUT /auth/venue-credentials` — encrypt and store one or more venues' fields.
pub async fn put(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<Value>,
) -> Response {
    let grouped = match parse_body(&body) {
        Ok(g) => g,
        Err(e) => return error_response(&e),
    };
    let store = store(&state);
    let mut saved = Vec::new();
    for (venue, fields) in &grouped {
        if let Err(e) = store.save(token.user_id(), venue, fields).await {
            return error_response(&e);
        }
        saved.push(venue.clone());
    }
    Json(json!({ "saved": saved })).into_response()
}

/// `DELETE /auth/venue-credentials/{venue}`.
pub async fn delete(
    State(state): State<AppState>,
    token: BearerToken,
    Path(venue): Path<String>,
) -> Response {
    match store(&state).remove(token.user_id(), &venue).await {
        Ok(true) => Json(json!({ "venue": venue, "deleted": true })).into_response(),
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_found", "venue": venue })),
        )
            .into_response(),
        Err(e) => error_response(&e),
    }
}

/// `POST /auth/venue-credentials/{venue}/verify` — refused honestly (501).
///
/// Mounted under the venue, not at `/auth/venue-credentials/verify`: `{venue}` is
/// a closed vocabulary (`KNOWN_VENUES`), and a literal sibling of it would take
/// every method of its own path from the parameter
/// (`a_static_route_does_not_shadow_its_parameterised_sibling`).
pub async fn verify(_token: BearerToken, Path(_venue): Path<String>) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        Json(json!({
            "error": "venue_verification_not_available",
            "detail": "verifying a credential means calling the venue, and the live venue \
                       adapters are Set E 3.x. Nothing was checked; nothing is reported as \
                       verified.",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_shape_is_one_venue() {
        let g = parse_body(&json!({"venue": "tradier", "api_key": "abcd1234efgh"})).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g["tradier"]["api_key"], "abcd1234efgh");
    }

    #[test]
    fn the_signup_shape_groups_by_prefix() {
        let g = parse_body(&json!({
            "alpaca_api_key": "k1", "alpaca_api_secret": "s1", "coinbase_api_key": "k2"
        }))
        .unwrap();
        assert_eq!(g["alpaca"].len(), 2);
        assert_eq!(g["coinbase"]["api_key"], "k2");
    }

    #[test]
    fn an_unknown_venue_or_an_empty_body_is_refused() {
        assert!(parse_body(&json!({"venue": "mtgox", "api_key": "x"})).is_err());
        assert!(parse_body(&json!({"nothing": "x"})).is_err());
        assert!(parse_body(&json!({})).is_err());
        assert!(parse_body(&json!({"venue": "alpaca", "api_key": 5})).is_err());
    }

    #[test]
    fn last4_never_reveals_a_short_secret() {
        assert_eq!(last4("abcdefgh1234"), Some("1234".into()));
        assert_eq!(last4("short"), None);
    }
}
