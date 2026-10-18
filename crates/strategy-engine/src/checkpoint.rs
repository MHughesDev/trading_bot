//! Checkpoint and restore (Set U U-4.4, FEAT-007 §9.5).
//!
//! A live deployment checkpoints after every decision so that a restart
//! *resumes* the strategy rather than starting it again. Starting again is the
//! dangerous failure: a strategy that wakes up with no memory of the bracket it
//! armed, or of the tags it already used, can place an entry it has already
//! placed.
//!
//! What is stored is what bars cannot give back:
//!
//! - the **planner state** — brackets armed and waiting for their entry's fill,
//!   and the tag sequence, so a restarted strategy never reuses a tag (a reused
//!   tag is a reused idempotency key);
//! - the **tick counter**;
//! - the close of the **last bar seen on each input**.
//!
//! What is *not* stored is indicator history and the state of stateful
//! operators. Both are a pure function of the bars, so restore replays the
//! stored bars up to the checkpoint as warm-up and arrives at exactly the state
//! the engine had. Storing them as well would be a second copy that could
//! disagree with the first.
//!
//! A checkpoint names the plan revision it came from, and restoring it into a
//! different plan is refused: the planner state of one graph means nothing in
//! another.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::engine::{Engine, EngineError, Event};
use crate::history::Bar;
use crate::plan::{InputIdx, Plan};
use crate::planner::PlannerState;
use crate::portfolio::{InstrumentSpec, PortfolioView};

/// The checkpoint format. Bump when the shape changes; an older checkpoint is
/// refused rather than misread.
pub const CHECKPOINT_FORMAT: u32 = 1;

/// An engine's resumable state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub format: u32,
    /// The plan revision this state belongs to.
    pub revision: String,
    pub tick: u64,
    pub planner: PlannerState,
    /// Close of the last bar seen on each input, by input index.
    pub last_close_ns: Vec<Option<i64>>,
}

/// Why a checkpoint could not be restored.
#[derive(Debug, PartialEq, thiserror::Error)]
pub enum RestoreError {
    #[error("checkpoint is from plan revision {checkpoint}, not {plan}: a strategy's state is meaningless in another graph")]
    PlanMismatch { checkpoint: String, plan: String },
    #[error("checkpoint format {0} is not {CHECKPOINT_FORMAT}")]
    Format(u32),
    #[error("checkpoint has {checkpoint} inputs, the plan {plan}")]
    InputCount { checkpoint: usize, plan: usize },
    #[error("engine: {0}")]
    Engine(#[from] EngineError),
}

impl Engine {
    /// The engine's resumable state.
    ///
    /// # Errors
    /// [`EngineError::TickPending`] while a tick waits for model answers: a
    /// half-evaluated tick is not a state to resume from.
    pub fn checkpoint(&self) -> Result<Checkpoint, EngineError> {
        if self.is_pending() {
            return Err(EngineError::TickPending);
        }
        Ok(Checkpoint {
            format: CHECKPOINT_FORMAT,
            revision: self.plan.revision.clone(),
            tick: self.tick,
            planner: self.planner.clone(),
            last_close_ns: self.inst.last_close.clone(),
        })
    }

    /// Rebuild an engine from a checkpoint and the stored bars of each input.
    ///
    /// Every bar in `history` that closed at or before the checkpoint's last
    /// close on its input is replayed as warm-up — rebuilding indicators and
    /// stateful operators exactly — and bars after it are ignored (they are the
    /// caller's to feed live). Then the tick counter and planner state are put
    /// back.
    ///
    /// # Errors
    /// A checkpoint from another plan or format, or an engine error.
    pub fn restore(
        plan: Arc<Plan>,
        checkpoint: &Checkpoint,
        history: impl IntoIterator<Item = (InputIdx, Bar)>,
        spec: &InstrumentSpec,
    ) -> Result<Self, RestoreError> {
        Self::restore_with_external(plan, checkpoint, history, spec, None)
    }

    /// [`Self::restore`], with the external rows the plan's macro and
    /// fundamentals indicators read (P-3.6) in place before the history is
    /// replayed, so those columns rebuild exactly too.
    ///
    /// # Errors
    /// As [`Self::restore`].
    pub fn restore_with_external(
        plan: Arc<Plan>,
        checkpoint: &Checkpoint,
        history: impl IntoIterator<Item = (InputIdx, Bar)>,
        spec: &InstrumentSpec,
        external: Option<crate::external::ExternalData>,
    ) -> Result<Self, RestoreError> {
        if checkpoint.format != CHECKPOINT_FORMAT {
            return Err(RestoreError::Format(checkpoint.format));
        }
        if checkpoint.revision != plan.revision {
            return Err(RestoreError::PlanMismatch {
                checkpoint: checkpoint.revision.clone(),
                plan: plan.revision.clone(),
            });
        }
        if checkpoint.last_close_ns.len() != plan.inputs.len() {
            return Err(RestoreError::InputCount {
                checkpoint: checkpoint.last_close_ns.len(),
                plan: plan.inputs.len(),
            });
        }
        let mut engine = Self::new(plan)?;
        if let Some(data) = external {
            engine.inst.external = Arc::new(data);
        }
        let mut bars: Vec<(InputIdx, Bar)> = history
            .into_iter()
            .filter(|(i, b)| {
                checkpoint
                    .last_close_ns
                    .get(*i)
                    .copied()
                    .flatten()
                    .is_some_and(|last| b.close_ns <= last)
            })
            .collect();
        // The order bars closed in, other inputs before the primary at an equal
        // instant — the order every environment feeds them.
        bars.sort_by_key(|(i, b)| (b.close_ns, *i == crate::plan::PRIMARY, *i));
        let view = PortfolioView::default();
        for (input, bar) in bars {
            engine.on_event(
                Event::Bar {
                    input,
                    bar,
                    warmup: true,
                },
                &view,
                spec,
            )?;
        }
        engine.tick = checkpoint.tick;
        engine.planner = checkpoint.planner.clone();
        engine.inst.last_close.clone_from(&checkpoint.last_close_ns);
        Ok(engine)
    }
}
