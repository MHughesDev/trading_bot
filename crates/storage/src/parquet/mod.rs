//! Raw event archive writer — Parquet files partitioned by lane/venue/instrument/date.
pub mod compaction;
pub mod partition;

use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;

use arrow::array::BinaryArray;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;

#[derive(Debug, Error)]
pub enum ParquetError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parquet: {0}")]
    Parquet(String),
    #[error("arrow: {0}")]
    Arrow(String),
}

/// Writes raw normalized events to Parquet files partitioned by lane/venue/instrument/date.
pub struct ParquetWriter {
    base_path: PathBuf,
}

impl ParquetWriter {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    /// Write a batch of JSON-encoded event bytes to the appropriate partition.
    ///
    /// Each row is stored as a single `raw_json` binary column.
    /// File path: `{base_path}/{partition_path}/{batch_id}.parquet`.
    pub async fn write_batch(
        &self,
        lane: &str,
        venue_id: &str,
        instrument_id: &str,
        date: chrono::NaiveDate,
        batch_id: &str,
        rows: &[Vec<u8>],
    ) -> Result<(), ParquetError> {
        let rel = partition::partition_path(lane, venue_id, instrument_id, date);
        let dir = self.base_path.join(&rel);
        tokio::fs::create_dir_all(&dir).await?;

        let file_path = dir.join(format!("{batch_id}.parquet"));

        let schema = Arc::new(Schema::new(vec![Field::new(
            "raw_json",
            DataType::Binary,
            false,
        )]));

        let array: BinaryArray = rows
            .iter()
            .map(|b| Some(b.as_slice()))
            .collect::<Vec<_>>()
            .into_iter()
            .collect();

        let batch = RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(array)])
            .map_err(|e| ParquetError::Arrow(e.to_string()))?;

        // Write synchronously inside a blocking task to avoid holding a file handle
        // across await points.
        let file_path_clone = file_path.clone();
        let schema_clone = Arc::clone(&schema);
        let batch_clone = batch;

        tokio::task::spawn_blocking(move || -> Result<(), ParquetError> {
            let file = std::fs::File::create(&file_path_clone)?;
            let mut writer = ArrowWriter::try_new(file, schema_clone, None)
                .map_err(|e| ParquetError::Parquet(e.to_string()))?;
            writer
                .write(&batch_clone)
                .map_err(|e| ParquetError::Parquet(e.to_string()))?;
            writer
                .close()
                .map_err(|e| ParquetError::Parquet(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| ParquetError::Parquet(e.to_string()))??;

        Ok(())
    }
}
