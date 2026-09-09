//! LLM provider credential + model-listing routes for the internal agent.
//!
//! Keys are verified against the live provider before being stored, are
//! AES-256-GCM envelope-encrypted at rest, and are never echoed by any
//! read path. Model listing is a POST so keys never appear in URLs.

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

use crate::credentials::store::LlmCredentialStore;
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

fn kek_missing() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": "cred_kek_missing",
            "message": "CRED_KEK is not configured on the platform — set a 32-byte hex key \
                        (openssl rand -hex 32) in the environment and restart",
        })),
    )
        .into_response()
}

fn llm_error_response(e: &LlmError) -> Response {
    let (status, code) = match e {
        LlmError::Auth(_) => (StatusCode::UNPROCESSABLE_ENTITY, "verification_failed"),
        LlmError::Network(_) => (StatusCode::UNPROCESSABLE_ENTITY, "provider_unreachable"),
        LlmError::RateLimited { .. } => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
        _ => (StatusCode::BAD_GATEWAY, "provider_error"),
    };
    (
        status,
        Json(json!({ "error": code, "message": e.to_string() })),
    )
        .into_response()
}

// ── PUT /api/llm/credentials/{provider} ──────────────────────────────────────

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

    // Verify against the live provider before persisting anything.
    let client = LlmClient::new(
        provider,
        (!api_key.is_empty()).then(|| api_key.clone()),
        base_url.clone(),
    );
    let model_count = match client.verify().await {
        Ok(n) => n,
        Err(e) => return llm_error_response(&e),
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
        tracing::error!(error = %e, provider = provider.as_str(), "save_credential failed");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "store_failed" })),
        )
            .into_response();
    }

    Json(json!({ "ok": true, "verified": true, "model_count": model_count })).into_response()
}

// ── GET /api/llm/credentials ─────────────────────────────────────────────────

pub async fn credential_status(State(state): State<AppState>, token: BearerToken) -> Response {
    let creds = store(&state);
    match creds.status(token.user_id()).await {
        Ok(providers) => Json(json!({
            "providers": providers,
            "encryption_available": creds.crypto_available(),
        }))
        .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "credential_status failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "status_failed" })),
            )
                .into_response()
        }
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
        Ok(false) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "not_configured" })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!(error = %e, "delete_credential failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "delete_failed" })),
            )
                .into_response()
        }
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

    // Supplied credentials win; otherwise use the stored ones.
    let (api_key, base_url) = if supplied_key.is_some() || !provider.requires_api_key() {
        (supplied_key, supplied_url)
    } else {
        match store(&state).load(token.user_id(), provider.as_str()).await {
            Ok(Some(cred)) => (
                (!cred.api_key.is_empty()).then_some(cred.api_key),
                supplied_url.or(cred.base_url),
            ),
            Ok(None) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(json!({ "error": "not_configured",
                        "message": format!("no stored {} credential — save one first or pass api_key",
                                           provider.as_str()) })),
                )
                    .into_response()
            }
            Err(e) => {
                tracing::error!(error = %e, "list_models: credential load failed");
                return kek_missing();
            }
        }
    };

    let client = LlmClient::new(provider, api_key, base_url);
    match client.list_models().await {
        Ok(models) => Json(json!({ "models": models })).into_response(),
        Err(e) => llm_error_response(&e),
    }
}
