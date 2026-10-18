//! Every user connects their own provider key, and only they can use it
//! (PROVIDERS track, 2026-10-05; docs/operations/connect-ai-provider.md).
//!
//! The whole path over HTTP, through the platform's own router and `AppState`:
//! paste a key -> it is verified with a real call -> it shows as connected with a
//! masked suffix and a verified time -> the provider's models list -> a desk
//! conversation on that provider -> the agent loop calls the provider **with that
//! user's key** -> the run's usage is recorded against that user.
//!
//! The provider is a local HTTP server speaking OpenAI's wire format, because
//! OpenAI is the half of the path that had never run: every hosted profile was
//! Claude's, and GPT-5-family models refuse the `temperature: 0.0` every frontier
//! profile sends. The fake refuses it the same way, so the retry is exercised too.
//!
//! Three properties are asserted across everything the test does, not only at
//! one point:
//!
//! * **isolation** — Bob cannot see, list with, verify, default to, delete, or run
//!   on Alice's key, and his own run reaches the provider with his key only;
//! * **no echo** — no response body, no stored error, no agent message and no log
//!   line contains any key the test used, including one the provider echoed back
//!   in its 401;
//! * **grants** — the platform's role can read and write the new columns
//!   (`SET LOCAL ROLE app_role`), because the tests run as owner.
//!
//! ```bash
//! DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5433/postgres \
//!   cargo test -j 2 -p api --test each_user_brings_their_own_key -- --ignored --nocapture
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use sqlx::{Connection, Executor, PgConnection, PgPool};
use tower::ServiceExt;
use uuid::Uuid;

// Shaped like real keys and long enough to scrub; none of them is real.
const ALICE_KEY: &str = "sk-proj-ALICEfixture0000000000000000aaaa";
const BOB_KEY: &str = "sk-proj-BOBfixture00000000000000000000bbbb";
const BAD_KEY: &str = "sk-proj-WRONGfixture000000000000000000zzzz";

// ── log capture ──────────────────────────────────────────────────────────────

/// Every log line this process emits, as text, so it can be scanned for keys.
fn log_buffer() -> &'static Arc<Mutex<String>> {
    static BUF: OnceLock<Arc<Mutex<String>>> = OnceLock::new();
    BUF.get_or_init(|| {
        let buf = Arc::new(Mutex::new(String::new()));
        let _ = tracing::subscriber::set_global_default(Capture {
            buf: Arc::clone(&buf),
            next: std::sync::atomic::AtomicU64::new(1),
        });
        buf
    })
}

/// A subscriber that writes every event and span field, at every level, into a
/// string. Hand-rolled because `api` has no `tracing-subscriber` dependency and
/// one test is not a reason to add one.
struct Capture {
    buf: Arc<Mutex<String>>,
    next: std::sync::atomic::AtomicU64,
}

struct Fields<'a>(&'a mut String);

impl tracing::field::Visit for Fields<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, "{}={:?} ", field.name(), value);
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn register_callsite(
        &self,
        _: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        tracing::subscriber::Interest::always()
    }
    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::TRACE)
    }
    fn new_span(&self, attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut line = format!("span {} ", attrs.metadata().name());
        attrs.record(&mut Fields(&mut line));
        if let Ok(mut b) = self.buf.lock() {
            b.push_str(&line);
            b.push('\n');
        }
        tracing::span::Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut line = String::from("record ");
        values.record(&mut Fields(&mut line));
        if let Ok(mut b) = self.buf.lock() {
            b.push_str(&line);
            b.push('\n');
        }
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut line = format!(
            "{} {} ",
            event.metadata().level(),
            event.metadata().target()
        );
        event.record(&mut Fields(&mut line));
        if let Ok(mut b) = self.buf.lock() {
            b.push_str(&line);
            b.push('\n');
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

// ── the fake provider ────────────────────────────────────────────────────────

/// One request the fake provider received.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    bearer: String,
    body: Value,
}

#[derive(Default)]
struct Provider {
    seen: Mutex<Vec<Seen>>,
    /// Alice's key, revoked at the provider.
    alice_revoked: AtomicBool,
}

impl Provider {
    fn accepts(&self, bearer: &str) -> bool {
        (bearer == ALICE_KEY && !self.alice_revoked.load(Ordering::SeqCst)) || bearer == BOB_KEY
    }
    fn chats_with(&self, key: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == "/v1/chat/completions" && s.bearer == key)
            .cloned()
            .collect()
    }
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string()
}

/// OpenAI's 401, which names the key it was given. Real providers do this; the
/// platform must not pass it on.
fn rejected(key: &str) -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": {
            "message": format!("Incorrect API key provided: {key}. You can find your API key at https://platform.openai.com/account/api-keys."),
            "type": "invalid_request_error",
            "code": "invalid_api_key"
        }})),
    )
}

/// The chat-completions answer. Behaves like a GPT-5-family model:
/// - `temperature` is refused with a 400 naming it;
/// - a request carrying a JSON schema (the title generator) gets a title;
/// - the first agent call that offers tools gets a call to a tool that needs no
///   arguments, and anything after a tool result finishes the task through the
///   loop's own `finish_task` tool, which is how a native-decode run ends.
fn chat_reply(body: &Value) -> (StatusCode, Value) {
    if body.get("temperature").is_some() {
        return (
            StatusCode::BAD_REQUEST,
            json!({"error": {
                "message": "Unsupported value: 'temperature' does not support 0.0 with this model. Only the default (1) value is supported.",
                "type": "invalid_request_error", "param": "temperature", "code": "unsupported_value"
            }}),
        );
    }
    let usage = json!({"prompt_tokens": 120, "completion_tokens": 9});
    if body.get("response_format").is_some() {
        return (
            StatusCode::OK,
            json!({"choices": [{"finish_reason": "stop",
                "message": {"content": "{\"title\":\"Instruments with clean bars\"}"}}],
                "usage": usage}),
        );
    }
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let after_tool = messages.iter().any(|m| m["role"] == "tool");
    let no_arg_tool = body["tools"].as_array().and_then(|tools| {
        tools.iter().find_map(|t| {
            let f = &t["function"];
            let required = f["parameters"]["required"].as_array().map_or(0, Vec::len);
            (required == 0).then(|| f["name"].as_str().unwrap_or("").to_string())
        })
    });
    let offers = |name: &str| {
        body["tools"]
            .as_array()
            .is_some_and(|t| t.iter().any(|x| x["function"]["name"] == name))
    };
    if after_tool && offers("finish_task") {
        return (
            StatusCode::OK,
            json!({"choices": [{"finish_reason": "tool_calls", "message": {
                "content": null,
                "tool_calls": [{"id": "call_fixture_2", "type": "function",
                    "function": {"name": "finish_task",
                        "arguments": "{\"result\":\"The fixture provider has nothing to report.\",\"evidence\":\"the one tool this turn called answered with an error, so nothing is established\"}"}}]
            }}], "usage": usage}),
        );
    }
    match (after_tool, no_arg_tool) {
        (false, Some(name)) if !name.is_empty() => (
            StatusCode::OK,
            json!({"choices": [{"finish_reason": "tool_calls", "message": {
                "content": null,
                "tool_calls": [{"id": "call_fixture_1", "type": "function",
                    "function": {"name": name, "arguments": "{}"}}]
            }}], "usage": usage}),
        ),
        _ => (
            StatusCode::OK,
            json!({"choices": [{"finish_reason": "stop", "message": {
                "content": "Nothing to report from the fixture provider."
            }}], "usage": usage}),
        ),
    }
}

async fn fake_openai() -> (String, Arc<Provider>) {
    let state = Arc::new(Provider::default());
    let models_state = Arc::clone(&state);
    let chat_state = Arc::clone(&state);
    let app = Router::new()
        .route(
            "/v1/models",
            get(move |headers: HeaderMap| {
                let p = Arc::clone(&models_state);
                async move {
                    let key = bearer(&headers);
                    p.seen.lock().unwrap().push(Seen {
                        path: "/v1/models".into(),
                        bearer: key.clone(),
                        body: Value::Null,
                    });
                    if !p.accepts(&key) {
                        return rejected(&key);
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"object": "list", "data": [
                            {"id": "gpt-5", "object": "model"},
                            {"id": "gpt-4o-mini", "object": "model"},
                            {"id": "text-embedding-3-small", "object": "model"},
                            {"id": "whisper-1", "object": "model"}
                        ]})),
                    )
                }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |headers: HeaderMap, Json(body): Json<Value>| {
                let p = Arc::clone(&chat_state);
                async move {
                    let key = bearer(&headers);
                    p.seen.lock().unwrap().push(Seen {
                        path: "/v1/chat/completions".into(),
                        bearer: key.clone(),
                        body: body.clone(),
                    });
                    if !p.accepts(&key) {
                        return rejected(&key);
                    }
                    let (status, reply) = chat_reply(&body);
                    (status, Json(reply))
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (base, state)
}

// ── the platform ─────────────────────────────────────────────────────────────

fn database_url() -> String {
    std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@127.0.0.1:5433/postgres".into())
}

fn with_db(url: &str, db: &str) -> String {
    let cut = url.rfind('/').expect("url has a path");
    format!("{}/{db}", &url[..cut])
}

/// A fresh database with every migration that applies here.
async fn scratch() -> (PgPool, String) {
    let name = format!("byok_{}", Uuid::new_v4().simple());
    let mut admin = PgConnection::connect(&with_db(&database_url(), "postgres"))
        .await
        .expect("postgres reachable");
    admin
        .execute(format!("CREATE DATABASE {name}").as_str())
        .await
        .unwrap();
    let pg = PgPool::connect(&with_db(&database_url(), &name))
        .await
        .unwrap();
    let mut skipped = Vec::new();
    for mig in sqlx::migrate!("../../migrations").iter() {
        if mig.migration_type.is_down_migration() {
            continue;
        }
        let mut tx = pg.begin().await.unwrap();
        match tx.execute(&*mig.sql).await {
            Ok(_) => tx.commit().await.unwrap(),
            Err(_) => {
                tx.rollback().await.ok();
                skipped.push(mig.version);
            }
        }
    }
    assert!(
        !skipped.contains(&34) && !skipped.contains(&250),
        "the credential migrations must apply: skipped {skipped:?}"
    );
    if !skipped.is_empty() {
        eprintln!("scratch database skipped migrations {skipped:?}");
    }
    (pg, name)
}

async fn drop_db(pg: PgPool, name: &str) {
    pg.close().await;
    let mut admin = PgConnection::connect(&with_db(&database_url(), "postgres"))
        .await
        .unwrap();
    admin
        .execute(format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

fn profiles_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../config/profiles")
}

/// The platform's state, built as `main.rs` builds it, with nothing spawned and
/// a test KEK in place of `CRED_KEK`.
fn state(pg: PgPool) -> api::AppState {
    let clickhouse_url = "http://127.0.0.1:9/none".to_string();
    let kill_switch = Arc::new(risk::KillSwitch::new(false));
    let risk_gate = Arc::new(risk::RiskGate::new(
        risk::GlobalRiskLimits::default(),
        Arc::clone(&kill_switch),
    ));
    let paper_engine = Arc::new(execution::paper::PaperTradingEngine::realistic());
    let execution_engine = Arc::new(execution::ExecutionEngine::new(Arc::new(
        paper_engine.multi_asset_broker(),
    )));
    let gateway = Arc::new(ui_gateway::SubscriptionRegistry::new(Arc::new(
        demand_manager::DemandRegistry::new(Arc::new(demand_manager::NoopPipelineFactory)),
    )));
    let backtest = backtest::BacktestManager::new(clickhouse_url.clone(), pg.clone());
    let models = model_registry::ModelManager::from_env(pg.clone());
    let inference = model_registry::InferenceGateway::new(
        pg.clone(),
        Arc::new(model_registry::sidecar::SidecarClient::from_env()),
    );
    let profiles = Arc::new(
        harness::ProfileSet::load_dir(&profiles_dir()).expect("the shipped profiles load"),
    );
    let agent = Arc::new(
        api::agent::AgentManager::new(
            pg.clone(),
            "http://127.0.0.1:9".into(),
            cfg::model::AgentConfig::default(),
        )
        .with_profiles(profiles),
    );
    let mut state = api::AppState::new(
        pg,
        risk_gate,
        kill_switch,
        execution_engine,
        paper_engine,
        gateway,
        backtest,
        models,
        inference,
        cfg::model::EmailConfig::default(),
        clickhouse_url,
        None,
        agent,
        None,
    );
    state.cred_crypto = Some(Arc::new(
        api::credentials::CredentialCrypto::new(vec![0x5a; 32], 1).expect("a test KEK"),
    ));
    state
}

/// Every response body the test received, for the final no-echo scan.
type Bodies = Arc<Mutex<Vec<String>>>;

async fn call(
    app: &Router,
    bodies: &Bodies,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 << 20)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    bodies
        .lock()
        .unwrap()
        .push(format!("{method} {uri} -> {status}: {text}"));
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn signed_in(pg: &PgPool, who: &str) -> (Uuid, String) {
    let (user,): (Uuid,) =
        sqlx::query_as("INSERT INTO users (email) VALUES ($1) RETURNING user_id")
            .bind(format!("{who}-{}@byok.test", Uuid::new_v4().simple()))
            .fetch_one(pg)
            .await
            .unwrap();
    let token = format!("tok_{}", Uuid::new_v4().simple());
    sqlx::query("INSERT INTO sessions (token, user_id) VALUES ($1, $2)")
        .bind(&token)
        .bind(user)
        .execute(pg)
        .await
        .unwrap();
    (user, token)
}

fn provider_row<'a>(status: &'a Value, provider: &str) -> &'a Value {
    status["providers"]
        .as_array()
        .expect("a provider list")
        .iter()
        .find(|p| p["provider"] == provider)
        .unwrap_or_else(|| panic!("{provider} is listed: {status}"))
}

async fn wait_until_over(pg: &PgPool, run: Uuid) -> String {
    for _ in 0..240 {
        let (status,): (String,) =
            sqlx::query_as("SELECT status FROM agent_runs WHERE run_id = $1")
                .bind(run)
                .fetch_one(pg)
                .await
                .expect("the run row");
        if !api::agent::conversations::ACTIVE_RUN_STATES.contains(&status.as_str()) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("run {run} did not finish in a minute");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a Postgres that can create databases (DATABASE_URL, default 127.0.0.1:5433)"]
async fn each_user_connects_verifies_and_runs_on_their_own_key() {
    let logs = log_buffer();
    let (base, provider) = fake_openai().await;
    let (pg, name) = scratch().await;
    let app = api::routes::router(state(pg.clone()));
    let bodies: Bodies = Arc::default();
    let (alice, alice_tok) = signed_in(&pg, "alice").await;
    let (_bob, bob_tok) = signed_in(&pg, "bob").await;

    // ── Save: a wrong key is refused, named, and not stored ──────────────────
    let (s, b) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/credentials/openai",
        &alice_tok,
        Some(json!({"api_key": BAD_KEY, "base_url": base})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["error"], "key_rejected");
    let reason = b["message"].as_str().unwrap_or("");
    assert!(
        reason.contains("Incorrect API key provided"),
        "the provider's reason reaches the card: {reason}"
    );
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(
        provider_row(&status, "openai")["configured"],
        false,
        "a rejected key is not stored"
    );

    // ── Save: a good key is verified, stored, and shown masked ───────────────
    let (s, b) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/credentials/openai",
        &alice_tok,
        Some(json!({"api_key": ALICE_KEY, "base_url": base})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["verified"], true);
    assert_eq!(b["key_last4"], "aaaa");
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    let row = provider_row(&status, "openai");
    assert_eq!(row["configured"], true);
    assert_eq!(row["key_last4"], "aaaa");
    assert!(row["verified_at"].is_string(), "a verified time: {row}");
    assert!(row["last_error"].is_null());
    assert_eq!(
        row["is_default"], true,
        "the first connected provider is the default"
    );
    assert_eq!(status["encryption_available"], true);
    assert!(status["encryption_problem"].is_null());
    // vLLM is listed: 0034's CHECK refused it, 0250 admits it.
    provider_row(&status, "vllm");

    // ── Models, with the stored key ──────────────────────────────────────────
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/openai/models",
        &alice_tok,
        Some(json!({})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let ids: Vec<&str> = b["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["id"].as_str())
        .collect();
    // Chat models only, and of those only the ones the platform enables.
    assert_eq!(ids, ["gpt-4o-mini"], "enabled chat models only");

    // ── Default ──────────────────────────────────────────────────────────────
    let (s, b) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/default",
        &alice_tok,
        Some(json!({"provider": "openai", "model": "gpt-4o-mini"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(provider_row(&status, "openai")["default_model"], "gpt-4o-mini");
    let (s, _) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/default",
        &alice_tok,
        Some(json!({"provider": "anthropic", "model": "claude-sonnet-5"})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::NOT_FOUND,
        "a default cannot name a provider nobody connected"
    );

    // ── Re-verify: a key revoked at the provider says so, then recovers ──────
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/credentials/openai/verify",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    provider.alice_revoked.store(true, Ordering::SeqCst);
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/credentials/openai/verify",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["error"], "key_rejected");
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    let row = provider_row(&status, "openai");
    let stored = row["last_error"].as_str().expect("the failure is recorded");
    assert!(stored.contains("rejected"), "{stored}");
    assert!(
        stored.contains("[redacted]"),
        "the echoed key was scrubbed: {stored}"
    );
    provider.alice_revoked.store(false, Ordering::SeqCst);
    let (s, _) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/credentials/openai/verify",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    assert!(
        provider_row(&status, "openai")["last_error"].is_null(),
        "a passing check clears the error"
    );

    // ── Isolation: Bob sees, lists, verifies, defaults and deletes nothing ───
    let (_, status) = call(&app, &bodies, "GET", "/api/llm/credentials", &bob_tok, None).await;
    let row = provider_row(&status, "openai");
    assert_eq!(row["configured"], false, "Bob does not see Alice's key");
    assert!(row["key_last4"].is_null());
    let (s, _) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/openai/models",
        &bob_tok,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::UNPROCESSABLE_ENTITY,
        "Bob cannot list models with Alice's key"
    );
    let (s, _) = call(
        &app,
        &bodies,
        "POST",
        "/api/llm/credentials/openai/verify",
        &bob_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/default",
        &bob_tok,
        Some(json!({"provider": "openai", "model": "gpt-4o-mini"})),
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (s, _) = call(
        &app,
        &bodies,
        "DELETE",
        "/api/llm/credentials/openai",
        &bob_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND, "Bob cannot delete Alice's key");

    // ── Alice's desk conversation runs on her key ────────────────────────────
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        "/api/agent/conversations",
        &alice_tok,
        Some(json!({"provider": "openai", "model": "gpt-4o-mini"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let alice_convo = b["conversation_id"].as_str().unwrap().to_string();
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        &format!("/api/agent/conversations/{alice_convo}/messages"),
        &alice_tok,
        Some(json!({"text": "What instruments do we have clean 1m bars for?"})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let alice_run: Uuid = b["run_id"].as_str().unwrap().parse().unwrap();
    let ended = wait_until_over(&pg, alice_run).await;
    let alice_calls = provider.chats_with(ALICE_KEY);
    eprintln!(
        "alice's run ended `{ended}` after {} provider calls",
        alice_calls.len()
    );
    for c in &alice_calls {
        let roles: Vec<&str> = c.body["messages"]
            .as_array()
            .map(|m| m.iter().filter_map(|x| x["role"].as_str()).collect())
            .unwrap_or_default();
        eprintln!(
            "  call: temperature={} tools={} response_format={} roles={roles:?}",
            c.body.get("temperature").is_some(),
            c.body["tools"].as_array().map_or(0, Vec::len),
            c.body.get("response_format").is_some(),
        );
    }
    let (err,): (Option<String>,) =
        sqlx::query_as("SELECT error FROM agent_runs WHERE run_id = $1")
            .bind(alice_run)
            .fetch_one(&pg)
            .await
            .unwrap();
    eprintln!("  run error: {err:?}");
    let agent_calls: Vec<&Seen> = alice_calls
        .iter()
        .filter(|c| c.body.get("response_format").is_none())
        .collect();
    assert!(
        !agent_calls.is_empty(),
        "the agent loop reached the provider with Alice's key"
    );
    assert!(
        agent_calls.iter().all(|c| c.body["model"] == "gpt-4o-mini"),
        "every call names the conversation's model"
    );
    // The first call on the model (the title, or the first step) asked for
    // `temperature`, was refused as GPT-5 refuses it, and everything after went
    // without it: the refusal is paid once, not once per step.
    assert!(
        alice_calls[0].body.get("temperature").is_some()
            && alice_calls[1..]
                .iter()
                .all(|c| c.body.get("temperature").is_none()),
        "the refused temperature was retried without it, and remembered"
    );
    assert!(
        agent_calls
            .iter()
            .any(|c| c.body["tools"].as_array().is_some_and(|t| !t.is_empty())),
        "the loop offered tools in OpenAI's function format"
    );
    // A tool call, executed, and its result fed back as OpenAI's `tool` role.
    let fed_back = agent_calls.iter().any(|c| {
        c.body["messages"].as_array().is_some_and(|m| {
            m.iter()
                .any(|msg| msg["role"] == "tool" && msg["tool_call_id"] == "call_fixture_1")
                && m.iter().any(|msg| {
                    msg["role"] == "assistant"
                        && msg["tool_calls"][0]["id"] == "call_fixture_1"
                        && msg["tool_calls"][0]["function"]["arguments"].is_string()
                })
        })
    });
    assert!(
        fed_back,
        "a tool call round-tripped through the OpenAI wire format"
    );
    assert_eq!(ended, "completed", "the run finished through `finish_task`");

    // Usage is recorded on Alice's run, against Alice.
    let (owner, prov, model, tokens_in, tokens_out): (Uuid, String, String, i64, i64) = sqlx::query_as(
        "SELECT user_id, provider, model, tokens_in, tokens_out FROM agent_runs WHERE run_id = $1",
    )
    .bind(alice_run)
    .fetch_one(&pg)
    .await
    .unwrap();
    assert_eq!(
        (owner, prov.as_str(), model.as_str()),
        (alice, "openai", "gpt-4o-mini")
    );
    assert!(
        tokens_in > 0 && tokens_out > 0,
        "usage recorded: {tokens_in} in, {tokens_out} out"
    );

    // ── Bob cannot run on Alice's key, and runs on his own ───────────────────
    let (s, _) = call(
        &app,
        &bodies,
        "POST",
        &format!("/api/agent/conversations/{alice_convo}/messages"),
        &bob_tok,
        Some(json!({"text": "use her key"})),
    )
    .await;
    assert_ne!(
        s,
        StatusCode::ACCEPTED,
        "Bob cannot send into Alice's conversation"
    );
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        "/api/agent/conversations",
        &bob_tok,
        Some(json!({"provider": "openai", "model": "gpt-4o-mini"})),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED);
    let bob_convo = b["conversation_id"].as_str().unwrap().to_string();
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        &format!("/api/agent/conversations/{bob_convo}/messages"),
        &bob_tok,
        Some(json!({"text": "hello"})),
    )
    .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "no key, no run: {b}");
    assert_eq!(b["error"], "provider_not_configured");
    let before = provider.chats_with(ALICE_KEY).len();
    let (s, _) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/credentials/openai",
        &bob_tok,
        Some(json!({"api_key": BOB_KEY, "base_url": base})),
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = call(
        &app,
        &bodies,
        "POST",
        &format!("/api/agent/conversations/{bob_convo}/messages"),
        &bob_tok,
        Some(json!({"text": "hello"})),
    )
    .await;
    assert_eq!(s, StatusCode::ACCEPTED, "{b}");
    let bob_run: Uuid = b["run_id"].as_str().unwrap().parse().unwrap();
    wait_until_over(&pg, bob_run).await;
    assert!(
        !provider.chats_with(BOB_KEY).is_empty(),
        "Bob's run used Bob's key"
    );
    assert_eq!(
        provider.chats_with(ALICE_KEY).len(),
        before,
        "and never Alice's"
    );

    // ── The platform role reads and writes the new columns ───────────────────
    let mut tx = pg.begin().await.unwrap();
    tx.execute("SET LOCAL ROLE app_role").await.unwrap();
    let (n,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM llm_credentials WHERE verified_at IS NOT NULL AND is_default",
    )
    .fetch_one(&mut *tx)
    .await
    .expect("app_role reads the 0250 columns");
    assert_eq!(n, 2, "one default each for Alice and Bob");
    sqlx::query("UPDATE llm_credentials SET last_checked_at = now() WHERE user_id = $1")
        .bind(alice)
        .execute(&mut *tx)
        .await
        .expect("app_role writes them");
    tx.rollback().await.unwrap();
    // One default per user, held by the schema.
    let second_default = sqlx::query(
        "INSERT INTO llm_credentials (user_id, provider, ciphertext, nonce, wrapped_dek, is_default) \
         VALUES ($1, 'anthropic', '\\x00', '\\x00', '\\x00', true)",
    )
    .bind(alice)
    .execute(&pg)
    .await;
    assert!(
        second_default.is_err(),
        "a second default for one user is refused"
    );

    // ── Remove ───────────────────────────────────────────────────────────────
    let (s, _) = call(
        &app,
        &bodies,
        "DELETE",
        "/api/llm/credentials/openai",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::OK);
    let (_, status) = call(
        &app,
        &bodies,
        "GET",
        "/api/llm/credentials",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(provider_row(&status, "openai")["configured"], false);
    let (s, _) = call(
        &app,
        &bodies,
        "DELETE",
        "/api/llm/credentials/openai",
        &alice_tok,
        None,
    )
    .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    let (_, status) = call(&app, &bodies, "GET", "/api/llm/credentials", &bob_tok, None).await;
    assert_eq!(
        provider_row(&status, "openai")["configured"],
        true,
        "removing Alice's key left Bob's"
    );

    // ── No key, anywhere ─────────────────────────────────────────────────────
    let everything_said = bodies.lock().unwrap().join("\n");
    let stored: Vec<(String,)> = sqlx::query_as(
        "SELECT coalesce(last_error, '') FROM llm_credentials \
         UNION ALL SELECT coalesce(error, '') || coalesce(summary, '') FROM agent_runs \
         UNION ALL SELECT content_json::text FROM agent_messages",
    )
    .fetch_all(&pg)
    .await
    .unwrap();
    let stored = stored
        .into_iter()
        .map(|r| r.0)
        .collect::<Vec<_>>()
        .join("\n");
    let logged = logs.lock().unwrap().clone();
    assert!(!logged.is_empty(), "the log capture is live");
    for key in [ALICE_KEY, BOB_KEY, BAD_KEY] {
        assert!(!everything_said.contains(key), "a response echoed {key}");
        assert!(!stored.contains(key), "a stored row holds {key}");
        assert!(!logged.contains(key), "a log line holds {key}");
    }

    drop_db(pg, &name).await;
}

/// Without `CRED_KEK`, the save is refused with the operator's instruction, and
/// the status says the same thing so the card can print it before anyone pastes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs a Postgres that can create databases (DATABASE_URL, default 127.0.0.1:5433)"]
async fn without_a_kek_the_operator_is_told_exactly_what_to_set() {
    let (pg, name) = scratch().await;
    let mut st = state(pg.clone());
    st.cred_crypto = None;
    let app = api::routes::router(st);
    let bodies: Bodies = Arc::default();
    let (_, tok) = signed_in(&pg, "carol").await;

    let (s, b) = call(
        &app,
        &bodies,
        "PUT",
        "/api/llm/credentials/anthropic",
        &tok,
        Some(json!({"api_key": BAD_KEY})),
    )
    .await;
    assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE, "{b}");
    assert_eq!(b["error"], "cred_kek_missing");
    assert_eq!(b["operator_action"], "set CRED_KEK");
    let msg = b["message"].as_str().unwrap();
    assert!(
        msg.contains("CRED_KEK") && msg.contains("64 hex characters") && msg.contains("restart"),
        "{msg}"
    );

    let (s, b) = call(&app, &bodies, "GET", "/api/llm/credentials", &tok, None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(b["encryption_available"], false);
    assert!(b["encryption_problem"]
        .as_str()
        .unwrap()
        .contains("CRED_KEK"));
    assert!(!bodies.lock().unwrap().join("\n").contains(BAD_KEY));

    drop_db(pg, &name).await;
}
