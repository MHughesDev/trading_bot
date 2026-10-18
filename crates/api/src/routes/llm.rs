//! LLM provider credential + model-listing routes for the internal agent.
//!
//! Every user brings their own key (PROVIDERS track, 2026-10-05;
//! docs/operations/connect-ai-provider.md). Keys are verified against the live
//! provider before being stored, are AES-256-GCM envelope-encrypted at rest with
//! the operator's `CRED_KEK`, and are never echoed by any path: not in a
//! response, not in a stored error, not in a log line. Model listing is a POST so
//! keys never appear in URLs.
//!
//! Only API keys. Anthropic and OpenAI allow API keys in third-party products;
//! consumer subscriptions (Claude Free/Pro/Max, ChatGPT Plus/Pro) are not a
//! credential this platform accepts, and there is deliberately no route that
//! takes a subscription token or proxies one.
//!
//! | Route | What it does |
//! |---|---|
//! | `GET /api/llm/credentials` | the caller's providers: connected, masked suffix, verified time, last error, default |
//! | `PUT /api/llm/credentials/{provider}` | verify a pasted key with a real call, then store it |
//! | `POST /api/llm/credentials/{provider}/verify` | re-check the stored key; records the outcome |
//! | `DELETE /api/llm/credentials/{provider}` | remove it |
//! | `PUT /api/llm/default` | the provider and model the desk starts on |
//! | `POST /api/llm/{provider}/models` | the models a key can reach |

use std::str::FromStr;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::json;

use llm::{LlmClient, LlmError, Provider};

use crate::credentials::store::{LlmCredentialStore, LlmStoreError};
use crate::{auth::BearerToken, state::AppState};

fn store(state: &AppState) -> LlmCredentialStore {
    LlmCredentialStore::new(state.pg.clone(), state.cred_crypto.clone())
}

fn parse_provider(raw: &str) -> Result<Provider, Box<Response>> {
    Provider::from_str(raw).map_err(|e| {
        Box::new(
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "unknown_provider", "message": e })),
            )
                .into_response(),
        )
    })
}

/// What the operator has to do for keys to be storable, in words the settings
/// page prints as they are.
///
/// Read from the environment at the time of the request, so it can tell "not
/// set" from "set but malformed". `AppState` reads `CRED_KEK` once at startup, so
/// both cases end with "restart".
#[must_use]
pub fn encryption_problem() -> String {
    match std::env::var("CRED_KEK") {
        Ok(v) if !v.trim().is_empty() => format!(
            "CRED_KEK is set but is not valid: it must be exactly 64 hex characters (32 bytes); \
             this one is {} characters. Generate one with `openssl rand -hex 32`, set it in the \
             platform's environment, and restart the platform.",
            v.trim().len()
        ),
        _ => "Keys can't be saved: credential encryption isn't configured on this platform. The \
              operator must set CRED_KEK to 64 hex characters (32 random bytes, e.g. `openssl \
              rand -hex 32`) in the platform's environment and restart it."
            .to_string(),
    }
}

fn kek_missing() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "cred_kek_missing",
            "operator_action": "set CRED_KEK",
            "message": encryption_problem(),
        })),
    )
        .into_response()
}

/// The provider's own sentence out of an error body, when it sent JSON.
///
/// Both providers answer `{"error": {"message": …}}`. A raw body is a wall of
/// JSON on a settings card; the sentence is what the person needs.
fn provider_sentence(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect())
}

/// Remove every occurrence of `secret` from `text`.
///
/// Providers echo keys back in error bodies — OpenAI's 401 names the key it was
/// given, masked or not depending on the endpoint — and a scrubbed reason is the
/// only kind this module returns or stores. Keys shorter than eight characters
/// are not keys any hosted provider issues, and scrubbing a short string would
/// mangle ordinary words.
#[must_use]
pub fn scrub(text: &str, secret: Option<&str>) -> String {
    match secret.map(str::trim) {
        Some(s) if s.len() >= 8 => text.replace(s, "[redacted]"),
        _ => text.to_string(),
    }
}

/// A short, key-free sentence for why a provider call failed.
#[must_use]
pub fn failure_reason(e: &LlmError, secret: Option<&str>) -> String {
    let raw = match e {
        LlmError::Auth(body) => format!(
            "the provider rejected this key ({})",
            provider_sentence(body)
        ),
        LlmError::Network(m) => format!("could not reach the provider: {m}"),
        LlmError::RateLimited { .. } => {
            "the provider is rate-limiting this key; wait a minute and test again".to_string()
        }
        LlmError::Overloaded => "the provider is overloaded; test again shortly".to_string(),
        LlmError::InvalidRequest(body) => provider_sentence(body),
        LlmError::Provider { status, body } => {
            format!(
                "the provider answered {status}: {}",
                provider_sentence(body)
            )
        }
        other => other.to_string(),
    };
    let scrubbed = scrub(&raw, secret);
    scrubbed.chars().take(300).collect()
}

fn llm_error_response(e: &LlmError, secret: Option<&str>) -> Response {
    let (status, code) = match e {
        LlmError::Auth(_) => (StatusCode::UNPROCESSABLE_ENTITY, "key_rejected"),
        LlmError::Network(_) => (StatusCode::UNPROCESSABLE_ENTITY, "provider_unreachable"),
        LlmError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        _ => (StatusCode::BAD_GATEWAY, "provider_error"),
    };
    (
        status,
        Json(json!({ "error": code, "message": failure_reason(e, secret) })),
    )
        .into_response()
}

fn store_failed(e: &LlmStoreError, what: &'static str) -> Response {
    // `LlmStoreError` carries no secret material (crypto and sqlx errors only),
    // so it may be logged.
    tracing::error!(error = %e, what, "llm credential store failed");
    match e {
        LlmStoreError::CryptoUnavailable => kek_missing(),
        LlmStoreError::Crypto(_) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": "undecryptable",
                "message": "The stored key can't be decrypted with this platform's CRED_KEK \
                            (it was saved under a different one). Replace the key.",
            })),
        )
            .into_response(),
        LlmStoreError::Db(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": what })),
        )
            .into_response(),
    }
}

fn not_configured(provider: Provider) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "error": "not_configured",
            "message": format!("no {} key is connected for this account", provider.as_str()),
        })),
    )
        .into_response()
}

// ── PUT /api/llm/credentials/{provider} ──────────────────────────────────────

/// The pasted key. Deliberately not `Debug`: nothing may format it.
#[derive(Deserialize)]
pub struct SaveCredentialBody {
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

pub async fn save_credential(
    State(state): State<AppState>,
    token: BearerToken,
    Path(provider_raw): Path<String>,
    Json(body): Json<SaveCredentialBody>,
) -> Response {
    let provider = match parse_provider(&provider_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let creds = store(&state);
    if !creds.crypto_available() {
        return kek_missing();
    }

    let api_key = body.api_key.as_deref().unwrap_or("").trim().to_string();
    if api_key.is_empty() && provider.requires_api_key() {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": "missing_api_key",
                "message": format!("{} requires an API key", provider.as_str()) })),
        )
            .into_response();
    }
    let base_url = body
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Verify against the live provider before persisting anything. Listing
    // models is a real authenticated call that costs nothing.
    let client = LlmClient::new(
        provider,
        (!api_key.is_empty()).then(|| api_key.clone()),
        base_url.clone(),
    );
    let model_count = match client.verify().await {
        Ok(n) => n,
        Err(e) => return llm_error_response(&e, Some(&api_key)),
    };

    if let Err(e) = creds
        .save(
            token.user_id(),
            provider.as_str(),
            &api_key,
            base_url.as_deref(),
        )
        .await
    {
        return store_failed(&e, "store_failed");
    }

    Json(json!({
        "ok": true,
        "verified": true,
        "model_count": model_count,
        "key_last4": crate::credentials::store::last4(&api_key),
    }))
    .into_response()
}

// ── POST /api/llm/credentials/{provider}/verify ──────────────────────────────

/// Re-check the stored key with a real call and record the answer, so the card
/// can say "verified <time>" or "Key rejected: <reason>" about a key that was
/// fine when it was saved and has since been revoked.
pub async fn verify_credential(
    State(state): State<AppState>,
    token: BearerToken,
    Path(provider_raw): Path<String>,
) -> Response {
    let provider = match parse_provider(&provider_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let creds = store(&state);
    let cred = match creds.load(token.user_id(), provider.as_str()).await {
        Ok(Some(c)) => c,
        Ok(None) => return not_configured(provider),
        Err(e) => return store_failed(&e, "verify_failed"),
    };
    let secret = (!cred.api_key.is_empty()).then_some(cred.api_key);
    let client = LlmClient::new(provider, secret.clone(), cred.base_url);
    let outcome = client.verify().await;
    let reason = outcome
        .as_ref()
        .err()
        .map(|e| failure_reason(e, secret.as_deref()));
    if let Err(e) = creds
        .record_check(
            token.user_id(),
            provider.as_str(),
            reason.as_deref().map_or(Ok(()), Err),
        )
        .await
    {
        return store_failed(&e, "verify_failed");
    }
    match outcome {
        Ok(n) => Json(json!({ "ok": true, "verified": true, "model_count": n })).into_response(),
        Err(e) => llm_error_response(&e, secret.as_deref()),
    }
}

// ── GET /api/llm/credentials ─────────────────────────────────────────────────

pub async fn credential_status(State(state): State<AppState>, token: BearerToken) -> Response {
    let creds = store(&state);
    match creds.status(token.user_id()).await {
        Ok(providers) => {
            let available = creds.crypto_available();
            Json(json!({
                "providers": providers,
                "encryption_available": available,
                // The operator's instruction, verbatim, when keys cannot be saved.
                "encryption_problem": (!available).then(encryption_problem),
            }))
            .into_response()
        }
        Err(e) => store_failed(&e, "status_failed"),
    }
}

// ── DELETE /api/llm/credentials/{provider} ───────────────────────────────────

pub async fn delete_credential(
    State(state): State<AppState>,
    token: BearerToken,
    Path(provider_raw): Path<String>,
) -> Response {
    let provider = match parse_provider(&provider_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    match store(&state)
        .delete(token.user_id(), provider.as_str())
        .await
    {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => not_configured(provider),
        Err(e) => store_failed(&e, "delete_failed"),
    }
}

// ── PUT /api/llm/default ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct DefaultBody {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
}

/// Whether `model` is shaped like a model id every provider uses: no spaces, no
/// control characters, nothing a picker would have to escape.
fn plausible_model_id(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '/' | '@'))
}

pub async fn set_default(
    State(state): State<AppState>,
    token: BearerToken,
    Json(body): Json<DefaultBody>,
) -> Response {
    let provider = match parse_provider(&body.provider) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let model = body
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    if let Some(m) = model {
        if !plausible_model_id(m) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "invalid_model",
                    "message": "a model id is letters, digits and . - _ : / @ only" })),
            )
                .into_response();
        }
        if !crate::model_policy::is_allowed(provider.as_str(), m) {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": "model_not_enabled",
                    "message": crate::model_policy::not_enabled_message(provider.as_str(), m) })),
            )
                .into_response();
        }
    }
    match store(&state)
        .set_default(token.user_id(), provider.as_str(), model)
        .await
    {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => not_configured(provider),
        Err(e) => store_failed(&e, "default_failed"),
    }
}

// ── POST /api/llm/{provider}/models ──────────────────────────────────────────

#[derive(Deserialize, Default)]
pub struct ListModelsBody {
    /// Preview with a key not yet saved; falls back to the stored credential.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

pub async fn list_models(
    State(state): State<AppState>,
    token: BearerToken,
    Path(provider_raw): Path<String>,
    body: Option<Json<ListModelsBody>>,
) -> Response {
    let provider = match parse_provider(&provider_raw) {
        Ok(p) => p,
        Err(resp) => return *resp,
    };
    let Json(body) = body.unwrap_or_default();

    let supplied_key = body
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let supplied_url = body
        .base_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Supplied credentials win; otherwise the caller's own stored ones. Never
    // another user's: `load` is keyed by the bearer's user id.
    let (api_key, base_url) = if supplied_key.is_some() {
        (supplied_key, supplied_url)
    } else {
        match store(&state).load(token.user_id(), provider.as_str()).await {
            Ok(Some(cred)) => (
                (!cred.api_key.is_empty()).then_some(cred.api_key),
                supplied_url.or(cred.base_url),
            ),
            Ok(None) if !provider.requires_api_key() => (None, supplied_url),
            Ok(None) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": "not_configured",
                        "message": format!("no stored {} key — connect one first",
                                           provider.as_str()) })),
                )
                    .into_response()
            }
            Err(e) => return store_failed(&e, "models_failed"),
        }
    };

    let client = crate::model_policy::client(provider, api_key.clone(), base_url);
    match client.list_models().await {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(e) => llm_error_response(&e, api_key.as_deref()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "sk-proj-abcdefghijklmnop1234";

    /// OpenAI's 401 names the key it was given. The reason this module returns
    /// and stores is the provider's sentence with the key cut out of it.
    #[test]
    fn a_rejected_key_is_not_echoed_in_the_reason() {
        let body = format!(
            r#"{{"error":{{"message":"Incorrect API key provided: {KEY}. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","code":"invalid_api_key"}}}}"#
        );
        let reason = failure_reason(&LlmError::Auth(body), Some(KEY));
        assert!(!reason.contains(KEY), "{reason}");
        assert!(reason.contains("[redacted]"), "{reason}");
        assert!(
            reason.starts_with("the provider rejected this key (Incorrect API key"),
            "{reason}"
        );
    }

    #[test]
    fn anthropics_error_shape_reads_as_its_sentence() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(
            failure_reason(&LlmError::Auth(body.into()), Some(KEY)),
            "the provider rejected this key (invalid x-api-key)"
        );
    }

    #[test]
    fn a_short_secret_does_not_mangle_ordinary_words() {
        assert_eq!(scrub("the key was bad", Some("key")), "the key was bad");
        assert_eq!(
            scrub("value sk-1234567890", Some("sk-1234567890")),
            "value [redacted]"
        );
    }

    /// The operator's instruction names the variable and its shape.
    #[test]
    fn the_encryption_problem_names_what_to_set() {
        let msg = encryption_problem();
        assert!(msg.contains("CRED_KEK"), "{msg}");
        assert!(msg.contains("64 hex characters"), "{msg}");
        assert!(msg.contains("restart"), "{msg}");
    }

    #[test]
    fn model_ids_are_checked_for_shape_only() {
        assert!(plausible_model_id("gpt-5"));
        assert!(plausible_model_id("claude-opus-5"));
        assert!(plausible_model_id("qwen2.5:7b-instruct"));
        assert!(plausible_model_id("accounts/fireworks/models/x@v1"));
        assert!(!plausible_model_id("gpt 5"));
        assert!(!plausible_model_id(""));
        assert!(!plausible_model_id("<script>"));
    }
}
