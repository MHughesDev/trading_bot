//! Nightly compaction — merges the small batch files of a closed partition
//! into one file (Set E 5.2).
//!
//! # Crash safety
//!
//! A compaction must never lose a row and must never leave a row readable
//! twice. The sequence, per partition:
//!
//! 1. Stream every source file's batches into `compacted-{id}.parquet.tmp`
//!    (in `spawn_blocking`, like the writer), and fsync it.
//! 2. Write the manifest `compacted-{id}.manifest` — the source file names —
//!    via its own temp file and rename.
//! 3. Rename the `.tmp` to `compacted-{id}.parquet`. From here the merged file
//!    is the truth.
//! 4. Delete the sources, then the manifest.
//!
//! [`recover_partition`] runs first on every pass and finishes or rolls back
//! whatever a crash interrupted: a manifest whose `.parquet` exists means step
//! 3 happened, so its listed sources are deleted; a manifest without one means
//! it did not, so the temp and manifest are removed and the sources (untouched)
//! stand. Rows are never reordered or deduplicated here — dedup happened at
//! write time.
//!
//! Only **closed** partitions are compacted: `date=` strictly before today
//! (UTC). Today's partition is still being appended to.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;

use super::ParquetError;

const TMP_SUFFIX: &str = ".parquet.tmp";
const MANIFEST_SUFFIX: &str = ".manifest";

/// What one partition pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompactionOutcome {
    /// Source files merged (0 when there was nothing to do).
    pub merged_files: usize,
    pub rows: usize,
    /// The merged file, when one was written.
    pub output: Option<PathBuf>,
}

fn pq_err<E: std::fmt::Display>(e: E) -> ParquetError {
    ParquetError::Parquet(e.to_string())
}

fn is_final_parquet(p: &Path) -> bool {
    p.is_file()
        && p.extension().is_some_and(|e| e == "parquet")
        && !p.to_string_lossy().ends_with(TMP_SUFFIX)
}

fn sorted_dir(dir: &Path) -> Result<Vec<PathBuf>, ParquetError> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    out.sort();
    Ok(out)
}

/// Finish or roll back a compaction a crash interrupted. Returns how many
/// manifests were resolved.
///
/// # Errors
/// An I/O error.
pub fn recover_partition(dir: &Path) -> Result<usize, ParquetError> {
    let mut resolved = 0;
    for path in sorted_dir(dir)? {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Some(stem) = name.strip_suffix(MANIFEST_SUFFIX) else {
            continue;
        };
        let merged = dir.join(format!("{stem}.parquet"));
        if merged.exists() {
            // Step 3 completed: the merged file is the truth; finish step 4.
            let listed = std::fs::read_to_string(&path)?;
            for src in listed.lines().filter(|l| !l.trim().is_empty()) {
                let p = dir.join(src.trim());
                if p.exists() && p != merged {
                    std::fs::remove_file(&p)?;
                }
            }
        } else {
            // Crashed before the rename: the sources were never touched.
            let tmp = dir.join(format!("{stem}{TMP_SUFFIX}"));
            if tmp.exists() {
                std::fs::remove_file(&tmp)?;
            }
        }
        std::fs::remove_file(&path)?;
        resolved += 1;
    }
    // Orphaned temps with no manifest (crash during step 1).
    for path in sorted_dir(dir)? {
        if path.to_string_lossy().ends_with(TMP_SUFFIX) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(resolved)
}

/// Merge every parquet file in `partition_dir` into one, crash-safely.
///
/// Fewer than two files is a no-op. Files must share one schema; a partition
/// that does not is refused untouched rather than merged lossily.
///
/// # Errors
/// I/O, parquet or schema-mismatch errors. On error the sources are intact.
pub async fn compact_partition(partition_dir: &Path) -> Result<CompactionOutcome, ParquetError> {
    let dir = partition_dir.to_path_buf();
    tokio::task::spawn_blocking(move || compact_blocking(&dir))
        .await
        .map_err(pq_err)?
}

fn compact_blocking(dir: &Path) -> Result<CompactionOutcome, ParquetError> {
    recover_partition(dir)?;
    let sources: Vec<PathBuf> = sorted_dir(dir)?
        .into_iter()
        .filter(|p| is_final_parquet(p))
        .collect();
    if sources.len() < 2 {
        return Ok(CompactionOutcome::default());
    }

    let id = uuid::Uuid::new_v4().simple().to_string();
    let stem = format!("compacted-{id}");
    let tmp = dir.join(format!("{stem}{TMP_SUFFIX}"));
    let merged = dir.join(format!("{stem}.parquet"));
    let manifest = dir.join(format!("{stem}{MANIFEST_SUFFIX}"));

    // 1. Merge into the temp file.
    let rows = match write_merged(&sources, &tmp) {
        Ok(rows) => rows,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };

    // 2. The manifest, atomically.
    let names: Vec<String> = sources
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .collect();
    let manifest_tmp = dir.join(format!("{stem}{MANIFEST_SUFFIX}.tmp"));
    std::fs::write(&manifest_tmp, names.join("\n"))?;
    std::fs::rename(&manifest_tmp, &manifest)?;

    // 3. Publish the merged file.
    std::fs::rename(&tmp, &merged)?;

    // 4. Remove the sources, then the manifest.
    for src in &sources {
        std::fs::remove_file(src)?;
    }
    std::fs::remove_file(&manifest)?;

    tracing::info!(
        path = %dir.display(),
        files = sources.len(),
        rows,
        "parquet partition compacted"
    );
    Ok(CompactionOutcome {
        merged_files: sources.len(),
        rows,
        output: Some(merged),
    })
}

fn write_merged(sources: &[PathBuf], tmp: &Path) -> Result<usize, ParquetError> {
    let mut writer: Option<ArrowWriter<std::fs::File>> = None;
    let mut schema = None;
    let mut rows = 0;
    for src in sources {
        let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(src)?)
            .map_err(pq_err)?
            .build()
            .map_err(pq_err)?;
        for batch in reader {
            let batch = batch.map_err(|e| ParquetError::Arrow(e.to_string()))?;
            match &schema {
                None => {
                    schema = Some(batch.schema());
                    let file = std::fs::File::create(tmp)?;
                    writer =
                        Some(ArrowWriter::try_new(file, batch.schema(), None).map_err(pq_err)?);
                }
                Some(s) if *s != batch.schema() => {
                    return Err(ParquetError::Arrow(format!(
                        "{} has a different schema from the rest of the partition; not compacting",
                        src.display()
                    )));
                }
                Some(_) => {}
            }
            rows += batch.num_rows();
            if let Some(w) = writer.as_mut() {
                w.write(&batch).map_err(pq_err)?;
            }
        }
    }
    let Some(w) = writer else {
        // Every source was empty: write nothing, merge nothing.
        return Err(ParquetError::Parquet("every source file is empty".into()));
    };
    let file = w.into_inner().map_err(pq_err)?;
    file.sync_all()?;
    let _: Option<Arc<arrow::datatypes::Schema>> = schema;
    Ok(rows)
}

/// Every `…/date=YYYY-MM-DD/` directory under `base_path/events` strictly
/// before `today`.
///
/// # Errors
/// An I/O error walking the tree.
pub fn closed_partitions(
    base_path: &Path,
    today: chrono::NaiveDate,
) -> Result<Vec<PathBuf>, ParquetError> {
    let root = base_path.join("events");
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Some(date) = name.strip_prefix("date=") {
                if chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok_and(|d| d < today) {
                    out.push(path);
                }
            } else {
                stack.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// Compact every closed partition under `base_path`. One partition's failure
/// is logged and does not stop the others. Returns the outcomes that merged.
pub async fn compact_closed_partitions(
    base_path: &Path,
    today: chrono::NaiveDate,
) -> Result<Vec<CompactionOutcome>, ParquetError> {
    let mut done = Vec::new();
    for dir in closed_partitions(base_path, today)? {
        match compact_partition(&dir).await {
            Ok(o) if o.merged_files > 0 => done.push(o),
            Ok(_) => {}
            Err(e) => {
                tracing::error!(path = %dir.display(), error = %e, "partition compaction failed; sources intact")
            }
        }
    }
    Ok(done)
}

/// The nightly scheduler: compact closed partitions once a day at `hour_utc`.
/// Spawned by the binary that owns the archive (`apps/platform`).
pub fn spawn_nightly(base_path: PathBuf, hour_utc: u32) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let now = chrono::Utc::now();
            let today_run = now
                .date_naive()
                .and_hms_opt(hour_utc.min(23), 0, 0)
                .map(|t| t.and_utc())
                .unwrap_or(now);
            let next = if today_run > now {
                today_run
            } else {
                today_run + chrono::Duration::days(1)
            };
            let wait = (next - now)
                .to_std()
                .unwrap_or(std::time::Duration::from_secs(3600));
            tokio::time::sleep(wait).await;
            match compact_closed_partitions(&base_path, chrono::Utc::now().date_naive()).await {
                Ok(done) => tracing::info!(
                    partitions = done.len(),
                    "nightly parquet compaction finished"
                ),
                Err(e) => {
                    tracing::error!(error = %e, "nightly parquet compaction could not walk the archive")
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, BinaryArray};
    use chrono::NaiveDate;

    /// A scratch directory removed on drop (no `tempfile` dependency here).
    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("compaction-test-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read_rows(dir: &Path) -> Vec<Vec<u8>> {
        let mut rows = Vec::new();
        for p in sorted_dir(dir)
            .unwrap()
            .into_iter()
            .filter(|p| is_final_parquet(p))
        {
            let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&p).unwrap())
                .unwrap()
                .build()
                .unwrap();
            for b in reader {
                let b = b.unwrap();
                let col = b.column(0).as_any().downcast_ref::<BinaryArray>().unwrap();
                for i in 0..col.len() {
                    rows.push(col.value(i).to_vec());
                }
            }
        }
        rows.sort();
        rows
    }

    async fn seed(base: &Path, date: NaiveDate, batches: usize) -> PathBuf {
        let w = super::super::ParquetWriter::new(base);
        for b in 0..batches {
            let rows: Vec<Vec<u8>> = (0..3)
                .map(|i| format!("{{\"b\":{b},\"i\":{i}}}").into_bytes())
                .collect();
            let refs: Vec<&[u8]> = rows.iter().map(Vec::as_slice).collect();
            w.write_batch(
                "market.trades",
                "kraken",
                "BTC-USD",
                date,
                &format!("batch-{b}"),
                &refs,
            )
            .await
            .unwrap();
        }
        base.join(super::super::partition::partition_path(
            "market.trades",
            "kraken",
            "BTC-USD",
            date,
        ))
    }

    #[tokio::test]
    async fn small_files_merge_into_one_with_identical_rows() {
        let tmp = TempDir::new();
        let dir = seed(tmp.path(), NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(), 4).await;
        let before = read_rows(&dir);
        let out = compact_partition(&dir).await.unwrap();
        assert_eq!(out.merged_files, 4);
        assert_eq!(out.rows, 12);
        let files: Vec<_> = sorted_dir(&dir).unwrap();
        assert_eq!(files.len(), 1, "exactly the merged file remains: {files:?}");
        assert_eq!(read_rows(&dir), before);
        // A second pass has nothing to do.
        assert_eq!(compact_partition(&dir).await.unwrap().merged_files, 0);
    }

    #[tokio::test]
    async fn a_crash_after_publishing_finishes_without_duplicating_rows() {
        let tmp = TempDir::new();
        let dir = seed(tmp.path(), NaiveDate::from_ymd_opt(2026, 6, 2).unwrap(), 3).await;
        let before = read_rows(&dir);
        // Simulate: steps 1-3 done (merged file + manifest), step 4 not run.
        let sources: Vec<PathBuf> = sorted_dir(&dir).unwrap();
        write_merged(&sources, &dir.join("compacted-x.parquet.tmp")).unwrap();
        let names: Vec<String> = sources
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        std::fs::write(dir.join("compacted-x.manifest"), names.join("\n")).unwrap();
        std::fs::rename(
            dir.join("compacted-x.parquet.tmp"),
            dir.join("compacted-x.parquet"),
        )
        .unwrap();
        // The crash state reads every row twice; recovery must fix that.
        assert_eq!(read_rows(&dir).len(), before.len() * 2);
        assert_eq!(recover_partition(&dir).unwrap(), 1);
        assert_eq!(read_rows(&dir), before);
    }

    #[tokio::test]
    async fn a_crash_before_publishing_loses_nothing() {
        let tmp = TempDir::new();
        let dir = seed(tmp.path(), NaiveDate::from_ymd_opt(2026, 6, 3).unwrap(), 3).await;
        let before = read_rows(&dir);
        let sources: Vec<PathBuf> = sorted_dir(&dir).unwrap();
        write_merged(&sources, &dir.join("compacted-y.parquet.tmp")).unwrap();
        std::fs::write(dir.join("compacted-y.manifest"), "whatever").unwrap();
        recover_partition(&dir).unwrap();
        assert_eq!(read_rows(&dir), before);
        assert_eq!(
            sorted_dir(&dir).unwrap().len(),
            3,
            "temp and manifest removed, sources stand"
        );
    }

    #[tokio::test]
    async fn only_closed_partitions_are_compacted() {
        let tmp = TempDir::new();
        let today = NaiveDate::from_ymd_opt(2026, 6, 10).unwrap();
        let old = seed(tmp.path(), NaiveDate::from_ymd_opt(2026, 6, 9).unwrap(), 2).await;
        let live = seed(tmp.path(), today, 2).await;
        let done = compact_closed_partitions(tmp.path(), today).await.unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(sorted_dir(&old).unwrap().len(), 1);
        assert_eq!(
            sorted_dir(&live).unwrap().len(),
            2,
            "today's partition is untouched"
        );
    }
}
