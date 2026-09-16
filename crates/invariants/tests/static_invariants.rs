//! Static, build-blocking invariant checks over the source tree.
//!
//! Each test asserts a violation cannot exist, by scanning the code that would
//! contain it. If one fails, remove the violation — never widen the allowlist to
//! make it pass without recording why in decisions/ADR-INDEX.md.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}

fn files(dirs: &[&str], ext: &[&str]) -> Vec<(String, String)> {
    fn walk(dir: &Path, ext: &[&str], out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
            if p.is_dir() {
                if !matches!(name, "target" | "node_modules" | ".git" | "docs" | "dist") {
                    walk(&p, ext, out);
                }
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| ext.contains(&x)) {
                out.push(p);
            }
        }
    }
    let r = root();
    let mut paths = Vec::new();
    for d in dirs {
        walk(&r.join(d), ext, &mut paths);
    }
    paths
        .into_iter()
        .filter_map(|p| {
            let rel = p.strip_prefix(&r).ok()?.to_string_lossy().replace('\\', "/");
            Some((rel, fs::read_to_string(&p).ok()?))
        })
        .collect()
}

fn rust_files() -> Vec<(String, String)> {
    files(&["crates", "apps"], &["rs"])
}

fn strip_comments(src: &str) -> String {
    // Block comments first: a `/* ... // ... */` would otherwise lose only its
    // inner line and leave the rest looking like code. Both kinds are stripped
    // because these scans forbid *implementations*, and explaining a ban — in a
    // doc comment, a JSX comment, or a block above the code — is exactly what
    // the modules that implement the alternative do.
    let block = Regex::new(r"(?s)/\*.*?\*/").unwrap();
    let line = Regex::new(r"(?m)//.*$").unwrap();
    line.replace_all(&block.replace_all(src, ""), "").into_owned()
}

/// AT-01 ⛔ · No non-PIT read path: only the reader (and migration machinery) may
/// name a raw bar table.
#[test]
fn at01_only_the_pit_reader_touches_raw_bar_tables() {
    let allowed = [
        "crates/backtest/src/store.rs",
        "crates/storage/src/clickhouse/migrate.rs",
        "crates/storage/src/clickhouse/backfill.rs",
        "crates/storage/src/clickhouse/canonical.rs",
        "crates/storage/examples/backfill_bars_v2.rs",
        "crates/storage/tests/canonical_migration.rs",
        "crates/storage/tests/clickhouse_migrate.rs",
        // AT-02's comparison scan exists only to prove the reader has no excuse.
        "crates/backtest/tests/bar_store_pit.rs",
        "crates/invariants/tests/static_invariants.rs",
        "crates/invariants/tests/db_invariants.rs",
    ];
    let raw = Regex::new(r"\b(FROM|JOIN|INTO|TABLE|insert\(\s*)\s*`?(market_bar|market_bars_v2|market_bars|option_bar_1m|pool_observation)`?\b").unwrap();
    let mut violations = Vec::new();
    for (path, src) in rust_files() {
        if allowed.contains(&path.as_str()) {
            continue;
        }
        let code = strip_comments(&src);
        if raw.is_match(&code) || code.contains("insert(\"market_bar") {
            violations.push(path);
        }
    }
    assert!(violations.is_empty(), "raw bar tables read outside the PIT reader: {violations:?}");
}

fn ddl() -> Vec<(String, String)> {
    let mut v = files(&["migrations"], &["sql"]);
    v.extend(files(&["clickhouse"], &["sql"]));
    v
}

/// AT-03 ⛔ · No adjusted-price columns anywhere in the schema.
#[test]
fn at03_no_adjusted_price_columns() {
    let bad = Regex::new(r"(?i)\b(adj_\w+|\w+_adjusted|split_adjusted\w*|adjusted_close)\b\s+(numeric|decimal|double|float|real|int)").unwrap();
    let hits: Vec<String> = ddl().into_iter().filter(|(_, s)| bad.is_match(s)).map(|(p, _)| p).collect();
    assert!(hits.is_empty(), "adjusted price columns are forbidden (INV-03): {hits:?}");
}

/// AT-04 ⛔ · Price columns in the ML data plane are DECIMAL(38,18), never float.
#[test]
fn at04_price_columns_are_decimal_38_18() {
    let price = Regex::new(r"(?im)^\s*(open|high|low|close|vwap|bid_close|ask_close|underlying_close|strike|price|rate|tick_size|lot_size|contract_size|tick_value|price_factor|volume_factor|cash_amount|reserve0|reserve1|tvl_usd|sqrt_price|liquidity)\s+(\w+(?:\((?:[^()]|\([^()]*\))*\))?)").unwrap();
    let mut bad = Vec::new();
    for (path, src) in ddl() {
        // Every DDL file that defines ML-data-plane prices must be listed here, or
        // it escapes INV-05 silently. `08_forward_series.sql` (Set P P-0.3/P-0.4)
        // did exactly that until it was added: a new file is out of scope by
        // default, which is the wrong default for an invariant.
        let in_scope = path.ends_with("0046_dataplane.sql")
            || path.ends_with("07_canonical_market.sql")
            || path.ends_with("08_forward_series.sql");
        if !in_scope {
            continue;
        }
        for cap in price.captures_iter(&src) {
            let ty = cap[2].to_ascii_lowercase().replace(' ', "");
            let ok = ty.contains("numeric(38,18)") || ty.contains("decimal(38,18)");
            if !ok {
                bad.push(format!("{path}: {} {}", &cap[1], &cap[2]));
            }
        }
    }
    assert!(bad.is_empty(), "prices must be DECIMAL(38,18) (INV-05): {bad:?}");
}

/// AT-05 ⛔ · `symbol` appears in no primary key in the ML data plane.
#[test]
fn at05_symbol_is_never_a_key() {
    let pk = Regex::new(r"(?is)PRIMARY KEY\s*\(([^)]*)\)").unwrap();
    let order_by = Regex::new(r"(?is)ORDER BY\s*\(([^)]*)\)").unwrap();
    let mut bad = Vec::new();
    for (path, src) in ddl() {
        let pg_scope = path.contains("migrations/004") || path.contains("migrations/005");
        let ch_scope = path.ends_with("07_canonical_market.sql");
        if pg_scope {
            for c in pk.captures_iter(&src) {
                if c[1].to_ascii_lowercase().contains("symbol") {
                    bad.push(format!("{path}: PRIMARY KEY ({})", &c[1]));
                }
            }
        }
        if ch_scope {
            for c in order_by.captures_iter(&src) {
                let cols = c[1].to_ascii_lowercase();
                // The symbol dimension is the one table whose job is to look symbols up.
                if cols.contains("symbol") && !src[..c.get(0).unwrap().start()].trim_end().ends_with("ReplacingMergeTree") {
                    bad.push(format!("{path}: ORDER BY ({})", &c[1]));
                }
            }
        }
    }
    assert!(bad.is_empty(), "symbol is a bitemporal attribute, never a key (INV-04): {bad:?}");
}

/// AT-08 · No greek column exists in any options table.
#[test]
fn at08_no_greek_columns() {
    let greek = Regex::new(r"(?im)^\s*(delta|gamma|vega|theta|rho|vanna|volga|charm)\s+(numeric|decimal|double|float|real|nullable)").unwrap();
    let hits: Vec<String> = ddl().into_iter().filter(|(_, s)| greek.is_match(s)).map(|(p, _)| p).collect();
    assert!(hits.is_empty(), "store IV, never greeks (INV-07): {hits:?}");
}

/// AT-10 ⛔ · No as-of join other than the backward-only wrapper.
#[test]
fn at10_asof_nearest_is_unreachable() {
    let asof = Regex::new(r#"(?i)(ASOF\s+(LEFT\s+)?JOIN|join_asof|strategy\s*[:=]\s*"nearest"|"nearest"\s*=>)"#).unwrap();
    let mut bad = Vec::new();
    for (path, src) in rust_files() {
        if path == "crates/dataplane/src/asof.rs" || path.starts_with("crates/invariants/") {
            continue;
        }
        if asof.is_match(&strip_comments(&src)) {
            bad.push(path);
        }
    }
    assert!(bad.is_empty(), "as-of joins must go through dataplane::asof (INV-10): {bad:?}");
}

/// AT-15 ⛔ · One implementation per feature: no second EMA/RSI anywhere.
#[test]
fn at15_one_feature_implementation() {
    let def = Regex::new(r"(?m)^\s*pub\s+struct\s+(Ema|Rsi)\b").unwrap();
    let mut defs = Vec::new();
    for (path, src) in rust_files() {
        for c in def.captures_iter(&src) {
            defs.push(format!("{}@{path}", &c[1]));
        }
    }
    defs.sort();
    assert_eq!(
        defs,
        vec!["Ema@crates/features/src/ema.rs".to_string(), "Rsi@crates/features/src/rsi.rs".to_string()],
        "features have exactly one implementation (INV-14)"
    );
}

/// AT-19 ⛔ (static half) · A TrialTicket can only be minted inside the ledger crate.
#[test]
fn at19_trial_tickets_are_only_minted_by_the_ledger() {
    let literal = Regex::new(r"TrialTicket\s*\{").unwrap();
    let mut bad = Vec::new();
    for (path, src) in rust_files() {
        if path.starts_with("crates/ledger/src/") || path.starts_with("crates/invariants/") {
            continue;
        }
        if literal.is_match(&strip_comments(&src)) {
            bad.push(path);
        }
    }
    assert!(bad.is_empty(), "TrialTicket constructed outside the ledger (INV-16): {bad:?}");
    let lib = fs::read_to_string(root().join("crates/ledger/src/lib.rs")).unwrap();
    assert!(!Regex::new(r"pub\s+fn\s+mint").unwrap().is_match(&lib), "mint must stay private");
    assert!(!Regex::new(r"pub\s+(trial_id|tenant_id|config_hash|state)\s*:").unwrap().is_match(
        &lib[lib.find("pub struct TrialTicket").unwrap()..lib.find("impl TrialTicket").unwrap()]
    ), "TrialTicket fields must stay private");
}

/// S-2 · Tenant context is only ever transaction-local.
#[test]
fn s2_no_session_level_tenant_set() {
    let bare = Regex::new(r"(?i)\bSET\s+(SESSION\s+)?app\.tenant_id|set_config\(\s*'app\.tenant_id'\s*,\s*[^,]+,\s*false").unwrap();
    let mut bad = Vec::new();
    for (path, src) in rust_files() {
        if path.starts_with("crates/invariants/") || path == "crates/ledger/tests/pg_tenancy.rs" {
            continue;
        }
        if bare.is_match(&src) {
            bad.push(path);
        }
    }
    assert!(bad.is_empty(), "bare SET leaks tenant context across pooled connections: {bad:?}");
}

fn tool_properties() -> Vec<(String, Vec<String>)> {
    let defs = mcp_server_lib::tool_definitions_for(mcp_server_lib::ToolProfile::Mcp);
    defs.as_array()
        .unwrap()
        .iter()
        .map(|t| {
            let name = t["name"].as_str().unwrap_or_default().to_string();
            let props = t["inputSchema"]["properties"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
            (name, props)
        })
        .collect()
}

/// AT-24 · The exploration floor is not a field any agent tool accepts.
#[test]
fn at24_exploration_floor_is_not_in_any_tool_schema() {
    for (tool, props) in tool_properties() {
        for p in props {
            assert!(!p.contains("exploration"), "tool {tool} exposes {p} (INV-21)");
        }
    }
}

/// AT-27 ⛔ · No tool accepts a trial count or N_eff.
#[test]
fn at27_no_tool_accepts_a_trial_count() {
    let forbidden = Regex::new(r"(?i)^(n_eff|neff|trial_count|num_trials|n_trials|effective_trials|trials_run)$").unwrap();
    for (tool, props) in tool_properties() {
        for p in props {
            assert!(!forbidden.is_match(&p), "tool {tool} accepts {p}: N_eff is platform-computed only (INV-22)");
        }
    }
}

/// INV-23 · No agent tool can write gate thresholds, the deflation code path, or
/// read smoothed regimes.
#[test]
fn inv23_no_governance_fields_in_tool_schemas() {
    let forbidden = Regex::new(r"(?i)(threshold|gate_profile_thresholds|deflation|smoothed|viterbi|regime_research|sealed_holdout_result)").unwrap();
    for (tool, props) in tool_properties() {
        for p in props {
            assert!(!forbidden.is_match(&p), "tool {tool} exposes governance field {p}");
        }
    }
}

/// SPEC §14.6 · Every dispatched tool step is recorded: the untraced dispatcher is
/// private and has exactly one caller, the recording wrapper.
#[test]
fn every_tool_dispatch_is_trajectory_logged() {
    let lib = fs::read_to_string(root().join("crates/mcp-server/src/lib.rs")).unwrap();
    assert!(!lib.contains("pub async fn dispatch_untraced"), "the untraced dispatcher must stay private");
    let calls = Regex::new(r"dispatch_untraced\(").unwrap();
    let mut total = 0;
    for (path, src) in rust_files() {
        if path.starts_with("crates/invariants/") {
            continue;
        }
        // Production code only. `mcp-server/src/lib.rs` carries a structural
        // test that asserts the dispatch table has exactly one caller, and it
        // has to name `dispatch_untraced(ctx` to do so. Counting the test's own
        // mentions would make the stronger check the reason the weaker one
        // fails — the same shape as AT-42.
        let n = calls.find_iter(&strip_comments(production_only(&src))).count();
        if n > 0 {
            assert_eq!(path, "crates/mcp-server/src/lib.rs", "dispatch_untraced referenced outside the MCP library");
        }
        total += n;
    }
    assert_eq!(total, 2, "one definition and one call, from dispatch_under");
    // The slice spans both public entry points: `dispatch_tool`, which opens the
    // audit envelope, and `dispatch_under`, which does the recording for both.
    let wrapper = &lib[lib.find("pub async fn dispatch_tool(").unwrap()..lib.find("async fn dispatch_untraced(").unwrap()];
    assert!(wrapper.contains("record_step"), "the dispatch path must record the step");
}

/// AT-62 ⛔ · No critical-difference renderer exists (SPEC §11.5, ADR-P2-13).
///
/// A critical-difference diagram's cliques depend on which *other* candidates
/// happen to be in the comparison, so adding an unrelated method can make two
/// that did not move become "not significantly different". §11.5 bans it and the
/// Multiple Comparison Matrix replaces it. This test is the ban: the check is
/// that no type, endpoint, component or asset in the repository names one, so
/// the absence is a fact about the tree rather than a decision someone has to
/// remember.
#[test]
fn at62_no_critical_difference_renderer_exists() {
    let forbidden = Regex::new(r"(?i)(critical[_\- ]?difference|cd[_\-]diagram|nemenyi)").unwrap();
    let sources = files(
        &["crates", "apps", "frontend", "web", "ui"],
        &["rs", "ts", "tsx", "js", "jsx", "py", "sql", "json"],
    );
    for (path, src) in sources {
        // This test has to name the thing it forbids.
        if path == "crates/invariants/tests/static_invariants.rs" {
            continue;
        }
        // Comments are stripped first: naming the ban in order to explain it is
        // exactly what `compare.rs` does in its documentation, and a comment
        // cannot render a diagram.
        let code = strip_comments(&src);
        if let Some(m) = forbidden.find(&code) {
            let line = code[..m.start()].matches('\n').count() + 1;
            panic!(
                "{path}:{line}: `{}` — §11.5 bans critical-difference diagrams; the comparison \
                 view is the Multiple Comparison Matrix (backtest::stats::compare)",
                m.as_str()
            );
        }
    }
}

/// SPEC §11.5 · A `Comparison` is minted only by the protocol that computes it
/// (ADR-P2-12).
///
/// The seal is the same one `TrialTicket` has, for the same reason: the type's
/// value is that holding one proves the context came with it — the gate profile,
/// `N_eff`, the declared effect size, both trial counts and every flag. A
/// struct-literal anywhere else would be a verdict somebody assembled by hand.
#[test]
fn only_the_protocol_constructs_a_comparison() {
    let literal = Regex::new(r"\bComparison\s*\{").unwrap();
    for (path, src) in rust_files() {
        if path == "crates/backtest/src/stats/compare.rs"
            || path == "crates/invariants/tests/static_invariants.rs"
        {
            continue;
        }
        let code = strip_comments(&src);
        // `ComparisonPlan { .. }` and `ComparisonFlags { .. }` are the inputs and
        // are meant to be built by callers; only the verdict itself is sealed.
        for m in literal.find_iter(&code) {
            let before = &code[..m.start()];
            if before.ends_with("Plan") || before.ends_with("Flags") || before.ends_with("Matrix") {
                continue;
            }
            let line = before.matches('\n').count() + 1;
            panic!(
                "{path}:{line}: a `Comparison` is constructed outside the comparison protocol; \
                 use ComparisonPlan::judge so the verdict cannot arrive without its context"
            );
        }
    }
}

/// AT-35 · The decision threshold is closed-form: it consumes the cost matrix
/// and performs no search (SPEC §11.4, ADR-P2-11).
///
/// A *tuned* threshold is a search over out-of-sample outcomes, which is an
/// evaluation trial, which means the counter must climb for it. Tuning it in
/// post-processing and calling the result post-processing is exactly how that
/// count gets avoided, so the check is structural: the function's body contains
/// no loop, no grid and no argmax, and its signature admits no data to tune
/// against.
#[test]
fn at35_the_decision_threshold_performs_no_search() {
    let src = fs::read_to_string(root().join("apps/model-trainer/app/posthoc.py"))
        .expect("the post-hoc pipeline exists");

    let signature = "def decision_threshold(costs: CostMatrix) -> float:";
    assert!(
        src.contains(signature),
        "the threshold must take the cost matrix and nothing else — a parameter carrying scores, \
         labels or outcomes is a parameter it can be tuned against"
    );

    // The body runs from the signature to the next top-level `def`.
    let start = src.find(signature).unwrap() + signature.len();
    let rest = &src[start..];
    let end = rest.find("\ndef ").unwrap_or(rest.len());
    let body = &rest[..end];

    let search = Regex::new(r"(?i)\b(for|while|argmax|argmin|linspace|arange|grid|sweep|optimi[sz]e|minimize|maximize)\b").unwrap();
    // Strip the docstring: it is allowed to explain the ban it is documenting.
    let code: String = body
        .lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with('#') && !t.is_empty()
        })
        .collect::<Vec<_>>()
        .join("\n");
    let code = code.split("\"\"\"").enumerate().filter(|(i, _)| i % 2 == 0).map(|(_, s)| s).collect::<String>();

    if let Some(m) = search.find(&code) {
        panic!(
            "apps/model-trainer/app/posthoc.py: `{}` inside decision_threshold — the threshold is \
             arithmetic on the cost matrix (SPEC §11.4); a search over it is an uncounted trial",
            m.as_str()
        );
    }

    // And the pipeline has no way to reorder or skip its steps.
    let run = src.find("def run_posthoc(").expect("the pipeline exists");
    let head = &src[run..run + src[run..].find("-> PostHocReport:").expect("signature") ];
    for forbidden in ["order", "steps", "skip", "reorder", "enable"] {
        assert!(
            !head.contains(&format!("{forbidden}:")),
            "run_posthoc exposes `{forbidden}`; the order is the guarantee (SPEC §11.4)"
        );
    }
}

/// AT-63 ⛔ · The platform seed appears in no API type (SPEC §12.7, ADR-P2-18).
///
/// The column grant is the enforcement; this is the other half of the same
/// claim. A seed that never reaches an API response cannot be read by the agent
/// even if a future grant is widened by accident, and a response type that names
/// it is the shape that mistake would take.
#[test]
fn at63_no_api_type_carries_the_platform_seed() {
    let seed = Regex::new(r"platform_seed").unwrap();
    let allowed = [
        // The ledger's own accessor, which runs as the platform role.
        "crates/ledger/src/pg.rs",
        "crates/invariants/tests/static_invariants.rs",
        "crates/invariants/tests/db_invariants.rs",
        // Tests that exist to prove the seed is unreadable have to name it, the
        // same way AT-02's comparison scan has to name the raw bar tables.
        "crates/api/tests/pg_self_monitor.rs",
    ];
    let mut hits = Vec::new();
    for (path, src) in files(&["crates", "apps", "frontend", "web", "ui"], &["rs", "ts", "tsx"]) {
        if allowed.contains(&path.as_str()) {
            continue;
        }
        if seed.is_match(&strip_comments(&src)) {
            hits.push(path);
        }
    }
    assert!(
        hits.is_empty(),
        "the platform seed must not reach any type outside the ledger's own reader: {hits:?}"
    );
}

/// AT-64 · Denials are provable, through **every** door: the `pre` record is
/// written before the policy check, and a dispatch the platform will not record
/// does not happen (SPEC §15, ADR-P2-21, ADR-P2-34).
///
/// The property is an *ordering*, and ordering is the kind of thing a refactor
/// reverses without anyone noticing, so it is pinned in the source rather than
/// left to a convention.
///
/// # What changed, and why this test no longer lists callers
///
/// It used to enumerate every `dispatch_tool` caller and assert the list, with
/// `apps/mcp-server/src/main.rs` on it as a **named exemption**: the external
/// front door is a separate process and wrote no trail. A list is the weakest
/// possible form of this check — it goes stale by being edited, and the edit
/// that adds a new door looks exactly like the edit that keeps the list current.
///
/// The exemption is closed and the list is gone. Coverage is now structural:
/// `dispatch_tool`'s first argument is an `&McpContext`, an `McpContext` cannot
/// be built without an `AuditSink`, and `dispatch_under` — the variant for a
/// caller that opened its own record — takes an `AuditEnvelope` that only
/// `AuditSink::open` can mint. A new dispatch path that forgets to audit does
/// not fail this test; it fails to compile. What this test defends is the
/// *mechanism* those guarantees rest on, because every one of them is one
/// plausible-looking edit away from being lost.
#[test]
fn at64_no_dispatch_path_can_skip_the_audit_trail() {
    let lib = fs::read_to_string(root().join("crates/mcp-server/src/lib.rs"))
        .expect("the dispatch chokepoint exists");
    let src = strip_comments(&lib);

    // ── 1. The order, inside the one function every door goes through ────────
    let start = src.find("pub async fn dispatch_tool(").expect("dispatch_tool exists");
    let body = &src[start..];
    let end = body.find("pub async fn dispatch_under(").unwrap_or(body.len());
    let body = &body[..end];

    let open = body.find(".open(").expect("dispatch_tool must open an audit record");
    let dispatch = body.find("dispatch_under(").expect("dispatch_tool must dispatch");
    let close = body.find(".close(").expect("dispatch_tool must close the record");
    assert!(
        open < dispatch,
        "the audit pre-record must be opened BEFORE the dispatch: a denial that is not \
         recorded first leaves no evidence the control did anything"
    );
    assert!(close > dispatch, "the post-record must follow the dispatch");

    // ── 2. And a record the platform would not open means no dispatch ────────
    //
    // This is the half that makes it a property rather than a claim. `Err` must
    // return, not warn: a logged miss is a look nobody counted.
    let refusal = body.find("audit_unavailable").expect(
        "dispatch_tool must refuse when the platform opens no record; \
         `audit_unavailable` is the refusal's name",
    );
    assert!(
        refusal < dispatch,
        "the refusal must be decided before the dispatch, not reported after it"
    );

    // ── 3. The envelope is sealed, so `dispatch_under` cannot be forged ──────
    //
    // `AuditEnvelope(` appearing outside `AuditSink::open` would be a second
    // constructor, and a second constructor is a way to dispatch with no record.
    let declaration = src
        .find("pub struct AuditEnvelope(")
        .expect("AuditEnvelope is a newtype over the platform's audit id");
    let constructions: Vec<usize> = src
        .match_indices("AuditEnvelope(")
        .map(|(i, _)| i)
        // The type's own declaration is not a construction of it.
        .filter(|i| *i != declaration + "pub struct ".len())
        .collect();
    let sink_open = src.find("pub async fn open(").expect("AuditSink::open exists");
    let sink_open_end = src[sink_open..]
        .find("pub async fn close(")
        .map_or(src.len(), |o| sink_open + o);

    // One further site is allowed, named here rather than exempted by category.
    //
    // `AuditSink` has exactly one constructor and it posts to the platform, by
    // design -- there is no offline sink. So a test of anything *downstream* of
    // the audit record (the call gate, spec resolution) cannot reach its subject
    // without an envelope: against an unreachable base URL every such call stops
    // at `audit_unavailable` instead. The helper below mints one inside
    // `#[cfg(test)]`, where production code cannot call it.
    //
    // It is named, singular, and asserted to be inside the test module. A blanket
    // "tests do not count" would be the weaker rule: this one still fails if a
    // second test-side mint appears, or if this one escapes `#[cfg(test)]`.
    const TEST_MINT: &str = "AuditEnvelope(Uuid::nil())";
    let test_module = src.find("#[cfg(test)]").unwrap_or(src.len());
    let allowed: Vec<usize> = src.match_indices(TEST_MINT).map(|(i, _)| i).collect();
    assert!(
        allowed.len() <= 1,
        "more than one test-side AuditEnvelope mint: {allowed:?}. One is a necessity;          two is a habit"
    );
    for at in &allowed {
        assert!(
            *at > test_module,
            "the test-side AuditEnvelope mint is outside `#[cfg(test)]` and therefore              reachable from production"
        );
    }

    for at in constructions {
        let sealed = at > sink_open && at < sink_open_end;
        let test_side = allowed.contains(&at);
        assert!(
            sealed || test_side,
            "an AuditEnvelope is minted outside AuditSink::open at byte {at}; the seal is              what makes holding one proof that a pre-record exists"
        );
    }
    assert!(
        src.contains("_envelope: &AuditEnvelope"),
        "dispatch_under must take the envelope as a proof obligation, or a caller that \
         never opened a record can reach the dispatch"
    );

    // ── 4. No context exists without a sink ──────────────────────────────────
    //
    // A public `audit` field would let any crate build an `McpContext` with a
    // struct literal and skip `new`, which is the one place the sink is attached.
    assert!(
        src.contains("    audit: AuditSink,"),
        "McpContext::audit must stay private: a public field is a struct literal away \
         from a context with no audit trail"
    );
    assert!(
        src.contains("audit: AuditSink::through_platform("),
        "McpContext::new must build the sink"
    );
    // And there is no second way to build one — no disabled, no-op or defaulted
    // sink for tests, which is how this kind of seal is usually undone.
    for hatch in ["AuditSink::none", "AuditSink::noop", "AuditSink::disabled", "impl Default for AuditSink"] {
        assert!(!src.contains(hatch), "{hatch} is an escape hatch from the audit trail");
    }

    // ── 5. The record is written by the platform, not by the caller ──────────
    //
    // ADR-0022 made the MCP server a thin client precisely so enforcement lives
    // platform-side (ADR-0025). If the sink ever grew a database handle, the
    // external process would be writing its own trail again — and an audit a
    // client writes is an audit a client can decline to write.
    let sink = {
        let at = src.find("impl AuditSink {").expect("the sink exists");
        // Comments are stripped, so the section banner is gone; the next item
        // in the file is the context struct.
        &src[at..src[at..].find("pub struct McpContext").map_or(src.len(), |o| at + o)]
    };
    assert!(
        sink.contains("self.api.post(\"/api/agent/audit\""),
        "AuditSink::open must ask the platform to write the record"
    );
    for local_write in ["PgPool", "sqlx", "record_audit_pre"] {
        assert!(
            !sink.contains(local_write),
            "the sink writes the record itself ({local_write}); it must ask the platform, \
             which is the component a caller cannot route around"
        );
    }

    // ── 6. Callers that open their own envelope close it, in order ───────────
    //
    // `dispatch_under` exists for drivers whose record has to span more than the
    // dispatch — the SDK driver opens it before its policy gate decides, so the
    // four §15 actions it refuses are recorded as asked-for and denied, and its
    // two inline paths (`run_sweep`, `wait_for_backtest`) are recorded at all.
    for (path, file) in rust_files() {
        if path.starts_with("crates/invariants/") || path == "crates/mcp-server/src/lib.rs" {
            continue;
        }
        let clean = strip_comments(&file);
        let Some(at) = clean.find("dispatch_under(") else { continue };
        let opened = clean.find(".open(").unwrap_or(usize::MAX);
        let closed = clean.rfind(".close(").unwrap_or(0);
        assert!(
            opened < at,
            "{path} dispatches under an envelope it never opened"
        );
        assert!(
            closed > at,
            "{path} opens an audit record and never closes it; a `pre` with no `post` \
             reads as a call that is still in flight"
        );
    }

    // ── 7. The platform endpoint the sink depends on is actually routed ──────
    //
    // The seal above guarantees a caller *asks*. This guarantees someone answers:
    // without the route, every dispatch in the system refuses, which is safe but
    // is not the behaviour anybody wants to discover in production.
    let routes = fs::read_to_string(root().join("crates/api/src/routes/mod.rs")).unwrap();
    assert!(
        routes.contains("\"/api/agent/audit\"") && routes.contains("\"/api/agent/audit/{audit_id}\""),
        "the audit trail's two halves must be routed on the platform"
    );
}

/// SPEC §15 conformance over the AGENT-002 catalogue (checklist 2.16,
/// ADR-P2-19).
///
/// §15 is not a second tool surface to build beside the catalogue — three
/// tool-surface designs would mean three places for a forbidden field to appear.
/// It is a **checklist over the one catalogue**, and this is the checklist,
/// written as data so the gap is a machine-checked fact rather than a paragraph
/// somebody has to keep current.
///
/// Each row is a §15 tool and the catalogue name that answers it. `None` means
/// nothing answers it yet. The test fails in **both** directions: a pending tool
/// that quietly appears, and a mapped tool that disappears. So the list can only
/// go stale by failing the build.
#[test]
fn spec_15_tool_surface_conformance() {
    // (§15 name, the AGENT-002 tool that answers it)
    const CHECKLIST: &[(&str, Option<&str>)] = &[
        // data & datasets — the dataset plane is typed in `dataplane` but has no
        // agent tools yet (checklist 3.x).
        ("inspect_dataset", None),
        ("build_dataset_spec", None),
        ("diff_datasets", None),
        // assets & regimes — Phase 3 (fingerprints, embeddings, the regime model).
        ("describe_asset", None),
        ("find_similar_assets", None),
        ("get_regime", None),
        // experiments
        ("propose_experiment", Some("create_experiment")),
        ("estimate_cost", None),
        ("launch_trials", Some("run_sweep")),
        ("get_trial", Some("get_backtest")),
        ("cancel_trials", Some("cancel_sweep")),
        // search control — checklist 2.6/2.8/2.10.
        ("adjust_search_space", None),
        ("reallocate_budget", None),
        ("prune_branch", None),
        // checkpoints & artifacts — checklist 2.4/2.5.
        ("list_checkpoints", None),
        ("branch_from_checkpoint", None),
        ("inspect_artifact", None),
        // evaluation
        ("compare_candidates", Some("compare_backtests")),
        ("run_gates", Some("advance_gate")),
        ("explain_failure", Some("get_diagnostics")),
        ("request_promotion", Some("promote_model_version")),
        // memory — checklist 3.10.
        ("search_insights", None),
        ("write_insight", None),
    ];

    let catalogue: BTreeSet<String> = tool_properties().into_iter().map(|(n, _)| n).collect();

    let mut missing_mapped = Vec::new();
    let mut appeared = Vec::new();
    for (spec_name, mapped) in CHECKLIST {
        match mapped {
            Some(tool) => {
                if !catalogue.contains(*tool) {
                    missing_mapped.push(format!("§15 {spec_name} → {tool}"));
                }
            }
            None => {
                if catalogue.contains(*spec_name) {
                    appeared.push(*spec_name);
                }
            }
        }
    }
    assert!(
        missing_mapped.is_empty(),
        "a catalogue tool that answered a §15 requirement has gone: {missing_mapped:?}"
    );
    assert!(
        appeared.is_empty(),
        "these §15 tools now exist and the checklist still calls them pending — map them and          check their conventions: {appeared:?}"
    );

    // §15's cross-cutting conventions, checked on the tools that do answer it.
    // `response_format` is the one with teeth (~67 % token savings) and the one a
    // new tool most often forgets.
    let props: BTreeMap<String, BTreeSet<String>> = tool_properties()
        .into_iter()
        .map(|(n, p)| (n, p.into_iter().collect()))
        .collect();
    // §15 asks dispatching tools for an `idempotency_key`, and says in the same
    // breath what it is for: `launch_trials` "dedups by config_hash". The
    // platform does that deduplication in the submission transaction — a repeat
    // is a `DEDUPLICATED` trial naming the prior one (INV-1, ADR-P0-01) — so the
    // property §15 wants holds without a key travelling through the tool schema.
    //
    // What a retrying *caller* additionally needs is a way to rejoin work already
    // running rather than starting more of it, and that is a resume handle.
    let resume_handles = [("run_sweep", "sweep_id"), ("get_backtest", "backtest_id")];
    for (tool, handle) in resume_handles {
        let schema = props.get(tool).unwrap_or_else(|| panic!("{tool} is in the catalogue"));
        assert!(
            schema.contains(handle),
            "{tool} has no `{handle}`: a retry would start a second one rather than rejoin the first"
        );
    }

    // Promotion is gated rather than keyed: a second request cannot double-promote
    // because it pauses for a human either way (§15's approval envelopes).
    // The name here is the *tool* name, which is what both drivers pass to
    // `for_action`. It said `promote_model` until 2026-09-15 and so asserted a
    // property of a string no tool answers to; the catalogue's tool has always
    // been `promote_model_version` (Set S task S-0.2).
    assert!(
        ledger::audit::Envelope::for_action("promote_model_version").requires_approval(),
        "promotion must fall under an approval envelope"
    );

    // The not-exposed list (§15, last paragraph). AT-24 and AT-27 cover the
    // exploration floor and the trial count; this is the rest of it.
    let forbidden = Regex::new(r"(?i)(n_eff|deflat|threshold|gate_profile|delta_practical|regime_research)").unwrap();
    for (tool, schema) in &props {
        for p in schema {
            assert!(
                !forbidden.is_match(p),
                "tool {tool} exposes `{p}` — §15 exposes none of: gate thresholds,                  delta_practical after DEFINE, N_eff, the deflation formula, regime_research"
            );
        }
    }
}

/// AT-68 · A fingerprint dimension is not a feature (SPEC §5.2, ADR-P3-01).
///
/// INV-14 governs what a strategy may be served: one windowed implementation per
/// feature, evaluated through one function. Asset fingerprints are knowledge-plane
/// embeddings with their own `knowledge_time` and their own recomputation
/// schedule — resolving one by name inside the feature runtime would put a
/// quantity computed over a 21-day trailing window, by a different job, on a
/// different clock, into a bar-aligned feature vector.
///
/// If a fingerprint dimension is ever genuinely wanted as a feature, it is
/// registered as a windowed feature like any other. What must not happen is it
/// arriving by name.
#[test]
fn at68_the_feature_runtime_cannot_resolve_an_embedding_dimension() {
    // The feature runtime's resolver must not mention the knowledge plane at all.
    let runtime = fs::read_to_string(root().join("crates/features/src/runtime.rs"))
        .expect("the feature runtime exists");
    let knowledge = Regex::new(r"(?i)(asset_embedding|retrieval_vec|tier1|tier2|tier3|fingerprint)").unwrap();
    if let Some(m) = knowledge.find(&strip_comments(&runtime)) {
        panic!(
            "crates/features/src/runtime.rs names `{}` — a fingerprint dimension must not be \
             resolvable as a feature (INV-14, AT-68); register it as a windowed feature if it \
             is genuinely wanted",
            m.as_str()
        );
    }

    // And no feature registration anywhere names one.
    for (path, src) in rust_files() {
        if !path.starts_with("crates/features/") && !path.starts_with("crates/dataplane/") {
            continue;
        }
        let code = strip_comments(&src);
        for pattern in ["\"tier1_", "\"tier2_", "\"tier3_", "\"retrieval_vec", "\"embedding_dim"] {
            assert!(
                !code.contains(pattern),
                "{path} registers {pattern} as a feature name (AT-68)"
            );
        }
    }
}

/// SPEC §16.1 · No metric name embeds an instrument id (plan 5.2).
///
/// A metric named `sharpe.BTC-USD` is a new time series per instrument, and a
/// platform that trades a few hundred of them has a few hundred times the
/// cardinality it planned for. The cost is not the storage — it is that the
/// metrics backend degrades, then the dashboards degrade, then nobody looks at
/// them, and the observability layer quietly stops existing.
///
/// Per-instrument results go in **artifacts** instead, which is also what Gate
/// 12 reads for its concentration check: a Parquet file with an instrument
/// column, not a metric per instrument.
#[test]
fn no_metric_name_embeds_an_instrument_id() {
    // A metric name with an interpolated segment, or with something that looks
    // like a ticker in it.
    let interpolated = Regex::new(
        r"(?:counter|gauge|histogram|metric|record_metric|observe|increment)!?\s*\(\s*(?:&?format!\(|[a-z_]*&?format!\()",
    )
    .unwrap();
    let ticker_in_name = Regex::new(r#""[a-z_.]*\{instrument[a-z_]*\}[a-z_.]*""#).unwrap();

    let mut hits = Vec::new();
    for (path, src) in rust_files() {
        if path.starts_with("crates/invariants/") {
            continue;
        }
        let code = strip_comments(&src);
        if interpolated.is_match(&code) || ticker_in_name.is_match(&code) {
            hits.push(path);
        }
    }
    assert!(
        hits.is_empty(),
        "a metric name is built by interpolation, which is how an instrument id ends up in one \
         (§16.1); emit per-instrument results as an artifact instead: {hits:?}"
    );
}

/// SPEC §14.5 · No public benchmark score gates a release (checklist 4.17,
/// ADR-P4-04).
///
/// AGENT-004's private suite **is** the eval harness; the one rule added on top
/// is that nothing in the release path may read a public benchmark. The reason
/// is narrow and well established: public benchmarks leak into training corpora,
/// so a score on one measures contamination as much as capability — and a
/// release gate that moves with contamination is a gate that opens by itself
/// over time.
///
/// The non-inferiority gate against the private suite is the only gate.
#[test]
fn no_public_benchmark_gates_a_release() {
    let public = Regex::new(
        r"(?i)\b(mmlu|humaneval|gsm8k|hellaswag|truthfulqa|arc_challenge|bigbench|swe_?bench|mbpp)\b",
    )
    .unwrap();
    let mut hits = Vec::new();
    for (path, src) in files(&["crates", "apps", "evals"], &["rs", "py", "yaml", "yml", "json"]) {
        if path.starts_with("crates/invariants/") {
            continue;
        }
        let code = strip_comments(&src);
        if let Some(m) = public.find(&code) {
            hits.push(format!("{path}: {}", m.as_str()));
        }
    }
    assert!(
        hits.is_empty(),
        "a public benchmark appears in the release path. Public benchmarks leak into training \
         corpora, so their scores measure contamination as much as capability and a gate on one \
         opens by itself over time. The private suite's non-inferiority gate is the only gate \
         (§14.5, ADR-P4-04): {hits:?}"
    );
}

/// No CHECK constraint uses `array_length` to require a non-empty array.
///
/// `array_length(x, 1)` returns **NULL** for an empty array, `NULL >= 1` is
/// NULL, and a CHECK that evaluates to NULL *passes*. So the obvious spelling of
/// "this array must not be empty" accepts an empty one — silently, and in
/// exactly the constraints the pack calls REQUIRED: the leakage suite's checks,
/// a gate profile's factor battery, an insight's evidence, the seed holdout's
/// trials.
///
/// Four of those were live in this repository and were found by AT-42's live
/// test rather than by reading the DDL, which is the whole argument for this
/// scan: the bug is invisible at review because the SQL says what it means.
/// `cardinality(x)` returns 0 for an empty array and is the correct spelling.
#[test]
fn no_check_requires_a_non_empty_array_with_array_length() {
    let bad = Regex::new(r"(?is)CHECK\s*\([^)]*\barray_length\s*\(").unwrap();
    let mut hits = Vec::new();
    for (path, src) in ddl() {
        let code = strip_comments(&src);
        for m in bad.find_iter(&code) {
            let line = code[..m.start()].matches('\n').count() + 1;
            hits.push(format!("{path}:{line}"));
        }
    }
    assert!(
        hits.is_empty(),
        "`array_length(x, 1)` in a CHECK is NULL for an empty array, and a NULL CHECK passes — \
         use `cardinality(x)`, which is 0: {hits:?}"
    );
}

// ── AT-64b · Approval envelopes name actions that exist ──────────────────────
//
// `Envelope::for_action` matches on a *name*, and both drivers hand it the tool
// name they are about to dispatch (`api/src/agent/driver.rs:684`,
// `harness/src/drive.rs:1564`). A gated name that no tool answers to therefore
// gates nothing, silently, in the safe-looking direction: the list still reads as
// though promotion is covered.
//
// That was live until 2026-09-15. `PROMOTION_ACTIONS` said `promote_model`; the
// catalogue's tool has always been `promote_model_version`, so the one action
// AGENT-001 §15 says needs a human ran without one on both drivers.
//
// Set S task S-0.2 — AGENT-006 §2.6.

/// Gated actions that are deliberately **not** catalogue tools.
///
/// The envelope lists gate *platform actions*, and most of them are reached
/// through a route or a job rather than through an MCP tool. Each one is named
/// here with the reason, so that adding a gated action is a choice between
/// naming a real tool and writing down why there isn't one — and so that a typo
/// in a tool name cannot hide among them.
const NON_TOOL_GATED_ACTIONS: &[(&str, &str)] = &[
    ("place_order", "execution route /api/orders; no MCP tool places orders"),
    ("start_campaign", "campaign dispatcher; Set S phase 10 may add a tool"),
    ("submit_job", "job service admission, crates/jobs"),
    ("promote_strategy", "capital path; no tool exists yet (Set T)"),
    ("raise_capital_fraction", "ramp mechanics; Set T phase 3"),
    ("evaluate_sealed_holdout", "holdout path, crates/ledger; Set S phase 3 tool pending"),
    ("open_vault", "holdout path, crates/ledger; Set S phase 3 tool pending"),
    ("write_insight", "tier-3 memory writer; Set S phase 4"),
    ("record_insight", "tier-3 memory writer; Set S phase 4"),
    ("update_playbook", "tier-3 memory writer; Set S phase 4"),
];

/// The check itself, over supplied inputs, so the adversarial test below can run
/// it against a deliberately broken catalogue and assert it *rejects*.
///
/// A test that only asserts the real lists are fine proves the lists are fine
/// today; it does not prove the check would catch tomorrow's typo.
fn envelope_name_problems(
    gated: &[&str],
    catalogue: &BTreeSet<String>,
    non_tools: &[(&str, &str)],
) -> Vec<String> {
    let allowed: BTreeSet<&str> = non_tools.iter().map(|(n, _)| *n).collect();
    let mut problems = Vec::new();
    for action in gated {
        let is_tool = catalogue.contains(*action);
        let is_allowed = allowed.contains(action);
        if !is_tool && !is_allowed {
            problems.push(format!(
                "gated action `{action}` is neither a catalogue tool nor listed in \
                 NON_TOOL_GATED_ACTIONS — it gates nothing"
            ));
        }
        if is_tool && is_allowed {
            problems.push(format!(
                "`{action}` is now a catalogue tool and must be removed from \
                 NON_TOOL_GATED_ACTIONS, which claims no tool has that name"
            ));
        }
    }
    problems
}

fn catalogue_names() -> BTreeSet<String> {
    let defs = mcp_server_lib::tool_definitions();
    let names: BTreeSet<String> = defs
        .as_array()
        .expect("tool definitions are an array")
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()).map(str::to_owned))
        .collect();
    assert!(!names.is_empty(), "the catalogue is empty; the check would pass vacuously");
    names
}

#[test]
fn every_envelope_entry_is_a_real_action() {
    let gated = ledger::audit::gated_actions();
    let problems = envelope_name_problems(&gated, &catalogue_names(), NON_TOOL_GATED_ACTIONS);
    assert!(problems.is_empty(), "{problems:#?}");
}

/// Adversarial: the check rejects the two ways the list can drift.
///
/// Without this, `every_envelope_entry_is_a_real_action` could be a function that
/// returns an empty vector and nobody would know.
#[test]
fn the_envelope_name_check_rejects_a_name_no_tool_answers_to() {
    let catalogue: BTreeSet<String> =
        ["promote_model_version", "arm_automation"].into_iter().map(str::to_owned).collect();

    // 1. The exact defect that was live: a plausible-looking name, misspelled.
    let problems = envelope_name_problems(&["promote_model"], &catalogue, &[]);
    assert_eq!(problems.len(), 1, "a name no tool answers to must be rejected: {problems:?}");
    assert!(problems[0].contains("promote_model"));

    // 2. A stale exemption: the tool now exists, so the "not a tool" claim is a lie.
    let problems =
        envelope_name_problems(&["arm_automation"], &catalogue, &[("arm_automation", "stale")]);
    assert_eq!(problems.len(), 1, "a stale exemption must be rejected: {problems:?}");
    assert!(problems[0].contains("must be removed"));

    // 3. And it passes what it should: a real tool, and a declared non-tool.
    let problems = envelope_name_problems(
        &["promote_model_version", "place_order"],
        &catalogue,
        &[("place_order", "execution route")],
    );
    assert!(problems.is_empty(), "{problems:?}");
}

/// The other direction of AGENT-006 §2.6: every tool that *declares* an envelope
/// is in the list `Envelope::for_action` matches on, and every list entry that
/// names a tool has that tool declaring the same envelope.
///
/// `every_envelope_entry_is_a_real_action` above stops a gated name from naming
/// nothing. This stops the mirror failure: a tool that says it needs approval and
/// is in no list, which is ungated with a comment claiming otherwise. Set S task
/// S-1.1 — it needs `ToolSpec.envelope`, which did not exist when S-0.2 landed.
#[test]
fn every_tool_envelope_matches_the_ledger() {
    use ledger::audit::Envelope as Led;
    use mcp_server_lib::spec::Envelope as Dec;

    let mut problems = Vec::new();
    for spec in mcp_server_lib::spec::catalog() {
        let ledger_says = Led::for_action(spec.name);
        let agree = matches!(
            (spec.envelope, ledger_says),
            (Dec::None, Led::None)
                | (Dec::Spend, Led::Spend)
                | (Dec::Promotion, Led::Promotion)
                | (Dec::SealedHoldout, Led::SealedHoldout)
                | (Dec::Tier3Memory, Led::Tier3Memory)
        );
        if !agree {
            problems.push(format!(
                "`{}` declares envelope `{}` but ledger::audit gates it as {ledger_says:?} — \
                 one of the two is a lie about whether a human sees this call",
                spec.name,
                spec.envelope.as_str()
            ));
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");

    // And the declaration is not vacuous: at least one tool is actually gated.
    // Without this the test passes on a catalogue where nothing declares an
    // envelope and nothing is listed, which is exactly the state it exists to
    // make impossible to reach quietly.
    let mut gated: Vec<&str> = mcp_server_lib::spec::catalog()
        .iter()
        .filter(|s| s.envelope.requires_approval())
        .map(|s| s.name)
        .collect();
    gated.sort_unstable();
    assert_eq!(
        gated,
        vec!["arm_automation", "promote_model_version"],
        "the gated catalogue tools changed; that is a decision, not a diff"
    );
}

/// Adversarial: the old spelling gates nothing, and the new one gates promotion.
///
/// Asserting the fix works is half a test. This asserts the broken spelling is
/// *not* quietly still accepted by some fallback.
#[test]
fn the_old_promotion_spelling_matches_no_envelope() {
    use ledger::audit::Envelope;
    assert_eq!(
        Envelope::for_action("promote_model"),
        Envelope::None,
        "`promote_model` must match nothing: it is not a tool, and keeping it would \
         restore the illusion the real tool is gated"
    );
    assert_eq!(Envelope::for_action("promote_model_version"), Envelope::Promotion);
    assert!(Envelope::for_action("promote_model_version").requires_approval());
}

/// The one module allowed to name the smoothed regime path, and the reason it
/// is allowed: it is analysis, it writes its answer into the research schema,
/// and nothing that evaluates holds USAGE on that schema.
const REGIME_ANALYSIS_MODULE: &str = "crates/api/src/regime_separation.rs";

/// Everything before a file's `#[cfg(test)]`, which is the code that ships.
///
/// A file with no test module is returned whole.
fn production_only(src: &str) -> &str {
    match src.find("#[cfg(test)]") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// Everything a strategy's result passes through on its way to a verdict.
fn evaluating_path(path: &str) -> bool {
    path.starts_with("crates/backtest/")
        || path.starts_with("crates/strategy-runtime/")
        || path.starts_with("crates/strategy-validator/")
        || path.starts_with("crates/features/")
        || path.starts_with("crates/risk/")
        || path.starts_with("crates/execution/")
        || path == "crates/api/src/gate_worker.rs"
        || path == "crates/api/src/knowledge.rs"
}

/// AT-42 ⛔ (static half) · Nothing on an evaluating path names the smoothed
/// regime path (SPEC §5.4, R-07, ADR-P3-02, ADR-P3-09).
///
/// The GRANT is the enforcement and AT-42's live half checks it. This is the
/// half that fails at build time rather than at runtime, and it exists because
/// the runtime failure arrives as a permission error inside a backtest — which
/// somebody will "fix" by widening a grant.
#[test]
fn at42_no_evaluating_module_names_the_smoothed_regime_path() {
    let forbidden =
        Regex::new(r"(?i)(regime_research|p_smoothed|regime_state_smoothed|viterbi)").unwrap();

    let mut hits = Vec::new();
    for (path, src) in rust_files() {
        if !evaluating_path(&path) {
            continue;
        }
        assert_ne!(
            path, REGIME_ANALYSIS_MODULE,
            "the analysis module must not be on the evaluating list"
        );
        // Production code only. A test that asserts a module offers no smoothed
        // read has to name the thing it forbids — `knowledge.rs` carries exactly
        // such a test, and its own comment says so. Scanning the test module
        // would make the stronger, file-local version of this rule the reason
        // the weaker repo-wide one fails, which is the wrong way round.
        let code = strip_comments(production_only(&src));
        if let Some(m) = forbidden.find(&code) {
            let line = code[..m.start()].matches('\n').count() + 1;
            hits.push(format!("{path}:{line} names `{}`", m.as_str()));
        }
    }
    assert!(
        hits.is_empty(),
        "an evaluating path names the smoothed regime state. Smoothed probabilities are \
         P(state | all data) and are worth about 2.2x Sharpe to anything that sees them \
         (SPEC §5.4); the separation between them and a backtest is a schema grant, and naming \
         the schema here is the first step to widening it: {hits:?}"
    );

    // And the allowance is a statement about one file rather than a hole: no
    // other module in the tree names the smoothed path at all.
    let named: Vec<String> = rust_files()
        .into_iter()
        // Production code, for the same reason as above: `knowledge.rs`
        // names the schema only inside the test that forbids it, and a
        // module proving it has no smoothed read must not be counted as
        // having one.
        .filter(|(p, s)| {
            p.starts_with("crates/") && forbidden.is_match(&strip_comments(production_only(s)))
        })
        .map(|(p, _)| p)
        .filter(|p| !p.contains("/tests/"))
        .collect();
    assert!(
        named.iter().all(|p| p == REGIME_ANALYSIS_MODULE),
        "exactly one module may name the smoothed path, and it is the analysis one: {named:?}"
    );
}

/// The separation test's own result is not reachable from an evaluating path
/// either (ADR-P3-09).
///
/// It is a full-sample statement about which regime paid. A strategy that could
/// read it would know which market state to lean into before the state was
/// knowable — the same lookahead as a smoothed probability, in a friendlier
/// shape. It lives in `regime_research`, which the scan above already forbids
/// naming; this adds the table's own name, for the day somebody grants it out.
#[test]
fn the_separation_result_is_not_named_on_an_evaluating_path() {
    let forbidden = Regex::new(r"regime_separation").unwrap();
    let hits: Vec<String> = rust_files()
        .into_iter()
        .filter(|(p, _)| evaluating_path(p))
        .filter(|(_, s)| forbidden.is_match(&strip_comments(s)))
        .map(|(p, _)| p)
        .collect();
    assert!(
        hits.is_empty(),
        "the separation test's result must not be reachable from anything that evaluates: {hits:?}"
    );
}

/// The separation test's significance level is pre-registered and cannot be
/// supplied (SPEC §5.4).
///
/// A test whose alpha is a caller's parameter is a test that gets re-run with a
/// bigger alpha, and the second run is the one that gets recorded.
#[test]
fn the_separation_tests_alpha_cannot_be_supplied() {
    let src = fs::read_to_string(root().join(REGIME_ANALYSIS_MODULE))
        .expect("the separation module exists");
    let production = src.split("#[cfg(test)]").next().unwrap_or_default();
    let code = strip_comments(production);
    assert!(
        code.contains("pub const ALPHA: f64 = 0.05;"),
        "alpha must stay a pre-registered constant"
    );
    for sig in code.split("fn ").skip(1) {
        let params = sig.split(')').next().unwrap_or_default();
        assert!(
            !params.contains("alpha"),
            "alpha must not be a parameter: fn {}",
            sig.lines().next().unwrap_or_default()
        );
    }
}

/// A market scope arrives with its reasoning or it does not arrive
/// (ADR-P3-07, ADR-P3-08).
///
/// ADR-P3-07 made scope configuration a decision rather than a default, on the
/// grounds that "the market" means a different series for crypto, equities and
/// futures. A decision with no recorded reason is a default with extra steps, so
/// `rationale` is NOT NULL under a length CHECK — in the DDL, where an INSERT
/// that omits it fails, rather than in a convention somebody follows.
#[test]
fn a_regime_scope_cannot_be_configured_without_its_reasoning() {
    let ddl = ddl()
        .into_iter()
        .find(|(p, _)| p.ends_with("0117_regime_scopes_and_separation.sql"))
        .map(|(_, s)| s)
        .expect("migration 0117 exists");
    let code = strip_comments(&ddl);
    assert!(code.contains("rationale              TEXT NOT NULL"), "rationale must be NOT NULL");
    assert!(
        Regex::new(r"(?is)CHECK\s*\(\s*length\s*\(\s*btrim\s*\(\s*rationale").unwrap().is_match(&code),
        "an empty rationale must be refused by the database, not by review"
    );
    assert!(
        !Regex::new(r"(?i)rationale[^,]*DEFAULT").unwrap().is_match(&code),
        "a default rationale is the default ADR-P3-07 refused, wearing a different hat"
    );
}
