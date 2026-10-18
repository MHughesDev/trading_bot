//! Which hosted models the platform will call (owner decision, 2026-10-05).
//!
//! While the platform is in testing, each hosted provider is held to one cheap
//! baseline model, so a test cannot run up a bill on a frontier model. Local
//! backends (vLLM, Ollama) cost nothing per call and are not restricted.
//!
//! Enforced where a client is built: [`client`] hands every caller an
//! [`LlmClient`] that refuses any other model before a request leaves the
//! process and lists only the baseline. The routes also refuse to store or open
//! a conversation on another model, so the person is told at the click and not
//! by a failed run.
//!
//! To widen access later, add models to [`ALLOWED`]; nothing else changes.

use llm::{LlmClient, Provider};

/// The hosted models the platform may call, by provider. The first model for a
/// provider is its baseline: the model a conversation starts on.
pub const ALLOWED: &[(&str, &[&str])] = &[
    ("openai", &["gpt-4o-mini"]),
    ("anthropic", &["claude-sonnet-5"]),
];

/// The models `provider` may call, or `None` when it is not restricted (local).
#[must_use]
pub fn allowed_for(provider: &str) -> Option<&'static [&'static str]> {
    ALLOWED
        .iter()
        .find(|(p, _)| *p == provider)
        .map(|(_, models)| *models)
}

/// Whether `model` may be called on `provider`.
#[must_use]
pub fn is_allowed(provider: &str, model: &str) -> bool {
    allowed_for(provider).is_none_or(|m| m.contains(&model))
}

/// The model a conversation on `provider` starts on, when it is restricted.
#[must_use]
pub fn baseline(provider: &str) -> Option<&'static str> {
    allowed_for(provider).and_then(|m| m.first().copied())
}

/// `model` when it may be called on `provider`, otherwise the provider's
/// baseline. For defaults only: a model the person named explicitly is refused,
/// never silently swapped.
#[must_use]
pub fn or_baseline(provider: &str, model: String) -> String {
    if is_allowed(provider, &model) {
        model
    } else {
        baseline(provider).map_or(model, str::to_string)
    }
}

/// A client for `provider`, held to the models it may call.
#[must_use]
pub fn client(provider: Provider, api_key: Option<String>, base_url: Option<String>) -> LlmClient {
    let c = LlmClient::new(provider, api_key, base_url);
    match allowed_for(provider.as_str()) {
        Some(models) => c.with_allowed_models(models),
        None => c,
    }
}

/// The refusal a route returns for a model that is not enabled.
#[must_use]
pub fn not_enabled_message(provider: &str, model: &str) -> String {
    let allowed = allowed_for(provider).unwrap_or_default().join(", ");
    format!("`{model}` is not enabled on this platform yet; for {provider} use {allowed}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_hosted_provider_is_held_to_its_baseline() {
        assert!(is_allowed("openai", "gpt-4o-mini"));
        assert!(!is_allowed("openai", "gpt-6.1-sol"));
        assert!(is_allowed("anthropic", "claude-sonnet-5"));
        assert!(!is_allowed("anthropic", "claude-opus-5"));
        assert_eq!(baseline("openai"), Some("gpt-4o-mini"));
        assert_eq!(baseline("anthropic"), Some("claude-sonnet-5"));
    }

    #[test]
    fn local_backends_are_not_restricted() {
        assert!(is_allowed("vllm", "qwen3.6-35b-a3b"));
        assert!(is_allowed("ollama", "qwen2.5:7b-instruct"));
        assert_eq!(baseline("ollama"), None);
    }

    #[test]
    fn a_default_falls_back_to_the_baseline() {
        assert_eq!(
            or_baseline("anthropic", "claude-opus-5".into()),
            "claude-sonnet-5"
        );
        assert_eq!(or_baseline("openai", "gpt-4o-mini".into()), "gpt-4o-mini");
        assert_eq!(or_baseline("vllm", "anything".into()), "anything");
    }

    #[tokio::test]
    async fn the_client_refuses_another_model_before_any_request() {
        // No server listens on this port: a request that left the process would
        // fail as a network error, not as the refusal.
        let c = client(
            Provider::OpenAi,
            Some("sk-test".into()),
            Some("http://127.0.0.1:9".into()),
        );
        let req = llm::ChatRequest {
            model: "gpt-6.1-sol".into(),
            ..Default::default()
        };
        let err = c.chat(&req).await.unwrap_err();
        assert!(matches!(err, llm::LlmError::InvalidRequest(ref m) if m.contains("not enabled")));
    }
}
