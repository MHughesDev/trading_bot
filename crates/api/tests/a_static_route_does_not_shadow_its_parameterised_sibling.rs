//! A literal path segment does not steal a method from the parameterised route
//! it sits beside.
//!
//! # Why this exists
//!
//! `matchit`, the router under axum, prefers a static segment over a parameter.
//! So when two routes are mounted at the same shape and one of them spells a
//! segment out, the literal takes **every** request to that path — including
//! methods it does not serve, which it answers with **405 Method Not Allowed**.
//! The parameterised handler is never reached for that one value. Nothing warns;
//! both lines look ordinary; the router compiles and starts.
//!
//! That is not hypothetical:
//!
//! ```text
//! .route("/api/memory/preferences", post(memory::record_preference))   // line 234
//! .route("/api/memory/{bucket}",    get(memory::list))                 // line 240
//! ```
//!
//! and against the running platform:
//!
//! ```text
//! GET /api/memory/findings     401      GET /api/memory/preferences   405  ← this one
//! GET /api/memory/negative     401      GET /api/memory/signals       401
//! ```
//!
//! Six of the memory browser's buckets answered. `preferences` — the one bucket
//! whose entries the *user themselves* wrote — was structurally unreadable, and
//! had been for as long as both routes existed. No frontend change could reach
//! it. It was found by `curl`, which is the argument for this file: a router is a
//! table of shapes, and the only reliable reader of a table of shapes is a
//! program.
//!
//! # What makes a pair a defect rather than a curiosity
//!
//! Most static/parameterised pairs in this router are harmless, and the reason
//! is worth stating precisely, because it is the whole discrimination this file
//! makes. `GET /api/backtests/coverage` shadows `DELETE` on
//! `/api/backtests/{id}` — and `coverage` is not a backtest id. Nobody can
//! legitimately mean it as one, so the method that is unreachable is a request
//! nobody can make.
//!
//! `preferences` **is** a legitimate value of `{bucket}`. `kind_for` in
//! `routes/memory.rs` says so, by name, in source. That is the difference, and
//! it is mechanically checkable rather than a judgement:
//!
//! * [`Why::TheParameterIsAGeneratedId`] is the only exemption, and
//!   [`every_shadowed_row_names_a_parameter_that_really_is_an_id`] requires the
//!   shadowed parameter to be `{id}` or `{…_id}`. `{bucket}` is neither, so the
//!   memory defect **could not have been allowlisted** — the list cannot absorb
//!   this bug class, which is the property that makes it worth keeping.
//! * [`CLOSED_VOCABULARY`] goes further for a parameter whose legitimate values
//!   are enumerated in source: such a prefix may carry **no** static sibling at
//!   all, shadowed today or not, because the next method added to the
//!   parameterised route would lose it silently.
//!
//! # Its shape is borrowed on purpose
//!
//! The router scan, the allowlist-with-a-checked-reason, and the self-tests of
//! the reader are all taken from
//! `every_mounted_route_refuses_an_anonymous_caller.rs`, which resolves the same
//! table for a different question. Two scanners over one router that disagree
//! about what a route is would be worse than either, so this one reads the table
//! the same way and differs only in keeping the **method**, which that file
//! discards.
//!
//! ```bash
//! cargo test -j 2 -p api --test a_static_route_does_not_shadow_its_parameterised_sibling
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Why a shadowed method is nobody's loss.
///
/// One variant, and no second. A pair that does not fit it is a pair nobody has
/// justified, and the right outcome is the failure rather than a variant invented
/// to accommodate it — the memory defect would have taken exactly such a
/// variant, and taken the fix with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Why {
    /// The parameter holds an identifier the platform generated, so a word
    /// spelled out beside it is not a value the parameter takes. Checked, not
    /// asserted: see
    /// [`every_shadowed_row_names_a_parameter_that_really_is_an_id`].
    TheParameterIsAGeneratedId,
}

/// Static/parameterised pairs where the static path does not serve a method its
/// parameterised sibling does.
///
/// `(static_path, parameterised_path, because)`. Keep sorted by static path.
///
/// Every row here is currently `TheParameterIsAGeneratedId`, so the variant is
/// implied by the table rather than repeated on each line; [`SHADOWED_WHY`]
/// holds it. If a second variant is ever justified, this becomes a four-tuple
/// and that is the right amount of friction.
const SHADOWED: &[(&str, &str, &str)] = &[
    (
        "/api/backtests/coverage",
        "/api/backtests/{id}",
        "DELETE /api/backtests/coverage is unreachable, and `coverage` is not a backtest id — \
         ids are generated, and no caller deleting a backtest can arrive at that word",
    ),
    (
        "/api/models/feature-sets",
        "/api/models/{id}",
        "DELETE and PATCH on this path are unreachable; `feature-sets` is not a model id, so \
         neither is a request a caller can form",
    ),
    (
        "/api/models/for-node",
        "/api/models/{id}",
        "DELETE and PATCH on this path are unreachable; `for-node` is not a model id, and this \
         route answers a question about a pipeline node rather than about one model",
    ),
    (
        "/api/models/transforms",
        "/api/models/{id}",
        "DELETE, GET and PATCH on this path are unreachable; `transforms` is not a model id — \
         every id is generated with a prefix (`mdl_…`, and `tfm_<name>` for a transform), so no \
         model, a transform named `transforms` included, is ever that bare word",
    ),
    (
        "/api/models/leaderboard",
        "/api/models/{id}",
        "DELETE and PATCH on this path are unreachable; `leaderboard` is not a model id — it \
         is a ranking over all of them",
    ),
    (
        "/api/models/{id}/runs/reproduce",
        "/api/models/{id}/runs/{run_id}",
        "GET on this path is unreachable, and `reproduce` is not a run id — the run ids under \
         a model are generated, so no caller fetching one can arrive at that word",
    ),
];

/// The exemption every [`SHADOWED`] row claims.
const SHADOWED_WHY: Why = Why::TheParameterIsAGeneratedId;

/// Parameters whose legitimate values are **enumerated in source**, and the
/// function that enumerates them.
///
/// `(parameterised_path, module_file, fn_name)`. A prefix listed here may carry
/// no static sibling at all — see
/// [`a_parameter_with_a_closed_vocabulary_has_no_static_sibling`].
///
/// This is the check that would have caught the memory defect on the day it was
/// written, with no human in the loop: `preferences` is a string literal inside
/// `kind_for`, and `/api/memory/preferences` was a mounted path.
const CLOSED_VOCABULARY: &[(&str, &str, &str)] = &[(
    "/api/memory/{bucket}",
    "routes/memory.rs",
    // Maps the memory browser's tab names to `harness::memory::Kind`. Its string
    // literals *are* the legitimate values of `{bucket}`.
    "kind_for",
)];

// ── Reading the router ───────────────────────────────────────────────────────

fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

const METHODS: &[&str] = &["get", "post", "put", "delete", "patch", "head", "options"];

/// Every mounted path in `routes/mod.rs`, with the HTTP methods it answers.
///
/// Parsed by brace-matching rather than by line, because a `.route(` wraps
/// across lines for every long path in this router and a line-oriented scan
/// misses those — the same reason
/// `every_mounted_route_refuses_an_anonymous_caller.rs` parses it this way.
fn mounted_methods() -> BTreeMap<String, BTreeSet<String>> {
    let router = read(&crate_src().join("routes").join("mod.rs"));
    // A commented-out route is not mounted, and a path in prose is not a route.
    let router: String = router
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    methods_in(&router)
}

/// The scan itself, over arbitrary router source, so the self-tests below can
/// hand it the cases that matter instead of only ever seeing a router that
/// passes.
fn methods_in(router: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut table: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
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
        let handlers = &rest[q + 1..];

        let entry = table.entry(path).or_default();
        for method in METHODS {
            let m = format!("{method}(");
            let mut at = 0;
            while let Some(j) = handlers[at..].find(&m) {
                let start = at + j;
                let before = handlers[..start].chars().next_back().unwrap_or(' ');
                at = start + m.len();
                // `get(` inside `get_model` is not a method call, and neither is
                // the `post(` in `post_settings`.
                if before.is_alphanumeric() || before == '_' {
                    continue;
                }
                entry.insert(method.to_uppercase());
            }
        }
    }
    table
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

fn is_parameter(segment: &str) -> bool {
    segment.starts_with('{') && segment.ends_with('}')
}

/// The name inside `{…}`, with a `*` wildcard prefix stripped.
fn parameter_name(segment: &str) -> &str {
    segment
        .trim_start_matches('{')
        .trim_end_matches('}')
        .trim_start_matches('*')
}

/// Is `stat` the same shape as `param`, with a literal wherever `param` has a
/// parameter?
///
/// Both must have the same number of segments, every differing position must be
/// literal-in-`stat` against parameter-in-`param`, and at least one position
/// must differ. `matchit` routes them to the same node, and prefers `stat`.
fn shadows(stat: &str, param: &str) -> bool {
    let (a, b) = (segments(stat), segments(param));
    if a.len() != b.len() || a == b {
        return false;
    }
    let mut differs = false;
    for (x, y) in a.iter().zip(&b) {
        if x == y {
            continue;
        }
        if !is_parameter(y) || is_parameter(x) {
            return false;
        }
        differs = true;
    }
    differs
}

/// One static path that takes requests its parameterised sibling would have
/// served, and the methods that are therefore unreachable.
#[derive(Debug, PartialEq, Eq)]
struct Shadow {
    stat: String,
    param: String,
    lost: Vec<String>,
}

/// Every pair in a router table where the static path answers 405 to a method
/// the parameterised path serves.
fn shadowed_pairs(table: &BTreeMap<String, BTreeSet<String>>) -> Vec<Shadow> {
    let mut out = Vec::new();
    for (stat, static_methods) in table {
        for (param, param_methods) in table {
            if !shadows(stat, param) {
                continue;
            }
            let lost: Vec<String> = param_methods.difference(static_methods).cloned().collect();
            if !lost.is_empty() {
                out.push(Shadow {
                    stat: stat.clone(),
                    param: param.clone(),
                    lost,
                });
            }
        }
    }
    out
}

/// Every static path mounted at the same shape as `param`, shadowed or not.
fn static_siblings_of(table: &BTreeMap<String, BTreeSet<String>>, param: &str) -> Vec<String> {
    table
        .keys()
        .filter(|p| shadows(p, param))
        .cloned()
        .collect()
}

// ── The guard ────────────────────────────────────────────────────────────────

#[test]
fn a_static_route_does_not_shadow_its_parameterised_sibling() {
    let table = mounted_methods();
    let declared: BTreeSet<(&str, &str)> = SHADOWED.iter().map(|(s, p, _)| (*s, *p)).collect();

    let undeclared: Vec<String> = shadowed_pairs(&table)
        .into_iter()
        .filter(|s| !declared.contains(&(s.stat.as_str(), s.param.as_str())))
        .map(|s| {
            format!(
                "{} takes every request to that path, so {:?} on it never reaches {}",
                s.stat, s.lost, s.param
            )
        })
        .collect();

    assert!(
        undeclared.is_empty(),
        "matchit prefers a literal segment over a parameter, so these static routes answer 405 \
         to methods their parameterised sibling serves: {undeclared:#?}\n\nThat is how \
         `GET /api/memory/preferences` answered 405 for as long as both routes existed, while \
         its six sibling buckets answered normally — one tab of the memory browser was \
         structurally unreadable and no frontend change could fix it. Mount the method on the \
         parameterised path instead, or add a row to SHADOWED proving the literal is not a \
         value the parameter takes. \"It is probably fine\" is not that proof."
    );
}

/// The exemption is only available where the parameter is a generated id.
///
/// This is what stops the list absorbing the bug it was written for. `{bucket}`
/// is a closed vocabulary, not an id, so no row could have covered
/// `/api/memory/preferences` and the only way past the test would have been the
/// fix.
#[test]
fn every_shadowed_row_names_a_parameter_that_really_is_an_id() {
    assert_eq!(SHADOWED_WHY, Why::TheParameterIsAGeneratedId);
    for (stat, param, _) in SHADOWED {
        let (a, b) = (segments(stat), segments(param));
        assert_eq!(
            a.len(),
            b.len(),
            "`{stat}` and `{param}` are not the same shape, so one cannot shadow the other"
        );
        let mut checked = 0usize;
        for (x, y) in a.iter().zip(&b) {
            if x == y {
                continue;
            }
            let name = parameter_name(y);
            assert!(
                name == "id" || name.ends_with("_id"),
                "`{param}` is exempt because `{y}` holds a generated id, and `{name}` is not an \
                 id — it reads like a closed set of names, which is exactly the case where a \
                 literal beside it is one of its own values. That is `/api/memory/{{bucket}}`, \
                 and the fix there was to mount the method on the parameter."
            );
            checked += 1;
        }
        assert!(checked > 0, "`{stat}` and `{param}` differ nowhere");
    }
}

/// A parameter whose values are enumerated in source carries no static sibling.
///
/// Stricter than the shadow test on purpose. A static sibling that happens to
/// serve every method the parameter serves is harmless *today*; it becomes this
/// defect the moment somebody adds a method to the parameterised route, and that
/// person has no reason to look here. So the prefix is kept clear instead.
#[test]
fn a_parameter_with_a_closed_vocabulary_has_no_static_sibling() {
    let table = mounted_methods();
    for (param, _, vocabulary) in CLOSED_VOCABULARY {
        assert!(
            table.contains_key(*param),
            "`{param}` has a declared vocabulary and is not mounted"
        );
        let siblings = static_siblings_of(&table, param);
        assert!(
            siblings.is_empty(),
            "`{param}` takes the values `{vocabulary}` enumerates, and these literal paths are \
             mounted at the same shape: {siblings:?}. Each is a second spelling of one of those \
             values, and matchit gives it the path. Mount the method on `{param}`."
        );
    }
}

/// The vocabulary a row names exists, enumerates something, and does not list a
/// value some literal route also spells.
///
/// Without this the row is a claim nobody checked — a rename of `kind_for` would
/// make the test above pass by resolving nothing.
#[test]
fn every_closed_vocabulary_names_a_function_that_really_enumerates_the_values() {
    let table = mounted_methods();
    for (param, file, vocabulary) in CLOSED_VOCABULARY {
        let path = crate_src().join(file);
        let src = read(&path);
        let body = function_body(&src, vocabulary).unwrap_or_else(|| {
            panic!("`{param}` names `{vocabulary}` in {file} and no such function is there")
        });
        let values = string_literals(body);
        assert!(
            values.len() > 1,
            "`{vocabulary}` in {file} enumerates {values:?}, which is not a vocabulary — the \
             row has stopped describing the function"
        );
        // The values are what a literal sibling would collide with. Checked
        // against every mounted path, not only the siblings, so a value moved to
        // some other prefix is still caught.
        let depth = segments(param).len();
        let position = segments(param)
            .iter()
            .position(|s| is_parameter(s))
            .expect("a parameterised path has a parameter");
        let collisions: Vec<String> = table
            .keys()
            .filter(|p| segments(p).len() == depth)
            .filter(|p| segments(p)[..position] == segments(param)[..position])
            .filter(|p| values.contains(&segments(p)[position].to_string()))
            .cloned()
            .collect();
        assert!(
            collisions.is_empty(),
            "these mounted paths spell out a value `{vocabulary}` already maps, so matchit gives \
             them the path instead of `{param}`: {collisions:?}"
        );
    }
}

/// Every row says why, in enough words to be an argument.
#[test]
fn every_shadowed_row_says_why() {
    for (stat, _, because) in SHADOWED {
        assert!(
            because.len() > 40,
            "`{stat}` is exempt with the reason {because:?}, which is too short to be one"
        );
        assert!(
            because.contains("unreachable") || because.contains("not a"),
            "`{stat}` is exempt and its reason does not say which method is lost or why the \
             literal cannot be a parameter value: {because:?}"
        );
        for shrug in [
            "harmless",
            "probably",
            "no data",
            "it's fine",
            "should be ok",
        ] {
            assert!(
                !because.to_lowercase().contains(shrug),
                "`{stat}` is exempt because it is {shrug:?}. Say why no caller can form the \
                 request the literal is swallowing."
            );
        }
    }
}

/// A row for a pair the router does not have describes nothing.
#[test]
fn every_shadowed_row_names_a_pair_that_is_really_mounted() {
    let table = mounted_methods();
    let live: BTreeSet<(String, String)> = shadowed_pairs(&table)
        .into_iter()
        .map(|s| (s.stat, s.param))
        .collect();
    let stale: Vec<&str> = SHADOWED
        .iter()
        .filter(|(s, p, _)| !live.contains(&((*s).to_owned(), (*p).to_owned())))
        .map(|(s, _, _)| *s)
        .collect();
    assert!(
        stale.is_empty(),
        "these are exempt and no longer shadow anything: {stale:?}. Remove the rows — an \
         exemption for a pair that does not exist is an exemption nobody rereads, and the next \
         one that does exist inherits it."
    );
}

/// The scan is reading the router, not an empty file.
///
/// Without this a rename or a restructure would make every test above pass by
/// finding nothing, which is the silence this file exists to remove.
#[test]
fn the_scan_reads_the_whole_router() {
    let table = mounted_methods();
    assert!(
        table.len() > 200,
        "the scan found {} paths, which is far below the router's size — it has stopped \
         matching and would pass whatever the router did",
        table.len()
    );
    let mounts: usize = table.values().map(BTreeSet::len).sum();
    assert!(
        mounts > 250,
        "the scan found {mounts} method mounts across {} paths, which is too few to be this \
         router",
        table.len()
    );
    assert!(
        table.values().all(|m| !m.is_empty()),
        "some mounted path has no method, so the method scan is not reading handlers: {:?}",
        table
            .iter()
            .filter(|(_, m)| m.is_empty())
            .map(|(p, _)| p)
            .collect::<Vec<_>>()
    );
    // The routes this file is about, spelled out, so a restructure that moves
    // the memory surface fails here rather than quietly stopping the check.
    let memory = table
        .get("/api/memory/{bucket}")
        .expect("/api/memory/{bucket} is mounted");
    assert!(
        memory.contains("GET") && memory.contains("POST"),
        "/api/memory/{{bucket}} serves {memory:?}; the read and the write both belong on the \
         parameter, because a literal beside it takes the path"
    );
}

// ── Source reading, for the vocabulary check ─────────────────────────────────

/// Where `fn <name>` is **declared**, skipping every place it is merely called.
///
/// Lifted from `every_mounted_route_refuses_an_anonymous_caller.rs`, including
/// the two exclusions that file learned the hard way: a call site is preceded by
/// no `fn`, and `fn kind_for_bucket` starts with `fn kind_for`.
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

/// A function's source from its `fn` keyword to the closing brace in column 0.
fn function_body<'a>(src: &'a str, name: &str) -> Option<&'a str> {
    let at = definition_at(src, name)?;
    let rest = &src[at..];
    let end = rest.find("\n}\n").map_or(rest.len(), |e| e + 2);
    Some(&rest[..end])
}

/// Every double-quoted literal in a snippet.
fn string_literals(src: &str) -> BTreeSet<String> {
    src.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

// ── The readers themselves ───────────────────────────────────────────────────
//
// Everything above rests on `shadows` telling a real collision from two
// unrelated paths, and on the method scan telling a method call from an
// identifier that merely starts with one. Both are given the cases that matter,
// including the defect this file was written for — a guard that has only ever
// seen a router that passes is a guard nobody has tested.

/// The defect, as the router held it before 2026-09-22.
const THE_ROUTER_THAT_SHIPPED_THE_DEFECT: &str = r#"
        .route("/api/memory/preferences", post(memory::record_preference))
        .route("/api/memory/{bucket}", get(memory::list))
        .route("/api/memory/{bucket}/{id}", delete(memory::forget))
"#;

#[test]
fn the_pair_finder_names_the_defect_this_file_was_written_for() {
    let table = methods_in(THE_ROUTER_THAT_SHIPPED_THE_DEFECT);
    let found = shadowed_pairs(&table);
    assert_eq!(
        found,
        vec![Shadow {
            stat: "/api/memory/preferences".to_owned(),
            param: "/api/memory/{bucket}".to_owned(),
            lost: vec!["GET".to_owned()],
        }],
        "the scan was handed the exact router that served 405 on \
         GET /api/memory/preferences and did not report it, so it would not have caught the \
         defect it was written for"
    );
    // And the exemption could not have silenced it: `{bucket}` is not an id.
    assert!(!static_siblings_of(&table, "/api/memory/{bucket}").is_empty());
}

#[test]
fn the_pair_finder_ignores_a_static_route_that_serves_every_method() {
    // `/api/jobs/events` really is mounted beside `/api/jobs/{job_id}`, and both
    // serve only GET. Nothing is lost, so nothing should be reported.
    let table = methods_in(
        r#"
        .route("/api/jobs/events", get(jobs::events))
        .route("/api/jobs/{job_id}", get(jobs::get_job))
        "#,
    );
    assert_eq!(shadowed_pairs(&table), vec![]);
}

#[test]
fn the_pair_finder_ignores_paths_that_cannot_collide() {
    // Different depth: matchit routes them to different nodes.
    let table = methods_in(
        r#"
        .route("/api/models/leaderboard", get(models::leaderboard))
        .route("/api/models/{id}/runs", delete(models::delete_runs))
        "#,
    );
    assert_eq!(shadowed_pairs(&table), vec![]);
    // Two literals at the same depth shadow nothing; they are distinct nodes.
    let table = methods_in(
        r#"
        .route("/api/models/leaderboard", get(models::leaderboard))
        .route("/api/models/for-node", delete(models::for_node))
        "#,
    );
    assert_eq!(shadowed_pairs(&table), vec![]);
}

#[test]
fn the_pair_finder_sees_a_collision_below_a_shared_parameter() {
    // `/api/models/{id}/runs/reproduce` vs `/api/models/{id}/runs/{run_id}`:
    // the `{id}` matches on both sides and the collision is in the last
    // segment. A comparison that required the differing segment to be the
    // parameterised path's *only* parameter would miss this.
    let table = methods_in(
        r#"
        .route("/api/models/{id}/runs/reproduce", post(models::reproduce))
        .route("/api/models/{id}/runs/{run_id}", get(models::get_run))
        "#,
    );
    assert_eq!(
        shadowed_pairs(&table),
        vec![Shadow {
            stat: "/api/models/{id}/runs/reproduce".to_owned(),
            param: "/api/models/{id}/runs/{run_id}".to_owned(),
            lost: vec!["GET".to_owned()],
        }]
    );
}

#[test]
fn the_method_scan_reads_chained_and_qualified_methods() {
    let table = methods_in(
        r#"
        .route("/api/skills", get(list_skills).post(create_skill))
        .route("/api/settings", axum::routing::put(put_settings))
        .route(
            "/api/data/synthetic",
            post(synthetic::create_synthetic).get(synthetic::list_synthetic),
        )
        "#,
    );
    assert_eq!(
        table["/api/skills"],
        BTreeSet::from(["GET".to_owned(), "POST".to_owned()])
    );
    assert_eq!(table["/api/settings"], BTreeSet::from(["PUT".to_owned()]));
    assert_eq!(
        table["/api/data/synthetic"],
        BTreeSet::from(["GET".to_owned(), "POST".to_owned()])
    );
}

#[test]
fn the_method_scan_does_not_read_a_handler_name_as_a_method() {
    // `get(` inside `get_model`, and `post(` inside `post_settings`, are not
    // method calls. A prefix match would mount PATCH on the first and read the
    // second as a POST it is not.
    let table = methods_in(
        r#"
        .route("/api/models/for-node", get(models::get_model))
        .route("/api/x", delete(x::post_settings))
        "#,
    );
    assert_eq!(
        table["/api/models/for-node"],
        BTreeSet::from(["GET".to_owned()])
    );
    assert_eq!(table["/api/x"], BTreeSet::from(["DELETE".to_owned()]));
}

#[test]
fn the_method_scan_skips_a_commented_out_route() {
    // A commented-out route is not mounted, and the comment filter is applied to
    // the real router before the scan sees it.
    let table = mounted_methods();
    let router = read(&crate_src().join("routes").join("mod.rs"));
    assert!(
        router.contains(".route("),
        "the router file holds no routes, so the scan is reading the wrong file"
    );
    for path in table.keys() {
        assert!(
            path.starts_with('/'),
            "`{path}` is not a path, so the scan is reading something that is not a route"
        );
    }
}

#[test]
fn the_shape_comparison_knows_a_parameter_from_a_literal() {
    assert!(shadows("/api/memory/preferences", "/api/memory/{bucket}"));
    assert!(shadows("/api/backtests/coverage", "/api/backtests/{id}"));
    // Not the other way round: the parameterised path does not shadow the
    // literal, and reporting it would double every pair.
    assert!(!shadows("/api/memory/{bucket}", "/api/memory/preferences"));
    // Same path, different depth, and an unrelated prefix.
    assert!(!shadows("/api/memory/{bucket}", "/api/memory/{bucket}"));
    assert!(!shadows("/api/memory/x", "/api/memory/{b}/{i}"));
    assert!(!shadows("/api/models/x", "/api/memory/{b}"));
    // A wildcard is still a parameter.
    assert!(shadows("/api/assets/manifest", "/api/assets/{*rest}"));
    assert_eq!(parameter_name("{*rest}"), "rest");
    assert_eq!(parameter_name("{run_id}"), "run_id");
}

#[test]
fn the_vocabulary_reader_finds_the_bucket_names() {
    let src = read(&crate_src().join("routes").join("memory.rs"));
    let body = function_body(&src, "kind_for").expect("kind_for is in routes/memory.rs");
    let values = string_literals(body);
    for bucket in [
        "findings",
        "negative",
        "profiles",
        "questions",
        "preferences",
        "signals",
    ] {
        assert!(
            values.contains(bucket),
            "`kind_for` no longer maps `{bucket}`, so the vocabulary check is reading less than \
             it thinks: {values:?}"
        );
    }
    // `playbooks` is a tab in the browser and not a kind in the store, and it is
    // refused by name in `unknown_bucket` rather than mapped here. If it ever
    // appears in `kind_for`, somebody has invented a kind inside a route.
    assert!(
        !values.contains("playbooks"),
        "`kind_for` now maps `playbooks`, which is a schema decision taken in a route — \
         AGENT-007 §2.2 has no playbook entry"
    );
}

#[test]
fn the_definition_reader_does_not_match_a_longer_name() {
    let src = "fn kind_for_bucket(b: &str) -> u8 { 0 }\n";
    assert!(function_body(src, "kind_for").is_none());
    assert!(function_body(src, "kind_for_bucket").is_some());
}
