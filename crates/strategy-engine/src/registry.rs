//! What the compiler may look up: indicator families, feature sets, and model
//! contracts (ADR-0045 decision 2 — these three registries, and nothing else,
//! define what a strategy may say).
//!
//! The engine never reaches a database. A caller supplies model contracts —
//! the API from the model registry, a test from a map — through [`Registries`].

use std::collections::BTreeMap;

use domain::model_def::contract::IoContract;

/// A model reference resolved to a pinned version and its contract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModel {
    /// The model's stable id.
    pub model: String,
    /// The pinned version. Aliases are resolved before compilation, never after
    /// (ADR-0043 decision 4).
    pub version: u32,
    pub contract: IoContract,
}

/// Lookups the compiler needs.
pub trait Registries {
    /// Declared lookback of a DATA-006 indicator, `None` if no such indicator.
    fn feature_lookback(&self, name: &str) -> Option<u32>;
    /// The ordered features of a named feature set.
    fn feature_set(&self, name: &str) -> Option<Vec<String>>;
    /// Resolve `name@version` or `name@alias` to a pinned model and contract.
    fn model(&self, reference: &str) -> Option<ResolvedModel>;
}

/// Indicators and feature sets from the `features` crate — the one indicator
/// engine — and models from an in-memory table. The API supplies its own
/// [`Registries`] backed by the model registry; tests and the backtest runner of
/// an already-pinned run use this.
#[derive(Clone, Debug, Default)]
pub struct StdRegistries {
    models: BTreeMap<String, ResolvedModel>,
}

impl StdRegistries {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a model under the exact reference strategies use for it.
    #[must_use]
    pub fn with_model(mut self, reference: &str, model: ResolvedModel) -> Self {
        self.models.insert(reference.to_string(), model);
        self
    }
}

impl Registries for StdRegistries {
    fn feature_lookback(&self, name: &str) -> Option<u32> {
        features::runtime::lookback_bars(name)
    }

    fn feature_set(&self, name: &str) -> Option<Vec<String>> {
        features::feature_sets::resolve(name).map(|s| s.features.clone())
    }

    fn model(&self, reference: &str) -> Option<ResolvedModel> {
        self.models.get(reference).cloned()
    }
}
