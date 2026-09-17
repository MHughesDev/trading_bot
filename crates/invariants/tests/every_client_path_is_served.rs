//! A path the MCP client posts to is a path the platform serves.
//!
//! # The defect this generalises
//!
//! `gate::ApiTrialRegistrar` posts to `POST /api/ledger/trials`. That route does
//! not exist — the only occurrence of the string in the repository is the `post`
//! call itself. A 404 is an `Err`, which becomes `registration_failed`, which
//! `CallGate::admit` returns before the dispatch table is reached. **Every tool
//! declaring a trial is therefore refused**, on every path, and the entire
//! research loop cannot run (OQ-20).
//!
//! The gate's own tests all pass, and correctly: they assert that a registrar
//! which refuses causes the call to be refused, which is exactly what happens.
//! Nothing tested that the registrar could succeed, because nothing tested that
//! the route it calls is one the platform answers.
//!
//! This is the repository's dominant defect shape — a caller with no callee —
//! reached across an HTTP boundary, where the compiler cannot see it. That is
//! what makes a text-level check worth having: within a crate the type system
//! catches it, and between two crates that talk over a URL nothing does.
//!
//! # How it checks
//!
//! Literal paths only. A path built with `format!` carries a parameter, and
//! matching those against the router's `{param}` syntax means re-implementing
//! axum's matcher — which would be a second matcher, and the wrong one would be
//! this test. Literal paths are the ones where a typo or a missing route is
//! total, and `/api/ledger/trials` is one of them.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .to_path_buf()
}

fn read(rel: &str) -> String {
    let p = root().join(rel);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Every `.rs` under a directory.
fn sources(rel: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let base = root().join(rel);
    let mut stack = vec![base.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                if let Ok(src) = fs::read_to_string(&p) {
                    out.push((p.display().to_string(), src));
                }
            }
        }
    }
    out
}

/// Literal `/api/...` paths the client calls, excluding `format!` templates.
fn client_paths() -> BTreeSet<String> {
    // `.post("/api/x", ..)`, `.get("/api/x")`, `.delete("/api/x")`.
    let call = Regex::new(r#"\.(?:post|get|delete)\(\s*"(/api/[^"]*)""#).expect("regex");
    let mut out = BTreeSet::new();
    for (path, src) in sources("crates/mcp-server/src") {
        // This file names the broken path in prose; so does the gate's own
        // comment. Only the actual call sites count.
        if path.contains("every_client_path_is_served") {
            continue;
        }
        for c in call.captures_iter(&src) {
            let p = c[1].to_string();
            // Drop a query string: the router matches on the path.
            let p = p.split('?').next().unwrap_or(&p).to_string();
            out.insert(p);
        }
    }
    out
}

/// Every literal path the router registers.
fn served_paths() -> BTreeSet<String> {
    let route = Regex::new(r#"\.route\(\s*"([^"]+)""#).expect("regex");
    let mut out = BTreeSet::new();
    for (_, src) in sources("crates/api/src/routes") {
        for c in route.captures_iter(&src) {
            out.insert(c[1].to_string());
        }
    }
    // The router is assembled in `routes/mod.rs`; if it ever moves, this test
    // would silently find nothing and pass.
    assert!(
        out.len() > 50,
        "found only {} routes, so the scan is looking in the wrong place and would pass \
         whatever the client called",
        out.len()
    );
    out
}

/// Paths the client calls that the platform does not serve.
///
/// **Pinned, and the list may only shrink.** Each entry is a live defect with a
/// reason and a ticket, not an exemption — the alternative is a test that fails
/// today and gets deleted, which would take the finding with it.
const KNOWN_UNSERVED: &[(&str, &str)] = &[(
    "/api/ledger/trials",
    "OQ-20: the trial gate registers at the tool call and `mlops.trial` describes compute, so \
     the route cannot be written honestly without deciding what a trial is. Every \
     trial-spending tool is refused until it is.",
)];

#[test]
fn every_literal_client_path_is_a_route_the_platform_serves() {
    let served = served_paths();
    let known: BTreeSet<&str> = KNOWN_UNSERVED.iter().map(|(p, _)| *p).collect();

    let mut unserved = Vec::new();
    for path in client_paths() {
        if served.contains(&path) {
            continue;
        }
        if known.contains(path.as_str()) {
            continue;
        }
        unserved.push(path);
    }
    assert!(
        unserved.is_empty(),
        "the MCP client calls paths the platform does not serve, so every call through them \
         fails with a 404 the caller reports as its own failure: {unserved:#?}"
    );
}

/// A known-unserved path that has been fixed must leave the list.
///
/// Without this the pinned list would quietly become a list of paths that are
/// fine, and the next real one would be added to it by somebody assuming that is
/// what the list is for.
#[test]
fn the_known_unserved_list_holds_only_paths_that_are_still_unserved() {
    let served = served_paths();
    for (path, why) in KNOWN_UNSERVED {
        assert!(
            !served.contains(*path),
            "`{path}` is served now — remove it from KNOWN_UNSERVED. It was listed as: {why}"
        );
    }
}

/// The scan sees the call it was written for.
///
/// A regex that matched nothing would make both tests above pass on an empty
/// set, which is the failure mode of every text-level check.
#[test]
fn the_scan_finds_the_call_it_exists_for() {
    let paths = client_paths();
    assert!(
        paths.len() > 5,
        "found only {} client paths, so the scan is not reading the client",
        paths.len()
    );
    assert!(
        paths.contains("/api/ledger/trials"),
        "the scan no longer sees the trial registrar's call; either it was fixed — in which \
         case KNOWN_UNSERVED should have failed first — or the scan has stopped working"
    );
    // And the router scan sees a route that certainly exists.
    assert!(read("crates/api/src/routes/mod.rs").contains("/api/lifecycle/preregistrations"));
}
