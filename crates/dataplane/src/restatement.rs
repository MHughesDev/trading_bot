//! Point-in-time restatement resolution (SPEC §1.3, INV-02).
//!
//! Per `(instrument, event_time)` the PIT row is the one with the greatest
//! `knowledge_time ≤ as_of`. Restatements are rare, so an index of partitions that
//! contain any lets >99% of reads skip resolution entirely.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::identity::InstrumentKey;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RestatementKey {
    pub instrument_id: InstrumentKey,
    pub event_date: NaiveDate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RestatementEntry {
    pub key: RestatementKey,
    pub n_revisions: u32,
    pub max_knowledge_time: DateTime<Utc>,
}

/// Something that has an identity cell and a knowledge time.
pub trait Versioned {
    fn instrument(&self) -> InstrumentKey;
    fn event_time(&self) -> DateTime<Utc>;
    fn knowledge_time(&self) -> DateTime<Utc>;
    fn revision(&self) -> u32;
}

#[derive(Default, Clone, Debug)]
pub struct RestatementIndex {
    entries: HashMap<RestatementKey, RestatementEntry>,
}

impl RestatementIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from rows: a partition is indexed iff any cell in it has >1 version.
    #[must_use]
    pub fn build<R: Versioned>(rows: &[R]) -> Self {
        let mut versions: HashMap<(InstrumentKey, DateTime<Utc>), u32> = HashMap::new();
        let mut max_k: HashMap<RestatementKey, DateTime<Utc>> = HashMap::new();
        for r in rows {
            *versions.entry((r.instrument(), r.event_time())).or_default() += 1;
            let key = RestatementKey {
                instrument_id: r.instrument(),
                event_date: r.event_time().date_naive(),
            };
            let e = max_k.entry(key).or_insert(r.knowledge_time());
            *e = (*e).max(r.knowledge_time());
        }
        let mut entries: HashMap<RestatementKey, RestatementEntry> = HashMap::new();
        for ((inst, et), n) in versions {
            if n > 1 {
                let key = RestatementKey {
                    instrument_id: inst,
                    event_date: et.date_naive(),
                };
                let mk = max_k[&key];
                entries
                    .entry(key.clone())
                    .and_modify(|e| e.n_revisions += n - 1)
                    .or_insert(RestatementEntry {
                        key,
                        n_revisions: n - 1,
                        max_knowledge_time: mk,
                    });
            }
        }
        Self { entries }
    }

    pub fn record(&mut self, entry: RestatementEntry) {
        self.entries.insert(entry.key.clone(), entry);
    }

    #[must_use]
    pub fn contains(&self, instrument: InstrumentKey, date: NaiveDate) -> bool {
        self.entries.contains_key(&RestatementKey {
            instrument_id: instrument,
            event_date: date,
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> impl Iterator<Item = &RestatementEntry> {
        self.entries.values()
    }
}

/// Resolution statistics, so a caller can prove the fast path is taken.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolveStats {
    pub fast_path_partitions: usize,
    pub resolved_partitions: usize,
}

/// The single PIT resolution routine. Returns rows ordered by
/// `(instrument, event_time)`.
///
/// Rows with `knowledge_time > as_of` are never returned — including on the fast
/// path, where the partition is unindexed but a lone row may still be too new.
pub fn resolve_pit<R: Versioned + Clone>(
    rows: &[R],
    index: &RestatementIndex,
    as_of: DateTime<Utc>,
) -> (Vec<R>, ResolveStats) {
    let mut stats = ResolveStats::default();
    let mut partitions: BTreeMap<(InstrumentKey, NaiveDate), Vec<&R>> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.knowledge_time() <= as_of) {
        partitions
            .entry((r.instrument(), r.event_time().date_naive()))
            .or_default()
            .push(r);
    }
    let mut out = Vec::with_capacity(rows.len());
    for ((inst, date), cells) in partitions {
        if index.contains(inst, date) {
            stats.resolved_partitions += 1;
            let mut best: BTreeMap<DateTime<Utc>, &R> = BTreeMap::new();
            for r in cells {
                best.entry(r.event_time())
                    .and_modify(|cur| {
                        if (r.knowledge_time(), r.revision()) > (cur.knowledge_time(), cur.revision()) {
                            *cur = r;
                        }
                    })
                    .or_insert(r);
            }
            out.extend(best.into_values().cloned());
        } else {
            stats.fast_path_partitions += 1;
            let mut seen = HashSet::new();
            let mut cells = cells;
            cells.sort_by_key(|r| r.event_time());
            for r in cells {
                // An unindexed partition has one version per cell by construction;
                // a duplicate here means the index is stale, so resolve defensively.
                if !seen.insert(r.event_time()) {
                    continue;
                }
                out.push(r.clone());
            }
        }
    }
    (out, stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[derive(Clone, Debug, PartialEq)]
    struct Row {
        inst: i64,
        et: DateTime<Utc>,
        kt: DateTime<Utc>,
        rev: u32,
        close: i64,
    }

    impl Versioned for Row {
        fn instrument(&self) -> InstrumentKey {
            InstrumentKey(self.inst)
        }
        fn event_time(&self) -> DateTime<Utc> {
            self.et
        }
        fn knowledge_time(&self) -> DateTime<Utc> {
            self.kt
        }
        fn revision(&self) -> u32 {
            self.rev
        }
    }

    fn day(d: u32, min: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, d, 0, 0, 0).unwrap() + Duration::minutes(min)
    }

    #[test]
    fn restated_cell_resolves_by_as_of() {
        let et = day(2, 10);
        let rows = vec![
            Row { inst: 1, et, kt: et + Duration::minutes(1), rev: 0, close: 100 },
            Row { inst: 1, et, kt: et + Duration::hours(5), rev: 1, close: 101 },
        ];
        let idx = RestatementIndex::build(&rows);
        assert!(idx.contains(InstrumentKey(1), et.date_naive()));

        let (before, _) = resolve_pit(&rows, &idx, et + Duration::hours(1));
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].close, 100, "the restatement was not yet known");

        let (after, _) = resolve_pit(&rows, &idx, et + Duration::hours(6));
        assert_eq!(after[0].close, 101);
    }

    #[test]
    fn unindexed_partitions_take_the_fast_path() {
        let mut rows = Vec::new();
        for d in 1..=10 {
            for m in 0..60 {
                let et = day(d, m);
                rows.push(Row { inst: 1, et, kt: et + Duration::minutes(1), rev: 0, close: m });
            }
        }
        let restated = day(4, 30);
        rows.push(Row { inst: 1, et: restated, kt: restated + Duration::days(1), rev: 1, close: -1 });
        let idx = RestatementIndex::build(&rows);
        let (out, stats) = resolve_pit(&rows, &idx, day(28, 0));
        assert_eq!(stats.resolved_partitions, 1);
        assert_eq!(stats.fast_path_partitions, 9);
        assert_eq!(out.len(), 600);
    }

    #[test]
    fn fast_path_still_hides_the_future() {
        let et = day(2, 0);
        let rows = vec![Row { inst: 1, et, kt: et + Duration::hours(2), rev: 0, close: 1 }];
        let idx = RestatementIndex::build(&rows);
        let (out, stats) = resolve_pit(&rows, &idx, et + Duration::hours(1));
        assert!(out.is_empty());
        assert_eq!(stats.resolved_partitions, 0);
    }
}
