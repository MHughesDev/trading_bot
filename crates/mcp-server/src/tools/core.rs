//! The `core` namespace: the protocol tools every profile exposes.
//!
//! `get_authoring_guide` is the escape hatch that makes AGENT-006 P1 hold — the
//! catalogue may be any size, the prompt may not, so the prose a tool used to
//! carry in its description lives in the guide and is fetched a section at a
//! time. The handler is in `lib.rs`; this module holds only the declaration.

use serde_json::json;

use crate::spec::ToolSpec;
use crate::taxonomy::Risk;

pub fn specs() -> Vec<ToolSpec> {
    vec![ToolSpec::new(
        "get_authoring_guide",
        "core",
        Risk::Read,
        "Return the strategy-authoring protocol: definition JSON format, expression grammar, \
         supported features, backtest workflow, and result interpretation. Call it once before \
         authoring anything.",
        json!({ "type": "object", "properties": {} }),
    )
    .internal()]
}
