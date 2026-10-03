//! What a strategy can be made of, generated from the registries (Set U U-7.6;
//! AGENT-011; ADR-0045).
//!
//! The agent is told what exists by reading the same three registries the
//! compiler checks against — the operator registry, the feature registry
//! (DATA-006) and the model contracts — so it can be told about nothing else.
//! A hand-written list is a second list, and a second list is the defect this
//! repository keeps finding: a capability ceiling written in prose is still a
//! ceiling.

use domain::model_def::contract::IoContract;
use domain::strategy_graph::ops::{self, OpSpec};
use serde::Serialize;

/// One feature family, as an author writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeatureEntry {
    pub family: &'static str,
    /// The name a strategy writes, e.g. `rsi_14`.
    pub example: String,
    pub params: Vec<FeatureParam>,
    /// Bars before the example has a value; absent when it has no fixed
    /// lookback.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warmup_bars: Option<u32>,
    pub use_when: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias_of: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FeatureParam {
    pub name: &'static str,
    pub min: usize,
    pub max: usize,
}

/// One callable model version and its contract.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelEntry {
    /// `name@version`, as a strategy writes it.
    pub reference: String,
    pub kind: String,
    pub contract: IoContract,
}

/// Everything a strategy can use.
#[derive(Clone, Debug, Serialize)]
pub struct Catalogue {
    pub operators: Vec<&'static OpSpec>,
    pub features: Vec<FeatureEntry>,
    pub models: Vec<ModelEntry>,
}

/// The feature registry, as authors see it.
#[must_use]
pub fn features() -> Vec<FeatureEntry> {
    features::registry::families()
        .iter()
        .map(|f| FeatureEntry {
            family: f.name,
            example: f.example(),
            params: f
                .params
                .iter()
                .map(|p| FeatureParam {
                    name: p.name,
                    min: p.min,
                    max: p.max,
                })
                .collect(),
            warmup_bars: f.lookback_bars(&f.example_params()),
            use_when: f.use_when,
            alias_of: f.alias_of,
        })
        .collect()
}

/// The catalogue, with the model versions the caller's registry holds (the
/// one part that lives in a database).
#[must_use]
pub fn catalogue(models: Vec<ModelEntry>) -> Catalogue {
    Catalogue {
        operators: ops::ops().iter().collect(),
        features: features(),
        models,
    }
}

fn ty(t: &ops::Ty) -> String {
    use ops::Ty;
    match t {
        Ty::Num => "num".into(),
        Ty::Bool => "bool".into(),
        Ty::Text => "text".into(),
        Ty::Var(c) => c.to_string(),
        Ty::CatOf(c) => format!("cat<{c}>"),
        Ty::ProbsOf(c) => format!("probs<{c}>"),
        Ty::AnyDist => "dist".into(),
        Ty::AnyVec => "vec".into(),
        Ty::AnyRecord => "record".into(),
        Ty::NumList => "[num]".into(),
        Ty::BoolList => "[bool]".into(),
        Ty::Window => "window".into(),
        Ty::FromContract => "per contract".into(),
        Ty::FromSchema => "per schema".into(),
    }
}

/// The marker pair the guide's generated block sits between.
pub const GUIDE_BEGIN: &str = "<!-- generated from the registries by strategy_lang::v3::catalogue::guide_block; do not edit -->";
pub const GUIDE_END: &str = "<!-- end generated -->";

/// The operator and feature tables of `docs/agent/slv3-guide.md`, generated
/// from the registries (U-7.6). A test keeps the file equal to this.
#[must_use]
pub fn guide_block() -> String {
    let cell = |s: &str| s.replace('|', "\\|");
    let mut s = String::new();
    s.push_str(GUIDE_BEGIN);
    s.push_str("\n\n### Operators\n\n");
    s.push_str(
        "Positional arguments bind to the ports in the order shown; `lit` ports take a literal \
         or a `$param`.\n\n",
    );
    s.push_str("| operator | ports | gives | use when |\n|---|---|---|---|\n");
    for o in ops::ops() {
        let mut ports: Vec<String> = o
            .inputs
            .iter()
            .map(|p| {
                let mut x = format!("{}: {}", p.name, ty(&p.ty));
                if p.literal {
                    x.push_str(" (lit)");
                }
                if p.optional {
                    x.push_str(" (optional)");
                }
                x
            })
            .collect();
        if o.open_inputs {
            ports.push("…named".into());
        }
        let scope = if o.cross_sectional {
            " *(selection only)*"
        } else {
            ""
        };
        s.push_str(&format!(
            "| `{}` | {} | {} | {}{scope} |\n",
            o.name,
            ports.join(", "),
            ty(&o.output),
            cell(o.use_when)
        ));
    }
    s.push_str("\n### Features\n\n");
    s.push_str(
        "Read with `feature(\"<name>\")`. A family with parameters is named with them: the \
         example column is a name that resolves.\n\n",
    );
    s.push_str("| example | warm-up bars | use when |\n|---|---|---|\n");
    for f in features() {
        let warm = f
            .warmup_bars
            .map_or_else(|| "—".to_string(), |w| w.to_string());
        let alias = f
            .alias_of
            .map_or_else(String::new, |a| format!(" (alias of `{a}`)"));
        s.push_str(&format!(
            "| `{}` | {warm} | {}{alias} |\n",
            f.example,
            cell(f.use_when)
        ));
    }
    s.push('\n');
    s.push_str(GUIDE_END);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// U-7.6's acceptance: every operator and family in the registries
    /// appears, and nothing else does.
    #[test]
    fn the_catalogue_is_the_registries_and_nothing_else() {
        let c = catalogue(Vec::new());
        let ops_in: BTreeSet<&str> = c.operators.iter().map(|o| o.name).collect();
        let ops_reg: BTreeSet<&str> = ops::ops().iter().map(|o| o.name).collect();
        assert_eq!(ops_in, ops_reg);
        assert_eq!(c.operators.len(), ops::ops().len(), "none twice");
        let fam_in: BTreeSet<&str> = c.features.iter().map(|f| f.family).collect();
        let fam_reg: BTreeSet<&str> = features::registry::families()
            .iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(fam_in, fam_reg);
        assert!(
            c.operators.iter().all(|o| !o.use_when.is_empty()),
            "every operator says when to use it"
        );
        // Every family's example is a name the engine resolves.
        for f in &c.features {
            assert!(
                features::runtime::feature(&f.example).is_ok(),
                "{} does not resolve",
                f.example
            );
        }
    }
}
