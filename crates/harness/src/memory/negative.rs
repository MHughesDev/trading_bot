//! Negative results are **derived, not declared** (AGENT-007 §3.2, P1, P3;
//! Set S task S-4.4).
//!
//! # The behaviour this stops
//!
//! An agent that can write its own `failure_class` will write
//! `capacity_insufficient` on an idea that died of weak effect, because
//! `capacity_insufficient` comes with a retry condition and weak effect does not.
//! That is not dishonesty; it is the shape of the incentive. A dead idea then
//! stays alive across every later session, spending budget on a retry condition
//! that will never be met, and the negative-result ledger — the entry AGENT-007
//! §2.4 calls the cheapest row in the store and the likeliest to save a budget —
//! quietly becomes the opposite.
//!
//! So the write path is inverted. The **platform** writes the entry from a
//! [`NegativeSource`]: a gate the funnel refused, a study objective that failed,
//! or a workflow verdict of `reject`. The class is a function of that source
//! ([`FailureClass::from_source`]) and the retry condition is a function of the
//! class. The agent's `memory.record_negative_result` tool reaches
//! [`annotate`] and nothing else.
//!
//! `derive_negative_result` takes a `NegativeSource` and there is no other
//! constructor — the type system is doing the enforcement, and
//! [`agent_declared`] exists only so the refusal has a name and a test.

use serde::{Deserialize, Serialize};

use super::{Confidence, DataWindow, Entry, EvidenceClass, Kind, Provenance};

/// The closed enum. AGENT-007 §3.2 plus AGENT-013 §8.2's four additions.
///
/// Closed is the operative word: every variant is produced by a named platform
/// event, and there is no `Other`. An escape hatch here is the whole hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// The effect is real but too small to survive costs.
    WeakEffect,
    /// The effect is real and collapses above an AUM the platform cannot reach.
    CapacityInsufficient,
    /// Transaction costs eat the edge.
    CostsDominate,
    /// The result did not survive out-of-sample.
    ReplicationFailed,
    /// Not enough data to decide.
    InsufficientData,
    /// The null comparison was not beaten.
    NoBetterThanNull,
    // ── AGENT-013 §8.2 ──
    /// A leak was found before the result was believed.
    Leakage,
    /// The measurement protocol itself is not stable across reruns.
    ProtocolUnstable,
    /// The result exists only at a knife-edge parameter setting.
    FragileParameters,
    /// The result holds in one regime and nowhere else.
    RegimeSpecific,
}

impl FailureClass {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            FailureClass::WeakEffect => "weak_effect",
            FailureClass::CapacityInsufficient => "capacity_insufficient",
            FailureClass::CostsDominate => "costs_dominate",
            FailureClass::ReplicationFailed => "replication_failed",
            FailureClass::InsufficientData => "insufficient_data",
            FailureClass::NoBetterThanNull => "no_better_than_null",
            FailureClass::Leakage => "leakage",
            FailureClass::ProtocolUnstable => "protocol_unstable",
            FailureClass::FragileParameters => "fragile_parameters",
            FailureClass::RegimeSpecific => "regime_specific",
        }
    }

    /// Every class this platform recognises. Migration 0066's CHECK is generated
    /// from the same list, and `the_sql_enum_matches_the_rust_enum` asserts it.
    #[must_use]
    pub fn all() -> &'static [FailureClass] {
        &[
            FailureClass::WeakEffect,
            FailureClass::CapacityInsufficient,
            FailureClass::CostsDominate,
            FailureClass::ReplicationFailed,
            FailureClass::InsufficientData,
            FailureClass::NoBetterThanNull,
            FailureClass::Leakage,
            FailureClass::ProtocolUnstable,
            FailureClass::FragileParameters,
            FailureClass::RegimeSpecific,
        ]
    }

    /// What would have to be true for the idea to be worth another look
    /// (AGENT-007 §3.2).
    ///
    /// `None` is the important half. A weak effect has no retry condition: there
    /// is nothing that could change to make a measured-small effect large, and
    /// offering a condition anyway is exactly the relabelling this module exists
    /// to stop.
    #[must_use]
    pub fn retry_condition(self, detail: &NegativeSource) -> Option<String> {
        match self {
            FailureClass::CapacityInsufficient => Some(format!(
                "retry if AUM target <= collapse_aum ({})",
                detail.quantity_hint()
            )),
            FailureClass::CostsDominate => Some(
                "retry if a venue with lower effective cost is added, or the holding period \
                 lengthens"
                    .to_string(),
            ),
            FailureClass::InsufficientData => Some(format!(
                "retry when the data window extends beyond {}",
                detail.window_hint()
            )),
            FailureClass::RegimeSpecific => Some(
                "retry when the regime that supported it recurs, as a prespecified conditional \
                 hypothesis"
                    .to_string(),
            ),
            FailureClass::ProtocolUnstable => {
                Some("retry once the protocol is deterministic across reruns".to_string())
            }
            FailureClass::Leakage => {
                Some("retry only after the leak is removed and the feature re-derived".to_string())
            }
            FailureClass::WeakEffect
            | FailureClass::ReplicationFailed
            | FailureClass::NoBetterThanNull
            | FailureClass::FragileParameters => None,
        }
    }

    /// Map a platform event to a class. The only way a class is ever chosen.
    #[must_use]
    pub fn from_source(source: &NegativeSource) -> FailureClass {
        match source {
            NegativeSource::GateRefused { class, .. }
            | NegativeSource::StudyObjectiveFailed { class, .. }
            | NegativeSource::WorkflowRejected { class, .. } => *class,
        }
    }
}

/// A platform event that produced a negative result.
///
/// Every variant names a **ticket the platform minted**. That id is what makes
/// the entry checkable afterwards: a negative result whose ticket does not
/// resolve is a claim, and this store does not hold claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum NegativeSource {
    /// The staged gate funnel refused a gate (Set J).
    GateRefused {
        ticket_id: String,
        gate: String,
        class: FailureClass,
        detail: String,
        window: DataWindow,
    },
    /// A study failed its prespecified objective.
    StudyObjectiveFailed {
        ticket_id: String,
        study_id: String,
        class: FailureClass,
        detail: String,
        window: DataWindow,
    },
    /// A workflow's verdict was `reject` (AGENT-013 §8).
    WorkflowRejected {
        ticket_id: String,
        workflow: String,
        class: FailureClass,
        detail: String,
        window: DataWindow,
    },
}

impl NegativeSource {
    #[must_use]
    pub fn ticket_id(&self) -> &str {
        match self {
            NegativeSource::GateRefused { ticket_id, .. }
            | NegativeSource::StudyObjectiveFailed { ticket_id, .. }
            | NegativeSource::WorkflowRejected { ticket_id, .. } => ticket_id,
        }
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            NegativeSource::GateRefused { detail, .. }
            | NegativeSource::StudyObjectiveFailed { detail, .. }
            | NegativeSource::WorkflowRejected { detail, .. } => detail,
        }
    }

    #[must_use]
    pub fn window(&self) -> &DataWindow {
        match self {
            NegativeSource::GateRefused { window, .. }
            | NegativeSource::StudyObjectiveFailed { window, .. }
            | NegativeSource::WorkflowRejected { window, .. } => window,
        }
    }

    fn stage(&self) -> String {
        match self {
            NegativeSource::GateRefused { gate, .. } => gate.clone(),
            NegativeSource::StudyObjectiveFailed { study_id, .. } => {
                format!("study {study_id}")
            }
            NegativeSource::WorkflowRejected { workflow, .. } => {
                format!("workflow {workflow}")
            }
        }
    }

    fn quantity_hint(&self) -> String {
        self.detail().to_string()
    }

    fn window_hint(&self) -> String {
        self.window().end.clone()
    }
}

/// Why a write was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RefusedWrite {
    #[error(
        "a negative result cannot be created by the model. `{attempted}` was not produced by \
         any gate, study or workflow verdict in this run — the platform writes negative \
         results from the funnel, and `memory.record_negative_result` may only annotate one \
         that exists. If an idea is dead, run the comparison that kills it."
    )]
    ClassNotProducedByTheFunnel { attempted: String },
    #[error("no negative result with ticket {ticket_id} exists to annotate")]
    NoSuchEntry { ticket_id: String },
    #[error("annotation may not change the failure class (from {from} to {to})")]
    AnnotationChangedClass { from: String, to: String },
}

/// The platform's only constructor.
///
/// Note what is **not** a parameter: the class. It is derived from the source,
/// which is derived from an event the funnel emitted. There is no path from a
/// string the model chose to the stored class.
#[must_use]
pub fn derive_negative_result(
    id: impl Into<String>,
    subject: impl Into<String>,
    source: &NegativeSource,
    provenance: Provenance,
) -> Entry {
    let class = FailureClass::from_source(source);
    let retry = class.retry_condition(source);
    let subject = subject.into();
    let claim = format!(
        "{}: refused at {} — {} ({})",
        subject,
        source.stage(),
        class.as_str(),
        truncate(source.detail(), 120)
    );
    let mut e = Entry::finding(
        id,
        subject,
        truncate(&claim, 300),
        source.window().clone(),
        provenance,
    );
    e.kind = Kind::NegativeResult;
    // A negative result is as established as the event that produced it. The
    // funnel refused a gate; that is a fact about the platform, not a model's
    // summary of one.
    e.confidence = Confidence::Established;
    e.evidence_class = EvidenceClass::DirectStat;
    e.evidence_ids
        .insert("ticket_id".to_string(), source.ticket_id().to_string());
    e.extra = serde_json::json!({
        "failure_class": class.as_str(),
        "ticket_id": source.ticket_id(),
        "retry_condition": retry,
        "source": source,
        "notes": [],
    });
    e.expiring()
}

/// The tool the agent actually has. It can add a note and nothing else.
///
/// Every field that decides what the entry *means* — class, ticket, retry
/// condition — is untouched, and an annotation that tries to change the class is
/// refused rather than silently ignored. Silent ignoring is how a model learns
/// the field is advisory.
pub fn annotate(entry: &mut Entry, note: &str, claimed_class: Option<&str>) -> Result<(), RefusedWrite> {
    if entry.kind != Kind::NegativeResult {
        return Err(RefusedWrite::NoSuchEntry {
            ticket_id: entry.id.clone(),
        });
    }
    let stored = entry
        .extra
        .get("failure_class")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(c) = claimed_class {
        if c != stored {
            return Err(RefusedWrite::AnnotationChangedClass {
                from: stored,
                to: c.to_string(),
            });
        }
    }
    let note = truncate(note, 500);
    match entry.extra.get_mut("notes").and_then(|n| n.as_array_mut()) {
        Some(notes) => notes.push(serde_json::Value::String(note)),
        None => {
            if let Some(obj) = entry.extra.as_object_mut() {
                obj.insert(
                    "notes".to_string(),
                    serde_json::Value::Array(vec![serde_json::Value::String(note)]),
                );
            }
        }
    }
    Ok(())
}

/// What happens when the agent tries to create one.
///
/// It exists so the refusal has a name, a code and a test. There is no argument
/// that makes it succeed — S-4.4's acceptance is precisely that this is
/// unreachable — and the error text tells the model what to do instead, because
/// a refusal the model cannot act on gets retried three times and fences the
/// session.
///
/// # Errors
///
/// Always. That is the contract.
pub fn agent_declared(attempted_class: &str) -> Result<Entry, RefusedWrite> {
    Err(RefusedWrite::ClassNotProducedByTheFunnel {
        attempted: attempted_class.to_string(),
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prov() -> Provenance {
        Provenance::new("run_7", "gate_funnel", "v2", "tpl_gate")
    }

    fn refused(class: FailureClass, detail: &str) -> NegativeSource {
        NegativeSource::GateRefused {
            ticket_id: "tkt_00011122".into(),
            gate: "gate_2".into(),
            class,
            detail: detail.into(),
            window: DataWindow::new("2026-01-01", "2026-06-30"),
        }
    }

    /// S-4.4's acceptance, stated adversarially: there is no path from a class
    /// the model chose to a stored entry.
    #[test]
    fn an_agent_cannot_create_a_negative_result_with_a_class_the_funnel_did_not_produce() {
        let err = agent_declared("capacity_insufficient").unwrap_err();
        assert!(matches!(
            err,
            RefusedWrite::ClassNotProducedByTheFunnel { .. }
        ));
        let msg = err.to_string();
        assert!(msg.contains("capacity_insufficient"), "{msg}");
        assert!(
            msg.contains("run the comparison that kills it"),
            "the refusal must say what to do instead: {msg}"
        );
    }

    /// The specific relabelling §3.2 names: a dead idea kept alive by a class
    /// that comes with a retry condition.
    #[test]
    fn annotation_cannot_relabel_weak_effect_as_capacity_insufficient() {
        let mut e = derive_negative_result(
            "n_1",
            "BTC-USD momentum",
            &refused(FailureClass::WeakEffect, "t = 0.4"),
            prov(),
        );
        let err = annotate(&mut e, "I think it is a capacity issue", Some("capacity_insufficient"))
            .unwrap_err();
        assert_eq!(
            err,
            RefusedWrite::AnnotationChangedClass {
                from: "weak_effect".into(),
                to: "capacity_insufficient".into()
            }
        );
        assert_eq!(e.extra["failure_class"], "weak_effect");
        assert!(
            e.extra["retry_condition"].is_null(),
            "a weak effect has no retry condition; offering one is the relabelling"
        );
    }

    #[test]
    fn annotation_appends_a_note_and_changes_nothing_else() {
        let mut e = derive_negative_result(
            "n_1",
            "BTC-USD momentum",
            &refused(FailureClass::WeakEffect, "t = 0.4"),
            prov(),
        );
        let before = e.clone();
        annotate(&mut e, "worth revisiting if the venue changes", None).unwrap();
        assert_eq!(e.extra["notes"][0], "worth revisiting if the venue changes");
        assert_eq!(e.extra["failure_class"], before.extra["failure_class"]);
        assert_eq!(e.confidence, before.confidence);
        assert_eq!(e.evidence_ids, before.evidence_ids);
    }

    #[test]
    fn the_class_and_the_retry_condition_both_come_from_the_source() {
        let cap = derive_negative_result(
            "n_2",
            "BTC-USD carry",
            &refused(FailureClass::CapacityInsufficient, "collapse_aum = 4.1e6"),
            prov(),
        );
        assert_eq!(cap.extra["failure_class"], "capacity_insufficient");
        assert!(cap.extra["retry_condition"]
            .as_str()
            .unwrap()
            .contains("collapse_aum"));
        assert_eq!(cap.evidence_ids["ticket_id"], "tkt_00011122");
    }

    #[test]
    fn a_negative_result_never_expires() {
        let e = derive_negative_result(
            "n_3",
            "BTC-USD",
            &refused(FailureClass::NoBetterThanNull, "null p = 0.6"),
            prov(),
        );
        assert!(e.valid_until.is_none());
        assert_eq!(e.kind, Kind::NegativeResult);
    }

    #[test]
    fn every_class_is_a_platform_event_and_the_enum_is_closed() {
        // Ten variants, each reachable only through a `NegativeSource`. If a
        // variant is added without a source that produces it, this count moves
        // and someone has to say why.
        assert_eq!(FailureClass::all().len(), 10);
        for c in FailureClass::all() {
            let src = refused(*c, "d");
            assert_eq!(FailureClass::from_source(&src), *c);
        }
    }

    #[test]
    fn a_derived_entry_validates_and_carries_its_ticket() {
        let e = derive_negative_result(
            "n_4",
            "BTC-USD",
            &refused(FailureClass::Leakage, "look-ahead in feature f_7"),
            prov(),
        );
        e.validate().expect("derived entries must be storable");
        assert!(e.claim.chars().count() <= 300);
    }
}
