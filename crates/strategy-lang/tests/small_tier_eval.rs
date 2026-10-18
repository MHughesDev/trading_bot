//! A 7B writes a strategy through the clause grammar (S-5.9; AGENT-011 §8.7, AC-7).
//!
//! The acceptance: *"on `qwen2.5-7b`, a research goal produces a compiled SLv2
//! strategy in ≤ 6 fills with no `unknown_feature` error."*
//!
//! The mechanical half is proved elsewhere (`form.rs`: a strategy is four to six
//! fills, and the schema cannot name a feature the engine does not serve). This
//! is the half that needs a model, and it is **ignored by default** because it
//! needs one: a local Ollama with `qwen2.5:7b-instruct`.
//!
//! ```text
//! cargo test -p strategy-lang --test small_tier_eval -- --ignored --nocapture
//! ```
//!
//! `SLV2_EVAL_URL` (default `http://127.0.0.1:11434`) and `SLV2_EVAL_MODEL`
//! (default `qwen2.5:7b-instruct`) override the target.
//!
//! # What it does
//!
//! For each goal below it makes one constrained call per clause — header, entry,
//! size, two exits — each under that clause's JSON Schema with every `$ref`
//! inlined, so the sampler can only emit what the form accepts. The fills are
//! assembled into a [`FormStrategy`] and compiled **through the language** (the
//! form has no compiler of its own). It reports fills, the compile result and
//! every diagnostic.
//!
//! # What the dataset is
//!
//! Eight research goals written for this test, one sentence each, covering the
//! four skeleton families and shorts. It is **not** AGENT-009's research suite:
//! those tasks run a whole agent loop against planted-edge data, which needs the
//! harness, not a grammar. A fill count and an `unknown_feature` count over
//! eight goals is evidence about the grammar, not about research quality.
//!
//! The shell-out is to `curl` so the crate takes no HTTP dependency for one
//! ignored test.

use std::process::Command;

use serde_json::{json, Value};
use strategy_lang::form::{self, Entry, Exit, FormStrategy, Header, NonEmpty, Size};

const GOALS: &[&str] = &[
    "Buy when price has dropped sharply below its recent average and is likely to snap back; take a small profit or leave after a day.",
    "Follow a trend: be long while a short moving average is above a long one, with a protective stop.",
    "Buy a breakout above the highest high of the last 20 bars when volume is well above normal.",
    "Trade only in calm, trending markets: enter when volatility is low and the move is efficient.",
    "Go short when the market is overbought on RSI, and leave if it goes against you by 3 percent.",
    "Buy an oversold RSI reading, with a stop loss and a time limit on the trade.",
    "Buy when MACD turns positive and use a trailing stop of 3 percent.",
    "Enter long when price is above its 50-bar average and 10-bar momentum is positive.",
];

fn url() -> String {
    std::env::var("SLV2_EVAL_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into())
}

fn model() -> String {
    std::env::var("SLV2_EVAL_MODEL").unwrap_or_else(|_| "qwen2.5:7b-instruct".into())
}

/// The compact schema by default; `SLV2_EVAL_EXACT=1` uses the per-family one
/// (exact arity and range, about 270 KB for a whole document) so the two can be
/// compared on the same goals.
fn clause_schema(name: &str) -> Value {
    let exact = std::env::var_os("SLV2_EVAL_EXACT").is_some();
    (if exact {
        form::clause_schema_inlined(name)
    } else {
        form::clause_schema_compact(name)
    })
    .unwrap_or_else(|| panic!("no clause schema `{name}`"))
}

/// One constrained call. Returns the parsed JSON the model emitted.
fn fill(system: &str, user: &str, schema: &Value, tag: &str) -> Result<Value, String> {
    let body = json!({
        "model": model(),
        "stream": false,
        "format": schema,
        "options": {"temperature": 0.0, "seed": 7, "num_predict": 600},
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let dir = env!("CARGO_TARGET_TMPDIR");
    let path = format!("{dir}/slv2_eval_{tag}.json");
    std::fs::write(&path, body.to_string()).map_err(|e| e.to_string())?;
    let out = Command::new("curl")
        .args(["-s", "-m", "300", "-X", "POST"])
        .arg(format!("{}/api/chat", url()))
        .args(["-H", "Content-Type: application/json", "--data-binary"])
        .arg(format!("@{path}"))
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    let reply: Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("not JSON: {e}: {}", String::from_utf8_lossy(&out.stdout)))?;
    if let Some(err) = reply.get("error") {
        return Err(format!("ollama: {err}"));
    }
    let content = reply["message"]["content"]
        .as_str()
        .ok_or("no message.content")?;
    serde_json::from_str(content).map_err(|e| format!("fill is not JSON: {e}: {content}"))
}

fn families() -> String {
    strategy_lang::Catalogue::engine()
        .iter()
        .map(|f| f.name.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

const SYSTEM: &str = "You write one clause of a trading strategy as JSON that matches the schema you are constrained to. Be conservative: use common, simple choices. There are no declared inputs and no declared parameters, so never use an `input` or `param` operand. A filter tests something on its left against something on its right: the left is a feature or a bar field, the right is a number or another feature. Example filter, for an oversold market: {\"filter\":\"compare\",\"lhs\":{\"operand\":\"feature\",\"family\":\"rsi\",\"args\":[14]},\"op\":\"lt\",\"rhs\":{\"operand\":\"number\",\"value\":\"30\"}}. Feature arguments are whole numbers of bars. Stop, target and trailing distances are positive percentages such as \"3\". Features available: ";

struct Outcome {
    goal: &'static str,
    fills: usize,
    error: Option<String>,
    diagnostics: Vec<(String, String)>,
    text: String,
}

fn run_goal(i: usize, goal: &'static str) -> Outcome {
    let system = format!("{SYSTEM}{}.", families());
    let mut fills = 0;
    let mut say = |tag: &str, user: String, schema: &str| -> Result<Value, String> {
        fills += 1;
        fill(
            &system,
            &user,
            &clause_schema(schema),
            &format!("{i}_{tag}"),
        )
    };

    let ctx = format!("Goal: {goal}\n");
    let attempt = (|| -> Result<FormStrategy, String> {
        let header: Header = serde_json::from_value(say(
            "header",
            format!("{ctx}Fill the header: a snake_case name for this strategy, instrument \"$bound_at_init\", and a timeframe such as 1h."),
            "header",
        )?)
        .map_err(|e| e.to_string())?;
        let entry: Entry = serde_json::from_value(say(
            "entry",
            format!("{ctx}Fill the entry: the side and the filters that must hold to enter. Each filter compares two operands."),
            "entry",
        )?)
        .map_err(|e| e.to_string())?;
        let size: Size = serde_json::from_value(say(
            "size",
            format!("{ctx}Fill the position size."),
            "size",
        )?)
        .map_err(|e| e.to_string())?;
        let exit1: Exit = serde_json::from_value(say(
            "exit1",
            format!(
                "{ctx}Fill the first exit, a protective one such as a stop or a trailing stop."
            ),
            "exit",
        )?)
        .map_err(|e| e.to_string())?;
        let exit2: Exit = serde_json::from_value(say(
            "exit2",
            format!("{ctx}Fill a second exit: a profit target or a holding time in bars."),
            "exit",
        )?)
        .map_err(|e| e.to_string())?;
        Ok(FormStrategy {
            header,
            params: vec![],
            constraints: vec![],
            inputs: vec![],
            entries: NonEmpty::new(entry, vec![]),
            size,
            exits: NonEmpty::new(exit1, vec![exit2]),
            risk: vec![],
        })
    })();

    match attempt {
        Err(e) => Outcome {
            goal,
            fills,
            error: Some(e),
            diagnostics: vec![],
            text: String::new(),
        },
        Ok(form) => {
            let text = form.to_text();
            match form.compile() {
                Ok(_) => Outcome {
                    goal,
                    fills,
                    error: None,
                    diagnostics: vec![],
                    text,
                },
                Err(ds) => Outcome {
                    goal,
                    fills,
                    error: None,
                    diagnostics: ds
                        .iter()
                        .map(|d| (d.code.as_str().to_string(), d.message.clone()))
                        .collect(),
                    text,
                },
            }
        }
    }
}

#[test]
#[ignore = "needs a local Ollama with qwen2.5:7b-instruct; see the module docs"]
fn a_7b_writes_a_compiled_strategy_in_at_most_six_fills_with_no_unknown_feature() {
    let mut compiled = 0;
    let mut unknown = 0;
    let mut over_budget = 0;
    let mut report = String::new();
    for (i, goal) in GOALS.iter().enumerate() {
        let o = run_goal(i, goal);
        let ok = o.error.is_none() && o.diagnostics.is_empty();
        compiled += usize::from(ok);
        unknown += o
            .diagnostics
            .iter()
            .filter(|(c, _)| c == "unknown_feature")
            .count();
        over_budget += usize::from(o.fills > 6);
        report.push_str(&format!(
            "\n--- goal {i}: {}\nfills: {}  compiled: {ok}\n{}\n{:?}\n{:?}\n",
            o.goal, o.fills, o.text, o.error, o.diagnostics
        ));
    }
    println!("{report}");
    println!(
        "RESULT model={} goals={} compiled={compiled} unknown_feature={unknown} over_6_fills={over_budget}",
        model(),
        GOALS.len()
    );
    assert_eq!(over_budget, 0, "a strategy took more than 6 fills");
    assert_eq!(unknown, 0, "the grammar let an unknown feature through");
    // The compile rate is reported, not asserted: the acceptance names fills and
    // `unknown_feature`. Measured 2026-10-03 on qwen2.5:7b-instruct: 3 of 8 with
    // the compact schema (every failure `wrong_arity`, which that schema gives up
    // by design), 5 of 8 with the exact per-family schema (SLV2_EVAL_EXACT=1; two
    // of the three failures were the sampler giving up on its 175 KB grammar).
    assert!(compiled > 0, "no goal compiled at all");
}
