use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdapterSpec {
    /// e.g. "ollama", "openai", "anthropic"
    pub provider: String,
    /// e.g. "gemma2:9b"
    pub model: String,
    /// e.g. "http://localhost:11434"
    pub endpoint: String,
    #[serde(default)]
    pub default_params: serde_json::Value,
    /// Cost per 1k tokens as decimal string (ADR-0002 compliant).
    #[serde(default = "default_cost")]
    pub cost_per_1k_tokens: String,
}

fn default_cost() -> String {
    "0.0".to_string()
}
