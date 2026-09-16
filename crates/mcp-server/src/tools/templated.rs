//! The templated tier: tools that are Python files (AGENT-006 §2.2, §3.3; S-3.1).
//!
//! Every other module in `tools/` writes its [`ToolSpec`]s by hand. This one
//! reads them out of the headers of the files under
//! `apps/skill-runner/templates/`, which `build.rs` embedded. The declaration and
//! the implementation are therefore the same file, and there is no second place
//! to forget.
//!
//! # Why these are exposed to a model that may not run code
//!
//! A profile with `code_execution: denied` cannot be handed a Python sandbox. It
//! can be handed a *tool* — because the harness runs the template, not the model.
//! That is the whole argument for the tier: a 7B gets the same deflated Sharpe a
//! frontier model gets, computed by the same file, rather than an arithmetic it
//! invented and phrased confidently.
//!
//! # Failure is loud
//!
//! A malformed header panics at catalogue construction rather than dropping the
//! tool. Set S learned this the expensive way: a generated catalogue silently
//! lost thirteen tools and every test stayed green, because a reconciliation test
//! proves two sides agree and never that either side is complete (ADR-I-01). A
//! template that does not parse is a deployment bug, and a deployment bug that
//! presents as "that tool isn't in the list" costs a day.

use std::sync::OnceLock;

use serde_json::Value;
use skills::template::{self, Template};

use crate::spec::{Envelope, OutputBudget, Tier, ToolSpec, ToolStatus, TrialKind};
use crate::taxonomy::Risk;
use crate::ToolProfile;

include!(concat!(env!("OUT_DIR"), "/templates.rs"));

const MCP_ONLY: &[ToolProfile] = &[ToolProfile::Mcp];
const BOTH: &[ToolProfile] = &[ToolProfile::Mcp, ToolProfile::InternalAgent];

/// Every template, parsed, in catalogue order.
///
/// # Panics
/// If a template's header is missing or malformed, naming the file. See the
/// module docs for why this is not a skip.
pub fn templates() -> &'static [Template] {
    static PARSED: OnceLock<Vec<Template>> = OnceLock::new();
    PARSED.get_or_init(|| {
        TEMPLATE_FILES
            .iter()
            .map(|(rel, source)| {
                template::parse(rel, source).unwrap_or_else(|e| {
                    panic!(
                        "a template did not parse, so the catalogue would have been silently \
                         short one tool: {e}"
                    )
                })
            })
            .collect()
    })
}

/// Find one by name, for the dispatcher.
#[must_use]
pub fn find(name: &str) -> Option<&'static Template> {
    templates().iter().find(|t| t.header().name == name)
}

/// Route a templated call to the platform.
///
/// This crate is a thin HTTP client (ADR-0010) and stays one: it does not spawn
/// Python, does not read the database and does not touch the parquet cache. It
/// says *which* template and with what arguments, and `crates/api` does the
/// materialisation and the run — which is also where the trial ledger and the
/// approval envelopes live, so a templated tool cannot acquire a way around
/// either by being a file instead of a function.
pub async fn dispatch(api: &crate::ApiClient, name: &str, params: &Value) -> Value {
    let body = serde_json::json!({
        "template": name,
        "arguments": params.clone(),
    });
    match api.post("/api/agent/templates/run", body).await {
        Ok(v) => v,
        Err(e) => e.to_tool_error(),
    }
}

/// The catalogue entries for the templated tier.
pub fn specs() -> Vec<ToolSpec> {
    templates().iter().map(spec_for).collect()
}

/// A header is `String`s and a [`ToolSpec`] is `&'static str`s, so the strings
/// are leaked.
///
/// This is deliberate and bounded: the catalogue is a process-lifetime singleton
/// built exactly once, there are tens of templates, and the alternative is
/// threading a lifetime through every consumer of `ToolSpec` for the benefit of
/// memory that is freed when the process exits anyway.
fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

fn spec_for(t: &'static Template) -> ToolSpec {
    let h = t.header();
    // Not a match: `skills::template::parse` refuses any other class, because a
    // template has no network and no database and so has no side effect to
    // classify. See the test below.
    let risk = Risk::Read;
    debug_assert_eq!(h.risk, "read");
    let trial = match h.spends_trial.as_str() {
        "search" => TrialKind::Search,
        "falsification" => TrialKind::Falsification,
        _ => TrialKind::None,
    };
    let envelope = match h.envelope.as_str() {
        "spend" => Envelope::Spend,
        "promotion" => Envelope::Promotion,
        "sealed_holdout" => Envelope::SealedHoldout,
        "tier3_memory" => Envelope::Tier3Memory,
        _ => Envelope::None,
    };
    let detail_fields: &'static [(&'static str, &'static str)] = Box::leak(
        h.detail_fields
            .iter()
            .map(|(f, u)| (leak(f), leak(u)))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let evidence: &'static [&'static str] = Box::leak(
        h.evidence
            .iter()
            .map(|e| leak(e))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );

    ToolSpec {
        name: leak(&h.name),
        version: h.version,
        status: ToolStatus::Active,
        namespace: leak(&h.namespace),
        risk,
        description: leak(&h.description),
        input_schema: h.input_schema.clone(),
        output: OutputBudget {
            summary_tokens: 400,
            summary_template: h.summary_template.as_deref().map(leak),
            detail_fields,
        },
        evidence,
        spends_trial: trial,
        envelope,
        max_calls_per_run: h.max_calls_per_run,
        profiles: if h.internal { BOTH } else { MCP_ONLY },
        tier: Tier::Templated {
            template: leak(t.rel_path()),
        },
        notes_section: h.notes_section.as_deref().map(leak),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Completeness, not agreement.
    ///
    /// A test that only compared `specs()` against `templates()` would pass with
    /// both empty — which is precisely the failure ADR-I-01 records, where a
    /// generated catalogue lost thirteen tools with every test green. So this
    /// walks the directory on disk, independently of anything `build.rs`
    /// produced, and asserts the counts and the names match.
    #[test]
    fn every_template_on_disk_reached_the_catalogue() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap()
            .join("apps/skill-runner/templates");
        if !root.is_dir() {
            // Before the first template exists there is nothing to reconcile,
            // and an empty catalogue is the honest answer rather than a failure.
            assert!(specs().is_empty(), "no templates directory, but specs exist");
            return;
        }
        let mut on_disk: Vec<String> = Vec::new();
        for ns in std::fs::read_dir(&root).unwrap().filter_map(Result::ok) {
            if !ns.path().is_dir() {
                continue;
            }
            for f in std::fs::read_dir(ns.path()).unwrap().filter_map(Result::ok) {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) == Some("py") {
                    on_disk.push(p.file_stem().unwrap().to_string_lossy().to_string());
                }
            }
        }
        on_disk.sort();

        let mut declared: Vec<String> = specs().iter().map(|s| s.name.to_string()).collect();
        declared.sort();

        assert_eq!(
            on_disk, declared,
            "the templates on disk and the ones in the catalogue differ. A stale build is the \
             usual cause; `touch` the templates directory."
        );
    }

    /// The header *is* the catalogue schema, so this cannot drift — but it can
    /// be broken by a future refactor that renders the schema rather than
    /// carrying it, so the identity is asserted rather than assumed.
    #[test]
    fn a_templates_header_schema_is_its_catalogue_schema() {
        for (spec, t) in specs().iter().zip(templates()) {
            assert_eq!(spec.name, t.header().name);
            assert_eq!(spec.input_schema, t.header().input_schema);
            assert!(
                matches!(spec.tier, Tier::Templated { template } if template == t.rel_path()),
                "{} is not routed to its own file",
                spec.name
            );
        }
    }

    /// The tier's whole argument: a model that may not run code still gets
    /// these, and it is safe to offer them because a template cannot reach
    /// anything. A templated tool that showed up as `write` would mean the
    /// parser's guard had been relaxed without the ADR that relaxing it needs.
    #[test]
    fn a_templated_tool_is_read_only() {
        for spec in specs() {
            assert_eq!(
                spec.risk,
                Risk::Read,
                "{} is templated and not read-only",
                spec.name
            );
        }
    }
}
