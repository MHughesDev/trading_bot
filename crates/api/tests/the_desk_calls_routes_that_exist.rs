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

/// Calls whose last segment is **computed**, so this scan cannot match them to
/// a literal route however hard it tries.
///
/// `` `/api/automations/${id}/${armed ? 'disarm' : 'arm'}` `` normalises to
/// `/api/automations/{}/{}`, and the router spells the two routes out. Both are
/// served; nothing is missing. Listing them here rather than widening the
/// matcher keeps the failure mode obvious — a new row is a claim somebody has to
/// check, where a cleverer matcher would silently start accepting real gaps.
///
/// Each row names the routes it actually reaches, so the claim is falsifiable.
const COMPUTED: &[(&str, &str)] = &[(
    "/api/automations/{}/{}",
    "/api/automations/{id}/arm and /api/automations/{id}/disarm",
)];

/// Calls to routes that do not exist and that **no plan task owns**.
///
/// Different from [`UNBUILT`] in the way that matters: an unbuilt row is a
/// forward reference somebody scheduled, and one of these is a **defect** — a
/// surface calling something nobody is building. They were invisible until this
/// scan was widened past `frontend/src/api` on 2026-09-21.
///
/// A row here is a bug report, not a plan. It should leave by the route being
/// built or the call being removed, never by being re-filed as unbuilt.
const UNOWNED: &[(&str, &str)] = &[
    (
        "/api/automations/{}/stages",
        "frontend/src/components/automations/StageBoard.tsx GETs it; the router serves          /api/automations/{id}, /arm and /disarm, and no /stages",
    ),
    (
        "/api/strategies/{}",
        "frontend/src/lib/api.ts PUTs and DELETEs it; the router serves          /api/strategies/{id}/config, /start and /stop, and no bare /api/strategies/{id}",
    ),
];

/// Paths the frontend calls on purpose that nothing serves yet, with the task
/// that owns each. Keep sorted; keep the reason.
const UNBUILT: &[(&str, &str)] = &[
    // Phase 8 — a run's report. `/api/reports/{}` was found by this test and by
    // nothing else: the shell diff that produced the other twelve only read
    // `client.get(` calls in `frontend/src/api/*.ts`, and this one lives in
    // `research.ts` behind a different spelling. Thirteen, not twelve — which is
    // the argument for the test over the diff, made one more time.
    (
        "/api/reports/{}",
        "a run's report, which is not the compiled dossier S-8.7 built and serves",
    ),
    // The two dossier paths that were here are served as of S-8.7, and the
    // comment thread was added on both sides at once so it never needed an
    // entry. What is left of phase 8 on this list is `/api/reports/{}`, which
    // is a different surface: a run's report rather than a compiled dossier.
    // The four phase-11 rows that were here are served as of S-11.1 and S-11.14:
    // `crates/research-session` exists, `0138` carries the four tables, and
    // `JobKind::ContinuousResearch` has a registered worker. They were the
    // oldest rows on this list — the desk's composer has posted a charter to
    // `/api/research/sessions` since it was written — which is the case this
    // test was added for.
    // S-6.11's `/api/agent/runs/{}/pins` left this list on 2026-10-04: the route
    // is mounted (`agent::routes::pin_run`), the driver feeds `Input::Pin` and the
    // router reads it first.
    // Phase 10 — lifecycle. `crates/harness/src/lifecycle/` does not exist.
    (
        "/api/agent/runs/{}/lifecycle",
        "S-10.x — the lifecycle module is not built",
    ),
    // Phase 5 — the strategy editor's compile and preview. Found 2026-09-21 when
    // this scan was widened past `frontend/src/api`: `Operations.tsx` posts to
    // both directly, and they had been invisible here for as long as this test
    // existed. The component renders honestly when they fail, so nothing is
    // currently lying to an operator — but the desk is calling a compiler that
    // does not answer, and `crates/strategy-lang` now holds the compiler it
    // would reach.
    (
        "/api/strategies/compile",
        "S-5.8 — the authoring tools have no HTTP surface; strategy-lang::text_to_definition exists",
    ),
    (
        "/api/strategies/preview",
        "S-5.4 — preview_signals is an MCP tool; the desk's editor has no route to it",
    ),
    // The desk's run controls. Two of the three left this list on 2026-09-22:
    // `…/pause` and `…/resume` are served, bounded by ADR-D-27 — every pause
    // carries a deadline, the parked driver task enforces it, and on expiry the
    // run stops rather than resuming.
    //
    // `…/escalate` stays, and stays *struck* rather than scheduled: a mid-run
    // tier change carries its own budget consequences, has no caller and no
    // demand, and the way to change a run's tier is to stop it and start one.
    // The row is here because `frontend/src/components/desk/ControlsRow.tsx`
    // still names the path in the prose explaining why its button was deleted,
    // which this scan counts — deliberately, in the safe direction.
    ("/api/agent/runs/{}/escalate", "S-6.x — run controls unbuilt; escalate is struck, not scheduled"),
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
            // A query string is not a different route. `/api/approvals?state=all`
            // and `/api/approvals` are one endpoint, and treating them as two
            // would make every filtered read look unserved.
            '?' => break,
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// Every `/api/...` the frontend names, from **anywhere** under `frontend/src`.
///
/// This read `frontend/src/api` alone until 2026-09-21, on the reasoning that
/// every call goes through the client layer. Most do. Two did not:
/// `Operations.tsx`'s strategy editor posts to `/api/strategies/compile` and
/// `/api/strategies/preview` directly, neither of which the router serves — and
/// they were invisible here for as long as this test existed, while the phase
/// documentation claimed the test "fails the build for it".
///
/// The cost of widening it is that a path named in a component's *prose* counts
/// as a call. That is the same over-counting the backend side already accepts,
/// and in the same safe direction: it can produce a row somebody has to declare,
/// never miss one that should have been.
fn frontend_paths(root: &Path) -> BTreeSet<String> {
    let dir = root.join("frontend/src");
    let mut found = api_paths_in(&dir, "ts");
    found.extend(api_paths_in(&dir, "tsx"));
    found
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
                    // A **call**, not a mention. Widening the scan past
                    // `frontend/src/api` brought in the desk's own empty-state
                    // copy, which by design begins with the endpoint it is about
                    // — "`/api/catalogue` is served, so this is the request
                    // failing rather than the endpoint missing". Those are
                    // sentences, and a sentence is not a route.
                    //
                    // A path literal has no whitespace in it. That is the whole
                    // rule, and it is one a reader can check. The entity decode
                    // is for the same widening: JSX escapes a brace it prints,
                    // and `&#123;bucket&#125;` is `{bucket}` written for a human.
                    //
                    // **Normalise first, then test.** A template's interpolation
                    // can hold spaces of its own —
                    // `` `/api/automations/${id}/${armed ? 'disarm' : 'arm'}` `` —
                    // and testing the raw literal would drop a real call for
                    // looking like prose. `normalise` collapses every
                    // interpolation to `{}` before anything is judged.
                    let piece = piece.replace("&#123;", "{").replace("&#125;", "}");
                    if !piece.starts_with("/api/") {
                        continue;
                    }
                    let path = normalise(&piece);
                    if !path.contains(char::is_whitespace) {
                        found.insert(path);
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
    if !root.join("frontend/src").is_dir() {
        // The backend is testable without the frontend checked out.
        return;
    }

    let called = frontend_paths(&root);
    assert!(
        called.len() > 20,
        "the scan found {} endpoints in frontend/src/api, which means it is not reading them \
         and would pass whatever the router did",
        called.len()
    );

    let served = api_paths_in(&root.join("crates/api/src"), "rs");
    let declared: BTreeSet<&str> = UNBUILT
        .iter()
        .chain(UNOWNED)
        .chain(COMPUTED)
        .map(|(p, _)| *p)
        .collect();

    let missing: Vec<&String> = called
        .iter()
        .filter(|p| !served.contains(*p) && !declared.contains(p.as_str()))
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

/// A computed row names the routes it claims to reach, and they are served.
///
/// Without this the list is a way to silence the scan by asserting a match
/// nobody checked.
#[test]
fn every_computed_row_reaches_routes_that_are_served() {
    let served = api_paths_in(&workspace_root().join("crates/api/src"), "rs");
    for (path, reaches) in COMPUTED {
        assert!(
            reaches.contains("/api/"),
            "`{path}` is listed as computed and names no route it reaches"
        );
        let named: Vec<&str> = reaches
            .split_whitespace()
            .filter(|w| w.starts_with("/api/"))
            .collect();
        assert!(!named.is_empty(), "`{path}` names no concrete route");
        for route in named {
            let n = normalise(route);
            assert!(
                served.contains(&n),
                "`{path}` claims to reach `{route}`, and nothing serves it — so the row is \
                 hiding a gap rather than explaining one"
            );
        }
    }
}

/// An unowned row names the file that makes the call.
///
/// It is a bug report, and a bug report with no reproduction is a row nobody
/// acts on.
#[test]
fn every_unowned_row_names_the_caller() {
    for (path, reason) in UNOWNED {
        assert!(
            reason.contains("frontend/src"),
            "`{path}` is listed as unowned and does not say which file calls it"
        );
        assert!(
            !reason.contains("S-"),
            "`{path}` names a task, so it belongs in UNBUILT rather than here — unowned means \
             nobody is building it"
        );
    }
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
    assert_eq!(
        normalise("/api/agent/runs/${runId}/state"),
        "/api/agent/runs/{}/state"
    );
    assert_eq!(
        normalise("/api/agent/runs/{id}/state"),
        "/api/agent/runs/{}/state"
    );
    assert_eq!(
        normalise("/api/agent/runs/:id/state"),
        "/api/agent/runs/{}/state"
    );
    assert_eq!(normalise("/api/catalogue"), "/api/catalogue");
    // Two parameters, and a trailing one.
    assert_eq!(
        normalise("/api/dossiers/${t}/verify/${c}"),
        "/api/dossiers/{}/verify/{}"
    );
    assert_eq!(normalise("/api/memory/{bucket}/{id}"), "/api/memory/{}/{}");
}
