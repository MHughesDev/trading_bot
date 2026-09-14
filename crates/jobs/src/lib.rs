//! Durable job service and content-addressed artifact store (COMP-005, ADR-0030).
//!
//! One place where every slow thing runs: backtests, sweeps, studies, training,
//! backfills, data QC, skill verification and eval tasks. Before this, each of those
//! had its own ad-hoc handling — a fixed 3-concurrent semaphore in the backtest
//! manager, sweeps in an in-memory map, asset-init jobs elsewhere — so a restart
//! orphaned compute and could double-count trials.
//!
//! Three properties carry the design:
//!
//! 1. **Idempotent submission** (JB-02). A job is identified by the canonical hash of
//!    its manifest, its code and its data snapshot, so asking twice gets one job.
//! 2. **Trials counted at submission** (JB-03, INV-1). Registration happens inside the
//!    submission transaction, so no client can run an evaluation without the count.
//! 3. **Leases, not liveness assumptions** (JB-04). A worker holds a lease it must
//!    renew; if it dies, the job returns to the queue rather than vanishing.

pub mod checkpoint;
pub mod models;
pub mod artifacts;
pub mod manifest;
pub mod store;
pub mod types;
pub mod worker;

pub use artifacts::{Artifact, ArtifactRegistry};
pub use store::{
    ExplorationEntry, Job, JobStore, JobStoreError, PgTrialCounter, RefuseTrials, Submission,
    Submitted, TrialCounter,
};
pub use types::{
    Cost, JobError, JobKind, JobOutput, JobState, Progress, Queue, SubmittedBy, WorkerClass,
};
pub use worker::{run_lease_reaper, JobContext, Worker, WorkerPool};
