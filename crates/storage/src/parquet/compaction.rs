//! Nightly compaction — merges small batch files per partition.
//! Invoked by a scheduled job in Phase 2.
use std::path::Path;

use super::ParquetError;

pub async fn compact_partition(partition_dir: &Path) -> Result<(), ParquetError> {
    // Full merge logic is deferred to Phase 2.  Log a warning so scheduled
    // invocations don't silently appear successful while doing nothing.
    tracing::warn!(
        path = %partition_dir.display(),
        "compact_partition: not yet implemented — partition not compacted"
    );
    Ok(())
}
