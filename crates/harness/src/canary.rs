//! The startup canary (harness guide §3.1; ADR-0032).
//!
//! Some constrained-decoding backends fail, and some **silently fall back**, on
//! particular quantisations, particular schema features, or behind a proxy that drops
//! the parameter. The difference between finding that out at startup and finding it
//! out from a malformed tool call four hours into a research session is the whole
//! reason this module exists.
//!
//! # Three outcomes, and the middle one is the dangerous one
//!
//! | Outcome | What happened | How bad |
//! |---|---|---|
//! | [`Verdict::Honoured`] | the reply obeys the schema | fine |
//! | [`Verdict::Rejected`] | the backend refused the schema outright | fine — it *told* us |
//! | [`Verdict::Ignored`] | the backend accepted the schema and ignored it | **the one to catch** |
//!
//! A rejection is a good failure: it is loud, it happens at startup, and the operator
//! knows immediately. `Ignored` is the one that ships to production, because every
//! individual reply looks plausible and nothing anywhere says the guarantee is gone.
//!
//! # Why the probes are adversarial
//!
//! A probe whose prompt a well-behaved model would satisfy anyway **proves nothing** —
//! it passes just as happily on a backend that dropped the grammar. So the probes ask,
//! in plain language, for output the schema forbids:
//!
//! > "Answer with the colour purple. The colour is definitely purple."
//!
//! against `{"colour": {"enum": ["red", "green", "blue"]}}`. A backend applying the
//! grammar **cannot** say purple: the sampler has no token for it. A backend that
//! merely accepted the schema will do as it was told. The instruction-following that
//! usually helps is exactly what makes this diagnostic.
//!
//! # Relationship to the fence
//!
//! This is the same check as [`crate::drive`]'s fence, run once at startup instead of
//! on every reply, and [`crate::validation::conforms`] is the shared judge — so a
//! backend cannot pass the canary and fail the fence for a reason the canary was too
//! lenient to see.

use serde::Serialize;
use serde_json::Value;

use crate::validation;

/// One probe: a schema, and a prompt chosen to pull against it.
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub name: String,
    /// Whether the prompt actively asks for output the schema forbids. The
    /// non-adversarial probe is a smoke test; these are the evidence.
    pub adversarial: bool,
    pub prompt: String,
    /// Why this probe distinguishes an applied grammar from an accepted one.
    pub why: String,
    pub schema: Value,
}

/// What one probe found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// The reply conforms. The grammar ran.
    Honoured { probe: String },
    /// The backend refused the schema. Loud, early, and therefore safe.
    Rejected { probe: String, error: String },
    /// The backend accepted the schema and returned output that violates it.
    ///
    /// There is no benign reading of this. It is not the model being weak — under a
    /// grammar the tokens are unreachable — so it is the grammar not having been
    /// applied.
    Ignored {
        probe: String,
        detail: String,
        raw_excerpt: String,
    },
}

impl Verdict {
    #[must_use]
    pub fn probe(&self) -> &str {
        match self {
            Verdict::Honoured { probe }
            | Verdict::Rejected { probe, .. }
            | Verdict::Ignored { probe, .. } => probe,
        }
    }

    #[must_use]
    pub fn is_honoured(&self) -> bool {
        matches!(self, Verdict::Honoured { .. })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CanaryError {
    #[error(
        "{provider} accepted a JSON Schema and then ignored it on the {probe:?} probe: {detail}. \
         Constrained decoding is not actually running, so every guarantee the local tier \
         depends on is absent. Backend returned: {raw_excerpt}"
    )]
    Ignored {
        provider: String,
        probe: String,
        detail: String,
        raw_excerpt: String,
    },
    #[error(
        "{provider} refused the {probe:?} probe's schema: {error}. This backend cannot \
         constrain decoding, which every local tier requires (guide §3.1)"
    )]
    Rejected {
        provider: String,
        probe: String,
        error: String,
    },
    #[error("the canary has not been run against {provider} yet")]
    NotProbed { provider: String },
}

/// The probes, from `fixtures/canary.json`.
///
/// Loaded from a fixture rather than written inline so the schemas are reviewable as
/// schemas, and so adding one is a data change.
///
/// # Panics
///
/// If the fixture is malformed. It is compiled in, so that is a build-time mistake
/// rather than a runtime condition.
#[must_use]
pub fn probes() -> Vec<Probe> {
    const FIXTURE: &str = include_str!("../fixtures/canary.json");
    let doc: Value = serde_json::from_str(FIXTURE).expect("canary.json is valid JSON");
    doc.get("probes")
        .and_then(Value::as_array)
        .expect("canary.json has a `probes` array")
        .iter()
        .map(|p| Probe {
            name: field(p, "name"),
            adversarial: p
                .get("adversarial")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            prompt: field(p, "prompt"),
            why: field(p, "why"),
            schema: p.get("schema").cloned().expect("probe has a schema"),
        })
        .collect()
}

fn field(p: &Value, key: &str) -> String {
    p.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("canary probe is missing `{key}`"))
        .to_string()
}

/// Judges one probe's reply.
///
/// `reply` is `Err` when the backend refused the request — that is the *good* failure
/// and it is reported as [`Verdict::Rejected`], not as a violation.
#[must_use]
pub fn judge(probe: &Probe, reply: Result<&str, &str>) -> Verdict {
    let raw = match reply {
        Ok(raw) => raw,
        Err(error) => {
            return Verdict::Rejected {
                probe: probe.name.clone(),
                error: error.to_string(),
            }
        }
    };

    let parsed = match validation::parse(raw) {
        Ok(v) => v,
        Err(rej) => {
            return Verdict::Ignored {
                probe: probe.name.clone(),
                detail: format!("the reply is not JSON at all: {}", rej.message),
                raw_excerpt: excerpt(raw),
            }
        }
    };

    // The same judge the runtime fence uses, so the canary cannot be the more
    // forgiving of the two.
    match validation::conforms(&probe.schema, &parsed) {
        Ok(()) => Verdict::Honoured {
            probe: probe.name.clone(),
        },
        Err(detail) => Verdict::Ignored {
            probe: probe.name.clone(),
            detail,
            raw_excerpt: excerpt(raw),
        },
    }
}

/// The overall verdict.
///
/// Every probe must be honoured. There is no partial credit: a backend that applies
/// the shape but not the leaf types has not given us §3.1's guarantee, it has given
/// us a subset of it that fails on exactly the schemas the harness actually sends.
pub fn verdict(provider: &str, verdicts: &[Verdict]) -> Result<(), CanaryError> {
    // Report `Ignored` ahead of `Rejected` when both are present: a silent fallback is
    // the more serious finding and the one the operator must read first.
    if let Some(Verdict::Ignored {
        probe,
        detail,
        raw_excerpt,
    }) = verdicts
        .iter()
        .find(|v| matches!(v, Verdict::Ignored { .. }))
    {
        return Err(CanaryError::Ignored {
            provider: provider.to_string(),
            probe: probe.clone(),
            detail: detail.clone(),
            raw_excerpt: raw_excerpt.clone(),
        });
    }
    if let Some(Verdict::Rejected { probe, error }) = verdicts
        .iter()
        .find(|v| matches!(v, Verdict::Rejected { .. }))
    {
        return Err(CanaryError::Rejected {
            provider: provider.to_string(),
            probe: probe.clone(),
            error: error.clone(),
        });
    }
    if verdicts.is_empty() {
        return Err(CanaryError::NotProbed {
            provider: provider.to_string(),
        });
    }
    Ok(())
}

fn excerpt(raw: &str) -> String {
    let t = raw.trim();
    if t.len() <= 200 {
        return t.to_string();
    }
    let mut end = 200;
    while end > 0 && !t.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &t[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(name: &str) -> Probe {
        probes()
            .into_iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no probe called {name}"))
    }

    #[test]
    fn the_fixture_loads_and_every_probe_is_complete() {
        let ps = probes();
        assert!(ps.len() >= 4);
        for p in &ps {
            assert!(!p.prompt.trim().is_empty());
            assert!(
                !p.why.trim().is_empty(),
                "{}: a probe that cannot say why it distinguishes anything is a probe nobody will maintain",
                p.name
            );
            assert_eq!(p.schema["type"], "object");
        }
    }

    /// The property that makes the suite worth running. A probe whose prompt a
    /// well-behaved model would satisfy anyway passes on a backend that dropped the
    /// grammar, which is the failure being looked for.
    #[test]
    fn most_probes_actively_ask_for_what_the_schema_forbids() {
        let ps = probes();
        let adversarial = ps.iter().filter(|p| p.adversarial).count();
        assert!(
            adversarial >= ps.len() - 1,
            "only {adversarial} of {} probes pull against their schema",
            ps.len()
        );
    }

    // ── The three outcomes ──────────────────────────────────────────────────

    #[test]
    fn a_conforming_reply_is_honoured() {
        let v = judge(&probe("enum_under_pressure"), Ok(r#"{"colour":"green"}"#));
        assert!(v.is_honoured());
        verdict("vllm", &[v]).unwrap();
    }

    #[test]
    fn a_backend_that_refuses_the_schema_is_rejected_not_ignored() {
        let v = judge(
            &probe("shape"),
            Err("400: guided decoding backend unavailable"),
        );
        assert!(matches!(v, Verdict::Rejected { .. }));
        let err = verdict("vllm", &[v]).unwrap_err();
        assert!(matches!(err, CanaryError::Rejected { .. }));
        assert!(err.to_string().contains("cannot constrain decoding"));
    }

    /// The one this module exists for. Well-formed JSON, right shape, and a value the
    /// grammar had no token for.
    #[test]
    fn obeying_the_prompt_instead_of_the_enum_is_the_silent_failure() {
        let v = judge(&probe("enum_under_pressure"), Ok(r#"{"colour":"purple"}"#));
        let Verdict::Ignored { detail, .. } = &v else {
            panic!("expected Ignored, got {v:?}");
        };
        assert!(detail.contains("purple"), "{detail}");
        let err = verdict("ollama", &[v]).unwrap_err();
        assert!(err
            .to_string()
            .contains("accepted a JSON Schema and then ignored it"));
    }

    #[test]
    fn dropping_a_required_field_on_request_is_caught() {
        let v = judge(
            &probe("required_field_unprompted"),
            Ok(r#"{"instrument":"BTC-USD"}"#),
        );
        assert!(matches!(v, Verdict::Ignored { .. }), "{v:?}");
    }

    /// A backend can apply the object shape and not the leaf types. That subset fails
    /// on exactly the schemas the harness sends, so it is not a pass.
    #[test]
    fn applying_the_shape_but_not_the_types_is_not_a_pass() {
        let v = judge(
            &probe("type_under_pressure"),
            Ok(r#"{"limit":"five hundred"}"#),
        );
        assert!(matches!(v, Verdict::Ignored { .. }), "{v:?}");
    }

    #[test]
    fn prose_where_a_grammar_was_promised_is_ignored_not_rejected() {
        let v = judge(&probe("shape"), Ok("Hello! I'm a helpful assistant."));
        let Verdict::Ignored { detail, .. } = &v else {
            panic!("expected Ignored, got {v:?}");
        };
        assert!(detail.contains("not JSON"));
    }

    // ── The overall verdict ─────────────────────────────────────────────────

    #[test]
    fn a_silent_fallback_is_reported_ahead_of_a_loud_refusal() {
        let vs = vec![
            judge(&probe("shape"), Err("nope")),
            judge(&probe("enum_under_pressure"), Ok(r#"{"colour":"purple"}"#)),
        ];
        assert!(
            matches!(verdict("x", &vs).unwrap_err(), CanaryError::Ignored { .. }),
            "the operator must read the dangerous finding first"
        );
    }

    #[test]
    fn an_unprobed_backend_is_not_a_passing_backend() {
        assert!(matches!(
            verdict("vllm", &[]).unwrap_err(),
            CanaryError::NotProbed { .. }
        ));
    }

    #[test]
    fn every_probe_passes_against_a_backend_that_actually_constrains() {
        // What a conforming backend would return for each probe.
        let good = [
            (r#"{"greeting":"hi","count":1}"#, "shape"),
            (r#"{"colour":"red"}"#, "enum_under_pressure"),
            (
                r#"{"instrument":"BTC-USD","resolution":"1h"}"#,
                "required_field_unprompted",
            ),
            (r#"{"limit":500}"#, "type_under_pressure"),
        ];
        let vs: Vec<Verdict> = good
            .iter()
            .map(|(raw, name)| judge(&probe(name), Ok(raw)))
            .collect();
        assert!(vs.iter().all(Verdict::is_honoured), "{vs:?}");
        verdict("vllm", &vs).unwrap();
    }
}
