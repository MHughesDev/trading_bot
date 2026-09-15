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
    // ── falsification (EXTERNAL-REVIEW-2026-09 §4) ──────────────────────────
    //
    // Filing a report is a `write`: it appends a row nothing can edit
    // afterwards. It is emphatically NOT destructive — an agent that hesitates
    // to report a defect because the tool is flagged dangerous is an agent that
    // exploits the defect instead, which is the failure the whole mechanism
    // exists to prevent.
    ("report_kill", "falsification", Risk::Write),
    ("report_evaluator_defect", "falsification", Risk::Write),
    // Both of these only measure; neither spends a trial. That is the point of
    // asking them *before* a search rather than after it disappoints.
    ("check_tradeable_frontier", "falsification", Risk::Read),
    ("run_oracle_test", "falsification", Risk::Read),
    // ── evidence (ADR-P7-05) ────────────────────────────────────────────────
    //
    // All reads, and all cheap. They exist because the falsification tools
    // shipped with a defect vocabulary and no way to gather evidence for it.
    ("get_trade_list", "falsification", Risk::Read),
    ("probe_data_quality", "falsification", Risk::Read),
    ("find_similar_trials", "falsification", Risk::Read),
    // ── web access (ADR-P7-01) ──────────────────────────────────────────────
    //
    // The first `Outbound` tools in this catalogue. They restore a leg of the
    // lethal trifecta this platform previously cut, and they are classified
    // `Outbound` rather than `Read` precisely so that fact is visible in the
    // policy layer and in `the_only_outbound_tools_are_the_two_web_tools`.
    // Reads to the platform are `Read`; a read of the *internet* is not the same
    // act and must not be filed as one.
    ("web_search", "web", Risk::Outbound),
    ("fetch_url", "web", Risk::Outbound),
    // ── the skill library (ADR-P7-03) ───────────────────────────────────────
    //
    // Emphatically NOT outbound: a skill has no network and no database. It is
    // a pure transformation over the payload it is handed, which is what makes
    // `run_skill` a `Write` (it appends a run record) rather than anything more.
    ("create_skill", "skills", Risk::Write),
    ("list_skills", "skills", Risk::Read),
    ("run_skill", "skills", Risk::Write),
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
    /// The catalogue's outbound surface, pinned by name.
    ///
    /// This replaced `the_catalogue_has_no_outbound_tools` on 2026-09-14, when
    /// web access was added deliberately. The assertion did not go away — it
    /// narrowed. Before, the platform cut the exfiltration leg of the lethal
    /// trifecta entirely; now it has exactly two outbound tools, both behind
    /// `webfetch::egress`, and a third appearing here is a decision somebody has
    /// to make on purpose rather than a diff nobody noticed.
    #[test]
    fn the_only_outbound_tools_are_the_two_web_tools() {
        let mut outbound: Vec<&str> = TAXONOMY
            .iter()
            .filter(|(_, _, r)| *r == Risk::Outbound)
            .map(|(n, _, _)| *n)
            .collect();
        outbound.sort_unstable();
        assert_eq!(
            outbound,
            vec!["fetch_url", "web_search"],
            "the outbound surface changed. Every tool that can reach the internet must go              through `webfetch::egress`, which is the one place destinations are decided;              adding another means updating config/profiles/*.yaml and this test together,              deliberately."
        );
    }

    /// A skill runs with no network. Classifying one of these `Outbound` would
    /// be describing the sandbox as something it is not.
    #[test]
    fn the_skill_tools_are_not_outbound() {
        for name in ["create_skill", "list_skills", "run_skill"] {
            let (_, _, risk) = TAXONOMY
                .iter()
                .find(|(n, _, _)| *n == name)
                .unwrap_or_else(|| panic!("{name} must be classified"));
            assert_ne!(*risk, Risk::Outbound, "{name} has no network");
        }
    }
}
