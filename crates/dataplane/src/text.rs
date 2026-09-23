//! Text items and their point-in-time rules (Set P, P-3.5; DATA-005 §10;
//! BS-007 06_DATA §7).
//!
//! Three timestamps, kept apart on purpose:
//!
//! - `published_at` — when the publisher released it. **This is the knowledge
//!   time of the news itself.**
//! - `ingested_time` — when the platform stored it. Later, sometimes much later.
//! - `available_time` — when the platform could first have acted on it. Never
//!   earlier than `published_at`; for a backfilled item it is `published_at` plus
//!   the source's declared lag.
//!
//! Every read filters on `available_time <= as_of`. A correction is a **new row**
//! with a higher `revision`, never an update, so a read as of a past instant
//! returns the text as it stood then — including the version that was later
//! corrected, which is the version a strategy would actually have traded on.
//!
//! The revision rule is enforced here rather than documented: [`TextItem::revise`]
//! is the only way to produce a successor, it consumes nothing and mutates
//! nothing, and [`validate_revision_chain`] refuses a chain that edits history.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One version of one text item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextItem {
    /// Stable identity of the *story*, shared by every revision of it.
    pub item_id: String,
    /// 0 for the first version; each correction increments it.
    pub revision: u32,
    /// `dataplane.source.source_id` of the feed it came from.
    pub source_id: i32,
    /// The publisher's own id for this story, for idempotent ingestion.
    pub external_id: String,
    pub headline: String,
    pub body: Option<String>,
    /// Instruments the item is about, as the publisher tagged them.
    pub symbols: Vec<String>,
    pub published_at: DateTime<Utc>,
    pub ingested_time: DateTime<Utc>,
    pub available_time: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error(
        "item {item_id} rev {revision}: available_time {available_time} precedes \
         published_at {published_at} — an item cannot be actionable before it is published"
    )]
    AvailableBeforePublished {
        item_id: String,
        revision: u32,
        published_at: DateTime<Utc>,
        available_time: DateTime<Utc>,
    },
    #[error("item {item_id}: revision {revision} repeats an earlier revision number")]
    DuplicateRevision { item_id: String, revision: u32 },
    #[error(
        "item {item_id}: revision {revision} is available at {available_time}, at or before \
         revision {prev_revision} at {prev_available_time} — a correction cannot predate \
         what it corrects"
    )]
    RevisionNotAfterPredecessor {
        item_id: String,
        revision: u32,
        available_time: DateTime<Utc>,
        prev_revision: u32,
        prev_available_time: DateTime<Utc>,
    },
    #[error("item {item_id}: revisions must share one item_id, saw {other_id}")]
    MixedIdentity { item_id: String, other_id: String },
    #[error("item id must not be empty")]
    EmptyId,
}

impl TextItem {
    /// The first version of an item, available as soon as it was published plus
    /// `declared_lag_ms` (0 for a live stream).
    #[must_use]
    pub fn first(
        item_id: impl Into<String>,
        source_id: i32,
        external_id: impl Into<String>,
        headline: impl Into<String>,
        published_at: DateTime<Utc>,
        ingested_time: DateTime<Utc>,
        declared_lag_ms: i64,
    ) -> Self {
        let available_time = published_at + chrono::Duration::milliseconds(declared_lag_ms.max(0));
        Self {
            item_id: item_id.into(),
            revision: 0,
            source_id,
            external_id: external_id.into(),
            headline: headline.into(),
            body: None,
            symbols: Vec::new(),
            published_at,
            ingested_time,
            available_time,
        }
    }

    /// A correction: a **new row**, one revision higher.
    ///
    /// `corrected_at` is when the correction was published, and becomes the new
    /// row's `published_at`. The original row is untouched — this takes `&self`
    /// and returns a new value, so there is no in-place edit to write by mistake.
    #[must_use]
    pub fn revise(
        &self,
        headline: impl Into<String>,
        body: Option<String>,
        corrected_at: DateTime<Utc>,
        ingested_time: DateTime<Utc>,
        declared_lag_ms: i64,
    ) -> Self {
        let available_time = corrected_at + chrono::Duration::milliseconds(declared_lag_ms.max(0));
        Self {
            item_id: self.item_id.clone(),
            revision: self.revision + 1,
            source_id: self.source_id,
            external_id: self.external_id.clone(),
            headline: headline.into(),
            body,
            symbols: self.symbols.clone(),
            published_at: corrected_at,
            ingested_time,
            available_time,
        }
    }

    /// # Errors
    /// Rejects an item actionable before it was published.
    pub fn validate(&self) -> Result<(), TextError> {
        if self.item_id.trim().is_empty() {
            return Err(TextError::EmptyId);
        }
        if self.available_time < self.published_at {
            return Err(TextError::AvailableBeforePublished {
                item_id: self.item_id.clone(),
                revision: self.revision,
                published_at: self.published_at,
                available_time: self.available_time,
            });
        }
        Ok(())
    }
}

/// Checks that `chain` is a legitimate revision history of one item.
///
/// # Errors
/// Rejects mixed identities, repeated revision numbers, and a correction that
/// claims to have been available no later than the version it corrects — the
/// shape an in-place edit takes once somebody writes it as two rows.
pub fn validate_revision_chain(chain: &[TextItem]) -> Result<(), TextError> {
    let Some(first) = chain.first() else {
        return Ok(());
    };
    let mut sorted: Vec<&TextItem> = chain.iter().collect();
    sorted.sort_by_key(|i| i.revision);
    let mut prev: Option<&TextItem> = None;
    for item in sorted {
        item.validate()?;
        if item.item_id != first.item_id {
            return Err(TextError::MixedIdentity {
                item_id: first.item_id.clone(),
                other_id: item.item_id.clone(),
            });
        }
        if let Some(p) = prev {
            if p.revision == item.revision {
                return Err(TextError::DuplicateRevision {
                    item_id: item.item_id.clone(),
                    revision: item.revision,
                });
            }
            if item.available_time <= p.available_time {
                return Err(TextError::RevisionNotAfterPredecessor {
                    item_id: item.item_id.clone(),
                    revision: item.revision,
                    available_time: item.available_time,
                    prev_revision: p.revision,
                    prev_available_time: p.available_time,
                });
            }
        }
        prev = Some(item);
    }
    Ok(())
}

/// The version of every item that was current at `as_of`.
///
/// Items not yet available are absent; items available in an older version return
/// that older version. Ordered by availability, then by item id, so the result is
/// stable across runs.
#[must_use]
pub fn items_as_of(rows: &[TextItem], as_of: DateTime<Utc>) -> Vec<&TextItem> {
    let mut ids: Vec<&str> = rows.iter().map(|r| r.item_id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    let mut out: Vec<&TextItem> = ids
        .into_iter()
        .filter_map(|id| item_as_of(rows, id, as_of))
        .collect();
    out.sort_by(|a, b| {
        a.available_time
            .cmp(&b.available_time)
            .then(a.item_id.cmp(&b.item_id))
    });
    out
}

/// The version of `item_id` that was current at `as_of`, if any.
#[must_use]
pub fn item_as_of<'a>(
    rows: &'a [TextItem],
    item_id: &str,
    as_of: DateTime<Utc>,
) -> Option<&'a TextItem> {
    rows.iter()
        .filter(|r| r.item_id == item_id && r.available_time <= as_of)
        .max_by(|a, b| {
            a.available_time
                .cmp(&b.available_time)
                .then(a.revision.cmp(&b.revision))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0)
            .single()
            .expect("valid instant")
    }

    fn story() -> Vec<TextItem> {
        let original = TextItem::first(
            "alpaca:1001",
            30,
            "1001",
            "Acme beats estimates",
            t(2024, 5, 1, 13),
            t(2024, 5, 1, 13),
            0,
        );
        let corrected = original.revise(
            "Correction: Acme misses estimates",
            None,
            t(2024, 5, 1, 17),
            t(2024, 5, 1, 17),
            0,
        );
        vec![original, corrected]
    }

    /// P-3.5 acceptance: a corrected article produces two rows and an as-of read
    /// returns the version current at that time — including the wrong one, which
    /// is the one that moved the market.
    #[test]
    fn a_correction_is_a_new_row_and_does_not_rewrite_the_past() {
        let rows = story();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].revision, 0);
        assert_eq!(rows[1].revision, 1);
        assert_eq!(rows[0].item_id, rows[1].item_id);

        let midday =
            item_as_of(&rows, "alpaca:1001", t(2024, 5, 1, 15)).expect("published at 13:00");
        assert_eq!(midday.headline, "Acme beats estimates");
        assert_eq!(midday.revision, 0);

        let evening =
            item_as_of(&rows, "alpaca:1001", t(2024, 5, 1, 19)).expect("corrected at 17:00");
        assert_eq!(evening.headline, "Correction: Acme misses estimates");
        assert_eq!(evening.revision, 1);
    }

    /// The adversarial half: a read before publication returns nothing.
    #[test]
    fn an_item_is_never_returned_before_its_publication_time() {
        let rows = story();
        assert!(item_as_of(&rows, "alpaca:1001", t(2024, 5, 1, 12)).is_none());
        assert!(items_as_of(&rows, t(2024, 5, 1, 12)).is_empty());
    }

    #[test]
    fn a_declared_lag_pushes_availability_past_publication() {
        let item = TextItem::first(
            "alpaca:2",
            30,
            "2",
            "Delayed feed",
            t(2024, 5, 1, 13),
            t(2024, 5, 1, 13),
            900_000,
        );
        assert_eq!(
            item.available_time,
            t(2024, 5, 1, 13) + chrono::Duration::minutes(15)
        );
        assert!(item_as_of(std::slice::from_ref(&item), "alpaca:2", t(2024, 5, 1, 13)).is_none());
    }

    #[test]
    fn an_item_available_before_it_was_published_is_refused() {
        let mut bad = story().remove(0);
        bad.available_time = bad.published_at - chrono::Duration::hours(1);
        assert!(matches!(
            bad.validate(),
            Err(TextError::AvailableBeforePublished { .. })
        ));
    }

    /// An in-place edit written as two rows: same availability, higher revision.
    /// Refused, because it would silently change what a past as-of read returns.
    #[test]
    fn a_revision_that_backdates_itself_is_refused() {
        let rows = story();
        let mut backdated = rows[1].clone();
        backdated.published_at = rows[0].published_at;
        backdated.available_time = rows[0].available_time;
        let chain = vec![rows[0].clone(), backdated];
        assert!(matches!(
            validate_revision_chain(&chain),
            Err(TextError::RevisionNotAfterPredecessor { .. })
        ));
    }

    #[test]
    fn a_duplicated_revision_number_is_refused() {
        let rows = story();
        let mut dup = rows[1].clone();
        dup.revision = 0;
        let chain = vec![rows[0].clone(), dup];
        assert!(matches!(
            validate_revision_chain(&chain),
            Err(TextError::DuplicateRevision { .. })
        ));
    }

    #[test]
    fn a_well_formed_chain_validates() {
        assert_eq!(validate_revision_chain(&story()), Ok(()));
    }

    #[test]
    fn items_as_of_returns_one_row_per_story() {
        let mut rows = story();
        rows.push(TextItem::first(
            "alpaca:1002",
            30,
            "1002",
            "Other news",
            t(2024, 5, 1, 14),
            t(2024, 5, 1, 14),
            0,
        ));
        let at_1800 = items_as_of(&rows, t(2024, 5, 1, 18));
        assert_eq!(at_1800.len(), 2);
        assert_eq!(at_1800[0].item_id, "alpaca:1002");
        assert_eq!(at_1800[1].headline, "Correction: Acme misses estimates");
    }
}
