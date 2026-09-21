//! The six batteries (AGENT-006 §6; Set S tasks S-8.2, S-8.3).
//!
//! Three verdict workflows and three capital workflows, declared step by step
//! against the tools that perform them.
//!
//! # Every step the spec names is here, including the ones nobody built
//!
//! Five of the six are [`Stage::Owed`] somewhere, and are therefore not in the
//! catalogue. That is the honest shape and not an oversight: a `leakage_audit`
//! that quietly ran three of its six checks would emit `clean` in exactly the
//! form the spec describes, and nothing downstream — not the funnel, not the
//! dossier, not a reader — could tell it apart from the real thing. The gap is
//! written into the battery where the battery is, so the next person building
//! `scan_strategy_for_lookahead` can see what it is for.
//!
//! What each owed stage needs is in its `needs` field, and
//! [`super::owed`] is the list.
//!
//! # "No edge" and "leaking" are different conclusions
//!
//! `leakage_audit` cannot answer `clean` when the label shuffle finds nothing,
//! because a strategy with no edge also survives a label shuffle. The two are
//! distinguished by which checks *fired*, which is why the verdict function
//! reads the steps rather than a single number.

use super::{Arg, Battery, Stage, Step, Verdict, When, Workflow};

// ── Helpers the verdict functions share ──────────────────────────────────────

fn because(v: &'static str, why: impl Into<String>) -> Verdict {
    Verdict {
        value: v,
        because: why.into(),
    }
}

/// The two answers every verdict function needs before it looks at anything:
/// nothing ran, or something broke.
///
/// A battery that could not complete does not get to emit the verdict it would
/// have emitted had every step run. That is the whole reason `inconclusive` (and
/// its siblings) are in each closed set — without them, a backend outage reads
/// as a pass.
fn cannot_say(b: &Battery, verdict: &'static str) -> Option<Verdict> {
    if b.steps.is_empty() {
        return Some(because(verdict, "no step of this battery ran"));
    }
    let failed = b.failures();
    if !failed.is_empty() {
        return Some(because(
            verdict,
            format!(
                "{} step(s) did not complete ({}), so this battery has not been run",
                failed.len(),
                failed.join(", ")
            ),
        ));
    }
    None
}

// ── S-8.2: the verdict workflows ─────────────────────────────────────────────

/// `leakage_audit` — is this result an artefact of information the strategy
/// could not have had?
///
/// Six checks, of which three have no tool. The three that exist are real and
/// are declared here so the battery is one edit from running.
fn leakage(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "suspect") {
        return v;
    }

    // Any flagged bar is a fact about the data. On its own it is not leakage —
    // `probe_data_quality` says so itself — so it raises suspicion rather than
    // deciding.
    let flagged = b
        .step("provenance")
        .and_then(|s| s.field("flagged"))
        .and_then(|f| f.as_array().map(Vec::len))
        .unwrap_or(0);

    // The label shuffle is the one check that can say `leaking` on its own.
    //
    // A `permutation_null` Study runs the strategy in a null world — the
    // structure the null destroys is gone. A **positive median there** means the
    // strategy still earns with the labels shuffled, which means it is reading
    // something other than the labels. There is no benign version of that.
    let shuffle_earns = b
        .step("label shuffle")
        .and_then(|s| s.study_flag("positive_median"));
    if shuffle_earns == Some(true) {
        return because(
            "leaking",
            "the strategy still earns with the labels shuffled, so its edge is not coming from \
             the labels",
        );
    }

    let mut suspicions: Vec<String> = Vec::new();
    if flagged > 0 {
        suspicions.push(format!(
            "{flagged} bar(s) failed the data-quality probe; show whether the P&L depends on \
             them before citing this result"
        ));
    }
    // An isolated spike is not a leak, but it is a result that has not been shown
    // to be about the market — worth saying before anyone cites it.
    if b.step("perturbation")
        .and_then(|s| s.study_flag("plateau"))
        == Some(false)
    {
        suspicions.push(
            "the result is an isolated spike rather than a plateau: it holds at the parameters \
             it was found at and not around them"
                .to_string(),
        );
    }
    // A shuffle that could not be read is not a shuffle that came back clean.
    if shuffle_earns.is_none() {
        suspicions.push(
            "the label shuffle did not report a median, so the one check that can detect a leak \
             on its own did not answer"
                .to_string(),
        );
    }
    if !suspicions.is_empty() {
        return because("suspect", suspicions.join("; and "));
    }
    because(
        "clean",
        "nothing in this battery found information the strategy could not have had. This is \
         not a claim that it has an edge — a strategy with no edge passes every check here",
    )
}

/// `selection_bias_verdict` — is this result what the best of that many looks
/// looks like?
fn selection_bias(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "inconclusive") {
        return v;
    }

    let survivors = b
        .step("family-wise correction")
        .and_then(|s| s.number("n_survivors"));
    let candidates = b
        .step("family-wise correction")
        .and_then(|s| s.number("n_candidates"));
    let blocks = b
        .step("minimum track record")
        .and_then(|s| s.flag("blocks_promotion"));
    let pbo = b
        .step("probability of overfit")
        .and_then(|s| s.number("probability_of_overfit"));
    let trials = b
        .step("deflated Sharpe")
        .and_then(|s| s.number("trials"));
    let agree = b
        .step("deflated Sharpe")
        .and_then(|s| s.flag("corroborators_agree"));

    // A number this battery could not read is not a number it may assume. Every
    // default here would be the permissive one.
    let (Some(survivors), Some(blocks), Some(pbo), Some(trials), Some(agree)) =
        (survivors, blocks, pbo, trials, agree)
    else {
        return because(
            "inconclusive",
            "one of the four statistics did not come back with the field this verdict reads; \
             a missing number is not a passing one",
        );
    };

    if survivors < 1.0 {
        return because(
            "fails",
            format!(
                "nothing in the family of {} survives the family-wise correction over {trials} \
                 effective trials — the best result of the sweep is what the best of that many \
                 draws looks like",
                candidates.unwrap_or(f64::NAN)
            ),
        );
    }
    if blocks {
        return because(
            "fails",
            "the run is too short for this Sharpe to be distinguished from luck, which is a \
             promotion precondition and not a suggestion",
        );
    }
    if pbo >= 0.5 {
        return because(
            "fails",
            format!(
                "the in-sample-best configuration underperforms out of sample more often than \
                 not (PBO {pbo:.2}), which is what an overfitted sweep looks like"
            ),
        );
    }
    if pbo >= 0.2 || !agree {
        return because(
            "inconclusive",
            if agree {
                format!("PBO {pbo:.2} is elevated: the selection carries real risk of being noise")
            } else {
                "the deflated Sharpe's corroborators disagree with its primary p-value; \
                 investigate before citing either"
                    .to_string()
            },
        );
    }
    because(
        "passes",
        format!(
            "{survivors} candidate(s) survive the correction over {trials} effective trials, \
             PBO {pbo:.2}, and the track record is long enough to tell this apart from luck"
        ),
    )
}

/// `hypothesis_precheck` — has this been asked, and has it been answered?
fn precheck(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "blocked") {
        return v;
    }
    let repeats = b
        .step("duplicate search")
        .and_then(|s| s.field("trials"))
        .and_then(|t| t.as_array().map(Vec::len))
        .unwrap_or(0);
    if repeats > 0 {
        return because(
            "duplicate",
            format!("{repeats} trial(s) already ran this configuration; read them first"),
        );
    }
    because("proceed", "nothing in the ledger has asked this yet")
}

// ── S-8.3: the capital workflows ─────────────────────────────────────────────

/// Generators whose output may contribute to a pass.
///
/// Bootstrap and fitted parametric generators resample or re-parameterise the
/// data the strategy was measured on, so a pass over their paths is a statement
/// about that data. A GAN or diffusion generator produces paths whose
/// relationship to the original process is whatever the generator learned, and
/// a strategy that survives them may have survived an artefact of the generator.
/// That is not a reason to forbid running them — it is a reason they cannot be
/// the thing that licenses capital.
pub const PASSING_GENERATORS: &[&str] = &[
    "block_bootstrap",
    "stationary_bootstrap",
    "iid_bootstrap",
    "fitted_garch",
    "fitted_ar",
];

/// `precapital_battery` — what has to survive before this is worth paper money.
fn precapital(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "reject") {
        return v;
    }

    // S-8.3's acceptance, and the one check here that is about *provenance*
    // rather than about a number: a generator outside the list cannot
    // contribute to a pass, whatever its paths said.
    let generator = b
        .step("synthetic stress")
        .and_then(|s| s.field("generator"))
        .and_then(|g| g.as_str())
        .map(str::to_string);
    let synthetic_counts = match generator.as_deref() {
        None => false,
        Some(g) => {
            PASSING_GENERATORS.contains(&g)
                && b.step("synthetic stress").and_then(super::Ran::study_survived) == Some(true)
        }
    };

    // The cost model's own test, first. If a perfect-foresight strategy trading
    // every bar is profitable, the cost model is not charging what it claims and
    // **every** number in this battery was measured under it — so this rejects
    // before any of the others is read.
    if b.step("red-team falsify")
        .and_then(|s| s.flag("cost_model_is_sane"))
        == Some(false)
    {
        return because(
            "reject",
            "the oracle test failed: a perfect-foresight strategy trading every bar is \
             profitable at this horizon, so the cost model is not charging what it claims and \
             every result in this battery was measured under it",
        );
    }

    // A Study survives when its median is above zero **and** its worst-5% is
    // survivable. Not a p-value: a Study does not produce one, and reading a
    // field nothing writes would have rejected every strategy forever.
    let survived = |label: &str| b.step(label).and_then(super::Ran::study_survived);
    let hard = ["ablation", "parameter fragility", "cost shock"];
    let failed: Vec<&str> = hard
        .iter()
        .copied()
        .filter(|l| survived(l) != Some(true))
        .collect();

    if !failed.is_empty() {
        return because(
            "reject",
            format!(
                "the strategy does not survive {}; a result that depends on the component it \
                 was ablated against is the component",
                failed.join(" or ")
            ),
        );
    }

    // Fragility asks a second question the other Studies do not: a broad plateau
    // or an isolated spike. A spike that survives on its median is a strategy
    // that works at the parameters it was found at.
    if b.step("parameter fragility")
        .and_then(|s| s.study_flag("plateau"))
        == Some(false)
    {
        return because(
            "reject",
            "the neighbourhood is an isolated spike rather than a plateau: the result holds at \
             the parameters it was found at and not around them",
        );
    }
    let soft = ["trade reshuffle", "regime conditioning"];
    let wobbled: Vec<&str> = soft
        .iter()
        .copied()
        .filter(|l| survived(l) != Some(true))
        .collect();
    if !wobbled.is_empty() || !synthetic_counts {
        let mut why = Vec::new();
        if !wobbled.is_empty() {
            why.push(format!("it does not survive {}", wobbled.join(" or ")));
        }
        if !synthetic_counts {
            why.push(match generator {
                Some(g) => format!(
                    "the synthetic stress ran on `{g}`, which is not a generator whose paths \
                     can license capital ({})",
                    PASSING_GENERATORS.join(", ")
                ),
                None => "the synthetic stress did not say which generator produced its paths"
                    .to_string(),
            });
        }
        return because("reduced_size_paper", why.join("; and "));
    }
    because(
        "full_paper",
        "the result survives ablation, fragility, cost shock, reshuffling, regime conditioning \
         and synthetic paths from a generator whose output can license capital",
    )
}

/// `signal_identity` — what *is* this signal, if it is anything?
fn identity(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "unidentified") {
        return v;
    }
    because(
        "ambiguous",
        "this battery has no step that can identify a signal yet; see its owed stages",
    )
}

/// `go_live_readiness` — can this be traded, as opposed to measured?
fn readiness(b: &Battery) -> Verdict {
    if let Some(v) = cannot_say(b, "no_go") {
        return v;
    }
    because(
        "conditional",
        "the venue constraints were read; capacity, funding, divergence and a shadow session \
         are not yet checkable — see this battery's owed stages",
    )
}

// ── The declarations ─────────────────────────────────────────────────────────

/// A `run_study` step: the battery decides the kind, the metric and the
/// question, and the caller supplies only the dimension to vary.
macro_rules! study {
    ($label:expr, $kind:expr, $vary_arg:expr, $question:expr) => {
        Stage::Call(Step {
            tool: "run_study",
            label: $label,
            forward: &["experiment_id", "metric", "base_params"],
            rename: &[("vary", $vary_arg)],
            pinned: &[
                ("kind", Arg::Str($kind)),
                ("study_id", Arg::StepId),
                ("question", Arg::Str($question)),
                // Never a selection rule inside a battery. A battery that
                // carried a config forward would be choosing a winner from the
                // evidence it was gathering about winners.
                ("selection_rule", Arg::Str("none")),
            ],
            when: When::Always,
        })
    };
}

pub static ALL: &[Workflow] = &[
    Workflow {
        name: "leakage_audit",
        namespace: "audit",
        description: "One call: provenance, bar-boundary alignment, label shuffle and \
                      perturbation. Answers clean, suspect or leaking — `clean` is not `has \
                      an edge`. Refused for text features or SLv2 expressions.",
        arguments: &[
            ("experiment_id", "the experiment the result belongs to", true),
            ("instrument_id", "the instrument whose bars are probed", true),
            ("metric", "the metric the studies are scored on", true),
            ("base_params", "the configuration under test", true),

            ("timeframe", "the fast bar timeframe, e.g. 1m", false),
            ("higher_timeframe", "the aggregate the strategy also reads, e.g. 1h", true),
            (
                "rich_expressions",
                "true when the strategy uses SLv2 expressions",
                false,
            ),
            ("start", "window start", false),
            ("end", "window end", false),
            ("null_seeds", "`vary` for the label shuffle (seeds)", true),
            ("perturbation", "`vary` for the perturbation (neighborhood)", true),
            ("text_features", "true when the feature set contains text", false),
        ],
        subject: "experiment_id",
        stages: &[
            Stage::Call(Step {
                tool: "probe_data_quality",
                label: "provenance",
                forward: &["instrument_id", "timeframe", "start", "end"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            // **Deferred by design, not unbuilt** (S-3.11). The frozen expression
            // grammar admits exactly two calls — `feature(name)` and
            // `bar(open|high|low|close|volume)` — and the feature vocabulary a
            // backtest can evaluate is causal. A scan for full-window
            // statistics, future correlation and disguised label transforms has
            // nothing expressible to find, so building it now would produce a
            // pass that returns clean on every strategy the platform can
            // represent. **A leakage scan that has never found anything reads as
            // evidence there is nothing to find**, which is worse than not
            // having one.
            //
            // Conditional on `rich_expressions`, which is what SLv2 (phase 5)
            // introduces: expressions rich enough to hide a leak. A caller that
            // says its strategy uses them is refused, naming this step.
            Stage::Owed {
                label: "static scan",
                needs: "a `feature_leakage_scan` pass over SLv2 expressions \
                        (`crates/strategy-validator`, phase 5)",
                why: "the cheapest leak to find and the only one findable without running \
                      anything. Deferred rather than missing: today's grammar cannot express \
                      the constructs it looks for, so it would return clean on everything",
                when: When::IfArgument("rich_expressions"),
            },
            // This was declared owed and **is built** (S-3.11):
            // `bar_boundary_check` is a templated tool and has been since
            // phase 3. It answers the question from the other end than the one
            // this battery first asked for — it proves whether a
            // higher-timeframe bar could have been read before it closed, by
            // checking every aggregate against only the constituents that had
            // completed, which catches the defect for every strategy at once
            // rather than one run's fills at a time.
            Stage::Call(Step {
                tool: "bar_boundary_check",
                label: "bar boundary",
                forward: &["instrument_id", "timeframe", "higher_timeframe", "start", "end"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            study!(
                "label shuffle",
                "permutation_null",
                "null_seeds",
                "does the edge survive shuffling the labels?"
            ),
            study!(
                "perturbation",
                "neighborhood",
                "perturbation",
                "does the edge survive perturbing the parameters it was found at?"
            ),
            // **A stated non-goal**, not an oversight: AGENT-006 §11 defers
            // `llm_contamination_check` "until a text feature exists". So this
            // is conditional on the caller saying its feature set contains text
            // — for a price feature set the step does not apply and the battery
            // runs; for a text one the battery refuses and names this.
            Stage::Owed {
                label: "contamination",
                needs: "an `llm_contamination_probe` tool over the text feature corpus \
                        (AGENT-006 §11 defers it until a text feature exists)",
                why: "a language model that saw the evaluation window during pre-training \
                      produces a feature that knows the future, and no amount of bar forensics \
                      finds it",
                when: When::IfArgument("text_features"),
            },
        ],
        verdicts: &["clean", "suspect", "leaking"],
        verdict: leakage,
    },
    Workflow {
        name: "selection_bias_verdict",
        namespace: "audit",
        description: "One call: the full selection-bias battery over a result — effective \
                      trial count, minimum track record, deflated Sharpe, probability of \
                      overfit and the family-wise correction. Answers passes, fails or \
                      inconclusive.",
        arguments: &[
            ("run_id", "the run whose result is under audit", true),
            (
                "experiment_id",
                "the hypothesis family the run belongs to. A Sharpe discounted by a trial \
                 count nobody registered is not discounted at all",
                true,
            ),
            ("n_periods", "CSCV splits for the overfit probability (2-16)", false),
            ("alpha", "family-wise error rate (default 0.05)", false),
            ("seed", "bootstrap seed, so the same family gives the same verdict twice", false),
        ],
        // Four calls for five of §6's steps, and this is the one place in this
        // file where that is not a gap. N_eff is not a separate look: it is
        // `trials` on the deflated Sharpe, computed from the ledger by the route
        // that discounts by it. Calling a fifth tool to read the same number
        // would register a second look at the same evidence.
        subject: "experiment_id",
        stages: &[
            Stage::Call(Step {
                tool: "min_track_record",
                label: "minimum track record",
                forward: &["run_id"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            Stage::Call(Step {
                tool: "deflated_sharpe",
                label: "deflated Sharpe",
                forward: &["run_id", "experiment_id"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            Stage::Call(Step {
                tool: "probability_of_overfit",
                label: "probability of overfit",
                forward: &["experiment_id", "n_periods"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            Stage::Call(Step {
                tool: "family_wise_test",
                label: "family-wise correction",
                forward: &["experiment_id", "alpha", "seed"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
        ],
        verdicts: &["passes", "fails", "inconclusive"],
        verdict: selection_bias,
    },
    Workflow {
        name: "hypothesis_precheck",
        // `audit` rather than `research`, for two reasons and the second is the
        // binding one. What it does is check whether a question has already been
        // answered, which is what this namespace is for. And `research` is at
        // AGENT-006 §2.1's cap of sixteen: a seventeenth would fail
        // `no_namespace_holds_more_than_sixteen_tools` the day this battery
        // becomes runnable, which is a trap to leave for whoever builds its
        // last tool.
        namespace: "audit",
        description: "One call, before a hypothesis is registered: has this configuration \
                      already been tried, is the hypothesis already in the registry, and did \
                      it already produce a negative result?",
        arguments: &[
            // Required, though `find_similar_trials` would accept a config hash
            // alone: a battery files its verdict against a subject, and a
            // verdict filed against nothing cannot be looked up by the thing it
            // is about.
            ("experiment_id", "the line of work this belongs to", true),
            ("config_hash", "the configuration about to be tried", false),
            ("statement", "the hypothesis, for the registry lookup", true),
        ],
        subject: "experiment_id",
        stages: &[
            Stage::Call(Step {
                tool: "find_similar_trials",
                label: "duplicate search",
                forward: &["config_hash", "experiment_id"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            Stage::Owed {
                label: "registry lookup",
                needs: "a read tool over the hypothesis registry, by statement rather than by id",
                why: "`hypothesise` writes the registry and nothing reads it by content, so \
                      the same hypothesis can be registered twice under two ids and both \
                      counted as independent looks",
                when: When::Always,
            },
            Stage::Owed {
                label: "negative results",
                needs: "a read over prior `execution_infeasible` and refuted verdicts, and the \
                        capability-delta rule that lets one be retried",
                why: "AGENT-006 §6 allows a prior `execution_infeasible` to be retried only \
                      with a cited capability delta. Nothing stores the deltas, so the rule \
                      cannot be applied and a run can re-ask a question the platform has \
                      already answered no to",
                when: When::Always,
            },
        ],
        verdicts: &["proceed", "duplicate", "blocked"],
        verdict: precheck,
    },
    Workflow {
        name: "precapital_battery",
        namespace: "audit",
        description: "One call: everything that has to survive before a strategy is worth \
                      paper money — ablation, fragility, cost shock, trade reshuffle, timing \
                      jitter, regime conditioning, synthetic stress and a red-team pass. \
                      Answers reject, reduced_size_paper or full_paper.",
        arguments: &[
            ("experiment_id", "the experiment under test", true),
            ("instrument_id", "the instrument, for the red-team oracle pass", true),
            ("metric", "the metric every study is scored on", true),
            ("base_params", "the configuration under test", true),
            ("ablation_grid", "`vary` for the ablation: {\"vary\":\"params\",\"grid\":[...]}", true),
            ("neighborhood", "`vary` for the fragility map", true),
            ("cost_ladder", "`vary` for the cost shock: optimistic to pessimistic", true),
            ("trade_resamples", "`vary` for the trade reshuffle", true),
            ("regime_windows", "`vary` for the regime conditioning", true),
            ("synthetic_seeds", "`vary` for the synthetic stress", true),
        ],
        subject: "experiment_id",
        stages: &[
            study!(
                "ablation",
                "parameter_sweep",
                "ablation_grid",
                "which component is the result actually coming from?"
            ),
            study!(
                "parameter fragility",
                "neighborhood",
                "neighborhood",
                "is this a region or a point?"
            ),
            study!(
                "cost shock",
                "cost_sweep",
                "cost_ladder",
                "at what cost assumption does this stop working?"
            ),
            study!(
                "trade reshuffle",
                "trade_monte_carlo",
                "trade_resamples",
                "how much of this is the order the trades happened to arrive in?"
            ),
            Stage::Owed {
                label: "timing jitter",
                needs: "a `timing_jitter_sensitivity` tool, and an execution-delay dimension \
                        in the simulator for it to vary",
                why: "a strategy whose edge disappears under a one-bar fill delay has no edge \
                      that survives a real venue. `VarySpec` has no jitter dimension, so this \
                      cannot be a `run_study` kind — the simulator has to gain the knob first",
                when: When::Always,
            },
            study!(
                "regime conditioning",
                "regime_conditional",
                "regime_windows",
                "is this one regime's result wearing the whole sample's clothes?"
            ),
            study!(
                "synthetic stress",
                "synthetic_paths",
                "synthetic_seeds",
                "does it survive paths the market did not take?"
            ),
            Stage::Call(Step {
                tool: "run_oracle_test",
                label: "red-team falsify",
                forward: &["instrument_id"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
        ],
        verdicts: &["full_paper", "reduced_size_paper", "reject"],
        verdict: precapital,
    },
    Workflow {
        name: "signal_identity",
        namespace: "audit",
        description: "One call: what this signal is, if it is anything — half-life, spanning \
                      against known factors, style, cross-sectional versus time-series, \
                      health and crowding.",
        arguments: &[
            ("experiment_id", "the experiment whose signal is being identified", true),
            ("run_id", "the run that produced it", true),
        ],
        subject: "experiment_id",
        stages: &[
            Stage::Owed {
                label: "half-life",
                needs: "a `signal_half_life` tool over the signal series",
                why: "a signal whose half-life is shorter than the holding period is not the \
                      signal the backtest traded",
                when: When::Always,
            },
            Stage::Owed {
                label: "spanning",
                needs: "a `spanning_regression` tool against the platform's factor set",
                why: "a signal spanned by momentum is momentum, and paying for it twice is \
                      the most expensive thing a research programme can do",
                when: When::Always,
            },
            Stage::Owed {
                label: "style",
                needs: "a style-attribution tool",
                why: "without it, `signal_identity` cannot say what kind of thing it found",
                when: When::Always,
            },
            Stage::Owed {
                label: "cross-section vs time series",
                needs: "a tool that re-scores the signal cross-sectionally and as a time series",
                why: "the two have different capacity and different failure modes, and a \
                      signal presented as one when it is the other is sized wrongly",
                when: When::Always,
            },
            Stage::Owed {
                label: "health check",
                needs: "a decay/turnover health tool over the live signal",
                why: "a signal that has decayed since the sample is a signal that is no longer \
                      there",
                when: When::Always,
            },
            Stage::Owed {
                label: "crowding",
                needs: "a crowding proxy over positioning or flow data the platform does not \
                        hold",
                why: "free-only data (BS-007) has no positioning feed, so this may be owed for \
                      a long time — which is worth stating rather than quietly dropping",
                when: When::Always,
            },
        ],
        verdicts: &["identified", "ambiguous", "unidentified"],
        verdict: identity,
    },
    Workflow {
        name: "go_live_readiness",
        namespace: "audit",
        description: "One call, before paper becomes live: capacity, venue constraints, \
                      funding, divergence from the backtest and a shadow session. Answers go, \
                      conditional or no_go.",
        arguments: &[
            ("experiment_id", "the experiment being readied", true),
            ("venue", "the venue it would trade on", true),
        ],
        subject: "experiment_id",
        stages: &[
            Stage::Owed {
                label: "capacity",
                needs: "a capacity estimate from the trade list and the instrument's depth",
                why: "a strategy that works at a size nobody would trade is not ready for \
                      anything, and the number is not derivable from a backtest that assumed \
                      infinite depth",
                when: When::Always,
            },
            Stage::Call(Step {
                tool: "get_venue_constraints",
                label: "venue constraints",
                forward: &["venue"],
                rename: &[],
                pinned: &[],
                when: When::Always,
            }),
            Stage::Owed {
                label: "funding",
                needs: "a funding-and-borrow cost read for the instrument",
                why: "a perpetual's funding can exceed the edge, and the backtest's cost model \
                      does not carry it",
                when: When::Always,
            },
            Stage::Owed {
                label: "divergence",
                needs: "a paper-versus-backtest divergence read over the paper session",
                why: "the one check that compares what the platform predicted to what the \
                      platform did. Without it, going live is the first time anyone finds out",
                when: When::Always,
            },
            Stage::Owed {
                label: "shadow session",
                needs: "a shadow-trading mode that computes intents against live marks without \
                        placing orders",
                why: "the last rehearsal before capital, and the only one that exercises the \
                      live path end to end",
                when: When::Always,
            },
        ],
        verdicts: &["go", "conditional", "no_go"],
        verdict: readiness,
    },
];

#[cfg(test)]
mod tests {
    use super::super::{Ran, Status};
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn wf(name: &str) -> &'static Workflow {
        ALL.iter().find(|w| w.name == name).expect("declared")
    }

    fn step(label: &'static str, detail: serde_json::Value) -> Ran {
        Ran {
            tool: "x",
            label,
            status: Status::Ran,
            ids: BTreeMap::new(),
            detail: json!({ "detail": detail }),
        }
    }

    /// One `run_study` answer, in the shape `backtest::suite::StudyView`
    /// serialises: flat, with the conclusion under `verdict`.
    ///
    /// Written as the real thing rather than as whatever the verdict function
    /// happens to read — which is the mistake this helper replaced. The two are
    /// held together by `invariants::a_battery_reads_the_study_it_ran`, which
    /// builds this shape from `backtest`'s own types.
    fn study(label: &'static str, positive: bool, survivable: bool, plateau: Option<bool>) -> Ran {
        Ran {
            tool: "run_study",
            label,
            status: Status::Ran,
            ids: BTreeMap::new(),
            detail: json!({
                "study_id": format!("fam-{label}"),
                "sealed": true,
                "trial_delta": 4,
                "verdict": {
                    "summary": "v",
                    "positive_median": positive,
                    "survivable_worst5": survivable,
                    "plateau": plateau,
                },
            }),
        }
    }

    fn battery(name: &'static str, steps: Vec<Ran>) -> Battery {
        Battery {
            workflow: name,
            family_id: "fam".into(),
            steps,
        }
    }

    /// The distinction S-8.2 names in as many words.
    #[test]
    fn no_edge_and_leaking_are_different_conclusions() {
        // A strategy with no edge: the shuffle finds nothing, because there was
        // nothing to find. That is `clean`, not `leaking`.
        // A `permutation_null` Study with a non-positive median: the strategy
        // earns nothing once the labels are shuffled, which is what a strategy
        // with no edge and a strategy with a real one both do.
        let no_edge = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [] })),
                study("label shuffle", false, true, None),
                study("perturbation", true, true, Some(true)),
            ],
        );
        assert_eq!(leakage(&no_edge).value, "clean", "{}", leakage(&no_edge).because);

        // A strategy whose edge survives the shuffle is reading something other
        // than the labels. Same battery, opposite conclusion.
        let leaks = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [] })),
                study("label shuffle", true, true, None),
                study("perturbation", true, true, Some(true)),
            ],
        );
        assert_eq!(leakage(&leaks).value, "leaking");
    }

    /// `clean` says what it means, so a reader cannot take it for a result.
    #[test]
    fn clean_does_not_claim_an_edge() {
        let b = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [] })),
                study("label shuffle", false, true, None),
            ],
        );
        assert!(leakage(&b).because.contains("no edge"), "{}", leakage(&b).because);
    }

    /// A flagged bar raises suspicion and does not decide — which is what
    /// `probe_data_quality` says about its own output.
    #[test]
    fn a_flagged_bar_is_suspect_and_not_leaking() {
        let b = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [{ "index": 4 }] })),
                study("label shuffle", false, true, None),
            ],
        );
        assert_eq!(leakage(&b).value, "suspect");
        assert!(leakage(&b).because.contains("data-quality"));
    }

    /// A shuffle that did not answer is not a shuffle that came back clean.
    ///
    /// The defect this replaced: the verdict function read a `p_value` no Study
    /// produces, so the one check that can detect a leak on its own silently
    /// never fired, and every battery would have answered `clean`.
    #[test]
    fn a_shuffle_that_did_not_answer_is_not_a_clean_one() {
        let b = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [] })),
                // A Study answer with no verdict object — what a shape change
                // would look like from here.
                step("label shuffle", json!({ "study_id": "s1" })),
            ],
        );
        let v = leakage(&b);
        assert_eq!(v.value, "suspect", "{}", v.because);
        assert!(v.because.contains("did not answer"), "{}", v.because);
    }

    /// An isolated spike is not a leak and is not nothing.
    #[test]
    fn a_spike_rather_than_a_plateau_is_suspect() {
        let b = battery(
            "leakage_audit",
            vec![
                step("provenance", json!({ "flagged": [] })),
                study("label shuffle", false, true, None),
                study("perturbation", true, true, Some(false)),
            ],
        );
        let v = leakage(&b);
        assert_eq!(v.value, "suspect");
        assert!(v.because.contains("isolated spike"), "{}", v.because);
    }

    fn clean_selection() -> Vec<Ran> {
        vec![
            step("minimum track record", json!({ "blocks_promotion": false })),
            step(
                "deflated Sharpe",
                json!({ "trials": 40, "corroborators_agree": true }),
            ),
            step(
                "probability of overfit",
                json!({ "probability_of_overfit": 0.08 }),
            ),
            step(
                "family-wise correction",
                json!({ "n_survivors": 2, "n_candidates": 60 }),
            ),
        ]
    }

    #[test]
    fn a_result_that_survives_every_correction_passes() {
        let v = selection_bias(&battery("selection_bias_verdict", clean_selection()));
        assert_eq!(v.value, "passes");
        assert!(v.because.contains("40"), "{}", v.because);
    }

    /// Each of the three refusals fires on its own.
    #[test]
    fn each_correction_can_fail_the_verdict_alone() {
        for (label, detail) in [
            (
                "family-wise correction",
                json!({ "n_survivors": 0, "n_candidates": 60 }),
            ),
            ("minimum track record", json!({ "blocks_promotion": true })),
            (
                "probability of overfit",
                json!({ "probability_of_overfit": 0.7 }),
            ),
        ] {
            let mut steps = clean_selection();
            let at = steps.iter().position(|s| s.label == label).expect("present");
            steps[at] = step(steps[at].label, detail);
            let v = selection_bias(&battery("selection_bias_verdict", steps));
            assert_eq!(v.value, "fails", "{label}: {}", v.because);
        }
    }

    /// A number the battery could not read is not a number it may assume. Every
    /// default would be the permissive one.
    #[test]
    fn a_missing_statistic_is_inconclusive_and_not_a_pass() {
        for label in [
            "minimum track record",
            "deflated Sharpe",
            "probability of overfit",
            "family-wise correction",
        ] {
            let steps: Vec<Ran> = clean_selection()
                .into_iter()
                .map(|s| {
                    if s.label == label {
                        step(s.label, json!({}))
                    } else {
                        s
                    }
                })
                .collect();
            let v = selection_bias(&battery("selection_bias_verdict", steps));
            assert_eq!(v.value, "inconclusive", "{label}");
        }
    }

    /// Corroborators that disagree stop a pass without becoming a failure —
    /// they are a reason to look, not a result.
    #[test]
    fn disagreeing_corroborators_are_inconclusive() {
        let mut steps = clean_selection();
        steps[1] = step(
            "deflated Sharpe",
            json!({ "trials": 40, "corroborators_agree": false }),
        );
        let v = selection_bias(&battery("selection_bias_verdict", steps));
        assert_eq!(v.value, "inconclusive");
        assert!(v.because.contains("corroborators"), "{}", v.because);
    }

    fn clean_precapital() -> Vec<Ran> {
        let mut synthetic = study("synthetic stress", true, true, None);
        // The generator rides on the Study's own answer, beside the verdict.
        synthetic.detail["generator"] = json!("block_bootstrap");
        vec![
            study("ablation", true, true, None),
            study("parameter fragility", true, true, Some(true)),
            study("cost shock", true, true, None),
            study("trade reshuffle", true, true, None),
            study("regime conditioning", true, true, None),
            synthetic,
            step("red-team falsify", json!({ "cost_model_is_sane": true })),
        ]
    }

    fn with_generator(steps: &mut [Ran], generator: Option<&str>) {
        let at = steps
            .iter()
            .position(|s| s.label == "synthetic stress")
            .expect("present");
        match generator {
            Some(g) => steps[at].detail["generator"] = json!(g),
            None => {
                steps[at].detail.as_object_mut().expect("object").remove("generator");
            }
        }
    }

    #[test]
    fn a_strategy_that_survives_the_whole_battery_reaches_full_paper() {
        let v = precapital(&battery("precapital_battery", clean_precapital()));
        assert_eq!(v.value, "full_paper", "{}", v.because);
    }

    /// S-8.3's acceptance, and the reason it is a list rather than a threshold:
    /// a GAN or diffusion generator can never contribute to a pass.
    #[test]
    fn a_learned_generator_cannot_license_capital() {
        for generator in ["timegan", "diffusion_v2", "quantgan", "made_up"] {
            let mut steps = clean_precapital();
            with_generator(&mut steps, Some(generator));
            let v = precapital(&battery("precapital_battery", steps));
            assert_ne!(
                v.value, "full_paper",
                "`{generator}` licensed capital: {}",
                v.because
            );
            assert!(v.because.contains(generator), "{}", v.because);
        }
        // And every generator whose paths *can* is accepted, so the list is a
        // rule rather than a single-name special case.
        for generator in PASSING_GENERATORS {
            let mut steps = clean_precapital();
            with_generator(&mut steps, Some(generator));
            assert_eq!(
                precapital(&battery("precapital_battery", steps)).value,
                "full_paper",
                "{generator}"
            );
        }
    }

    /// A synthetic step that does not say which generator produced its paths is
    /// treated as one that cannot license capital. Unknown provenance is not
    /// good provenance.
    #[test]
    fn an_unnamed_generator_is_not_a_permitted_one() {
        let mut steps = clean_precapital();
        with_generator(&mut steps, None);
        let v = precapital(&battery("precapital_battery", steps));
        assert_eq!(v.value, "reduced_size_paper");
        assert!(v.because.contains("did not say"), "{}", v.because);
    }

    /// The three hard checks reject; the soft ones reduce the size.
    #[test]
    fn the_hard_checks_reject_and_the_soft_ones_reduce_size() {
        // Failing on the median and failing on the tail are both failures, and
        // both are checked — a strategy with a positive median and an
        // unsurvivable worst-5% is the one that works until it does not.
        for (positive, survivable) in [(false, true), (true, false), (false, false)] {
            for label in ["ablation", "parameter fragility", "cost shock"] {
                let steps: Vec<Ran> = clean_precapital()
                    .into_iter()
                    .map(|s| {
                        if s.label == label {
                            study(s.label, positive, survivable, Some(true))
                        } else {
                            s
                        }
                    })
                    .collect();
                assert_eq!(
                    precapital(&battery("precapital_battery", steps)).value,
                    "reject",
                    "{label} positive={positive} survivable={survivable}"
                );
            }
            for label in ["trade reshuffle", "regime conditioning"] {
                let steps: Vec<Ran> = clean_precapital()
                    .into_iter()
                    .map(|s| {
                        if s.label == label {
                            study(s.label, positive, survivable, None)
                        } else {
                            s
                        }
                    })
                    .collect();
                assert_eq!(
                    precapital(&battery("precapital_battery", steps)).value,
                    "reduced_size_paper",
                    "{label} positive={positive} survivable={survivable}"
                );
            }
        }
    }

    /// An isolated spike rejects even when every median is positive. The
    /// neighbourhood Study is the only one that answers this question.
    #[test]
    fn an_isolated_spike_is_rejected() {
        let steps: Vec<Ran> = clean_precapital()
            .into_iter()
            .map(|s| {
                if s.label == "parameter fragility" {
                    study(s.label, true, true, Some(false))
                } else {
                    s
                }
            })
            .collect();
        let v = precapital(&battery("precapital_battery", steps));
        assert_eq!(v.value, "reject");
        assert!(v.because.contains("isolated spike"), "{}", v.because);
    }

    /// The oracle test rejects before anything else is read: if the cost model
    /// is not charging what it claims, every other number in the battery was
    /// measured under it.
    #[test]
    fn a_broken_cost_model_rejects_before_any_study_is_read() {
        let steps: Vec<Ran> = clean_precapital()
            .into_iter()
            .map(|s| {
                if s.label == "red-team falsify" {
                    step(s.label, json!({ "cost_model_is_sane": false }))
                } else {
                    s
                }
            })
            .collect();
        let v = precapital(&battery("precapital_battery", steps));
        assert_eq!(v.value, "reject");
        assert!(v.because.contains("cost model"), "{}", v.because);
    }

    /// A Study that did not answer is not a Study that passed. The bug this
    /// replaced read a `p_value` no Study produces, and the reject was silent
    /// and permanent; this pins the *other* direction — an unanswered Study must
    /// not pass either.
    #[test]
    fn a_study_that_did_not_answer_does_not_pass() {
        let steps: Vec<Ran> = clean_precapital()
            .into_iter()
            .map(|s| {
                if s.label == "ablation" {
                    step(s.label, json!({ "study_id": "s1" }))
                } else {
                    s
                }
            })
            .collect();
        assert_eq!(
            precapital(&battery("precapital_battery", steps)).value,
            "reject"
        );
    }

    /// Every battery names the arguments its steps read, and every argument a
    /// step forwards or renames is one the schema declares. Otherwise a battery
    /// asks for a field the model has no way to know about.
    #[test]
    fn every_argument_a_step_reads_is_in_the_schema() {
        for w in ALL {
            let declared: Vec<&str> = w.arguments.iter().map(|(n, _, _)| *n).collect();
            for s in w.calls() {
                for f in s.forward {
                    assert!(
                        declared.contains(f),
                        "`{}` step `{}` forwards `{f}`, which is not in its schema",
                        w.name,
                        s.label
                    );
                }
                for (_, from) in s.rename {
                    assert!(
                        declared.contains(from),
                        "`{}` step `{}` reads `{from}`, which is not in its schema",
                        w.name,
                        s.label
                    );
                }
            }
            if let When::IfArgument(flag) = s_when(w) {
                assert!(declared.contains(&flag), "`{}` gates on `{flag}`", w.name);
            }
        }
    }

    fn s_when(w: &Workflow) -> When {
        w.calls()
            .iter()
            .map(|s| s.when)
            .find(|x| matches!(x, When::IfArgument(_)))
            .unwrap_or(When::Always)
    }

    /// No battery carries a configuration forward. A battery that selected a
    /// winner would be choosing from the evidence it was gathering about
    /// choosing.
    #[test]
    fn no_battery_carries_a_configuration_forward() {
        for w in ALL {
            for s in w.calls() {
                if s.tool != "run_study" {
                    continue;
                }
                let rule = s
                    .pinned
                    .iter()
                    .find(|(k, _)| *k == "selection_rule")
                    .map(|(_, v)| *v);
                assert_eq!(
                    rule,
                    Some(Arg::Str("none")),
                    "`{}` step `{}` does not pin `selection_rule` to none",
                    w.name,
                    s.label
                );
            }
        }
    }

    /// Every battery's name is unique, because the catalogue would otherwise
    /// panic at first use with a duplicate.
    #[test]
    fn battery_names_are_unique() {
        let mut names: Vec<&str> = ALL.iter().map(|w| w.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
    }

    /// The batteries that are not runnable say so, and the count is the one the
    /// plan records. A battery becoming runnable changes this number, which is
    /// the point — nobody has to remember to cross it off.
    #[test]
    fn the_owed_stages_are_the_ones_the_plan_records() {
        let owed = super::super::owed();
        let by_workflow: Vec<(&str, usize)> = ALL
            .iter()
            .map(|w| (w.name, w.owed().len()))
            .filter(|(_, n)| *n > 0)
            .collect();
        assert_eq!(
            by_workflow,
            vec![
                ("leakage_audit", 2),
                ("hypothesis_precheck", 2),
                ("precapital_battery", 1),
                ("signal_identity", 6),
                ("go_live_readiness", 4),
            ],
            "the owed list changed; update phase-8-integration.md and COMPLACENCY_LOG.md"
        );
        assert_eq!(owed.len(), 15);
        // Two runnable, not one. `leakage_audit` joined when its bar-boundary
        // step turned out to be **built** — `bar_boundary_check` has been a
        // templated tool since S-3.11 — and its other two owed stages turned out
        // to be *deferred by design* and conditional: the static scan has
        // nothing expressible to find under today's grammar, and the
        // contamination probe is a stated non-goal until a text feature exists.
        // A battery is withheld for a gap that applies to every call, not for
        // one that applies to inputs this call does not have.
        assert_eq!(
            ALL.iter().filter(|w| w.is_runnable()).count(),
            2,
            "the runnable count changed"
        );
        assert!(wf("selection_bias_verdict").is_runnable());
        assert!(wf("leakage_audit").is_runnable());
    }
}
