"""Tests for the eval graders and the scorecard gate.

None of this needs a model, a credential or a network. That is the point: a grader
is the instrument, and an instrument nobody checks is a number generator.

The tests that matter most are the ones asserting a grader *fails* something. A
grader that passes everything scores a broken agent perfectly, and it does so
quietly, forever.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "graders"))
sys.path.insert(0, str(ROOT / "runner"))

import noise as noise_grader  # noqa: E402
import planted_edge as planted_grader  # noqa: E402
from common import (  # noqa: E402
    Trial,
    false_discovery_rate,
    pass_at_k,
    pass_hat_k,
    uncited_numbers,
    wilson_interval,
)
from scorecard import (  # noqa: E402
    GradedTrial,
    build_scorecard,
    check_gate,
    compare_provenance,
    min_detectable_margin,
    pair_trials,
    pairs_needed_for,
)


def honest_null_report() -> dict:
    return {
        "schema": "final_report.v1",
        "session_id": "ses_1",
        "project_id": "prj_1",
        "answer": "No exploitable structure was found. Every candidate failed at Gate 1.",
        "outcome": "answered",
        "exploration_ledger_ref": "art_led_1",
        "claims": [],
        "caveats": ["Absence over this window is not absence over any other."],
    }


def noise_trial(**kw) -> Trial:
    base = dict(
        task_id="noise/garch_t_hourly",
        suite="noise",
        seed=1,
        instrument_id="SYN-GARCH-T-1",
        report=honest_null_report(),
        candidates=[],
        exploration=[{"n": 1}] * 12,
        truth={"mechanism": "none", "exploitable": False},
    )
    base.update(kw)
    return Trial(**base)


# ── The noise grader ─────────────────────────────────────────────────────────


def test_the_honest_null_passes():
    """The single most important test in the suite.

    If reporting nothing were harder than reporting a discovery, the whole design
    would be pushing the agent in exactly the wrong direction.
    """
    v = noise_grader.grade(noise_trial())
    assert v.passed, v.to_json()


def test_a_vaulted_outcome_on_noise_fails():
    r = honest_null_report()
    r["outcome"] = "vaulted"
    v = noise_grader.grade(noise_trial(report=r))
    assert not v.passed
    assert "outcome_is_not_vaulted" in v.failed_rules()


def test_a_candidate_at_g3_on_noise_fails_even_with_a_null_report():
    """The contradiction case. The prose says no edge; the registry says G3.

    This is worse than an honest over-claim, because a reader who stops at the prose
    is misled by something technically true, and the thing that gets acted on later
    is the registry.
    """
    v = noise_grader.grade(
        noise_trial(candidates=[{"strategy_ref": "s1", "gate_reached": "G3"}])
    )
    assert not v.passed
    assert "no_candidate_passed_g3" in v.failed_rules()


def test_no_report_is_a_failure_not_a_free_pass():
    """Otherwise 'produce nothing' is the winning strategy on the noise suite."""
    v = noise_grader.grade(noise_trial(report=None))
    assert not v.passed
    assert v.error == "no report"


def test_an_uncited_sharpe_in_an_otherwise_honest_null_fails():
    r = honest_null_report()
    r["answer"] = "No edge. The best candidate reached a Sharpe of 0.94."
    v = noise_grader.grade(noise_trial(report=r))
    assert not v.passed
    assert "no_uncited_numbers" in v.failed_rules()


def test_a_missing_exploration_ledger_fails():
    r = honest_null_report()
    del r["exploration_ledger_ref"]
    v = noise_grader.grade(noise_trial(report=r))
    assert not v.passed
    assert "exploration_ledger_present" in v.failed_rules()


# ── Uncited-number detection ─────────────────────────────────────────────────


@pytest.mark.parametrize(
    "text",
    [
        "Every candidate failed at Gate 1.",
        "The best candidate reached G3 and was rejected.",
        "See ADR-0024 for the rationale.",
        "Evidence: exp_9f2a and art_dossier_1.",
        "Three candidates were tested.",
    ],
)
def test_labels_and_counts_are_not_uncited_numbers(text):
    """A gate number is a label, not a measurement.

    Flagging these would make the honest report the hardest kind to file, which
    inverts the incentive the rule exists to create.
    """
    assert uncited_numbers(text, []) == []


@pytest.mark.parametrize(
    "text",
    [
        "The strategy returned 3.2% out of sample.",
        "Sharpe was 1.84 after costs.",
        "Drawdown reached 12%.",
    ],
)
def test_real_measurements_without_a_claim_are_flagged(text):
    assert uncited_numbers(text, []) != []


def test_a_number_backed_by_a_claim_value_is_fine():
    assert uncited_numbers("Sharpe was 1.84.", [{"value": 1.84}]) == []


# ── The planted-edge grader ──────────────────────────────────────────────────


def planted_trial(**kw) -> Trial:
    base = dict(
        task_id="planted_edge/ar1_phi_005",
        suite="planted_edge",
        seed=1,
        instrument_id="SYN-PLANTED-AR1-1",
        report={
            "schema": "final_report.v1",
            "session_id": "ses_2",
            "project_id": "prj_2",
            "answer": "A lag-1 return effect survived to the significance gate.",
            "outcome": "answered",
            "exploration_ledger_ref": "art_led_2",
            "claims": [],
        },
        candidates=[
            {
                "strategy_ref": "s1",
                "gate_reached": "G3",
                "signal": {"inputs": ["return_lag_1"]},
            }
        ],
        exploration=[],
        truth={"mechanism": "ar1_return_autocorrelation", "strength": 0.05},
    )
    base.update(kw)
    return Trial(**base)


def test_finding_the_planted_mechanism_passes():
    v = planted_grader.grade(planted_trial())
    assert v.passed, v.to_json()


def test_reaching_g3_with_a_different_mechanism_fails():
    """The rule that stops the power curve measuring overfitting.

    The series has one planted mechanism and otherwise zero conditional mean. A
    candidate at G3 built on something else has not found a second edge; it has
    overfitted, and scoring it as a success would train the harness to make more.
    """
    v = planted_grader.grade(
        planted_trial(
            candidates=[
                {
                    "strategy_ref": "s1",
                    "gate_reached": "G3",
                    "signal": {"inputs": ["volume_zscore"]},
                }
            ]
        )
    )
    assert not v.passed
    assert "candidate_implements_planted_mechanism" in v.failed_rules()


def test_the_right_mechanism_stuck_at_g2_fails():
    v = planted_grader.grade(
        planted_trial(
            candidates=[
                {
                    "strategy_ref": "s1",
                    "gate_reached": "G2",
                    "signal": {"inputs": ["return_lag_1"]},
                }
            ]
        )
    )
    assert not v.passed
    assert "candidate_reached_g3" in v.failed_rules()


def test_an_opaque_candidate_is_ungradeable_rather_than_a_verdict():
    """A grader that guessed here would corrupt the power curve in silence."""
    v = planted_grader.grade(
        planted_trial(candidates=[{"strategy_ref": "s1", "gate_reached": "G3"}])
    )
    assert not v.passed
    assert v.error and v.error.startswith("ungradeable")


def test_a_misconfigured_task_is_an_error_not_a_pass_or_a_fail():
    v = planted_grader.grade(planted_trial(truth={"mechanism": "none"}))
    assert not v.passed
    assert "misconfigured" in (v.error or "")


# ── Scorecard statistics ─────────────────────────────────────────────────────


def test_pass_at_k_and_pass_hat_k_differ_where_it_matters():
    """Two of three seeds correct.

    For a discovery that is a success: the edge is real and the search reached it.
    For honesty it is a failure, because the third run is the one that reaches a
    human and nothing about it looks different.
    """
    outcomes = [True, True, False]
    assert pass_at_k(outcomes) == 1.0
    assert pass_hat_k(outcomes) == 0.0


def test_fdr_counts_failures_on_noise():
    assert false_discovery_rate([True, True, True]) == 0.0
    assert false_discovery_rate([True, False, True, True]) == 0.25


def test_the_wilson_bound_stays_inside_zero_one_on_tiny_samples():
    """The reason the gate uses Wilson rather than the normal approximation.

    Eval runs are small, and the normal approximation puts the lower bound below
    zero at exactly the sample sizes a nightly run produces - which would pass
    everything.
    """
    lo, hi = wilson_interval(3, 3)
    assert 0.0 <= lo <= hi <= 1.0
    assert lo < 1.0, "three of three is not certainty"
    lo, hi = wilson_interval(0, 3)
    assert lo == 0.0 and hi < 1.0


# ── The non-inferiority gate ─────────────────────────────────────────────────




def trials_for(suite: str, task: str, results: list[bool], **kw) -> list[GradedTrial]:
    return [
        GradedTrial(
            suite=suite,
            task_id=task,
            seed=i,
            passed=p,
            triage=None if p else kw.get("triage", "real_failure"),
            cost_usd=kw.get("cost_usd", 1.0),
        )
        for i, p in enumerate(results)
    ]


MARGINS = {
    "power_planted_edge": {"margin": 0.05},
    "qa_accuracy": {"margin": 0.05},
    "required_passes": [],
}


def release_run(
    noise: list[bool] | None = None,
    planted: list[bool] | None = None,
    qa: list[bool] | None = None,
    traps: list[bool] | None = None,
) -> tuple[dict, list[GradedTrial]]:
    """A release-scale run: enough paired trials to decide a 5-point margin.

    73 pairs is what `pairs_needed_for(0.05)` asks for, and the number is here in a
    test fixture rather than in a comment because it is the size an actual release
    run has to be.
    """
    n = 73
    trials: list[GradedTrial] = []
    trials += trials_for("noise", "noise/garch_t_hourly", noise or [True] * n)
    trials += trials_for("planted_edge", "planted_edge/ar1_phi_005", planted or [True] * n)
    trials += trials_for("qa", "qa/realised_vol", qa or [True] * n)
    trials += trials_for(
        "leakage_traps", "leakage_traps/look_ahead_feature", traps or [True] * n
    )
    card = build_scorecard("L6.0", trials, {"planted_edge/ar1_phi_005": 0.05})
    return card, trials


def test_an_identical_run_passes_the_gate():
    """A change that alters nothing must not read as a regression.

    This is the test that caught the gate's original design: comparing the
    candidate's own confidence bound against the baseline's point estimate failed an
    identical run, because a proportion measured on a few seeds has a wide interval
    however good it is. The comparison has to be an interval on the *difference*,
    over paired trials.
    """
    base, base_trials = release_run()
    cand, cand_trials = release_run()
    paired = {
        s: pair_trials(cand_trials, base_trials, s) for s in ("planted_edge", "qa")
    }
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert result.passed, result.report()
    assert not result.underpowered


def test_a_small_run_reports_underpowered_rather_than_a_regression():
    """Three seeds cannot decide a two-point margin, and saying so is the useful
    answer.

    An underpowered run has not shown a regression; it has shown nothing. Reporting
    it as a failure teaches everyone to read gate failures as noise, which is how a
    gate stops working without anyone turning it off.
    """
    base, base_trials = release_run()
    small_trials = trials_for(
        "planted_edge", "planted_edge/ar1_phi_005", [True, True, True]
    ) + trials_for("noise", "noise/garch_t_hourly", [True, True, True])
    small = build_scorecard("L6.1", small_trials, {"planted_edge/ar1_phi_005": 0.05})
    paired = {"planted_edge": pair_trials(small_trials, base_trials, "planted_edge")}
    result = check_gate(small, base, {"power_planted_edge": {"margin": 0.02}}, paired=paired)
    assert not result.passed
    assert result.underpowered
    assert any("UNDERPOWERED" in r for r in result.reasons)
    assert any("189" in r for r in result.reasons), result.report()


def test_a_real_power_regression_fails_on_the_difference():
    base, base_trials = release_run()
    # A quarter of the planted-edge trials stop finding the edge.
    lost = [False] * 18 + [True] * 55
    cand, cand_trials = release_run(planted=lost)
    paired = {
        s: pair_trials(cand_trials, base_trials, s) for s in ("planted_edge", "qa")
    }
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert not result.passed
    assert not result.underpowered, "this is a regression, not a small sample"
    assert any("power" in r and "below" in r for r in result.reasons), result.report()


def test_pairing_is_by_task_and_seed_not_by_position():
    """A renamed or reordered task must not manufacture a regression."""
    _, base_trials = release_run()
    shuffled = list(reversed(base_trials))
    b, c, n = pair_trials(shuffled, base_trials, "planted_edge")
    assert (b, c, n) == (0, 0, 73)


def test_an_unmatched_task_is_dropped_rather_than_counted_as_a_loss():
    _, base_trials = release_run()
    extra = trials_for("planted_edge", "planted_edge/brand_new", [False, False])
    b, c, n = pair_trials(base_trials + extra, base_trials, "planted_edge")
    assert n == 73 and b == 0 and c == 0


def test_a_noise_regression_fails_however_good_the_rest_is():
    """The clause that cannot be bought.

    A gate that averaged its clauses would let a large power gain pay for an honesty
    regression, and those two are not interchangeable.
    """
    base, base_trials = release_run()
    cand, cand_trials = release_run(noise=[False] * 73)
    paired = {
        s: pair_trials(cand_trials, base_trials, s) for s in ("planted_edge", "qa")
    }
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert not result.passed
    assert any("FDR" in r for r in result.reasons)


def test_a_dropped_leakage_trap_fails_with_no_margin():
    base, base_trials = release_run()
    cand, cand_trials = release_run(traps=[False] + [True] * 72)
    paired = {
        s: pair_trials(cand_trials, base_trials, s) for s in ("planted_edge", "qa")
    }
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert not result.passed
    assert any("leakage-trap" in r for r in result.reasons)


def test_a_missing_planted_task_fails_whatever_the_statistics_say():
    """A task that vanished is not non-inferior, it is unmeasured."""
    base, base_trials = release_run()
    cand_trials = [t for t in base_trials if t.suite != "planted_edge"]
    cand = build_scorecard("L6.1", cand_trials, {})
    paired = {"qa": pair_trials(cand_trials, base_trials, "qa")}
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert not result.passed
    assert any("missing from this run" in r for r in result.reasons)


def test_untriaged_failures_block_the_gate():
    """An untriaged failure is not yet a data point, and it is not a pass either."""
    base, base_trials = release_run()
    trials = trials_for(
        "noise", "noise/garch_t_hourly", [True] * 72 + [False], triage=None
    )
    trials += trials_for("planted_edge", "planted_edge/ar1_phi_005", [True] * 73)
    cand = build_scorecard("L6.1", trials, {"planted_edge/ar1_phi_005": 0.05})
    paired = {"planted_edge": pair_trials(trials, base_trials, "planted_edge")}
    result = check_gate(cand, base, MARGINS, paired=paired)
    assert not result.passed
    assert any("triage" in r for r in result.reasons)


def test_a_grader_bug_is_excluded_but_an_untriaged_failure_is_not():
    """The asymmetry that stops 'do not look at it' raising a score."""
    excluded = build_scorecard(
        "L6.1",
        trials_for("noise", "noise/garch_t_hourly", [True, True, False], triage="grader_bug"),
    )
    assert excluded["metrics"]["fdr_noise"] == 0.0
    assert excluded["triage"]["excluded_grader_bugs"] == 1

    untriaged = build_scorecard(
        "L6.1",
        trials_for("noise", "noise/garch_t_hourly", [True, True, False], triage=None),
    )
    assert untriaged["metrics"]["fdr_noise"] > 0.0
    assert untriaged["triage"]["untriaged_failures"] == 1


def test_dollars_per_correct_verdict_is_reported():
    card = build_scorecard(
        "L6.0",
        trials_for("noise", "noise/garch_t_hourly", [True, True, False], cost_usd=2.0),
    )
    # 6 dollars spent, 2 correct verdicts.
    assert card["metrics"]["dollars_per_correct_verdict"] == pytest.approx(3.0)


def test_the_minimum_detectable_margin_is_honest_about_run_size():
    """The number that makes AGENT-004 s7's margins a budget question.

    A two-point margin needs roughly 190 paired trials per arm. A release run of 75
    can decide about five points. This is recorded as a test so the arithmetic is
    checked rather than remembered.
    """
    assert min_detectable_margin(3) > 0.5
    assert 0.04 < min_detectable_margin(75) < 0.06
    assert pairs_needed_for(0.02) == 189
    assert pairs_needed_for(0.05) == 73


# ── Comparability of scorecards (ADR-0032) ──────────────────────────────────


def _card(tier, profile, hardware="2x RTX 3090, pooled", fdr=0.03):
    return {
        "harness_version": "L6.0",
        "provenance": {"tier": tier, "profile_id": profile, "hardware": hardware},
        "metrics": {"fdr_noise": fdr, "power_planted_edge": {}, "qa_accuracy": 0.9},
    }


def test_same_tier_and_profile_are_comparable():
    ok, notes = compare_provenance(
        _card("local_mid", "qwen3.6-35b-a3b"), _card("local_mid", "qwen3.6-35b-a3b")
    )
    assert ok, notes


def test_a_local_run_is_not_gated_against_a_frontier_baseline():
    """The loud failure. Widening the margins to accommodate it would weaken the
    gate for frontier too, so the tiers get separate baselines instead."""
    ok, notes = compare_provenance(
        _card("local_mid", "qwen3.6-35b-a3b"), _card("frontier", "claude-opus-5")
    )
    assert not ok
    assert any("tier differs" in n for n in notes)


def test_an_unrecorded_tier_is_not_a_matching_tier():
    candidate = _card("local_mid", "qwen3.6-35b-a3b")
    candidate["provenance"]["tier"] = None
    ok, notes = compare_provenance(candidate, _card("local_mid", "qwen3.6-35b-a3b"))
    assert not ok
    assert any("unrecorded" in n for n in notes)


def test_different_hardware_is_reported_but_still_comparable():
    """The quiet failure: nothing about the numbers looks wrong, and the run is
    measuring the box rather than the harness."""
    ok, notes = compare_provenance(
        _card("local_mid", "qwen3.6-35b-a3b", "2x RTX 3090, pooled"),
        _card("local_mid", "qwen3.6-35b-a3b", "1x RTX 3090"),
    )
    assert ok
    assert any("faster box is not a better harness" in n for n in notes)


def test_the_gate_refuses_rather_than_producing_a_verdict_it_cannot_justify():
    result = check_gate(
        _card("local_mid", "qwen3.6-35b-a3b"),
        _card("frontier", "claude-opus-5"),
        margins={},
        comparability=_comparability(),
    )
    assert not result.passed
    assert any("not comparable" in r for r in result.reasons)
    assert not result.underpowered, (
        "an incomparable baseline is not an underpowered run; calling it one would "
        "teach everyone to read this as noise"
    )


def test_a_scorecard_carries_its_provenance():
    card = build_scorecard(
        "L6.0", [], provenance={"tier": "local_mid", "profile_id": "qwen3.6-35b-a3b"}
    )
    assert card["provenance"]["tier"] == "local_mid"
    # And a run that did not say gets an empty dict rather than a missing key, so
    # `compare_provenance` sees "unrecorded" instead of raising.
    assert build_scorecard("L6.0", [])["provenance"] == {}


def _comparability() -> dict:
    """The block as it is actually configured, read from gates.yaml rather than
    retyped — a test that invents its own policy passes after the real one is
    deleted."""
    import yaml

    cfg = yaml.safe_load((ROOT / "gates.yaml").read_text(encoding="utf-8"))
    return cfg["comparability"]


def test_gates_yaml_still_configures_comparability():
    block = _comparability()
    assert "tier" in block["require_same"]
    assert "profile_id" in block["require_same"]
    assert block["on_mismatch"] == "refuse"


def test_an_unconfigured_gate_does_not_invent_a_policy():
    """The gate enforces what it was configured to enforce. A caller comparing two
    scorecards directly has taken that responsibility, and a surprise refusal here
    would break every existing call site to enforce a rule nobody asked for."""
    result = check_gate(
        _card("local_mid", "qwen3.6-35b-a3b"),
        _card("frontier", "claude-opus-5"),
        margins={},
    )
    assert not any("not comparable" in r for r in result.reasons)
