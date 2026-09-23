//! Every route this crate mounts refuses a caller who holds no session, unless
//! somebody wrote down why it must not.
//!
//! # Why this exists
//!
//! Authentication in `crates/api` is per handler, through a `BearerToken`
//! argument on the function signature. There is no middleware and no default.
//! So a route is **public by omission**: leaving one argument off a signature
//! publishes it, the router line looks exactly the same, and the handler reads
//! like every other handler in the file. There is nothing for a reviewer to
//! notice.
//!
//! That is not hypothetical. `GET /api/backtests/coverage` answered **HTTP 200
//! with the platform's entire data inventory** — every instrument, every
//! timeframe, bar counts, first and last timestamp held — to an anonymous
//! caller, and had done since it was written. One handler out of 268. It was
//! found by `curl`, not by review, which is the whole argument for this file:
//! the only reliable reader of a function signature is a program.
//!
//! # The allowlist is the useful half
//!
//! [`PUBLIC`] is the list of routes that answer a caller with no session, and
//! each row carries a [`Gate`] saying what stands there instead of the
//! extractor. The `Gate` is the point. "It looked harmless" is how `coverage`
//! shipped, so a row may not merely assert that a route is fine — it has to
//! name a mechanism, and the tests below go and check that the mechanism is
//! really in the handler. A row whose claim has stopped being true fails here.
//!
//! A row is also a tuple with no `Default`, so a row with no reason does not
//! compile, and [`every_public_row_says_why`] rejects one whose reason is a
//! shrug.
//!
//! # What it deliberately does not check
//!
//! Not *authorisation* — whether the token that got in may do the thing. That
//! is scope work, and `agent_authority.rs` and `monitor_authority.rs` hold it.
//! This file asks the one question underneath: does anybody have to be someone
//! at all.
//!
//! There used to be a third `Gate` here, `UnverifiedQueryToken`, recording that
//! the two WebSocket routes demanded a `?token=` they never checked — any
//! non-empty string was a user on the live data feed. It is gone because the
//! defect is: both now take `WsBearerToken`, which reads the token from the
//! query string and resolves it against the `sessions` table exactly as
//! `BearerToken` does from the header. Since the type's name contains
//! `BearerToken`, the scan below counts them as authenticated and they need no
//! row at all. `the_websocket_sockets_verify_their_token.rs` pins that.
//!
//! ```bash
//! cargo test -j 2 -p api --test every_mounted_route_refuses_an_anonymous_caller
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// What stands in the extractor's place on a route that takes no `BearerToken`.
///
/// Two kinds, and no third: a row that does not fit one of these is a route
/// nobody has justified, and the right outcome is the failure rather than a new
/// variant invented to accommodate it. There was a third, `UnverifiedQueryToken`,
/// and the way it ended is the point — a route was fixed, its rows left, and the
/// variant went with them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// There is no caller to identify yet. This route is how a caller obtains a
    /// session, or how one who has lost the ability to obtain a session gets it
    /// back. Requiring a token here would be requiring the thing the route
    /// exists to hand out.
    ///
    /// Constrained to the `/auth/` surface by
    /// [`only_the_auth_surface_may_establish_a_session`], so this cannot become
    /// the variant everything else is filed under.
    EstablishesTheSession,

    /// The handler authenticates in its own body instead of through the
    /// extractor, and names the function that does it. That function is checked
    /// to exist and to reject — see
    /// [`every_in_handler_gate_names_a_check_the_handler_really_makes`].
    InHandler(&'static str),
}

/// Routes that answer a caller holding no session, with the design that says so.
///
/// `(path, handler, gate, because)`. Keep sorted by path.
const PUBLIC: &[(&str, &str, Gate, &str)] = &[
    (
        "/auth/forgot-password",
        "handlers::forgot_password",
        Gate::EstablishesTheSession,
        "a caller who has lost their password cannot hold a session; this is the route that \
         starts giving one back, and gating it would gate recovery on the thing being recovered",
    ),
    (
        "/auth/login",
        "handlers::login",
        Gate::EstablishesTheSession,
        "the route that mints the token every other route demands; nothing precedes it",
    ),
    (
        "/auth/register",
        "handlers::register",
        Gate::EstablishesTheSession,
        "account creation is the first door; a platform whose first account needs an existing \
         account has no first account",
    ),
    (
        "/auth/reset-password",
        "handlers::reset_password",
        Gate::EstablishesTheSession,
        "the third step of recovery, and it carries its own proof: the emailed code is the \
         credential, checked in the handler against the same email",
    ),
    (
        "/auth/verify-reset-code",
        "handlers::verify_reset_code",
        Gate::EstablishesTheSession,
        "the second step of recovery; it proves possession of the emailed code and nothing else",
    ),
    (
        "/llm/v1/messages",
        "crate::llm_proxy::messages",
        Gate::InHandler("resolve_session"),
        "the Agent SDK inside a sandbox points ANTHROPIC_BASE_URL here and sends its session \
         token itself, so the handler resolves it against the sessions table and requires the \
         llm:proxy scope before any upstream call — authenticated, just not by the extractor",
    ),
];

// ── Reading the router ───────────────────────────────────────────────────────

/// One mounted route: the path, the handler expression the router names, and
/// whether that handler's signature takes a `BearerToken`.
#[derive(Debug)]
struct Mounted {
    path: String,
    handler: String,
    file: PathBuf,
    authenticated: bool,
}

fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
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

/// Where `fn <name>` is **declared**, skipping every place it is merely called.
///
/// The distinction is the whole reliability of this file. `me(token)` inside
/// another function is preceded by no `fn`, and `fn get_model` starts with
/// `fn get_mode`, so both a call site and a longer name have to be excluded
/// explicitly — a plain `find` gets both wrong.
fn definition_at(src: &str, name: &str) -> Option<usize> {
    let needle = format!("fn {name}");
    let mut from = 0;
    while let Some(i) = src[from..].find(&needle) {
        let at = from + i;
        let before = src[..at].chars().next_back().unwrap_or(' ');
        let after = src[at + needle.len()..].chars().next().unwrap_or(' ');
        if !(before.is_alphanumeric() || before == '_') && (after == '(' || after == '<') {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

/// The text between a function's parentheses, or `None` if it is not defined here.
///
/// The signature rather than the body, deliberately: a body that happens to
/// mention `BearerToken` in a nested call is not a handler that takes one.
fn signature<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = definition_at(src, name)?;
    let open = at + src[at..].find('(')?;
    let mut depth = 0usize;
    for (k, c) in src[open..].char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&src[open..open + k + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// A function's source from its `fn` keyword to the closing brace in column 0.
fn body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = definition_at(src, name)?;
    let rest = &src[at..];
    let end = rest.find("\n}\n").map_or(rest.len(), |e| e + 2);
    Some(&rest[..end])
}

/// Which file defines `module::name`, following the aliases `routes/mod.rs` uses.
///
/// The aliases are spelled out rather than parsed. If one is renamed, resolution
/// fails and [`the_scan_resolves_every_handler_the_router_names`] says so, which
/// is the loud direction — a resolver that guessed would quietly stop checking
/// a whole module.
fn file_for(src_root: &Path, module: &str) -> Option<PathBuf> {
    let alias = match module {
        "handlers" => Some(src_root.join("auth").join("handlers.rs")),
        "al" => Some(src_root.join("routes").join("asset_lifecycle.rs")),
        _ => None,
    };
    if let Some(p) = alias {
        return p.is_file().then_some(p);
    }
    if let Some(rest) = module.strip_prefix("crate::") {
        let mut p = src_root.to_path_buf();
        for seg in rest.split("::") {
            p.push(seg);
        }
        let file = p.with_extension("rs");
        if file.is_file() {
            return Some(file);
        }
        let m = p.join("mod.rs");
        return m.is_file().then_some(m);
    }
    let p = src_root.join("routes").join(format!("{module}.rs"));
    if p.is_file() {
        return Some(p);
    }
    let p = src_root.join(format!("{module}.rs"));
    p.is_file().then_some(p)
}

/// A bare handler name — one the router imported rather than qualified — found
/// by searching the crate for its single definition.
fn file_for_bare(src_root: &Path, name: &str) -> Option<PathBuf> {
    let mut hits: Vec<PathBuf> = Vec::new();
    for f in rust_files(src_root) {
        if signature(&read(&f), name).is_some() {
            hits.push(f);
        }
    }
    (hits.len() == 1).then(|| hits.remove(0))
}

const METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Every `.route("path", …)` in `routes/mod.rs`, resolved to its handlers.
///
/// Returns the resolved routes and the handler expressions it could not resolve.
fn mounted() -> (Vec<Mounted>, Vec<String>) {
    let src_root = crate_src();
    let router = read(&src_root.join("routes").join("mod.rs"));
    // A commented-out route is not mounted, and a path in prose is not a route.
    let router: String = router
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut routes = Vec::new();
    let mut unresolved = Vec::new();
    let mut from = 0;
    while let Some(i) = router[from..].find(".route(") {
        let open = from + i + ".route(".len();
        let mut depth = 1usize;
        let mut end = open;
        for (k, c) in router[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        let call = &router[open..end];
        from = end + 1;

        let Some(rest) = call.trim_start().strip_prefix('"') else {
            continue;
        };
        let Some(q) = rest.find('"') else { continue };
        let path = rest[..q].to_owned();
        let handlers_part = &rest[q + 1..];

        for method in METHODS {
            let m = format!("{method}(");
            let mut at = 0;
            while let Some(j) = handlers_part[at..].find(&m) {
                let start = at + j;
                let before = handlers_part[..start].chars().next_back().unwrap_or(' ');
                at = start + m.len();
                // `get(` inside `get_model` is not a method call.
                if before.is_alphanumeric() || before == '_' {
                    continue;
                }
                let tail = handlers_part[at..].trim_start();
                let ident: String = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':')
                    .collect();
                // A handler is a plain path expression; anything else — a
                // closure, a nested builder — is not something this scan can
                // read, and it must be reported rather than skipped.
                if ident.is_empty() || !tail[ident.len()..].trim_start().starts_with(')') {
                    unresolved.push(format!("{path} {method} <not a plain path expression>"));
                    continue;
                }

                let (module, name) = match ident.rsplit_once("::") {
                    Some((m, n)) => (m.to_owned(), n.to_owned()),
                    None => (String::new(), ident.clone()),
                };
                let file = if module.is_empty() {
                    file_for_bare(&src_root, &name)
                } else {
                    file_for(&src_root, &module)
                };
                let Some(file) = file else {
                    unresolved.push(format!("{path} -> {ident} (no file)"));
                    continue;
                };
                let text = read(&file);
                let Some(sig) = signature(&text, &name) else {
                    unresolved.push(format!("{path} -> {ident} (no fn in {})", file.display()));
                    continue;
                };
                let authenticated = sig.contains("BearerToken");
                routes.push(Mounted {
                    path: path.clone(),
                    handler: ident.clone(),
                    file,
                    authenticated,
                });
            }
        }
    }
    (routes, unresolved)
}

// ── The guard ────────────────────────────────────────────────────────────────

#[test]
fn every_mounted_route_refuses_an_anonymous_caller() {
    let (routes, _) = mounted();
    let declared: BTreeSet<(&str, &str)> = PUBLIC.iter().map(|(p, h, _, _)| (*p, *h)).collect();

    let open: Vec<String> = routes
        .iter()
        .filter(|r| !r.authenticated)
        .filter(|r| !declared.contains(&(r.path.as_str(), r.handler.as_str())))
        .map(|r| format!("{} -> {} ({})", r.path, r.handler, r.file.display()))
        .collect();

    assert!(
        open.is_empty(),
        "these mounted routes take no BearerToken, so they answer anybody who can reach the \
         port: {open:#?}\n\nThat is how GET /api/backtests/coverage served the platform's \
         whole data inventory to an anonymous caller. Either add the extractor argument, or \
         add a row to PUBLIC with a Gate that names what stands there instead — and \"it looks \
         harmless\" is not a Gate."
    );
}

/// The scan must resolve every handler the router names, or it is checking less
/// than it appears to.
///
/// A rename that breaks resolution would otherwise make the route vanish from
/// this test rather than fail it, which is exactly the silence the file exists
/// to remove.
#[test]
fn the_scan_resolves_every_handler_the_router_names() {
    let (routes, unresolved) = mounted();
    assert!(
        unresolved.is_empty(),
        "the router names handlers this scan cannot resolve, so nothing is checking them: \
         {unresolved:#?}"
    );
    assert!(
        routes.len() > 250,
        "the scan resolved only {} handlers, which is far below the router's size — it has \
         stopped matching and would pass whatever the router did",
        routes.len()
    );
    let authenticated = routes.iter().filter(|r| r.authenticated).count();
    assert_eq!(
        routes.len() - authenticated,
        PUBLIC.len(),
        "{} handlers take no BearerToken and PUBLIC has {} rows",
        routes.len() - authenticated,
        PUBLIC.len()
    );
}

/// A row that names a route nobody mounts describes nothing.
#[test]
fn every_public_row_names_a_route_that_is_mounted() {
    let (routes, _) = mounted();
    let live: BTreeSet<(&str, &str)> = routes
        .iter()
        .map(|r| (r.path.as_str(), r.handler.as_str()))
        .collect();
    let stale: Vec<&str> = PUBLIC
        .iter()
        .filter(|(p, h, _, _)| !live.contains(&(*p, *h)))
        .map(|(p, _, _, _)| *p)
        .collect();
    assert!(
        stale.is_empty(),
        "these are declared public and the router does not mount them under that handler: \
         {stale:?}. Remove the row — an allowlist entry for a route that no longer exists is \
         an entry nobody rereads."
    );
}

/// A row whose handler now takes the extractor should leave the list.
///
/// Without this, fixing a route leaves behind a permission for the next person
/// who edits that handler.
#[test]
fn the_public_list_holds_no_route_that_now_authenticates() {
    let (routes, _) = mounted();
    let fixed: Vec<&str> = PUBLIC
        .iter()
        .filter(|(p, h, _, _)| {
            routes
                .iter()
                .any(|r| r.path == *p && r.handler == *h && r.authenticated)
        })
        .map(|(p, _, _, _)| *p)
        .collect();
    assert!(
        fixed.is_empty(),
        "these are declared public and their handlers take a BearerToken: {fixed:?}. Delete \
         the rows — a stale exemption is a standing permission."
    );
}

/// Every row says why, in enough words to be an argument.
#[test]
fn every_public_row_says_why() {
    for (path, _, _, because) in PUBLIC {
        assert!(
            because.len() > 40,
            "`{path}` is exempt with the reason {because:?}, which is too short to be a reason. \
             An empty justification is a failure, not a default."
        );
        for shrug in [
            "harmless",
            "no data",
            "nothing sensitive",
            "it's fine",
            "public data",
        ] {
            assert!(
                !because.to_lowercase().contains(shrug),
                "`{path}` is exempt because it {shrug:?}. That is the argument that shipped \
                 /api/backtests/coverage. Say what stands in the extractor's place."
            );
        }
    }
}

/// `EstablishesTheSession` is only available to the credential surface.
///
/// The variant is the widest of the three — it claims nothing checks the caller
/// at all — so it is fenced to the routes that hand out or restore a session.
/// Any other route claiming it fails here.
#[test]
fn only_the_auth_surface_may_establish_a_session() {
    for (path, _, gate, _) in PUBLIC {
        if *gate == Gate::EstablishesTheSession {
            assert!(
                path.starts_with("/auth/"),
                "`{path}` claims nobody can hold a session yet, and it is not on the credential \
                 surface. Only the routes that mint or restore a session may say that."
            );
        }
    }
}

/// An `InHandler` row names a check, and the check is really in the handler.
///
/// This is the difference between an allowlist and a proof. The row claims the
/// handler authenticates itself; here that function is found in the same file,
/// called by the handler, and shown to reject with 401.
#[test]
fn every_in_handler_gate_names_a_check_the_handler_really_makes() {
    let (routes, _) = mounted();
    let mut checked = 0usize;
    for (path, handler, gate, _) in PUBLIC {
        let Gate::InHandler(check) = gate else {
            continue;
        };
        let route = routes
            .iter()
            .find(|r| r.path == *path && r.handler == *handler)
            .unwrap_or_else(|| panic!("`{path}` is not mounted"));
        let src = read(&route.file);
        let name = handler.rsplit_once("::").map_or(*handler, |(_, n)| n);
        let handler_body = body(&src, name).unwrap_or_else(|| panic!("no body for {handler}"));
        assert!(
            handler_body.contains(&format!("{check}(")),
            "`{path}` is exempt because `{check}` authenticates it, and the handler never calls \
             `{check}`"
        );
        let check_body = body(&src, check)
            .unwrap_or_else(|| panic!("`{path}` names `{check}` and no such function is here"));
        assert!(
            check_body.contains("UNAUTHORIZED"),
            "`{path}` is exempt because `{check}` authenticates it, and `{check}` never refuses \
             anybody"
        );
        checked += 1;
    }
    assert!(checked > 0, "the scan matched no InHandler row");
}

// ── The readers themselves ───────────────────────────────────────────────────
//
// Everything above rests on `signature` telling a definition from a call, and on
// the router scan telling a handler from an identifier that merely starts with a
// method name. Both have been wrong before in this repo's scanners, so both are
// tested against the cases that caught them.

#[test]
fn the_signature_reader_finds_definitions_and_not_call_sites() {
    let src = "\
fn helper(a: u8) {}
pub async fn me(token: BearerToken) -> impl IntoResponse { helper(1) }
fn not_me() { me(token) }
";
    assert_eq!(signature(src, "helper"), Some("(a: u8)"));
    assert_eq!(signature(src, "me"), Some("(token: BearerToken)"));
    // `me` is called inside `not_me`; the reader must not return that call's
    // parentheses, which hold no type at all.
    assert!(signature(src, "me").unwrap().contains("BearerToken"));
    assert_eq!(signature(src, "absent"), None);
}

#[test]
fn the_signature_reader_does_not_match_a_longer_name() {
    // `fn get_model` contains `fn get_mo`; a prefix match would resolve
    // `get_mo` to it and report the wrong signature as the right one.
    let src = "pub async fn get_model(_t: BearerToken) {}\n";
    assert!(signature(src, "get_mode").is_none());
    assert!(signature(src, "get_model").is_some());
}

#[test]
fn the_router_scan_reads_chained_and_qualified_handlers() {
    let (routes, unresolved) = mounted();
    assert!(unresolved.is_empty(), "{unresolved:#?}");
    let has = |path: &str, handler: &str| {
        routes
            .iter()
            .any(|r| r.path == path && r.handler == handler)
    };
    // A chained builder: both halves of `get(..).post(..)` are handlers.
    assert!(has("/api/skills", "web_and_skills::list_skills"));
    assert!(has("/api/skills", "web_and_skills::create_skill"));
    // A fully qualified method: `axum::routing::put(..)` is still a `put`.
    assert!(has("/api/settings", "portfolio::put_settings"));
    // A handler named through a `use` alias, and one the router imported bare.
    assert!(has("/assets/initialized", "al::list_initialized"));
    assert!(has("/ws/live", "ws_live"));
    // A handler whose own name begins with a method name.
    assert!(has("/api/models/for-node", "models::for_node"));
}
