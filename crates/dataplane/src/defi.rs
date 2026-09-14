//! DeFi pools: blocks keyed by hash, reorgs, finality policies (SPEC §1.8, INV-09).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Block identity. The hash is part of the key: block number alone is never a key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BlockRef {
    pub chain_id: i32,
    pub block_number: i64,
    pub block_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChainBlock {
    pub block: BlockRef,
    pub parent_hash: String,
    pub block_time: DateTime<Utc>,
    pub finalized_at: Option<DateTime<Utc>>,
    /// Not null ⇒ reorged out. The row is retained, never deleted.
    pub orphaned_at: Option<DateTime<Utc>>,
    pub knowledge_time: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PoolObservation {
    pub pool_id: String,
    pub block: BlockRef,
    pub price: Decimal,
    pub liquidity: Decimal,
    /// When the indexer saw it.
    pub knowledge_time: DateTime<Utc>,
}

/// Part of the dataset content hash for any dataset reading pool data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "n")]
pub enum FinalityPolicy {
    NConfirmations(u32),
    FinalizedOnly,
}

#[derive(Default, Debug)]
pub struct ChainState {
    blocks: HashMap<BlockRef, ChainBlock>,
}

impl ChainState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, b: ChainBlock) {
        self.blocks.insert(b.block.clone(), b);
    }

    #[must_use]
    pub fn get(&self, r: &BlockRef) -> Option<&ChainBlock> {
        self.blocks.get(r)
    }

    /// Apply a reorg: every block on `chain_id` at or above `from_number` that is not
    /// in `new_canonical` is marked orphaned at `seen_at`. Returns the orphaned refs.
    pub fn apply_reorg(&mut self, chain_id: i32, from_number: i64, new_canonical: &[BlockRef], seen_at: DateTime<Utc>) -> Vec<BlockRef> {
        let keep: HashSet<&BlockRef> = new_canonical.iter().collect();
        let mut orphaned = Vec::new();
        for (r, b) in &mut self.blocks {
            if r.chain_id == chain_id && r.block_number >= from_number && !keep.contains(r) && b.orphaned_at.is_none() {
                b.orphaned_at = Some(seen_at);
                orphaned.push(r.clone());
            }
        }
        orphaned.sort();
        orphaned
    }

    fn tip(&self, chain_id: i32, as_of: DateTime<Utc>) -> Option<i64> {
        self.blocks
            .values()
            .filter(|b| b.block.chain_id == chain_id && b.knowledge_time <= as_of && b.orphaned_at.is_none_or(|o| o > as_of))
            .map(|b| b.block.block_number)
            .max()
    }

    /// Was this block trustworthy under `policy`, as known by `as_of`?
    #[must_use]
    pub fn admissible(&self, r: &BlockRef, policy: FinalityPolicy, as_of: DateTime<Utc>) -> bool {
        let Some(b) = self.blocks.get(r) else { return false };
        if b.knowledge_time > as_of || b.orphaned_at.is_some_and(|o| o <= as_of) {
            return false;
        }
        match policy {
            FinalityPolicy::FinalizedOnly => b.finalized_at.is_some_and(|f| f <= as_of),
            FinalityPolicy::NConfirmations(n) => self
                .tip(r.chain_id, as_of)
                .is_some_and(|tip| tip - r.block_number >= i64::from(n)),
        }
    }

    /// PIT pool read under a finality policy.
    #[must_use]
    pub fn read<'a>(&self, obs: &'a [PoolObservation], policy: FinalityPolicy, as_of: DateTime<Utc>) -> Vec<&'a PoolObservation> {
        obs.iter()
            .filter(|o| o.knowledge_time <= as_of && self.admissible(&o.block, policy, as_of))
            .collect()
    }
}

/// Trials whose datasets consumed any orphaned block must be flagged (SPEC §17).
#[must_use]
pub fn dependent_trials<'a>(orphaned: &[BlockRef], consumption: &'a [(uuid::Uuid, Vec<BlockRef>)]) -> Vec<&'a uuid::Uuid> {
    let set: HashSet<&BlockRef> = orphaned.iter().collect();
    consumption
        .iter()
        .filter(|(_, blocks)| blocks.iter().any(|b| set.contains(b)))
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use rust_decimal_macros::dec;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    fn br(n: i64, fork: &str) -> BlockRef {
        BlockRef { chain_id: 1, block_number: n, block_hash: format!("0x{fork}{n}") }
    }

    /// AT-09: a 12-block reorg.
    #[test]
    fn twelve_block_reorg() {
        let mut chain = ChainState::new();
        let mut obs = Vec::new();
        for n in 0..40 {
            let seen = t0() + Duration::seconds(12 * n);
            chain.insert(ChainBlock { block: br(n, "a"), parent_hash: String::new(), block_time: seen, finalized_at: (n < 20).then(|| seen + Duration::minutes(13)), orphaned_at: None, knowledge_time: seen });
            obs.push(PoolObservation { pool_id: "p".into(), block: br(n, "a"), price: dec!(1), liquidity: dec!(1), knowledge_time: seen });
        }
        // Reorg replaces blocks 28..=39 with fork "b".
        let reorg_seen = t0() + Duration::seconds(12 * 40);
        let canon: Vec<BlockRef> = (0..28).map(|n| br(n, "a")).chain((28..40).map(|n| br(n, "b"))).collect();
        for n in 28..40 {
            chain.insert(ChainBlock { block: br(n, "b"), parent_hash: String::new(), block_time: reorg_seen, finalized_at: None, orphaned_at: None, knowledge_time: reorg_seen });
        }
        let orphaned = chain.apply_reorg(1, 28, &canon, reorg_seen);
        assert_eq!(orphaned.len(), 12);
        // Orphans are retained with orphaned_at.
        assert!(chain.get(&br(30, "a")).unwrap().orphaned_at.is_some());

        // A finalized_only dataset never saw orphaned data, at any as_of.
        for as_of in [reorg_seen - Duration::seconds(1), reorg_seen + Duration::hours(1)] {
            let read = chain.read(&obs, FinalityPolicy::FinalizedOnly, as_of);
            assert!(read.iter().all(|o| o.block.block_number < 20));
        }

        // Dependent trials are flagged.
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let consumption = vec![(a, vec![br(10, "a")]), (b, vec![br(10, "a"), br(33, "a")])];
        assert_eq!(dependent_trials(&orphaned, &consumption), vec![&b]);
    }

    #[test]
    fn n_confirmations_policy() {
        let mut chain = ChainState::new();
        for n in 0..10 {
            chain.insert(ChainBlock { block: br(n, "a"), parent_hash: String::new(), block_time: t0(), finalized_at: None, orphaned_at: None, knowledge_time: t0() });
        }
        assert!(chain.admissible(&br(3, "a"), FinalityPolicy::NConfirmations(6), t0()));
        assert!(!chain.admissible(&br(5, "a"), FinalityPolicy::NConfirmations(6), t0()));
    }
}
