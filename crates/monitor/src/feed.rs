//! What a feed is, and why a bare symbol is not one.
//!
//! # The measurement that made this a type
//!
//! On 2026-09-21 the live `trading.market_bar` table held five 60-second feeds.
//! Three of them were `BTC-USD`:
//!
//! | symbol  | venue    | age of last bar |
//! |---------|----------|-----------------|
//! | BTC-USD | kraken   | 92 seconds      |
//! | BTC-USD | coinbase | 5,372 s (90 min)|
//! | BTC-USD | binance  | 7,893,272 s (91 days) |
//!
//! One symbol, three venues, staleness spanning five orders of magnitude. A
//! staleness verdict keyed on the string `"BTC-USD"` has to choose one of those
//! numbers, and **both choices are wrong in a direction that costs money**: take
//! the freshest and a strategy pricing off a feed dead for three months keeps
//! trading; take the stalest and every strategy on the 92-second feed is halted
//! for a fault at a venue it does not touch. ADR-D-11 makes a hard-stale feed a
//! *halt*, so the second mistake now stops the book rather than shrinking it.
//!
//! So the key is a pair, and the pair is a type rather than a convention.
//! [`FeedKey`] cannot be built from one string: its constructor takes a symbol
//! and a [`Venue`], and `Venue` is a distinct type with no `From<&str>` and no
//! `FromStr`, so the two halves cannot be transposed and neither can be omitted.
//! A comment asking callers to remember the venue is exactly what was there
//! before — [`crate::family::Fact::HardDataStaleness`] carried an `instrument:
//! String` and every test passed.
//!
//! # The names these strings are drawn from
//!
//! A venue is named the way the operational database names it: `instruments.
//! venue_id` and `dataplane.venue.name` both hold `coinbase`, `kraken`,
//! `binance`, and [`risk::Scope::venue`] already speaks the same strings, so a
//! `Venue` here and the scope a reconciliation mismatch halts are the same
//! vocabulary rather than two that must be kept in step.

/// A key that would name a feed nobody can identify.
///
/// A `Result` rather than a silent normalisation, and both variants are blanks
/// for the same reason: a `FeedKey` whose venue is `""` is a symbol-keyed entry
/// wearing the new type, and it would match nothing in a dependency map built
/// from real rows while looking, at every call site, exactly like a key that
/// works.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MalformedFeedKey {
    /// No symbol, so the key names no instrument.
    #[error("a feed key must name a symbol")]
    BlankSymbol,
    /// No venue, so the key is back to the ambiguity this type exists to remove.
    #[error("a feed key must name the venue the feed is at")]
    BlankVenue,
}

/// A trading venue, by name.
///
/// Deliberately its own type and not a `String`. The only defect [`FeedKey`]
/// could still admit with two `&str` parameters is the transposed call —
/// `FeedKey::new("kraken", "BTC-USD")` — which compiles, runs, halts nothing it
/// should and halts nothing at all, and is invisible in review. With `Venue` it
/// does not compile.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Venue(String);

impl Venue {
    /// The venue with this name.
    ///
    /// Named `named` rather than `new` or `from` so that the call site reads as
    /// an assertion about what the string is. There is no `From<&str>` and no
    /// `FromStr`: a blanket conversion would let a symbol become a venue through
    /// inference at a call site that never mentions either type, which is the
    /// whole class of mistake this module removes.
    pub fn named(name: &str) -> Result<Self, MalformedFeedKey> {
        let name = name.trim();
        if name.is_empty() {
            return Err(MalformedFeedKey::BlankVenue);
        }
        Ok(Self(name.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Venue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One feed: one symbol, at one venue.
///
/// The unit a strategy actually depends on and the unit staleness is actually
/// measured in. `BTC-USD` is not a feed; `BTC-USD at kraken` is.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeedKey {
    symbol: String,
    venue: Venue,
}

impl FeedKey {
    /// The feed for this symbol at this venue.
    ///
    /// The venue is not optional and has no default. An `Option<Venue>` here, or
    /// a `venue.unwrap_or("unknown")`, would reintroduce the ambiguous key with
    /// an extra step — and `unknown` is a real row in `dataplane.venue`, so it
    /// would even join.
    pub fn new(symbol: &str, venue: Venue) -> Result<Self, MalformedFeedKey> {
        let symbol = symbol.trim();
        if symbol.is_empty() {
            return Err(MalformedFeedKey::BlankSymbol);
        }
        Ok(Self {
            symbol: symbol.to_owned(),
            venue,
        })
    }

    /// The symbol alone.
    ///
    /// Read-only, and there is no constructor that takes only this back. The
    /// getter exists so an event can name the symbol; it is not a key.
    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    #[must_use]
    pub fn venue(&self) -> &Venue {
        &self.venue
    }

    /// The scope a decision about this feed's venue would be written against.
    ///
    /// Here rather than at the call sites so that the venue string reaching
    /// [`risk::Scope`] is one that was validated as a venue.
    #[must_use]
    pub fn venue_scope(&self) -> risk::Scope {
        risk::Scope::venue(self.venue.as_str())
    }
}

impl std::fmt::Display for FeedKey {
    /// `BTC-USD at kraken` — the form an operator reads in an event.
    ///
    /// Prose rather than a separator like `BTC-USD@kraken`, because a compact
    /// round-trippable form is the first thing somebody parses back into a key,
    /// and parsing a key back out of a string is how the bare symbol returns.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.symbol, self.venue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_feed_is_a_symbol_and_a_venue_and_neither_half_may_be_blank() {
        let k = FeedKey::new("BTC-USD", Venue::named("kraken").expect("a venue")).expect("a feed");
        assert_eq!(k.symbol(), "BTC-USD");
        assert_eq!(k.venue().as_str(), "kraken");
        assert_eq!(k.to_string(), "BTC-USD at kraken");

        assert_eq!(Venue::named(""), Err(MalformedFeedKey::BlankVenue));
        assert_eq!(Venue::named("   "), Err(MalformedFeedKey::BlankVenue));
        assert_eq!(
            FeedKey::new("  ", Venue::named("kraken").expect("a venue")),
            Err(MalformedFeedKey::BlankSymbol)
        );
    }

    #[test]
    fn the_same_symbol_at_two_venues_is_two_different_feeds() {
        // The property the whole module exists for, stated against the live
        // database's own rows: `trading.market_bar` holds BTC-USD at coinbase,
        // kraken and binance, and on 2026-09-21 their last 60s bars were 90
        // minutes, 92 seconds and 91 days old. If these two values compared
        // equal, one staleness verdict would stand for all three.
        let venue = |v: &str| Venue::named(v).expect("a venue");
        let kraken = FeedKey::new("BTC-USD", venue("kraken")).expect("a feed");
        let binance = FeedKey::new("BTC-USD", venue("binance")).expect("a feed");
        assert_ne!(kraken, binance);
        assert_eq!(kraken.symbol(), binance.symbol());
        // And the two are distinguishable to a map, not merely to `==`.
        let mut set = std::collections::BTreeSet::new();
        set.insert(kraken.clone());
        set.insert(binance);
        assert_eq!(set.len(), 2);
        // Anti-vacuity: the same feed named twice is one feed.
        set.insert(FeedKey::new("BTC-USD", venue("kraken")).expect("a feed"));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn a_venue_named_with_surrounding_space_is_the_same_venue() {
        // Two keys that differ only by whitespace are two entries in a
        // `BTreeMap`, and the one the gauge writes would then find no
        // dependants — a stale feed halting nothing, silently.
        assert_eq!(
            Venue::named(" kraken ").expect("a venue"),
            Venue::named("kraken").expect("a venue")
        );
        let padded = FeedKey::new(" BTC-USD ", Venue::named("kraken").expect("a venue"))
            .expect("a feed");
        let plain =
            FeedKey::new("BTC-USD", Venue::named("kraken").expect("a venue")).expect("a feed");
        assert_eq!(padded, plain);
    }

    #[test]
    fn there_is_no_way_back_to_a_bare_string() {
        // The door this module was written to close, asserted rather than
        // requested. Every one of these would reopen it, and each is a thing a
        // later reader adds in good faith while making a call site tidier:
        //
        //   `impl From<&str> for Venue`     — inference turns a symbol into a venue
        //   `impl FromStr for FeedKey`      — `"BTC-USD".parse()` names no venue
        //   `fn new(symbol: &str, venue: &str)` — transposable, and silent
        //
        // A doc comment asking for none of them is what `Fact::HardDataStaleness`
        // had, in the form of a field called `instrument` that everybody read as
        // a symbol, which it was.
        let source = include_str!("feed.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("this file has a production half");
        // Anti-vacuity first: if the split stopped finding the constructors,
        // every assertion below is true of an empty string.
        assert!(production.contains("fn named("), "the guard is broken, not satisfied");
        assert!(production.contains("fn new("), "the guard is broken, not satisfied");
        for door in [
            "impl From<&str>",
            "impl std::str::FromStr",
            "fn from_str",
            "venue: &str",
        ] {
            assert!(
                !production.contains(door),
                "src/feed.rs has grown {door:?}, which puts back the single-string key \
                 this module exists to remove: BTC-USD is three live feeds whose last \
                 bars differ by five orders of magnitude, and since ADR-D-11 choosing \
                 the wrong one halts strategies whose prices are seconds old"
            );
        }
    }

    #[test]
    fn a_feeds_venue_scope_is_the_scope_the_rest_of_the_platform_halts() {
        // The vocabulary join: `instruments.venue_id`, `dataplane.venue.name`
        // and `risk::Scope::venue` all speak `kraken`, so a venue validated here
        // is one the kill switch can be asked about rather than a second
        // spelling that silently halts nothing.
        let k = FeedKey::new("BTC-USD", Venue::named("kraken").expect("a venue")).expect("a feed");
        assert_eq!(k.venue_scope(), risk::Scope::venue("kraken"));
    }
}
