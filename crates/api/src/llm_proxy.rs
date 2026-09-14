//! Platform LLM proxy (AGENT-001 §9, ADR-0024 §3, RT-22, RT-23).
//!
//! The agent's container holds no long-lived secret. Its `ANTHROPIC_BASE_URL` points
//! here and its `ANTHROPIC_AUTH_TOKEN` is the session token, which is useless
//! anywhere else. This process authenticates that token, swaps in the real provider
//! credential, enforces the project's dollar budget, and records usage.
//!
//! That arrangement is what makes three separate guarantees hold at once:
//!
//! - **No secrets in the sandbox.** An agent that reads its own environment finds a
//!   token scoped to one project and revocable in one row.
//! - **Budgets the agent cannot bypass.** Spend is checked here, before the upstream
//!   call, not reported by the thing doing the spending.
//! - **Cache TTL is settable at all.** The Agent SDK exposes no cache-TTL option
//!   (AGENT-001 §23), so rewriting `cache_control` on the way out is the only place
//!   a 1-hour TTL can be chosen (RT-22).

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::state::AppState;

/// Cache TTLs the Anthropic API accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheTtl {
    FiveMinutes,
    OneHour,
}

impl CacheTtl {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheTtl::FiveMinutes => "5m",
            CacheTtl::OneHour => "1h",
        }
    }
}

/// Which credential a project's calls are billed to (D-16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    /// Anthropic API key — exact per-token billing.
    ApiKey,
    /// Claude subscription (OAuth) — flat rate, so spend is estimated.
    Subscription,
}

/// Rewrites every `cache_control` block in a Messages request to `ttl`.
///
/// The CLI inside the container decides where its cache breakpoints go and picks a
/// TTL itself; we do not want to move the breakpoints, only to lengthen their life.
/// A research session runs for hours with a stable system prompt and workspace map,
/// so a 5-minute TTL re-pays for the same prefix over and over.
///
/// Requires the `extended-cache-ttl-2025-04-11` beta header, which is added alongside
/// whatever betas the client already asked for.
pub fn rewrite_cache_ttl(body: &mut Value, ttl: CacheTtl) -> usize {
    fn walk(node: &mut Value, ttl: CacheTtl, count: &mut usize) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(cache)) = map.get_mut("cache_control") {
                    // Only touch ephemeral breakpoints; anything else is not ours to
                    // reinterpret.
                    if cache.get("type").and_then(Value::as_str) == Some("ephemeral") {
                        cache.insert("ttl".into(), json!(ttl.as_str()));
                        *count += 1;
                    }
                }
                for (_, value) in map.iter_mut() {
                    walk(value, ttl, count);
                }
            }
            Value::Array(items) => {
                for item in items {
                    walk(item, ttl, count);
                }
            }
            _ => {}
        }
    }
    let mut count = 0;
    walk(body, ttl, &mut count);
    count
}

/// Adds a beta flag without dropping any the client already set.
pub fn merge_beta_header(existing: Option<&str>, addition: &str) -> String {
    match existing {
        Some(current) if current.split(',').any(|b| b.trim() == addition) => current.to_string(),
        Some(current) if !current.trim().is_empty() => format!("{current},{addition}"),
        _ => addition.to_string(),
    }
}

/// Usage extracted from a Messages response.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}

impl Usage {
    pub fn from_response(body: &Value) -> Self {
        let usage = body.get("usage").cloned().unwrap_or(Value::Null);
        let get = |key: &str| usage.get(key).and_then(Value::as_i64).unwrap_or(0);
        Self {
            input_tokens: get("input_tokens"),
            output_tokens: get("output_tokens"),
            cache_read: get("cache_read_input_tokens"),
            cache_write: get("cache_creation_input_tokens"),
        }
    }

    /// List-price estimate in dollars.
    ///
    /// Used verbatim on the subscription path, where there is no per-call charge to
    /// read, and as a cross-check on the key path. It is explicitly an estimate: a
    /// budget stated in dollars needs *some* number, and a wrong-but-consistent one
    /// is more useful for stopping a runaway session than no number at all.
    pub fn estimated_usd(&self, input_per_mtok: f64, output_per_mtok: f64) -> f64 {
        let million = 1_000_000.0;
        // Cache reads bill at a tenth of input; cache writes at 1.25x.
        let input_cost = (self.input_tokens as f64
            + self.cache_read as f64 * 0.1
            + self.cache_write as f64 * 1.25)
            * input_per_mtok
            / million;
        let output_cost = self.output_tokens as f64 * output_per_mtok / million;
        input_cost + output_cost
    }
}

/// A session resolved from the proxy's bearer token.
#[derive(Debug, Clone)]
pub struct ProxySession {
    pub user_id: Uuid,
    pub project_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub scopes: Vec<String>,
}

/// Resolves and authorises the caller.
async fn resolve_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<ProxySession, axum::response::Response> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "missing_token"})),
            )
                .into_response()
        })?;

    let row = sqlx::query_as::<_, (Uuid, Option<Uuid>, Vec<String>)>(
        "SELECT user_id, project_id, scopes FROM sessions \
         WHERE token = $1 AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(token)
    .fetch_optional(&state.pg)
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "internal", "message": e.to_string()})),
        )
            .into_response()
    })?;

    let Some((user_id, project_id, scopes)) = row else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "invalid_token"})),
        )
            .into_response());
    };

    if !crate::auth::scopes::permits(&scopes, crate::auth::scopes::LLM_PROXY) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "scope_denied",
                "message": "this token does not carry llm:proxy",
            })),
        )
            .into_response());
    }

    Ok(ProxySession {
        user_id,
        project_id,
        session_id: None,
        scopes,
    })
}

/// Estimates the input tokens in an Anthropic Messages request.
///
/// Counts the system prompt, every message's text content and the tool schemas,
/// at the same ~4-bytes-per-token rate the Context Manager uses. Deliberately an
/// over-estimate for dense English and JSON: a budget enforced with an optimistic
/// estimate is not enforced.
///
/// It does not need to be exact. It needs to catch the case the cap exists for —
/// a prompt that has grown past what the profile says this model should be handed.
#[must_use]
pub fn estimate_request_tokens(payload: &Value) -> u32 {
    fn text_len(v: &Value) -> usize {
        match v {
            Value::String(s) => s.len(),
            Value::Array(a) => a.iter().map(text_len).sum(),
            Value::Object(o) => o.values().map(text_len).sum(),
            _ => 0,
        }
    }

    let mut bytes = 0usize;
    if let Some(system) = payload.get("system") {
        bytes += text_len(system);
    }
    if let Some(messages) = payload.get("messages") {
        bytes += text_len(messages);
    }
    // Tool schemas are part of the prompt and are exactly what the exposure budget
    // exists to bound, so they count.
    if let Some(tools) = payload.get("tools") {
        bytes += text_len(tools);
    }
    u32::try_from(bytes.div_ceil(4)).unwrap_or(u32::MAX)
}

/// Remaining budget for a project, in dollars. `None` means unlimited.
async fn remaining_budget(state: &AppState, project_id: Uuid) -> Option<f64> {
    let row = sqlx::query_as::<_, (Option<rust_decimal::Decimal>,)>(
        "SELECT budget_usd_total FROM research_projects WHERE project_id=$1",
    )
    .bind(project_id)
    .fetch_optional(&state.pg)
    .await
    .ok()??;

    let total: f64 = row.0?.try_into().ok()?;
    let spent = sqlx::query_as::<_, (Option<rust_decimal::Decimal>,)>(
        "SELECT sum(cost_usd) FROM llm_usage WHERE project_id=$1",
    )
    .bind(project_id)
    .fetch_optional(&state.pg)
    .await
    .ok()
    .flatten()
    .and_then(|r| r.0)
    .and_then(|d| TryInto::<f64>::try_into(d).ok())
    .unwrap_or(0.0);

    Some(total - spent)
}

/// POST /llm/v1/messages — Anthropic Messages passthrough.
pub async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let session = match resolve_session(&state, &headers).await {
        Ok(s) => s,
        Err(response) => return response,
    };

    // Budget is checked before the call, not after. A session that has run out must
    // stop spending, and the only place that can be decided is in front of the
    // upstream request.
    if let Some(project_id) = session.project_id {
        if let Some(remaining) = remaining_budget(&state, project_id).await {
            if remaining <= 0.0 {
                return (
                    StatusCode::PAYMENT_REQUIRED,
                    Json(json!({
                        "error": "budget_exhausted",
                        "message": format!(
                            "project {project_id} has spent its total budget"
                        ),
                        "fix": "ask the operator to raise budget_usd_total",
                    })),
                )
                    .into_response();
            }
        }
    }

    let mut payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_json", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    // The context budget, enforced where every call is visible (ADR-0031; harness
    // guide §4.1 and anti-pattern 4: no code path may send an unbudgeted prompt).
    //
    // The Agent SDK assembles its own prompts inside the container, so the platform's
    // Context Manager is not the only assembler. The proxy is: every upstream call
    // passes through here, which makes it the one place a hard cap can actually be a
    // cap rather than an intention.
    //
    // Refuse rather than truncate. Silently trimming a prompt the SDK built would
    // remove something it believed it had sent, and the resulting answer would look
    // like a model failure. A refusal that names the number is a bug report.
    if let Some(profile) = state.profile(payload.get("model").and_then(Value::as_str)) {
        let estimated = estimate_request_tokens(&payload);
        let cap = profile.input_budget_tokens();
        if estimated > cap {
            tracing::warn!(
                model = %profile.model_id,
                estimated,
                cap,
                "refused an over-budget prompt at the proxy"
            );
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({
                    "error": "context_budget_exceeded",
                    "message": format!(
                        "this request is about {estimated} input tokens against a budget of {cap}"
                    ),
                    "fix": "compact the conversation, or move large tool output to the                             workspace and reference it by path",
                    "profile": profile.model_id,
                })),
            )
                .into_response();
        }
    }

    let rewritten = rewrite_cache_ttl(&mut payload, CacheTtl::OneHour);

    let Some(crypto) = state.cred_crypto.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "error": "credentials_unavailable",
                "message": "CRED_KEK is not set, so no provider credential can be unwrapped",
            })),
        )
            .into_response();
    };

    let store = crate::credentials::store::LlmCredentialStore::new(state.pg.clone(), Some(crypto));
    let credential = match store.load(session.user_id, "anthropic").await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({
                    "error": "no_credential",
                    "message": "no Anthropic credential is stored for this user",
                })),
            )
                .into_response()
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "credential_error", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    let beta = merge_beta_header(
        headers.get("anthropic-beta").and_then(|v| v.to_str().ok()),
        "extended-cache-ttl-2025-04-11",
    );

    let client = reqwest::Client::new();
    let upstream = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", credential.api_key.as_str())
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", beta)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .await;

    let response = match upstream {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": "upstream_unreachable", "message": e.to_string()})),
            )
                .into_response()
        }
    };

    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::Null);

    // Record usage even on a non-2xx: a request that failed after the model ran is
    // still billed, and a budget that ignored those would drift.
    let usage = Usage::from_response(&parsed);
    if usage.input_tokens + usage.output_tokens > 0 {
        let model = parsed
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        // Opus list price. Approximate by design — see `estimated_usd`.
        let cost = usage.estimated_usd(15.0, 75.0);
        let _ = sqlx::query(
            "INSERT INTO llm_usage \
               (session_id, project_id, user_id, model, input_tokens, output_tokens, \
                cache_read, cache_write, cost_usd) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(session.session_id)
        .bind(session.project_id)
        .bind(session.user_id)
        .bind(model)
        .bind(usage.input_tokens)
        .bind(usage.output_tokens)
        .bind(usage.cache_read)
        .bind(usage.cache_write)
        .bind(rust_decimal::Decimal::try_from(cost).unwrap_or_default())
        .execute(&state.pg)
        .await;
    }

    tracing::debug!(
        cache_breakpoints_rewritten = rewritten,
        status = status.as_u16(),
        "llm proxy call"
    );

    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        text,
    )
        .into_response()
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_estimate_counts_system_messages_and_tool_schemas() {
        let small = estimate_request_tokens(&json!({"messages": []}));
        assert_eq!(small, 0);

        let with_system = estimate_request_tokens(&json!({
            "system": "a".repeat(4000),
            "messages": []
        }));
        assert!(with_system >= 1000);

        // Tool schemas are part of the prompt, and are exactly what the exposure
        // budget exists to bound.
        let with_tools = estimate_request_tokens(&json!({
            "messages": [],
            "tools": [{"name": "x", "description": "b".repeat(4000)}]
        }));
        assert!(
            with_tools >= 1000,
            "tool schemas must count toward the budget"
        );
    }

    #[test]
    fn nested_content_blocks_are_counted() {
        let nested = estimate_request_tokens(&json!({
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "c".repeat(8000)}
                ]}
            ]
        }));
        assert!(
            nested >= 2000,
            "content blocks are where the text actually lives"
        );
    }

    /// The estimate must never come in under the truth, or the cap leaks.
    #[test]
    fn the_estimate_does_not_under_count_plain_text() {
        let body = json!({"messages": [{"role": "user", "content": "hello world"}]});
        // 11 bytes of content plus the envelope; at 4 bytes/token that is >= 2.
        assert!(estimate_request_tokens(&body) >= 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_control_blocks_are_rewritten_wherever_they_appear() {
        let mut body = json!({
            "system": [{"type": "text", "text": "core", "cache_control": {"type": "ephemeral"}}],
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}
                ]}
            ],
            "tools": [{"name": "t", "cache_control": {"type": "ephemeral"}}]
        });
        let count = rewrite_cache_ttl(&mut body, CacheTtl::OneHour);
        assert_eq!(count, 3, "every breakpoint, at any depth");
        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["ttl"],
            "1h"
        );
        assert_eq!(body["tools"][0]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn rewriting_moves_no_breakpoints() {
        // The CLI chooses where to cache; the proxy only changes how long it lasts.
        // Adding or removing a breakpoint here would change what gets cached and
        // silently alter cost and behaviour.
        let mut body = json!({"messages": [{"role": "user", "content": "plain"}]});
        assert_eq!(rewrite_cache_ttl(&mut body, CacheTtl::OneHour), 0);
        assert_eq!(body["messages"][0]["content"], "plain");
    }

    #[test]
    fn non_ephemeral_cache_blocks_are_left_alone() {
        let mut body = json!({"x": {"cache_control": {"type": "persistent"}}});
        assert_eq!(rewrite_cache_ttl(&mut body, CacheTtl::OneHour), 0);
        assert!(body["x"]["cache_control"].get("ttl").is_none());
    }

    #[test]
    fn beta_headers_are_merged_not_replaced() {
        // Dropping a beta the client asked for would change model behaviour in a way
        // that looks like a model bug.
        assert_eq!(
            merge_beta_header(
                Some("context-1m-2025-08-07"),
                "extended-cache-ttl-2025-04-11"
            ),
            "context-1m-2025-08-07,extended-cache-ttl-2025-04-11"
        );
        assert_eq!(merge_beta_header(None, "x"), "x");
        assert_eq!(merge_beta_header(Some(""), "x"), "x");
    }

    #[test]
    fn a_beta_is_not_added_twice() {
        assert_eq!(merge_beta_header(Some("a,x"), "x"), "a,x");
    }

    #[test]
    fn usage_is_read_from_the_response() {
        let body = json!({"usage": {
            "input_tokens": 100, "output_tokens": 20,
            "cache_read_input_tokens": 5000, "cache_creation_input_tokens": 300
        }});
        let usage = Usage::from_response(&body);
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read, 5000);
        assert_eq!(usage.cache_write, 300);
    }

    #[test]
    fn missing_usage_is_zero_rather_than_an_error() {
        // A malformed or error response must not stop the proxy from replying.
        assert_eq!(Usage::from_response(&json!({})), Usage::default());
        assert_eq!(Usage::from_response(&Value::Null), Usage::default());
    }

    #[test]
    fn cache_reads_cost_a_tenth_of_fresh_input() {
        let fresh = Usage {
            input_tokens: 1_000_000,
            ..Default::default()
        };
        let cached = Usage {
            cache_read: 1_000_000,
            ..Default::default()
        };
        let fresh_cost = fresh.estimated_usd(15.0, 75.0);
        let cached_cost = cached.estimated_usd(15.0, 75.0);
        assert!((fresh_cost - 15.0).abs() < 1e-9);
        assert!((cached_cost - 1.5).abs() < 1e-9);
        assert!(
            cached_cost < fresh_cost,
            "if caching did not show up as cheaper, the budget would not reward it"
        );
    }
}
