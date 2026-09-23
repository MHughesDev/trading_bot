//! Take out what nothing reaches, before the first trial (AGENT-013 §5; Set S
//! task S-10.5).
//!
//! # Why a simplification is a research act and not tidying
//!
//! The validator checks references **forwards**: every `signal.when` names a real
//! condition, every `action.on_signal` names a real emit. It does not check
//! backwards, so a condition no signal watches and a signal no action consumes
//! are both accepted. They cost nothing at run time — and that is the problem.
//! They cost at *design* time:
//!
//! * a parameter used only by a dead node is still swept, and every member of
//!   that sweep is a counted trial spent moving a number nothing reads;
//! * a strategy that looks like eleven nodes and behaves like four is graded,
//!   compared and written up as the eleven, so the complexity a reviewer is
//!   asked to judge is not the complexity that ran;
//! * and a dead node is the cheapest possible false lead when a run is asked
//!   *why* a result moved.
//!
//! So this runs before the design is bound, and what it removes is recorded as
//! [`Move::RemoveNode`] — the same vocabulary a worker uses to propose a
//! structural change, because "we took this out" is the same kind of fact
//! whoever did it.
//!
//! # What it will not touch, and why that is the important half
//!
//! Only `condition` and `signal` nodes, and only when nothing reaches them from
//! an action **or an exit rule**. Three deliberate limits:
//!
//! * **Exits are roots.** `ExitRule::OnSignal { emit }` consumes a signal, and a
//!   reachability walk that started only from `actions` would delete the signal
//!   a strategy exits on. That is not a hypothetical: `ADR-S5-02` records the
//!   mirror-image bug, where a rising edge was counted as an entry for a signal
//!   no action named.
//! * **The v1.5 pipeline nodes are left alone.** `DataSource`, `Rank`, `Filter`,
//!   `Sizing` and the inference nodes form a second graph with its own
//!   topology, and removing a node whose reachability this module does not model
//!   would be guessing with somebody's strategy. They are *reported* as
//!   unreferenced-by-the-signal-graph and never removed.
//! * **Nothing is rewritten.** No condition is folded, no expression is
//!   simplified, no two nodes are merged. Every one of those changes what the
//!   strategy computes if it is wrong, and this is meant to be a change that
//!   cannot be wrong.
//!
//! It is therefore deliberately weaker than "simplify" suggests, and the name is
//! the spec's. [`Simplification::is_behaviour_preserving`] states the claim the
//! module actually makes.

use std::collections::BTreeSet;

use domain::strategy_def::exits::{ExitPolicy, ExitRule};
use domain::strategy_def::nodes::NodeKind;
use domain::strategy_def::StrategyDefinition;

use crate::propose::Move;

/// Why a node was removed, or why one that looks dead was kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No signal watches this condition.
    ConditionNothingWatches,
    /// No action and no exit rule consumes this signal.
    SignalNothingConsumes,
    /// Unreferenced by the signal graph, and **kept**: this module does not
    /// model what reaches it.
    KeptUnmodelledKind,
}

impl Reason {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConditionNothingWatches => "no signal watches this condition",
            Self::SignalNothingConsumes => "no action and no exit rule consumes this signal",
            Self::KeptUnmodelledKind => {
                "unreferenced by the signal graph and kept: this node kind belongs to the \
                 universe pipeline, whose reachability this pass does not model"
            }
        }
    }

    /// Whether a node with this reason was actually taken out.
    #[must_use]
    pub fn removed(self) -> bool {
        !matches!(self, Self::KeptUnmodelledKind)
    }
}

/// What one pass found.
#[derive(Debug, Clone, PartialEq)]
pub struct Simplification {
    /// The definition with the dead nodes gone. Identical to the input when
    /// nothing was dead — including when nodes were *found* and kept.
    pub simplified: StrategyDefinition,
    /// One per removed node, in the vocabulary a worker proposes changes in.
    pub moves: Vec<Move>,
    /// `(node id, reason)` for everything this pass has an opinion about,
    /// removed or kept.
    pub findings: Vec<(String, Reason)>,
}

impl Simplification {
    /// Whether anything changed.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.moves.is_empty()
    }

    /// The claim this module makes, stated rather than implied.
    ///
    /// Everything removed was unreachable from every action and every exit rule,
    /// so no bar can produce a different command. It is not a claim that the
    /// *result* is identical — a sweep over a parameter only a dead node used
    /// will now enumerate fewer members, which is the saving, and a run's trial
    /// count legitimately changes.
    #[must_use]
    pub fn is_behaviour_preserving(&self) -> bool {
        true
    }

    /// Node ids this pass found unreferenced and did not remove.
    #[must_use]
    pub fn kept(&self) -> Vec<&str> {
        self.findings
            .iter()
            .filter(|(_, r)| !r.removed())
            .map(|(id, _)| id.as_str())
            .collect()
    }
}

/// Every signal name something consumes: an action, or an exit rule.
fn consumed_signals(def: &StrategyDefinition) -> BTreeSet<&str> {
    let mut out: BTreeSet<&str> = def.actions.iter().map(|a| a.on_signal.as_str()).collect();
    // Exits are roots. A walk that started only from `actions` would delete the
    // signal a strategy exits on.
    if let Some(ExitPolicy::Rules { rules, .. }) = &def.exits {
        for rule in rules {
            if let ExitRule::OnSignal { emit } = rule {
                out.insert(emit.as_str());
            }
        }
    }
    out
}

/// Remove what nothing reaches.
///
/// Iterated to a fixed point: removing a signal can orphan the condition it
/// watched, and one pass would leave that condition behind. Bounded by the node
/// count, because each round removes at least one node or stops.
#[must_use]
pub fn simplify(def: &StrategyDefinition) -> Simplification {
    let mut current = def.clone();
    let mut moves: Vec<Move> = Vec::new();
    let mut findings: Vec<(String, Reason)> = Vec::new();

    loop {
        let consumed = consumed_signals(&current);
        // Conditions any surviving signal watches.
        let watched: BTreeSet<&str> = current
            .nodes
            .iter()
            .filter_map(|n| match &n.kind {
                // Only a signal that is itself consumed keeps its condition
                // alive. A dead signal's condition is dead too, and this is
                // what makes the fixed point converge in one round rather than
                // needing the loop — the loop stays because a future node kind
                // could chain further.
                NodeKind::Signal { when, emit } if consumed.contains(emit.as_str()) => {
                    Some(when.as_str())
                }
                _ => None,
            })
            .collect();

        let mut doomed: Vec<(String, Reason)> = Vec::new();
        for node in &current.nodes {
            match &node.kind {
                NodeKind::Signal { emit, .. } if !consumed.contains(emit.as_str()) => {
                    doomed.push((node.id.clone(), Reason::SignalNothingConsumes));
                }
                NodeKind::Condition { .. } if !watched.contains(node.id.as_str()) => {
                    doomed.push((node.id.clone(), Reason::ConditionNothingWatches));
                }
                _ => {}
            }
        }
        if doomed.is_empty() {
            break;
        }
        for (id, reason) in &doomed {
            moves.push(Move::RemoveNode { node: id.clone() });
            findings.push((id.clone(), *reason));
        }
        let gone: BTreeSet<&str> = doomed.iter().map(|(id, _)| id.as_str()).collect();
        current.nodes.retain(|n| !gone.contains(n.id.as_str()));
    }

    // Everything else that nothing in the signal graph names: reported, never
    // removed. This pass does not model what reaches a universe-pipeline node,
    // and guessing with somebody's strategy is the one thing a simplification
    // must not do.
    let named: BTreeSet<&str> = current
        .nodes
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Signal { when, .. } => Some(when.as_str()),
            _ => None,
        })
        .collect();
    for node in &current.nodes {
        let unmodelled = !matches!(
            node.kind,
            NodeKind::Condition { .. } | NodeKind::Signal { .. }
        );
        if unmodelled && !named.contains(node.id.as_str()) {
            findings.push((node.id.clone(), Reason::KeptUnmodelledKind));
        }
    }

    Simplification {
        simplified: current,
        moves,
        findings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::order::Side;
    use domain::strategy_def::actions::{Action, ActionKind, OrderSpec, SizeMode};
    use domain::strategy_def::nodes::Node;

    fn condition(id: &str) -> Node {
        Node {
            id: id.to_string(),
            kind: NodeKind::Condition {
                expr: "feature('ema_10') > feature('ema_20')".to_string(),
            },
        }
    }

    fn signal(id: &str, when: &str, emit: &str) -> Node {
        Node {
            id: id.to_string(),
            kind: NodeKind::Signal {
                when: when.to_string(),
                emit: emit.to_string(),
            },
        }
    }

    fn action(on_signal: &str) -> Action {
        Action {
            on_signal: on_signal.to_string(),
            kind: ActionKind::PlaceOrder {
                order: OrderSpec {
                    side: Side::Buy,
                    size_mode: SizeMode::Fixed,
                    size: "0.01".to_string(),
                },
            },
        }
    }

    /// Built from the serde shape rather than a struct literal, so this fixture
    /// stays valid as the definition grows fields — and so it is exactly what a
    /// stored strategy deserialises to.
    fn def(nodes: Vec<Node>, actions: Vec<Action>) -> StrategyDefinition {
        let mut d: StrategyDefinition = serde_json::from_value(serde_json::json!({
            "strategy_id": "s",
            "definition_version": "1.0",
            "asset_class": "crypto_spot_cex",
            "inputs": [],
            "nodes": [],
            "actions": []
        }))
        .expect("a minimal definition");
        d.nodes = nodes;
        d.actions = actions;
        d
    }

    /// The ordinary case: a live chain is untouched.
    #[test]
    fn a_strategy_where_everything_is_reached_is_unchanged() {
        let d = def(
            vec![condition("c1"), signal("s1", "c1", "long")],
            vec![action("long")],
        );
        let out = simplify(&d);
        assert!(!out.changed());
        assert_eq!(out.simplified, d);
    }

    /// A signal nothing consumes goes, and so does the condition it watched.
    #[test]
    fn a_dead_signal_takes_its_condition_with_it() {
        let d = def(
            vec![
                condition("c1"),
                signal("s1", "c1", "long"),
                condition("c_dead"),
                signal("s_dead", "c_dead", "never_used"),
            ],
            vec![action("long")],
        );
        let out = simplify(&d);
        assert!(out.changed());
        let left: Vec<&str> = out.simplified.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(left, vec!["c1", "s1"]);
        assert_eq!(out.moves.len(), 2);
        assert!(out
            .moves
            .iter()
            .any(|m| matches!(m, Move::RemoveNode { node } if node == "s_dead")));
        assert!(out
            .moves
            .iter()
            .any(|m| matches!(m, Move::RemoveNode { node } if node == "c_dead")));
    }

    /// **The one that would have been a real bug.** A signal consumed only by an
    /// exit rule is live, and a walk that started at `actions` alone would have
    /// deleted the signal the strategy exits on.
    #[test]
    fn a_signal_only_an_exit_rule_consumes_is_not_dead() {
        let mut d = def(
            vec![
                condition("c1"),
                signal("s1", "c1", "long"),
                condition("c_exit"),
                signal("s_exit", "c_exit", "get_out"),
            ],
            vec![action("long")],
        );
        d.exits = Some(ExitPolicy::Rules {
            rules: vec![ExitRule::OnSignal {
                emit: "get_out".into(),
            }],
        });
        let out = simplify(&d);
        assert!(
            !out.changed(),
            "the exit signal was removed: {:?}",
            out.moves
        );
    }

    /// A condition no signal watches goes on its own.
    #[test]
    fn a_condition_nothing_watches_goes() {
        let d = def(
            vec![
                condition("c1"),
                signal("s1", "c1", "long"),
                condition("c_orphan"),
            ],
            vec![action("long")],
        );
        let out = simplify(&d);
        assert_eq!(out.moves.len(), 1);
        assert_eq!(
            out.findings
                .iter()
                .find(|(id, _)| id == "c_orphan")
                .map(|(_, r)| *r),
            Some(Reason::ConditionNothingWatches)
        );
    }

    /// Removal reaches a fixed point rather than one layer.
    #[test]
    fn a_chain_of_dead_nodes_goes_entirely() {
        let d = def(
            vec![
                condition("c1"),
                signal("s1", "c1", "long"),
                condition("c_a"),
                signal("s_a", "c_a", "unused_a"),
                condition("c_b"),
                signal("s_b", "c_b", "unused_b"),
            ],
            vec![action("long")],
        );
        let out = simplify(&d);
        assert_eq!(out.simplified.nodes.len(), 2);
        assert_eq!(out.moves.len(), 4);
    }

    /// Nothing is rewritten. Every surviving node is byte-identical to the one
    /// it came from — a simplification that edited an expression would be
    /// changing what the strategy computes.
    #[test]
    fn surviving_nodes_are_untouched() {
        let d = def(
            vec![
                condition("c1"),
                signal("s1", "c1", "long"),
                condition("c_orphan"),
            ],
            vec![action("long")],
        );
        let out = simplify(&d);
        for node in &out.simplified.nodes {
            let before = d.nodes.iter().find(|n| n.id == node.id).expect("kept");
            assert_eq!(before, node);
        }
        assert_eq!(out.simplified.actions, d.actions);
        assert_eq!(out.simplified.parameters, d.parameters);
    }

    /// An empty strategy is not made emptier, and does not loop.
    #[test]
    fn a_strategy_with_no_actions_is_reported_and_not_emptied_silently() {
        // Every signal is dead by definition when nothing consumes one, which is
        // a real and loud answer: this definition trades on nothing.
        let d = def(vec![condition("c1"), signal("s1", "c1", "long")], vec![]);
        let out = simplify(&d);
        assert!(out.simplified.nodes.is_empty());
        assert_eq!(out.moves.len(), 2);
    }

    /// The claim is stated rather than implied.
    #[test]
    fn the_pass_says_what_it_preserves() {
        let out = simplify(&def(vec![], vec![]));
        assert!(out.is_behaviour_preserving());
        assert!(!out.changed());
    }
}
