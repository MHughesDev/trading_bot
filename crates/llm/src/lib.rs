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
    ChatRequest, ChatResponse, Message, ModelInfo, StopReason, ToolCall, ToolDef, Usage,
};

/// The supported providers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    OpenAi,
    Anthropic,
    Ollama,
}

impl Provider {
    pub fn as_str(&self) -> &'static str {
        match self {
            Provider::OpenAi => "openai",
            Provider::Anthropic => "anthropic",
            Provider::Ollama => "ollama",
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            Provider::OpenAi => "https://api.openai.com",
            Provider::Anthropic => "https://api.anthropic.com",
            Provider::Ollama => "http://localhost:11434",
        }
    }

    /// Whether an API key is required to talk to this provider.
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, Provider::Ollama)
    }
}

impl std::str::FromStr for Provider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openai" => Ok(Provider::OpenAi),
            "anthropic" => Ok(Provider::Anthropic),
            "ollama" | "local" => Ok(Provider::Ollama),
            other => Err(format!(
                "unknown provider '{other}' (expected openai | anthropic | ollama)"
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
            Provider::OpenAi => openai::chat(&self.http, &self.base_url, self.key()?, req).await,
            Provider::Anthropic => {
                anthropic::chat(&self.http, &self.base_url, self.key()?, req).await
            }
            Provider::Ollama => ollama::chat(&self.http, &self.base_url, req).await,
        }
    }

    /// List models available from this provider (live query).
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        match self.provider {
            Provider::OpenAi => openai::list_models(&self.http, &self.base_url, self.key()?).await,
            Provider::Anthropic => {
                anthropic::list_models(&self.http, &self.base_url, self.key()?).await
            }
            Provider::Ollama => ollama::list_models(&self.http, &self.base_url).await,
        }
    }

    /// Verify the key/endpoint works; returns the number of models visible.
    pub async fn verify(&self) -> Result<usize, LlmError> {
        Ok(self.list_models().await?.len())
    }
}
