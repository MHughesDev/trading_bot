use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::email::send_reset_code;
use crate::state::AppState;

// ── helpers ──────────────────────────────────────────────────────────────────

pub(crate) fn new_session_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Insert a web session for `user_id` and return the credential to hand back.
///
/// The row stores `session_token_hash(token)` — the migration-0150 trigger
/// hashes on insert — so the returned string exists only in this response.
/// `web:full` is set explicitly: the column default is `{}`, and a web session
/// minted with it (as every login was between 0038 and 0150) held no scope at
/// all. The expiry starts at the idle window and slides on use
/// ([`super::session::touch_key`]).
pub(crate) async fn mint_web_session(pg: &sqlx::PgPool, user_id: Uuid) -> Result<String, sqlx::Error> {
    let token = new_session_token();
    sqlx::query(
        "INSERT INTO sessions (token, user_id, kind, scopes, expires_at, last_seen_at) \
         VALUES ($1, $2, 'web', ARRAY['web:full'], now() + make_interval(days => $3), now())",
    )
    .bind(&token)
    .bind(user_id)
    .bind(i32::try_from(super::session::WEB_IDLE_TTL_DAYS).unwrap_or(7))
    .execute(pg)
    .await?;
    Ok(token)
}

fn six_digit_code() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    let n = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{n:06}")
}

fn hash_password(password: &str) -> Result<String, StatusCode> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .and_then(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .ok()
        })
        .is_some()
}

// ── shared response types ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user: UserResponse,
}

// ── POST /auth/register ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub password: String,
}

pub async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterBody>,
) -> Result<Json<LoginResponse>, Response> {
    if body.password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "password must be at least 8 characters",
        )
            .into_response());
    }

    let hash = hash_password(&body.password).map_err(|s| s.into_response())?;

    let user_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (email, password_hash) VALUES ($1, $2)
         ON CONFLICT (email) DO NOTHING
         RETURNING user_id",
    )
    .bind(&body.email)
    .bind(&hash)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "register: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?
    .ok_or_else(|| (StatusCode::CONFLICT, "email already registered").into_response())?;

    let token = mint_web_session(&state.pg, user_id).await.map_err(|e| {
        tracing::error!(error = %e, "register: session insert error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let created_at: chrono::DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&state.pg)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    Ok(Json(LoginResponse {
        token,
        user: UserResponse {
            id: user_id.to_string(),
            email: body.email,
            created_at: created_at.to_rfc3339(),
        },
    }))
}

// ── POST /auth/login ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LoginBody {
    pub email: String,
    pub password: String,
}

pub async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<LoginResponse>, Response> {
    let row: Option<(Uuid, String, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT user_id, password_hash, created_at FROM users WHERE email = $1 AND active = true",
    )
    .bind(&body.email)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "login: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let (user_id, _hash, created_at) = row
        .filter(|(_, hash, _)| !hash.is_empty() && verify_password(&body.password, hash))
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "invalid email or password").into_response())?;

    let token = mint_web_session(&state.pg, user_id).await.map_err(|e| {
        tracing::error!(error = %e, "login: session insert error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    Ok(Json(LoginResponse {
        token,
        user: UserResponse {
            id: user_id.to_string(),
            email: body.email,
            created_at: created_at.to_rfc3339(),
        },
    }))
}

// ── GET /auth/me ──────────────────────────────────────────────────────────────

pub async fn me(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
) -> Result<Json<UserResponse>, Response> {
    let row: Option<(Uuid, String, chrono::DateTime<Utc>)> = sqlx::query_as(
        "SELECT u.user_id, u.email, u.created_at
         FROM sessions s
         JOIN users u ON u.user_id = s.user_id
         WHERE s.token = $1 AND s.expires_at > now() AND u.active = true",
    )
    .bind(&bearer.token)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "me: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let (user_id, email, created_at) = row
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid").into_response())?;

    Ok(Json(UserResponse {
        id: user_id.to_string(),
        email,
        created_at: created_at.to_rfc3339(),
    }))
}

// ── POST /auth/logout ─────────────────────────────────────────────────────────

pub async fn logout(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
) -> StatusCode {
    let _ = sqlx::query("DELETE FROM sessions WHERE token = $1")
        .bind(&bearer.token)
        .execute(&state.pg)
        .await;
    StatusCode::OK
}

// ── POST /auth/touch ──────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct TouchResponse {
    /// When this session now expires.
    pub expires_at: String,
    /// `true` for a web session, whose expiry was recomputed (it may already
    /// sit at its absolute cap and so not move). `false` for a service session,
    /// which never slides.
    pub extended: bool,
}

/// `POST /auth/touch` — slide this web session's expiry now (Set E 1.2).
///
/// The extractor already slides on use (throttled to once a minute); this is
/// the explicit form the SPA calls on activity so an open tab nobody clicks in
/// does not lose its session at the idle limit. A service session is answered
/// with its unchanged expiry and `extended: false` — it was minted with the
/// lifetime it is meant to have.
pub async fn touch(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
) -> Result<Json<TouchResponse>, Response> {
    let moved = super::session::touch_key(&state.pg, &bearer.token)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "touch: db error");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        })?;
    if let Some(expires_at) = moved {
        return Ok(Json(TouchResponse {
            expires_at: expires_at.to_rfc3339(),
            extended: true,
        }));
    }
    let expires_at: Option<chrono::DateTime<Utc>> =
        sqlx::query_scalar("SELECT expires_at FROM sessions WHERE token = $1 AND expires_at > now()")
            .bind(&bearer.token)
            .fetch_optional(&state.pg)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let expires_at = expires_at
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "session expired or invalid").into_response())?;
    Ok(Json(TouchResponse {
        expires_at: expires_at.to_rfc3339(),
        extended: false,
    }))
}

// ── Service tokens ────────────────────────────────────────────────────────────
//
// Long-lived bearer sessions (kind = 'service') for headless clients: the MCP
// server process and internal agent runs. The full token is echoed exactly
// once, at mint time; listings expose only an 8-char prefix.

#[derive(Deserialize)]
pub struct CreateServiceTokenBody {
    pub label: String,
}

#[derive(Serialize)]
pub struct ServiceTokenCreated {
    pub token: String,
    pub token_prefix: String,
    pub label: String,
    pub expires_at: String,
}

#[derive(Serialize)]
pub struct ServiceTokenSummary {
    pub token_prefix: String,
    pub label: Option<String>,
    pub created_at: String,
    pub expires_at: String,
}

pub async fn create_service_token(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
    Json(body): Json<CreateServiceTokenBody>,
) -> Result<(StatusCode, Json<ServiceTokenCreated>), Response> {
    let label = body.label.trim();
    if label.is_empty() || label.len() > 64 {
        return Err((StatusCode::BAD_REQUEST, "label must be 1-64 characters").into_response());
    }

    let token = new_session_token();
    // `RETURNING token` is the stored key (the digest, hashed by the 0150
    // trigger). Its prefix is what listings show and what delete matches, so the
    // prefix echoed now is the same one the list will show later — and it says
    // nothing about the credential.
    let (key, expires_at): (String, chrono::DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO sessions (token, user_id, label, kind, expires_at)
         VALUES ($1, $2, $3, 'service', now() + INTERVAL '365 days')
         RETURNING token, expires_at",
    )
    .bind(&token)
    .bind(bearer.user_id)
    .bind(label)
    .fetch_one(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "create_service_token: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    Ok((
        StatusCode::CREATED,
        Json(ServiceTokenCreated {
            token_prefix: key[..8].to_string(),
            token,
            label: label.to_string(),
            expires_at: expires_at.to_rfc3339(),
        }),
    ))
}

pub async fn list_service_tokens(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
) -> Result<Json<serde_json::Value>, Response> {
    // Row shape: (token, label, created_at, expires_at).
    type TokenRow = (
        String,
        Option<String>,
        chrono::DateTime<Utc>,
        chrono::DateTime<Utc>,
    );
    let rows: Vec<TokenRow> = sqlx::query_as(
        "SELECT token, label, created_at, expires_at FROM sessions
             WHERE user_id = $1 AND kind = 'service'
             ORDER BY created_at DESC",
    )
    .bind(bearer.user_id)
    .fetch_all(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "list_service_tokens: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let tokens: Vec<ServiceTokenSummary> = rows
        .into_iter()
        .map(
            |(token, label, created_at, expires_at)| ServiceTokenSummary {
                token_prefix: token.chars().take(8).collect(),
                label,
                created_at: created_at.to_rfc3339(),
                expires_at: expires_at.to_rfc3339(),
            },
        )
        .collect();

    Ok(Json(serde_json::json!({ "tokens": tokens })))
}

pub async fn delete_service_token(
    State(state): State<AppState>,
    bearer: super::session::BearerToken,
    axum::extract::Path(token_prefix): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, Response> {
    if token_prefix.len() < 8 {
        return Err((StatusCode::BAD_REQUEST, "token prefix too short").into_response());
    }

    let result = sqlx::query(
        "DELETE FROM sessions
         WHERE user_id = $1 AND kind = 'service' AND token LIKE $2 || '%'",
    )
    .bind(bearer.user_id)
    .bind(&token_prefix)
    .execute(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "delete_service_token: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    if result.rows_affected() == 0 {
        return Err((StatusCode::NOT_FOUND, "no matching service token").into_response());
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ── POST /auth/forgot-password ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ForgotPasswordBody {
    pub email: String,
}

pub async fn forgot_password(
    State(state): State<AppState>,
    Json(body): Json<ForgotPasswordBody>,
) -> Result<StatusCode, Response> {
    let user: Option<(Uuid,)> =
        sqlx::query_as("SELECT user_id FROM users WHERE email = $1 AND active = true")
            .bind(&body.email)
            .fetch_optional(&state.pg)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    let Some((user_id,)) = user else {
        return Err((StatusCode::NOT_FOUND, "no account found with that email").into_response());
    };

    // Invalidate any existing unused codes for this user.
    let _ =
        sqlx::query("UPDATE password_resets SET used = true WHERE user_id = $1 AND used = false")
            .bind(user_id)
            .execute(&state.pg)
            .await;

    let code = six_digit_code();

    let _ = sqlx::query("INSERT INTO password_resets (user_id, code) VALUES ($1, $2)")
        .bind(user_id)
        .bind(&code)
        .execute(&state.pg)
        .await;

    send_reset_code(&state.email, &body.email, &code);

    Ok(StatusCode::OK)
}

// ── POST /auth/verify-reset-code ──────────────────────────────────────────────

#[derive(Deserialize)]
pub struct VerifyCodeBody {
    pub email: String,
    pub code: String,
}

pub async fn verify_reset_code(
    State(state): State<AppState>,
    Json(body): Json<VerifyCodeBody>,
) -> Result<StatusCode, Response> {
    let valid: Option<(Uuid,)> = sqlx::query_as(
        "SELECT pr.id FROM password_resets pr
         JOIN users u ON u.user_id = pr.user_id
         WHERE u.email = $1 AND pr.code = $2 AND pr.used = false AND pr.expires_at > now()
         LIMIT 1",
    )
    .bind(&body.email)
    .bind(&body.code)
    .fetch_optional(&state.pg)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    if valid.is_some() {
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::BAD_REQUEST, "invalid or expired code").into_response())
    }
}

// ── POST /auth/reset-password ─────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ResetPasswordBody {
    pub email: String,
    pub code: String,
    pub new_password: String,
}

pub async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordBody>,
) -> Result<StatusCode, Response> {
    if body.new_password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "password must be at least 8 characters",
        )
            .into_response());
    }

    let row: Option<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT pr.id, pr.user_id
         FROM password_resets pr
         JOIN users u ON u.user_id = pr.user_id
         WHERE u.email = $1
           AND pr.code = $2
           AND pr.used = false
           AND pr.expires_at > now()
         LIMIT 1",
    )
    .bind(&body.email)
    .bind(&body.code)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "reset_password: db error");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    })?;

    let (reset_id, user_id) =
        row.ok_or_else(|| (StatusCode::BAD_REQUEST, "invalid or expired code").into_response())?;

    let new_hash = hash_password(&body.new_password).map_err(|s| s.into_response())?;

    // Mark code as used.
    sqlx::query("UPDATE password_resets SET used = true WHERE id = $1")
        .bind(reset_id)
        .execute(&state.pg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    // Update password.
    sqlx::query("UPDATE users SET password_hash = $1 WHERE user_id = $2")
        .bind(&new_hash)
        .bind(user_id)
        .execute(&state.pg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    // Kill all existing sessions for this user so they must re-login.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.pg)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;

    Ok(StatusCode::OK)
}
