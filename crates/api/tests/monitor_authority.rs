//! COMP-007 §8.6 — no agent token can reach any monitor mutation.
//!
//! The brief for this asked for the proof to be **per token scope, not per
//! endpoint**, and the reason is worth stating: an endpoint-by-endpoint test
//! passes on the day it is written and says nothing about the endpoint added next
//! month. So the assertions here are:
//!
//! 1. Over the *authority function*: nothing an agent session can hold satisfies
//!    it, asserted by iterating the agent's whole vocabulary rather than naming
//!    scopes one at a time.
//! 2. Over the *database*: a real minted service token is read back and its scopes
//!    are checked, and the trigger refuses `monitor.control` outright.
//! 3. Over the *source*: every handler in `routes/monitor.rs` that takes a
//!    `BearerToken` and mutates calls `require_control`, so a new route cannot skip
//!    the check by being forgotten.
//!
//! ```bash
//! API_TEST_DATABASE_URL=postgres://trading:trading@localhost:5432/trading \
//!   cargo test -j 2 -p api --test monitor_authority -- --test-threads=1
//! ```

use sqlx::{PgPool, Row};
use uuid::Uuid;

use api::auth::scopes;

async fn pool() -> Option<PgPool> {
    let url = std::env::var("API_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("MONITOR_TEST_DATABASE_URL"))
        .ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

async fn make_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("monitor-auth-{id}@local"))
        .execute(pool)
        .await
        .expect("create user");
    id
}

async fn cleanup(pool: &PgPool, user: Uuid) {
    sqlx::query("DELETE FROM sessions WHERE user_id=$1")
        .bind(user)
        .execute(pool)
        .await
        .ok();
    sqlx::query("DELETE FROM users WHERE user_id=$1")
        .bind(user)
        .execute(pool)
        .await
        .ok();
}

// ── 1. The authority function ───────────────────────────────────────────────

#[test]
fn nothing_in_the_agents_vocabulary_reaches_the_monitor() {
    let held: Vec<String> = scopes::RESEARCH_SCOPES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    assert!(
        !scopes::permits_monitor_control(&held),
        "an agent holding every research scope must still not reach the monitor"
    );
    // And each on its own, so that adding a scope to RESEARCH_SCOPES without
    // thinking about the monitor fails here rather than in production.
    for scope in scopes::RESEARCH_SCOPES {
        assert!(
            !scopes::permits_monitor_control(&[(*scope).to_owned()]),
            "{scope} must not reach the monitor"
        );
    }
}

#[test]
fn the_scope_an_agent_would_need_is_one_it_can_never_be_granted() {
    assert!(scopes::NEVER_FOR_AGENTS.contains(&scopes::MONITOR_CONTROL));
    assert!(scopes::validate_agent_scopes(&[scopes::MONITOR_CONTROL.to_owned()]).is_err());
    assert!(scopes::validate_agent_scopes(&[scopes::WEB_FULL.to_owned()]).is_err());
}

#[test]
fn a_freshly_minted_service_token_holds_nothing_and_reaches_nothing() {
    // What `POST /auth/service-tokens` actually produces is a session with an
    // empty scope array. The failure this guards is an emptiness check somewhere
    // being read as "unrestricted".
    assert!(!scopes::permits_monitor_control(&[]));
}

// ── 2. The database ─────────────────────────────────────────────────────────

#[tokio::test]
async fn the_database_refuses_to_grant_monitor_control_to_any_service_session() {
    // Migration 0075 widens 0038's trigger from project-bound service sessions to
    // every service session. Before that, a service token minted without a project
    // — which is exactly what the mint endpoint produces — could have been granted
    // `web:full` by a direct write and would have satisfied every check in the API.
    let Some(pg) = pool().await else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping");
        return;
    };
    let user = make_user(&pg).await;

    for scope in [scopes::MONITOR_CONTROL, scopes::WEB_FULL, "orders.place"] {
        let attempt = sqlx::query(
            "INSERT INTO sessions (token, user_id, kind, scopes, expires_at)
             VALUES ($1, $2, 'service', ARRAY[$3], now() + INTERVAL '1 day')",
        )
        .bind(format!("tok-{}", Uuid::new_v4()))
        .bind(user)
        .bind(scope)
        .execute(&pg)
        .await;
        assert!(
            attempt.is_err(),
            "the database must refuse {scope} on a service session"
        );
    }

    // A research-scoped service session is fine, and still reaches nothing.
    let token = format!("tok-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO sessions (token, user_id, kind, scopes, expires_at)
         VALUES ($1, $2, 'service', $3, now() + INTERVAL '1 day')",
    )
    .bind(&token)
    .bind(user)
    .bind(
        scopes::RESEARCH_SCOPES
            .iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<String>>(),
    )
    .execute(&pg)
    .await
    .expect("a research service session is legitimate");

    let held: Vec<String> = sqlx::query("SELECT scopes FROM sessions WHERE token = session_token_hash($1)")
        .bind(&token)
        .fetch_one(&pg)
        .await
        .expect("read back")
        .get("scopes");
    assert!(
        !scopes::permits_monitor_control(&held),
        "a real minted agent token, read back from the database, reaches no monitor POST"
    );

    cleanup(&pg, user).await;
}

// ── 3. The source ───────────────────────────────────────────────────────────

#[test]
fn every_mutating_monitor_handler_asks_the_one_authority_question() {
    // Per-endpoint tests rot; this one fails when a route is *added* without the
    // check, which is the way the hole actually gets opened.
    let src = include_str!("../src/routes/monitor.rs");

    // Every `pub async fn` that takes a non-`_`-prefixed `token: BearerToken` is a
    // handler that intends to act on behalf of a caller, and must call
    // `require_control`. A read-only handler names it `_token` and is exempt.
    let mut checked = 0usize;
    let mut offenders: Vec<String> = Vec::new();
    for (i, _) in src.match_indices("pub async fn ") {
        let rest = &src[i..];
        let end = rest.find("\n}\n").map_or(rest.len(), |e| e + 2);
        let body = &rest[..end];
        let name: String = body["pub async fn ".len()..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // `_token: BearerToken` *contains* `token: BearerToken`, so the
        // underscore form has to be excluded explicitly or the exemption this
        // test documents never fires and every read-only handler is reported
        // as an offender. Checking for the parameter with its comma-or-newline
        // boundary is the narrow fix; checking `!body.contains("_token:")` is
        // not, because a handler may take both.
        let acts_on_behalf = body.contains(
            "
    token: BearerToken",
        ) || body.contains("(token: BearerToken");
        if !acts_on_behalf {
            continue;
        }
        if body.contains("require_control(&token)") {
            checked += 1;
        } else {
            offenders.push(name);
        }
    }
    assert!(
        offenders.is_empty(),
        "these monitor handlers take an authenticated token and never ask whether it \
         may act: {offenders:?}"
    );
    assert!(
        checked >= 7,
        "expected the mutating monitor surface to be gated; only {checked} handlers \
         call require_control, which suggests this scan stopped matching"
    );

    // And there is exactly one implementation of the question, so it cannot be
    // answered two different ways in two different places.
    assert_eq!(
        src.matches("fn require_control").count(),
        1,
        "one gate, one definition"
    );
    assert_eq!(
        src.matches("permits_monitor_control").count(),
        1,
        "every handler must go through require_control rather than asking directly"
    );
}

#[test]
fn the_legacy_trading_routes_are_gated_too() {
    // These predate COMP-007 and took any bearer token at all — including a
    // service token minted for an agent. They are the oldest hole in the file and
    // the easiest to forget, because they are not in routes/monitor.rs.
    let src = include_str!("../src/routes/trading.rs");
    for handler in ["trip_kill_switch", "reset_kill_switch"] {
        let i = src
            .find(&format!("pub async fn {handler}"))
            .unwrap_or_else(|| panic!("{handler} not found"));
        let body = &src[i..];
        let end = body.find("\n}\n").map_or(body.len(), |e| e + 2);
        assert!(
            body[..end].contains("permits_monitor_control"),
            "{handler} must require monitor.control"
        );
    }
}
