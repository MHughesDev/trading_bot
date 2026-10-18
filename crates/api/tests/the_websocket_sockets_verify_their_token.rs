//! The two WebSocket routes resolve their `?token=` against the `sessions`
//! table, and refuse the upgrade when it does not resolve.
//!
//! # What was wrong
//!
//! Both sockets demanded a `?token=` query parameter and neither verified it.
//! `ws_live` used the raw token string **as the user id**, so any non-empty
//! string made you that user on the live and paper data feed.
//! `ws_backtest_suite` passed it through `backtest::suite::user_id_from_token`
//! — a UUIDv5 over the token bytes, the M-17 Phase 1 placeholder that
//! `crates/api/src/auth/session.rs` had already replaced everywhere else — so
//! any non-empty string derived a valid-looking user id there too.
//!
//! A browser's `WebSocket` constructor cannot set request headers, which is why
//! the token is in the query string and why `BearerToken` could not be used.
//! That is a reason for a different extractor, not for no check.
//!
//! # What this file pins
//!
//! Two halves, and both are needed.
//!
//! The **structural** half needs no database and therefore always runs. It
//! names `WsBearerToken` exactly, which is the hole in the guard test next
//! door: `every_mounted_route_refuses_an_anonymous_caller` decides a handler is
//! authenticated by asking whether its signature `contains("BearerToken")`, and
//! a substring test would also accept a type merely *named* like one. Here the
//! type is named, its resolver is shown to read `sessions`, and the placeholder
//! is shown to have no caller left.
//!
//! The **behavioural** half proves the refusal against a real Postgres. It is
//! skipped without one, which is exactly why it cannot be the only half.
//!
//! ```bash
//! cargo test -j 2 -p api --test the_websocket_sockets_verify_their_token
//! API_TEST_DATABASE_URL=postgres://trading:trading@localhost:5433/trading \
//!   cargo test -j 2 -p api --test the_websocket_sockets_verify_their_token
//! ```

use std::path::{Path, PathBuf};

use sqlx::PgPool;
use uuid::Uuid;

use api::ws::resolve_ws_session;

fn src(rel: &str) -> String {
    let p: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("rs") {
                out.push(p);
            }
        }
    }
    out
}

// ── Structural: the mechanism is where it is claimed to be ───────────────────

/// The text between a function's parentheses.
///
/// Depth-balanced rather than "up to the first `)`". An axum handler's argument
/// list is full of destructuring patterns — `State(state)`, `Query(query)` — so
/// the first `)` is nearly always the wrong one, and a reader that stops there
/// reads a different set of arguments than the one it reports.
fn signature<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = src.find(&format!("fn {name}("))?;
    let open = at + src[at..].find('(')?;
    let mut depth = 0usize;
    for (k, c) in src[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open..=open + k]);
                }
            }
            _ => {}
        }
    }
    None
}

#[test]
fn the_signature_reader_does_not_stop_at_a_destructuring_pattern() {
    // The bug this guards: `State(state)` closes a paren that is not the
    // argument list's, and an unbalanced reader would report the arguments as
    // ending there — silently dropping every argument that follows.
    let src = "async fn h(State(s): State<A>, session: WsBearerToken) -> Response {}\n";
    let sig = signature(src, "h").expect("reader found no signature");
    assert!(
        sig.contains("WsBearerToken"),
        "the reader stopped early and returned {sig:?}"
    );
}

/// Both handlers take the verified-session extractor by its real name.
///
/// Spelled out per file rather than scanned generically: these are the only two
/// sockets, and naming them means a third one added without the extractor does
/// not quietly inherit this test's blessing.
#[test]
fn both_sockets_take_the_verified_session_extractor() {
    for (file, handler) in [
        ("ws/live.rs", "ws_live"),
        ("ws/backtest.rs", "ws_backtest_suite"),
    ] {
        let text = src(file);
        let signature = signature(&text, handler)
            .unwrap_or_else(|| panic!("{file} no longer defines `{handler}`"));
        assert!(
            signature.contains("WsBearerToken"),
            "`{handler}` does not take `WsBearerToken`, so nothing resolved its token before the \
             upgrade. Its signature is {signature:?}. A browser WebSocket cannot send an \
             Authorization header; that is a reason for this extractor, not for no check."
        );
    }
}

/// The extractor's resolver really reads the sessions table, with the same
/// expiry predicate the REST surface uses.
///
/// Without this the test above would pass for an extractor that resolved
/// nothing — which is precisely the shape of the defect it replaced.
#[test]
fn the_resolver_reads_the_sessions_table_and_checks_expiry() {
    let text = src("ws/auth.rs");
    assert!(
        text.contains("crate::auth::session::resolve(pg, token)"),
        "`resolve_ws_session` no longer goes through the one session resolver. The \
         extractor's whole content is that lookup; without it the type is a name."
    );
    // And the one resolver is the hashed, expiry-checked lookup (Set E 1.1).
    let session = src("auth/session.rs");
    assert!(
        session
            .contains("FROM sessions WHERE token = session_token_hash($1) AND expires_at > now()"),
        "`auth::session::resolve` no longer selects from `sessions` by the token's digest \
         with an expiry check"
    );
    assert!(
        text.contains("StatusCode::UNAUTHORIZED"),
        "`ws/auth.rs` refuses nobody"
    );
}

/// A failure to resolve refuses. It never falls back to an id derived from the
/// token, and a database error is a refusal too.
///
/// This is the rule the old code broke in the most direct way available: it
/// treated the credential as the identity.
#[test]
fn the_websocket_surface_never_derives_an_identity_from_the_token() {
    for file in ["ws/auth.rs", "ws/live.rs", "ws/backtest.rs"] {
        let text = src(file);
        assert!(
            !text.contains("user_id_from_token"),
            "{file} still reaches for `user_id_from_token`. That function derives a user id from \
             a string nobody checked; an unresolvable token is a refusal, not a different user."
        );
    }

    // And nowhere else in the crate, either — the point of removing the last
    // caller is that the next person cannot find one to copy.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let reaching: Vec<String> = rust_files(&root)
        .into_iter()
        .filter(|f| std::fs::read_to_string(f).is_ok_and(|t| t.contains("user_id_from_token")))
        .map(|f| f.display().to_string())
        .collect();
    assert!(
        reaching.is_empty(),
        "these files call the M-17 placeholder `user_id_from_token`: {reaching:#?}. It is a \
         UUIDv5 over the token bytes and proves nothing about who is calling."
    );
}

// ── Behavioural: it actually refuses ─────────────────────────────────────────

async fn pool() -> Option<PgPool> {
    let url = std::env::var("API_TEST_DATABASE_URL").ok()?;
    Some(PgPool::connect(&url).await.expect("connect"))
}

async fn make_user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (user_id, email) VALUES ($1, $2)")
        .bind(id)
        .bind(format!("ws-auth-test-{id}@local"))
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

#[tokio::test]
async fn a_token_that_is_absent_empty_or_unknown_is_refused() {
    let Some(pool) = pool().await else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping the live half");
        return;
    };

    assert!(
        resolve_ws_session(&pool, None).await.is_err(),
        "a socket opened for a caller who sent no token"
    );
    assert!(
        resolve_ws_session(&pool, Some("")).await.is_err(),
        "a socket opened for an empty token"
    );
    // The exact shape that used to work: any non-empty string.
    for guess in ["x", "admin", &Uuid::new_v4().to_string()] {
        assert!(
            resolve_ws_session(&pool, Some(guess)).await.is_err(),
            "the token {guess:?} is in no session row and it opened a socket anyway"
        );
    }
}

#[tokio::test]
async fn a_real_session_resolves_to_its_own_user_and_an_expired_one_does_not() {
    let Some(pool) = pool().await else {
        eprintln!("API_TEST_DATABASE_URL unset — skipping the live half");
        return;
    };
    let user = make_user(&pool).await;
    let live = format!("ws-live-{}", Uuid::new_v4());
    let expired = format!("ws-expired-{}", Uuid::new_v4());

    sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, now() + INTERVAL '1 hour')")
        .bind(&live)
        .bind(user)
        .execute(&pool)
        .await
        .expect("insert live session");
    sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, now() - INTERVAL '1 hour')")
        .bind(&expired)
        .bind(user)
        .execute(&pool)
        .await
        .expect("insert expired session");

    let resolved = resolve_ws_session(&pool, Some(&live))
        .await
        .expect("a live session token must open the socket");
    assert_eq!(
        resolved.user_id(),
        user,
        "the socket resolved to some other user than the one holding the session"
    );
    // The id is the sessions-table id, not a function of the token — which is
    // what makes it equal to the `created_by` the REST routes stamp on frames.
    assert_ne!(
        resolved.user_id(),
        Uuid::new_v5(&Uuid::NAMESPACE_OID, live.as_bytes()),
        "the socket's user id is still derived from the token string"
    );

    assert!(
        resolve_ws_session(&pool, Some(&expired)).await.is_err(),
        "an expired session opened a socket"
    );

    cleanup(&pool, user).await;
}
