//! The templated tier: a tool that is a Python file plus a header.
//!
//! AGENT-006 §2.2, §3.3, §10.5; Set S tasks S-3.1, S-3.2.
//!
//! # Why this tier exists
//!
//! A 7B and a frontier model must get the same statistics, and they will not if
//! the statistics are in the prompt. `describe_returns` returning a Jarque-Bera
//! p-value, a Ljung-Box statistic on `r` and `r²`, an ARCH-LM p-value and a Hill
//! tail index is the same answer on every tier, because the model's contribution
//! is the *call*, not the arithmetic (decision D-S9). That is also why templated
//! tools stay exposed on `code_execution: denied` profiles: the harness runs the
//! template, the model does not. Only free-form `run_python` is hidden there.
//!
//! # The contract
//!
//! - **Location.** `apps/skill-runner/templates/<namespace>/<name>.py`. The path
//!   *is* the namespace and the name; `build.rs` fails the build if the header
//!   disagrees with it.
//! - **Header.** A fenced `json` block in the module docstring carrying
//!   `{name, namespace, risk, description, input_schema, output, evidence,
//!   limits}`. There is no second declaration in Rust — see `build.rs` for why
//!   two copies of a tool's identity is the defect shape this platform already
//!   paid for once.
//! - **Data.** A skill has no network and no database, so the harness
//!   materialises the series into a parquet file and passes
//!   `payload["data_path"]`. [`data_key`] is the content address of that file:
//!   the same `(instrument, timeframe, start, end)` hashes to the same path and
//!   is written once.
//! - **Determinism.** Every template takes `seed` and prints the numpy / pandas /
//!   scipy / statsmodels versions it used into `detail.provenance`, so two runs
//!   that disagree can be told apart from two libraries that disagree.
//! - **Limits.** The runner's 30 s default; a template may declare up to its
//!   namespace's maximum ([`max_wall_seconds`]) — 120 s for `analysis` and
//!   `falsification`, 300 s for `audit` and `attribution`, where block bootstraps
//!   are genuinely slow.
//!
//! # What is *not* built here
//!
//! Execution. `crates/skills` and `apps/skill-runner`'s runner are not in this
//! worktree's tree (roadmap gate **H-1**: they are uncommitted in the user's
//! checkout and are not workspace members). [`run`] therefore dispatches through
//! the platform API like every other tool in this crate (ADR-0010) and will
//! start working the moment `POST /api/skills/templates/run` exists. The
//! declaration half — catalogue entry, namespace, risk, schema, budget, trial
//! rule — is complete and enforced today.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::spec::{Envelope, OutputBudget, Tier, ToolSpec, TrialKind};
use crate::taxonomy::Risk;
use crate::ApiClient;

include!(concat!(env!("OUT_DIR"), "/templates.rs"));

/// The runner's own defaults (`crates/skills/src/runner.rs`), restated here so a
/// template that declares nothing still has a bound.
pub const DEFAULT_WALL_SECONDS: u32 = 30;

/// The most wall time a namespace's templates may ask for (§3.3).
///
/// `audit` and `attribution` get 300 s because a Romano-Wolf step-down over five
/// thousand block-bootstrap resamples is not a 30-second computation and
/// pretending otherwise would push the work back into the prompt, which is the
/// one place it must not be.
#[must_use]
pub fn max_wall_seconds(namespace: &str) -> u32 {
    match namespace {
        "audit" | "attribution" => 300,
        "analysis" | "falsification" | "model" | "portfolio" => 120,
        _ => DEFAULT_WALL_SECONDS,
    }
}

/// Keys the harness supplies to every template, which a header therefore need not
/// declare and a body may read freely.
pub const HARNESS_SUPPLIED: &[&str] = &["data_path", "data_paths", "run_id", "work_dir"];

// ── The header ───────────────────────────────────────────────────────────────

/// The fenced JSON in a template's module docstring.
#[derive(Debug, Deserialize)]
pub struct TemplateHeader {
    pub name: String,
    pub namespace: String,
    pub risk: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub output: HeaderOutput,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub limits: HeaderLimits,
    /// `"none" | "search" | "falsification"`. Defaults to none: a statistic over
    /// a materialised series runs no simulator.
    #[serde(default)]
    pub spends_trial: Option<String>,
    #[serde(default)]
    pub notes_section: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HeaderOutput {
    #[serde(default)]
    pub summary_template: Option<String>,
    /// `[[field, unit], …]` — JSON has no tuples, so pairs are two-element
    /// arrays.
    #[serde(default)]
    pub detail_fields: Vec<(String, String)>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HeaderLimits {
    #[serde(default)]
    pub wall_seconds: Option<u32>,
}

/// One installed template: its path, its parsed header and its source.
pub struct Template {
    pub path: &'static str,
    pub header: TemplateHeader,
    pub source: &'static str,
}

/// Every installed template, parsed once.
///
/// Panics at first use on a malformed header, for the same reason
/// `spec::assert_no_duplicates` panics: every caller downstream — routing,
/// exposure, gating — would otherwise be reading a catalogue that silently lost a
/// tool.
pub fn templates() -> &'static [Template] {
    static PARSED: OnceLock<Vec<Template>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TEMPLATE_FILES
            .iter()
            .map(|(path, header, source)| {
                let header: TemplateHeader = serde_json::from_str(header).unwrap_or_else(|e| {
                    panic!("template {path}: its ```json header does not parse: {e}")
                });
                Template {
                    path,
                    header,
                    source,
                }
            })
            .collect()
    })
}

/// The catalogue entries for the templated tier (§2.3).
///
/// `&'static str` is what [`ToolSpec`] holds and a header yields `String`, so the
/// strings are leaked — once, at first use, bounded by the number of installed
/// templates. That is the only concession this tier makes to the type.
pub fn specs() -> Vec<ToolSpec> {
    templates()
        .iter()
        .map(|t| {
            let h = &t.header;
            let mut spec = ToolSpec::new(
                leak(&h.name),
                leak(&h.namespace),
                parse_risk(&h.risk, t.path),
                leak(&h.description),
                h.input_schema.clone(),
            )
            .internal()
            .tier(Tier::Templated {
                template: t.path,
            })
            .spends_trial(parse_trial(h.spends_trial.as_deref(), t.path))
            .evidence(leak_slice(&h.evidence));
            if let Some(template) = &h.output.summary_template {
                spec = spec.summary(
                    leak(template),
                    leak_pairs(&h.output.detail_fields),
                );
            }
            if let Some(section) = &h.notes_section {
                spec = spec.notes(leak(section));
            }
            spec.envelope(Envelope::None)
        })
        .collect()
}

fn parse_risk(s: &str, path: &str) -> Risk {
    match s {
        "read" => Risk::Read,
        "write" => Risk::Write,
        "destructive" => Risk::Destructive,
        "outbound" => Risk::Outbound,
        other => panic!("template {path}: risk `{other}` is not one of read/write/destructive/outbound"),
    }
}

fn parse_trial(s: Option<&str>, path: &str) -> TrialKind {
    match s {
        None | Some("none") => TrialKind::None,
        Some("search") => TrialKind::Search,
        Some("falsification") => TrialKind::Falsification,
        Some(other) => panic!("template {path}: spends_trial `{other}` is not none/search/falsification"),
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn leak_slice(v: &[String]) -> &'static [&'static str] {
    Box::leak(v.iter().map(|s| leak(s)).collect::<Vec<_>>().into_boxed_slice())
}

fn leak_pairs(v: &[(String, String)]) -> &'static [(&'static str, &'static str)] {
    Box::leak(
        v.iter()
            .map(|(a, b)| (leak(a), leak(b)))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

// ── Data materialisation (S-3.2) ─────────────────────────────────────────────

/// The content address of a materialised series.
///
/// The same `(instrument, timeframe, start, end)` yields the same key, so the
/// harness writes the parquet once and every later call reads it. Content
/// addressing rather than a counter is what makes that true across processes and
/// across restarts — a per-run temp name would materialise the same window
/// repeatedly and, worse, would make two tools' `data_path` differ for data that
/// is byte-identical.
///
/// Order-independent by construction: the fields are named, not positional, and
/// hashed through the platform's canonical-JSON hash, the same one the job
/// service uses for submission dedup (§2.4b).
#[must_use]
pub fn data_key(instrument: &str, timeframe: &str, start: &str, end: &str) -> String {
    let canonical = json!({
        "instrument": instrument,
        "timeframe": timeframe,
        "start": start,
        "end": end,
        "v": 1,
    });
    let hash = dataplane::content_hash(&canonical).unwrap_or_default();
    let short: String = hash
        .strip_prefix("sha256:")
        .unwrap_or(&hash)
        .chars()
        .take(32)
        .collect();
    format!("data/{short}.parquet")
}

// ── Dispatch ─────────────────────────────────────────────────────────────────

/// Run a template through the platform's skill runner.
///
/// The `seed` is forwarded verbatim and defaults to 0 rather than to a clock: a
/// template whose seed is the time of day cannot produce byte-identical `detail`
/// twice, which is the determinism acceptance for this tier.
pub async fn run(api: &ApiClient, name: &str, params: &Value) -> Value {
    let Some(t) = templates().iter().find(|t| t.header.name == name) else {
        return json!({ "error": "unknown_tool", "tool": name });
    };
    let body = json!({
        "template": t.path,
        "arguments": params,
        "seed": params.get("seed").cloned().unwrap_or(json!(0)),
        "limits": { "wall_seconds": wall_seconds(t) },
    });
    match api.post("/api/skills/templates/run", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// The wall-clock bound this template actually runs under.
#[must_use]
pub fn wall_seconds(t: &Template) -> u32 {
    t.header
        .limits
        .wall_seconds
        .unwrap_or(DEFAULT_WALL_SECONDS)
        .min(max_wall_seconds(&t.header.namespace))
}

// ── The parameters a body reads (S-3.2) ──────────────────────────────────────

/// Every `payload["x"]` and `payload.get("x")` in a template's source.
///
/// This is the mechanical half of "the header schema equals the template's
/// schema". The header cannot drift from the *catalogue* — the catalogue is
/// generated from it — but it can absolutely drift from the *body*, and that
/// drift is invisible: a template that reads `payload["horizon_bars"]` where the
/// header declares `horizon` fails at run time with a `KeyError`, on a profile
/// where the model is least able to work out why.
#[must_use]
pub fn parameters_read(source: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for (open, close) in [("payload[\"", "\""), ("payload.get(\"", "\"")] {
        let mut rest = source;
        while let Some(at) = rest.find(open) {
            let after = &rest[at + open.len()..];
            if let Some(end) = after.find(close) {
                found.insert(after[..end].to_string());
                rest = &after[end..];
            } else {
                break;
            }
        }
    }
    found
}

/// The property names a header's `input_schema` declares.
#[must_use]
pub fn parameters_declared(header: &TemplateHeader) -> BTreeSet<String> {
    header
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

/// The property names a header marks `required`.
#[must_use]
pub fn parameters_required(header: &TemplateHeader) -> BTreeSet<String> {
    header
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── S-3.2: the header and the body agree ─────────────────────────────────

    /// The schema-agreement gate. Every parameter a template's body reads is one
    /// its header declares, and every parameter the header *requires* is one the
    /// body reads.
    ///
    /// The second direction matters as much as the first: a required parameter
    /// nothing reads is a question the model is forced to answer for no reason,
    /// and on a constrained tier every required field is a place decoding can go
    /// wrong.
    #[test]
    fn every_template_reads_exactly_the_parameters_its_header_declares() {
        let mut problems = Vec::new();
        for t in templates() {
            let declared = parameters_declared(&t.header);
            let required = parameters_required(&t.header);
            let read = parameters_read(t.source);
            let supplied: BTreeSet<String> =
                HARNESS_SUPPLIED.iter().map(ToString::to_string).collect();

            for p in read.difference(&declared) {
                if supplied.contains(p) {
                    continue;
                }
                problems.push(format!(
                    "{}: body reads `{p}`, which the header does not declare — this \
                     is a KeyError at run time on the tier least able to diagnose it",
                    t.path
                ));
            }
            for p in required.difference(&read) {
                problems.push(format!(
                    "{}: header requires `{p}`, which the body never reads — a \
                     required field nothing uses is a decoding hazard for nothing",
                    t.path
                ));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// Adversarial: the agreement check *rejects*. The real templates agreeing
    /// proves nothing about whether the comparison works.
    #[test]
    fn a_body_reading_an_undeclared_parameter_is_detected() {
        let header: TemplateHeader = serde_json::from_str(
            r#"{"name":"x","namespace":"analysis","risk":"read","description":"d",
                "input_schema":{"type":"object","properties":{"instrument":{}},
                "required":["instrument"]}}"#,
        )
        .unwrap();
        let body = r#"i = payload["instrument"]; h = payload["horizon_bars"]"#;
        let read = parameters_read(body);
        let declared = parameters_declared(&header);
        let undeclared: Vec<&String> = read.difference(&declared).collect();
        assert_eq!(undeclared, vec!["horizon_bars"]);
    }

    /// Adversarial, the other direction.
    #[test]
    fn a_required_parameter_the_body_ignores_is_detected() {
        let header: TemplateHeader = serde_json::from_str(
            r#"{"name":"x","namespace":"analysis","risk":"read","description":"d",
                "input_schema":{"type":"object","properties":{"instrument":{},"seed":{}},
                "required":["instrument","seed"]}}"#,
        )
        .unwrap();
        let read = parameters_read(r#"payload["instrument"]"#);
        let required = parameters_required(&header);
        let unused: Vec<&String> = required.difference(&read).collect();
        assert_eq!(unused, vec!["seed"]);
    }

    // ── S-3.1: limits, seeds, provenance ─────────────────────────────────────

    /// A template may not ask for more wall time than its namespace allows, and
    /// `wall_seconds` clamps rather than trusts.
    #[test]
    fn no_template_runs_longer_than_its_namespace_allows() {
        for t in templates() {
            let ceiling = max_wall_seconds(&t.header.namespace);
            assert!(
                wall_seconds(t) <= ceiling,
                "{} runs for {} s; `{}` allows {ceiling} s",
                t.path,
                wall_seconds(t),
                t.header.namespace
            );
            if let Some(declared) = t.header.limits.wall_seconds {
                assert!(
                    declared <= ceiling,
                    "{} declares {declared} s, over the {ceiling} s ceiling for \
                     `{}` — raise the ceiling deliberately or make the template \
                     cheaper",
                    t.path,
                    t.header.namespace
                );
            }
        }
    }

    /// Adversarial: an over-declaration is clamped, not honoured. A template that
    /// could ask for ten minutes would hold a runner slot against every other
    /// call in the run.
    #[test]
    fn a_template_asking_for_more_time_than_its_namespace_allows_is_clamped() {
        let header: TemplateHeader = serde_json::from_str(
            r#"{"name":"greedy","namespace":"analysis","risk":"read","description":"d",
                "input_schema":{"type":"object"},"limits":{"wall_seconds":600}}"#,
        )
        .unwrap();
        let t = Template {
            path: "analysis/greedy.py",
            header,
            source: "",
        };
        assert_eq!(wall_seconds(&t), 120, "clamped to the analysis ceiling");
    }

    /// Determinism (§3.3): every template takes a seed and reports the library
    /// versions it used. Without the second half, two runs that disagree cannot
    /// be told apart from two library versions that disagree.
    #[test]
    fn every_template_takes_a_seed_and_reports_its_provenance() {
        let mut problems = Vec::new();
        for t in templates() {
            if !parameters_declared(&t.header).contains("seed") {
                problems.push(format!("{}: no `seed` parameter", t.path));
            }
            if !t.source.contains("provenance") {
                problems.push(format!(
                    "{}: never writes `provenance`; a result whose library versions \
                     are unknown is not reproducible evidence",
                    t.path
                ));
            }
        }
        assert!(problems.is_empty(), "{problems:#?}");
    }

    /// The header is the only declaration, so a templated spec's namespace and
    /// risk come from it — and the same catalogue-wide rules apply to this tier
    /// as to any other.
    #[test]
    fn a_templated_spec_carries_the_same_contract_as_a_typed_one() {
        for s in specs() {
            assert!(
                matches!(s.tier, Tier::Templated { .. }),
                "{} is in templated::specs() and is not templated",
                s.name
            );
            assert!(
                !s.description.is_empty() && s.description.len() <= 1_200,
                "{}: description out of budget",
                s.name
            );
            assert!(
                s.input_schema.get("type").is_some(),
                "{}: input_schema has no type",
                s.name
            );
        }
    }

    // ── S-3.2: data materialisation ──────────────────────────────────────────

    /// The same window hashes to the same path, so it is materialised once.
    #[test]
    fn the_same_window_is_the_same_parquet_path() {
        let a = data_key("BTC-USD", "1h", "2024-01-01", "2024-06-01");
        let b = data_key("BTC-USD", "1h", "2024-01-01", "2024-06-01");
        assert_eq!(a, b);
        assert!(a.starts_with("data/") && a.ends_with(".parquet"), "{a}");
    }

    /// Adversarial: a different window is a different file. A key that collided
    /// would serve one instrument's bars to another instrument's analysis, and
    /// every number downstream would be confidently wrong.
    #[test]
    fn a_different_window_is_a_different_parquet_path() {
        let base = data_key("BTC-USD", "1h", "2024-01-01", "2024-06-01");
        let mut seen = BTreeSet::new();
        seen.insert(base.clone());
        for k in [
            data_key("ETH-USD", "1h", "2024-01-01", "2024-06-01"),
            data_key("BTC-USD", "1d", "2024-01-01", "2024-06-01"),
            data_key("BTC-USD", "1h", "2024-01-02", "2024-06-01"),
            data_key("BTC-USD", "1h", "2024-01-01", "2024-06-02"),
        ] {
            assert!(seen.insert(k.clone()), "`{k}` collides with another window");
        }
    }

    /// Adversarial: the field order in the key cannot be confused. `("BTC","1h")`
    /// and `("1h","BTC")` are different files.
    #[test]
    fn the_data_key_is_not_a_concatenation() {
        assert_ne!(
            data_key("BTC-USD", "1h", "a", "b"),
            data_key("1h", "BTC-USD", "a", "b")
        );
    }

    // ── The sandbox ──────────────────────────────────────────────────────────

    /// Adversarial: the network scan `build.rs` runs rejects the imports it is
    /// there to reject, and tolerates the words appearing in prose.
    ///
    /// The scan itself runs in `build.rs` and fails the build; this asserts the
    /// rule it applies, on fixtures, because a passing build proves only that no
    /// installed template violates it today.
    #[test]
    fn the_sandbox_scan_rejects_network_imports_and_tolerates_prose() {
        fn imports_network(source: &str) -> bool {
            const FORBIDDEN: &[&str] = &[
                "socket",
                "requests",
                "subprocess",
                "urllib",
                "http.client",
                "httpx",
                "ftplib",
                "smtplib",
                "sqlite3",
            ];
            source.lines().any(|line| {
                let t = line.trim();
                let module = if let Some(m) = t.strip_prefix("import ") {
                    m
                } else if let Some(m) = t.strip_prefix("from ") {
                    m.split_whitespace().next().unwrap_or("")
                } else {
                    return false;
                };
                let head = module
                    .split(|c: char| c == ' ' || c == ',' || c == '.')
                    .next()
                    .unwrap_or("");
                let dotted = module.split_whitespace().next().unwrap_or("");
                FORBIDDEN.iter().any(|b| head == *b || dotted == *b)
            })
        }
        assert!(imports_network("import socket"));
        assert!(imports_network("import requests"));
        assert!(imports_network("from urllib import request"));
        assert!(imports_network("import http.client"));
        assert!(imports_network("    import subprocess"));
        assert!(!imports_network("import numpy as np"));
        assert!(
            !imports_network("# this template never opens a socket or a subprocess"),
            "a comment is not an import; a scan that cannot tell them apart gets \
             disabled the first time it is wrong"
        );
        assert!(
            !imports_network(r#"raise SystemExit("no network: requests is unavailable")"#),
            "the word in a refusal message is not an import"
        );
    }

    /// Every installed template really is sandbox-clean. Belt and braces with
    /// `build.rs`, which would have failed the build — but this is the test that
    /// says so in a test report.
    #[test]
    fn no_installed_template_imports_the_network() {
        for t in templates() {
            for line in t.source.lines() {
                let trimmed = line.trim();
                for bad in ["import socket", "import requests", "import subprocess"] {
                    assert!(
                        !trimmed.starts_with(bad),
                        "{} has `{bad}`; a skill has no network",
                        t.path
                    );
                }
            }
        }
    }

    /// Names are unique across the tier, which `spec::assert_no_duplicates` would
    /// also catch — but only once the templated specs are folded into the
    /// catalogue, and by then the panic names indices rather than files.
    #[test]
    fn template_names_are_unique() {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for t in templates() {
            if let Some(first) = seen.insert(&t.header.name, t.path) {
                panic!("`{}` is declared by {first} and {}", t.header.name, t.path);
            }
        }
    }
}
