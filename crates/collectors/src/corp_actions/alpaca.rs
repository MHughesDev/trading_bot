//! Alpaca corporate actions → `dataplane.corporate_action` (Set P, P-3.3).
//!
//! Alpaca's `/v1/corporate-actions` returns splits, dividends, mergers and name
//! changes, each with the dates the market cares about. This module turns them
//! into [`CorporateAction`] facts whose `knowledge_time` is the **declaration**
//! date, not the ex-date — a 4-for-1 split announced in July and effective in
//! August was tradeable information in July, and a store that recorded only the
//! ex-date would hand a backtest the factor a month late.
//!
//! The direction of the factor is the part that is easy to get backwards.
//! `dataplane::corporate::factors_at` multiplies prices observed **before** the
//! effective time, so a 4-for-1 split carries `price_factor = 1/4` and
//! `volume_factor = 4`: a pre-split $400 print becomes $100 in post-split terms.
//! [`split_factors`] is the one place that conversion happens.
//!
//! ## Free-plan depth — not verified, and claimed as unverified
//!
//! Corporate actions ride the same Alpaca market-data entitlement as news, so
//! they share [`VenueLimit::alpaca_market_data`]'s self-imposed budget. Alpaca's
//! documentation publishes **no corporate-actions archive start date** and no
//! per-plan request ceiling on the pages checked on 2026-09-22, so this module
//! claims neither. Splits and dividends are separately recoverable from EDGAR
//! filings and exchange notices, so this is not a forward-only series in the
//! FM-11 sense, but the *depth actually available from Alpaca* is unmeasured
//! until a census job asks.

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use dataplane::corporate::{ActionType, CorporateAction};
use dataplane::identity::InstrumentKey;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::ratelimit::{RateLimiter, VenueLimit};

/// The `dataplane.source.name` this collector writes under. The row itself is a
/// migration this track does not write — see the report.
pub const SOURCE_NAME: &str = "alpaca_corp_actions";

// No `SOURCE_ID` here on purpose. `dataplane.corporate_action` (migration 0046,
// line 159) has no `source_id` column, so a corporate action the platform stores
// records *that* a split happened but not *who said so*. Declaring a numeric id
// this module cannot write would be a const that looks like plumbing and is not.
// The gap is named in the report instead.

#[derive(Debug, thiserror::Error)]
pub enum CorpActionError {
    #[error("malformed response: {0}")]
    Parse(String),
    #[error("split for {symbol} on {ex_date} has a non-positive ratio ({old} -> {new})")]
    NonPositiveRatio {
        symbol: String,
        ex_date: NaiveDate,
        old: Decimal,
        new: Decimal,
    },
    #[error(
        "action for {symbol} on {ex_date} was declared {declared}, after its effective date — \
         an action cannot take effect before it is announced"
    )]
    DeclaredAfterEffective {
        symbol: String,
        ex_date: NaiveDate,
        declared: NaiveDate,
    },
}

// ── Alpaca response shapes ───────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct CorporateActionsResponse {
    #[serde(default)]
    pub corporate_actions: CorporateActions,
    #[serde(default)]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CorporateActions {
    #[serde(default)]
    pub forward_splits: Vec<SplitEvent>,
    #[serde(default)]
    pub reverse_splits: Vec<SplitEvent>,
    #[serde(default)]
    pub cash_dividends: Vec<CashDividendEvent>,
}

#[derive(Debug, Deserialize)]
pub struct SplitEvent {
    pub symbol: String,
    pub ex_date: NaiveDate,
    #[serde(default)]
    pub process_date: Option<NaiveDate>,
    /// The share count before the split (the `1` in "4-for-1" is `new_rate`).
    pub old_rate: Decimal,
    /// The share count after.
    pub new_rate: Decimal,
    /// When the issuer declared it. Absent on some historical rows.
    #[serde(default)]
    pub declaration_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
pub struct CashDividendEvent {
    pub symbol: String,
    pub ex_date: NaiveDate,
    pub rate: Decimal,
    #[serde(default)]
    pub declaration_date: Option<NaiveDate>,
    #[serde(default)]
    pub payable_date: Option<NaiveDate>,
    #[serde(default)]
    pub special: Option<bool>,
}

/// The price and volume factors a split implies for prices observed *before* it.
///
/// A 4-for-1 forward split has `old_rate = 1`, `new_rate = 4`: one old share
/// becomes four. Prices before it are divided by 4 and volumes multiplied by 4.
///
/// # Errors
/// A non-positive rate on either side. A zero would make every earlier price
/// zero, silently, across the whole history.
pub fn split_factors(
    symbol: &str,
    ex_date: NaiveDate,
    old_rate: Decimal,
    new_rate: Decimal,
) -> Result<(Decimal, Decimal), CorpActionError> {
    if old_rate <= Decimal::ZERO || new_rate <= Decimal::ZERO {
        return Err(CorpActionError::NonPositiveRatio {
            symbol: symbol.to_owned(),
            ex_date,
            old: old_rate,
            new: new_rate,
        });
    }
    let ratio = new_rate / old_rate;
    Ok((Decimal::ONE / ratio, ratio))
}

/// Midnight UTC of `date` — used where Alpaca gives a date and no time.
fn at_start_of(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight exists"))
}

/// Converts one split event into a [`CorporateAction`].
///
/// `knowledge_time` is the declaration date when Alpaca supplies one, and the
/// ex-date otherwise — never earlier than the declaration, so an action can never
/// appear before it was announced.
///
/// # Errors
/// A non-positive ratio, or a declaration dated after the ex-date.
pub fn to_corporate_action(
    event: &SplitEvent,
    instrument: InstrumentKey,
) -> Result<CorporateAction, CorpActionError> {
    let (price_factor, volume_factor) =
        split_factors(&event.symbol, event.ex_date, event.old_rate, event.new_rate)?;
    let effective_time = at_start_of(event.ex_date);
    let declared = event.declaration_date.unwrap_or(event.ex_date);
    if declared > event.ex_date {
        return Err(CorpActionError::DeclaredAfterEffective {
            symbol: event.symbol.clone(),
            ex_date: event.ex_date,
            declared,
        });
    }
    let announcement_time = at_start_of(declared);
    Ok(CorporateAction {
        instrument_id: instrument,
        action_type: ActionType::Split,
        announcement_time,
        ex_date: event.ex_date,
        effective_time,
        price_factor: Some(price_factor),
        volume_factor: Some(volume_factor),
        cash_amount: None,
        currency: None,
        knowledge_time: announcement_time,
        revision_seq: 0,
    })
}

/// Converts one cash dividend into a [`CorporateAction`].
///
/// # Errors
/// A declaration dated after the ex-date.
pub fn dividend_to_corporate_action(
    event: &CashDividendEvent,
    instrument: InstrumentKey,
) -> Result<CorporateAction, CorpActionError> {
    let declared = event.declaration_date.unwrap_or(event.ex_date);
    if declared > event.ex_date {
        return Err(CorpActionError::DeclaredAfterEffective {
            symbol: event.symbol.clone(),
            ex_date: event.ex_date,
            declared,
        });
    }
    let announcement_time = at_start_of(declared);
    Ok(CorporateAction {
        instrument_id: instrument,
        action_type: ActionType::Dividend,
        announcement_time,
        ex_date: event.ex_date,
        effective_time: at_start_of(event.ex_date),
        // A dividend's price factor depends on the close before the ex-date, which
        // this collector does not hold. Left None: the read path composes it from
        // the cash amount and the unadjusted close, so a guess here would become
        // an un-auditable number in every adjusted series.
        price_factor: None,
        volume_factor: None,
        cash_amount: Some(event.rate),
        currency: Some("USD".to_owned()),
        knowledge_time: announcement_time,
        revision_seq: 0,
    })
}

/// Parses a corporate-actions page.
///
/// # Errors
/// A malformed payload.
pub fn parse_corporate_actions(body: &str) -> Result<CorporateActionsResponse, CorpActionError> {
    // Decoding a body fetched from an external REST API — see the note on the
    // `lint-no-json-hotpath` exception in `collectors::macro::fred`.
    serde_json::from_str::<CorporateActionsResponse>(body)
        .map_err(|e| CorpActionError::Parse(e.to_string()))
}

/// The budget every caller of this module must spend against.
#[must_use]
pub fn limiter() -> RateLimiter {
    RateLimiter::new(VenueLimit::alpaca_market_data())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dataplane::corporate::{factors_at, AdjustmentPolicy};
    use rust_decimal_macros::dec;

    const PAGE: &str = r#"{
      "corporate_actions": {
        "forward_splits": [
          {"symbol":"AAPL","ex_date":"2020-08-31","process_date":"2020-08-31",
           "old_rate":1,"new_rate":4,"declaration_date":"2020-07-30"}
        ],
        "cash_dividends": [
          {"symbol":"AAPL","ex_date":"2020-08-07","rate":0.82,
           "declaration_date":"2020-07-30","payable_date":"2020-08-13"}
        ]
      }
    }"#;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid instant")
            .with_timezone(&Utc)
    }

    #[test]
    fn a_page_parses() {
        let page = parse_corporate_actions(PAGE).expect("fixture parses");
        assert_eq!(page.corporate_actions.forward_splits.len(), 1);
        assert_eq!(page.corporate_actions.cash_dividends.len(), 1);
    }

    #[test]
    fn a_four_for_one_split_divides_earlier_prices_by_four() {
        let (price, volume) =
            split_factors("AAPL", NaiveDate::from_ymd_opt(2020, 8, 31).unwrap(), dec!(1), dec!(4))
                .expect("a positive ratio");
        assert_eq!(price, dec!(0.25));
        assert_eq!(volume, dec!(4));
    }

    #[test]
    fn a_reverse_split_multiplies_earlier_prices() {
        let (price, volume) =
            split_factors("XYZ", NaiveDate::from_ymd_opt(2023, 1, 5).unwrap(), dec!(10), dec!(1))
                .expect("a positive ratio");
        assert_eq!(price, dec!(10));
        assert_eq!(volume, dec!(0.1));
    }

    /// P-3.3 acceptance: a stored price is unadjusted, and a read applies the
    /// factor as of the read's `as_of`. The stored number never changes.
    #[test]
    fn the_stored_price_is_unadjusted_and_the_read_applies_the_factor() {
        let page = parse_corporate_actions(PAGE).expect("fixture parses");
        let action = to_corporate_action(&page.corporate_actions.forward_splits[0], InstrumentKey(7))
            .expect("well-formed split");
        let actions = vec![action];

        // What printed on 2020-08-03, stored and never touched.
        let stored_close = dec!(435.75);
        let bar_time = t("2020-08-03T20:00:00Z");

        // Before the split was declared, nothing adjusts it.
        let before = factors_at(
            &actions,
            InstrumentKey(7),
            bar_time,
            t("2020-07-01T00:00:00Z"),
            AdjustmentPolicy::SplitsOnly,
        );
        assert_eq!(stored_close * before.price, dec!(435.75));

        // After the declaration, the same stored number reads as post-split.
        let after = factors_at(
            &actions,
            InstrumentKey(7),
            bar_time,
            t("2020-09-01T00:00:00Z"),
            AdjustmentPolicy::SplitsOnly,
        );
        assert_eq!(stored_close * after.price, dec!(108.9375));
    }

    /// The adversarial half: the split must be invisible to a read whose `as_of`
    /// precedes the declaration. A store that adjusted retroactively would hand a
    /// July backtest a factor announced on 30 July.
    #[test]
    fn a_split_is_invisible_before_it_was_declared() {
        let page = parse_corporate_actions(PAGE).expect("fixture parses");
        let action = to_corporate_action(&page.corporate_actions.forward_splits[0], InstrumentKey(7))
            .expect("well-formed split");
        assert_eq!(action.knowledge_time, t("2020-07-30T00:00:00Z"));
        let actions = vec![action];
        let factors = factors_at(
            &actions,
            InstrumentKey(7),
            t("2020-07-15T20:00:00Z"),
            t("2020-07-29T23:59:59Z"),
            AdjustmentPolicy::SplitsOnly,
        );
        assert_eq!(factors.price, dec!(1));
    }

    #[test]
    fn a_dividend_carries_its_cash_amount_and_no_guessed_price_factor() {
        let page = parse_corporate_actions(PAGE).expect("fixture parses");
        let action =
            dividend_to_corporate_action(&page.corporate_actions.cash_dividends[0], InstrumentKey(7))
                .expect("well-formed dividend");
        assert_eq!(action.action_type, ActionType::Dividend);
        assert_eq!(action.cash_amount, Some(dec!(0.82)));
        assert_eq!(action.price_factor, None);
    }

    #[test]
    fn a_zero_ratio_is_refused() {
        let ex = NaiveDate::from_ymd_opt(2020, 8, 31).unwrap();
        assert!(matches!(
            split_factors("AAPL", ex, dec!(1), dec!(0)),
            Err(CorpActionError::NonPositiveRatio { .. })
        ));
        assert!(matches!(
            split_factors("AAPL", ex, dec!(0), dec!(4)),
            Err(CorpActionError::NonPositiveRatio { .. })
        ));
    }

    /// An action declared after it took effect is impossible and is refused
    /// rather than stored with a knowledge time that predates its own evidence.
    #[test]
    fn an_action_declared_after_its_ex_date_is_refused() {
        let event = SplitEvent {
            symbol: "AAPL".into(),
            ex_date: NaiveDate::from_ymd_opt(2020, 8, 31).unwrap(),
            process_date: None,
            old_rate: dec!(1),
            new_rate: dec!(4),
            declaration_date: NaiveDate::from_ymd_opt(2020, 9, 15),
        };
        assert!(matches!(
            to_corporate_action(&event, InstrumentKey(7)),
            Err(CorpActionError::DeclaredAfterEffective { .. })
        ));
    }

    /// With no declaration date, the knowledge time falls back to the ex-date —
    /// late rather than early. Being late costs signal; being early is lookahead.
    #[test]
    fn a_missing_declaration_date_falls_back_to_the_ex_date() {
        let event = SplitEvent {
            symbol: "AAPL".into(),
            ex_date: NaiveDate::from_ymd_opt(2020, 8, 31).unwrap(),
            process_date: None,
            old_rate: dec!(1),
            new_rate: dec!(4),
            declaration_date: None,
        };
        let action = to_corporate_action(&event, InstrumentKey(7)).expect("well-formed split");
        assert_eq!(action.knowledge_time, t("2020-08-31T00:00:00Z"));
    }
}
