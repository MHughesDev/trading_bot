//! LLM answers as typed values (Set U U-5.6; ADR-0043; FEAT-007 §5.3).
//!
//! An LLM adapter's contract declares its outputs like any model's —
//! `sentiment: num`, `stance: cat`, `reasons: text` — and the adapter is asked
//! to reply with a JSON object holding them. [`parse_reply`] turns the reply
//! into exactly those typed outputs or into `missing(model_abstained)` naming
//! what was wrong: a reply that is not JSON, lacks a field, or has a field of
//! the wrong type never becomes a default value a strategy then trades on.
//!
//! Cost is capped twice, per call and per day ([`LlmCaps`]). A call whose
//! estimated cost would break either cap is not made: the model abstains with
//! the cap that stopped it, which the decision journal records like any other
//! reason a rule held.

use std::sync::Mutex;

use chrono::{NaiveDate, Utc};
use domain::model_def::contract::IoContract;
use rust_decimal::Decimal;
use strategy_engine::ModelOutcome;

use super::typed_value;
use crate::sidecar::{LlmPredictRequest, LlmPredictResponse};

/// The LLM call, behind a trait so it can be tested without a provider.
#[async_trait::async_trait]
pub trait LlmClient: Send + Sync {
    async fn predict_llm(&self, req: LlmPredictRequest) -> anyhow::Result<LlmPredictResponse>;

    /// An embedding version's vectors (U-5.2). A client that cannot embed
    /// says so; the version then answers `unavailable` naming why.
    async fn embed(&self, _req: EmbedRequest) -> anyhow::Result<EmbedResponse> {
        anyhow::bail!("this model client does not embed")
    }
}

#[async_trait::async_trait]
impl LlmClient for crate::sidecar::SidecarClient {
    async fn predict_llm(&self, req: LlmPredictRequest) -> anyhow::Result<LlmPredictResponse> {
        Self::predict_llm(self, req).await
    }

    async fn embed(&self, req: EmbedRequest) -> anyhow::Result<EmbedResponse> {
        static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
        let http = HTTP.get_or_init(|| {
            reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .connect_timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("http client")
        });
        // The inference service's address, as `SidecarClient::from_env` reads it.
        let base = std::env::var("MODEL_INFERENCE_URL")
            .unwrap_or_else(|_| "http://localhost:8002".to_string());
        let resp = http
            .post(format!("{}/v2/embed", base.trim_end_matches('/')))
            .json(&req)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("inference embed unreachable: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("inference embed returned {status}: {body}");
        }
        Ok(resp.json::<EmbedResponse>().await?)
    }
}

/// `POST /v2/embed` (`apps/model-inference/app/predictors/embed_predictor.py`):
/// the version's adapter and the texts to embed.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmbedRequest {
    pub model_id: String,
    pub version: i32,
    pub adapter: serde_json::Value,
    pub rows: Vec<EmbedRow>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmbedRow {
    pub key: String,
    pub text: String,
}

/// One answer per text: `embedding` in `outputs`, or why not.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EmbedResponse {
    pub model_id: String,
    pub version: i32,
    pub answers: Vec<crate::sidecar::V2Answer>,
}

/// Spending limits for LLM calls, in USD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LlmCaps {
    pub per_call_usd: Decimal,
    pub per_day_usd: Decimal,
}

impl Default for LlmCaps {
    fn default() -> Self {
        Self {
            per_call_usd: Decimal::new(10, 2), // $0.10
            per_day_usd: Decimal::from(10),    // $10.00
        }
    }
}

/// What has been spent today (UTC).
#[derive(Debug, Default)]
pub struct Spend(Mutex<Option<(NaiveDate, Decimal)>>);

impl Spend {
    /// Spent so far today.
    #[must_use]
    pub fn today(&self) -> Decimal {
        let today = Utc::now().date_naive();
        self.0
            .lock()
            .ok()
            .and_then(|g| *g)
            .filter(|(d, _)| *d == today)
            .map_or(Decimal::ZERO, |(_, s)| s)
    }

    pub fn add(&self, usd: Decimal) {
        let today = Utc::now().date_naive();
        if let Ok(mut g) = self.0.lock() {
            let so_far = g
                .filter(|(d, _)| *d == today)
                .map_or(Decimal::ZERO, |(_, s)| s);
            *g = Some((today, so_far + usd));
        }
    }
}

/// The JSON object inside a reply — bare, or fenced as ```json … ```.
fn json_object(text: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(&text[start..=end])
        .ok()?
        .as_object()
        .cloned()
}

/// An LLM's reply as the contract's typed outputs, or an abstain naming why
/// not. Strict: every declared output must be present and of its type.
#[must_use]
pub fn parse_reply(text: &str, contract: &IoContract) -> ModelOutcome {
    let Some(obj) = json_object(text) else {
        let shown: String = text.chars().take(80).collect();
        return ModelOutcome::Abstained {
            detail: format!("the reply is not a JSON object: {shown:?}"),
        };
    };
    let mut outputs = std::collections::BTreeMap::new();
    for out in &contract.outputs {
        let Some(v) = obj.get(&out.name) else {
            return ModelOutcome::Abstained {
                detail: format!("the reply has no `{}`", out.name),
            };
        };
        match typed_value(&out.name, v, &out.value_type) {
            Ok(v) => {
                outputs.insert(out.name.clone(), v);
            }
            Err(detail) => return ModelOutcome::Abstained { detail },
        }
    }
    ModelOutcome::Answered { outputs }
}

/// The estimated cost of one call: the adapter's price per 1k tokens times
/// the call's token budget (`max_tokens`, 1 000 when unset).
#[must_use]
pub fn estimate_usd(adapter: &serde_json::Value, params: &serde_json::Value) -> Decimal {
    let per_1k: Decimal = adapter
        .get("cost_per_1k_tokens")
        .and_then(|c| {
            c.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| c.as_f64().and_then(Decimal::from_f64_retain))
        })
        .unwrap_or(Decimal::ZERO);
    let tokens = params
        .get("max_tokens")
        .or_else(|| {
            adapter
                .get("default_params")
                .and_then(|p| p.get("max_tokens"))
        })
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1_000);
    per_1k * Decimal::from(tokens) / Decimal::from(1_000)
}

/// The cap a call would break, if any.
#[must_use]
pub fn cap_breached(caps: &LlmCaps, spend: &Spend, estimate: Decimal) -> Option<String> {
    if estimate > caps.per_call_usd {
        return Some(format!(
            "cost cap: the call is estimated at ${estimate}, over the ${} per-call cap",
            caps.per_call_usd
        ));
    }
    let today = spend.today();
    if today + estimate > caps.per_day_usd {
        return Some(format!(
            "cost cap: ${today} spent today; ${estimate} more would pass the ${} daily cap",
            caps.per_day_usd
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::model_def::contract::{ContractOutput, CostClass, PitSpec};
    use domain::strategy_graph::{Value, ValueType};

    fn contract() -> IoContract {
        IoContract {
            inputs: vec![],
            outputs: vec![
                ContractOutput {
                    name: "sentiment".into(),
                    value_type: ValueType::Num,
                },
                ContractOutput {
                    name: "stance".into(),
                    value_type: ValueType::Cat {
                        labels: vec!["bear".into(), "bull".into()],
                    },
                },
            ],
            horizon: None,
            pit: PitSpec::default(),
            cost_class: CostClass::Expensive,
            timeout_ms: 2_000,
        }
    }

    #[test]
    fn a_reply_matching_the_schema_is_a_typed_record_fenced_or_not() {
        for text in [
            r#"{"sentiment": 0.7, "stance": "bull"}"#,
            "Sure!\n```json\n{\"sentiment\": 0.7, \"stance\": \"bull\", \"note\": \"extra is fine\"}\n```",
        ] {
            let ModelOutcome::Answered { outputs } = parse_reply(text, &contract()) else { panic!("{text}") };
            assert!(matches!(outputs["sentiment"], Value::Num(n) if (n.get() - 0.7).abs() < 1e-12));
            assert_eq!(outputs["stance"], Value::Cat("bull".into()));
        }
    }

    #[test]
    fn a_reply_that_does_not_match_is_missing_never_a_default() {
        for (text, names) in [
            ("I think the market looks bullish.", "not a JSON object"),
            (r#"{"stance": "bull"}"#, "sentiment"),
            (r#"{"sentiment": "high", "stance": "bull"}"#, "sentiment"),
        ] {
            let out = parse_reply(text, &contract());
            assert!(
                matches!(&out, ModelOutcome::Abstained { detail } if detail.contains(names)),
                "{text}: {out:?}"
            );
        }
    }

    #[test]
    fn caps_refuse_before_overspending() {
        let caps = LlmCaps {
            per_call_usd: Decimal::new(5, 2),
            per_day_usd: Decimal::new(10, 2),
        };
        let spend = Spend::default();
        let adapter = serde_json::json!({"cost_per_1k_tokens": "0.03"});
        let est = estimate_usd(&adapter, &serde_json::json!({"max_tokens": 1000}));
        assert_eq!(est, Decimal::new(3, 2));
        assert!(cap_breached(&caps, &spend, est).is_none());
        for _ in 0..3 {
            spend.add(est);
        }
        let why = cap_breached(&caps, &spend, est).expect("a fourth call would pass $0.10");
        assert!(why.contains("daily cap"), "{why}");
        let big = estimate_usd(&adapter, &serde_json::json!({"max_tokens": 4000}));
        assert!(cap_breached(&caps, &Spend::default(), big)
            .unwrap()
            .contains("per-call"));
    }
}
