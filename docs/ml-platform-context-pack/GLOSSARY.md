# Glossary

Terms used with specific, non-obvious meaning in this pack.

**Anytime-valid confidence sequence** — a CI that stays valid under continuous peeking. Required because agents inspect results constantly; a fixed-n CI is invalid the moment someone looks early.

**ASHA** — Asynchronous Successive Halving. Parallel multi-fidelity early stopping. Naively applied to noisy validation metrics it promotes near-randomly; §11.2 lists the five mandatory hardening steps.

**Bitemporal** — a record carrying both when a fact was true (`event_time`) and when it became knowable (`knowledge_time`). Here extended to four timestamps (§0.1).

**CBPE** — Confidence-Based Performance Estimation. Estimates model performance without labels. **Assumes no concept drift**, so it cannot gate alpha-adjacent models (R-12).

**Censoring** — an ASHA-killed or preempted run has a *right-censored* outcome, not a bad one and not a missing row. Dropping censored rows is the largest bias source in experiment databases.

**Content addressing** — identifying an artifact by the hash of its bytes, so identical content deduplicates and different content cannot collide.

**CPCV** — Combinatorial Purged Cross-Validation. Measures model-class generalization, **not** what a trader could have earned — its paths train on data after some test blocks. Required *alongside* causal walk-forward, never alone.

**`delta_practical`** — the smallest difference in the objective that is worth acting on. Required on every campaign, no default. Most stopping and promotion questions are unanswerable without it.

**DSR** — Deflated Sharpe Ratio. Sharpe adjusted for the number of trials attempted. Exquisitely sensitive to `N`, which is why `N_eff` is platform-computed.

**EDGE** — Efficient Discrete Generalized Estimator for bid-ask spreads from OHLC. Replaces Roll and Corwin–Schultz (R-02).

**Embargo** — a gap after each test window during which training samples are excluded, sized from label horizon + feature lookback + knowledge lag + settlement.

**Exploration floor** — the ≥5% of trials drawn uniformly at random. Not waste: it is the unbiased sample that keeps the ledger a valid training set, and a surrogate-misspecification canary.

**Feature firewall** — the `info_class` whitelist controlling what may cross a tenant boundary, enforced by a build-blocking CI test.

**Filtered vs smoothed** — filtered regime probabilities use only past data; smoothed use the whole sample. A strategy seeing smoothed probabilities inflates Sharpe 0.78 → 1.74 (R-07).

**Fixation** — making a written fact unchangeable. Content addressing + hash chaining + Iceberg tags + bitemporal supersession.

**Information class (`info_class`)** — the tenancy classification of a feature or insight. Only `platform_physics`, `methodology` and `market_public` cross tenant boundaries.

**`knowledge_time`** — when a fact became queryable by a strategy. The PIT anchor. Cannot be reconstructed retroactively.

**Landmarker** — running cheap reference models on a task and using their scores as the task's representation. Superseded here by greedy submodular reference *portfolios* (R-03).

**LCB ranking** — ranking ledger-derived recommendations by a lower confidence bound rather than a point estimate, then shrinking toward the default policy. Non-bypassable (R-06).

**Learning debt (ρ_t)** — expected loss reduction from retraining now. Retrain iff `ρ_t > c_churn/(c_churn + c_wait)`. Replaces drift triggers (R-11).

**Meta-leakage** — using information from the future to make a *meta*-decision: e.g. retrieving neighbors using an asset embedding computed with data the decision could not have seen.

**MNAR** — Missing Not At Random. The Outcome Tensor's missingness is structural, not random: configs ran because something liked them.

**`N_eff`** — the effective number of independent trials, from correlation clustering over stored return series across the tenant's whole ledger, including gate failures. Platform-computed; never self-reported.

**PBO** — Probability of Backtest Overfitting. A property of a *selection procedure over a family*, meaningless for a single candidate.

**PIT** — point-in-time. A read that returns only what was knowable as of a given moment.

**PRM** — Process Reward Model. Scores intermediate steps rather than final outcomes. A 4B environment-grounded PRM beats much larger generic ones here.

**Propensity** — `p(chosen config | context, policy)`. Logged at every decision. Without it, the ledger is a biased sample and off-policy evaluation is formally impossible.

**Purging** — removing training samples whose labels overlap the test window. Purge on `t1` (label end); `t0` under-purges by the full horizon.

**Romano–Wolf stepdown** — multiple-testing correction using the actual correlation structure of your trials rather than a guessed count. The most defensible single gate.

**Trial Ledger** — the immutable, hash-chained, propensity-logged record of every trial. The platform's primary asset.

**TSFM** — Time-Series Foundation Model. Benchmark numbers are heavily contamination-affected; claims must be re-earned on your own data (R-03 note).

**WORM** — Write Once Read Many. Scoped here to ledger anchors and traded-model artifacts only; on analytical tables it would block compaction forever.
