//! Job domain types (COMP-005 §2, §5).

use serde::{Deserialize, Serialize};
use std::fmt;

/// What kind of work a job performs.
///
/// The variants are exactly COMP-005 §2. Two properties are attached to the kind
/// rather than to the submission, so that a caller cannot choose them: the worker
/// class that may run it, and — the one that matters for honesty — whether it counts
/// as an evaluation trial (INV-1, JB-03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// The campaign driver (SPEC §10, ADR-P2-04). It folds
    /// `mlops.campaign_event` and dispatches each phase's work as a child job.
    /// It does **not** count as a trial: the campaign itself looks at nothing —
    /// its children do, and they are counted where they are submitted.
    Campaign,
    Backtest,
    Sweep,
    Study,
    GateAdvance,
    DatasetBuild,
    FeatureMaterialise,
    Train,
    Hpo,
    PredictSeries,
    SimulatePaths,
    ResearchRun,
    Backfill,
    DataQc,
    InstrumentProfile,
    SkillVerify,
    EvalTask,
    AssetInit,
}

impl JobKind {
    pub const ALL: &'static [JobKind] = &[
        JobKind::Campaign,
        JobKind::Backtest,
        JobKind::Sweep,
        JobKind::Study,
        JobKind::GateAdvance,
        JobKind::DatasetBuild,
        JobKind::FeatureMaterialise,
        JobKind::Train,
        JobKind::Hpo,
        JobKind::PredictSeries,
        JobKind::SimulatePaths,
        JobKind::ResearchRun,
        JobKind::Backfill,
        JobKind::DataQc,
        JobKind::InstrumentProfile,
        JobKind::SkillVerify,
        JobKind::EvalTask,
        JobKind::AssetInit,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Campaign => "campaign",
            JobKind::Backtest => "backtest",
            JobKind::Sweep => "sweep",
            JobKind::Study => "study",
            JobKind::GateAdvance => "gate_advance",
            JobKind::DatasetBuild => "dataset_build",
            JobKind::FeatureMaterialise => "feature_materialise",
            JobKind::Train => "train",
            JobKind::Hpo => "hpo",
            JobKind::PredictSeries => "predict_series",
            JobKind::SimulatePaths => "simulate_paths",
            JobKind::ResearchRun => "research_run",
            JobKind::Backfill => "backfill",
            JobKind::DataQc => "data_qc",
            JobKind::InstrumentProfile => "instrument_profile",
            JobKind::SkillVerify => "skill_verify",
            JobKind::EvalTask => "eval_task",
            JobKind::AssetInit => "asset_init",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        JobKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// The worker pool that may claim this kind.
    pub fn worker_class(self) -> WorkerClass {
        match self {
            JobKind::Backtest | JobKind::Sweep | JobKind::Study | JobKind::GateAdvance => {
                WorkerClass::Backtest
            }
            JobKind::Campaign
            | JobKind::DatasetBuild
            | JobKind::FeatureMaterialise
            | JobKind::SimulatePaths
            | JobKind::ResearchRun => WorkerClass::Research,
            JobKind::Train | JobKind::Hpo | JobKind::PredictSeries => WorkerClass::Trainer,
            JobKind::Backfill
            | JobKind::DataQc
            | JobKind::InstrumentProfile
            | JobKind::AssetInit => WorkerClass::Data,
            JobKind::SkillVerify | JobKind::EvalTask => WorkerClass::Eval,
        }
    }

    /// Whether submitting this kind consumes an evaluation trial (INV-1).
    ///
    /// This is the single place that decides it. Counting happens inside the
    /// submission transaction (COMP-005 §10), so a client cannot submit work that
    /// looks at out-of-sample behaviour without the trial being recorded — which is
    /// the entire mechanism protecting the trial count from wishful bookkeeping.
    pub fn counts_trial(self) -> bool {
        matches!(
            self,
            JobKind::Backtest
                | JobKind::Sweep
                | JobKind::Study
                | JobKind::GateAdvance
                | JobKind::Train
                | JobKind::Hpo
        )
    }

    /// Whether this kind appends to the exploration ledger (D-13, JB-11).
    ///
    /// Exploration is logged, never counted: it is the looking-around that precedes
    /// a hypothesis, and reporting it is what lets a reader judge how much searching
    /// produced a verdict.
    pub fn logs_exploration(self) -> bool {
        matches!(
            self,
            JobKind::ResearchRun
                | JobKind::FeatureMaterialise
                | JobKind::DataQc
                | JobKind::InstrumentProfile
                | JobKind::SimulatePaths
        )
    }
}

impl fmt::Display for JobKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The worker pool a job runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerClass {
    Backtest,
    Research,
    Trainer,
    Data,
    Eval,
}

impl WorkerClass {
    pub const ALL: &'static [WorkerClass] = &[
        WorkerClass::Backtest,
        WorkerClass::Research,
        WorkerClass::Trainer,
        WorkerClass::Data,
        WorkerClass::Eval,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            WorkerClass::Backtest => "backtest",
            WorkerClass::Research => "research",
            WorkerClass::Trainer => "trainer",
            WorkerClass::Data => "data",
            WorkerClass::Eval => "eval",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        WorkerClass::ALL.iter().copied().find(|c| c.as_str() == s)
    }
}

/// Which fair-share queue a job waits in (COMP-005 §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Queue {
    Agent,
    Human,
    System,
}

impl Queue {
    pub const ALL: &'static [Queue] = &[Queue::Agent, Queue::Human, Queue::System];

    pub fn as_str(self) -> &'static str {
        match self {
            Queue::Agent => "agent",
            Queue::Human => "human",
            Queue::System => "system",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Queue::ALL.iter().copied().find(|q| q.as_str() == s)
    }

    /// Default weights, 3 : 5 : 2 (COMP-005 §7). Humans outrank the agent on purpose:
    /// a person waiting at a screen is a scarcer resource than a long-running agent.
    pub fn weight(self) -> u32 {
        match self {
            Queue::Agent => 3,
            Queue::Human => 5,
            Queue::System => 2,
        }
    }
}

/// Who submitted the job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmittedBy {
    Agent,
    User,
    System,
}

impl SubmittedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            SubmittedBy::Agent => "agent",
            SubmittedBy::User => "user",
            SubmittedBy::System => "system",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(SubmittedBy::Agent),
            "user" => Some(SubmittedBy::User),
            "system" => Some(SubmittedBy::System),
            _ => None,
        }
    }
    /// The queue a submitter's work lands in by default.
    pub fn default_queue(self) -> Queue {
        match self {
            SubmittedBy::Agent => Queue::Agent,
            SubmittedBy::User => Queue::Human,
            SubmittedBy::System => Queue::System,
        }
    }
}

/// Job lifecycle state (COMP-005 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Leased,
    Running,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(self) -> &'static str {
        match self {
            JobState::Queued => "queued",
            JobState::Leased => "leased",
            JobState::Running => "running",
            JobState::Paused => "paused",
            JobState::Succeeded => "succeeded",
            JobState::Failed => "failed",
            JobState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(JobState::Queued),
            "leased" => Some(JobState::Leased),
            "running" => Some(JobState::Running),
            "paused" => Some(JobState::Paused),
            "succeeded" => Some(JobState::Succeeded),
            "failed" => Some(JobState::Failed),
            "cancelled" => Some(JobState::Cancelled),
            _ => None,
        }
    }

    /// Terminal states never change again (COMP-005 §5), enforced by a database
    /// trigger as well as here.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Succeeded | JobState::Failed | JobState::Cancelled
        )
    }
}

/// Progress reported by a running worker. Rate-limited to one update per 5 s per job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pct: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Cost estimate or actual, in compute seconds and dollars.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Cost {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// A structured job failure (JB-07).
///
/// Capped at 400 bytes on the wire: the detail belongs in an artifact, referenced by
/// `detail_ref`. An agent reading a 50 kB stack trace into its context learns almost
/// nothing a four-field summary would not have told it, and pays for the whole thing.
///
/// Every failure carries the [`TerminalReason`] the ledger will record. It is a
/// field of the type rather than something inferred from `code` downstream: the
/// settlement paths used to recover the reason by substring-matching the message
/// (`contains("nan")`, `contains("data")`), which meant a reworded error message
/// silently changed a trial's censoring — and every unrecognised message became
/// `dependency_failure`, the reason that says nothing (ADR-P2-30).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobError {
    /// Machine-readable, e.g. `invalid_manifest`, `data_gap`, `lost_worker`. Free
    /// text for diagnosis; it is *not* what the ledger reads.
    pub code: String,
    /// What §9 records when this failure settles a trial, and therefore which
    /// censoring INV-17 applies. REQUIRED: no serde default, no constructor that
    /// omits it. A `JobError` that does not say how the trial ended cannot exist.
    pub terminal: ledger::TerminalReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// What the caller should do differently.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<String>,
    /// Handle of a `log` artifact carrying the full detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_ref: Option<String>,
    /// Whether this failure is worth retrying. Logic failures never are (JB-07).
    #[serde(default)]
    pub retryable: bool,
}

impl JobError {
    /// A failure the caller caused: the request is wrong and retrying it
    /// unchanged produces the same answer. The reason is explicit because
    /// "the caller was wrong" and "the data was missing" settle differently.
    pub fn logic(terminal: ledger::TerminalReason, code: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            terminal,
            field: None,
            rule: None,
            fix: Some(fix.into()),
            detail_ref: None,
            retryable: false,
        }
    }

    /// A failure of something the job depended on. The reason is fixed rather
    /// than chosen: `dependency_failure` is the definition of this constructor,
    /// not a fallback for an unrecognised one.
    pub fn infrastructure(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            terminal: ledger::TerminalReason::DependencyFailure,
            field: None,
            rule: None,
            fix: None,
            detail_ref: None,
            retryable: true,
        }
    }

    /// A failure of the work itself, with the reason the worker observed.
    ///
    /// Not retryable: a run that diverged to NaN or exhausted its budget does so
    /// again. `infrastructure` is the retryable constructor.
    pub fn terminal(terminal: ledger::TerminalReason, code: impl Into<String>, fix: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            terminal,
            field: None,
            rule: None,
            fix: Some(fix.into()),
            detail_ref: None,
            retryable: false,
        }
    }

    /// The censoring this failure implies (INV-17). Delegates: the reason owns
    /// the mapping, so there is one place it is decided.
    #[must_use]
    pub fn censoring(&self) -> ledger::Censoring {
        self.terminal.censoring()
    }

    /// Serialised size cap from JB-07. Enforced when writing, not merely documented.
    pub const MAX_BYTES: usize = 400;

    /// A `code` is meant to be a short identifier a caller can branch on.
    pub const MAX_CODE_BYTES: usize = 64;

    /// Truncates the human-readable fields until the whole error fits the cap.
    ///
    /// Also rescues a `code` that is really a message. Passing a formatted error
    /// string as the code is an easy mistake — it compiles, and the result looks
    /// almost right in logs — but it destroys the one field callers are supposed to
    /// match on, because every occurrence is then unique. Rather than silently
    /// truncating it into a different kind of nonsense, the message is moved to
    /// `fix` where prose belongs.
    pub fn clamped(mut self) -> Self {
        if self.code.len() > Self::MAX_CODE_BYTES || self.code.contains(' ') {
            let message = std::mem::replace(&mut self.code, "unspecified_error".to_string());
            self.fix = Some(match self.fix.take() {
                Some(existing) => format!("{message}; {existing}"),
                None => message,
            });
        }
        for _ in 0..4 {
            let size = serde_json::to_vec(&self).map(|v| v.len()).unwrap_or(0);
            if size <= Self::MAX_BYTES {
                break;
            }
            // Drop the least load-bearing field first. `code` is never dropped: it is
            // the only part a caller can branch on.
            if self.rule.is_some() {
                self.rule = None;
            } else if self.field.is_some() {
                self.field = None;
            } else if let Some(fix) = self.fix.take() {
                let cut = fix.chars().take(120).collect::<String>();
                self.fix = Some(cut);
            } else {
                self.detail_ref = None;
            }
        }
        self
    }
}

/// What a worker returns on success.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobOutput {
    /// A one-line-ish summary written for a model to read. Capped at 1536 bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Metrics and handles.
    #[serde(default)]
    pub result: serde_json::Value,
    /// Artifacts produced, as `(handle, role)`.
    #[serde(default)]
    pub artifacts: Vec<(String, String)>,
    #[serde(default)]
    pub actual: Cost,
}

impl JobOutput {
    pub const MAX_SUMMARY_BYTES: usize = 1536;

    /// Truncates the summary to the cap on a character boundary.
    pub fn clamped(mut self) -> Self {
        if let Some(summary) = self.summary.take() {
            if summary.len() <= Self::MAX_SUMMARY_BYTES {
                self.summary = Some(summary);
            } else {
                let mut end = Self::MAX_SUMMARY_BYTES;
                while end > 0 && !summary.is_char_boundary(end) {
                    end -= 1;
                }
                let mut cut = summary[..end].to_string();
                cut.push('…');
                self.summary = Some(cut);
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_through_their_wire_names() {
        for kind in JobKind::ALL {
            assert_eq!(JobKind::parse(kind.as_str()), Some(*kind));
        }
        assert_eq!(JobKind::parse("not_a_kind"), None);
    }

    #[test]
    fn every_kind_has_a_worker_class_and_a_counting_decision() {
        // Both are total functions over the enum, so adding a kind forces a decision
        // rather than defaulting to "does not count" — which would be a silent hole
        // in INV-1.
        for kind in JobKind::ALL {
            let _ = kind.worker_class();
            let _ = kind.counts_trial();
        }
    }

    #[test]
    fn evaluation_counted_kinds_are_exactly_the_spec_list() {
        let counted: Vec<&str> = JobKind::ALL
            .iter()
            .filter(|k| k.counts_trial())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            counted,
            vec!["backtest", "sweep", "study", "gate_advance", "train", "hpo"],
            "COMP-005 §2 fixes this list; changing it changes what the trial counter means"
        );
    }

    #[test]
    fn terminal_states_are_terminal() {
        assert!(JobState::Succeeded.is_terminal());
        assert!(JobState::Failed.is_terminal());
        assert!(JobState::Cancelled.is_terminal());
        assert!(!JobState::Queued.is_terminal());
        assert!(!JobState::Running.is_terminal());
        assert!(!JobState::Paused.is_terminal());
    }

    /// AT-60 — the mapping from a job failure to a `TerminalReason` is total
    /// *because there is no mapping*: the reason is the field. There is no
    /// string a caller can put in `code` that produces a reason nobody chose,
    /// and no way to construct a `JobError` that does not say how the trial
    /// ended — `JobError { code, .. }` without `terminal` is a compile error,
    /// which is the "unmapped code fails the build" property the plan asks for.
    #[test]
    fn every_failure_names_the_reason_the_ledger_records() {
        for reason in ledger::TerminalReason::ALL {
            let e = JobError::terminal(reason, "worker_failed", "see the log artifact");
            assert_eq!(e.terminal, reason);
            assert_eq!(e.censoring(), reason.censoring());
            // Clamping is about wire size; it must never touch the reason.
            assert_eq!(e.clamped().terminal, reason);
        }
    }

    /// AT-60 — `asha_stopped` is right-censored, never a failure. A scheduler
    /// that stops a trial early has observed less, not observed a loss, and
    /// M3/M4/M5 read this distinction directly.
    #[test]
    fn an_asha_stop_is_censored_not_failed() {
        let e = JobError::terminal(ledger::TerminalReason::AshaStopped, "asha_stopped", "rung 2");
        assert_eq!(e.censoring(), ledger::Censoring::RightAsha);
        assert_ne!(e.censoring(), ledger::Censoring::Failed);
    }

    /// The field has no serde default, so a stored failure that predates it
    /// does not silently deserialise into one reason or another — it refuses to
    /// load, and migration 0051 is what makes those rows readable again.
    #[test]
    fn a_failure_without_a_reason_does_not_deserialise() {
        let without = serde_json::json!({ "code": "data_gap", "retryable": false });
        assert!(serde_json::from_value::<JobError>(without).is_err());

        let with = serde_json::json!({
            "code": "data_gap",
            "terminal": "data_error",
            "retryable": false,
        });
        let e: JobError = serde_json::from_value(with).expect("a reason makes it readable");
        assert_eq!(e.terminal, ledger::TerminalReason::DataError);
    }

    #[test]
    fn errors_are_clamped_to_the_wire_cap() {
        let huge = JobError {
            code: "data_gap".into(),
            terminal: ledger::TerminalReason::DataError,
            field: Some("x".repeat(200)),
            rule: Some("y".repeat(200)),
            fix: Some("z".repeat(400)),
            detail_ref: Some("art_abc".into()),
            retryable: false,
        }
        .clamped();
        let size = serde_json::to_vec(&huge).unwrap().len();
        assert!(size <= JobError::MAX_BYTES, "still {size} bytes");
        assert_eq!(huge.code, "data_gap", "the code must always survive");
    }

    #[test]
    fn summaries_are_clamped_on_a_char_boundary() {
        let out = JobOutput {
            summary: Some("é".repeat(2000)),
            ..Default::default()
        }
        .clamped();
        let summary = out.summary.unwrap();
        // Must not panic and must not exceed the cap by more than the ellipsis.
        assert!(summary.len() <= JobOutput::MAX_SUMMARY_BYTES + 4);
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn a_message_passed_as_a_code_is_rescued_into_fix() {
        // Found in real use: a worker did `JobError::infrastructure(format!(...))`,
        // which put a whole ClickHouse message in `code`. It reads fine in a log and
        // is useless to anything trying to branch on the failure.
        let error = JobError::infrastructure(
            "clickhouse: not enough data, probably a row type mismatches a database schema",
        )
        .clamped();
        assert_eq!(error.code, "unspecified_error");
        assert!(
            error.fix.unwrap().contains("not enough data"),
            "the message must survive, just not as the code"
        );
    }

    #[test]
    fn a_real_code_is_left_alone() {
        let error = JobError::infrastructure("lost_worker").clamped();
        assert_eq!(error.code, "lost_worker");
        assert!(error.fix.is_none());
    }

    #[test]
    fn logic_failures_are_never_retryable() {
        assert!(!JobError::logic(ledger::TerminalReason::IntegrityRejected, "bad_manifest", "fix it").retryable);
        assert!(JobError::infrastructure("lost_worker").retryable);
    }
}
