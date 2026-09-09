//! Unified chat types shared by all providers.
//!
//! One request/response shape; each provider module maps it to and from its
//! wire format. Tool calls are normalized to `ToolCall { id, name, arguments }`
//! with `arguments` always a parsed JSON object (OpenAI's stringified
//! arguments are parsed at the boundary; Ollama's missing ids are synthesized).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A chat completion request in provider-neutral form.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    /// System prompt; mapped to the provider's native system slot.
    pub system: Option<String>,
    pub messages: Vec<Message>,
    /// Tools the model may call; empty = no tool use.
    #[serde(default)]
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
}

/// One transcript entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Message {
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(default)]
        tool_calls: Vec<ToolCall>,
    },
    /// Result of executing one tool call, fed back to the model.
    ToolResult {
        tool_call_id: String,
        name: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

/// A tool the model may call (JSON Schema input).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A tool invocation requested by the model.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Always a parsed JSON value (object in practice).
    pub arguments: Value,
}

/// Why the model stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other(String),
}

/// Token accounting for one call.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A chat completion response in provider-neutral form.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

/// One model available from a provider.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub display_name: Option<String>,
}
