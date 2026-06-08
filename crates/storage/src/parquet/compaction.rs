//! Nightly compaction stub — merges small batch files per partition.
//! Invoked by a scheduled job in Phase 2.
use std::path::Path;

use super::ParquetError;

pub async fn compact_partition(_partition_dir: &Path) -> Result<(), ParquetError> {
    // Phase 2: merge all *.parquet files in partition into one
    Ok(())
}
