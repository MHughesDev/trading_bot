//! SLv3 — the text view of the canonical strategy graph (Set U U-7.1;
//! FEAT-007 §10; ADR-0045).
//!
//! The graph (`tbot.strategy/3`) is the strategy; SLv3 is one of its three
//! views, beside the visual editor and builder tool-calls. Every SLv3 text is a
//! graph and every graph prints as SLv3, and the trip is exact:
//! `parse(print(g)) == g`. Layout, comments and argument order are not part of
//! the graph, so reformatting a strategy never changes its revision.
//!
//! ```text
//! strategy trend_with_vol_gate kind trading asset_class crypto_spot_cex timeframe 1h
//! param fast: int(default: 12, max: 50, min: 5, step: 1)
//! param slow: int(default: 48, max: 200, min: 20, step: 1)
//! constraint "param('fast') < param('slow')"
//!
//! let f = ema(close, $fast)
//! let s = ema(close, $slow)
//! let vol = model("vol_forecaster@12", window: features("core_v3", 64))
//! let entry = crossed_above(f, s) and vol.sigma < 0.03 and state.flat
//! let leave = crossed_below(f, s) and state.long
//!
//! when entry -> bracket(entry: target(size: equity_fraction(x: 0.5)),
//!                       stop: {amount: 2, unit: atr(n: 14)},
//!                       take_profit: {amount: 4, unit: atr(n: 14)})
//! when leave -> close
//! otherwise hold
//! ```
//!
//! SLv2 (the rest of this crate) stays the text of v1 definitions until Phase 8
//! retires them.

pub mod build;
pub mod catalogue;
pub mod explain;
pub mod lex;
pub mod parse;
pub mod print;
pub mod translate_v1;
pub mod validate;

pub use build::{build, BuildOp};
pub use parse::{parse, SyntaxError};
pub use print::print;
pub use validate::{validate, Finding, Source, Valid};

#[cfg(test)]
mod tests {
    use super::*;
    use domain::strategy_graph::StrategyGraph;

    fn graph(v: serde_json::Value) -> StrategyGraph {
        serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{e}\n{v:#}"))
    }

    /// FEAT-007 §5.2's trading example, canonical.
    fn trading_example() -> StrategyGraph {
        graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "trend_with_vol_gate",
          "asset_class": "crypto_spot_cex", "timeframe": "1h",
          "params": {
            "fast": {"type":"int","default":12,"min":5,"max":50},
            "slow": {"type":"int","default":48,"min":20,"max":200}
          },
          "constraints": ["param('fast') < param('slow')"],
          "inputs": [ {"id":"btc_1d", "instrument":{"kind":"fixed","instrument":"BTC-USD"}, "timeframe":"1d"} ],
          "nodes": [
            {"id":"f",    "op":"ema",           "args":{"x":"close", "n":"$fast"}},
            {"id":"s",    "op":"ema",           "args":{"x":"close", "n":"$slow"}},
            {"id":"vol",  "op":"model",         "args":{"ref":{"str":"vol_forecaster@12"},
                                                       "window":{"op":"features","args":{"set":{"str":"core_v3"},"lookback":64}}}},
            {"id":"up",   "op":"crossed_above", "args":{"a":"f", "b":"s"}},
            {"id":"calm", "op":"lt",            "args":{"a":"vol.sigma", "b":0.03}},
            {"id":"go",   "op":"and",           "args":{"a":"up", "b":"calm"}},
            {"id":"entry","op":"and",           "args":{"a":"go", "b":"state.flat"}},
            {"id":"down", "op":"crossed_below", "args":{"a":"f", "b":"s"}},
            {"id":"leave","op":"and",           "args":{"a":"down", "b":"state.long"}}
          ],
          "decide": {
            "rules": [
              {"when":"entry", "then":{"kind":"bracket",
                 "entry":{"kind":"target","size":{"mode":"equity_fraction","x":0.5}},
                 "stop":{"amount":2,"unit":{"unit":"atr","n":14}},
                 "take_profit":{"amount":4,"unit":{"unit":"atr","n":14}}}},
              {"when":"leave", "then":{"kind":"close"}}
            ],
            "otherwise": "hold"
          },
          "risk": {"max_position_fraction": "0.5", "max_orders_per_min": 6}
        }))
    }

    /// FEAT-007 §5.2's selection example, canonical.
    fn selection_example() -> StrategyGraph {
        graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "selection", "name": "momentum_top10",
          "asset_class": "crypto_spot_cex", "timeframe": "1d",
          "universe": {"kind":"asset_class","asset_class":"crypto_spot_cex"},
          "nodes": [
            {"id":"mom",    "op":"feature", "args":{"name":{"str":"roc_20"}}},
            {"id":"mom_z",  "op":"cs_zscore","args":{"x":"mom"}},
            {"id":"liq",    "op":"feature", "args":{"name":{"str":"rel_volume_20"}}},
            {"id":"liq_ok", "op":"gt",      "args":{"a":"liq","b":0.5}}
          ],
          "select": {"score":"mom_z", "where":"liq_ok", "top_n":10, "order":"desc",
                     "attributes": {"weight":"mom_z"}},
          "triggers": {"bar_close": false, "schedule": "55 23 * * *"}
        }))
    }

    /// Every intent, size, style, distance unit and anchor the format has.
    fn every_intent() -> StrategyGraph {
        graph(serde_json::json!({
          "format": "tbot.strategy/3", "kind": "trading", "name": "all_intents",
          "description": "every intent \"quoted\"", "asset_class": "crypto_spot_cex", "timeframe": "5m",
          "params": {
            "k": {"type":"float","default":1.5,"min":-2.0,"max":3.0},
            "side": {"type":"enum","default":"long","choices":["long","short"]}
          },
          "nodes": [
            {"id":"m", "op":"model", "args":{"ref":{"str":"clf@3"}, "x":"close"}},
            {"id":"p", "op":"prob", "args":{"p":"m.probs", "label":{"str":"up"}}},
            {"id":"z", "op":"neg", "args":{"x": 2}},
            {"id":"w", "op":"neg", "args":{"x":{"op":"add","args":{"a":"p","b":-0.25}}}},
            {"id":"c", "op":"if", "args":{"cond":{"op":"not","args":{"x":{"op":"lt","args":{"a":"p","b":0.5}}}},
                                          "then":{"op":"mul","args":{"a":{"op":"sub","args":{"a":"p","b":0.1}},"b":"$k"}},
                                          "else":0}},
            {"id":"v", "op":"weighted_mean", "args":{"xs":["p","c"], "weights":[0.25, 0.75]}},
            {"id":"g", "op":"or", "args":{"a":"state.flat","b":{"op":"and","args":{"a":true,"b":{"op":"gt","args":{"a":"v","b":"w"}}}}}}
          ],
          "decide": {"rules": [
            {"when":"g", "then":{"kind":"target","size":{"mode":"units","x":"c"},"via":{"type":"limit","price":{"op":"mul","args":{"a":"close","b":0.99}}}}},
            {"name":"o1","when":"g", "then":{"kind":"order","side":"buy","size":{"mode":"notional","x":100},
               "style":{"type":"stop_limit","trigger":101.5,"price":102},"tif":"ioc","reduce_only":true,"post_only":true,"tag":"entry-1"}},
            {"group":"protect","when":"state.long", "then":{"kind":"protect","stop":{"amount":1,"unit":{"unit":"pct"},"from":"mark"},
               "trailing":{"amount":3,"unit":{"unit":"sigma","of":"m.vol"},"from":"high_water"}}},
            {"name":"when","group":"protect","when":"g", "then":{"kind":"bracket",
               "entry":{"kind":"order","side":"sell","size":{"mode":"risk","fraction":0.01,"stop":{"amount":2,"unit":{"unit":"abs"}}}},
               "stop":{"amount":2,"unit":{"unit":"atr","n":14}},"trailing":{"amount":1,"unit":{"unit":"pct"}}}},
            {"when":"g", "then":{"kind":"add","size":{"mode":"vol_target","target":0.1,"vol":"m.vol"},"via":{"type":"trailing_stop","trail":{"amount":1,"unit":{"unit":"pct"}}}}},
            {"when":"g", "then":{"kind":"reduce","size":{"mode":"equity_fraction","x":0.25},"via":{"type":"market"}}},
            {"when":"g", "then":{"kind":"close","fraction":0.5,"via":{"type":"stop","trigger":"low"}}},
            {"when":"g", "then":{"kind":"cancel","tag":"entry-1"}},
            {"when":"g", "then":{"kind":"cancel"}},
            {"when":"g", "then":{"kind":"hold"}}
          ]},
          "triggers": {"bar_close": true, "on_fill": true},
          "risk": {"max_notional": "2500.75", "max_daily_loss_fraction": "0.02"}
        }))
    }

    fn round_trips(g: &StrategyGraph) {
        let text = print(g);
        let back = parse(&text).unwrap_or_else(|e| panic!("{e}\n---\n{text}"));
        assert_eq!(&back, g, "\n---\n{text}");
        assert_eq!(print(&back), text, "printing is a fixed point");
        assert_eq!(back.revision().unwrap(), g.revision().unwrap());
    }

    #[test]
    fn the_feat_007_examples_round_trip_through_text() {
        round_trips(&trading_example());
        round_trips(&selection_example());
    }

    #[test]
    fn every_intent_and_operator_form_round_trips() {
        round_trips(&every_intent());
    }

    #[test]
    fn the_module_example_is_the_trading_example_in_another_layout() {
        let text = r#"
strategy trend_with_vol_gate kind trading asset_class crypto_spot_cex timeframe 1h
param fast: int(default: 12, min: 5, max: 50)
param slow: int(min: 20, max: 200, default: 48)   # field order does not matter
constraint "param('fast') < param('slow')"
input btc_1d: fixed(instrument: "BTC-USD") 1d
let f = ema(close, $fast)
let s = ema(x: close, n: $slow)
let vol = model("vol_forecaster@12", window: features("core_v3", 64))
let up = crossed_above(f, s)
let calm = vol.sigma < 0.03
let go = up and calm
let entry = go and state.flat
let down = crossed_below(f, s)
let leave = down and state.long
when entry -> bracket(entry: target(size: equity_fraction(x: 0.5)),
                      stop: {amount: 2, unit: atr(n: 14)},
                      take_profit: {amount: 4, unit: atr(n: 14)})
when leave -> close
risk {max_position_fraction: "0.5", max_orders_per_min: 6}
"#;
        let g = parse(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(g, trading_example());
        // A formatting change is not a change: same graph, same revision.
        let squashed = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let again = parse(&squashed.replace("# field order does not matter", "")).unwrap();
        assert_eq!(again.revision().unwrap(), g.revision().unwrap());
    }

    #[test]
    fn the_selection_example_reads_as_a_screener() {
        let text = print(&selection_example());
        assert!(
            text.contains("universe asset_class(asset_class: crypto_spot_cex)"),
            "{text}"
        );
        assert!(
            text.contains("select mom_z where liq_ok top 10 attribute weight = mom_z"),
            "{text}"
        );
        assert!(
            text.contains("triggers {bar_close: false, on_fill: false, schedule: \"55 23 * * *\"}"),
            "{text}"
        );
        assert!(text.contains("let liq_ok = liq > 0.5"), "{text}");
    }

    #[test]
    fn mistakes_are_named_where_they_are_with_a_fix() {
        let err = |t: &str| parse(t).unwrap_err();
        let header = "strategy s kind trading asset_class c timeframe 1h\n";
        let e = err(&format!("{header}let f = emaa(close, 5)"));
        assert!(e.fix.contains("`ema`"), "{e}");
        assert_eq!(e.line, 2);
        let e = err(&format!("{header}let f = 3"));
        assert!(e.message.contains("operator"), "{e}");
        let e = err(&format!("{header}when a < b < c -> close"));
        assert!(e.message.contains("do not chain"), "{e}");
        let e = err(&format!("{header}lett f = ema(close, 5)"));
        assert!(e.fix.contains("`let`"), "{e}");
        let e = err("strategy s kind trading timeframe 1h\nlet a = ema(close, 5)");
        assert!(e.message.contains("asset_class"), "{e}");
        let e = err(&format!("{header}let a = ema(close, n: 5, 3)"));
        assert!(e.message.contains("positional"), "{e}");
        let e = err("strategy s kind selection asset_class c timeframe 1d\nlet a = ema(close, 5)");
        assert!(e.message.contains("not a strategy"), "{e}");
    }

    /// Every SLv3 block in FEAT-007 parses, compiles against the registries
    /// (models aside — none is registered here), and round-trips: the spec
    /// cannot show a language the parser does not speak.
    #[test]
    fn every_slv3_block_in_feat_007_parses_and_round_trips() {
        let spec = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/specs/FEAT-007-unified-strategy-engine.md"),
        )
        .expect("FEAT-007 is in the repository");
        let blocks: Vec<&str> = spec
            .split("```slv3")
            .skip(1)
            .map(|b| b.split("```").next().unwrap_or_default())
            .collect();
        assert!(!blocks.is_empty(), "FEAT-007 shows SLv3");
        for b in blocks {
            let g = parse(b).unwrap_or_else(|e| {
                panic!(
                    "{e}
{b}"
                )
            });
            round_trips(&g);
            assert_eq!(g.nodes.iter().filter(|n| n.op == "model").count(), 1);
        }
    }
}
