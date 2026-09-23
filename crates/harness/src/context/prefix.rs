//! Cross-turn prefix continuity (AGENT-005 §21, §30.7; Set S task S-9.9).
//!
//! # The waste
//!
//! Every turn re-assembles its prompt from scratch, so the provider cache is
//! cold at the start of every turn — even though the charter is identical, most
//! of the tool slice is identical, and the previous summary is a *prefix* of
//! this turn's history rather than something inserted into the middle of it.
//!
//! Ten turns should pay one cold prefix, not ten.
//!
//! # What "superset" means mechanically
//!
//! A cached prefix matches by **bytes**, from the start, until the first
//! difference. So a prefix is continuous with the previous turn's exactly when
//! the previous turn's bytes are a prefix of this one's. Three rules follow, and
//! they are the whole module:
//!
//! * **Same charter fragment.** A different `GoalKind` is a different prefix, and
//!   there is no way around that — which is why S-9.8's fragments are byte-stable
//!   *per kind* rather than per run.
//! * **Tools append, never reorder and never drop.** Reordering the tools array
//!   invalidates the cache exactly as thoroughly as removing one. This is the
//!   same rule `Loop`'s `session_exposure` already applies within a turn, carried
//!   across the turn boundary — which is where it was being thrown away.
//! * **The summary is appended, not inserted.** Rewriting the earlier history to
//!   fold in what the last turn concluded moves every byte after it.
//!
//! # It reports rather than enforces
//!
//! [`Prefix::extend`] returns what it had to break, if anything. A ledger that
//! silently repaired a discontinuity would hide the one thing worth knowing: a
//! run whose kind changed mid-conversation *should* pay a second cold prefix, and
//! a run paying ten is a defect.

use std::collections::BTreeSet;

/// One conversation's prefix, carried across turns.
#[derive(Debug, Clone, Default)]
pub struct Prefix {
    charter: String,
    tools: Vec<String>,
    turns: u32,
    /// Turns that could not continue the previous prefix, and why.
    breaks: Vec<String>,
}

/// What one turn did to the prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Continuity {
    /// The tool slice this turn should send, in order.
    pub tools: Vec<String>,
    /// Tools this turn added to the end.
    pub appended: Vec<String>,
    /// `None` when the turn continued the previous prefix.
    pub broken_by: Option<String>,
}

impl Continuity {
    #[must_use]
    pub fn is_continuous(&self) -> bool {
        self.broken_by.is_none()
    }
}

impl Prefix {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The tool order the next turn must start from.
    ///
    /// Handed to `Loop::with_prior_exposure`, so routing appends to it rather
    /// than choosing a fresh order. Without this the second turn re-routes from
    /// nothing and the slice comes back in whatever order routing produced —
    /// usually the same tools, often not the same order, and the cache cannot
    /// tell the difference between those two.
    #[must_use]
    pub fn tools(&self) -> &[String] {
        &self.tools
    }

    #[must_use]
    pub fn turns(&self) -> u32 {
        self.turns
    }

    /// Turns that paid a cold prefix, with the reason each one did.
    #[must_use]
    pub fn breaks(&self) -> &[String] {
        &self.breaks
    }

    /// Fold one turn in, and say whether it continued the prefix.
    ///
    /// `tools_used` is what the turn actually ended up exposing. Order within it
    /// is ignored for tools already in the prefix: the prefix's own order wins,
    /// because that is the order already in the cache.
    pub fn extend(&mut self, charter: &str, tools_used: &[String]) -> Continuity {
        let first_turn = self.turns == 0;
        let mut broken_by = None;

        if first_turn {
            self.charter = charter.to_string();
        } else if self.charter != charter {
            // A different charter fragment is a different prefix from byte one.
            // Recorded rather than repaired: a conversation whose goal kind
            // changed *should* pay for it once.
            broken_by = Some("charter fragment changed".to_string());
            self.charter = charter.to_string();
            self.tools.clear();
        }

        let known: BTreeSet<&String> = self.tools.iter().collect();
        let appended: Vec<String> = tools_used
            .iter()
            .filter(|t| !known.contains(*t))
            .cloned()
            .collect();
        // Dropping is not an error and is not honoured: a tool the turn did not
        // need stays in the slice, because removing it would invalidate every
        // byte after it for the sake of a schema nobody reads.
        self.tools.extend(appended.iter().cloned());

        self.turns += 1;
        if first_turn {
            broken_by = Some("first turn".to_string());
        }
        if let Some(reason) = &broken_by {
            self.breaks.push(format!("turn {}: {reason}", self.turns));
        }
        Continuity {
            tools: self.tools.clone(),
            appended,
            broken_by,
        }
    }

    /// Cold prefixes this conversation has paid for.
    ///
    /// One is correct — the first turn. More than one means something changed
    /// that did not have to.
    #[must_use]
    pub fn cold_prefixes(&self) -> usize {
        self.breaks.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    /// Ten turns pay one cold prefix. The acceptance.
    #[test]
    fn ten_turns_pay_one_cold_prefix() {
        let mut p = Prefix::new();
        let charter = "identity + tools + termination";
        for turn in 0..10 {
            let c = p.extend(
                charter,
                &names(&["search_tools", "get_bars", "finish_task"]),
            );
            if turn == 0 {
                assert!(!c.is_continuous(), "the first turn is always cold");
            } else {
                assert!(c.is_continuous(), "turn {turn}: {:?}", c.broken_by);
                assert!(c.appended.is_empty());
            }
        }
        assert_eq!(p.cold_prefixes(), 1, "{:?}", p.breaks());
    }

    /// A turn that needs a new tool appends it rather than reordering.
    #[test]
    fn a_new_tool_is_appended_not_woven_in() {
        let mut p = Prefix::new();
        p.extend("c", &names(&["search_tools", "get_bars"]));
        // Routing returned them in a different order and added one.
        let c = p.extend("c", &names(&["run_backtest", "get_bars", "search_tools"]));
        assert_eq!(
            c.tools,
            names(&["search_tools", "get_bars", "run_backtest"]),
            "the prefix's own order wins: it is the order already in the cache"
        );
        assert_eq!(c.appended, names(&["run_backtest"]));
        assert!(
            c.is_continuous(),
            "appending is the one change a cached prefix survives"
        );
    }

    /// A tool the turn did not need stays in the slice.
    #[test]
    fn a_tool_is_never_dropped_from_the_prefix() {
        let mut p = Prefix::new();
        p.extend("c", &names(&["search_tools", "get_bars", "run_backtest"]));
        let c = p.extend("c", &names(&["search_tools"]));
        assert_eq!(
            c.tools,
            names(&["search_tools", "get_bars", "run_backtest"])
        );
        assert!(
            c.is_continuous(),
            "removing a schema nobody reads would invalidate every byte after it"
        );
    }

    /// A changed charter fragment is a real break, and it is reported.
    ///
    /// Not repaired: a conversation whose goal kind changed should pay for it
    /// once, and hiding that would hide the case where it happens ten times.
    #[test]
    fn a_changed_charter_breaks_the_prefix_and_says_so() {
        let mut p = Prefix::new();
        p.extend("lookup charter", &names(&["search_tools"]));
        let c = p.extend("research charter", &names(&["search_tools"]));
        assert!(!c.is_continuous());
        assert_eq!(c.broken_by.as_deref(), Some("charter fragment changed"));
        assert_eq!(p.cold_prefixes(), 2);
        assert!(
            p.breaks().iter().any(|b| b.contains("turn 2")),
            "{:?}",
            p.breaks()
        );
    }

    /// A conversation that keeps its kind never pays twice, however long it runs.
    #[test]
    fn a_long_conversation_that_keeps_its_kind_stays_warm() {
        let mut p = Prefix::new();
        for turn in 0..40u32 {
            // Each turn discovers one more tool, which is the worst realistic
            // case for a slice that must stay append-only.
            let mut tools = names(&["search_tools"]);
            tools.push(format!("tool_{turn}"));
            p.extend("c", &tools);
        }
        assert_eq!(p.cold_prefixes(), 1);
        assert_eq!(p.tools().len(), 41);
        assert_eq!(
            p.tools()[0],
            "search_tools",
            "and the first byte of the slice never moved"
        );
    }
}
