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
        let in_scope = path.ends_with("0046_dataplane.sql") || path.ends_with("07_canonical_market.sql");
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
        let n = calls.find_iter(&strip_comments(&src)).count();
        if n > 0 {
            assert_eq!(path, "crates/mcp-server/src/lib.rs", "dispatch_untraced referenced outside the MCP library");
        }
        total += n;
    }
    assert_eq!(total, 2, "one definition and one call, from dispatch_tool");
    let wrapper = &lib[lib.find("pub async fn dispatch_tool(").unwrap()..lib.find("async fn dispatch_untraced(").unwrap()];
    assert!(wrapper.contains("record_step"), "dispatch_tool must record the step");
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

/// AT-64 · Denials are provable: the `pre` record is written before the policy
/// check (SPEC §15, ADR-P2-21).
///
/// The property is an *ordering*, and ordering is the kind of thing a refactor
/// reverses without anyone noticing, so it is pinned in the source rather than
/// left to a convention. `bridge::execute` is the single function every internal
/// agent tool call passes through; this asserts the `pre` write appears before
/// the dispatch in it, that the `post` write appears after, and that nothing
/// else in the tree dispatches around it.
#[test]
fn at64_the_audit_pre_record_precedes_the_dispatch() {
    let src = fs::read_to_string(root().join("crates/api/src/agent/bridge.rs"))
        .expect("the bridge exists");
    let start = src.find("pub async fn execute(").expect("the dispatch chokepoint exists");
    let body = &src[start..];
    let end = body.find("\nfn compact(").unwrap_or(body.len());
    let body = strip_comments(&body[..end]);

    let pre = body.find(".pre(").expect("execute must write the audit pre-record");
    let dispatch = body.find("dispatch_tool(").expect("execute must dispatch");
    let post = body.find(".post(").expect("execute must write the audit post-record");
    assert!(
        pre < dispatch,
        "the audit pre-record must be written BEFORE the dispatch: a denial that is not \
         recorded first leaves no evidence the control did anything"
    );
    assert!(post > dispatch, "the post-record must follow the dispatch");

    // The SDK driver dispatches directly rather than through the bridge, so it
    // opens its own record — in the same order.
    let driver = fs::read_to_string(root().join("crates/api/src/agent/driver.rs")).unwrap();
    let driver = strip_comments(&driver);
    let d_pre = driver.find("audit_trail.pre(").expect("the SDK driver writes a pre-record");
    let d_dispatch = driver.find("dispatch_tool(&mcp_ctx").expect("the SDK driver dispatches");
    let d_post = driver.find("audit_trail.post(").expect("the SDK driver writes a post-record");
    assert!(d_pre < d_dispatch && d_post > d_dispatch, "the SDK driver's audit order is reversed");

    // Nothing else routes around them. The external MCP front door is a separate
    // process holding a service token and is **not** audited by this trail — a
    // gap, reported in decisions/PROGRESS.md rather than hidden by widening this
    // list quietly.
    let mut callers: Vec<String> = Vec::new();
    for (path, file) in rust_files() {
        if path.starts_with("crates/invariants/") {
            continue;
        }
        if strip_comments(&file).contains("dispatch_tool(") {
            callers.push(path);
        }
    }
    callers.sort();
    assert_eq!(
        callers,
        vec![
            "apps/mcp-server/src/main.rs".to_string(),
            "crates/api/src/agent/bridge.rs".to_string(),
            "crates/api/src/agent/driver.rs".to_string(),
            "crates/mcp-server/src/lib.rs".to_string(),
            "crates/mcp-server/tests/integration.rs".to_string(),
        ],
        "a new dispatch path appeared; audit it or record why it is exempt"
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
    assert!(
        ledger::audit::Envelope::for_action("promote_model").requires_approval(),
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
