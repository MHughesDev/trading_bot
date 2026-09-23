//! What a number is denominated in (EXTERNAL-REVIEW-2026-09 §3.6, ADR-P6-07).
//!
//! The platform used to answer "is this USD?" with
//! `matches!(currency, "USD" | "USDC" | "USDT")` and sum the matches. That is
//! three different assets and one of them is an unsecured claim on a private
//! issuer. During the March 2023 USDC depeg, USDC pairs moved about ten percent
//! on nothing; a strategy long USDC-quoted assets and short USD-quoted ones
//! would have shown that as alpha, and every backtest across the pair would have
//! kept showing it.
//!
//! So spot crypto returns quoted in a stablecoin carry issuer credit risk that
//! no price series models, and the error is silent in exactly the place it
//! matters: a cross-pair "discovery" contains an unmeasured fraction of
//! stablecoin drift.
//!
//! ## The rule
//!
//! Parity is an **assumption**, not a fact, so it has to be stated. There is no
//! function here that turns a stablecoin amount into dollars without returning
//! the assumption it made alongside the number. [`Converted`] carries both, and
//! it has no accessor that hands over the amount while dropping the assumption
//! — which is the only way to stop a rollup three layers up from rendering
//! `$1,204,551` with nothing beside it.
//!
//! This is not a claim that parity is the wrong policy. For a dashboard
//! refreshing every five seconds it is almost always the right one. The claim is
//! that "we assumed parity" and "these were dollars" must not be the same
//! sentence.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The asset a number is denominated in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Numeraire {
    /// Actual US dollars held at a venue. The only variant with no peg risk.
    Usd,
    /// A dollar-pegged token. The peg is a promise by an issuer, and promises
    /// have been broken.
    Stablecoin {
        /// Ticker as the venue quotes it — `"USDC"`, `"USDT"`, `"DAI"`.
        symbol: String,
    },
    /// Anything else: ETH-denominated NFT accounts, JPY, a BTC-quoted pair.
    Other { symbol: String },
}

impl Numeraire {
    /// Classify a venue's currency code.
    ///
    /// The stablecoin list is explicit rather than a heuristic on the string.
    /// A `symbol.starts_with("USD")` test would classify `USDe`, `USDD` and a
    /// token called `USDScam` identically, and being wrong here is silent.
    #[must_use]
    pub fn of(currency: &str) -> Self {
        match currency.to_ascii_uppercase().as_str() {
            "USD" => Self::Usd,
            s @ ("USDC" | "USDT" | "DAI" | "TUSD" | "USDP" | "FDUSD" | "PYUSD" | "BUSD") => {
                Self::Stablecoin {
                    symbol: s.to_string(),
                }
            }
            s => Self::Other {
                symbol: s.to_string(),
            },
        }
    }

    /// The ticker.
    #[must_use]
    pub fn symbol(&self) -> &str {
        match self {
            Self::Usd => "USD",
            Self::Stablecoin { symbol } | Self::Other { symbol } => symbol,
        }
    }

    /// Whether this carries issuer credit and depeg risk.
    #[must_use]
    pub const fn has_peg_risk(&self) -> bool {
        matches!(self, Self::Stablecoin { .. })
    }

    /// Whether amounts in this numeraire can be summed with dollars under *some*
    /// policy. `Other` cannot: adding ETH to USD without a rate is how a
    /// dashboard starts lying, and no policy here supplies a rate.
    #[must_use]
    pub const fn is_dollar_denominated(&self) -> bool {
        matches!(self, Self::Usd | Self::Stablecoin { .. })
    }
}

/// How to treat a dollar-pegged token when totalling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PegPolicy {
    /// One token is one dollar. Cheap, right almost always, and wrong at
    /// precisely the moments anyone cares about the answer. Permitted — and
    /// recorded every time it is used.
    AssumeParity,
    /// Convert at an observed stablecoin/USD rate supplied by the caller.
    Observed,
    /// Refuse to sum across numeraires at all. What an evaluation path should
    /// use: a research result denominated in two assets is two results.
    Refuse,
}

/// An amount converted to dollars, inseparable from the assumption that made it
/// possible.
///
/// There is deliberately no `into_amount()` and no `Deref<Target = Decimal>`:
/// every route to the number goes past the assumption. That is the entire
/// mechanism.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Converted {
    amount_usd: Decimal,
    assumptions: Vec<PegAssumption>,
    /// Numeraires that could not be converted and were left out of the total.
    excluded: Vec<String>,
}

/// One recorded act of treating a non-dollar as a dollar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PegAssumption {
    /// Which token.
    pub symbol: String,
    /// The rate applied. `1` under [`PegPolicy::AssumeParity`].
    pub rate: Decimal,
    /// How the rate was arrived at.
    pub policy: PegPolicy,
    /// Gross amount in that token, before conversion. Kept because the size of
    /// the assumption is the size of the exposure: assuming parity on $12 and on
    /// $12M are different risks wearing the same word.
    pub gross: Decimal,
}

impl Converted {
    /// The dollar total **and** what was assumed to get it. Destructuring is the
    /// only way in, so a caller cannot take the number and leave the caveat.
    #[must_use]
    pub fn parts(&self) -> (Decimal, &[PegAssumption], &[String]) {
        (self.amount_usd, &self.assumptions, &self.excluded)
    }

    /// Whether any part of this total rests on a peg holding.
    #[must_use]
    pub fn rests_on_a_peg(&self) -> bool {
        !self.assumptions.is_empty()
    }

    /// Total exposure to peg failure, in dollars. The number a risk view should
    /// show next to the equity figure.
    #[must_use]
    pub fn peg_exposure_usd(&self) -> Decimal {
        self.assumptions.iter().map(|a| a.gross * a.rate).sum()
    }

    /// Numeraires left out of the total.
    #[must_use]
    pub fn excluded(&self) -> &[String] {
        &self.excluded
    }
}

/// Sum `(numeraire, amount)` pairs into dollars under `policy`.
///
/// `rate_for` supplies the observed rate under [`PegPolicy::Observed`]; it is
/// never called under the other two. A stablecoin with no observed rate is
/// **excluded**, not defaulted to parity — falling back to the assumption the
/// caller explicitly declined is worse than returning a smaller number.
///
/// # Errors
/// [`NumeraireError::CrossNumeraireRefused`] when `policy` is
/// [`PegPolicy::Refuse`] and any non-USD dollar-denominated amount is present.
pub fn total_usd(
    amounts: &[(Numeraire, Decimal)],
    policy: PegPolicy,
    mut rate_for: impl FnMut(&str) -> Option<Decimal>,
) -> Result<Converted, NumeraireError> {
    let mut amount_usd = Decimal::ZERO;
    let mut assumptions: Vec<PegAssumption> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();

    for (n, amt) in amounts {
        match n {
            Numeraire::Usd => amount_usd += *amt,
            Numeraire::Stablecoin { symbol } => match policy {
                PegPolicy::Refuse => {
                    return Err(NumeraireError::CrossNumeraireRefused {
                        symbol: symbol.clone(),
                    })
                }
                PegPolicy::AssumeParity => {
                    amount_usd += *amt;
                    record(&mut assumptions, symbol, Decimal::ONE, policy, *amt);
                }
                PegPolicy::Observed => match rate_for(symbol) {
                    Some(rate) => {
                        amount_usd += *amt * rate;
                        record(&mut assumptions, symbol, rate, policy, *amt);
                    }
                    None => push_unique(&mut excluded, symbol),
                },
            },
            Numeraire::Other { symbol } => push_unique(&mut excluded, symbol),
        }
    }

    Ok(Converted {
        amount_usd,
        assumptions,
        excluded,
    })
}

fn record(
    into: &mut Vec<PegAssumption>,
    symbol: &str,
    rate: Decimal,
    policy: PegPolicy,
    gross: Decimal,
) {
    // One assumption per symbol, with the gross accumulated: five USDT accounts
    // are one exposure to Tether, not five.
    if let Some(existing) = into.iter_mut().find(|a| a.symbol == symbol) {
        existing.gross += gross;
    } else {
        into.push(PegAssumption {
            symbol: symbol.to_string(),
            rate,
            policy,
            gross,
        });
    }
}

fn push_unique(into: &mut Vec<String>, s: &str) {
    if !into.iter().any(|e| e == s) {
        into.push(s.to_string());
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum NumeraireError {
    #[error(
        "refusing to total {symbol} into a USD figure: this path declared PegPolicy::Refuse, and \
         a result denominated in two assets is two results"
    )]
    CrossNumeraireRefused { symbol: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn no_rates(_: &str) -> Option<Decimal> {
        None
    }

    // ── classification ───────────────────────────────────────────────────────

    #[test]
    fn dollars_and_dollar_tokens_are_different_things() {
        assert_eq!(Numeraire::of("USD"), Numeraire::Usd);
        assert!(!Numeraire::of("USD").has_peg_risk());
        assert!(Numeraire::of("USDT").has_peg_risk());
        assert!(Numeraire::of("USDC").has_peg_risk());
        assert_ne!(Numeraire::of("USD"), Numeraire::of("USDT"));
    }

    #[test]
    fn classification_is_case_insensitive() {
        assert_eq!(Numeraire::of("usdt"), Numeraire::of("USDT"));
        assert_eq!(Numeraire::of("usd"), Numeraire::Usd);
    }

    /// A prefix test would classify anything starting with "USD" as a
    /// stablecoin. The list is explicit for exactly that reason.
    #[test]
    fn an_unknown_token_is_not_a_stablecoin_just_because_it_starts_with_usd() {
        let n = Numeraire::of("USDSCAM");
        assert!(!n.has_peg_risk());
        assert!(!n.is_dollar_denominated());
        assert_eq!(
            n,
            Numeraire::Other {
                symbol: "USDSCAM".into()
            }
        );
    }

    #[test]
    fn eth_is_not_dollar_denominated_under_any_policy() {
        assert!(!Numeraire::of("ETH").is_dollar_denominated());
    }

    // ── the assumption cannot be dropped ─────────────────────────────────────

    /// The defect this module exists for: a USD equity figure that silently
    /// contains USDT. It is still permitted — and it now says so.
    #[test]
    fn a_parity_total_reports_what_it_assumed() {
        let c = total_usd(
            &[
                (Numeraire::Usd, dec!(1000)),
                (Numeraire::of("USDT"), dec!(500)),
                (Numeraire::of("USDC"), dec!(250)),
            ],
            PegPolicy::AssumeParity,
            no_rates,
        )
        .unwrap();

        let (amount, assumptions, excluded) = c.parts();
        assert_eq!(amount, dec!(1750));
        assert!(c.rests_on_a_peg());
        assert_eq!(assumptions.len(), 2);
        assert!(excluded.is_empty());
        assert_eq!(c.peg_exposure_usd(), dec!(750));
    }

    #[test]
    fn a_pure_dollar_total_rests_on_no_peg() {
        let c = total_usd(
            &[(Numeraire::Usd, dec!(42))],
            PegPolicy::AssumeParity,
            no_rates,
        )
        .unwrap();
        assert!(!c.rests_on_a_peg());
        assert_eq!(c.peg_exposure_usd(), Decimal::ZERO);
    }

    /// The size of the assumption is the size of the exposure. Five accounts in
    /// the same token are one claim on one issuer.
    #[test]
    fn repeated_holdings_of_one_token_are_one_exposure() {
        let c = total_usd(
            &[
                (Numeraire::of("USDT"), dec!(100)),
                (Numeraire::of("USDT"), dec!(200)),
                (Numeraire::of("USDT"), dec!(300)),
            ],
            PegPolicy::AssumeParity,
            no_rates,
        )
        .unwrap();
        let (_, assumptions, _) = c.parts();
        assert_eq!(assumptions.len(), 1, "one issuer, one exposure");
        assert_eq!(assumptions[0].gross, dec!(600));
        assert_eq!(c.peg_exposure_usd(), dec!(600));
    }

    // ── observed rates ───────────────────────────────────────────────────────

    /// The March 2023 case: USDC at 0.88. Under parity this reads as a total of
    /// 2000 and a strategy long USDC-quoted assets shows a 12% edge from nothing.
    #[test]
    fn an_observed_depeg_changes_the_total_and_parity_hides_it() {
        let holdings = [
            (Numeraire::Usd, dec!(1000)),
            (Numeraire::of("USDC"), dec!(1000)),
        ];

        let parity = total_usd(&holdings, PegPolicy::AssumeParity, no_rates).unwrap();
        assert_eq!(parity.parts().0, dec!(2000));

        let observed = total_usd(&holdings, PegPolicy::Observed, |s| {
            (s == "USDC").then(|| dec!(0.88))
        })
        .unwrap();
        assert_eq!(observed.parts().0, dec!(1880));
        assert_eq!(observed.parts().1[0].rate, dec!(0.88));
        assert_eq!(observed.parts().1[0].policy, PegPolicy::Observed);
    }

    /// Falling back to parity when the caller explicitly asked for an observed
    /// rate would silently reinstate the assumption they declined.
    #[test]
    fn a_missing_observed_rate_excludes_rather_than_assuming_parity() {
        let c = total_usd(
            &[
                (Numeraire::Usd, dec!(100)),
                (Numeraire::of("USDT"), dec!(900)),
            ],
            PegPolicy::Observed,
            no_rates,
        )
        .unwrap();
        assert_eq!(c.parts().0, dec!(100), "the 900 must not appear at parity");
        assert_eq!(c.excluded(), ["USDT"]);
        assert!(!c.rests_on_a_peg());
    }

    // ── refusal ──────────────────────────────────────────────────────────────

    /// What an evaluation path uses. A research result denominated in two
    /// assets is two results.
    #[test]
    fn refuse_rejects_a_cross_numeraire_total_and_names_the_token() {
        let err = total_usd(
            &[(Numeraire::Usd, dec!(1)), (Numeraire::of("USDT"), dec!(1))],
            PegPolicy::Refuse,
            no_rates,
        )
        .unwrap_err();
        assert_eq!(
            err,
            NumeraireError::CrossNumeraireRefused {
                symbol: "USDT".into()
            }
        );
    }

    #[test]
    fn refuse_still_totals_a_pure_dollar_portfolio() {
        let c = total_usd(
            &[(Numeraire::Usd, dec!(5)), (Numeraire::Usd, dec!(7))],
            PegPolicy::Refuse,
            no_rates,
        )
        .unwrap();
        assert_eq!(c.parts().0, dec!(12));
    }

    // ── exclusion ────────────────────────────────────────────────────────────

    #[test]
    fn non_dollar_numeraires_are_excluded_under_every_policy() {
        for policy in [PegPolicy::AssumeParity, PegPolicy::Observed] {
            let c = total_usd(
                &[(Numeraire::Usd, dec!(10)), (Numeraire::of("ETH"), dec!(3))],
                policy,
                |_| Some(dec!(3000)),
            )
            .unwrap();
            assert_eq!(c.parts().0, dec!(10), "ETH must never be summed into USD");
            assert_eq!(c.excluded(), ["ETH"]);
        }
    }

    #[test]
    fn exclusions_are_listed_once_each() {
        let c = total_usd(
            &[
                (Numeraire::of("ETH"), dec!(1)),
                (Numeraire::of("ETH"), dec!(2)),
                (Numeraire::of("BTC"), dec!(3)),
            ],
            PegPolicy::AssumeParity,
            no_rates,
        )
        .unwrap();
        assert_eq!(c.excluded(), ["ETH", "BTC"]);
    }

    #[test]
    fn an_empty_portfolio_totals_to_zero_without_assumptions() {
        let c = total_usd(&[], PegPolicy::AssumeParity, no_rates).unwrap();
        assert_eq!(c.parts().0, Decimal::ZERO);
        assert!(!c.rests_on_a_peg());
    }

    #[test]
    fn a_converted_total_round_trips_through_json_with_its_assumptions() {
        let c = total_usd(
            &[(Numeraire::of("USDT"), dec!(5))],
            PegPolicy::AssumeParity,
            no_rates,
        )
        .unwrap();
        let s = serde_json::to_string(&c).unwrap();
        assert!(s.contains("assume_parity"), "{s}");
        assert_eq!(serde_json::from_str::<Converted>(&s).unwrap(), c);
    }
}

// ── Comparability (Set T task T-0.1) ─────────────────────────────────────────

/// Two series that are not in the same unit and were about to be compared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("refusing to compare a {left} series with a {right} one: different numeraires")]
pub struct NumeraireMismatch {
    pub left: String,
    pub right: String,
}

impl Numeraire {
    /// Refuse to treat two differently-denominated series as one series.
    ///
    /// There is deliberately no conversion here. Converting a *return* series
    /// between numeraires is not a scalar multiply — it needs the FX path over
    /// the same bars, and inventing one inside a comparison is how a divergence
    /// figure becomes a fiction. Converting happens upstream, where the equity
    /// curve still exists and the rate is a PIT lookup; by the time two return
    /// series meet, the only honest operation is to check they agree.
    ///
    /// A return series is a sequence of dimensionless ratios, which is exactly
    /// why the unit has to travel beside it: nothing about `0.0031` says whether
    /// the equity curve it came from was counted in dollars, in Tether, or in
    /// the base asset.
    ///
    /// # Errors
    /// [`NumeraireMismatch`] when the two differ.
    pub fn assert_comparable(&self, other: &Numeraire) -> Result<(), NumeraireMismatch> {
        if self == other {
            Ok(())
        } else {
            Err(NumeraireMismatch {
                left: self.symbol().to_string(),
                right: other.symbol().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod comparability_tests {
    use super::*;

    #[test]
    fn the_same_numeraire_compares() {
        assert!(Numeraire::Usd.assert_comparable(&Numeraire::Usd).is_ok());
    }

    /// The whole point: a dollar-pegged token is not a dollar, and the
    /// comparison has to say so rather than adding the numbers.
    #[test]
    fn a_stablecoin_series_is_not_comparable_with_a_dollar_one() {
        let err = Numeraire::Usd
            .assert_comparable(&Numeraire::of("USDT"))
            .expect_err("USDT is not USD");
        assert_eq!(err.left, "USD");
        assert_eq!(err.right, "USDT");
    }

    /// And two different stablecoins are not each other either. USDC and USDT
    /// have depegged in opposite directions on the same day.
    #[test]
    fn two_stablecoins_are_not_comparable_with_each_other() {
        assert!(Numeraire::of("USDC")
            .assert_comparable(&Numeraire::of("USDT"))
            .is_err());
    }
}

/// Renders the venue's own symbol: `USD`, `USDC`, `JPY`.
///
/// Deliberately the symbol and not a prettier form. An error message that says
/// "USD and USDC" is one a reader can act on; one that says "dollars and
/// dollars" is the bug this module exists to prevent, printed.
impl std::fmt::Display for Numeraire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.symbol())
    }
}

// ── Storage (Set T, migration 0105) ──────────────────────────────────────────

/// A numeraire the forward-observation tables have no token for.
///
/// The schema pins six: `usd`, `usdt`, `usdc`, `eur`, `btc`, `eth`. This type is
/// deliberately wider than that — a venue can quote in `DAI` or `JPY` and the
/// domain has to be able to say so — so the narrowing is a real operation that
/// can fail, and it fails *here*, naming the symbol, rather than as a CHECK
/// violation from Postgres three layers down with the row already built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "no forward-observation storage token for numeraire `{symbol}`: migration 0105 accepts      usd, usdt, usdc, eur, btc and eth. Widen the CHECK and this match together, or convert      upstream where the equity curve still exists."
)]
pub struct UnstorableNumeraire {
    pub symbol: String,
}

impl Numeraire {
    /// The lowercase token migration 0105's CHECK accepts.
    ///
    /// # Errors
    /// [`UnstorableNumeraire`] for anything outside the schema's six.
    pub fn as_storage_token(&self) -> Result<&'static str, UnstorableNumeraire> {
        match self.symbol().to_ascii_uppercase().as_str() {
            "USD" => Ok("usd"),
            "USDT" => Ok("usdt"),
            "USDC" => Ok("usdc"),
            "EUR" => Ok("eur"),
            "BTC" => Ok("btc"),
            "ETH" => Ok("eth"),
            other => Err(UnstorableNumeraire {
                symbol: other.to_string(),
            }),
        }
    }
}

/// Parses the storage token, and the venue symbol it came from.
///
/// Case-insensitive, because `usd` comes back from Postgres and `USD` comes in
/// from a venue, and a comparison keyed on equality would otherwise pass for two
/// series that were never in the same unit.
impl std::str::FromStr for Numeraire {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Numeraire::of(s))
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn the_schemas_six_round_trip_through_their_tokens() {
        for token in ["usd", "usdt", "usdc", "eur", "btc", "eth"] {
            let n = Numeraire::from_str(token).unwrap();
            assert_eq!(n.as_storage_token().unwrap(), token, "{token}");
        }
    }

    /// The failure is named and typed, not a Postgres CHECK violation.
    #[test]
    fn a_numeraire_the_schema_has_no_token_for_is_refused_by_name() {
        let err = Numeraire::of("DAI")
            .as_storage_token()
            .expect_err("DAI is not in the six");
        assert_eq!(err.symbol, "DAI");
        assert!(err.to_string().contains("migration 0105"));
    }

    /// `usd` from Postgres and `USD` from a venue are the same unit.
    #[test]
    fn parsing_is_case_insensitive() {
        assert_eq!(
            Numeraire::from_str("usd").unwrap(),
            Numeraire::from_str("USD").unwrap()
        );
        assert_eq!(
            Numeraire::from_str("usdt").unwrap(),
            Numeraire::from_str("USDT").unwrap()
        );
    }
}
