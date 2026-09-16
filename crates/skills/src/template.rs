//! A tool that is a Python file (AGENT-006 §2.2, §3.3; Set S tasks S-3.1, S-3.2).
//!
//! A *template* is a platform-authored Python file whose module docstring carries
//! a fenced JSON header. The header is the tool's declaration — name, namespace,
//! schema, summary template, limits — and the body is its implementation. One
//! file, so the two cannot be edited apart.
//!
//! # Why a template and not a Rust tool
//!
//! Sixty statistical tools written in Rust is sixty ports of code that already
//! exists in numpy and scipy, each one a place the port can differ from the
//! reference it was ported from. More importantly, templated tools are exposed
//! **even to profiles with `code_execution: denied`**: the harness runs the
//! template, the model does not. That is what makes a 7B get the same deflated
//! Sharpe a frontier model gets, rather than an arithmetic it invented.
//!
//! # Why a template and not an agent-authored skill
//!
//! A skill is whatever the agent wrote and is evidence about the agent. A
//! template ships with the platform, is reviewed, is versioned in the catalogue,
//! and its results are citeable. They share the runner and nothing else.
//!
//! # The parser lives here and only here
//!
//! `crates/mcp-server` builds its catalogue from these headers and `crates/api`
//! runs the bodies. If each parsed the file its own way, a template could declare
//! one schema to the model and be run under another — the failure would look like
//! a model calling a tool wrongly. So there is one parser, in the crate both
//! already depend on, rather than a copy and a reconciliation test.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::runner::Limits;

/// Shared helpers prepended to every template body before it runs.
///
/// Templates cannot `import` a local module: the runner receives one source
/// string on stdin and has no import path into the workspace (deliberately — it
/// shares no filesystem with us beyond the one materialised data file). So the
/// helpers are *concatenated* ahead of the body.
///
/// The cost is that a traceback's line numbers are offset by the preamble's
/// length. [`preamble_lines`] is that offset, and [`Template::source_line`] maps
/// a reported line back to the file the author edited.
pub const PREAMBLE: &str = include_str!("../../../apps/skill-runner/preamble.py");

/// How many lines [`PREAMBLE`] occupies once concatenated.
#[must_use]
pub fn preamble_lines() -> usize {
    PREAMBLE.lines().count() + 1
}

/// How long a namespace's templates may ask to run for.
///
/// The runner's default is 30 s, which is right for a transformation and wrong
/// for a block bootstrap over ten years of minute bars. Rather than let any
/// template declare any number, the ceiling is per namespace and stated here:
/// `analysis` fits in two minutes, `audit` and `attribution` bootstrap and need
/// five, and everything else has not earned an exception.
///
/// A template asking for more than its namespace allows is a parse error, not a
/// silent clamp — a template written expecting 300 s and clamped to 30 s does not
/// fail, it produces a `killed` outcome that reads like a bug in the statistics.
const WALL_CEILING: &[(&str, u64)] = &[("analysis", 120), ("audit", 300), ("attribution", 300)];

/// The runner's default, for namespaces not named above.
const WALL_DEFAULT: u64 = 30;

#[must_use]
fn wall_ceiling(namespace: &str) -> u64 {
    WALL_CEILING
        .iter()
        .find(|(ns, _)| *ns == namespace)
        .map_or(WALL_DEFAULT, |(_, s)| *s)
}

// ── The header ───────────────────────────────────────────────────────────────

/// What series a template needs materialised before it runs (§3.3).
///
/// A template has no network and no database — `taxonomy.rs` asserts the first
/// and the runner's blocked socket layer explains it — so every row it reads
/// arrives as a parquet file at `payload["data"][<role>]`. The header says which
/// series; the harness fetches them.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SeriesRequest {
    /// The key under `payload["data"]` the file lands at.
    pub role: String,
    /// Which arguments name the instrument and timeframe. Defaults to the
    /// obvious ones so the common case declares only a role.
    #[serde(default = "default_instrument_arg")]
    pub instrument_arg: String,
    #[serde(default = "default_timeframe_arg")]
    pub timeframe_arg: String,
}

fn default_instrument_arg() -> String {
    "instrument_id".into()
}
fn default_timeframe_arg() -> String {
    "timeframe".into()
}

/// The fenced JSON block in a template's module docstring.
///
/// Every field that has a conservative reading has that reading as its default:
/// no trial, no envelope, no evidence, read risk. A template that spends a trial
/// or reaches a sealed holdout has to say so.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateHeader {
    pub name: String,
    #[serde(default = "one")]
    pub version: u32,
    pub namespace: String,
    #[serde(default = "read")]
    pub risk: String,
    pub description: String,
    pub input_schema: Value,
    #[serde(default)]
    pub summary_template: Option<String>,
    /// `[field, unit]` pairs. A JSON array of two-element arrays rather than an
    /// object, because the order is the order they are rendered in.
    #[serde(default)]
    pub detail_fields: Vec<(String, String)>,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// `"none" | "search" | "falsification"`.
    #[serde(default = "none")]
    pub spends_trial: String,
    /// `"none" | "spend" | "promotion" | "sealed_holdout" | "tier3_memory"`.
    #[serde(default = "none")]
    pub envelope: String,
    #[serde(default)]
    pub max_calls_per_run: Option<u32>,
    /// Also offered to the in-app agent, not only to MCP clients.
    #[serde(default = "yes")]
    pub internal: bool,
    #[serde(default)]
    pub wall_seconds: Option<u64>,
    #[serde(default)]
    pub memory_mb: Option<u64>,
    #[serde(default)]
    pub series: Vec<SeriesRequest>,
    /// Heading in the authoring guide's `## Tool notes`, when there is prose.
    #[serde(default)]
    pub notes_section: Option<String>,
}

fn one() -> u32 {
    1
}
fn read() -> String {
    "read".into()
}
fn none() -> String {
    "none".into()
}
fn yes() -> bool {
    true
}

/// A parsed template: its declaration, and the body that implements it.
#[derive(Clone, Debug)]
pub struct Template {
    header: TemplateHeader,
    source: String,
    /// `<namespace>/<name>.py`, as it sits under `apps/skill-runner/templates/`.
    rel_path: String,
}

impl Template {
    #[must_use]
    pub const fn header(&self) -> &TemplateHeader {
        &self.header
    }
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub fn rel_path(&self) -> &str {
        &self.rel_path
    }

    /// What the runner is actually handed: helpers, then the body.
    #[must_use]
    pub fn runnable_source(&self) -> String {
        format!("{PREAMBLE}\n{}", self.source)
    }

    /// Map a line number in a traceback back to the file the author edited.
    ///
    /// Returns `None` for a line inside the preamble, which is a platform bug
    /// rather than a template bug and should be reported as one.
    #[must_use]
    pub fn source_line(&self, reported: usize) -> Option<usize> {
        reported.checked_sub(preamble_lines()).filter(|n| *n > 0)
    }

    /// The input/output contract the runner checks on both sides.
    ///
    /// Inputs are the header schema's `required` list, so a template never gets
    /// as far as spawning a process on a call that was missing an argument.
    ///
    /// Outputs are `detail` alone, and deliberately not `n`/`se`/`ids`: a
    /// template that refuses answers with `refused`, `reason` and `detail`, and
    /// a contract naming the success shape would turn every honest refusal into
    /// a contract violation — which is recorded as the template misbehaving.
    #[must_use]
    pub fn contract(&self) -> crate::Contract {
        let required: Vec<String> = self
            .header
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        crate::Contract::new(
            required,
            vec!["detail".to_string()],
            format!("templated tool `{}`", self.header.name),
        )
        .expect("a contract over a non-empty output list is well formed")
    }

    /// The limits this template runs under, already checked against its
    /// namespace's ceiling by [`parse`].
    #[must_use]
    pub fn limits(&self) -> Limits {
        let d = Limits::default();
        let wall = self.header.wall_seconds.unwrap_or(d.wall_seconds);
        Limits {
            wall_seconds: wall,
            // CPU tracks the wall clock rather than staying at its default: a
            // template allowed 300 s of wall and 25 s of CPU is a template that
            // is killed by the CPU limit at second twenty-five, which reads as a
            // hang. Five seconds of headroom for interpreter startup.
            cpu_seconds: wall.saturating_sub(5).max(5),
            memory_mb: self.header.memory_mb.unwrap_or(d.memory_mb),
            output_bytes: d.output_bytes,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TemplateError {
    #[error("{path}: a template starts with a module docstring; this one does not")]
    NoDocstring { path: String },
    #[error("{path}: the module docstring has no fenced json header block")]
    NoHeader { path: String },
    #[error("{path}: the json header is not valid JSON: {detail}")]
    BadJson { path: String, detail: String },
    #[error("{path}: {detail}")]
    Invalid { path: String, detail: String },
}

/// Read one template file's text into a [`Template`].
///
/// # Errors
/// [`TemplateError`], always naming the path, because these are read in bulk at
/// build time and an error without a path is an error you have to bisect for.
pub fn parse(rel_path: &str, text: &str) -> Result<Template, TemplateError> {
    let path = rel_path.to_string();
    let doc = docstring(text).ok_or(TemplateError::NoDocstring { path: path.clone() })?;
    let block = fenced_json(&doc).ok_or(TemplateError::NoHeader { path: path.clone() })?;

    let header: TemplateHeader =
        serde_json::from_str(&block).map_err(|e| TemplateError::BadJson {
            path: path.clone(),
            detail: e.to_string(),
        })?;

    let invalid = |detail: String| TemplateError::Invalid {
        path: path.clone(),
        detail,
    };

    if !crate::is_valid_name(&header.name) {
        return Err(invalid(format!(
            "{:?} is not a usable tool name: lowercase letters, digits and underscores",
            header.name
        )));
    }
    // §2.3's ceiling. The catalogue asserts it too; failing here means the author
    // is told while editing the file rather than by a test in another crate.
    if header.name.len() > 32 {
        return Err(invalid(format!(
            "tool names are at most 32 characters; {:?} is {}",
            header.name,
            header.name.len()
        )));
    }
    let expected = format!("{}/{}.py", header.namespace, header.name);
    if rel_path.replace('\\', "/") != expected {
        return Err(invalid(format!(
            "a template lives at <namespace>/<name>.py — this header says {expected} but the \
             file is at {rel_path}"
        )));
    }
    if !matches!(
        header.risk.as_str(),
        "read" | "write" | "destructive" | "outbound"
    ) {
        return Err(invalid(format!("{:?} is not a risk class", header.risk)));
    }
    if header.risk != "read" {
        // A template has no network and no database: its rows arrive as a file
        // and its answer leaves as JSON. So `outbound` is either a mistake or an
        // attempt to acquire the egress policy's exemptions, and `write` or
        // `destructive` names a side effect it has no way to cause — which would
        // put a tool in the policy's dangerous slice for nothing, and teach a
        // reader that risk classes are approximate.
        //
        // Relaxing this needs an ADR rather than an edit: the tier's argument is
        // that these are safe enough to expose to a profile that may not run
        // code at all.
        return Err(invalid(format!(
            "a template has no network and no database, so it is `read`, not `{}`. The web \
             tools are the only way out, and every row arrives as a file.",
            header.risk
        )));
    }
    if !matches!(
        header.spends_trial.as_str(),
        "none" | "search" | "falsification"
    ) {
        return Err(invalid(format!(
            "{:?} is not a trial kind",
            header.spends_trial
        )));
    }
    if !matches!(
        header.envelope.as_str(),
        "none" | "spend" | "promotion" | "sealed_holdout" | "tier3_memory"
    ) {
        return Err(invalid(format!(
            "{:?} is not an approval envelope",
            header.envelope
        )));
    }
    if !header.input_schema.is_object() {
        return Err(invalid("input_schema must be a JSON Schema object".into()));
    }
    let ceiling = wall_ceiling(&header.namespace);
    if let Some(asked) = header.wall_seconds {
        if asked > ceiling {
            return Err(invalid(format!(
                "the `{}` namespace allows {ceiling} s and this template asks for {asked} s. \
                 Work that needs longer is a job, not a template: a job can be cancelled and \
                 observed while it runs.",
                header.namespace
            )));
        }
        if asked == 0 {
            return Err(invalid("wall_seconds must be positive".into()));
        }
    }
    // The summary is generated from declared fields (§2.4). A slot naming a field
    // the template never declares renders as a hole in the one line a flat-tier
    // model reads first.
    if let Some(t) = &header.summary_template {
        let declared: Vec<&str> = header
            .detail_fields
            .iter()
            .map(|(f, _)| f.as_str())
            .collect();
        for slot in slots(t) {
            if !declared.contains(&slot.as_str()) {
                return Err(invalid(format!(
                    "the summary template names a slot `{slot}` that is not in detail_fields"
                )));
            }
        }
    }
    crate::validate_source(text).map_err(|e| invalid(e.to_string()))?;

    Ok(Template {
        header,
        source: text.to_string(),
        rel_path: rel_path.replace('\\', "/"),
    })
}

/// The slot names in a summary template.
#[must_use]
pub fn slots(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                out.push(after[..close].to_string());
                rest = &after[close + 1..];
            }
            None => break,
        }
    }
    out
}

/// The module docstring: the first triple-quoted string in the file, ignoring
/// leading comments and blank lines.
fn docstring(text: &str) -> Option<String> {
    let mut offset = 0usize;
    for line in text.lines() {
        let t = line.trim_start();
        if t.is_empty() || t.starts_with('#') {
            offset += line.len() + 1;
            continue;
        }
        break;
    }
    let rest = text.get(offset..)?;
    let double = "\"\"\"";
    let single = "'''";
    for quote in [double, single] {
        if let Some(stripped) = rest.strip_prefix(quote) {
            if let Some(end) = stripped.find(quote) {
                return Some(stripped[..end].to_string());
            }
        }
    }
    None
}

/// The contents of the first json fence in a docstring.
fn fenced_json(doc: &str) -> Option<String> {
    let fence = "```";
    let opener = format!("{fence}json");
    let start = doc.find(&opener)? + opener.len();
    let rest = &doc[start..];
    let end = rest.find(fence)?;
    Some(rest[..end].trim().to_string())
}

// ── Materialisation (S-3.2) ──────────────────────────────────────────────────

/// The identity of one materialised window.
///
/// A template's input is a *file*, and two calls asking for the same rows must
/// get the same file rather than two files that were computed twice and might
/// differ — which is the whole of the determinism requirement. So the path is a
/// hash of the request, and writing is write-once: if the path exists, the rows
/// behind it are already there.
///
/// `end` is exclusive and all three instants are RFC 3339, so the key is a
/// *window*, not "the last N bars" — the latter changes meaning every minute and
/// would give two runs a day apart the same key and different data.
///
/// `as_of` is in the key and is not optional. It bounds which *revisions* of the
/// bars are visible, which is a different thing from which bars: a restatement
/// published yesterday is inside `end` and outside `as_of` for a project whose
/// horizon is older. Two projects with different horizons asking for the same
/// window are asking for different data, and a key without `as_of` would serve
/// the first one's file to the second — which is a future leak with a cache in
/// front of it, and the hardest kind to notice.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataKey {
    pub instrument_id: String,
    pub timeframe: String,
    pub start: String,
    pub end: String,
    pub as_of: String,
}

impl DataKey {
    /// A stable 32-hex digest of the four fields.
    ///
    /// Built from a `BTreeMap` rendered by `serde_json`, so the field order is
    /// the map's and not the struct's: a later field added to `DataKey` in the
    /// middle of the declaration does not silently re-key every existing file.
    #[must_use]
    pub fn digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut fields: BTreeMap<&str, &str> = BTreeMap::new();
        fields.insert("instrument_id", &self.instrument_id);
        fields.insert("timeframe", &self.timeframe);
        fields.insert("start", &self.start);
        fields.insert("end", &self.end);
        fields.insert("as_of", &self.as_of);
        let canonical = serde_json::to_string(&fields).unwrap_or_default();
        let digest = Sha256::digest(canonical.as_bytes());
        hex::encode(&digest[..16])
    }

    /// Where the parquet for this window lives under `root`.
    ///
    /// Sharded one level by the first two hex characters: a flat directory of a
    /// hundred thousand parquet files is slow to list on every filesystem that
    /// matters, and listing it is exactly what a cleanup job does.
    #[must_use]
    pub fn path(&self, root: &std::path::Path) -> std::path::PathBuf {
        let d = self.digest();
        root.join(&d[..2]).join(format!("{d}.parquet"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Built rather than written as a literal: a raw string holding a fenced
    /// block inside a docstring inside Rust is three quoting systems deep, and
    /// the one time it was written literally the escape sweep in S-1 corrupted
    /// it.
    fn good() -> String {
        let q = "\"\"\"";
        let f = "```";
        format!(
            "{q}Describe a return series.\n\n{f}json\n{{\n  \"name\": \"describe_returns\",\n  \
             \"namespace\": \"analysis\",\n  \"description\": \"Moments, tail index and \
             autocorrelation diagnostics for one instrument.\",\n  \"input_schema\": {{\"type\": \
             \"object\", \"properties\": {{\"instrument_id\": {{\"type\": \"string\"}}}}}},\n  \
             \"summary_template\": \"mean {{mean}}, n {{n}}\",\n  \"detail_fields\": [[\"mean\", \
             \"ret/bar\"], [\"n\", \"bars\"]],\n  \"wall_seconds\": 120,\n  \"series\": \
             [{{\"role\": \"primary\"}}]\n}}\n{f}\n{q}\n\ndef run(payload: dict) -> dict:\n    \
             return {{\"ok\": True}}\n"
        )
    }

    #[test]
    fn a_good_template_parses_and_keeps_its_declaration() {
        let t = parse("analysis/describe_returns.py", &good()).expect("parses");
        assert_eq!(t.header().name, "describe_returns");
        assert_eq!(t.header().namespace, "analysis");
        // The conservative defaults are what an omitted field means.
        assert_eq!(t.header().risk, "read");
        assert_eq!(t.header().spends_trial, "none");
        assert_eq!(t.header().envelope, "none");
        assert_eq!(t.header().version, 1);
        assert!(t.header().internal);
        assert_eq!(t.header().series[0].instrument_arg, "instrument_id");
    }

    #[test]
    fn the_runnable_source_is_the_preamble_then_the_body() {
        let t = parse("analysis/describe_returns.py", &good()).unwrap();
        let s = t.runnable_source();
        assert!(s.starts_with(PREAMBLE));
        assert!(s.ends_with(&good()));
        // And a traceback's line number maps back to the file the author edited.
        assert_eq!(t.source_line(preamble_lines() + 7), Some(7));
        assert_eq!(t.source_line(3), None, "a preamble line is a platform bug");
    }

    #[test]
    fn a_template_may_not_exceed_its_namespaces_ceiling() {
        let over = good().replace("\"wall_seconds\": 120", "\"wall_seconds\": 300");
        let err = parse("analysis/describe_returns.py", &over).unwrap_err();
        assert!(err.to_string().contains("allows 120 s"), "{err}");
        // `audit` is allowed the same 300 s that `analysis` is refused.
        let audit = good()
            .replace("\"namespace\": \"analysis\"", "\"namespace\": \"audit\"")
            .replace("\"wall_seconds\": 120", "\"wall_seconds\": 300");
        assert!(parse("audit/describe_returns.py", &audit).is_ok());
    }

    #[test]
    fn cpu_seconds_follow_the_wall_clock() {
        // The bug this exists to prevent: 300 s of wall and the default 25 s of
        // CPU is a kill at second twenty-five that reads like a hang.
        let audit = good()
            .replace("\"namespace\": \"analysis\"", "\"namespace\": \"audit\"")
            .replace("\"wall_seconds\": 120", "\"wall_seconds\": 300");
        let t = parse("audit/describe_returns.py", &audit).unwrap();
        assert_eq!(t.limits().wall_seconds, 300);
        assert!(t.limits().cpu_seconds >= 290, "{:?}", t.limits());
    }

    #[test]
    fn the_path_must_agree_with_the_header() {
        let err = parse("analysis/moments.py", &good()).unwrap_err();
        assert!(
            err.to_string().contains("analysis/describe_returns.py"),
            "{err}"
        );
    }

    #[test]
    fn a_summary_slot_must_name_a_declared_field() {
        let bad = good().replace("mean {mean}, n {n}", "mean {mean}, sharpe {sharpe}");
        let err = parse("analysis/describe_returns.py", &bad).unwrap_err();
        assert!(err.to_string().contains("`sharpe`"), "{err}");
    }

    #[test]
    fn a_template_cannot_claim_a_side_effect_it_cannot_have() {
        for claimed in ["outbound", "write", "destructive"] {
            let bad = good().replace(
                "\"namespace\": \"analysis\"",
                &format!("\"risk\": \"{claimed}\", \"namespace\": \"analysis\""),
            );
            let err = parse("analysis/describe_returns.py", &bad).unwrap_err();
            assert!(
                err.to_string().contains("no network and no database"),
                "{err}"
            );
        }
    }

    #[test]
    fn an_unknown_header_field_is_an_error_not_a_shrug() {
        // `deny_unknown_fields`: a typo'd `spend_trial` that parsed as "no trial"
        // is a tool that spends trials without counting them.
        let bad = good().replace("\"wall_seconds\": 120", "\"spend_trial\": \"search\"");
        let err = parse("analysis/describe_returns.py", &bad).unwrap_err();
        assert!(err.to_string().contains("spend_trial"), "{err}");
    }

    #[test]
    fn a_file_without_a_header_says_so() {
        let bare = "\"\"\"Just prose.\"\"\"\n\ndef run(payload: dict) -> dict:\n    return {}\n";
        assert!(matches!(
            parse("analysis/x.py", bare),
            Err(TemplateError::NoHeader { .. })
        ));
    }

    #[test]
    fn the_same_window_hashes_to_the_same_path_and_a_different_one_does_not() {
        let root = std::path::Path::new("/data");
        let a = DataKey {
            instrument_id: "BTC-USD".into(),
            timeframe: "1h".into(),
            start: "2024-01-01T00:00:00Z".into(),
            end: "2025-01-01T00:00:00Z".into(),
            as_of: "2025-01-01T00:00:00Z".into(),
        };
        let same = a.clone();
        let mut later = a.clone();
        later.end = "2025-01-02T00:00:00Z".into();

        assert_eq!(a.path(root), same.path(root));
        assert_ne!(a.path(root), later.path(root));
        // Sharded, so the cache directory stays listable.
        assert!(a.path(root).starts_with(root.join(&a.digest()[..2])));

        // The one that is easy to leave out, and is a future leak with a cache
        // in front of it: the same window seen from two different horizons is
        // two different sets of bar revisions.
        let mut newer_horizon = a.clone();
        newer_horizon.as_of = "2025-06-01T00:00:00Z".into();
        assert_ne!(
            a.path(root),
            newer_horizon.path(root),
            "two horizons must not share a cache file"
        );
    }
}
