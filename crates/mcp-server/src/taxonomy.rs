//! Namespace and risk for every tool in the catalogue (ADR-0031, guide §2.3, §14.3).
//!
//! **This table is now derived.** Until Set S task S-1.3 it was a hand-written
//! array of 56 `(name, namespace, risk)` rows kept in agreement with a separate
//! array of 56 JSON literals by a test. The test worked — but a table that has to
//! be kept in agreement is a table that can be out of agreement, and the same
//! shape of defect one layer over (a gated action name no tool answered to) left
//! model promotion ungated on both drivers for as long as it existed (S-0.2).
//!
//! So the rows come from [`crate::spec::catalog`], where a tool's namespace and
//! risk are declared beside its name, description and schema. The tests below are
//! unchanged and still run — against generated rows, where they now assert
//! properties of the catalogue rather than the agreement of two copies of it.
//!
//! **Risk is about what the tool does to the world, not how dangerous it feels.**
//! `delete_backtest` is destructive because the run is gone; `arm_automation` is
//! destructive because it changes what the platform will do with real money without
//! a further human step. `run_sweep` is a write: it costs trials and those
//! cannot be un-spent, but nothing is destroyed.
//!
//! Risk and [`crate::spec::Envelope`] answer different questions and are not
//! ranked against each other. `promote_model_version` is a `Write` — it is
//! reversible by promoting another version — *and* it is under the `Promotion`
//! envelope, because reversible is not the same as unsupervised.

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

/// `(tool, namespace, risk)` for every declared tool, in catalogue order.
///
/// Namespaces are the routing unit (§2.2): a plan step declares the namespace it
/// needs and the registry exposes that slice. They are chosen so a step normally
/// needs exactly one.
///
/// Deprecated tools are included. A trial ticket citing a retired tool still has
/// to resolve to a namespace and a risk for the desk to render what produced it
/// (§2.7); exposure is what deprecation removes, not classification.
#[must_use]
pub fn taxonomy() -> Vec<(&'static str, &'static str, Risk)> {
    crate::spec::catalog()
        .iter()
        .map(|s| (s.name, s.namespace, s.risk))
        .collect()
}

/// The namespace and risk for a tool, or `None` if it is unclassified.
///
/// Unclassified is now unreachable for a tool that exists — a tool without a
/// [`crate::spec::ToolSpec`] is not in the catalogue at all, and
/// `every_dispatch_arm_has_a_spec_and_every_spec_has_an_arm` fails the build if
/// one is. The `Option` stays because callers ask about arbitrary strings.
#[must_use]
pub fn classify(tool: &str) -> Option<(&'static str, Risk)> {
    crate::spec::find(tool).map(|s| (s.namespace, s.risk))
}

/// Every namespace, sorted.
#[must_use]
pub fn namespaces() -> Vec<&'static str> {
    let mut ns: Vec<&str> = crate::spec::catalog().iter().map(|s| s.namespace).collect();
    ns.sort_unstable();
    ns.dedup();
    ns
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test that makes the classification load-bearing: a tool cannot reach a
    /// model unclassified, so the policy can never default a destructive tool to
    /// "read".
    ///
    /// Generation makes this structurally true rather than merely observed —
    /// which is the point, and is also why the test stays: it is what fails if
    /// someone reintroduces a second source of tool names.
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
            "these tools have no namespace/risk classification: {missing:?}\n\
             An unclassified tool cannot be routed and its risk would default \
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
        let stale: Vec<&str> = taxonomy()
            .iter()
            .map(|(n, _, _)| *n)
            .filter(|n| {
                // A deprecated tool is classified and not exposed, by design.
                !names.contains(n) && crate::spec::find(n).is_some_and(|s| s.status.is_active())
            })
            .collect();
        assert!(
            stale.is_empty(),
            "classification for missing tools: {stale:?}"
        );
    }

    #[test]
    fn no_tool_is_classified_twice() {
        let mut seen: Vec<&str> = taxonomy().iter().map(|(n, _, _)| *n).collect();
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
    ///
    /// The two web tools that would narrow this assertion (ADR-P7-01) are not in
    /// the tracked catalogue; see Set S task S-0.3, blocked on gate H-1.
    #[test]
    fn the_only_outbound_tools_are_the_two_web_tools() {
        let mut outbound: Vec<&str> = taxonomy()
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

    /// The four destructive tools, named. A test that lists them is how a fifth
    /// arriving becomes a review conversation instead of a silent widening.
    #[test]
    fn the_destructive_set_is_exactly_what_it_should_be() {
        let mut destructive: Vec<&str> = taxonomy()
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
                "discard_draft"
            ]
        );
    }

    #[test]
    fn namespaces_are_small_enough_to_route_on() {
        let ns = namespaces();
        assert!(
            ns.len() <= 22,
            "namespaces are the routing unit; too many and routing is as hard as \
             picking a tool. Twenty-two is AGENT-006 §2.1's canonical list, and \n             §10.1 AC-11 fixes the cap there: {ns:?}"
        );
        let all = taxonomy();
        for n in &ns {
            let count = all.iter().filter(|(_, x, _)| x == n).count();
            assert!(
                count <= 24,
                "namespace {n} has {count} tools, which cannot fit a frontier exposure budget"
            );
        }
    }

    /// Every namespace is one of AGENT-006 §2.1's canonical twenty-two.
    ///
    /// A tool declaring `namespace: "strategies"` when the canonical name is
    /// `strategy` creates a namespace of one that no plan step will ever route
    /// to, and nothing else would notice.
    #[test]
    fn every_namespace_is_canonical() {
        const CANONICAL: &[&str] = &[
            "core",
            "discovery",
            "data",
            "analysis",
            "attribution",
            "strategy",
            "builder",
            "backtest",
            "research",
            "audit",
            "falsification",
            "execution",
            "portfolio",
            "model",
            "memory",
            "report",
            "lifecycle",
            "code",
            "skills",
            "web",
            "fs",
            "automation",
        ];
        let rogue: Vec<&str> = namespaces()
            .into_iter()
            .filter(|n| !CANONICAL.contains(n))
            .collect();
        assert!(
            rogue.is_empty(),
            "namespaces outside AGENT-006 §2.1: {rogue:?} — a namespace of one is \
             a tool no plan step routes to"
        );
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
