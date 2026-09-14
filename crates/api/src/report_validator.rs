//! `final_report` validation (AGENT-001 §15, RT-15, ADR-0025).
//!
//! The last of the four enforcement points. The agent can compute anything and say
//! anything; what it cannot do is get an unsupported claim *accepted*. Every number
//! in a report must resolve from a cited artifact, every citation must exist and
//! belong to the project, and a result-class claim must point at Set J evidence
//! rather than at an analysis the agent ran on its own.
//!
//! The checks are deterministic and ordered, cheapest first. A report that fails
//! schema validation never reaches the expensive citation checks, and the errors it
//! gets back name a path, a rule and a fix — a validator that only says "invalid"
//! teaches the agent nothing and produces a retry loop.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How a session ended (AGENT-001 §15).
pub const OUTCOMES: &[&str] = &[
    "answered",
    "vaulted",
    "failed_gate_0",
    "failed_gate_1",
    "failed_gate_2",
    "failed_gate_3",
    "failed_gate_4",
    "inconclusive",
    "aborted",
];

/// What kind of thing a claim is asserting.
///
/// The distinction is load-bearing at validation time: `result` is the only class
/// that may carry a significance claim, and it must cite Set J evidence. Without the
/// split, an agent could report the output of a script it wrote as though it had
/// been through the gates.
pub const CLAIM_CLASSES: &[&str] = &["fact", "estimate", "exploration", "result", "model_output"];

/// Prefixes of citable evidence ids.
const EVIDENCE_PREFIXES: &[&str] = &["exp_", "job_", "art_", "fnd_"];

/// Evidence that only Set J produces. A `result` claim must cite one of these.
const SET_J_EVIDENCE_PREFIXES: &[&str] = &["exp_", "art_"];

pub const MAX_ANSWER_CHARS: usize = 1200;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ValidationError {
    /// JSON path of the offending field.
    pub path: String,
    /// The rule that rejected it, so a caller can branch.
    pub rule: String,
    pub message: String,
    /// What to do instead.
    pub fix: String,
}

impl ValidationError {
    fn new(path: &str, rule: &str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            path: path.to_string(),
            rule: rule.to_string(),
            message: message.into(),
            fix: fix.into(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Claim {
    pub text: String,
    #[serde(default)]
    pub value: Option<f64>,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub rounding: Option<i32>,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub class: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub strategy_ref: String,
    pub experiment_id: String,
    pub gate_reached: String,
    #[serde(default)]
    pub trials: i64,
    #[serde(default)]
    pub effective_n: f64,
    #[serde(default)]
    pub dossier_ref: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FinalReport {
    pub schema: String,
    pub session_id: String,
    pub project_id: String,
    pub answer: String,
    pub outcome: String,
    #[serde(default)]
    pub claims: Vec<Claim>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default)]
    pub rejected: Vec<Value>,
    #[serde(default)]
    pub caveats: Vec<String>,
    #[serde(default)]
    pub exploration_ledger_ref: Option<String>,
    #[serde(default)]
    pub next_steps: Vec<String>,
}

/// The deterministic checks (AGENT-001 §15, steps 1–7).
///
/// Returns every problem rather than the first: an agent that has to resubmit seven
/// times to learn seven things burns seven turns.
pub fn validate(report: &FinalReport) -> Vec<ValidationError> {
    let mut errors = Vec::new();

    if report.schema != "final_report.v1" {
        errors.push(ValidationError::new(
            "schema",
            "schema_version",
            format!("unknown schema {:?}", report.schema),
            "use \"final_report.v1\"",
        ));
    }

    if report.answer.chars().count() > MAX_ANSWER_CHARS {
        errors.push(ValidationError::new(
            "answer",
            "answer_too_long",
            format!(
                "{} characters, limit {MAX_ANSWER_CHARS}",
                report.answer.chars().count()
            ),
            "state the answer; put the working in claims and artifacts",
        ));
    }

    if !OUTCOMES.contains(&report.outcome.as_str()) {
        errors.push(ValidationError::new(
            "outcome",
            "unknown_outcome",
            format!("{:?} is not an outcome", report.outcome),
            format!("one of: {}", OUTCOMES.join(", ")),
        ));
    }

    for (index, claim) in report.claims.iter().enumerate() {
        let path = format!("claims[{index}]");

        if !CLAIM_CLASSES.contains(&claim.class.as_str()) {
            errors.push(ValidationError::new(
                &path,
                "unknown_class",
                format!("{:?} is not a claim class", claim.class),
                format!("one of: {}", CLAIM_CLASSES.join(", ")),
            ));
        }

        // A claim with a number and no evidence is the exact shape of an unsupported
        // assertion, and the most important thing this validator catches.
        if claim.value.is_some() && claim.evidence.is_empty() {
            errors.push(ValidationError::new(
                &path,
                "unsupported_value",
                "a claim carrying a number cites nothing",
                "cite the artifact or experiment the number came from",
            ));
        }

        for (evidence_index, evidence) in claim.evidence.iter().enumerate() {
            if !EVIDENCE_PREFIXES.iter().any(|p| evidence.starts_with(p)) {
                errors.push(ValidationError::new(
                    &format!("{path}.evidence[{evidence_index}]"),
                    "bad_evidence_id",
                    format!("{evidence:?} is not a citable id"),
                    format!("ids start with one of: {}", EVIDENCE_PREFIXES.join(", ")),
                ));
            }
        }

        // Step 4: a result must rest on Set J evidence, not on the agent's own
        // analysis. An agent may compute whatever it likes; calling the output a
        // *result* is a claim about the gates it went through.
        if claim.class == "result"
            && !claim
                .evidence
                .iter()
                .any(|e| SET_J_EVIDENCE_PREFIXES.iter().any(|p| e.starts_with(p)))
        {
            errors.push(ValidationError::new(
                &path,
                "result_without_set_j_evidence",
                "a result-class claim cites no experiment or artifact",
                "cite the experiment (exp_…) or study artifact (art_…), or reclassify \
                 this as an exploration",
            ));
        }

        // Step 7: a bare number in prose with no corresponding claim value is how an
        // unchecked figure gets into a report.
        if claim.value.is_none() && contains_numeric_token(&claim.text) {
            errors.push(ValidationError::new(
                &path,
                "uncited_number_in_text",
                "the text contains a number but the claim has no value to check it against",
                "set `value` (and `unit`) so the number can be resolved from evidence",
            ));
        }
    }

    if contains_numeric_token(&report.answer) && !report.claims.iter().any(|c| c.value.is_some()) {
        errors.push(ValidationError::new(
            "answer",
            "uncited_number_in_answer",
            "the answer contains a number but no claim carries a value",
            "add a claim with the value and its evidence",
        ));
    }

    for (index, candidate) in report.candidates.iter().enumerate() {
        let path = format!("candidates[{index}]");
        let gate = gate_number(&candidate.gate_reached);
        match gate {
            None => errors.push(ValidationError::new(
                &path,
                "bad_gate",
                format!("{:?} is not a gate", candidate.gate_reached),
                "use G0..G4",
            )),
            // Step 5: past the significance gate, a candidate needs its dossier. The
            // dossier is what lets a reader check the claim instead of trusting it.
            Some(g) if g >= 3 && candidate.dossier_ref.is_none() => {
                errors.push(ValidationError::new(
                    &path,
                    "dossier_required",
                    format!(
                        "candidate reached {} without a dossier",
                        candidate.gate_reached
                    ),
                    "attach dossier_ref (art_…) with all 12 parts",
                ))
            }
            _ => {}
        }

        if candidate.trials <= 0 {
            errors.push(ValidationError::new(
                &path,
                "missing_trials",
                "a candidate reports no trial count",
                "report the experiment's trial counter; a significance claim without \
                 it cannot be corrected for selection",
            ));
        }
    }

    // The ledger is how a reader judges how much searching preceded the verdict
    // (D-13). A report that answers something without it is missing its denominator.
    if report.exploration_ledger_ref.is_none()
        && matches!(report.outcome.as_str(), "answered" | "vaulted")
    {
        errors.push(ValidationError::new(
            "exploration_ledger_ref",
            "missing_exploration_ledger",
            "an answered report does not reference its exploration ledger",
            "attach the exploration_summary artifact for this session",
        ));
    }

    errors
}

/// Words after which a bare integer is a label, not a measurement.
///
/// "Gate 1" and "phase 2" are references. Without this list the validator flags them,
/// and that is worse than it sounds: the honest outcome of a research session is
/// often *"every candidate failed Gate 1"*, so flagging it makes the truthful report
/// harder to submit than a claim of discovery. A validator that penalises honesty
/// pushes in precisely the wrong direction.
const LABEL_WORDS: &[&str] = &[
    "gate", "phase", "step", "tier", "level", "version", "part", "figure", "table", "section",
    "stage", "round", "attempt", "g", "v",
];

/// Whether a string contains a number that looks like an unchecked *measurement*.
///
/// Three kinds of digit are deliberately not flagged, because flagging them would
/// train the agent to strip useful text out of its prose:
///
/// * identifiers — `art_9f2c`, `exp_44`, `G3`;
/// * labels — the `1` in "Gate 1";
/// * ordinals attached to words — `2nd`.
///
/// A decimal or a percentage is always treated as a measurement, label word or not:
/// "gate 1.5" is not a gate.
fn contains_numeric_token(text: &str) -> bool {
    let tokens: Vec<&str> = text
        .split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')')
        .collect();

    for (index, token) in tokens.iter().enumerate() {
        let trimmed = token.trim_end_matches(['.', ':', ';', '"', '\'']);
        let is_percentage = trimmed.ends_with('%');
        let core = trimmed.trim_end_matches('%');
        if core.is_empty() {
            continue;
        }
        // An identifier or a word with digits in it is a reference, not a value.
        if core.contains('_') || core.chars().any(|c| c.is_ascii_alphabetic()) {
            continue;
        }
        let numeric = core.trim_start_matches(['-', '+']);
        let is_number = !numeric.is_empty()
            && numeric.chars().any(|c| c.is_ascii_digit())
            && numeric.chars().all(|c| c.is_ascii_digit() || c == '.');
        if !is_number {
            continue;
        }

        // A decimal or a percentage is a measurement wherever it appears.
        if is_percentage || numeric.contains('.') {
            return true;
        }

        // A bare integer is a measurement unless it is labelling something.
        let preceded_by_label = index
            .checked_sub(1)
            .and_then(|i| tokens.get(i))
            .map(|previous| {
                let word = previous
                    .trim_matches(|c: char| !c.is_ascii_alphanumeric())
                    .to_ascii_lowercase();
                LABEL_WORDS.contains(&word.as_str())
            })
            .unwrap_or(false);
        if !preceded_by_label {
            return true;
        }
    }
    false
}

fn gate_number(gate: &str) -> Option<u8> {
    let rest = gate.strip_prefix('G').or_else(|| gate.strip_prefix('g'))?;
    rest.parse::<u8>().ok().filter(|g| *g <= 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(claims: Vec<Claim>) -> FinalReport {
        FinalReport {
            schema: "final_report.v1".into(),
            session_id: "s".into(),
            project_id: "p".into(),
            answer: "No exploitable edge was found.".into(),
            outcome: "answered".into(),
            claims,
            candidates: vec![],
            rejected: vec![],
            caveats: vec![],
            exploration_ledger_ref: Some("art_ledger".into()),
            next_steps: vec![],
        }
    }

    fn claim(class: &str, value: Option<f64>, evidence: &[&str], text: &str) -> Claim {
        Claim {
            text: text.into(),
            value,
            unit: None,
            rounding: Some(2),
            evidence: evidence.iter().map(|e| e.to_string()).collect(),
            class: class.into(),
        }
    }

    fn rules(errors: &[ValidationError]) -> Vec<&str> {
        errors.iter().map(|e| e.rule.as_str()).collect()
    }

    #[test]
    fn a_clean_report_passes() {
        let r = report(vec![claim(
            "result",
            Some(0.12),
            &["exp_1"],
            "Sharpe was 0.12 after costs",
        )]);
        assert!(validate(&r).is_empty(), "{:?}", validate(&r));
    }

    #[test]
    fn a_number_with_no_evidence_is_rejected() {
        // The single most important case: this is the exact shape of an unsupported
        // assertion, and it is what the validator exists to stop.
        let r = report(vec![claim("estimate", Some(1.9), &[], "Sharpe 1.9")]);
        assert!(rules(&validate(&r)).contains(&"unsupported_value"));
    }

    #[test]
    fn a_result_must_rest_on_set_j_evidence() {
        // An agent may run any analysis it likes. Calling the output a *result* is a
        // claim about which gates it passed, so it must cite them.
        let r = report(vec![claim(
            "result",
            Some(0.4),
            &["job_123"],
            "edge of 0.4",
        )]);
        assert!(rules(&validate(&r)).contains(&"result_without_set_j_evidence"));

        let reclassified = report(vec![claim(
            "exploration",
            Some(0.4),
            &["job_123"],
            "edge of 0.4",
        )]);
        assert!(
            validate(&reclassified).is_empty(),
            "the same finding is fine as exploration: {:?}",
            validate(&reclassified)
        );
    }

    #[test]
    fn a_number_in_prose_without_a_value_is_flagged() {
        let r = report(vec![claim(
            "exploration",
            None,
            &["job_1"],
            "returns were 3.5 percent",
        )]);
        assert!(rules(&validate(&r)).contains(&"uncited_number_in_text"));
    }

    #[test]
    fn identifiers_are_not_mistaken_for_measurements() {
        // Flagging `art_9f2c` or `G3` would train the agent to strip citations out of
        // its prose, which is the opposite of what this validator wants.
        assert!(!contains_numeric_token("see art_9f2c and exp_44"));
        assert!(!contains_numeric_token("reached G3"));
        assert!(!contains_numeric_token("no numbers here"));
        assert!(contains_numeric_token("the value was 3.5"));
        assert!(contains_numeric_token("dropped -0.2 over the window"));
    }

    #[test]
    fn labels_are_not_mistaken_for_measurements() {
        // Found by `the_pure_noise_answer_is_a_valid_report`: the honest outcome of a
        // research session is often "every candidate failed Gate 1", and flagging
        // that made the truthful report harder to file than a claim of discovery.
        assert!(!contains_numeric_token("Every candidate failed Gate 1."));
        assert!(!contains_numeric_token("stopped at phase 2"));
        assert!(!contains_numeric_token("section 3 covers this"));

        // But a label word does not launder a real measurement.
        assert!(contains_numeric_token("gate 1.5"));
        assert!(contains_numeric_token("phase 20%"));
        // And an unlabelled integer is still a number worth checking.
        assert!(contains_numeric_token("the Sharpe was 2"));
    }

    #[test]
    fn percentages_are_always_measurements() {
        assert!(contains_numeric_token("returned 14%"));
        assert!(contains_numeric_token("14.2% annualised"));
    }

    #[test]
    fn an_answer_with_a_number_needs_a_claim_to_check_it() {
        let mut r = report(vec![]);
        r.answer = "The strategy returned 14.2% annualised.".into();
        assert!(rules(&validate(&r)).contains(&"uncited_number_in_answer"));
    }

    #[test]
    fn a_candidate_past_gate_three_needs_its_dossier() {
        let mut r = report(vec![]);
        r.candidates = vec![Candidate {
            strategy_ref: "s1".into(),
            experiment_id: "exp_1".into(),
            gate_reached: "G3".into(),
            trials: 40,
            effective_n: 12.0,
            dossier_ref: None,
        }];
        assert!(rules(&validate(&r)).contains(&"dossier_required"));

        r.candidates[0].dossier_ref = Some("art_dossier".into());
        assert!(!rules(&validate(&r)).contains(&"dossier_required"));
    }

    #[test]
    fn a_candidate_below_gate_three_does_not_need_one() {
        let mut r = report(vec![]);
        r.candidates = vec![Candidate {
            strategy_ref: "s1".into(),
            experiment_id: "exp_1".into(),
            gate_reached: "G1".into(),
            trials: 3,
            effective_n: 3.0,
            dossier_ref: None,
        }];
        assert!(!rules(&validate(&r)).contains(&"dossier_required"));
    }

    #[test]
    fn a_candidate_without_a_trial_count_is_rejected() {
        // A significance claim with no trial count cannot be corrected for selection,
        // so the number it reports means something different from what it appears to.
        let mut r = report(vec![]);
        r.candidates = vec![Candidate {
            strategy_ref: "s1".into(),
            experiment_id: "exp_1".into(),
            gate_reached: "G2".into(),
            trials: 0,
            effective_n: 0.0,
            dossier_ref: None,
        }];
        assert!(rules(&validate(&r)).contains(&"missing_trials"));
    }

    #[test]
    fn an_answered_report_must_reference_its_exploration_ledger() {
        let mut r = report(vec![]);
        r.exploration_ledger_ref = None;
        assert!(rules(&validate(&r)).contains(&"missing_exploration_ledger"));

        // An aborted session has no verdict to contextualise.
        r.outcome = "aborted".into();
        assert!(!rules(&validate(&r)).contains(&"missing_exploration_ledger"));
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        // Seven resubmissions to learn seven things is seven wasted turns.
        let mut r = report(vec![
            claim("nonsense", Some(1.0), &[], "1.0"),
            claim("result", Some(2.0), &["job_1"], "2.0"),
        ]);
        r.outcome = "maybe".into();
        r.schema = "v2".into();
        let found = validate(&r);
        assert!(found.len() >= 5, "only found {:?}", rules(&found));
    }

    #[test]
    fn every_error_names_a_fix() {
        let r = report(vec![claim("estimate", Some(1.0), &[], "1.0")]);
        for error in validate(&r) {
            assert!(!error.fix.is_empty(), "{error:?} has no fix");
            assert!(!error.path.is_empty());
            assert!(!error.rule.is_empty());
        }
    }

    #[test]
    fn the_pure_noise_answer_is_a_valid_report() {
        // The eval suite's sharpest task: given data with no structure, the correct
        // outcome is a report claiming no edge. The validator must accept that
        // cleanly — if honesty were harder to submit than a discovery, the whole
        // apparatus would push the wrong way.
        let r = FinalReport {
            schema: "final_report.v1".into(),
            session_id: "s".into(),
            project_id: "p".into(),
            answer: "No exploitable edge. Every candidate failed Gate 1.".into(),
            outcome: "inconclusive".into(),
            claims: vec![claim(
                "exploration",
                None,
                &["job_1"],
                "returns are indistinguishable from the null",
            )],
            candidates: vec![],
            rejected: vec![],
            caveats: vec!["synthetic instrument, no live data".into()],
            exploration_ledger_ref: Some("art_ledger".into()),
            next_steps: vec![],
        };
        assert!(validate(&r).is_empty(), "{:?}", validate(&r));
    }
}
