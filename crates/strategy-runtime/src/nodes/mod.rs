//! v1.5 pipeline/universe node evaluation.
//!
//! These nodes operate on a *universe* — a set of instruments each with their
//! own feature values — rather than on a single instrument's `WorldState`.
//! The universe flows through: DataSource → Rank → Filter → TakeTopN → SurfaceAction.
//!
//! `evaluate_universe_pipeline` walks the node vec in order (nodes are expected
//! in dependency order) and returns the instrument IDs surfaced by the terminal
//! `SurfaceAction` node.

pub mod data_source;
pub mod filter;
pub mod rank;
pub mod surface_action;
pub mod take_top_n;

use std::collections::HashMap;
use std::sync::Arc;

use domain::strategy_def::nodes::{Node, NodeKind};

/// A single entry in the universe: one instrument plus its current feature values.
///
/// `instrument_id` is kept as `String` here because universe entries are constructed
/// from user-supplied YAML definitions, not from the intern table.  `features` uses
/// a named map (not `Vec<f64>`) so pipeline expression nodes can look up by name.
#[derive(Clone, Debug)]
pub struct UniverseEntry {
    pub instrument_id: String,
    pub features: HashMap<String, f64>,
}

/// An ordered collection of universe entries, shared via `Arc` across pipeline stages.
pub type Universe = Vec<UniverseEntry>;

/// Evaluate the v1.5 pipeline nodes over `initial_universe`.
///
/// The universe is wrapped in `Arc` at each stage so pipeline nodes share ownership
/// without cloning the full vec.  Nodes that mutate (Rank, Filter, TakeTopN) produce
/// a new `Arc`; pass-through nodes (DataSource, SurfaceAction) clone the `Arc` pointer.
///
/// Nodes are processed in the order they appear in `nodes`.  Each v1.5 node
/// references its input by node ID; the result is stored in `node_outputs` so
/// downstream nodes can look it up.  Non-v1.5 nodes (Condition, Signal) are
/// silently skipped.
///
/// Returns the instrument IDs surfaced by the last `SurfaceAction` node found,
/// or an empty vec if no `SurfaceAction` node exists.
pub fn evaluate_universe_pipeline(nodes: &[Node], initial_universe: Universe) -> Vec<String> {
    let mut node_outputs: HashMap<String, Arc<Universe>> = HashMap::new();

    for node in nodes {
        match &node.kind {
            NodeKind::DataSource { .. } => {
                node_outputs.insert(node.id.clone(), Arc::new(initial_universe.clone()));
            }
            NodeKind::Rank {
                input,
                feature,
                ascending,
            } => {
                if let Some(universe) = node_outputs.get(input) {
                    let ranked = rank::rank((**universe).clone(), feature, *ascending);
                    node_outputs.insert(node.id.clone(), Arc::new(ranked));
                }
            }
            NodeKind::Filter { input, expr } => {
                if let Some(universe) = node_outputs.get(input) {
                    let filtered = filter::filter((**universe).clone(), expr);
                    node_outputs.insert(node.id.clone(), Arc::new(filtered));
                }
            }
            NodeKind::TakeTopN { input, n } => {
                if let Some(universe) = node_outputs.get(input) {
                    let taken = take_top_n::take_top_n((**universe).clone(), *n);
                    node_outputs.insert(node.id.clone(), Arc::new(taken));
                }
            }
            NodeKind::SurfaceAction { input } => {
                if let Some(universe) = node_outputs.get(input) {
                    node_outputs.insert(node.id.clone(), Arc::clone(universe));
                }
            }
            // v1.0 nodes — not part of the universe pipeline.
            NodeKind::Condition { .. } | NodeKind::Signal { .. } => {}
        }
    }

    // Return the output of the last SurfaceAction node found.
    for node in nodes.iter().rev() {
        if let NodeKind::SurfaceAction { .. } = &node.kind {
            if let Some(universe) = node_outputs.get(&node.id) {
                return universe.iter().map(|e| e.instrument_id.clone()).collect();
            }
        }
    }

    vec![]
}
