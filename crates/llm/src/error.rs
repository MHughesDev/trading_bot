//! Provider-neutral error taxonomy.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LlmError {
    /// 401/403 — bad or missing API key. Fail fast; the user must fix the key.
    #[error("authentication failed: {0}")]
    Auth(String),

    /// 429 — retry after a delay.
    #[error("rate limited")]
    RateLimited { retry_after_secs: Option<u64> },

    /// 503/529 — provider overloaded; retryable.
    #[error("provider overloaded")]
    Overloaded,

    /// Request exceeds the model's context window — caller must compact.
    #[error("context too large: {0}")]
    ContextTooLarge(String),

    /// The selected model cannot do tool calling (common on Ollama models).
    #[error("model does not support tools: {0}")]
    ToolsUnsupported(String),

    /// Other 4xx — the request itself is wrong; not retryable.
    #[error("invalid request: {0}")]
    InvalidRequest(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("unexpected response shape: {0}")]
    Deserialize(String),

    /// Uncategorized provider error.
    #[error("provider error {status}: {body}")]
    Provider { status: u16, body: String },
}

impl LlmError {
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            LlmError::RateLimited { .. } | LlmError::Overloaded | LlmError::Network(_)
        ) || matches!(self, LlmError::Provider { status, .. } if *status >= 500)
    }
}

/// Map an HTTP error status + body to the taxonomy. Providers call this after
/// handling their provider-specific 400 cases (context length, tools support).
pub(crate) fn classify_status(status: u16, body: &str, retry_after: Option<u64>) -> LlmError {
    match status {
        401 | 403 => LlmError::Auth(truncate(body)),
        429 => LlmError::RateLimited {
            retry_after_secs: retry_after,
        },
        503 | 529 => LlmError::Overloaded,
        s if s >= 500 => LlmError::Provider {
            status: s,
            body: truncate(body),
        },
        s if s >= 400 => LlmError::InvalidRequest(truncate(body)),
        s => LlmError::Provider {
            status: s,
            body: truncate(body),
        },
    }
}

pub(crate) fn truncate(s: &str) -> String {
    const MAX: usize = 2000;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}
