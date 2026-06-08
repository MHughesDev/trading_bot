//! Pipeline lifecycle types: handles, factory trait, and a no-op implementation.

use std::fmt;

use domain::lanes::Lane;

/// A handle to a running pipeline.  Calling `stop()` signals the pipeline to shut down.
pub struct PipelineHandle {
    stop_tx: tokio::sync::oneshot::Sender<()>,
}

impl fmt::Debug for PipelineHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PipelineHandle")
    }
}

impl PipelineHandle {
    pub fn new(stop_tx: tokio::sync::oneshot::Sender<()>) -> Self {
        Self { stop_tx }
    }

    /// Send the stop signal to the pipeline (consumes self).
    pub fn stop(self) {
        let _ = self.stop_tx.send(());
    }
}

/// Responsible for starting a data pipeline for a `(lane, instrument)` pair.
pub trait PipelineFactory: Send + Sync + 'static {
    fn start(&self, lane: &Lane, instrument: &str) -> PipelineHandle;
}

/// A no-op factory — used in tests and when no real pipeline is wired up.
pub struct NoopPipelineFactory;

impl PipelineFactory for NoopPipelineFactory {
    fn start(&self, _lane: &Lane, _instrument: &str) -> PipelineHandle {
        let (tx, _rx) = tokio::sync::oneshot::channel();
        PipelineHandle::new(tx)
    }
}
