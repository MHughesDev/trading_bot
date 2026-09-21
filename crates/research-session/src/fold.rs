//! The fold (AGENT-014 P1, §3): a session's state is its event log replayed,
//! and there is nothing else to recover.
//!
//! # Why there is no status column
//!
//! A status column and an event log are two answers to the same question, and a
//! driver killed between writing one and writing the other leaves them
//! disagreeing with no way to tell which is right. `mlops.campaign_event` solved
//! this in ADR-P2-04 and the campaign driver has run on it since; this is the
//! same shape, for the same reason, down to the BEFORE INSERT trigger that
//! assigns `seq` so the writer never chooses its own position in the log.
//!
//! # What the trigger can and cannot hold
//!
//! Most event kinds do not change the session's state — an agenda item being
//! added, a selection being made, a run starting. Those carry `state = NULL` and
//! the trigger only numbers them. The kinds that *are* transitions carry the
//! state they move to, and the trigger refuses an illegal one against the last
//! non-null state in the log.
//!
//! That is the whole of what the database enforces, and saying so here matters:
//! the trigger does **not** check that a `run_finished` follows a `run_started`
//! for the same item, because a rule over pairs of events needs the payload and
//! belongs in [`fold`] where it can be tested without a database. [`Folded`]
//! carries [`Folded::anomalies`] for exactly those, and a driver that finds one
//! stops rather than acting on a state it cannot justify.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where a session is (AGENT-014 §2.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionState {
    Draft,
    Running,
    Paused { why: String },
    Stalled { diagnosis: String },
    Complete,
    Stopped,
}

impl SessionState {
    /// The word the `state` column holds. The payload — *why* it paused, *what*
    /// the diagnosis was — rides in `payload_json`, because a transition table
    /// over free text is not a transition table.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            SessionState::Draft => "draft",
            SessionState::Running => "running",
            SessionState::Paused { .. } => "paused",
            SessionState::Stalled { .. } => "stalled",
            SessionState::Complete => "complete",
            SessionState::Stopped => "stopped",
        }
    }

    /// Every state word, in the order the transition table is written in.
    pub const WORDS: &'static [&'static str] = &[
        "draft", "running", "paused", "stalled", "complete", "stopped",
    ];

    /// Whether the driver may do work in this state (AGENT-014 §3: the loop
    /// exits unless the state is `running`).
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, SessionState::Running)
    }
}

/// Whether a session may move from one state word to another.
///
/// **The one definition.** The migration's SQL function is held equal to this by
/// `crates/invariants/tests/a_session_transition_table_has_one_definition.rs`,
/// which walks every ordered pair of [`SessionState::WORDS`] and compares the
/// two tables cell by cell as text. Two copies of a transition table that drift
/// is the defect the 0052 discipline exists to avoid, and writing it twice
/// without a test comparing them would reintroduce it.
///
/// `stopped` is the only terminal state. `complete` is not: AGENT-014 §7 says a
/// drained agenda completes and a user adding an item starts it again, which is
/// the difference between a session that finished its list and a session the
/// human ended.
#[must_use]
pub fn transition_ok(prev: &str, next: &str) -> bool {
    match (prev, next) {
        (_, p) if p == prev => false,
        ("stopped", _) => false,
        ("draft", "running" | "stopped") => true,
        ("running", "paused" | "stalled" | "complete" | "stopped") => true,
        ("paused", "running" | "complete" | "stopped") => true,
        ("stalled", "running" | "complete" | "stopped") => true,
        ("complete", "running" | "stopped") => true,
        _ => false,
    }
}

/// The first state a session's log may open with.
pub const OPENING_STATE: &str = "draft";

/// What an event says happened (AGENT-014 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// The session row exists and the charter (or question) is written.
    SessionCreated,
    SessionStarted,
    SessionPaused,
    SessionResumed,
    SessionStalled,
    SessionCompleted,
    SessionStopped,
    CharterAmended,
    AgendaItemAdded,
    AgendaItemMerged,
    AgendaItemApproved,
    AgendaItemVetoed,
    AgendaItemSuperseded,
    AgendaItemExpired,
    /// An **agenda item** is waiting for a human before it may run (§6.3).
    AwaitingApproval,
    /// A **proposal a run made** is waiting for a human to decide it
    /// (AGENT-014 P6, rings 1 and 2).
    ///
    /// Distinct from [`Self::AwaitingApproval`], which is about an item on the
    /// agenda, because the two open different things and a reader of the log
    /// cannot tell them apart from a payload. Appending this **records that a
    /// human is owed a decision and nothing else**: there is no kind here that
    /// clears one, no `decided` payload the fold reads, and
    /// [`crate::harvest::AwaitingHuman`] has no decision field for one to reach
    /// — a session that could log its own approval would be a session that
    /// approves its own proposals.
    ProposalAwaiting,
    Selected,
    RunStarted,
    RunFinished,
    WindowClosed,
    DayBudgetExhausted,
    DigestWritten,
    /// Something the driver wants on the log with no state behind it — a
    /// skipped offer and why, an exclusion the stall detector could not see.
    ///
    /// Exists so that a note is not filed as an event that means something
    /// else. Before it, the driver reached for [`Self::AgendaItemMerged`] for
    /// prose, and a log where a merge might not be a merge is a log the fold
    /// cannot be trusted to read.
    Note,
}

impl EventKind {
    pub const ALL: &'static [EventKind] = &[
        EventKind::SessionCreated,
        EventKind::SessionStarted,
        EventKind::SessionPaused,
        EventKind::SessionResumed,
        EventKind::SessionStalled,
        EventKind::SessionCompleted,
        EventKind::SessionStopped,
        EventKind::CharterAmended,
        EventKind::AgendaItemAdded,
        EventKind::AgendaItemMerged,
        EventKind::AgendaItemApproved,
        EventKind::AgendaItemVetoed,
        EventKind::AgendaItemSuperseded,
        EventKind::AgendaItemExpired,
        EventKind::AwaitingApproval,
        EventKind::ProposalAwaiting,
        EventKind::Selected,
        EventKind::RunStarted,
        EventKind::RunFinished,
        EventKind::WindowClosed,
        EventKind::DayBudgetExhausted,
        EventKind::DigestWritten,
        EventKind::Note,
    ];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::SessionCreated => "session_created",
            EventKind::SessionStarted => "session_started",
            EventKind::SessionPaused => "session_paused",
            EventKind::SessionResumed => "session_resumed",
            EventKind::SessionStalled => "session_stalled",
            EventKind::SessionCompleted => "session_completed",
            EventKind::SessionStopped => "session_stopped",
            EventKind::CharterAmended => "charter_amended",
            EventKind::AgendaItemAdded => "agenda_item_added",
            EventKind::AgendaItemMerged => "agenda_item_merged",
            EventKind::AgendaItemApproved => "agenda_item_approved",
            EventKind::AgendaItemVetoed => "agenda_item_vetoed",
            EventKind::AgendaItemSuperseded => "agenda_item_superseded",
            EventKind::AgendaItemExpired => "agenda_item_expired",
            EventKind::AwaitingApproval => "awaiting_approval",
            EventKind::ProposalAwaiting => "proposal_awaiting",
            EventKind::Selected => "selected",
            EventKind::RunStarted => "run_started",
            EventKind::RunFinished => "run_finished",
            EventKind::WindowClosed => "window_closed",
            EventKind::DayBudgetExhausted => "day_budget_exhausted",
            EventKind::DigestWritten => "digest_written",
            EventKind::Note => "note",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.as_str() == s)
    }

    /// The state this kind moves the session to, if it is a transition.
    ///
    /// Every other kind carries `NULL` in the `state` column and the trigger
    /// only numbers it. This is the function the writer uses, so a kind cannot
    /// be a transition in Rust and not in the log.
    #[must_use]
    pub fn moves_to(self) -> Option<&'static str> {
        match self {
            EventKind::SessionCreated => Some("draft"),
            EventKind::SessionStarted | EventKind::SessionResumed => Some("running"),
            EventKind::SessionPaused => Some("paused"),
            EventKind::SessionStalled => Some("stalled"),
            EventKind::SessionCompleted => Some("complete"),
            EventKind::SessionStopped => Some("stopped"),
            _ => None,
        }
    }
}

/// One row of `session_events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub session_id: Uuid,
    pub seq: i64,
    pub kind: EventKind,
    #[serde(default)]
    pub payload: serde_json::Value,
}

impl SessionEvent {
    #[must_use]
    pub fn new(session_id: Uuid, seq: i64, kind: EventKind, payload: serde_json::Value) -> Self {
        Self {
            session_id,
            seq,
            kind,
            payload,
        }
    }

    fn item_id(&self) -> Option<Uuid> {
        self.payload
            .get("item")
            .and_then(serde_json::Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
    }

    fn run_id(&self) -> Option<Uuid> {
        self.payload
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|s| Uuid::parse_str(s).ok())
    }
}

/// Something in the log that the fold cannot justify.
///
/// Not an error: the fold still produces a state, because refusing to read a log
/// is worse than reading it and saying what was odd. The driver treats a
/// non-empty list as a reason to stop and file rather than to keep claiming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anomaly {
    pub seq: i64,
    pub what: String,
}

/// A session's state, replayed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Folded {
    pub state: Option<SessionState>,
    /// The number of events folded, which is also the next `seq`.
    pub event_seq: i64,
    /// Items the session has started a run for, and the run it started.
    pub running: BTreeMap<Uuid, Uuid>,
    /// Items with a verdict, and what it was.
    pub finished: BTreeMap<Uuid, String>,
    /// Items the human approved, for `supervised`.
    pub approved: Vec<Uuid>,
    /// Items the human vetoed. A veto is permanent: §12.12 says a vetoed item
    /// can never be claimed, so this list is consulted before every claim rather
    /// than only at the moment of the veto.
    pub vetoed: Vec<Uuid>,
    /// Items dropped because a duplicate got past the key.
    pub superseded: Vec<Uuid>,
    /// Items that ran out of attempts.
    pub expired: Vec<Uuid>,
    /// Items on the agenda at all, in insert order.
    pub items: Vec<Uuid>,
    /// Runs started, ever. `max_runs` is a budget on this, not on live children.
    pub runs_started: i64,
    /// The charter version the log has reached.
    pub charter_version: i32,
    /// The seq of the last digest, so the cadence is read from the log.
    pub last_digest_seq: Option<i64>,
    pub anomalies: Vec<Anomaly>,
}

impl Folded {
    /// Whether the driver may claim work.
    #[must_use]
    pub fn can_claim(&self) -> bool {
        self.state.as_ref().is_some_and(SessionState::is_running) && self.anomalies.is_empty()
    }

    /// Items with a run in flight.
    #[must_use]
    pub fn concurrent_runs(&self) -> usize {
        self.running.len()
    }

    /// Whether this item has already been started, which is what makes claiming
    /// idempotent on `(session_id, agenda_item_id)`.
    #[must_use]
    pub fn already_started(&self, item: Uuid) -> bool {
        self.running.contains_key(&item) || self.finished.contains_key(&item)
    }
}

/// Replay a session's log.
///
/// The events must be in `seq` order; the unique `(session_id, seq)` index and
/// the trigger that assigns `seq` are what make that free at the read site.
#[must_use]
pub fn fold(events: &[SessionEvent]) -> Folded {
    let mut f = Folded {
        charter_version: 1,
        ..Folded::default()
    };
    let mut expected_seq = 0_i64;

    for e in events {
        if e.seq != expected_seq {
            f.anomalies.push(Anomaly {
                seq: e.seq,
                what: format!(
                    "the log jumps from {expected_seq} to {}: a fold over a log with a hole is a \
                     fold over a different session",
                    e.seq
                ),
            });
        }
        expected_seq = e.seq + 1;
        f.event_seq = e.seq + 1;

        if let Some(next) = e.kind.moves_to() {
            let prev = f.state.as_ref().map(SessionState::word);
            let legal = match prev {
                None => next == OPENING_STATE,
                Some(p) => transition_ok(p, next),
            };
            if legal {
                f.state = Some(state_from(next, &e.payload));
            } else {
                f.anomalies.push(Anomaly {
                    seq: e.seq,
                    what: format!(
                        "{} moves to {next} from {}, which the transition table refuses",
                        e.kind.as_str(),
                        prev.unwrap_or("nothing")
                    ),
                });
            }
            continue;
        }

        match e.kind {
            EventKind::AgendaItemAdded => {
                if let Some(id) = e.item_id() {
                    if !f.items.contains(&id) {
                        f.items.push(id);
                    }
                }
            }
            EventKind::AgendaItemApproved => {
                if let Some(id) = e.item_id() {
                    if !f.approved.contains(&id) {
                        f.approved.push(id);
                    }
                }
            }
            EventKind::AgendaItemVetoed => {
                if let Some(id) = e.item_id() {
                    if !f.vetoed.contains(&id) {
                        f.vetoed.push(id);
                    }
                }
            }
            EventKind::AgendaItemSuperseded => {
                if let Some(id) = e.item_id() {
                    f.superseded.push(id);
                }
            }
            EventKind::AgendaItemExpired => {
                if let Some(id) = e.item_id() {
                    f.expired.push(id);
                }
            }
            EventKind::RunStarted => match (e.item_id(), e.run_id()) {
                (Some(item), Some(run)) => {
                    f.running.insert(item, run);
                    f.runs_started += 1;
                }
                _ => f.anomalies.push(Anomaly {
                    seq: e.seq,
                    what: "run_started with no item or no run_id: the pair is what makes the \
                           child idempotent, so an event missing one cannot be replayed"
                        .into(),
                }),
            },
            EventKind::RunFinished => {
                if let Some(item) = e.item_id() {
                    if f.running.remove(&item).is_none() {
                        f.anomalies.push(Anomaly {
                            seq: e.seq,
                            what: format!(
                                "run_finished for item {item} which the log never started"
                            ),
                        });
                    }
                    let verdict = e
                        .payload
                        .get("verdict")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown")
                        .to_string();
                    f.finished.insert(item, verdict);
                }
            }
            EventKind::CharterAmended => {
                f.charter_version = e
                    .payload
                    .get("version")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|v| i32::try_from(v).ok())
                    .unwrap_or(f.charter_version + 1);
            }
            EventKind::DigestWritten => f.last_digest_seq = Some(e.seq),
            EventKind::AgendaItemMerged
            | EventKind::AwaitingApproval
            // Neither changes the fold. `ProposalAwaiting` deliberately so:
            // the queue of proposals a human owes a decision on is theirs to
            // read, not a number this fold carries, and a fold that tracked it
            // would be one step from a fold that cleared it.
            | EventKind::ProposalAwaiting
            | EventKind::Selected
            | EventKind::WindowClosed
            | EventKind::DayBudgetExhausted
            | EventKind::Note => {}
            // Every transition kind returned above.
            EventKind::SessionCreated
            | EventKind::SessionStarted
            | EventKind::SessionPaused
            | EventKind::SessionResumed
            | EventKind::SessionStalled
            | EventKind::SessionCompleted
            | EventKind::SessionStopped => unreachable!("handled by moves_to"),
        }
    }
    f
}

fn state_from(word: &str, payload: &serde_json::Value) -> SessionState {
    let text = |key: &str, fallback: &str| -> String {
        payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or(fallback)
            .to_string()
    };
    match word {
        "draft" => SessionState::Draft,
        "running" => SessionState::Running,
        "paused" => SessionState::Paused {
            why: text("why", "unstated"),
        },
        "stalled" => SessionState::Stalled {
            diagnosis: text("diagnosis", "unstated"),
        },
        "complete" => SessionState::Complete,
        "stopped" => SessionState::Stopped,
        other => unreachable!("state word {other} is not one of SessionState::WORDS"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, kind: EventKind, payload: serde_json::Value) -> SessionEvent {
        SessionEvent::new(Uuid::nil(), seq, kind, payload)
    }

    fn opened() -> Vec<SessionEvent> {
        vec![
            ev(0, EventKind::SessionCreated, json!({})),
            ev(1, EventKind::SessionStarted, json!({})),
        ]
    }

    #[test]
    fn a_log_opens_in_draft_and_nothing_else() {
        let f = fold(&[ev(0, EventKind::SessionStarted, json!({}))]);
        assert_eq!(f.state, None);
        assert_eq!(f.anomalies.len(), 1);
        assert!(f.anomalies[0].what.contains("running"));
    }

    #[test]
    fn stopped_is_the_only_terminal_state() {
        for w in SessionState::WORDS {
            let onward: Vec<&&str> = SessionState::WORDS
                .iter()
                .filter(|n| transition_ok(w, n))
                .collect();
            if *w == "stopped" {
                assert!(onward.is_empty(), "stopped moved to {onward:?}");
            } else {
                assert!(!onward.is_empty(), "{w} is a dead end and should not be");
            }
        }
        // And complete is not terminal: a user adding an item restarts it.
        assert!(transition_ok("complete", "running"));
    }

    #[test]
    fn a_state_never_transitions_to_itself() {
        for w in SessionState::WORDS {
            assert!(!transition_ok(w, w), "{w} -> {w}");
        }
    }

    #[test]
    fn restarting_between_start_and_record_re_derives_the_same_state() {
        // AT-59's form: the fold before and after a driver's death is identical,
        // because the fold is all there is.
        let item = Uuid::new_v4();
        let run = Uuid::new_v4();
        let mut log = opened();
        log.push(ev(2, EventKind::AgendaItemAdded, json!({"item": item})));
        log.push(ev(3, EventKind::Selected, json!({"item": item})));

        let before = fold(&log);
        assert!(before.can_claim());
        assert!(!before.already_started(item));

        // The driver dies here, restarts, folds the same log, and gets the same
        // answer: this item has not been started.
        assert_eq!(fold(&log), before);

        log.push(ev(
            4,
            EventKind::RunStarted,
            json!({"item": item, "run_id": run}),
        ));
        let after = fold(&log);
        assert!(after.already_started(item));
        assert_eq!(after.running.get(&item), Some(&run));
        assert_eq!(after.runs_started, 1);
        assert_eq!(after.event_seq, 5);
    }

    #[test]
    fn a_finish_the_log_never_started_is_an_anomaly_and_stops_claiming() {
        let item = Uuid::new_v4();
        let mut log = opened();
        log.push(ev(
            2,
            EventKind::RunFinished,
            json!({"item": item, "verdict": "supported"}),
        ));
        let f = fold(&log);
        assert_eq!(f.finished.get(&item).map(String::as_str), Some("supported"));
        assert_eq!(f.anomalies.len(), 1);
        assert!(!f.can_claim(), "a fold with an anomaly must not claim");
    }

    #[test]
    fn a_hole_in_the_log_is_an_anomaly() {
        let mut log = opened();
        log.push(ev(5, EventKind::AgendaItemAdded, json!({"item": Uuid::new_v4()})));
        let f = fold(&log);
        assert_eq!(f.anomalies.len(), 1);
        assert!(f.anomalies[0].what.contains("hole"));
    }

    #[test]
    fn pause_and_stall_keep_their_reason() {
        let mut log = opened();
        log.push(ev(2, EventKind::SessionPaused, json!({"why": "kill_switch"})));
        assert_eq!(
            fold(&log).state,
            Some(SessionState::Paused {
                why: "kill_switch".into()
            })
        );
        log.push(ev(3, EventKind::SessionResumed, json!({})));
        log.push(ev(
            4,
            EventKind::SessionStalled,
            json!({"diagnosis": "barren_streak"}),
        ));
        let f = fold(&log);
        assert_eq!(
            f.state,
            Some(SessionState::Stalled {
                diagnosis: "barren_streak".into()
            })
        );
        assert!(!f.can_claim());
        assert!(f.anomalies.is_empty());
    }

    #[test]
    fn a_veto_survives_every_later_event() {
        let item = Uuid::new_v4();
        let mut log = opened();
        log.push(ev(2, EventKind::AgendaItemAdded, json!({"item": item})));
        log.push(ev(3, EventKind::AgendaItemVetoed, json!({"item": item})));
        log.push(ev(4, EventKind::DigestWritten, json!({})));
        let f = fold(&log);
        assert_eq!(f.vetoed, vec![item]);
        assert_eq!(f.last_digest_seq, Some(4));
    }

    #[test]
    fn every_event_kind_word_round_trips_and_transitions_are_declared_once() {
        for k in EventKind::ALL {
            assert_eq!(EventKind::parse(k.as_str()), Some(*k));
            if let Some(w) = k.moves_to() {
                assert!(
                    SessionState::WORDS.contains(&w),
                    "{} moves to {w}, which is not a state",
                    k.as_str()
                );
            }
        }
    }

    #[test]
    fn a_proposal_awaiting_and_a_note_are_their_own_kinds_and_change_nothing() {
        // The driver had neither, so it filed both as kinds that mean something
        // else: a proposal waiting for a human as `awaiting_approval` (which is
        // about an agenda item) and a plain note as `agenda_item_merged` (which
        // is about a merge that did not happen). Both now have a word.
        let item = Uuid::new_v4();
        let quiet = {
            let mut log = opened();
            log.push(ev(2, EventKind::AgendaItemAdded, json!({"item": item})));
            log
        };
        let noisy = {
            let mut log = quiet.clone();
            log.push(ev(3, EventKind::ProposalAwaiting, json!({"proposal": "p_1"})));
            log.push(ev(4, EventKind::Note, json!({"what": "an offer was skipped"})));
            log
        };

        let mut checked = 0;
        for kind in [EventKind::ProposalAwaiting, EventKind::Note] {
            assert!(EventKind::ALL.contains(&kind), "{kind:?} is not in ALL");
            assert_eq!(EventKind::parse(kind.as_str()), Some(kind));
            assert_eq!(
                kind.moves_to(),
                None,
                "{} is not a transition and must carry NULL state",
                kind.as_str()
            );
            checked += 1;
        }
        assert_eq!(checked, 2);

        assert_eq!(EventKind::parse("proposal_awaiting"), Some(EventKind::ProposalAwaiting));
        assert_eq!(EventKind::parse("note"), Some(EventKind::Note));

        // Neither reaches the fold. In particular nothing about the proposal
        // enters `Folded`, so there is no field a session could later clear:
        // the human's queue stays the human's. `event_seq` is the position the
        // log has reached and of course moves; everything else must not.
        let quiet_folded = fold(&quiet);
        let mut noisy_folded = fold(&noisy);
        assert_eq!(
            noisy_folded.event_seq,
            quiet_folded.event_seq + 2,
            "both events were read"
        );
        assert_eq!(quiet_folded.items, vec![item], "the fixture folded nothing");
        noisy_folded.event_seq = quiet_folded.event_seq;
        assert_eq!(noisy_folded, quiet_folded);
        assert!(
            noisy_folded.anomalies.is_empty(),
            "{:?}",
            noisy_folded.anomalies
        );

        // P6, held here as well as in `harvest`: `proposal_awaiting` is the
        // **only** kind in this vocabulary that names a proposal, so there is
        // no word a session could append to decide one. The anchor is that it
        // found exactly one; a scan finding none would be a scan of nothing.
        let about_proposals: Vec<&str> = EventKind::ALL
            .iter()
            .map(|k| k.as_str())
            .filter(|w| w.contains("proposal"))
            .collect();
        assert_eq!(
            about_proposals,
            vec!["proposal_awaiting"],
            "a kind that decides a proposal is a session that approves its own"
        );
    }

    #[test]
    fn no_two_event_kinds_share_a_word() {
        let mut words: Vec<&str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
        assert!(!words.is_empty());
        let before = words.len();
        words.sort_unstable();
        words.dedup();
        assert_eq!(before, words.len(), "two kinds parse to one: {words:?}");
    }

    #[test]
    fn nothing_after_stopped_is_folded_into_the_state() {
        let mut log = opened();
        log.push(ev(2, EventKind::SessionStopped, json!({})));
        log.push(ev(3, EventKind::SessionResumed, json!({})));
        let f = fold(&log);
        assert_eq!(f.state, Some(SessionState::Stopped));
        assert_eq!(f.anomalies.len(), 1);
    }
}
