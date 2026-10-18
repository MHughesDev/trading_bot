//! `list_features` and the engine accept the same names (S-5.2; AGENT-011 §2,
//! P2; AC-1).
//!
//! The acceptance: *"every feature the backtest accepts is served by
//! `list_features`, and every feature listed is accepted by the backtest, the
//! live runtime and the Phase 3 analysis templates. One engine, proven by test."*
//!
//! # What each clause means here
//!
//! - **"the backtest accepts"** is `features::runtime::feature`: the simulator
//!   resolves every strategy feature through it (`sim.rs`). This file asserts the
//!   registry and the engine agree on which names resolve, over a grid of
//!   parameters and across the declared range boundary, so a family cannot be
//!   listed and refused or accepted and unlisted.
//! - **"served by `list_features`"** is the tool's answer for the family: it
//!   returns the family, with an example the engine accepts.
//! - **"the live runtime"** reads features through the same
//!   `features::runtime`, so it accepts exactly what the engine does.
//! - **"the analysis templates"** do not read the registry — they are numpy, and
//!   compute their own estimators. What can be proved is that each estimator
//!   they share with the engine gives the same number over the same bars, which
//!   is `volatility_reconciliation.rs` and `features_f2_reconciliation.rs`; the
//!   families those cover are listed in
//!   [`the_families_the_templates_compute_exist_in_the_engine`] so a rename on
//!   either side fails here.
//!
//! # A finding this file does not pin, because it is a defect
//!
//! `backtest::requirements::parse_feature` — the gate `derive_requirements`
//! applies before the simulator or the preview runs — accepts a handful of
//! families (`ema`, `rsi`, and in the working tree `sma`, `atr`), not the
//! registry. So the *engine* accepts every listed family and the *backtest's
//! requirements gate* does not. That is outside this crate and is reported with
//! the exact change in the phase file (S-5.2); the clause "every feature listed
//! is accepted by the backtest" stays unmet until it lands.

use std::collections::BTreeSet;

use serde_json::json;

use crate::tools::strategy::list_features;

/// The parameter vectors worth trying for a family: each parameter at its
/// minimum, at its example, and at twice the example (capped, so a 100,000-bar
/// window is never built), in every combination.
fn grid(f: &features::registry::Family) -> Vec<Vec<usize>> {
    let ex = f.example_params();
    let mut out: Vec<Vec<usize>> = vec![vec![]];
    for (p, e) in f.params.iter().zip(&ex) {
        let mut vals: BTreeSet<usize> = BTreeSet::new();
        vals.insert(p.min);
        vals.insert(*e);
        vals.insert((e * 2).clamp(p.min, p.max).min(500));
        out = out
            .into_iter()
            .flat_map(|prefix| {
                vals.iter().map(move |v| {
                    let mut next = prefix.clone();
                    next.push(*v);
                    next
                })
            })
            .collect();
    }
    out
}

/// The registry and the engine accept the same names, in range and out of it.
#[test]
fn the_engine_and_the_registry_accept_the_same_names() {
    let mut checked = 0;
    for f in features::registry::families() {
        for params in grid(f) {
            let name = f.name_for(&params);
            let by_registry = features::registry::resolve(&name).is_some();
            let by_engine = features::runtime::lookback_bars(&name).is_some();
            assert_eq!(by_registry, by_engine, "`{name}`");
            assert!(by_registry, "`{name}` is inside its declared range");
            checked += 1;
        }
        // One step outside each parameter's range, on both sides: neither
        // resolves. A range is part of a name's meaning.
        for (i, p) in f.params.iter().enumerate() {
            let mut base = f.example_params();
            for bad in [p.min.checked_sub(1), p.max.checked_add(1)]
                .into_iter()
                .flatten()
            {
                base[i] = bad;
                let name = f.name_for(&base);
                assert!(
                    features::registry::resolve(&name).is_none(),
                    "`{name}` is outside `{}`'s range and resolved",
                    p.name
                );
                assert!(
                    features::runtime::lookback_bars(&name).is_none(),
                    "the engine accepted `{name}`, which the registry refuses"
                );
            }
        }
    }
    assert!(checked > 100, "only {checked} names were tried");
}

/// Every family is served, and what is served is accepted.
#[test]
fn every_family_is_served_by_list_features_and_its_example_is_accepted() {
    let total = features::registry::families().len();
    let all = list_features(&json!({}));
    assert_eq!(all["detail"]["n_total"], total, "{all}");

    for f in features::registry::families() {
        let one = list_features(&json!({ "family": f.name }));
        let fams = one["detail"]["families"].as_array().expect("families");
        assert_eq!(fams.len(), 1, "`{}` is not served by name", f.name);
        let example = fams[0]["example"].as_str().expect("an example");
        // Listed implies accepted: the example resolves in the registry and the
        // engine builds it, through the declared-lookback registration check.
        assert!(features::registry::resolve(example).is_some(), "{example}");
        features::runtime::feature(example)
            .unwrap_or_else(|e| panic!("`{example}` is listed and the engine refuses it: {e}"));
        // And its stated warm-up is the engine's.
        assert_eq!(
            fams[0]["warmup_bars"].as_u64().map(|w| w as u32),
            features::runtime::lookback_bars(example),
            "`{example}`"
        );
    }
}

/// An unfiltered listing is capped, and the answer says by how much. This pins
/// the arithmetic of the cap, not that a cap is right: see the phase file for
/// why a caller cannot page past it today.
#[test]
fn an_unfiltered_listing_reports_what_it_left_out() {
    let all = list_features(&json!({}));
    let d = &all["detail"];
    let shown = d["families"].as_array().expect("families").len();
    let matched = d["n_matched"].as_u64().expect("n_matched") as usize;
    assert_eq!(d["n_shown"].as_u64().unwrap() as usize, shown);
    assert_eq!(
        d["n_truncated"].as_u64().unwrap() as usize,
        matched - shown,
        "the answer must account for every family it did not return"
    );
}

/// Each estimator the analysis templates share with the engine exists in the
/// registry under the name the reconciliation tests use. A rename on either side
/// fails here, in the same run, rather than in a reconciliation that then fails
/// for the wrong reason.
#[test]
fn the_families_the_templates_compute_exist_in_the_engine() {
    for family in [
        "parkinson_vol",
        "garman_klass_vol",
        "rogers_satchell_vol",
        "rv",
        "bv",
        "autocorr",
        "variance_ratio",
        "efficiency_ratio",
    ] {
        assert!(
            features::registry::family(family).is_some(),
            "`{family}` is reconciled against a template and is not in the registry"
        );
    }
}

/// Paging reaches every family: following `next_offset` visits each exactly once.
#[test]
fn paging_reaches_every_family_exactly_once() {
    let mut seen: Vec<String> = Vec::new();
    let mut offset = 0u64;
    loop {
        let out = list_features(&json!({ "offset": offset }));
        for f in out["detail"]["families"].as_array().expect("families") {
            seen.push(f["family"].as_str().expect("name").to_string());
        }
        match out["detail"]["next_offset"].as_u64() {
            Some(n) => offset = n,
            None => break,
        }
    }
    let want: Vec<String> = features::registry::families()
        .iter()
        .map(|f| f.name.to_string())
        .collect();
    assert_eq!(seen, want);
}
