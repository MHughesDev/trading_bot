//! Every endpoint the frontend calls is one this crate serves.
//!
//! # Why this exists
//!
//! Three times in one day, a desk page was found calling an endpoint nobody
//! served. `Catalogue.tsx` had been calling `/api/catalogue` since it was
//! written; `AwayDigest.tsx` the same; `MemoryBrowser.tsx` rendered an empty
//! state that said so in as many words. Each rendered as an empty panel, which
//! is indistinguishable from a panel with nothing in it — the failure looks
//! exactly like the success.
//!
//! An audit found three. A systematic diff of `frontend/src/api/*.ts` against
//! the router then found **twelve**, four of which the audit had missed. That is
//! the argument for a test rather than a better audit: a list somebody maintains
//! by looking is a list that is wrong by the amount nobody looked.
//!
//! # The allowlist is the useful half
//!
//! An endpoint that is deliberately not built yet belongs in [`UNBUILT`], beside
//! the phase that will build it. That makes the frontend's forward references a
//! reviewable list rather than a discovery, and it means adding a call to
//! something unbuilt is a decision somebody writes down.
//!
//! Removing a row is how a phase closes. Adding one without a reason is how this
//! test stops meaning anything, so each carries the task that owns it.
//!
//! # What it deliberately does not check, and where it is weak
//!
//! Not the HTTP method, and not the response shape. Both need the router's own
//! type information, which a source scan does not have — this catches the
//! failure that actually happened three times, which is a path nothing answers.
//!
//! And one weakness worth stating rather than leaving to be discovered: the
//! backend side counts **any** `/api/…` string literal in `crates/api/src`,
//! including one inside a doc comment. So a path mentioned in prose and served
//! by nothing would read as served, and this test would miss it.
//!
//! That is the safe direction — it can fail to catch a gap, never invent one —
//! and tightening it to `.route(` alone would be worse, because route
//! declarations wrap across lines and a line-oriented scan missed four of the
//! twelve when it was tried. A mention-in-prose is the rarer mistake; a
//! multi-line `.route(` is every long path in the router.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Paths the frontend calls on purpose that nothing serves yet, with the task
/// that owns each. Keep sorted; keep the reason.
const UNBUILT: &[(&str, &str)] = &[
    // Phase 8 — the report layer (S-8.6, S-8.7). `crates/harness/src/report/`
    // does not exist. `/api/reports/{}` was found by this test and by nothing
    // else: the shell diff that produced the other twelve only read
    // `client.get(` calls in `frontend/src/api/*.ts`, and this one lives in
    // `research.ts` behind a different spelling. Thirteen, not twelve — which is
    // the argument for the test over the diff, made one more time.
    (
        "/api/reports/{}",
        "S-8.6 — the report layer is not built",
    ),
    // Phase 8 — the dossier (S-8.7). `crates/harness/src/report/` does not exist.
    ("/api/dossiers/{}", "S-8.7 — the dossier layer is not built"),
    (
        "/api/dossiers/{}/verify/{}",
        "S-8.7 — claim rechecking needs the dossier",
    ),
    // Phase 11 — continuous research (S-11.1). Needs `crates/research-session`,
    // four tables and a JobKind; depends on phases 10, 2, 4 and 7.
    ("/api/research/sessions", "S-11.1 — no research_sessions table"),
    (
        "/api/research/sessions/{}/agenda",
        "S-11.1 — no agenda_items table",
    ),
    (
        "/api/research/sessions/{}/agenda/order",
        "S-11.1 — no agenda_items table",
    ),
    (
        "/api/research/sessions/{}/agenda/{}/veto",
        "S-11.1 — no agenda_items table",
    ),
    // Phase 10 — lifecycle. `crates/harness/src/lifecycle/` does not exist.
    (
        "/api/agent/runs/{}/lifecycle",
        "S-10.x — the lifecycle module is not built",
    ),
    // The desk's run controls. `desk.ts` already marks these unbuilt in its own
    // comment ("Pause/resume/escalate are ✗"); this is the same statement where
    // a test can see it.
    ("/api/agent/runs/{}/pause", "S-6.x — run controls unbuilt"),
    ("/api/agent/runs/{}/resume", "S-6.x — run controls unbuilt"),
    ("/api/agent/runs/{}/escalate", "S-6.x — run controls unbuilt"),
    // Phase 6's State panel and Replay read these. The panel renders from the
    // stream today; the by-step fetch and the trajectory do not exist.
    (
        "/api/agent/runs/{}/state",
        "S-6.x — the State panel's by-step fetch (AGENT-012 §4)",
    ),
    (
        "/api/agent/runs/{}/trajectory",
        "S-6.x — Replay (AGENT-012 §4)",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/<name> sits two below the root")
        .to_path_buf()
}

/// `${...}` and `:name` and `{name}` all become `{}`, so a path matches however
/// its parameter is spelled on each side.
fn normalise(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let bytes: Vec<char> = p.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            '$' if bytes.get(i + 1) == Some(&'{') => {
                out.push_str("{}");
                while i < bytes.len() && bytes[i] != '}' {
                    i += 1;
                }
                i += 1;
            }
            '{' => {
                out.push_str("{}");
                while i < bytes.len() && bytes[i] != '}' {
                    i += 1;
                }
                i += 1;
            }
            ':' => {
                out.push_str("{}");
                i += 1;
                while i < bytes.len() && (bytes[i].is_alphanumeric() || bytes[i] == '_') {
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Every `/api/...` string literal in a source tree, normalised.
fn api_paths_in(dir: &Path, extension: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some(extension) {
                continue;
            }
            let src = std::fs::read_to_string(&p).unwrap_or_default();
            for quote in ['"', '\'', '`'] {
                for piece in src.split(quote).skip(1).step_by(2) {
                    if piece.starts_with("/api/") && !piece.contains('\n') {
                        found.insert(normalise(piece));
                    }
                }
            }
        }
    }
    found
}

#[test]
fn every_endpoint_the_frontend_calls_is_served_or_declared_unbuilt() {
    let root = workspace_root();
    let frontend = root.join("frontend/src/api");
    if !frontend.is_dir() {
        // The backend is testable without the frontend checked out.
        return;
    }

    let called = api_paths_in(&frontend, "ts");
    assert!(
        called.len() > 20,
        "the scan found {} endpoints in frontend/src/api, which means it is not reading them \
         and would pass whatever the router did",
        called.len()
    );

    let served = api_paths_in(&root.join("crates/api/src"), "rs");
    let unbuilt: BTreeSet<&str> = UNBUILT.iter().map(|(p, _)| *p).collect();

    let missing: Vec<&String> = called
        .iter()
        .filter(|p| !served.contains(*p) && !unbuilt.contains(p.as_str()))
        .collect();

    assert!(
        missing.is_empty(),
        "the desk calls these and nothing serves them: {missing:#?}\n\nAn unserved endpoint \
         renders as an empty panel, which is indistinguishable from a panel with nothing in \
         it — so this fails here rather than looking like a platform with no data. Either \
         build the route, or add the path to `UNBUILT` with the task that owns it."
    );
}

/// A row in `UNBUILT` that is now served should be removed, or the list stops
/// describing anything.
#[test]
fn the_unbuilt_list_has_no_rows_that_are_already_built() {
    let root = workspace_root();
    let served = api_paths_in(&root.join("crates/api/src"), "rs");
    let stale: Vec<&str> = UNBUILT
        .iter()
        .map(|(p, _)| *p)
        .filter(|p| served.contains(*p))
        .collect();
    assert!(
        stale.is_empty(),
        "these are listed as unbuilt and are served: {stale:?}. Remove them — a forward \
         reference that has landed is how the list stops being reviewable."
    );
}

/// Every row says which task owns it.
#[test]
fn every_unbuilt_row_names_the_task_that_will_build_it() {
    for (path, reason) in UNBUILT {
        assert!(
            reason.contains("S-") && reason.len() > 12,
            "`{path}` is allowlisted with the reason {reason:?}, which does not name a task. A \
             row with no owner is a row nobody removes."
        );
    }
}

#[test]
fn the_normaliser_treats_every_parameter_spelling_alike() {
    assert_eq!(normalise("/api/agent/runs/${runId}/state"), "/api/agent/runs/{}/state");
    assert_eq!(normalise("/api/agent/runs/{id}/state"), "/api/agent/runs/{}/state");
    assert_eq!(normalise("/api/agent/runs/:id/state"), "/api/agent/runs/{}/state");
    assert_eq!(normalise("/api/catalogue"), "/api/catalogue");
    // Two parameters, and a trailing one.
    assert_eq!(normalise("/api/dossiers/${t}/verify/${c}"), "/api/dossiers/{}/verify/{}");
    assert_eq!(normalise("/api/memory/{bucket}/{id}"), "/api/memory/{}/{}");
}
