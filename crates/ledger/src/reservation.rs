//! A look, authorised at the door (ADR-S10-10; migration 0132).
//!
//! # Two questions, two tables
//!
//! `mlops.trial` answers *what compute ran, under what config, over what data*.
//! Every one of its required columns is a fact about compute, which is why
//! `backtest::run` is the thing that writes it: at the moment a Run is
//! dispatched, all of them are known.
//!
//! A **tool call** knows none of them. `run_sweep` arrives with arguments and
//! the sweep picks the configs and the data slice afterwards. So the question
//! the gate is actually asking — *may this run spend, and has it spent one* —
//! had no table, and the route it posted to could not be written. That is the
//! whole of OQ-20.
//!
//! This is that table's type. It is deliberately **not** a `TrialTicket`: the
//! ticket is sealed, minted only by a ledger, and stands for compute that ran. A
//! reservation stands for permission that was granted, and conflating the two
//! would put the looser of them in front of every statistic that reads the
//! stricter.
//!
//! # What it is not
//!
//! Not a counter the exploration floor reads. `trials_counted`, the floor's
//! denominator and `N_eff` all read `mlops.trial` and are untouched by this —
//! which is the main argument for a second table rather than a second grain in
//! the first.
//!
//! Not reconciled. `settled_trial_id` exists and nothing sets it, so an
//! authorised look that never ran is not yet distinguishable from one that did.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::LedgerError;

/// What the gate asks for before a trial-spending tool reaches its handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reserve {
    pub tenant_id: String,
    /// The run that made the call, when there is one. A front door with no agent
    /// run behind it still spends against a budget.
    pub run_id: Option<Uuid>,
    pub tool: String,
    pub tool_version: i32,
    /// `search` or `falsification`. `none` is not a value here: a reservation
    /// exists because a tool declared a trial.
    pub kind: String,
    /// The run's sealed pre-registration (AGENT-013 §7.2).
    ///
    /// **Required.** §7.2 says every ticket a sealed run spends carries the
    /// hash, and until this existed no path delivered one: the gate carried it
    /// on `TrialRequest` and the registrar never put it in the body it posted.
    pub prereg_hash: String,
    /// The call's arguments by content.
    pub arguments_hash: String,
}

/// The kinds a reservation may carry, mirroring the catalogue's `TrialKind`
/// minus `None`.
///
/// Stated here and CHECKed by `0132`. `crates/invariants` holds the two equal —
/// a kind the catalogue declares and this refuses is a tool that cannot be
/// called at all, which is the failure OQ-20 was.
pub const RESERVABLE_KINDS: &[&str] = &["search", "falsification"];

impl Reserve {
    /// Structural checks that hold before any backend is involved.
    ///
    /// The database repeats each one; this is the half that fails fast and says
    /// which field.
    ///
    /// # Errors
    /// [`LedgerError::Invalid`] naming the field and why it is required.
    pub fn validate(&self) -> Result<(), LedgerError> {
        let invalid = |m: &str| Err(LedgerError::Invalid(m.to_string()));
        if self.tool.trim().is_empty() {
            return invalid("a reservation names the tool whose call it authorises");
        }
        if !RESERVABLE_KINDS.contains(&self.kind.as_str()) {
            return invalid(&format!(
                "`{}` is not a reservable trial kind; a reservation exists because a tool \
                 declared a trial, so `none` is not one of them",
                self.kind
            ));
        }
        if self.prereg_hash.trim().is_empty() {
            return invalid(
                "a reservation carries the run's sealed pre-registration (AGENT-013 §7.2): a \
                 look with no commitment behind it is the thing the gate exists to refuse, and \
                 a ticket that does not carry the hash cannot be joined to what it was spent \
                 under",
            );
        }
        if self.arguments_hash.trim().is_empty() {
            return invalid("a reservation hashes the arguments it authorised");
        }
        Ok(())
    }
}

/// The row that now exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reserved {
    pub reservation_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserve() -> Reserve {
        Reserve {
            tenant_id: "t".into(),
            run_id: Some(Uuid::new_v4()),
            tool: "run_sweep".into(),
            tool_version: 1,
            kind: "search".into(),
            prereg_hash: "sha256:abc".into(),
            arguments_hash: "sha256:def".into(),
        }
    }

    #[test]
    fn a_complete_reservation_validates() {
        reserve().validate().expect("complete");
    }

    /// The field §7.2 promised and no path delivered.
    #[test]
    fn a_reservation_without_the_registration_is_refused() {
        let mut r = reserve();
        r.prereg_hash = "  ".into();
        let err = r.validate().expect_err("no commitment behind the look");
        assert!(format!("{err}").contains("7.2"), "{err}");
    }

    /// `none` is not a reservable kind. A reservation exists *because* a tool
    /// declared a trial, so the absence of one cannot be reserved.
    #[test]
    fn the_absence_of_a_trial_is_not_reservable() {
        let mut r = reserve();
        r.kind = "none".into();
        assert!(r.validate().is_err());
        for kind in RESERVABLE_KINDS {
            let mut ok = reserve();
            ok.kind = (*kind).to_string();
            ok.validate().unwrap_or_else(|e| panic!("{kind}: {e}"));
        }
    }

    /// A run id is optional and everything else is not.
    ///
    /// The MCP front door may have no agent run behind it, and refusing the
    /// reservation for want of one would refuse the call — which is the failure
    /// this whole table exists to end.
    #[test]
    fn a_reservation_without_a_run_is_still_a_reservation() {
        let mut r = reserve();
        r.run_id = None;
        r.validate().expect("a front door with no run still spends");
    }
}
