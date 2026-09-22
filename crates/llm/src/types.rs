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
    /// A JSON Schema the response must conform to — grammar-constrained decoding
    /// (harness guide §3.1, D-18).
    ///
    /// This makes valid syntax a sampling-time guarantee rather than a parse-time
    /// hope, and it is REQUIRED for every local tier. Each provider maps it to its
    /// own mechanism: Ollama's `format`, vLLM's `guided_json`, OpenAI's
    /// `response_format: json_schema`.
    ///
    /// **It does not compose with `tools`.** Measured on Ollama 0.33.3: passing both
    /// makes `format` win and `tool_calls` come back null, with the output in
    /// `content` instead. That is the documented behaviour of this field — when it
    /// is set, expect the answer as conformant JSON in the content, not as a
    /// structured tool call. See docs/LOCAL_TIER_FINDINGS.md §2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    /// Context window to allocate on the backend, in tokens.
    ///
    /// Ollama otherwise uses the Modelfile default (often 4096) and silently
    /// left-truncates anything longer — dropping the system charter first. The
    /// harness knows the real budget, so it says it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_ctx: Option<u32>,
    /// How long the backend should keep the weights resident, e.g. `"30m"`.
    ///
    /// Without it a large local model is evicted between steps and reloaded from
    /// disk on the next call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_alive: Option<String>,
    /// Whether the model may, must, or must not call a tool this turn.
    ///
    /// The planner-executor mode needs "must call one of these" (guide §5.2): a
    /// planner that answers in prose when it was asked for a step has not failed
    /// gracefully, it has stalled the loop. `None` leaves the provider default.
    ///
    /// Not sent to Ollama, which has no equivalent — there the same guarantee comes
    /// from `schema`, which is stronger: the shape is enforced at sampling time
    /// rather than requested.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
}

/// Whether the model may, must, or must not call a tool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolChoice {
    /// The model decides. The provider default.
    Auto,
    /// The model must call some tool.
    Required,
    /// The model must answer in prose.
    None,
    /// The model must call this one. The second half of the two-step decode, for
    /// backends that express it through the tool API rather than a grammar.
    Tool(String),
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
///
/// `input_tokens` is what the provider charged at the *uncached* rate. With
/// prompt caching on (AGENT-005 §6.1) that is no longer the size of the prompt:
/// the bytes served from cache are reported separately and are not included in
/// it. Reading `input_tokens` as "how big was the prompt" was correct before
/// caching and is an undercount after it, which is why [`Usage::prompt_tokens`]
/// exists and why the estimator calibration in §7 must use it.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Prompt tokens served from a cache hit — billed at a discount, and the
    /// number AC-2 is measured against.
    #[serde(default)]
    pub cache_read_input_tokens: u64,
    /// Prompt tokens written into the cache on this call — billed at a premium
    /// and paid once per breakpoint per TTL.
    #[serde(default)]
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    /// Every prompt token this call actually read, cached or not.
    ///
    /// The three fields are disjoint: a token is fresh, or read from cache, or
    /// written to cache, never two of those. Summing them is therefore the
    /// prompt size, and it is the only one of the four numbers that can be
    /// compared against a harness estimate of the same request.
    #[must_use]
    pub fn prompt_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cache_read_input_tokens)
            .saturating_add(self.cache_creation_input_tokens)
    }

    /// The share of the prompt that did not have to be re-read, in `[0, 1]`.
    ///
    /// `None` when the call read no prompt at all, because a ratio over zero is
    /// not "perfectly cached" — it is "nothing happened", and reporting the
    /// first for the second is how an unused run comes to look like a success.
    #[must_use]
    pub fn cache_hit_rate(&self) -> Option<f64> {
        let total = self.prompt_tokens();
        if total == 0 {
            return None;
        }
        #[allow(clippy::cast_precision_loss)]
        Some(self.cache_read_input_tokens as f64 / total as f64)
    }
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
