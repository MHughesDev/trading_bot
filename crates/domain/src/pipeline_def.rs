//! Pipeline DAG definition format (I-5.1, Phase 5).
//!
//! A declarative pipeline composes existing model-registry capabilities
//! (materialize → features → target → train → calibrate → evaluate → register)
//! into a versioned, runnable, fan-outable artifact.

use serde::{Deserialize, Serialize};

pub const PIPELINE_SCHEMA_VERSION: &str = "1.1";

// ── Valid ops ─────────────────────────────────────────────────────────────────

/// All ops recognized by the DAG executor.
pub const TRAINING_OPS: &[&str] = &[
    "materialize",
    "features",
    "target",
    "train",
    "calibrate",
    "evaluate",
    "register",
];
pub const INFERENCE_OPS: &[&str] = &["load_bundle", "predict", "calibrate", "publish"];

/// Ops that are only legal in a training pipeline.
const TRAINING_ONLY: &[&str] = &["train", "register"];
/// Ops that are only legal in an inference pipeline.
const INFERENCE_ONLY: &[&str] = &["load_bundle", "predict", "publish"];

// ── Sub-types ─────────────────────────────────────────────────────────────────

/// One node in the pipeline DAG.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineNode {
    /// Local node identifier — unique within the DAG.
    pub id: String,
    /// Operation to execute (e.g. `"train"`, `"evaluate"`).
    pub op: String,
    /// Upstream node IDs this node waits on.
    #[serde(default)]
    pub needs: Vec<String>,
    /// Op-specific parameters (interpreted by the executor).
    #[serde(default)]
    pub params: serde_json::Value,
}

/// Fan-out matrix — the cross-product of all non-empty axes is instantiated.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct PipelineMatrix {
    /// Instrument / asset IDs to fan out over (e.g. `["BTC-USD", "ETH-USD"]`).
    #[serde(default)]
    pub asset: Vec<String>,
    /// Timeframe codes (e.g. `["5m", "1h"]`).
    #[serde(default)]
    pub timeframe: Vec<String>,
    /// Named window presets (e.g. `["fast", "slow"]`).
    #[serde(default)]
    pub window: Vec<String>,
}

impl PipelineMatrix {
    /// Returns true when the matrix produces ≥ 2 cells.
    pub fn is_fan_out(&self) -> bool {
        let lens = [self.asset.len(), self.timeframe.len(), self.window.len()];
        lens.iter().filter(|&&n| n > 0).copied().product::<usize>() > 1
    }

    /// Enumerate every (asset, timeframe, window) cell in the cross-product.
    /// Empty axes are represented as `None` in the tuple.
    pub fn cells(&self) -> Vec<MatrixCell> {
        let assets = if self.asset.is_empty() {
            vec![None]
        } else {
            self.asset.iter().map(|s| Some(s.clone())).collect()
        };
        let timeframes = if self.timeframe.is_empty() {
            vec![None]
        } else {
            self.timeframe.iter().map(|s| Some(s.clone())).collect()
        };
        let windows = if self.window.is_empty() {
            vec![None]
        } else {
            self.window.iter().map(|s| Some(s.clone())).collect()
        };

        let mut out = Vec::with_capacity(assets.len() * timeframes.len() * windows.len());
        for a in &assets {
            for t in &timeframes {
                for w in &windows {
                    out.push(MatrixCell {
                        asset: a.clone(),
                        timeframe: t.clone(),
                        window: w.clone(),
                    });
                }
            }
        }
        out
    }
}

/// One cell produced by expanding the fan-out matrix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MatrixCell {
    pub asset: Option<String>,
    pub timeframe: Option<String>,
    pub window: Option<String>,
}

impl MatrixCell {
    pub fn label(&self) -> String {
        let parts: Vec<&str> = [
            self.asset.as_deref(),
            self.timeframe.as_deref(),
            self.window.as_deref(),
        ]
        .iter()
        .flatten()
        .copied()
        .collect();
        if parts.is_empty() {
            "default".to_string()
        } else {
            parts.join("_")
        }
    }
}

// ── PipelineDefinition ────────────────────────────────────────────────────────

/// Bar-cadence scheduling spec — trigger the pipeline every `every_n_bars` bars
/// of `reference_instrument` at `timeframe`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BarSchedule {
    pub reference_instrument: String,
    pub timeframe: String,
    pub every_n_bars: u32,
}

/// The full pipeline definition stored in `pipelines.definition_json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineDefinition {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,

    /// `"training"` produces+registers bundles; `"inference"` assembles+publishes.
    pub kind: String,

    /// Human-readable name.
    pub name: String,

    /// Ordered list of DAG nodes.  Execution respects `needs` edges.
    pub dag: Vec<PipelineNode>,

    /// Optional fan-out matrix.  If absent or all axes empty — single cell.
    #[serde(default)]
    pub matrix: Option<PipelineMatrix>,

    /// Whether this definition is a reusable template (not directly runnable).
    #[serde(default)]
    pub template: bool,

    /// Bar-cadence schedule — if set the pipeline is triggered every N bars.
    #[serde(default)]
    pub schedule: Option<BarSchedule>,
}

fn default_schema_version() -> String {
    PIPELINE_SCHEMA_VERSION.to_string()
}

// ── Validation ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PipelineValidationError {
    pub path: String,
    pub message: String,
}

/// Validate a `PipelineDefinition`.  Returns `Ok(())` or a non-empty error list.
pub fn validate_pipeline(def: &PipelineDefinition) -> Result<(), Vec<PipelineValidationError>> {
    let mut errs: Vec<PipelineValidationError> = Vec::new();

    if def.dag.is_empty() {
        errs.push(PipelineValidationError {
            path: "dag".into(),
            message: "pipeline DAG must have at least one node".into(),
        });
    }

    // Collect node IDs for edge resolution.
    let node_ids: std::collections::HashSet<&str> = def.dag.iter().map(|n| n.id.as_str()).collect();

    // Duplicate node IDs.
    let mut seen = std::collections::HashSet::new();
    for node in &def.dag {
        if node.id.is_empty() {
            errs.push(PipelineValidationError {
                path: "dag[].id".into(),
                message: "node id must not be empty".into(),
            });
        }
        if !seen.insert(node.id.as_str()) {
            errs.push(PipelineValidationError {
                path: format!("dag[{}].id", node.id),
                message: format!("duplicate node id '{}'", node.id),
            });
        }
    }

    // Kind validation.
    let valid_kinds = ["training", "inference"];
    if !valid_kinds.contains(&def.kind.as_str()) {
        errs.push(PipelineValidationError {
            path: "kind".into(),
            message: format!(
                "unknown kind '{}'; expected one of: {}",
                def.kind,
                valid_kinds.join(", ")
            ),
        });
    }

    let all_known: Vec<&str> = TRAINING_OPS
        .iter()
        .chain(INFERENCE_OPS.iter())
        .copied()
        .collect();

    for node in &def.dag {
        // Unknown op.
        if !all_known.contains(&node.op.as_str()) {
            errs.push(PipelineValidationError {
                path: format!("dag[{}].op", node.id),
                message: format!("unknown op '{}'", node.op),
            });
        }

        // Kind-op legality.
        if def.kind == "training" && INFERENCE_ONLY.contains(&node.op.as_str()) {
            errs.push(PipelineValidationError {
                path: format!("dag[{}].op", node.id),
                message: format!("op '{}' is only legal in an inference pipeline", node.op),
            });
        }
        if def.kind == "inference" && TRAINING_ONLY.contains(&node.op.as_str()) {
            errs.push(PipelineValidationError {
                path: format!("dag[{}].op", node.id),
                message: format!("op '{}' is only legal in a training pipeline", node.op),
            });
        }

        // Unresolvable edges.
        for dep in &node.needs {
            if !node_ids.contains(dep.as_str()) {
                errs.push(PipelineValidationError {
                    path: format!("dag[{}].needs", node.id),
                    message: format!("depends on unknown node '{dep}'"),
                });
            }
        }
    }

    // Cycle detection via DFS.
    if errs.is_empty() && has_cycle(&def.dag) {
        errs.push(PipelineValidationError {
            path: "dag".into(),
            message: "pipeline DAG contains a cycle".into(),
        });
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

/// Returns true if the DAG contains a directed cycle.
fn has_cycle(nodes: &[PipelineNode]) -> bool {
    use std::collections::HashMap;
    let idx: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();

    let mut color = vec![0u8; nodes.len()]; // 0=white, 1=grey, 2=black

    fn dfs(
        i: usize,
        nodes: &[PipelineNode],
        idx: &std::collections::HashMap<&str, usize>,
        color: &mut Vec<u8>,
    ) -> bool {
        color[i] = 1;
        for dep in &nodes[i].needs {
            if let Some(&j) = idx.get(dep.as_str()) {
                if color[j] == 1 {
                    return true;
                }
                if color[j] == 0 && dfs(j, nodes, idx, color) {
                    return true;
                }
            }
        }
        color[i] = 2;
        false
    }

    for i in 0..nodes.len() {
        if color[i] == 0 && dfs(i, nodes, &idx, &mut color) {
            return true;
        }
    }
    false
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn training_def() -> PipelineDefinition {
        PipelineDefinition {
            schema_version: PIPELINE_SCHEMA_VERSION.into(),
            kind: "training".into(),
            name: "test".into(),
            dag: vec![
                PipelineNode {
                    id: "data".into(),
                    op: "materialize".into(),
                    needs: vec![],
                    params: serde_json::Value::Null,
                },
                PipelineNode {
                    id: "train".into(),
                    op: "train".into(),
                    needs: vec!["data".into()],
                    params: serde_json::Value::Null,
                },
                PipelineNode {
                    id: "evaluate".into(),
                    op: "evaluate".into(),
                    needs: vec!["train".into()],
                    params: serde_json::Value::Null,
                },
                PipelineNode {
                    id: "register".into(),
                    op: "register".into(),
                    needs: vec!["evaluate".into()],
                    params: serde_json::Value::Null,
                },
            ],
            matrix: None,
            template: false,
            schedule: None,
        }
    }

    #[test]
    fn valid_training_pipeline_round_trips() {
        let def = training_def();
        assert!(validate_pipeline(&def).is_ok());
        let json = serde_json::to_string(&def).unwrap();
        let def2: PipelineDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(def, def2);
    }

    #[test]
    fn empty_dag_is_rejected() {
        let mut def = training_def();
        def.dag.clear();
        assert!(validate_pipeline(&def).is_err());
    }

    #[test]
    fn unknown_op_is_rejected() {
        let mut def = training_def();
        def.dag[0].op = "fly_to_moon".into();
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("fly_to_moon")));
    }

    #[test]
    fn cycle_is_rejected() {
        let mut def = training_def();
        // data → train → evaluate → data (cycle)
        def.dag[0].needs.push("evaluate".into());
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("cycle")));
    }

    #[test]
    fn unresolvable_edge_is_rejected() {
        let mut def = training_def();
        def.dag[1].needs.push("ghost_node".into());
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("ghost_node")));
    }

    #[test]
    fn inference_only_op_rejected_in_training() {
        let mut def = training_def();
        def.dag[0].op = "predict".into();
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs
            .iter()
            .any(|e| e.message.contains("inference pipeline")));
    }

    #[test]
    fn training_only_op_rejected_in_inference() {
        let mut def = training_def();
        def.kind = "inference".into();
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.message.contains("training pipeline")));
    }

    #[test]
    fn matrix_cells_cross_product() {
        let m = PipelineMatrix {
            asset: vec!["BTC-USD".into(), "ETH-USD".into()],
            timeframe: vec!["5m".into(), "1h".into()],
            window: vec![],
        };
        let cells = m.cells();
        assert_eq!(cells.len(), 4);
        assert!(m.is_fan_out());
    }

    #[test]
    fn single_cell_matrix_not_fan_out() {
        let m = PipelineMatrix {
            asset: vec!["BTC-USD".into()],
            timeframe: vec![],
            window: vec![],
        };
        assert!(!m.is_fan_out());
    }

    #[test]
    fn invalid_kind_is_rejected() {
        let mut def = training_def();
        def.kind = "magic".into();
        let errs = validate_pipeline(&def).unwrap_err();
        assert!(errs.iter().any(|e| e.path == "kind"));
    }
}
