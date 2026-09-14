//! Unified LLM client — one API for OpenAI, Anthropic, and local Ollama.
//!
//! Callers build a [`ChatRequest`] in provider-neutral types and get back a
//! provider-neutral [`ChatResponse`]; each provider module owns the wire
//! mapping (see the module docs for the format quirks each one absorbs).
//! Enum dispatch, no trait objects — three fixed providers, no open extension.

mod anthropic;
mod error;
mod ollama;
mod openai;
mod types;

use std::time::Duration;

pub use error::LlmError;
pub use types::{
    ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, ToolChoice, ToolDef, Usage,
};

/// The supported providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Ollama,
    /// vLLM's OpenAI-compatible server — **the production local backend** (ADR-0032).
    ///
    /// It is a separate variant from [`Provider::OpenAi`] despite sharing the wire
    /// format because two things differ and both matter: it needs no API key, and it
    /// is the only backend that does grammar-constrained decoding on tool arguments
    /// *and* the model's native tool template at the same time. Ollama makes that an
    /// either/or (docs/LOCAL_TIER_FINDINGS.md §2), which is why Ollama is a dev-box
    /// backend and this is the deployment one.
    Vllm,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Ollama => "ollama",
            Provider::Vllm => "vllm",
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            Provider::OpenAi => "https://api.openai.com",
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::Ollama => "http://localhost:11434",
            Provider::Vllm => "http://localhost:8000",
        }
    }

    /// Whether an API key is required to talk to this provider.
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Provider::Ollama | Provider::Vllm)
    }

    /// Whether this provider runs the weights on hardware we own.
    ///
    /// Drives the guarantees the guide makes unconditional for local inference:
    /// constrained decoding, the startup canary, the native chat template.
    pub fn is_local(&self) -> bool {
        matches!(self, Provider::Ollama | Provider::Vllm)
    }

    /// Whether `schema` and `tools` may be sent together.
    ///
    /// Measured, not assumed. On Ollama they do not compose: `format` wins and
    /// `tool_calls` comes back null (LOCAL_TIER_FINDINGS §2). vLLM applies
    /// `guided_json` to the arguments while still using the model's native tool
    /// template, which is the whole reason it is the production backend.
    pub fn composes_schema_with_tools(&self) -> bool {
        matches!(self, Provider::Vllm | Provider::OpenAi)
    }
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Ok(Provider::OpenAi),
            "anthropic" => Ok(Provider::Anthropic),
            "ollama" | "local" => Ok(Provider::Ollama),
            "vllm" => Ok(Provider::Vllm),
            other => Err(format!(
                "unknown provider '{other}' (expected openai | anthropic | ollama | vllm)"
            )),
        }
    }
}

/// A configured client for one provider.
#[derive(Clone)]
pub struct LlmClient {
    provider: Provider,
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl LlmClient {
    /// `base_url: None` uses the provider default. Trailing slashes are trimmed.
    pub fn new(provider: Provider, api_key: Option<String>, base_url: Option<String>) -> Self {
        let mut base_url = base_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| provider.default_base_url().to_string());
        while base_url.ends_with('/') {
            base_url.pop();
        }
        Self {
            provider,
            http: reqwest::Client::builder()
                // Generous: local models can take minutes on long prompts.
                .timeout(Duration::from_secs(600))
                .build()
                .expect("reqwest client"),
            base_url,
            api_key,
        }
    }

    pub fn provider(&self) -> Provider {
        self.provider
    }

    fn key(&self) -> Result<&str, LlmError> {
        match (&self.api_key, self.provider.requires_api_key()) {
            (Some(k), _) if !k.is_empty() => Ok(k),
            (_, false) => Ok(""),
            _ => Err(LlmError::Auth(format!(
                "no API key configured for {}",
                self.provider.as_str()
            ))),
        }
    }

    /// One chat completion (non-streaming).
    pub async fn chat(&self, req: &ChatRequest) -> Result<ChatResponse, LlmError> {
        match self.provider {
            // vLLM speaks the OpenAI wire format, so it shares the mapping rather
            // than a near-copy of it that would drift.
            Provider::OpenAi => {
                openai::chat(
                    &self.http,
                    &self.base_url,
                    self.key()?,
                    req,
                    openai::Flavor::OpenAi,
                )
                .await
            }
            Provider::Vllm => {
                openai::chat(
                    &self.http,
                    &self.base_url,
                    self.key()?,
                    req,
                    openai::Flavor::Vllm,
                )
                .await
            }
            Provider::Anthropic => {
                anthropic::chat(&self.http, &self.base_url, self.key()?, req).await
            }
            Provider::Ollama => ollama::chat(&self.http, &self.base_url, req).await,
        }
    }

    /// List models available from this provider (live query).
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        match self.provider {
            Provider::OpenAi | Provider::Vllm => {
                openai::list_models(&self.http, &self.base_url, self.key()?).await
            }
            Provider::Anthropic => {
                anthropic::list_models(&self.http, &self.base_url, self.key()?).await
            }
            Provider::Ollama => ollama::list_models(&self.http, &self.base_url).await,
        }
    }

    /// Which models this backend currently holds in memory.
    ///
    /// Only local backends can answer, and only Ollama exposes it today. An empty
    /// list means "cannot say" rather than "none", so a caller must treat it as
    /// absence of evidence — see [`ollama::resident_models`] for why the distinction
    /// decides a memory fit check.
    pub async fn resident_models(&self) -> Vec<String> {
        match self.provider {
            Provider::Ollama => ollama::resident_models(&self.http, &self.base_url).await,
            _ => Vec::new(),
        }
    }

    /// Verify the key/endpoint works; returns the number of models visible.
    pub async fn verify(&self) -> Result<usize, LlmError> {
        Ok(self.list_models().await?.len())
    }
}
