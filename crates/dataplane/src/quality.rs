//! Quality flags bitmask (SPEC §1.9). Flags are data, not filters: a dataset spec
//! declares an exclusion mask, and that mask enters the dataset content hash.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct QualityFlags(pub u32);

impl QualityFlags {
    pub const STALE_QUOTE: Self = Self(0x0001);
    pub const CROSSED_BOOK: Self = Self(0x0002);
    pub const WIDE_SPREAD: Self = Self(0x0004);
    pub const LOW_UPDATE_COUNT: Self = Self(0x0008);
    pub const VENUE_OUTAGE: Self = Self(0x0010);
    pub const SUSPECT_VOLUME: Self = Self(0x0020);
    pub const HALTED: Self = Self(0x0040);
    pub const AUCTION_ONLY: Self = Self(0x0080);
    pub const CORP_ACTION_ADJ: Self = Self(0x0100);
    pub const EXPIRY_WEEK: Self = Self(0x0200);
    pub const REORG_PENDING: Self = Self(0x0400);
    pub const SYNTHETIC_ROLL: Self = Self(0x0800);
    pub const VENDOR_REVISED: Self = Self(0x1000);
    pub const INTERPOLATED: Self = Self(0x2000);
    pub const QUALITY_TIER_3: Self = Self(0x4000);
    /// `knowledge_time` was not observed at ingest and was backfilled with the
    /// declared sentinel `event_time + vendor_lag` (CLAUDE.md §6). An honest hole.
    pub const BACKFILLED_KNOWLEDGE_TIME: Self = Self(0x8000);
    /// Series derived from back-adjusted futures or quarantined adjusted history.
    pub const NON_REPRODUCIBLE: Self = Self(0x1_0000);

    pub const NONE: Self = Self(0);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Would a row with these flags be excluded under `mask`?
    #[must_use]
    pub const fn excluded_by(self, mask: Self) -> bool {
        self.intersects(mask)
    }
}

impl std::ops::BitOr for QualityFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for QualityFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_bit_values_are_fixed() {
        // These values are persisted; changing one silently re-labels history.
        assert_eq!(QualityFlags::STALE_QUOTE.0, 0x0001);
        assert_eq!(QualityFlags::VENUE_OUTAGE.0, 0x0010);
        assert_eq!(QualityFlags::REORG_PENDING.0, 0x0400);
        assert_eq!(QualityFlags::QUALITY_TIER_3.0, 0x4000);
    }

    #[test]
    fn exclusion_mask() {
        let f = QualityFlags::SUSPECT_VOLUME | QualityFlags::BACKFILLED_KNOWLEDGE_TIME;
        assert!(f.excluded_by(QualityFlags::SUSPECT_VOLUME));
        assert!(!f.excluded_by(QualityFlags::STALE_QUOTE));
        assert!(f.contains(QualityFlags::SUSPECT_VOLUME));
    }
}
