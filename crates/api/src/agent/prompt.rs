//! System-prompt assembly for the internal agent.

use serde_json::Value;

/// Build the system prompt: authoring protocol + constraints + termination
/// contract. The authoring guide is the same document the MCP server serves,
/// so both front doors teach the identical protocol.
pub fn system_prompt(constraints: &Value) -> String {
    let mut prompt = String::with_capacity(16_000);
    prompt.push_str(
        "You are a quantitative trading-strategy research agent operating inside a \
         trading platform. Your job: design a strategy for the user's goal, backtest \
         it against real historical data, read the results, and iterate until you have \
         something defensible or you run out of budget. You cannot place orders or \
         trade — you only research.\n\n\
         You have tools for listing instruments, validating/creating strategy \
         definitions, launching backtests, waiting on them, and reading results. \
         Prefer wait_for_backtest over repeated get_backtest polling — it blocks \
         server-side and costs you nothing while the simulation runs.\n\n\
         Iterate deliberately: change one thing at a time, keep slugs versioned \
         (my_strat_v1, my_strat_v2, …), and compare runs before concluding.\n\n\
         ── AUTHORING PROTOCOL ──\n\n",
    );
    prompt.push_str(mcp_server_lib::authoring_guide());

    prompt.push_str("\n\n── RUN CONSTRAINTS ──\n\n");
    let instrument = constraints.get("instrument_id").and_then(|v| v.as_str());
    let timeframe = constraints.get("timeframe").and_then(|v| v.as_str());
    let start = constraints.get("start").and_then(|v| v.as_str());
    let end = constraints.get("end").and_then(|v| v.as_str());
    let initial_balance = constraints.get("initial_balance").and_then(|v| v.as_str());

    prompt.push_str("- asset_class: crypto_spot_cex (this platform's v1 focus)\n");
    match instrument {
        Some(i) => prompt.push_str(&format!("- instrument: {i} (required — use this)\n")),
        None => prompt.push_str(
            "- instrument: your choice — call list_instruments and prefer one with deep coverage\n",
        ),
    }
    match timeframe {
        Some(t) => prompt.push_str(&format!("- timeframe: {t}\n")),
        None => prompt.push_str(
            "- timeframe: your choice — start on 1h for fast iterations, confirm finalists on finer bars\n",
        ),
    }
    if let (Some(s), Some(e)) = (start, end) {
        prompt.push_str(&format!("- backtest window: {s} → {e}\n"));
    }
    if let Some(b) = initial_balance {
        prompt.push_str(&format!("- initial balance: {b}\n"));
    }

    prompt.push_str(
        "\n── TERMINATION CONTRACT ──\n\n\
         When you are satisfied with a result (or your budget is nearly spent), stop \
         calling tools and reply with plain text that STARTS with `FINAL:` and includes \
         these lines:\n\
         FINAL: <one-paragraph summary of what you built and how it performed>\n\
         strategy: <strategy_id slug of your best strategy>\n\
         backtest_id: <uuid of the backtest run supporting your conclusion>\n\
         Then add your assessment: what works, what is fragile, what you would try next.\n",
    );
    prompt
}
