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
                return crate::tool_error::refuse_extra(
                    "unknown_venue",
                    format!("venue `{v}` is not one this server holds constraints for."),
                    "set venue to one of `known`, or omit it to list them all",
                    json!({ "venue": v, "known": known() }),
                )
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
    .notes("get_venue_constraints"),
        ToolSpec::new(
            "apply_funding_costs",
            "execution",
            Risk::Read,
            "Prices a position path against the venue's realised funding: cashflow, annualised \
             carry and the share of settlements that were never reported. A gap over 10% \
             downgrades the answer and nothing is interpolated.",
            json!({
                "type": "object",
                "required": ["instrument", "venue", "start", "end", "positions"],
                "properties": {
                    "instrument": { "type": "string" },
                    "venue": { "type": "string" },
                    "start": { "type": "string", "description": "RFC3339 UTC" },
                    "end": { "type": "string", "description": "RFC3339 UTC" },
                    "positions": {
                        "type": "array",
                        "description": "signed notional (long positive) held from ts until the next entry; 0 is flat",
                        "items": {
                            "type": "object",
                            "required": ["ts", "notional_usd"],
                            "properties": { "ts": {"type": "string"}, "notional_usd": {"type": "number"} }
                        }
                    },
                    "source": { "type": "string", "enum": ["historical"], "default": "historical" },
                    "interval_hours": { "type": "number", "description": "settlement spacing; omit to infer it" }
                }
            }),
        )
        .internal()
        .summary(
            "funding cashflow {total_cashflow_usd}, annualised carry {annualized_carry_bps}, {data_gap_pct} of held settlements unreported; confidence {confidence}",
            &[("total_cashflow_usd", "USD"), ("annualized_carry_bps", "bps"), ("data_gap_pct", "%"), ("confidence", "")],
        ),
        ToolSpec::new(
            "attribute_divergence",
            "execution",
            Risk::Read,
            "Splits a deployed strategy's divergence from its backtest into bars where the signal \
             did not reproduce and bars where it did, with the slippage evidence beside the \
             second. Timing and data are reported unmeasured, not zero.",
            json!({
                "type": "object",
                "required": ["strategy_id"],
                "properties": {
                    "strategy_id": { "type": "string" },
                    "window_days": { "type": "integer", "minimum": 1, "maximum": 365, "default": 30 }
                }
            }),
        )
        .internal()
        .summary(
            "dominant divergence source: {dominant} over {bars}; paper Sharpe decay {decay_pct}; {verdict}",
            &[("dominant", ""), ("bars", "bars"), ("decay_pct", "%"), ("verdict", "")],
        ),
    ]
}

// ── S-3.14: what carrying the position cost, and where the paper run diverged ──

use crate::tools::market::{funding_grid, urlencode, FUNDING_GAP_DOWNGRADE_PCT};
use crate::ApiClient;

fn ts_secs(v: &Value) -> Option<f64> {
    chrono::DateTime::parse_from_rfc3339(v.as_str()?)
        .ok()
        .map(|d| d.timestamp() as f64)
}

/// Price a position path against a funding response. Pure.
///
/// A settlement the venue never reported is **not** costed and **not**
/// interpolated: it is counted, and when it fell while a position was held it
/// downgrades the answer. A missing interval read as free is the one error in
/// this domain that looks like good news.
pub(crate) fn apply_funding(
    resp: &Value,
    positions: &[(f64, f64)],
    interval_hours: Option<f64>,
) -> Value {
    let Some(rows) = resp.get("funding").and_then(Value::as_array) else {
        return crate::tool_error::refuse_extra(
            "funding_unmeasured",
            "no funding series exists for this instrument in this database, so the carry is \
             unknown; a cost model must skip the interval or refuse, never treat it as free.",
            "pick an instrument with a funding series (see `unmeasured`), or report the carry as unmeasured",
            json!({ "unmeasured": resp.get("manifest").and_then(|m| m.get("unmeasured")).cloned() }),
        );
    };
    let w = resp.get("manifest").and_then(|m| m.get("window"));
    let (Some(from), Some(to)) = (
        w.and_then(|w| w.get("from")).and_then(ts_secs),
        w.and_then(|w| w.get("to")).and_then(ts_secs),
    ) else {
        return crate::tool_error::refuse(
            "malformed_funding_response",
            "the funding response carried no manifest.window, so the grid cannot be placed.",
            "call again; if it repeats, report that the platform's funding history has no window",
        );
    };
    let grid = match funding_grid(rows, from, to, interval_hours) {
        Ok(g) => g,
        Err(e) => return e,
    };
    let pos_at = |t: f64| -> f64 {
        positions
            .iter()
            .rev()
            .find(|(pt, _)| *pt <= t)
            .map_or(0.0, |(_, n)| *n)
    };
    let ppy = 365.0 * 24.0 * 3600.0 / grid.cadence_s;
    let (mut total, mut priced, mut unrated, mut flat) = (0.0_f64, 0usize, 0usize, 0usize);
    let mut carry_terms: Vec<f64> = Vec::new();
    let mut cashflows: Vec<Value> = Vec::new();
    for p in &grid.points {
        let pos = pos_at(p.t);
        if pos == 0.0 {
            flat += 1;
            continue;
        }
        match p.rate {
            Some(r) => {
                // Positive funding: longs pay shorts.
                let cf = -pos * r;
                total += cf;
                priced += 1;
                carry_terms.push(-pos.signum() * r);
                cashflows
                    .push(json!({"t": p.t, "position_usd": pos, "rate": r, "cashflow_usd": cf}));
            }
            None => unrated += 1,
        }
    }
    let positioned_grid = grid
        .missing_times
        .iter()
        .filter(|t| pos_at(**t) != 0.0)
        .count();
    let held_total = priced + unrated + positioned_grid;
    let unpriced_pct = if held_total == 0 {
        0.0
    } else {
        (unrated + positioned_grid) as f64 / held_total as f64 * 100.0
    };
    let confidence = if unpriced_pct > FUNDING_GAP_DOWNGRADE_PCT {
        "low"
    } else {
        "ok"
    };
    let annualized = (!carry_terms.is_empty())
        .then(|| carry_terms.iter().sum::<f64>() / carry_terms.len() as f64 * ppy * 1e4);
    let tail: Vec<Value> = cashflows
        .iter()
        .rev()
        .take(24)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    json!({
        "summary_fields": {
            "total_cashflow_usd": (total * 100.0).round() / 100.0,
            "annualized_carry_bps": annualized.map_or(Value::Null, |a| json!((a * 10.0).round() / 10.0)),
            "data_gap_pct": (unpriced_pct * 10.0).round() / 10.0,
            "confidence": confidence,
        },
        "detail": {
            "total_cashflow_usd": total,
            "sign_convention": "positive cashflow is received; positive funding rate means longs pay",
            "annualized_carry_bps": annualized,
            "annualized_note": "mean of (-side x rate) over priced settlements while positioned, x settlements per year",
            "priced_settlements": priced,
            "settlements_while_flat": flat,
            "settlements_without_rate": unrated,
            "unpriced_settlements_while_positioned": unrated + positioned_grid,
            "data_gap_pct": unpriced_pct,
            "window_gap_pct": grid.gap_pct(),
            "confidence": confidence,
            "downgrade_note": if confidence == "low" {
                json!("more than 10% of the settlements that fell while a position was held were never reported. They are not costed and not interpolated, so the total understates the carry's magnitude.")
            } else { Value::Null },
            "cadence_hours": grid.cadence_s / 3600.0,
            "latest_cashflows": tail,
        },
        "n": priced,
        "ids": {},
    })
}

/// `apply_funding_costs` -- price a position path against realised funding.
pub async fn apply_funding_costs(api: &ApiClient, params: &Value) -> Value {
    for f in ["instrument", "venue", "start", "end"] {
        if params
            .get(f)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return crate::tool_error::missing_field(f);
        }
    }
    if params
        .get("source")
        .and_then(Value::as_str)
        .is_some_and(|s| s != "historical")
    {
        return crate::tool_error::refuse(
            "source_not_offered",
            "only `historical` is offered: `synthetic_from_basis` needs a basis series the platform does not serve, and a synthetic rate presented as funding is a guess.",
            "set source to \"historical\" or omit it",
        );
    }
    let Some(arr) = params
        .get("positions")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
    else {
        return crate::tool_error::missing_field_with(
            "positions",
            "add `positions`: [{ts, notional_usd}] -- signed notional (long positive) holding from ts until the next entry; use 0 for flat",
        );
    };
    if arr.len() > 5000 {
        return crate::tool_error::refuse_extra(
            "too_many_positions",
            format!("{} positions is above the 5000 limit.", arr.len()),
            "send at most 5000 entries; merge consecutive entries that hold the same notional",
            json!({ "max": 5000 }),
        );
    }
    let mut positions: Vec<(f64, f64)> = Vec::with_capacity(arr.len());
    for p in arr {
        let (Some(t), Some(n)) = (
            p.get("ts").and_then(ts_secs),
            p.get("notional_usd")
                .and_then(Value::as_f64)
                .filter(|x| x.is_finite()),
        ) else {
            return crate::tool_error::refuse_extra(
                "invalid_position",
                "a positions entry has no RFC3339 `ts` or no finite `notional_usd`.",
                "send each entry as {ts: RFC3339, notional_usd: number}",
                json!({ "got": p }),
            );
        };
        positions.push((t, n));
    }
    positions.sort_by(|a, b| a.0.total_cmp(&b.0));
    let q = |k: &str| params.get(k).and_then(Value::as_str).unwrap_or("");
    let path = format!(
        "/api/data/funding?venue={}&instrument={}&from={}&to={}",
        urlencode(q("venue")),
        urlencode(q("instrument")),
        urlencode(q("start")),
        urlencode(q("end")),
    );
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    apply_funding(
        &resp,
        &positions,
        params.get("interval_hours").and_then(Value::as_f64),
    )
}

/// `attribute_divergence` -- where a deployed strategy's return left its backtest's.
pub async fn attribute_divergence(api: &ApiClient, params: &Value) -> Value {
    let Some(sid) = params
        .get("strategy_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return crate::tool_error::missing_field("strategy_id");
    };
    let mut path = format!("/api/agent/divergence?strategy_id={}", urlencode(sid));
    if let Some(d) = params.get("window_days").and_then(Value::as_i64) {
        path.push_str(&format!("&window_days={d}"));
    }
    let resp = match api.get(&path).await {
        Ok(v) => v,
        Err(e) => return e.to_tool_error(),
    };
    let r = resp.get("report").cloned().unwrap_or(Value::Null);
    json!({
        "summary_fields": {
            "dominant": r.get("dominant").cloned().unwrap_or(Value::Null),
            "bars": r.get("bars").cloned().unwrap_or(Value::Null),
            "decay_pct": r.get("decay_pct").cloned().unwrap_or(Value::Null),
            "verdict": r.get("verdict").cloned().unwrap_or(Value::Null),
        },
        "detail": resp,
        "n": r.get("bars").cloned().unwrap_or(Value::Null),
        "ids": {},
    })
}

#[cfg(test)]
mod funding_tests {
    use super::*;

    fn resp(n_hours: i64, drop_every: i64) -> Value {
        let rows: Vec<Value> = (0..n_hours)
            .filter(|h| drop_every == 0 || h % drop_every != 1)
            .map(|h| json!({"event_time": format!("2024-01-01T{:02}:00:00Z", h), "relative_rate": "0.0001"}))
            .collect();
        json!({"manifest": {"window": {"from": "2024-01-01T00:00:00Z", "to": "2024-01-02T00:00:00Z"}}, "funding": rows})
    }

    const T0: f64 = 1_704_067_200.0; // 2024-01-01T00:00:00Z

    /// A long pays positive funding, a short receives it.
    #[test]
    fn a_long_pays_and_a_short_receives() {
        let long = apply_funding(&resp(24, 0), &[(T0, 10_000.0)], Some(1.0));
        let short = apply_funding(&resp(24, 0), &[(T0, -10_000.0)], Some(1.0));
        let l = long["detail"]["total_cashflow_usd"].as_f64().unwrap();
        let s = short["detail"]["total_cashflow_usd"].as_f64().unwrap();
        assert!((l + 24.0).abs() < 1e-9, "{l}");
        assert!((s - 24.0).abs() < 1e-9, "{s}");
    }

    /// Settlements while flat cost nothing and are counted as flat.
    #[test]
    fn flat_settlements_are_not_charged() {
        let out = apply_funding(
            &resp(24, 0),
            &[(T0, 0.0), (T0 + 12.0 * 3600.0, 10_000.0)],
            Some(1.0),
        );
        assert_eq!(out["detail"]["settlements_while_flat"], 12);
        assert_eq!(out["detail"]["priced_settlements"], 12);
    }

    /// The acceptance: a gap over 10% downgrades and is never interpolated.
    #[test]
    fn a_gap_while_positioned_downgrades_and_is_not_filled() {
        let out = apply_funding(&resp(24, 4), &[(T0, 10_000.0)], Some(1.0));
        assert_eq!(out["detail"]["confidence"], "low");
        assert_eq!(out["detail"]["priced_settlements"], 18);
        assert_eq!(out["detail"]["unpriced_settlements_while_positioned"], 6);
    }

    #[test]
    fn an_absent_series_is_refused_not_costed_at_zero() {
        let out = apply_funding(
            &json!({"manifest": {}, "funding": null}),
            &[(T0, 1.0)],
            Some(1.0),
        );
        assert_eq!(out["error"]["code"], "funding_unmeasured");
    }
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
        assert!(
            detail["age_days"].as_i64().is_some(),
            "the age is computable"
        );
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
        assert_eq!(out["error"]["code"], "unknown_venue");
        assert!(out["known"].as_array().is_some_and(|a| a.len() == 3));
    }
}
