//! Model Adapter (harness guide §1.1, §3.1, §3.3).
//!
//! The provider boundary. **Nothing outside this module may know which provider is
//! in use** — that is the rule that makes "add a local model" a config entry and an
//! impl rather than a refactor of everything above it.
//!
//! Today there is one implementation, [`SdkExecutor`], which describes the Claude
//! Agent SDK running inside the per-project container (ADR-0024, D-07). The local
//! adapter is declared and **deliberately unimplemented**: ADR-0031 records that
//! Part 3 of the guide is profile surface until a local model is actually wired, and
//! the honest form of "not built" is a constructor that refuses, not a code path
//! that runs a local model with none of Part 3's guarantees.

use serde::Serialize;

use crate::canary::{self, CanaryError, Verdict};
use crate::profile::{Profile, Tier};

/// What a provider must tell the harness about itself.
pub trait ModelAdapter: Send + Sync {
    /// Stable name, for the trace.
    fn provider(&self) -> &str;

    /// Whether this adapter can enforce grammar/schema-constrained decoding.
    ///
    /// Required for every local tier (§3.1). A provider that cannot do it must say
    /// so here rather than accepting the flag and ignoring it.
    fn supports_constrained_decoding(&self) -> bool;

    /// Whether the adapter uses the model's own trained chat and tool-call template
    /// (§3.3). There is no legitimate `false`; the method exists so a provider that
    /// would have to invent a format has to admit it.
    fn uses_native_template(&self) -> bool;

    /// The startup canary (§3.1).
    ///
    /// Some constrained-decoding backends fail, or silently fall back, on certain
    /// quantised models or incomplete schemas. Asserting it once at startup is the
    /// difference between finding out now and finding out from a malformed tool call
    /// four hours into a research session.
    fn canary(&self) -> Result<(), AdapterError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("{provider}: constrained decoding is required for the {tier} tier but this backend does not support it")]
    NoConstrainedDecoding {
        provider: String,
        tier: &'static str,
    },
    #[error("{provider}: the startup canary failed: {detail}")]
    CanaryFailed { provider: String, detail: String },
    #[error("{provider}: no adapter is implemented for this provider yet")]
    NotImplemented { provider: String },
    #[error("{provider} does not use the model's native tool-call template; template mismatch destroys reliability faster than parameter count does (guide §3.3)")]
    NonNativeTemplate { provider: String },
}

/// Checks an adapter against the profile it is about to run.
///
/// Called at startup. The point is that an impossible combination — a local tier on
/// a backend that cannot constrain decoding — fails loudly here instead of producing
/// malformed tool calls that look like a model quality problem.
pub fn verify(adapter: &dyn ModelAdapter, profile: &Profile) -> Result<(), AdapterError> {
    if !adapter.uses_native_template() {
        return Err(AdapterError::NonNativeTemplate {
            provider: adapter.provider().to_string(),
        });
    }
    if profile.output.constrained_decoding && !adapter.supports_constrained_decoding() {
        return Err(AdapterError::NoConstrainedDecoding {
            provider: adapter.provider().to_string(),
            tier: profile.tier.as_str(),
        });
    }
    adapter.canary()
}

/// The Claude Agent SDK, running inside the per-project container (ADR-0024).
///
/// The SDK owns the observe-think-act loop, the chat template and the tool-call
/// format on its side of the container boundary. Everything the guide calls
/// harness-side — the profile, the tool budget, the policy, the context budget —
/// is enforced out here, where the agent cannot reach it.
#[derive(Debug, Clone, Serialize)]
pub struct SdkExecutor {
    pub model: String,
}

impl ModelAdapter for SdkExecutor {
    fn provider(&self) -> &str {
        "anthropic-agent-sdk"
    }

    fn supports_constrained_decoding(&self) -> bool {
        // The SDK does not expose grammar-constrained decoding, and a frontier
        // profile does not require it (§3.1 makes it a local-tier requirement).
        // Saying `false` honestly is what makes `verify` refuse a local profile on
        // this adapter instead of running one unprotected.
        false
    }

    fn uses_native_template(&self) -> bool {
        true
    }

    fn canary(&self) -> Result<(), AdapterError> {
        Ok(())
    }
}

/// A locally hosted open-weight model.
///
/// Unlike the SDK executor, this one cannot answer [`ModelAdapter::canary`] from
/// static knowledge: whether *this* backend, running *this* quantisation, actually
/// applies a grammar is a fact about a running process. So the verdict is carried
/// rather than assumed, and an executor nobody probed refuses.
///
/// That refusal is the honest form of "not checked". The alternative — defaulting to
/// `Ok` — is precisely the silent fallback the canary exists to catch, implemented in
/// our own code.
#[derive(Debug, Clone)]
pub struct LocalExecutor {
    pub provider: String,
    pub model: String,
    /// What the backend claims. Checked against [`Self::canary_verdicts`], never
    /// trusted on its own.
    pub claims_constrained_decoding: bool,
    /// One verdict per probe, from [`crate::canary`]. Empty means unprobed.
    pub canary_verdicts: Vec<Verdict>,
}

impl LocalExecutor {
    /// An executor that has not been probed. It will refuse `verify`.
    #[must_use]
    pub fn unprobed(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            claims_constrained_decoding: false,
            canary_verdicts: Vec::new(),
        }
    }

    /// An executor carrying the results of a live probe.
    #[must_use]
    pub fn probed(
        provider: impl Into<String>,
        model: impl Into<String>,
        verdicts: Vec<Verdict>,
    ) -> Self {
        let ok = !verdicts.is_empty() && verdicts.iter().all(Verdict::is_honoured);
        Self {
            provider: provider.into(),
            model: model.into(),
            // Derived from what the backend *did*, not from what it advertises. A
            // claim and a measurement cannot disagree if only one of them is stored.
            claims_constrained_decoding: ok,
            canary_verdicts: verdicts,
        }
    }
}

impl ModelAdapter for LocalExecutor {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn supports_constrained_decoding(&self) -> bool {
        self.claims_constrained_decoding
    }

    fn uses_native_template(&self) -> bool {
        // Both local backends apply the model's own chat template from its Modelfile
        // or its tokenizer config; neither invents a format.
        true
    }

    fn canary(&self) -> Result<(), AdapterError> {
        canary::verdict(&self.provider, &self.canary_verdicts).map_err(|e| match e {
            CanaryError::NotProbed { provider } => AdapterError::NotImplemented { provider },
            other => AdapterError::CanaryFailed {
                provider: self.provider.clone(),
                detail: other.to_string(),
            },
        })
    }
}

/// Picks an adapter for a profile.
pub fn for_profile(profile: &Profile) -> Box<dyn ModelAdapter> {
    if profile.tier == Tier::Frontier {
        Box::new(SdkExecutor {
            model: profile.model_id.clone(),
        })
    } else {
        // Unprobed: `for_profile` has no running backend to ask. The driver replaces
        // this with `LocalExecutor::probed` once the canary has actually run.
        Box::new(LocalExecutor::unprobed(
            profile.provider.clone(),
            profile.model_id.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::frontier_fixture;

    #[test]
    fn the_sdk_adapter_verifies_against_a_frontier_profile() {
        let p = frontier_fixture();
        verify(&*for_profile(&p), &p).unwrap();
    }

    /// The check that stops a local profile running on a backend that cannot honour
    /// it. Without this, `constrained_decoding: true` is a comment.
    #[test]
    fn a_local_profile_on_the_sdk_adapter_is_refused() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.output.constrained_decoding = true;
        let err = verify(&SdkExecutor { model: "x".into() }, &p).unwrap_err();
        assert!(matches!(err, AdapterError::NoConstrainedDecoding { .. }));
    }

    #[test]
    fn the_local_adapter_refuses_rather_than_pretending() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.provider = "ollama".into();
        p.output.constrained_decoding = false; // isolate the canary
        let err = verify(&*for_profile(&p), &p).unwrap_err();
        assert!(
            matches!(err, AdapterError::NotImplemented { .. }),
            "a stub that passed verify would run unconstrained with none of Part 3's guarantees"
        );
    }

    /// A backend that actually constrains passes, and the profile it passes for is a
    /// local one — which is the combination `verify` exists to permit.
    #[test]
    fn a_probed_backend_that_honoured_every_probe_verifies_a_local_profile() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.output.constrained_decoding = true;
        let honoured: Vec<Verdict> = crate::canary::probes()
            .into_iter()
            .map(|probe| Verdict::Honoured { probe: probe.name })
            .collect();
        verify(
            &LocalExecutor::probed("vllm", "qwen3.6-35b-a3b", honoured),
            &p,
        )
        .unwrap();
    }

    /// The failure this whole path is built around: the backend took the schema and
    /// ignored it, so the tier's guarantee is absent and startup must not continue.
    #[test]
    fn a_backend_that_ignored_the_grammar_fails_verification() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.output.constrained_decoding = true;
        let ignored = vec![Verdict::Ignored {
            probe: "enum_under_pressure".into(),
            detail: "the reply is \"purple\" but the schema allows only \"red\"".into(),
            raw_excerpt: "{\"colour\":\"purple\"}".into(),
        }];
        let err = verify(&LocalExecutor::probed("ollama", "q", ignored), &p).unwrap_err();
        assert!(
            matches!(err, AdapterError::NoConstrainedDecoding { .. }),
            "a backend measured ignoring the grammar must not claim to support it: {err}"
        );
    }

    #[test]
    fn a_non_native_template_is_refused_at_every_tier() {
        struct Bad;
        impl ModelAdapter for Bad {
            fn provider(&self) -> &str {
                "bespoke"
            }
            fn supports_constrained_decoding(&self) -> bool {
                true
            }
            fn uses_native_template(&self) -> bool {
                false
            }
            fn canary(&self) -> Result<(), AdapterError> {
                Ok(())
            }
        }
        let err = verify(&Bad, &frontier_fixture()).unwrap_err();
        assert!(matches!(err, AdapterError::NonNativeTemplate { .. }));
    }
}
