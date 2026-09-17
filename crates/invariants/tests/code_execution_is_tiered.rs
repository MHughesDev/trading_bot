//! Free-form code is offered only where the profile allows it (AGENT-006 §5.9,
//! AGENT-010 §10; Set S task S-8.8).
//!
//! Two crates hold one string. `harness::registry::CODE_NAMESPACE` is what the
//! exposure filter compares against; `mcp_server_lib::tools::code::NAMESPACE` is
//! what the tool is declared under. `harness` does not depend on `mcp-server` —
//! the dependency runs the other way — so the two are held equal here.
//!
//! The direction of a drift is what makes this worth a test: if the tool's
//! namespace were renamed and the filter's constant were not, the filter would
//! stop matching and `run_python` would be offered on **every** tier, including
//! the ones §11 says may not run free-form code at all. Nothing would fail, and
//! the tool would work.

use std::path::PathBuf;

use harness::profile::{CodeExecution, ProfileSet};

fn profiles() -> ProfileSet {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("config")
        .join("profiles");
    ProfileSet::load_dir(&dir).expect("the shipped profiles load")
}

#[test]
fn the_gated_namespace_is_the_one_the_tool_is_declared_in() {
    assert_eq!(
        harness::registry::CODE_NAMESPACE,
        mcp_server_lib::tools::code::NAMESPACE,
        "the exposure filter gates one namespace and `run_python` is declared in another, so \
         free-form code is offered on every tier"
    );
    for spec in mcp_server_lib::tools::code::specs() {
        assert_eq!(spec.namespace, harness::registry::CODE_NAMESPACE);
    }
}

/// The three shipped profiles are one of each tier, which is what makes this a
/// test of the platform rather than of a fixture.
#[test]
fn the_shipped_profiles_cover_all_three_tiers() {
    let set = profiles();
    let mut tiers: Vec<CodeExecution> = set
        .ids()
        .into_iter()
        .filter_map(|id| set.get(id).map(|p| p.orchestration.code_execution))
        .collect();
    tiers.sort_by_key(|t| format!("{t:?}"));
    tiers.dedup();
    assert_eq!(
        tiers.len(),
        3,
        "the shipped profiles no longer cover every code-execution tier, so this file stops \
         testing the ones they dropped"
    );
}

/// S-8.8's acceptance, both halves: on a `denied` tier the tool is not exposed,
/// and the templated tools still are.
#[test]
fn only_a_full_tier_is_offered_the_code_tool() {
    let registry = api::agent::bridge::registry().expect("the catalogue classifies");
    let namespaces: Vec<String> = mcp_server_lib::spec::catalog()
        .iter()
        .map(|s| s.namespace.to_string())
        .collect();
    // Asked for by name, so an absence is the filter and not the router.
    let pinned = vec!["run_python".to_string()];
    let templated: Vec<&str> = mcp_server_lib::spec::catalog()
        .iter()
        .filter(|s| matches!(s.tier, mcp_server_lib::spec::Tier::Templated { .. }))
        .map(|s| s.name)
        .collect();

    let set = profiles();
    for id in set.ids() {
        let mut profile = set.get(id).expect("listed").clone();
        let tier = profile.orchestration.code_execution;
        // Wide enough that an absence is the filter and not the cap.
        profile.tools.max_exposed_per_step = mcp_server_lib::spec::catalog().len() + 16;

        let exposure = registry
            .expose_pinned(&profile, &namespaces, &pinned)
            .expect("the catalogue fits this budget");

        let offered = exposure.names.iter().any(|n| n == "run_python");
        assert_eq!(
            offered,
            tier == CodeExecution::Full,
            "`{id}` has code_execution={tier:?} and is offered run_python={offered}"
        );

        // The other half of the acceptance. Easy to break by gating on something
        // broader than the namespace — a `Risk::Write` filter, say — and the
        // breakage would look like a tier that had simply lost its tools.
        for name in &templated {
            assert!(
                exposure.names.iter().any(|n| n == name),
                "`{name}` is a templated tool and `{id}` (code_execution={tier:?}) is not \
                 offered it"
            );
        }
    }
}
