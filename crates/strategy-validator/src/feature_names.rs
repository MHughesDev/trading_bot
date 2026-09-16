//! Declared features must resolve (AGENT-011 §5 `unknown_feature`; Set S S-5.3).
//!
//! # The gap this closes
//!
//! Until this pass, a definition declaring `rsi14` validated cleanly. Measured,
//! not assumed: `validate()` returned `Ok` for a document whose `features` list
//! held `rsi14` and `emaa_10`, neither of which the engine resolves.
//!
//! What happened next is the expensive part. The name failed inside
//! `features::runtime::feature` during the run — after the caller had committed
//! to a backtest, which for an agent means **after a trial was spent**. The
//! cheapest possible error, a typo, was being charged at the most expensive
//! point in the system, and the failure when it came named a feature id rather
//! than the definition field that held it.
//!
//! # Why the registry, and not a name pattern
//!
//! A pattern over "word, underscore, digits" would accept `emaa_10` and reject
//! `obv`. The registry is the only thing that knows which names resolve, and it
//! is the same table `runtime::feature` reads and `list_features` serves — so a
//! name this pass accepts is a name the simulator resolves, by construction
//! rather than by agreement.
//!
//! # Why the suggestion is edit distance
//!
//! `rsi14` and `rsi_14` differ by one inserted character, which no prefix rule
//! catches and which is the likeliest typo under this naming scheme. The
//! threshold is deliberately tight: a wrong suggestion is worse than none,
//! because the caller spends a turn on it.

use domain::strategy_def::StrategyDefinition;

use crate::ValidationError;

/// Levenshtein distance. Only small values matter, so there is no early exit.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The obvious repair: a missing underscore before the parameter.
///
/// Tried against `resolve_feature` rather than against the advertised examples,
/// and that distinction is the whole reason this function exists. `rsi14` is a
/// caller reaching for `rsi_14`, which resolves — but the registry's advertised
/// example is `rsi_20`, so an edit-distance search over examples suggests the
/// wrong period or, at distance 3, nothing at all. Measured: the first version
/// of this pass offered no suggestion for `rsi14`.
///
/// So the repair is checked against the authority. A suggestion that resolves is
/// right by construction; one that merely looks close is a guess about what the
/// caller meant, and a wrong guess costs them a turn.
fn repair(name: &str) -> Option<String> {
    let split = name.rfind(|c: char| !c.is_ascii_digit())? + 1;
    if split == name.len() || name[..split].ends_with('_') {
        return None;
    }
    let candidate = format!("{}_{}", &name[..split], &name[split..]);
    features::resolve_feature(&candidate).is_some().then_some(candidate)
}

/// The closest resolvable name, when one is close enough to be worth naming.
///
/// Compared against each family's own advertised `example`, which is the form a
/// caller should write. Suggesting a bare family name would send them to a name
/// that also does not resolve.
fn nearest(name: &str) -> Option<String> {
    let mut best: Option<(usize, String)> = None;
    for f in features::feature_families() {
        let example = f.example();
        let d = distance(name, &example);
        if best.as_ref().map_or(true, |(bd, _)| d < *bd) {
            best = Some((d, example));
        }
    }
    // Two edits on a short name is already a different name. The cap grows a
    // little with length so `parkinson_vol_20` gets the same latitude as `obv`.
    let limit = 2 + name.len() / 8;
    best.filter(|(d, _)| *d <= limit).map(|(_, s)| s)
}

/// Check every feature named in an `inputs` declaration.
#[must_use]
pub fn check(def: &StrategyDefinition) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    for (i, input) in def.inputs.iter().enumerate() {
        for (j, name) in input.features.iter().enumerate() {
            if features::resolve_feature(name).is_some() {
                continue;
            }
            let suggestion = repair(name).or_else(|| nearest(name)).map_or_else(
                || String::from(" Call `list_features` to see what the engine computes."),
                |s| format!(" Did you mean `{s}`?"),
            );
            errors.push(ValidationError::new(
                "unknown_feature",
                format!("inputs[{i}].features[{j}]"),
                format!(
                    "`{name}` is not a feature the engine resolves.{suggestion} Caught here \
                     rather than during the run, which for an agent is after a trial was spent."
                ),
            ));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def_with(features: &str) -> StrategyDefinition {
        let json = format!(
            r#"{{
                "strategy_id": "fx_v1",
                "definition_version": "1.0",
                "asset_class": "crypto_spot_cex",
                "inputs": [
                    {{ "lane": "market.bars.1m", "instrument": "$bound_at_init" }},
                    {{ "lane": "features.technical", "instrument": "$bound_at_init",
                      "features": {features} }}
                ],
                "nodes": [
                    {{ "id": "n1", "type": "condition", "expr": "feature('ema_10') > 0.0" }},
                    {{ "id": "n2", "type": "signal", "when": "n1", "emit": "long" }}
                ],
                "actions": [
                    {{ "on_signal": "long", "type": "place_order",
                      "order": {{ "side": "buy", "size_mode": "fixed", "size": "1" }} }}
                ]
            }}"#
        );
        serde_json::from_str(&json).expect("the fixture parses")
    }

    /// The measured case: this document used to validate.
    #[test]
    fn a_misspelled_feature_is_refused_and_the_right_name_is_offered() {
        let errors = check(&def_with(r#"["rsi14"]"#));
        assert_eq!(errors.len(), 1, "one bad name, one error");
        let e = &errors[0];
        assert_eq!(e.code, "unknown_feature");
        assert_eq!(e.path, "inputs[1].features[0]");
        assert!(
            e.message.contains("rsi_14"),
            "`rsi14` differs from `rsi_14` by one inserted character, the likeliest typo under \
             this naming scheme, and the message does not offer it: {}",
            e.message
        );
        assert!(e.fix().is_some(), "and it carries its fix");
    }

    /// Names the engine really resolves pass, including ones with no parameter.
    #[test]
    fn resolvable_names_pass() {
        let errors = check(&def_with(r#"["ema_10", "rsi_14", "atr_14", "obv", "hour_sin"]"#));
        assert!(errors.is_empty(), "these all resolve: {errors:?}");
    }

    /// Every family's own advertised example survives this pass.
    ///
    /// The registry is one table, so a family whose example this rejected would
    /// mean `list_features` advertising a name the validator refuses — the model
    /// reads the catalogue, writes exactly what it says, and is told no.
    #[test]
    fn every_advertised_example_survives_this_pass() {
        let names: Vec<String> = features::feature_families()
            .iter()
            .map(features::Family::example)
            .collect();
        let json = serde_json::to_string(&names).expect("a list of names");
        let errors = check(&def_with(&json));
        assert!(
            errors.is_empty(),
            "`list_features` advertises names this pass refuses: {errors:?}"
        );
    }

    /// A name nothing is close to says so rather than inventing a suggestion.
    #[test]
    fn a_name_with_no_near_neighbour_offers_the_catalogue_instead() {
        let errors = check(&def_with(r#"["qqqqqqqqqqqqzzzz"]"#));
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].message.contains("list_features"),
            "a wrong suggestion costs the caller a turn, so with nothing close it should point \
             at the catalogue: {}",
            errors[0].message
        );
    }
}
