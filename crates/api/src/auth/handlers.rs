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

fn new_session_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
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

    let token = new_session_token();
    sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(user_id)
        .execute(&state.pg)
        .await
        .map_err(|e| {
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

    let (user_id, hash, created_at) = row
        .filter(|(_, hash, _)| !hash.is_empty() && verify_password(&body.password, hash))
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "invalid email or password").into_response())?;

    let token = new_session_token();
    sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(user_id)
        .execute(&state.pg)
        .await
        .map_err(|e| {
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
    .bind(&bearer.0)
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
        .bind(&bearer.0)
        .execute(&state.pg)
        .await;
    StatusCode::OK
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
