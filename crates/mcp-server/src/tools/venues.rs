//! Venue rules, with a date on them (AGENT-006 §5.6; Set S task S-3.14).
//!
//! Exchange rules change, and a backtest built on a rule that changed is a
//! backtest of a market that no longer exists. So every fact here carries the
//! date it was verified and the tool computes its own staleness: past
//! [`STALE_AFTER_DAYS`] the answer says so, in the summary, where it cannot be
//! missed.
//!
//! # Answered locally, like `list_lanes`
//!
//! This is static reference data — no tenant, no secret, no row — so it is
//! served from this crate rather than through the API (the same exception
//! `list_lanes` already takes from ADR-0010). That also means updating a fact is
//! a code review and a deploy, which is the right friction for something whose
//! whole value is that somebody checked it.
//!
//! # What is here and what is deliberately not
//!
//! The **qualitative** rules are here, from the platform's own research brief
//! (`docs/research/quant-agent-toolbox-uncommon-tools.md`, Theme 5, verified
//! 2026-09-14): Coinbase's price-time priority and four self-trade-prevention
//! modes, Kraken's *decaying* order rate limit where a younger order costs more
//! to cancel, Alpaca's 2026 margin framework replacing the PDT rule.
//!
//! The **numeric** limits are not, and are flagged `unverified` rather than
//! filled in from memory. §5.6's acceptance — "a requote rate exceeding Kraken's
//! limit is refused" — needs a number this repository has no verified source
//! for, and a limit that is wrong in one direction refuses working strategies
//! while one wrong in the other lets a strategy get rate-limited live. The tool
//! names the constraint and says the threshold is unverified, which is more
//! useful than silence and more honest than a guess. Recorded as OQ-17.

use serde_json::{json, Value};

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

/// When a verified fact stops being treated as current (§5.6).
pub const STALE_AFTER_DAYS: i64 = 90;

/// The date the facts below were verified, from the research brief's own record.
const VERIFIED_AT: &str = "2026-09-14";

const SOURCE: &str = "docs/research/quant-agent-toolbox-uncommon-tools.md, Theme 5";

/// One venue's rules.
fn venue(id: &str) -> Option<Value> {
    let v = match id {
        "coinbase" => json!({
            "venue": "coinbase",
            "matching": {
                "priority": "price-time",
                "verified": true,
                "note": "a resting order's queue position is earned by arriving early at a \
                         price, so a strategy that re-quotes on every tick is repeatedly \
                         giving up its place. A backtest that fills at the bar close models \
                         none of that."
            },
            "self_trade_prevention": {
                "modes": 4,
                "verified": true,
                "note": "four modes exist and which one is configured changes what happens \
                         when your own orders cross. The mode NAMES are not verified here — a \
                         strategy running two sides on one account must read them from the \
                         venue's current documentation before assuming a behaviour."
            },
            "rate_limits": { "verified": false, "note": UNVERIFIED_LIMITS }
        }),
        "kraken" => json!({
            "venue": "kraken",
            "rate_limits": {
                "shape": "decaying counter; cancelling a YOUNGER order costs more",
                "verified": true,
                "penalises": "quote-and-yank",
                "note": "this is the rule most likely to invalidate a market-making backtest, \
                         because the naive model assumes a cancel is free. Under a decaying \
                         counter, a strategy that posts and pulls within seconds pays the \
                         highest penalty per action and throttles itself; the same strategy \
                         backtests perfectly.",
                "thresholds_verified": false,
                "thresholds_note": UNVERIFIED_LIMITS
            },
            "matching": { "verified": false, "note": "not verified in this table" }
        }),
        "alpaca" => json!({
            "venue": "alpaca",
            "margin": {
                "framework": "2026 real-time intraday margin",
                "replaces": "the pattern-day-trader rule",
                "minimum_equity_changed": "25,000 USD -> 2,000 USD",
                "verified": true,
                "note": "a strategy modelled against the PDT rule is modelled against a rule \
                         that no longer applies, in the permissive direction: the old rule \
                         blocked day trading below 25,000 USD and the new framework does not."
            },
            "rate_limits": { "verified": false, "note": UNVERIFIED_LIMITS }
        }),
        _ => return None,
    };
    Some(v)
}

const UNVERIFIED_LIMITS: &str =
    "the numeric limits are NOT verified in this table. Read them from the venue's current \
     documentation before sizing a requote rate against them — a limit guessed too high lets a \
     strategy be throttled live, and one guessed too low refuses a strategy that would work.";

/// Every venue this table knows about.
#[must_use]
pub fn known() -> Vec<&'static str> {
    vec!["coinbase", "kraken", "alpaca"]
}

/// `get_venue_constraints` — answered here, not over HTTP.
#[must_use]
pub fn get_venue_constraints(params: &Value) -> Value {
    let requested = params.get("venue").and_then(Value::as_str).map(str::trim);

    let venues: Vec<Value> = match requested {
        Some(v) if !v.is_empty() => match venue(v) {
            Some(found) => vec![found],
            None => {
                return json!({
                    "error": "unknown_venue",
                    "venue": v,
                    "known": known(),
                })
            }
        },
        _ => known().into_iter().filter_map(venue).collect(),
    };

    let age = age_days();
    let stale = age.is_none_or(|d| d > STALE_AFTER_DAYS);
    let staleness = match age {
        Some(d) if d <= STALE_AFTER_DAYS => {
            format!("verified {VERIFIED_AT}, {d} days ago — current")
        }
        Some(d) => format!(
            "STALE: verified {VERIFIED_AT}, {d} days ago, past the {STALE_AFTER_DAYS}-day bound. \
             Re-verify before relying on any of it; exchange rules change and a backtest built \
             on a changed rule is a backtest of a market that no longer exists."
        ),
        None => format!("verified {VERIFIED_AT}; the age could not be computed"),
    };

    json!({
        "summary_fields": {
            "n_venues": venues.len(),
            "verified_at": VERIFIED_AT,
            "staleness": staleness,
        },
        "detail": {
            "venues": venues,
            "verified_at": VERIFIED_AT,
            "age_days": age,
            "stale": stale,
            "stale_after_days": STALE_AFTER_DAYS,
            "staleness": staleness,
            "source": SOURCE,
            "coverage_note": "qualitative rules only. Every numeric limit in this table is \
                              flagged `verified: false` — this repository has no verified \
                              source for them (OQ-17), and a guessed limit is worse than a \
                              named gap because it reads exactly like a checked one.",
        },
        "ids": {},
    })
}

/// Days since [`VERIFIED_AT`], or `None` if the constant will not parse.
fn age_days() -> Option<i64> {
    let verified = chrono::NaiveDate::parse_from_str(VERIFIED_AT, "%Y-%m-%d").ok()?;
    let today = chrono::Utc::now().date_naive();
    Some((today - verified).num_days())
}

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        "get_venue_constraints",
        // `execution`, the canonical namespace for §5.6's realism tools — not
        // `market`, which is not in §2.1's list at all. `apply_funding_costs`
        // and `attribute_divergence` belong here too when they land.
        "execution",
        Risk::Read,
        "Venue rules that invalidate a backtest if ignored — matching priority, self-trade \
         prevention, rate limits, margin. Every fact carries the date it was verified and the \
         answer says when that is out of date.",
        json!({
            "type": "object",
            "properties": {
                "venue": {
                    "type": "string",
                    "enum": ["coinbase", "kraken", "alpaca"],
                    "description": "omit for all of them"
                }
            }
        }),
    )
    .internal()
    .summary(
        "{n_venues} venue(s); {staleness}",
        &[("n_venues", ""), ("staleness", "")],
    )
    .notes("get_venue_constraints")]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mechanism, which is the durable half: the answer knows how old it is.
    #[test]
    fn the_answer_carries_its_own_age() {
        let out = get_venue_constraints(&json!({}));
        let detail = &out["detail"];
        assert_eq!(detail["verified_at"], VERIFIED_AT);
        assert!(detail["age_days"].as_i64().is_some(), "the age is computable");
        assert_eq!(detail["stale_after_days"], STALE_AFTER_DAYS);
        // And the staleness reaches the *summary*, not only the detail — a
        // flat-tier model reads the summary and may never open the detail.
        assert!(out["summary_fields"]["staleness"].is_string());
    }

    /// Every numeric claim is flagged unverified, and the flag is machine
    /// readable rather than a sentence in prose.
    ///
    /// This is the assertion that stops a later edit quietly promoting a guessed
    /// number to a verified one: adding a limit means setting `verified: true`
    /// on it, which fails here until the table's coverage note is updated too.
    #[test]
    fn no_numeric_limit_claims_to_be_verified() {
        for id in known() {
            let v = venue(id).expect("a known venue resolves");
            if let Some(limits) = v.get("rate_limits") {
                let verified = limits
                    .get("thresholds_verified")
                    .or_else(|| limits.get("verified"))
                    .and_then(Value::as_bool);
                assert_eq!(
                    verified,
                    Some(false),
                    "{id} claims verified rate limits; this repository has no verified source \
                     for them (OQ-17)"
                );
            }
        }
    }

    #[test]
    fn an_unknown_venue_is_named_with_the_ones_that_exist() {
        let out = get_venue_constraints(&json!({ "venue": "binance" }));
        assert_eq!(out["error"], "unknown_venue");
        assert!(out["known"].as_array().is_some_and(|a| a.len() == 3));
    }
}
