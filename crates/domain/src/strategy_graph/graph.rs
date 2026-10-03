//! The canonical strategy graph, `tbot.strategy/3` (FEAT-007 §5.2, ADR-0040).
//!
//! This is the strategy. The visual editor, SLv3 text and the agent's builder
//! tool-calls are views of it (ADR-0045); its canonical JSON's hash is its
//! revision. It is data only — whether a graph *means* something (types line up,
//! no cycles, models exist) is the compiler's question, in `strategy-engine`. What
//! this module refuses at deserialisation is only what no compiler could make
//! sense of: an unknown format, a graph of the wrong kind for its sink, two nodes
//! with one id.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::intent::TradeIntent;
use crate::payloads::bar::Timeframe;
use crate::strategy_def::params::ParamSpec;
use crate::strategy_def::revision::{canonical_json, RevisionHash};

/// The one format string this module accepts.
pub const FORMAT: &str = "tbot.strategy/3";

// ── kinds ────────────────────────────────────────────────────────────────────

/// The two kinds of strategy (FEAT-007 §4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    /// *What do I do with this asset, now?* Bound to one instrument per running
    /// instance; ends in trade-intent rules.
    Trading,
    /// *Which assets should we be in?* Evaluated over a universe; ends in a ranked
    /// selection.
    Selection,
}

// ── timeframes ───────────────────────────────────────────────────────────────

/// A timeframe as people write it: `1s 1m 5m 15m 1h 4h 1d`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TimeframeKey(Timeframe);

impl TimeframeKey {
    const KEYS: [(&'static str, Timeframe); 7] = [
        ("1s", Timeframe::Seconds1),
        ("1m", Timeframe::Minutes1),
        ("5m", Timeframe::Minutes5),
        ("15m", Timeframe::Minutes15),
        ("1h", Timeframe::Hours1),
        ("4h", Timeframe::Hours4),
        ("1d", Timeframe::Daily),
    ];

    #[must_use]
    pub const fn timeframe(self) -> Timeframe {
        self.0
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        Self::KEYS
            .iter()
            .find(|(_, t)| *t == self.0)
            .map_or("?", |(k, _)| k)
    }

    /// Length of one bar, in seconds.
    #[must_use]
    pub const fn seconds(self) -> u32 {
        match self.0 {
            Timeframe::Seconds1 => 1,
            Timeframe::Minutes1 => 60,
            Timeframe::Minutes5 => 300,
            Timeframe::Minutes15 => 900,
            Timeframe::Hours1 => 3_600,
            Timeframe::Hours4 => 14_400,
            Timeframe::Daily => 86_400,
        }
    }
}

impl TryFrom<String> for TimeframeKey {
    type Error = GraphError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::KEYS
            .iter()
            .find(|(k, _)| *k == s)
            .map(|(_, t)| Self(*t))
            .ok_or(GraphError::UnknownTimeframe(s))
    }
}

impl From<TimeframeKey> for String {
    fn from(t: TimeframeKey) -> Self {
        t.key().to_string()
    }
}

// ── references and arguments ─────────────────────────────────────────────────

/// What a string argument refers to.
///
/// | written | means |
/// |---|---|
/// | `$fast` | the parameter `fast` |
/// | `state.flat` | a portfolio-view field (FEAT-007 §5.4) |
/// | `vol` | the node `vol`, or a built-in source such as `close` |
/// | `vol.sigma` | field `sigma` of node `vol`'s record output |
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefPath {
    Param(String),
    State(String),
    /// A node id *or* a built-in source name; the compiler decides which,
    /// because only it knows both the node set and the source registry.
    Name {
        name: String,
        field: Option<String>,
    },
}

impl RefPath {
    /// Parse a reference.
    ///
    /// # Errors
    /// [`GraphError::BadRef`] for an empty reference, an empty segment, a
    /// parameter with a field, or more than one `.`.
    pub fn parse(s: &str) -> Result<Self, GraphError> {
        let bad = || GraphError::BadRef(s.to_string());
        if s.is_empty() {
            return Err(bad());
        }
        if let Some(p) = s.strip_prefix('$') {
            return if is_ident(p) {
                Ok(Self::Param(p.into()))
            } else {
                Err(bad())
            };
        }
        let mut parts = s.split('.');
        let head = parts.next().ok_or_else(bad)?;
        let field = parts.next();
        if parts.next().is_some() || !is_ident(head) || field.is_some_and(|f| !is_ident(f)) {
            return Err(bad());
        }
        if head == "state" {
            return field.map(|f| Self::State(f.into())).ok_or_else(bad);
        }
        Ok(Self::Name {
            name: head.into(),
            field: field.map(Into::into),
        })
    }
}

impl fmt::Display for RefPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Param(p) => write!(f, "${p}"),
            Self::State(s) => write!(f, "state.{s}"),
            Self::Name { name, field: None } => f.write_str(name),
            Self::Name {
                name,
                field: Some(x),
            } => write!(f, "{name}.{x}"),
        }
    }
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|h| h.is_ascii_alphabetic() || h == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// A string literal, written `{"str": "up"}` so that a bare string can always
/// mean a reference. Without the wrapper, `"up"` would be ambiguous between the
/// label *up* and a node named `up`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StrLit {
    pub str: String,
}

/// A node written in place of a reference, without an id of its own — e.g. a
/// model's `window` input built by `features(...)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InlineNode {
    pub op: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Arg>,
}

/// One argument of a node, a rule or an intent field.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Bool(bool),
    Num(f64),
    /// A reference: a node, `node.field`, `$param`, `state.x`, or a built-in
    /// source. Kept as written; [`Arg::reference`] parses it.
    Ref(String),
    Str(StrLit),
    Inline(Box<InlineNode>),
    List(Vec<Arg>),
}

impl Arg {
    /// The reference this argument names, if it is one.
    ///
    /// # Errors
    /// When the string is not a well-formed reference.
    pub fn reference(&self) -> Result<Option<RefPath>, GraphError> {
        match self {
            Self::Ref(s) => RefPath::parse(s).map(Some),
            _ => Ok(None),
        }
    }

    /// A literal number, if this argument is one.
    #[must_use]
    pub const fn literal_num(&self) -> Option<f64> {
        match self {
            Self::Num(x) => Some(*x),
            _ => None,
        }
    }

    /// Every reference this argument mentions, recursively — inline nodes and
    /// lists included. Used to find a graph's edges.
    pub fn references(&self, out: &mut Vec<String>) {
        match self {
            Self::Ref(s) => out.push(s.clone()),
            Self::Inline(n) => n.args.values().for_each(|a| a.references(out)),
            Self::List(xs) => xs.iter().for_each(|a| a.references(out)),
            Self::Bool(_) | Self::Num(_) | Self::Str(_) => {}
        }
    }
}

// ── nodes ────────────────────────────────────────────────────────────────────

/// One named node: an operator from the registry and its argument bindings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub id: String,
    pub op: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, Arg>,
}

// ── declared data ────────────────────────────────────────────────────────────

/// Which instrument a data input reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstrumentRef {
    /// The instrument this running instance is bound to (`$instrument`).
    Bound,
    /// A named instrument, e.g. `BTC-USD` as a market factor for an alt.
    Fixed { instrument: String },
}

/// Data a strategy reads beyond its own instrument at its own timeframe.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataInput {
    /// The name nodes refer to, e.g. `btc_1d` → `btc_1d.close`.
    pub id: String,
    pub instrument: InstrumentRef,
    pub timeframe: TimeframeKey,
}

// ── the trading sink ─────────────────────────────────────────────────────────

/// One `when … then …` rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Rules are matched first-wins *within a group*. Separate groups let an
    /// entry rule and a protection rule both act on one tick.
    #[serde(default = "default_group", skip_serializing_if = "is_default_group")]
    pub group: String,
    /// A `bool` value: a reference or an inline node.
    pub when: Arg,
    pub then: TradeIntent,
}

fn default_group() -> String {
    "main".into()
}
fn is_default_group(g: &String) -> bool {
    g == "main"
}

/// A trading strategy's decision: ordered rules and what to do when none match.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decide {
    pub rules: Vec<Rule>,
    /// Only `hold` today; a field rather than a constant so the canonical form
    /// says it rather than leaving a reader to know the default.
    #[serde(default)]
    pub otherwise: Otherwise,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Otherwise {
    #[default]
    Hold,
}

// ── the selection sink ───────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// A selection strategy's output.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Select {
    /// A `num` per instrument; ranked by it.
    pub score: Arg,
    /// A `bool` per instrument; an instrument for which it is not `true` is
    /// excluded, and the exclusion is reported with its reason.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub filter: Option<Arg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_n: Option<u32>,
    #[serde(default)]
    pub order: SortOrder,
    /// Named values carried with each selected instrument — e.g. `weight`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Arg>,
}

/// Which instruments a selection strategy considers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UniverseRef {
    /// A named universe with point-in-time membership (`dataplane::universe`).
    Named { name: String },
    /// An explicit list.
    List { instruments: Vec<String> },
    /// Every initialised instrument of an asset class.
    AssetClass { asset_class: String },
}

// ── triggers and risk ────────────────────────────────────────────────────────

/// When a strategy is evaluated (ADR-0042). Never every tick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Triggers {
    /// On the close of every bar of the decision timeframe.
    #[serde(default = "yes")]
    pub bar_close: bool,
    /// When one of this strategy's own orders fills or changes state.
    #[serde(default)]
    pub on_fill: bool,
    /// A cron expression, evaluated against the exchange calendar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
}

impl Default for Triggers {
    fn default() -> Self {
        Self {
            bar_close: true,
            on_fill: false,
            schedule: None,
        }
    }
}

const fn yes() -> bool {
    true
}

/// Per-strategy limits. **Tighten-only**: the risk gate applies the stricter of
/// these and the global limits (ADR-0005), so nothing here can loosen anything.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimits {
    /// Largest position, as a fraction of equity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_position_fraction: Option<Decimal>,
    /// Largest position, in the quote currency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_notional: Option<Decimal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_orders_per_min: Option<u32>,
    /// Daily loss, as a fraction of start-of-day equity, that halts the strategy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_daily_loss_fraction: Option<Decimal>,
}

// ── the graph ────────────────────────────────────────────────────────────────

/// A strategy, in canonical form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RawGraph", into = "RawGraph")]
pub struct StrategyGraph {
    pub kind: StrategyKind,
    pub name: String,
    pub description: Option<String>,
    pub asset_class: String,
    pub timeframe: TimeframeKey,
    pub params: BTreeMap<String, ParamSpec>,
    pub constraints: Vec<String>,
    pub inputs: Vec<DataInput>,
    pub nodes: Vec<NodeDef>,
    pub sink: Sink,
    pub triggers: Triggers,
    pub risk: RiskLimits,
}

/// How the graph ends. One per kind, so a trading graph *cannot* carry a
/// selection sink — the type has no such state.
#[derive(Clone, Debug, PartialEq)]
pub enum Sink {
    Decide(Decide),
    Select {
        select: Select,
        universe: UniverseRef,
    },
}

impl StrategyGraph {
    /// The content address of this graph: SHA-256 of its canonical JSON.
    ///
    /// # Errors
    /// When the graph cannot be serialised at all.
    pub fn revision(&self) -> Result<RevisionHash, serde_json::Error> {
        RevisionHash::of_json(&serde_json::to_value(self)?)
    }

    /// Canonical JSON: sorted keys, no whitespace, array order preserved.
    ///
    /// # Errors
    /// When the graph cannot be serialised.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        Ok(canonical_json(&serde_json::to_value(self)?))
    }

    #[must_use]
    pub fn node(&self, id: &str) -> Option<&NodeDef> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// A graph from its wire JSON, telling a document that is not shaped like a
    /// graph at all apart from one that is shaped right and breaks a
    /// structural rule — so a validator can name the rule (Set U U-7.2).
    ///
    /// # Errors
    /// [`GraphParseError::Malformed`] with serde's account of the shape, or
    /// [`GraphParseError::Structural`] with the rule broken.
    pub fn from_json(v: &serde_json::Value) -> Result<Self, GraphParseError> {
        let raw: RawGraph = serde_json::from_value(v.clone())
            .map_err(|e| GraphParseError::Malformed(e.to_string()))?;
        Self::try_from(raw).map_err(GraphParseError::Structural)
    }
}

/// Why a JSON document is not a strategy graph.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphParseError {
    /// Not shaped like a graph: a missing field, a wrong type, an unknown key.
    #[error("{0}")]
    Malformed(String),
    /// Shaped like a graph, but breaks a structural rule.
    #[error("{0}")]
    Structural(GraphError),
}

impl GraphError {
    /// A stable code for each rule — what a validator reports.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Format(_) => "wrong_format",
            Self::SinkMismatch { .. } => "sink_mismatch",
            Self::SelectionWithoutUniverse => "selection_without_universe",
            Self::DuplicateNode(_) => "duplicate_node_id",
            Self::BadNodeId(_) => "bad_node_id",
            Self::BadParam(_) => "bad_param_name",
            Self::BadRef(_) => "bad_ref",
            Self::UnknownTimeframe(_) => "unknown_timeframe",
        }
    }
}

/// The wire form. Kept separate so that the invariants below are checked on the
/// way in, and so that the serialised form names `decide`/`select` exactly as a
/// person writes them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGraph {
    format: String,
    kind: StrategyKind,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    asset_class: String,
    timeframe: TimeframeKey,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    params: BTreeMap<String, ParamSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    inputs: Vec<DataInput>,
    nodes: Vec<NodeDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decide: Option<Decide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    select: Option<Select>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    universe: Option<UniverseRef>,
    #[serde(default)]
    triggers: Triggers,
    #[serde(default)]
    risk: RiskLimits,
}

impl TryFrom<RawGraph> for StrategyGraph {
    type Error = GraphError;

    fn try_from(r: RawGraph) -> Result<Self, Self::Error> {
        if r.format != FORMAT {
            return Err(GraphError::Format(r.format));
        }
        let sink = match (r.kind, r.decide, r.select, r.universe) {
            (StrategyKind::Trading, Some(d), None, None) => Sink::Decide(d),
            (StrategyKind::Selection, None, Some(s), Some(u)) => Sink::Select {
                select: s,
                universe: u,
            },
            (StrategyKind::Selection, None, Some(_), None) => {
                return Err(GraphError::SelectionWithoutUniverse)
            }
            (kind, d, s, u) => {
                return Err(GraphError::SinkMismatch {
                    kind,
                    has_decide: d.is_some(),
                    has_select: s.is_some(),
                    has_universe: u.is_some(),
                })
            }
        };
        let mut seen = BTreeSet::new();
        for n in &r.nodes {
            if !is_ident(&n.id) {
                return Err(GraphError::BadNodeId(n.id.clone()));
            }
            if !seen.insert(n.id.as_str()) {
                return Err(GraphError::DuplicateNode(n.id.clone()));
            }
        }
        for i in &r.inputs {
            if !seen.insert(i.id.as_str()) {
                return Err(GraphError::DuplicateNode(i.id.clone()));
            }
        }
        for p in r.params.keys() {
            if !is_ident(p) {
                return Err(GraphError::BadParam(p.clone()));
            }
        }
        Ok(Self {
            kind: r.kind,
            name: r.name,
            description: r.description,
            asset_class: r.asset_class,
            timeframe: r.timeframe,
            params: r.params,
            constraints: r.constraints,
            inputs: r.inputs,
            nodes: r.nodes,
            sink,
            triggers: r.triggers,
            risk: r.risk,
        })
    }
}

impl From<StrategyGraph> for RawGraph {
    fn from(g: StrategyGraph) -> Self {
        let (decide, select, universe) = match g.sink {
            Sink::Decide(d) => (Some(d), None, None),
            Sink::Select { select, universe } => (None, Some(select), Some(universe)),
        };
        Self {
            format: FORMAT.into(),
            kind: g.kind,
            name: g.name,
            description: g.description,
            asset_class: g.asset_class,
            timeframe: g.timeframe,
            params: g.params,
            constraints: g.constraints,
            inputs: g.inputs,
            nodes: g.nodes,
            decide,
            select,
            universe,
            triggers: g.triggers,
            risk: g.risk,
        }
    }
}

/// Why a document is not a strategy graph. Structural only; meaning is the
/// compiler's.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    #[error("format '{0}' is not {FORMAT}; a v1 definition must be translated (FEAT-007 §12)")]
    Format(String),
    #[error("a {kind:?} strategy must end in exactly one sink — trading → `decide`, selection → `select` + `universe` (has decide: {has_decide}, select: {has_select}, universe: {has_universe})")]
    SinkMismatch {
        kind: StrategyKind,
        has_decide: bool,
        has_select: bool,
        has_universe: bool,
    },
    #[error("a selection strategy must say which instruments it considers: add a `universe`")]
    SelectionWithoutUniverse,
    #[error("two nodes (or a node and an input) share the id '{0}'")]
    DuplicateNode(String),
    #[error("'{0}' is not a valid node id: letters, digits and _ only, not starting with a digit")]
    BadNodeId(String),
    #[error("'{0}' is not a valid parameter name")]
    BadParam(String),
    #[error("'{0}' is not a reference: write `node`, `node.field`, `$param` or `state.field`")]
    BadRef(String),
    #[error("'{0}' is not a timeframe; use one of 1s 1m 5m 15m 1h 4h 1d")]
    UnknownTimeframe(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FEAT-007 §5.2's trading example, in canonical form.
    pub(crate) fn trading_example() -> serde_json::Value {
        serde_json::json!({
          "format": "tbot.strategy/3",
          "kind": "trading",
          "name": "trend_with_vol_gate",
          "asset_class": "crypto_spot_cex",
          "timeframe": "1h",
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
        })
    }

    pub(crate) fn selection_example() -> serde_json::Value {
        serde_json::json!({
          "format": "tbot.strategy/3",
          "kind": "selection",
          "name": "momentum_top10",
          "asset_class": "crypto_spot_cex",
          "timeframe": "1d",
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
        })
    }

    #[test]
    fn both_examples_parse_and_round_trip_byte_identically() {
        for doc in [trading_example(), selection_example()] {
            let g: StrategyGraph = serde_json::from_value(doc.clone()).expect("parses");
            let once = g.canonical_json().unwrap();
            let back: StrategyGraph = serde_json::from_str(&once).unwrap();
            assert_eq!(back, g);
            assert_eq!(
                back.canonical_json().unwrap(),
                once,
                "canonical form is a fixpoint"
            );
        }
    }

    #[test]
    fn the_revision_is_stable_and_changes_with_any_byte_of_meaning() {
        let g: StrategyGraph = serde_json::from_value(trading_example()).unwrap();
        let a = g.revision().unwrap();
        assert_eq!(a, g.clone().revision().unwrap());
        let mut h = g;
        h.nodes[0].args.insert("n".into(), Arg::Num(13.0));
        assert_ne!(a, h.revision().unwrap());
    }

    #[test]
    fn key_order_in_the_input_does_not_change_the_revision() {
        let g1: StrategyGraph = serde_json::from_value(trading_example()).unwrap();
        // Re-serialise through a map with keys in a different order.
        let text = serde_json::to_string(&trading_example()).unwrap();
        let g2: StrategyGraph = serde_json::from_str(&text).unwrap();
        assert_eq!(g1.revision().unwrap(), g2.revision().unwrap());
    }

    #[test]
    fn a_trading_graph_with_a_select_sink_is_refused_by_name() {
        let mut doc = trading_example();
        doc["select"] = serde_json::json!({"score": "f"});
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly one sink"), "{err}");
    }

    #[test]
    fn a_graph_with_neither_sink_is_refused() {
        let mut doc = trading_example();
        doc.as_object_mut().unwrap().remove("decide");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly one sink"), "{err}");
    }

    #[test]
    fn a_selection_without_a_universe_is_refused_with_the_fix() {
        let mut doc = selection_example();
        doc.as_object_mut().unwrap().remove("universe");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("add a `universe`"), "{err}");
    }

    #[test]
    fn duplicate_node_ids_are_refused() {
        let mut doc = trading_example();
        doc["nodes"][1]["id"] = serde_json::json!("f");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("share the id 'f'"), "{err}");
    }

    #[test]
    fn a_node_id_colliding_with_a_data_input_is_refused() {
        let mut doc = trading_example();
        doc["nodes"][0]["id"] = serde_json::json!("btc_1d");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("btc_1d"), "{err}");
    }

    #[test]
    fn a_v1_document_is_refused_with_the_way_forward() {
        let mut doc = trading_example();
        doc["format"] = serde_json::json!("1.0");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("must be translated"), "{err}");
    }

    #[test]
    fn an_unknown_timeframe_is_refused_listing_the_known_ones() {
        let mut doc = trading_example();
        doc["timeframe"] = serde_json::json!("2h");
        let err = serde_json::from_value::<StrategyGraph>(doc)
            .unwrap_err()
            .to_string();
        assert!(err.contains("1s 1m 5m 15m 1h 4h 1d"), "{err}");
    }

    #[test]
    fn references_parse_into_what_they_name() {
        assert_eq!(
            RefPath::parse("$fast").unwrap(),
            RefPath::Param("fast".into())
        );
        assert_eq!(
            RefPath::parse("state.flat").unwrap(),
            RefPath::State("flat".into())
        );
        assert_eq!(
            RefPath::parse("vol.sigma").unwrap(),
            RefPath::Name {
                name: "vol".into(),
                field: Some("sigma".into())
            }
        );
        for bad in ["", "$", "$a.b", "a.b.c", "state", "1x", "a..b", "a b"] {
            assert!(RefPath::parse(bad).is_err(), "'{bad}' must be refused");
        }
    }

    #[test]
    fn a_bare_string_is_always_a_reference_and_a_literal_must_say_so() {
        let a: Arg = serde_json::from_str(r#""up""#).unwrap();
        assert_eq!(a, Arg::Ref("up".into()));
        let s: Arg = serde_json::from_str(r#"{"str":"up"}"#).unwrap();
        assert_eq!(s, Arg::Str(StrLit { str: "up".into() }));
        let n: Arg = serde_json::from_str("0.03").unwrap();
        assert_eq!(n.literal_num(), Some(0.03));
    }

    #[test]
    fn references_are_found_through_inline_nodes_and_lists() {
        let g: StrategyGraph = serde_json::from_value(trading_example()).unwrap();
        let mut refs = Vec::new();
        for n in &g.nodes {
            n.args.values().for_each(|a| a.references(&mut refs));
        }
        for want in ["close", "$fast", "f", "s", "vol.sigma", "state.flat"] {
            assert!(refs.iter().any(|r| r == want), "missing {want} in {refs:?}");
        }
    }

    #[test]
    fn unknown_fields_are_refused_rather_than_dropped() {
        // A misspelt field silently dropped is a strategy that does not do what
        // its author wrote.
        let mut doc = trading_example();
        doc["nodes"][0]["argz"] = serde_json::json!({});
        assert!(serde_json::from_value::<StrategyGraph>(doc).is_err());
        let mut doc = trading_example();
        doc["risk"]["max_positon_fraction"] = serde_json::json!("0.1");
        assert!(serde_json::from_value::<StrategyGraph>(doc).is_err());
    }
}
