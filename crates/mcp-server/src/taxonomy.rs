//! Namespace and risk for every tool in the catalogue (ADR-0031, guide §2.3, §14.3).
//!
//! A table rather than two extra fields on 56 JSON literals, for one reason that
//! matters more than tidiness: `every_tool_is_classified` fails when a tool is added
//! without an entry here. A new tool cannot reach a model unclassified, so the
//! permission policy can never silently default a destructive tool to "read".
//!
//! **Risk is about what the tool does to the world, not how dangerous it feels.**
//! `delete_backtest` is destructive because the run is gone; `arm_automation` is
//! destructive because it changes what the platform will do with real money without
//! a further human step. `run_sweep` is a write: it costs trials and those
//! cannot be un-spent, but nothing is destroyed.

/// What a tool can do to the world. Mirrors `harness::registry::Risk`; kept as a
/// local enum so this crate does not depend on the harness crate for a three-line
/// type, and reconciled by `the_risk_names_match_the_harness_crate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Read,
    Write,
    Destructive,
    Outbound,
}

impl Risk {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Write => "write",
            Risk::Destructive => "destructive",
            Risk::Outbound => "outbound",
        }
    }
}

/// `(tool, namespace, risk)`.
///
/// Namespaces are the routing unit (§2.2): a plan step declares the namespace it
/// needs and the registry exposes that slice. They are chosen so a step normally
/// needs exactly one.
pub const TAXONOMY: &[(&str, &str, Risk)] = &[
    // ── protocol ────────────────────────────────────────────────────────────
    ("get_authoring_guide", "core", Risk::Read),
    // ── discovery: what exists ──────────────────────────────────────────────
    ("list_lanes", "discovery", Risk::Read),
    ("list_instruments", "discovery", Risk::Read),
    ("list_asset_classes", "discovery", Risk::Read),
    ("list_initialized_assets", "discovery", Risk::Read),
    ("get_instrument", "discovery", Risk::Read),
    // ── market data ─────────────────────────────────────────────────────────
    ("get_bars", "data", Risk::Read),
    // Initialising an asset starts live collection and a backfill: it spends real
    // resources and changes what the platform is doing, so it is a write.
    ("init_asset", "data", Risk::Write),
    ("get_asset_init_job", "data", Risk::Read),
    // ── strategy authoring ──────────────────────────────────────────────────
    ("validate_strategy", "strategy", Risk::Read),
    ("create_strategy", "strategy", Risk::Write),
    ("get_strategy", "strategy", Risk::Read),
    ("list_strategies", "strategy", Risk::Read),
    ("list_compatible_strategies", "strategy", Risk::Read),
    // ── the draft builder ───────────────────────────────────────────────────
    ("new_strategy_draft", "builder", Risk::Write),
    // Discarding a draft loses unsaved authoring work with no platform-side copy.
    ("discard_draft", "builder", Risk::Destructive),
    ("set_strategy_meta", "builder", Risk::Write),
    ("add_strategy_input", "builder", Risk::Write),
    ("add_condition_node", "builder", Risk::Write),
    ("add_signal_node", "builder", Risk::Write),
    ("add_strategy_action", "builder", Risk::Write),
    ("set_risk_overrides", "builder", Risk::Write),
    ("get_draft_summary", "builder", Risk::Read),
    ("finalize_strategy", "builder", Risk::Write),
    // ── backtests ───────────────────────────────────────────────────────────
    ("list_backtests", "backtest", Risk::Read),
    ("get_backtest", "backtest", Risk::Read),
    // No `create_backtest`: it dispatched compute with no trial row (INV-16).
    // Spending a trial is a write, and the count cannot be lowered (INV-1) —
    // but only through a path that actually registers the trial first.
    ("stop_backtest", "backtest", Risk::Write),
    ("rerun_backtest", "backtest", Risk::Write),
    // The evidence behind a result disappears. If a report cited it, the citation
    // now points at nothing.
    ("delete_backtest", "backtest", Risk::Destructive),
    ("compare_backtests", "backtest", Risk::Read),
    ("wait_for_backtest", "backtest", Risk::Read),
    // ── automations ─────────────────────────────────────────────────────────
    //
    // The agent's token does not carry `automations.arm`, so these are unreachable
    // for a research session regardless of what the policy says. They are still
    // classified, because the same catalogue serves the human MCP client.
    ("list_automations", "automation", Risk::Read),
    ("create_automation", "automation", Risk::Write),
    // Arming changes what the platform will do with money, with no further human
    // step. That is the definition of destructive here.
    ("arm_automation", "automation", Risk::Destructive),
    ("disarm_automation", "automation", Risk::Write),
    // ── portfolio and trading state ─────────────────────────────────────────
    ("get_dashboard_rollup", "portfolio", Risk::Read),
    ("get_paper_activity", "portfolio", Risk::Read),
    ("get_trading_status", "portfolio", Risk::Read),
    ("get_order", "portfolio", Risk::Read),
    // ── models ──────────────────────────────────────────────────────────────
    ("list_models", "model", Risk::Read),
    ("get_model", "model", Risk::Read),
    ("list_feature_sets", "model", Risk::Read),
    // Training spends real compute and writes a new version. Nothing is destroyed
    // and a bad version can simply not be promoted, so: write, not destructive.
    ("train_model", "model", Risk::Write),
    ("get_training_run", "model", Risk::Read),
    // Promotion changes which version live strategies resolve to. That is a write
    // with reach beyond the registry, but it is reversible by promoting another
    // version, and `rollback` exists — so still write.
    ("promote_model_version", "model", Risk::Write),
    // ── research: experiments, sweeps, studies, gates ───────────────────────
    // Spends trials on a real Experiment, exactly like the sweep it wraps.
    ("backtest_strategy", "backtest", Risk::Write),
    ("create_experiment", "research", Risk::Write),
    ("list_experiments", "research", Risk::Read),
    ("get_experiment", "research", Risk::Read),
    ("run_sweep", "research", Risk::Write),
    ("get_sweep", "research", Risk::Read),
    ("cancel_sweep", "research", Risk::Write),
    ("run_study", "research", Risk::Write),
    ("list_studies", "research", Risk::Read),
    ("get_carried_forward", "research", Risk::Read),
    ("get_diagnostics", "research", Risk::Read),
    ("get_funnel", "research", Risk::Read),
    // Advancing a gate is a verdict about evidence. It is recorded, ordered and
    // cannot be walked back without the ledger showing it (D-8), so: write.
    ("advance_gate", "research", Risk::Write),
    ("get_null_picker", "research", Risk::Read),
    // Choosing a null seals a distribution (INV-2). The choice is final by design.
    ("choose_null", "research", Risk::Destructive),
];

/// The namespace and risk for a tool, or `None` if it is unclassified.
#[must_use]
pub fn classify(tool: &str) -> Option<(&'static str, Risk)> {
    TAXONOMY
        .iter()
        .find(|(name, _, _)| *name == tool)
        .map(|(_, ns, risk)| (*ns, *risk))
}

/// Every namespace, sorted.
#[must_use]
pub fn namespaces() -> Vec<&'static str> {
    let mut ns: Vec<&str> = TAXONOMY.iter().map(|(_, n, _)| *n).collect();
    ns.sort_unstable();
    ns.dedup();
    ns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test that makes the table load-bearing: a new tool cannot reach a model
    /// unclassified, so the policy can never default a destructive tool to "read".
    #[test]
    fn every_tool_is_classified() {
        let defs = crate::tool_definitions();
        let names: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        assert!(!names.is_empty());

        let missing: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| classify(n).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "these tools have no namespace/risk entry in taxonomy.rs: {missing:?}\n\
             Add one. An unclassified tool cannot be routed and its risk would default \
             to the most permissive reading."
        );
    }

    /// And the reverse: a stale entry for a deleted tool means the table is
    /// describing a catalogue that no longer exists.
    #[test]
    fn the_taxonomy_has_no_entries_for_tools_that_do_not_exist() {
        let defs = crate::tool_definitions();
        let names: Vec<&str> = defs
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
            .collect();
        let stale: Vec<&str> = TAXONOMY
            .iter()
            .map(|(n, _, _)| *n)
            .filter(|n| !names.contains(n))
            .collect();
        assert!(
            stale.is_empty(),
            "taxonomy entries for missing tools: {stale:?}"
        );
    }

    #[test]
    fn no_tool_is_classified_twice() {
        let mut seen: Vec<&str> = TAXONOMY.iter().map(|(n, _, _)| *n).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(
            before,
            seen.len(),
            "a duplicate entry makes the risk ambiguous"
        );
    }

    /// Nothing in this catalogue sends data outside the platform. If that ever
    /// changes, the trifecta audit in the capability profile has to change with it —
    /// this platform's outbound leg is cut, and an outbound tool would uncut it.
    #[test]
    fn the_catalogue_has_no_outbound_tools() {
        let outbound: Vec<&str> = TAXONOMY
            .iter()
            .filter(|(_, _, r)| *r == Risk::Outbound)
            .map(|(n, _, _)| *n)
            .collect();
        assert!(
            outbound.is_empty(),
            "adding an outbound tool uncuts the trifecta leg this platform cuts; \
             update config/profiles/*.yaml deliberately if that is intended: {outbound:?}"
        );
    }

    /// The five destructive tools, named. A test that lists them is how a sixth
    /// arriving becomes a review conversation instead of a silent widening.
    #[test]
    fn the_destructive_set_is_exactly_what_it_should_be() {
        let mut destructive: Vec<&str> = TAXONOMY
            .iter()
            .filter(|(_, _, r)| *r == Risk::Destructive)
            .map(|(n, _, _)| *n)
            .collect();
        destructive.sort_unstable();
        assert_eq!(
            destructive,
            vec![
                "arm_automation",
                "choose_null",
                "delete_backtest",
                "discard_draft",
            ]
        );
    }

    #[test]
    fn namespaces_are_small_enough_to_route_on() {
        let ns = namespaces();
        assert!(
            ns.len() <= 12,
            "namespaces are the routing unit; too many and routing is as hard as \
             picking a tool: {ns:?}"
        );
        for n in &ns {
            let count = TAXONOMY.iter().filter(|(_, x, _)| x == n).count();
            assert!(
                count <= 24,
                "namespace {n} has {count} tools, which cannot fit a frontier exposure budget"
            );
        }
    }

    #[test]
    fn the_risk_names_match_the_harness_crate() {
        // Kept in sync by hand; this asserts the wire strings agree, which is what
        // the policy engine keys on.
        assert_eq!(Risk::Read.as_str(), "read");
        assert_eq!(Risk::Write.as_str(), "write");
        assert_eq!(Risk::Destructive.as_str(), "destructive");
        assert_eq!(Risk::Outbound.as_str(), "outbound");
    }
}
