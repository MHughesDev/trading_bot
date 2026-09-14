//! Tool Registry (harness guide §2, §1.1).
//!
//! The catalogue may be any size; what is budgeted is **simultaneous exposure**.
//! This platform has 56 tools. Exposing all 56 on every call is the single largest
//! reliability defect available to an agent harness — the guide calls it F1,
//! tool-count collapse, and it degrades frontier models too, just later.
//!
//! Three things live here that did not exist before:
//!
//! 1. **A namespace per tool**, so routing has something to route on.
//! 2. **A risk per tool** (`read | write | destructive | outbound`), so the
//!    permission policy has something to key on. Before this, "does this tool need
//!    approval" was a question with no data behind it.
//! 3. **A canonical schema plus an automatic flattener**, so a local profile gets a
//!    flat schema for free rather than by someone hand-maintaining a second
//!    catalogue that drifts.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::profile::{Profile, Routing, SchemaStyle};

/// What a tool can do to the world (guide §2.3, §14.3).
///
/// Ordered by severity so a policy can compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Observes. Re-running it changes nothing.
    Read,
    /// Creates or modifies state that can be corrected afterwards.
    Write,
    /// Removes or overwrites something that cannot be recovered from the platform.
    Destructive,
    /// Sends data somewhere the platform does not control. The leg the trifecta
    /// rule is usually cut at (§14.1).
    Outbound,
}

impl Risk {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Read => "read",
            Risk::Write => "write",
            Risk::Destructive => "destructive",
            Risk::Outbound => "outbound",
        }
    }
}

/// One tool, described once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    /// `verb_noun`, unambiguous (§2.3).
    pub name: String,
    /// Dotted namespace: `data`, `jobs`, `research`, `fs`, `core`.
    pub namespace: String,
    /// ≤ 2 sentences saying WHEN to use it, not just what it does.
    pub description: String,
    pub risk: Risk,
    /// Always exposed, regardless of routing. Counts against the budget (§2.1).
    #[serde(default)]
    pub core: bool,
    /// Whether re-running with identical arguments is a no-op.
    #[serde(default)]
    pub idempotent: bool,
    /// The rich JSON Schema. The flat form is derived, never authored.
    pub input_schema: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("tool {0:?} is not in the catalogue")]
    Unknown(String),
    #[error("tool {name:?} is invalid: {detail}")]
    Invalid { name: String, detail: String },
    #[error("the catalogue has {catalogue} tools but the profile forbids routing and caps exposure at {budget}; enable dynamic routing or raise the budget")]
    CatalogueExceedsBudget { catalogue: usize, budget: usize },
}

/// The complete tool catalogue plus the rules for exposing a slice of it.
#[derive(Debug, Clone, Default)]
pub struct ToolRegistry {
    tools: BTreeMap<String, ToolDef>,
}

/// What one step may see.
#[derive(Debug, Clone)]
pub struct Exposure {
    /// Schemas in the profile's style, ready to hand to the adapter.
    pub schemas: Vec<Value>,
    /// The names exposed, for the validation ladder's name check (§3.2 step 2).
    pub names: BTreeSet<String>,
    /// Namespaces the router chose, for the trace.
    pub namespaces: Vec<String>,
}

/// The escape hatch (§2.2 step 4).
///
/// Without it a routing miss is fatal: the model needs a tool, cannot see it, and
/// has no way to say so. With it, a routing miss costs one step.
pub const SEARCH_TOOLS: &str = "search_tools";

/// Typed termination (guide §5.1).
///
/// The harness decides when a task is done, and it decides it from a validated tool
/// call with checkable evidence — never from a string prefix in prose. A model that
/// can end a task by writing the right words can end it by accident.
pub const FINISH_TASK: &str = "finish_task";

/// Writing down something established, so a long task can outlive its transcript.
///
/// The gap this closes has two halves that turn out to be the same hole.
///
/// **A plan step that needs no tool had no legal move.** The planner emits reasoning
/// steps — "count the number of instruments", "name two from the list" — and a step
/// could only advance by calling a tool, so the model reached for the nearest
/// topically-related one. Measured: a run that had its answer at step 1 spent the
/// other eleven steps re-reading data it already held, then reported a fabrication.
///
/// **What was established had nowhere durable to live.** Findings survived only as
/// raw tool results in `WorkingState`, which is exactly the section truncation eats
/// first under budget pressure. On a short task that is invisible. On a long one the
/// thing proved at step 2 is gone by step 20, and the agent either redoes it or
/// invents it.
///
/// A recorded finding is high-priority context, so it outlives the result it came
/// from; it is what `finish_task` can honestly cite; and it is the signal the loop
/// uses to tell progress from spinning.
pub const RECORD_FINDING: &str = "record_finding";

/// The agent's own folder (guide §4.3, §10.1).
///
/// Every conversation gets a workspace and these four tools over it. They are
/// namespaced `fs` and executed against `harness::workspace::Workspace`, which
/// canonicalises every path lexically and refuses anything that escapes the root —
/// so an agent cannot reach another agent's notes or the host filesystem, whatever
/// it writes in the argument.
///
/// Registered by [`ToolRegistry::with_workspace`] rather than [`ToolRegistry::with_core`]
/// because a harness with no workspace mounted must not advertise them.
pub const FS_TOOLS: &[&str] = &["write_file", "read_file", "list_files", "delete_file"];

impl ToolRegistry {
    /// An empty registry.
    ///
    /// Prefer [`ToolRegistry::with_core`] unless you are deliberately testing the
    /// empty case: a registry without the escape hatch will hand the model a
    /// repair instruction naming a tool it cannot call.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with the core primitives the loop itself depends on.
    ///
    /// `search_tools` has to be here, not merely named in a constant. The
    /// validation ladder's `tool_not_exposed` rejection tells the model "Call
    /// search_tools to request another" — so if nothing registers it, the harness's
    /// own repair instruction points at a tool that does not exist, and a routing
    /// miss becomes an unrecoverable loop instead of a one-turn detour. That was
    /// true of this code until it was caught by a review.
    #[must_use]
    pub fn with_core() -> Self {
        let mut reg = Self::default();
        reg.register(ToolDef {
            name: SEARCH_TOOLS.to_string(),
            namespace: "core".into(),
            description: "Find a tool that is not available this step. Use it when the                           tool you need was not offered."
                .into(),
            risk: Risk::Read,
            core: true,
            idempotent: true,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "What the tool should do, in a few words."
                    }
                },
                "required": ["query"]
            }),
        })
        .expect("the core escape hatch is well-formed");
        reg.register(ToolDef {
            name: RECORD_FINDING.to_string(),
            namespace: "core".into(),
            description: "Write down one thing you have established, with what showed                           it. Use it when a step is answered by what you already have                           rather than by another tool call."
                .into(),
            risk: Risk::Read,
            core: true,
            idempotent: false,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "finding": {
                        "type": "string",
                        "description": "One thing you now know, stated plainly."
                    },
                    "evidence": {
                        "type": "string",
                        "description": "Which tool call showed it, and what it returned."
                    }
                },
                "required": ["finding", "evidence"]
            }),
        })
        .expect("the core finding recorder is well-formed");
        reg.register(ToolDef {
            name: FINISH_TASK.to_string(),
            namespace: "core".into(),
            description: "End the task and report the result with its evidence. Use it                           when the acceptance criteria are met, or when they cannot be."
                .into(),
            risk: Risk::Write,
            core: true,
            idempotent: false,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "result": {"type": "string", "description": "The answer."},
                    "evidence": {
                        "type": "string",
                        "description": "Handles or ids supporting it."
                    }
                },
                "required": ["result", "evidence"]
            }),
        })
        .expect("the core termination tool is well-formed");
        reg
    }

    /// [`Self::with_core`] plus the agent's own filesystem.
    ///
    /// The paths are workspace-relative and must start with an area: `work/`,
    /// `memory/`, `outputs/`, `inbox/` or `logs/`. That is stated in each
    /// description rather than left to be discovered by rejection, because a model
    /// that learns the rule from three failed calls has spent three steps on it.
    #[must_use]
    pub fn with_workspace() -> Self {
        let mut reg = Self::with_core();
        let path_arg = |what: &str| {
            serde_json::json!({
                "type": "string",
                "description": format!(
                    "{what} Workspace-relative, starting with an area: work/, memory/, \
                     outputs/, logs/ or inbox/. For example work/notes.md."
                )
            })
        };

        reg.register(ToolDef {
            name: "write_file".into(),
            namespace: "fs".into(),
            description: "Write a file in your own workspace, creating or replacing it. \
                          Use it to keep notes, working data and anything you want to \
                          re-read later."
                .into(),
            // A write, not destructive: replacing your own scratch note is not the
            // same as deleting a study nobody can rebuild.
            risk: Risk::Write,
            core: false,
            idempotent: false,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": path_arg("Where to write it."),
                    "content": {"type": "string", "description": "The full file contents."}
                },
                "required": ["path", "content"]
            }),
        })
        .expect("write_file is well-formed");

        reg.register(ToolDef {
            name: "read_file".into(),
            namespace: "fs".into(),
            description: "Read a file back from your own workspace. Use it to recover \
                          detail that has been compacted out of context."
                .into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"path": path_arg("Which file to read.")},
                "required": ["path"]
            }),
        })
        .expect("read_file is well-formed");

        reg.register(ToolDef {
            name: "list_files".into(),
            namespace: "fs".into(),
            description: "List what is in your workspace. Use it when you are not sure \
                          what you saved earlier."
                .into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory to list, e.g. work/. Omit for the whole workspace."
                    }
                }
            }),
        })
        .expect("list_files is well-formed");

        reg.register(ToolDef {
            name: "delete_file".into(),
            namespace: "fs".into(),
            // Destructive, and therefore an approval at every tier — but only ever
            // over the agent's own folder, so the blast radius is its own notes.
            description: "Delete a file from your own workspace.".into(),
            risk: Risk::Destructive,
            core: false,
            idempotent: false,
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"path": path_arg("Which file to delete.")},
                "required": ["path"]
            }),
        })
        .expect("delete_file is well-formed");

        reg
    }

    /// Adds a tool, checking the design rules that can be checked mechanically.
    pub fn register(&mut self, tool: ToolDef) -> Result<(), RegistryError> {
        let bad = |detail: &str| RegistryError::Invalid {
            name: tool.name.clone(),
            detail: detail.to_string(),
        };
        if tool.name.trim().is_empty() {
            return Err(bad("a tool needs a name"));
        }
        if tool.namespace.trim().is_empty() {
            return Err(bad(
                "every tool declares a namespace so routing has something to route on",
            ));
        }
        if tool.description.trim().is_empty() {
            return Err(bad(
                "a description is model-facing UX: say WHEN to use this tool (guide §2.3)",
            ));
        }
        if !tool.input_schema.is_object() {
            return Err(bad("input_schema must be a JSON Schema object"));
        }
        if self.tools.contains_key(&tool.name) {
            return Err(bad("duplicate tool name"));
        }
        self.tools.insert(tool.name.clone(), tool);
        Ok(())
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ToolDef> {
        self.tools.get(name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    #[must_use]
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Every namespace, with a one-line summary. Cheap enough to keep in the charter
    /// so the model knows what exists without seeing 56 schemas (§2.2 step 1).
    #[must_use]
    pub fn namespace_index(&self) -> BTreeMap<String, usize> {
        let mut out: BTreeMap<String, usize> = BTreeMap::new();
        for t in self.tools.values() {
            *out.entry(t.namespace.clone()).or_insert(0) += 1;
        }
        out
    }

    /// Tools at or below a risk ceiling. Used to build a least-privilege grant for a
    /// sub-agent (§13.3).
    #[must_use]
    pub fn at_or_below(&self, ceiling: Risk) -> Vec<&ToolDef> {
        self.tools.values().filter(|t| t.risk <= ceiling).collect()
    }

    /// Selects and renders the tools for one step.
    ///
    /// `namespaces` is what the router chose — deterministically from the plan step
    /// where possible (§2.2 step 2a). Core tools are always included and **count
    /// against the budget**, because the budget is about what the model sees, not
    /// about which half of it we consider optional.
    pub fn expose(
        &self,
        profile: &Profile,
        namespaces: &[String],
    ) -> Result<Exposure, RegistryError> {
        self.expose_pinned(profile, namespaces, &[])
    }

    /// [`Self::expose`], with specific tools kept ahead of namespace filling.
    ///
    /// `pinned` is for tools the model **named** — the ones `search_tools` returned.
    /// Namespace priority alone is not enough: a search that finds `list_instruments`
    /// puts `discovery` at the front, and then `get_instrument` takes the slot because
    /// it sorts earlier inside that namespace. That happened on a live run, and the
    /// model went and did something else with the tools it could see.
    ///
    /// Pinned tools still count against the budget. The budget is about what the model
    /// sees, and an exception that quietly widened it would make the number a
    /// suggestion.
    pub fn expose_pinned(
        &self,
        profile: &Profile,
        namespaces: &[String],
        pinned: &[String],
    ) -> Result<Exposure, RegistryError> {
        let budget = profile.tools.max_exposed_per_step;

        // `routing: none` is a claim that the catalogue fits. Check it rather than
        // trusting it: a catalogue grows, and the day it outgrows the budget the
        // failure should be a startup error, not a quietly truncated tool list.
        if matches!(profile.tools.routing, Routing::None) && self.tools.len() > budget {
            return Err(RegistryError::CatalogueExceedsBudget {
                catalogue: self.tools.len(),
                budget,
            });
        }

        let mut selected: Vec<&ToolDef> = Vec::new();

        // Core first: they are the ones whose absence breaks the loop itself.
        for t in self.tools.values().filter(|t| t.core) {
            selected.push(t);
        }

        // Then anything the model explicitly asked for, before namespace filling.
        for name in pinned {
            if let Some(t) = self.tools.get(name) {
                if !t.core && !selected.iter().any(|s| s.name == t.name) {
                    selected.push(t);
                }
            }
        }

        // Then by namespace, **in the order the router asked for them**.
        //
        // The order is the whole point, and it was not always here. Selecting in
        // catalogue order and truncating at the budget is deterministic but not
        // sensible: it drops tools alphabetically, so a tool the model explicitly
        // searched for could lose its slot to an unrelated tool whose name sorts
        // earlier. That was observed live — a 7B correctly called `search_tools`,
        // the registry found `list_instruments`, and the budget then hid it behind
        // `create_strategy`. The model went and created a strategy instead, which
        // reads as the model being stupid and was the harness overruling it.
        //
        // Within a namespace, alphabetical: still deterministic, so the same step
        // always sees the same tools and a failure stays reproducible.
        if matches!(profile.tools.routing, Routing::None) {
            for t in self.tools.values().filter(|t| !t.core) {
                selected.push(t);
            }
        } else {
            let mut seen: BTreeSet<&str> = BTreeSet::new();
            for ns in namespaces {
                if !seen.insert(ns.as_str()) {
                    continue;
                }
                for t in self
                    .tools
                    .values()
                    .filter(|t| !t.core && t.namespace == *ns)
                {
                    if !selected.iter().any(|s| s.name == t.name) {
                        selected.push(t);
                    }
                }
            }
        }

        // Over budget: keep core, then take in the priority order established above.
        if selected.len() > budget {
            let (core, rest): (Vec<&ToolDef>, Vec<&ToolDef>) =
                selected.into_iter().partition(|t| t.core);
            let room = budget.saturating_sub(core.len());
            selected = core
                .into_iter()
                .chain(rest.into_iter().take(room))
                .collect();
        }

        let style = profile.tools.schema_style;
        let schemas = selected.iter().map(|t| render(t, style)).collect();
        let names = selected.iter().map(|t| t.name.clone()).collect();

        Ok(Exposure {
            schemas,
            names,
            namespaces: namespaces.to_vec(),
        })
    }

    /// Backs the `search_tools` escape hatch: substring match over name, namespace
    /// and description.
    ///
    /// Deliberately dumb. A router miss is already an error state; the recovery path
    /// should be the most predictable thing in the system, not a second model call
    /// that can also miss.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<&ToolDef> {
        let q = query.to_lowercase();
        let terms: Vec<&str> = q
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| t.len() > 2 && !STOPWORDS.contains(t))
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(usize, &ToolDef)> = self
            .tools
            .values()
            .filter_map(|t| {
                let name = t.name.to_lowercase();
                let ns = t.namespace.to_lowercase();
                let desc = t.description.to_lowercase();
                // A term in the NAME is stronger evidence than the same term buried
                // in prose — but only somewhat, and the first attempt at this weighted
                // it 4x, which broke the opposite way: for "run a backtest", every
                // tool NAMED *backtest* (get_, list_, compare_) crowded out `run_sweep`,
                // which is the tool that actually runs one and does not have the word
                // in its name. A noun-heavy weight finds things named after the noun,
                // not things that do the verb.
                let score: usize = terms
                    .iter()
                    .map(|term| {
                        usize::from(name.contains(term)) * 2
                            + usize::from(ns.contains(term))
                            + usize::from(desc.contains(term))
                    })
                    .sum();
                (score > 0).then_some((score, t))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        scored.into_iter().take(limit).map(|(_, t)| t).collect()
    }
}

/// Words that appear in almost every tool description and therefore discriminate
/// between none of them.
///
/// Routing scored a query by counting how many of its words appeared anywhere in a
/// tool's name, namespace or description. For "run a backtest on the strategy and
/// report the results" that is nine terms, four of which — a, on, the, and — match
/// essentially the whole catalogue, so the score was mostly noise and the alphabetical
/// tie-break decided the rest. Observed consequence: the step "run a backtest" was
/// routed to `get_authoring_guide` and `get_backtest`, and `create_backtest` — the one
/// tool that could have done it — was never offered. The agent then passed a training
/// run id to `get_backtest` and got a 404, which reads as a confused model and was a
/// routing failure.
const STOPWORDS: &[&str] = &[
    "and", "the", "for", "with", "that", "this", "from", "into", "its", "was", "are",
    "you", "your", "use", "using", "when", "then", "than", "but", "not", "all", "any",
    "one", "two", "out", "how", "what", "which", "who", "why", "can", "will", "would",
    "should", "must", "may", "have", "has", "had", "its", "it", "their", "them",
];

/// Renders one tool in the profile's schema style (§2.4).
#[must_use]
pub fn render(tool: &ToolDef, style: SchemaStyle) -> Value {
    let schema = match style {
        SchemaStyle::Rich => tool.input_schema.clone(),
        SchemaStyle::Flat => flatten(&tool.input_schema),
    };
    let description = match style {
        SchemaStyle::Rich => tool.description.clone(),
        SchemaStyle::Flat => first_line(&tool.description),
    };
    json!({
        "name": tool.name,
        "description": description,
        "namespace": tool.namespace,
        "risk": tool.risk.as_str(),
        "inputSchema": schema,
    })
}

fn first_line(s: &str) -> String {
    s.split(['\n', '.'])
        .find(|part| !part.trim().is_empty())
        .map(|part| {
            let t = part.trim();
            if t.ends_with('.') {
                t.to_string()
            } else {
                format!("{t}.")
            }
        })
        .unwrap_or_default()
}

/// How many optional parameters a flat schema may carry (§2.4).
///
/// Beyond two, defaults belong in the harness. Every optional parameter is a
/// decision the model has to make and can get wrong, and most of them have one
/// right answer anyway.
const FLAT_MAX_OPTIONAL: usize = 2;

/// Rebuilds nested arguments from a flat tier's answer.
///
/// [`flatten`] lifts `filter: {symbol}` to `filter_symbol` so a local model sees
/// primitives. Something has to put it back, or the tool underneath receives a shape
/// it does not implement — flatten shipped without an inverse, which made
/// `schema_style: flat` correct only for tools that happened to have no nested
/// object in them.
///
/// Driven by the **rich** schema, not by the flat keys: the rich schema is the
/// authority on where each value belongs, and anything the flat answer does not
/// carry is simply absent rather than guessed at.
#[must_use]
pub fn unflatten(schema: &Value, flat: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    rebuild(schema, "", flat, &mut out);
    out
}

/// Collapses `oneOf`/`anyOf` the same way [`flatten`] did, so the two agree on which
/// branch is in play.
fn first_branch(schema: &Value) -> &Value {
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .unwrap_or(schema)
}

fn rebuild(schema: &Value, prefix: &str, flat: &Map<String, Value>, out: &mut Map<String, Value>) {
    let schema = first_branch(schema);
    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    for (k, sub) in props {
        let key = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}_{k}")
        };
        let sub = first_branch(sub);
        if sub.get("type").and_then(Value::as_str) == Some("object")
            && sub.get("properties").is_some()
        {
            let mut nested = Map::new();
            rebuild(sub, &key, flat, &mut nested);
            if !nested.is_empty() {
                out.insert(k.clone(), Value::Object(nested));
            }
        } else if let Some(v) = flat.get(&key) {
            // The other half of flatten's collapse. `flatten` turns an array into a
            // `string`, so a flat tier is DECODING under `type: string` and returns
            // one — and validating that against the rich schema's `type: array`
            // produced "`backtest_ids` must be a array; got \"[]\"", three identical
            // times, then a fence. The model was doing exactly what it was told; the
            // harness was grading it against a schema it never saw.
            //
            // Nesting had this inverse and the array collapse did not, which is the
            // same omission one level down.
            if sub.get("type").and_then(Value::as_str) == Some("array") && v.is_string() {
                out.insert(k.clone(), parse_collapsed_array(v.as_str().unwrap_or(""), sub));
            } else if sub.get("type").and_then(Value::as_str) == Some("object") && v.is_string() {
                // Belt and braces. `flatten` now keeps a document as an object, so a
                // flat tier should never send a string here — but a model that has
                // seen a thousand APIs taking "definition_json" may write one anyway,
                // and parsing it is strictly better than rejecting an answer whose
                // only fault is an extra pair of quotes.
                match serde_json::from_str::<Value>(v.as_str().unwrap_or("")) {
                    Ok(parsed @ Value::Object(_)) => out.insert(k.clone(), parsed),
                    _ => out.insert(k.clone(), v.clone()),
                };
            } else {
                out.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Reads back a list a flat tier had to write as a string.
///
/// Accepts both spellings a model reasonably produces under `type: string` when the
/// description asks for a list — JSON (`["a","b"]`, `[]`) and plain
/// comma-separated (`a, b`) — because which one it picks is not something the
/// schema can pin down once the type is gone, and refusing one of them would fence a
/// session over punctuation.
///
/// Elements are coerced to `items.type` when the rich schema names one, so a list of
/// integers arrives as integers rather than as strings that look like them.
fn parse_collapsed_array(raw: &str, schema: &Value) -> Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Value::Array(Vec::new());
    }
    // JSON first: it is unambiguous when it parses, and a model that emitted it
    // meant a list rather than a single string containing brackets.
    let parts: Vec<Value> = match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Array(items)) => items,
        _ => trimmed
            .trim_start_matches('[')
            .trim_end_matches(']')
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| Value::String(p.trim_matches(|c: char| c == '\'' || c == '"').to_string()))
            .collect(),
    };

    let item_ty = schema
        .get("items")
        .and_then(|i| i.get("type"))
        .and_then(Value::as_str);
    let coerced = parts
        .into_iter()
        .map(|item| match (item_ty, item.as_str()) {
            (Some("integer"), Some(t)) => t.parse::<i64>().map(Value::from).unwrap_or(item),
            (Some("number"), Some(t)) => t.parse::<f64>().map(Value::from).unwrap_or(item),
            (Some("boolean"), Some(t)) => t.parse::<bool>().map(Value::Bool).unwrap_or(item),
            _ => item,
        })
        .collect();
    Value::Array(coerced)
}

/// Auto-flattens a rich JSON Schema for local tiers.
///
/// Code, not per-tool manual work — so a new tool gets both styles free and the two
/// cannot drift. Nested objects are lifted to `parent_child` primitives, `oneOf` and
/// `anyOf` collapse to their first branch, and descriptions shrink to one line.
#[must_use]
pub fn flatten(schema: &Value) -> Value {
    let mut props = Map::new();
    let mut required: Vec<Value> = Vec::new();
    lift(schema, "", &mut props, &mut required);

    // Trim optional parameters past the cap, keeping required ones.
    let required_names: BTreeSet<String> = required
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let mut optional_kept = 0usize;
    let trimmed: Map<String, Value> = props
        .into_iter()
        .filter(|(k, _)| {
            if required_names.contains(k) {
                true
            } else if optional_kept < FLAT_MAX_OPTIONAL {
                optional_kept += 1;
                true
            } else {
                false
            }
        })
        .collect();

    json!({
        "type": "object",
        "properties": Value::Object(trimmed),
        "required": Value::Array(required),
    })
}

fn lift(schema: &Value, prefix: &str, out: &mut Map<String, Value>, required: &mut Vec<Value>) {
    // A `oneOf`/`anyOf` is a choice a small model gets wrong; take the first branch
    // and let the semantic check catch a wrong shape rather than offering the union.
    let schema = schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .unwrap_or(schema);

    let Some(props) = schema.get("properties").and_then(Value::as_object) else {
        return;
    };
    let req: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    for (key, spec) in props {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}_{key}")
        };
        let ty = spec.get("type").and_then(Value::as_str).unwrap_or("string");

        if ty == "object" && spec.get("properties").is_some() {
            lift(spec, &name, out, required);
            continue;
        }

        // A free-form object — `type: object` with no declared `properties` — is a
        // DOCUMENT, and it survives flattening as an object rather than collapsing to
        // a string.
        //
        // The collapse below exists because a flat tier promises primitives, and for
        // a nested object with known properties that promise is kept by LIFTING it.
        // A document has nothing to lift, so collapsing it does not simplify anything
        // — it just discards the only guarantee constrained decoding could still
        // offer. Measured: `create_strategy` takes a whole strategy definition here,
        // the flat schema said `string`, and the model spent every retry emitting
        // JSON-inside-a-string that arrived malformed (`trailing characters at line 1
        // column 600`). Keeping it an object makes well-formedness a property of the
        // sampler instead of a hope about the model.
        if ty == "object" {
            let mut doc = Map::new();
            doc.insert("type".into(), json!("object"));
            if let Some(desc) = spec.get("description").and_then(Value::as_str) {
                doc.insert("description".into(), json!(first_line(desc)));
            }
            out.insert(name.clone(), Value::Object(doc));
            if req.contains(key.as_str()) {
                required.push(json!(name));
            }
            continue;
        }

        let mut flat = Map::new();
        // Arrays and objects without a shape become strings: a flat schema promises
        // primitives, and a model given `type: array` with no item type produces
        // whatever it likes.
        let flat_ty = match ty {
            "integer" | "number" | "boolean" | "string" => ty,
            _ => "string",
        };
        flat.insert("type".into(), json!(flat_ty));
        if let Some(desc) = spec.get("description").and_then(Value::as_str) {
            flat.insert("description".into(), json!(first_line(desc)));
        }
        // Say what the collapsed string should look like. Without this the model is
        // told "string" for something that is conceptually a list and has to guess
        // the spelling; `unflatten` accepts either, but asking for one costs nothing
        // and an empty list becomes expressible rather than improvised.
        if ty == "array" {
            let hint = "Comma-separated list; empty string for none.";
            let desc = match flat.get("description").and_then(Value::as_str) {
                Some(d) => format!("{d} {hint}"),
                None => hint.to_string(),
            };
            flat.insert("description".into(), json!(desc));
        }
        // Enums survive flattening: a known value set is the single most useful
        // thing a schema can tell a small model.
        if let Some(en) = spec.get("enum") {
            flat.insert("enum".into(), en.clone());
        }
        out.insert(name.clone(), Value::Object(flat));
        if req.contains(key.as_str()) {
            required.push(json!(name));
        }
    }
}

#[cfg(test)]
mod flat_array_roundtrip {
    use super::*;
    use serde_json::json;

    fn rich() -> Value {
        json!({
            "type": "object",
            "properties": {
                "backtest_ids": { "type": "array", "items": { "type": "string" },
                                  "description": "Runs to compare.
Second line." },
                "limits":       { "type": "array", "items": { "type": "integer" } },
                "name":         { "type": "string" }
            },
            "required": ["backtest_ids"]
        })
    }

    /// The fence, as a test.
    ///
    /// `flatten` sends `type: string` for an array, so the model decodes a string and
    /// returns `"[]"`. Grading that against the rich schema's `type: array` produced
    /// an identical `validation.bad_type` three times and fenced a healthy session —
    /// a failure the model could not fix, because it never saw the schema it was
    /// being judged against.
    #[test]
    fn an_empty_list_written_as_a_string_comes_back_as_an_empty_array() {
        let flat: Map<String, Value> = serde_json::from_value(json!({ "backtest_ids": "[]" }))
            .unwrap();
        let out = unflatten(&rich(), &flat);
        assert_eq!(out["backtest_ids"], json!([]));
    }

    #[test]
    fn a_json_list_survives_the_round_trip() {
        let flat: Map<String, Value> =
            serde_json::from_value(json!({ "backtest_ids": "[\"a\", \"b\"]" })).unwrap();
        assert_eq!(unflatten(&rich(), &flat)["backtest_ids"], json!(["a", "b"]));
    }

    /// The other spelling a model reasonably produces under `type: string`. Which one
    /// it picks is not something the schema can pin down once the type is gone, so
    /// refusing either would fence a session over punctuation.
    #[test]
    fn a_comma_separated_list_survives_the_round_trip() {
        let flat: Map<String, Value> =
            serde_json::from_value(json!({ "backtest_ids": "run_1, run_2" })).unwrap();
        assert_eq!(
            unflatten(&rich(), &flat)["backtest_ids"],
            json!(["run_1", "run_2"])
        );
    }

    #[test]
    fn an_empty_string_is_an_empty_list_not_a_one_element_list() {
        let flat: Map<String, Value> = serde_json::from_value(json!({ "backtest_ids": "" })).unwrap();
        assert_eq!(unflatten(&rich(), &flat)["backtest_ids"], json!([]));
    }

    #[test]
    fn elements_are_coerced_to_the_item_type_the_rich_schema_names() {
        let flat: Map<String, Value> =
            serde_json::from_value(json!({ "limits": "10, 20" })).unwrap();
        assert_eq!(unflatten(&rich(), &flat)["limits"], json!([10, 20]));
    }

    /// Only arrays are reinterpreted. A string that stayed a string must not be
    /// split on a comma it happens to contain.
    #[test]
    fn a_genuine_string_is_left_alone() {
        let flat: Map<String, Value> =
            serde_json::from_value(json!({ "name": "a, b" })).unwrap();
        assert_eq!(unflatten(&rich(), &flat)["name"], json!("a, b"));
    }

    /// A free-form document stays an object through flattening.
    ///
    /// `create_strategy` takes a whole strategy definition in one argument. Collapsed
    /// to `string`, constrained decoding protects nothing about it and a 7B emitted
    /// malformed JSON on every retry. Kept as an object, well-formedness is something
    /// the sampler cannot violate.
    #[test]
    fn a_free_form_document_is_not_collapsed_to_a_string() {
        let rich = json!({
            "type": "object",
            "properties": {
                "definition_json": { "type": "object", "description": "The definition.
More." },
                "note": { "type": "string" }
            },
            "required": ["definition_json"]
        });
        let flat = flatten(&rich);
        assert_eq!(
            flat["properties"]["definition_json"]["type"], "object",
            "a document with no properties has nothing to lift, so collapsing it only              discards the grammar"
        );
        assert_eq!(flat["properties"]["note"]["type"], "string");

        // And it round-trips untouched.
        let answer: Map<String, Value> = serde_json::from_value(json!({
            "definition_json": { "strategy_id": "s1", "nodes": [] }
        }))
        .unwrap();
        let out = unflatten(&rich, &answer);
        assert_eq!(out["definition_json"]["strategy_id"], "s1");
    }

    /// A model that writes the document as a string anyway is not punished for it.
    #[test]
    fn a_document_sent_as_a_json_string_is_still_parsed() {
        let rich = json!({
            "type": "object",
            "properties": { "definition_json": { "type": "object" } },
            "required": ["definition_json"]
        });
        let answer: Map<String, Value> = serde_json::from_value(json!({
            "definition_json": "{\"strategy_id\": \"s1\"}"
        }))
        .unwrap();
        let out = unflatten(&rich, &answer);
        assert_eq!(out["definition_json"]["strategy_id"], "s1");
    }

    /// The flat schema has to ask for the shape it can accept, or the model is told
    /// "string" for something conceptually a list and has to invent the spelling.
    #[test]
    fn the_flat_schema_says_how_to_write_the_list() {
        let flat = flatten(&rich());
        let desc = flat["properties"]["backtest_ids"]["description"]
            .as_str()
            .unwrap();
        assert!(desc.contains("Comma-separated"), "got {desc:?}");
        assert!(desc.contains("empty string for none"), "got {desc:?}");
        assert_eq!(flat["properties"]["backtest_ids"]["type"], "string");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{frontier_fixture, Tier};

    fn tool(name: &str, ns: &str, risk: Risk, core: bool) -> ToolDef {
        ToolDef {
            name: name.into(),
            namespace: ns.into(),
            description: format!("Use {name} when the step needs {ns}. Second sentence."),
            risk,
            core,
            idempotent: risk == Risk::Read,
            input_schema: json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "The id." } },
                "required": ["id"],
            }),
        }
    }

    fn registry_of(n: usize) -> ToolRegistry {
        let mut r = ToolRegistry::new();
        r.register(tool("finish_task", "core", Risk::Write, true))
            .unwrap();
        for i in 0..n {
            let ns = ["data", "jobs", "research"][i % 3];
            r.register(tool(&format!("{ns}_op_{i}"), ns, Risk::Read, false))
                .unwrap();
        }
        r
    }

    #[test]
    fn exposure_never_exceeds_the_profile_budget() {
        let reg = registry_of(50);
        let mut p = frontier_fixture();
        p.tools.max_exposed_per_step = 6;
        let e = reg
            .expose(&p, &["data".into(), "jobs".into(), "research".into()])
            .unwrap();
        assert_eq!(e.schemas.len(), 6, "the budget is a hard cap, not a target");
        assert!(
            e.names.contains("finish_task"),
            "core tools survive truncation"
        );
    }

    #[test]
    fn routing_restricts_to_the_selected_namespaces() {
        let reg = registry_of(30);
        let mut p = frontier_fixture();
        p.tools.max_exposed_per_step = 40;
        let e = reg.expose(&p, &["data".into()]).unwrap();
        for name in &e.names {
            assert!(
                name.starts_with("data_") || name == "finish_task",
                "{name} is outside the routed namespace"
            );
        }
    }

    /// `routing: none` is a claim that the catalogue fits. The day it stops being
    /// true should be a startup error, not a quietly truncated tool list.
    #[test]
    fn declaring_no_routing_with_an_oversized_catalogue_is_an_error() {
        let reg = registry_of(50);
        let mut p = frontier_fixture();
        p.tools.routing = Routing::None;
        p.tools.max_exposed_per_step = 10;
        let err = reg.expose(&p, &[]).unwrap_err().to_string();
        assert!(err.contains("51") && err.contains("routing"), "{err}");
    }

    /// The other half of the same rule. Namespace priority is not enough when the
    /// tool the model named sorts later than a sibling in the same namespace.
    #[test]
    fn a_tool_the_model_named_outranks_its_alphabetically_earlier_sibling() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.tools.parallel_calls = false;
        p.tools.max_exposed_per_step = 1;
        p.tools.routing = Routing::Dynamic;

        let mut r = ToolRegistry::new();
        for name in ["get_instrument", "list_instruments"] {
            r.register(ToolDef {
                name: name.into(),
                namespace: "discovery".into(),
                description: "A tool. Use it when the step needs it.".into(),
                risk: Risk::Read,
                core: false,
                idempotent: true,
                input_schema: json!({"type": "object", "properties": {}}),
            })
            .unwrap();
        }

        // Unpinned, the alphabetically-first one wins — deterministic, and not what
        // the model asked for.
        let e = r.expose(&p, &["discovery".into()]).unwrap();
        assert!(e.names.contains("get_instrument"));

        // Pinned, the searched-for tool keeps the slot.
        let e = r
            .expose_pinned(&p, &["discovery".into()], &["list_instruments".into()])
            .unwrap();
        assert!(
            e.names.contains("list_instruments"),
            "the tool the model searched for must survive: {:?}",
            e.names
        );
        assert_eq!(e.names.len(), 1, "pinning must not widen the budget");
    }

    /// The rule that keeps the escape hatch working under a tight budget.
    ///
    /// Observed live on the degraded tier: the model called `search_tools`, the
    /// registry found the tool it needed, and the exposure budget then hid it behind
    /// an unrelated tool that sorted earlier. The model did the wrong thing with the
    /// tools it could see, which looked like a model failure and was a harness one.
    #[test]
    fn a_namespace_the_router_asked_for_first_keeps_its_slot() {
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.tools.parallel_calls = false;
        p.tools.max_exposed_per_step = 2;
        p.tools.routing = Routing::Dynamic;

        let mut r = ToolRegistry::new();
        for (name, ns) in [("aaa_write", "strategy"), ("zzz_lookup", "discovery")] {
            r.register(ToolDef {
                name: name.into(),
                namespace: ns.into(),
                description: "A tool. Use it when the step needs it.".into(),
                risk: Risk::Read,
                core: false,
                idempotent: true,
                input_schema: json!({"type": "object", "properties": {}}),
            })
            .unwrap();
        }

        // Asked for `discovery` first: the alphabetically-later tool must survive.
        let e = r
            .expose(&p, &["discovery".into(), "strategy".into()])
            .unwrap();
        assert!(
            e.names.contains("zzz_lookup"),
            "the router's first choice lost its slot to alphabetical order: {:?}",
            e.names
        );

        // And the priority is the router's, not a fixed one.
        let e = r
            .expose(&p, &["strategy".into(), "discovery".into()])
            .unwrap();
        assert!(e.names.contains("aaa_write"), "{:?}", e.names);
    }

    #[test]
    fn truncation_is_deterministic() {
        let reg = registry_of(50);
        let mut p = frontier_fixture();
        p.tools.max_exposed_per_step = 5;
        let a = reg.expose(&p, &["data".into()]).unwrap();
        let b = reg.expose(&p, &["data".into()]).unwrap();
        assert_eq!(
            a.names, b.names,
            "a tool set that varied run to run would make a failure unreproducible"
        );
    }

    #[test]
    fn the_escape_hatch_finds_a_tool_the_router_missed() {
        let mut reg = registry_of(10);
        reg.register(ToolDef {
            name: "read_option_chain".into(),
            namespace: "data".into(),
            description: "Read an option chain for an expiry. Use when the step needs greeks."
                .into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: json!({"type": "object", "properties": {}}),
        })
        .unwrap();
        let hits = reg.search("option chain", 5);
        assert_eq!(hits[0].name, "read_option_chain");
        assert!(reg.search("", 5).is_empty());
    }

    #[test]
    fn least_privilege_grants_exclude_higher_risk_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(tool("read_bars", "data", Risk::Read, false))
            .unwrap();
        reg.register(tool("write_note", "memory", Risk::Write, false))
            .unwrap();
        reg.register(tool("delete_run", "admin", Risk::Destructive, false))
            .unwrap();
        reg.register(tool("send_report", "out", Risk::Outbound, false))
            .unwrap();

        let read_only = reg.at_or_below(Risk::Read);
        assert_eq!(read_only.len(), 1);
        assert_eq!(reg.at_or_below(Risk::Write).len(), 2);
        assert_eq!(reg.at_or_below(Risk::Outbound).len(), 4);
    }

    // ── The flattener ───────────────────────────────────────────────────────

    #[test]
    fn flattening_lifts_nested_objects_to_primitives() {
        let rich = json!({
            "type": "object",
            "properties": {
                "window": {
                    "type": "object",
                    "properties": {
                        "start": {"type": "string", "description": "RFC 3339 start.\nMore."},
                        "end": {"type": "string"}
                    },
                    "required": ["start"]
                },
                "tf": {"type": "string", "enum": ["1m", "1h"]}
            },
            "required": ["tf"]
        });
        let flat = flatten(&rich);
        let props = flat["properties"].as_object().unwrap();
        assert!(props.contains_key("window_start"), "nested keys are lifted");
        assert_eq!(props["window_start"]["type"], "string");
        assert_eq!(
            props["window_start"]["description"], "RFC 3339 start.",
            "descriptions shrink to one line"
        );
        assert_eq!(props["tf"]["enum"], json!(["1m", "1h"]), "enums survive: a known value set is the most useful thing a schema tells a small model");
        assert!(flat["properties"].as_object().unwrap().values().all(|v| {
            matches!(
                v["type"].as_str(),
                Some("string" | "integer" | "number" | "boolean")
            )
        }));
    }

    #[test]
    fn flattening_collapses_one_of_to_its_first_branch() {
        let rich = json!({
            "oneOf": [
                {"type": "object", "properties": {"by_id": {"type": "string"}}, "required": ["by_id"]},
                {"type": "object", "properties": {"by_name": {"type": "string"}}}
            ]
        });
        let flat = flatten(&rich);
        let props = flat["properties"].as_object().unwrap();
        assert!(props.contains_key("by_id"));
        assert!(
            !props.contains_key("by_name"),
            "a union is a choice a small model gets wrong"
        );
    }

    #[test]
    fn flattening_caps_optional_parameters() {
        let rich = json!({
            "type": "object",
            "properties": {
                "a": {"type": "string"}, "b": {"type": "string"}, "c": {"type": "string"},
                "d": {"type": "string"}, "e": {"type": "string"}
            },
            "required": ["a"]
        });
        let props = flatten(&rich);
        let n = props["properties"].as_object().unwrap().len();
        assert_eq!(
            n,
            1 + FLAT_MAX_OPTIONAL,
            "beyond two optionals, defaults belong in the harness"
        );
        assert!(props["properties"].as_object().unwrap().contains_key("a"));
    }

    #[test]
    fn arrays_without_an_item_type_become_strings_in_flat_mode() {
        let rich = json!({
            "type": "object",
            "properties": { "ids": {"type": "array"} },
            "required": []
        });
        let flat = flatten(&rich);
        assert_eq!(flat["properties"]["ids"]["type"], "string");
    }

    #[test]
    fn a_local_profile_gets_flat_schemas_without_a_second_catalogue() {
        let reg = registry_of(4);
        let mut p = frontier_fixture();
        p.tier = Tier::LocalMid;
        p.tools.schema_style = SchemaStyle::Flat;
        p.tools.max_exposed_per_step = 4;
        let e = reg.expose(&p, &["data".into()]).unwrap();
        for s in &e.schemas {
            assert!(s["inputSchema"]["properties"].is_object());
            assert!(
                s["risk"].is_string(),
                "risk travels with the schema so the policy can key on it"
            );
        }
    }

    #[test]
    fn a_tool_without_a_description_is_refused() {
        let mut reg = ToolRegistry::new();
        let mut t = tool("x_op", "x", Risk::Read, false);
        t.description = "  ".into();
        assert!(reg
            .register(t)
            .unwrap_err()
            .to_string()
            .contains("description"));
    }

    #[test]
    fn duplicate_names_are_refused() {
        let mut reg = ToolRegistry::new();
        reg.register(tool("dup", "x", Risk::Read, false)).unwrap();
        assert!(reg.register(tool("dup", "y", Risk::Read, false)).is_err());
    }
}

#[cfg(test)]
mod workspace_tool_tests {
    use super::*;

    #[test]
    fn the_agent_gets_a_readable_and_writable_folder() {
        let reg = ToolRegistry::with_workspace();
        for name in FS_TOOLS {
            assert!(reg.get(name).is_some(), "{name} is missing");
            assert_eq!(reg.get(name).unwrap().namespace, "fs");
        }
    }

    /// Deleting is destructive and therefore asks, at every tier. It is scoped to the
    /// agent's own folder, but "it was only my own notes" is a judgement the policy
    /// engine should not have to make.
    #[test]
    fn deleting_is_the_only_one_that_asks() {
        let reg = ToolRegistry::with_workspace();
        assert_eq!(reg.get("delete_file").unwrap().risk, Risk::Destructive);
        assert_eq!(reg.get("write_file").unwrap().risk, Risk::Write);
        assert_eq!(reg.get("read_file").unwrap().risk, Risk::Read);
        assert!(reg.get("read_file").unwrap().idempotent);
    }

    /// A harness with no workspace mounted must not advertise tools that would fail.
    #[test]
    fn a_harness_without_a_workspace_does_not_offer_them() {
        let reg = ToolRegistry::with_core();
        for name in FS_TOOLS {
            assert!(
                reg.get(name).is_none(),
                "{name} must not be in the core set"
            );
        }
    }

    /// The rule is in the description, not only in the rejection. A model that learns
    /// it from three failed calls has spent three steps on it.
    #[test]
    fn the_path_rule_is_stated_up_front() {
        let reg = ToolRegistry::with_workspace();
        let schema = &reg.get("write_file").unwrap().input_schema;
        let desc = schema["properties"]["path"]["description"]
            .as_str()
            .unwrap();
        assert!(desc.contains("work/"), "{desc}");
    }
}

#[cfg(test)]
mod core_tool_tests {
    use super::*;
    use crate::profile::frontier_fixture;
    use crate::validation::{validate, Rung};
    use serde_json::json;
    use std::collections::BTreeSet;

    /// The bug this exists to prevent: the validation ladder tells the model to call
    /// `search_tools` when it asks for an unrouted tool. If nothing registers that
    /// tool, the repair instruction names something the model cannot call, and a
    /// routing miss becomes an unrecoverable loop.
    #[test]
    fn the_escape_hatch_the_ladder_advertises_actually_exists() {
        let reg = ToolRegistry::with_core();
        let hatch = reg
            .get(SEARCH_TOOLS)
            .expect("the ladder's repair instruction must name a real tool");
        assert!(
            hatch.core,
            "the escape hatch is useless if routing can hide it"
        );
        assert_eq!(hatch.risk, Risk::Read);
    }

    /// And end to end: the rejection names it, and it is callable in the same step.
    #[test]
    fn a_routing_miss_is_recoverable_in_one_turn() {
        let mut reg = ToolRegistry::with_core();
        reg.register(ToolDef {
            name: "get_bars".into(),
            namespace: "data".into(),
            description: "Read bars. Use when the step needs price history.".into(),
            risk: Risk::Read,
            core: false,
            idempotent: true,
            input_schema: json!({"type": "object", "properties": {}}),
        })
        .unwrap();

        let profile = frontier_fixture();
        // Route to a namespace that does NOT contain get_bars.
        let exposure = reg.expose(&profile, &["research".into()]).unwrap();
        assert!(
            exposure.names.contains(SEARCH_TOOLS),
            "core survives routing"
        );
        assert!(!exposure.names.contains("get_bars"));

        // The model asks for the unrouted tool and is told what to do instead.
        let rejection = validate(
            &reg,
            &exposure.names,
            &json!({"name": "get_bars", "arguments": {}}),
        )
        .unwrap_err();
        assert_eq!(rejection.rung, Rung::Name);
        assert!(rejection.message.contains(SEARCH_TOOLS));

        // And that instruction is followable: the named tool validates this step.
        let recovered: BTreeSet<String> = exposure.names.clone();
        validate(
            &reg,
            &recovered,
            &json!({"name": SEARCH_TOOLS, "arguments": {"query": "price history"}}),
        )
        .expect("the repair instruction must be followable in the same step");
    }

    /// Termination is a validated tool call with evidence, not a prose marker.
    #[test]
    fn finishing_is_a_typed_call_with_evidence() {
        let reg = ToolRegistry::with_core();
        let finish = reg.get(FINISH_TASK).expect("finish_task is core");
        assert!(finish.core);
        let required = finish.input_schema["required"].as_array().unwrap();
        assert!(
            required.iter().any(|v| v == "evidence"),
            "a finish the harness cannot check against acceptance criteria is the model deciding it is done"
        );

        let exposed: BTreeSet<String> = [SEARCH_TOOLS.to_string(), FINISH_TASK.to_string()]
            .into_iter()
            .collect();
        // Missing evidence is refused.
        assert!(validate(
            &reg,
            &exposed,
            &json!({"name": FINISH_TASK, "arguments": {"result": "no edge"}})
        )
        .is_err());
        validate(
            &reg,
            &exposed,
            &json!({"name": FINISH_TASK, "arguments": {"result": "no edge", "evidence": "exp_1"}}),
        )
        .unwrap();
    }

    /// Core tools count against the budget (guide §2.1), so the core set and the
    /// budget are one decision, not two.
    ///
    /// This used to assert `len() == 2` and a budget of 3. Adding `record_finding`
    /// made core 3 and the assertion failed — correctly, and not merely as a stale
    /// number. At a budget of 3 the core set would have filled every slot and no work
    /// tool could ever have been exposed, which is a dead agent rather than a tight
    /// one. The rule is now stated as the property it always meant: whatever core
    /// grows to, a shipped local profile must still have room to offer a tool that
    /// does something.
    #[test]
    fn the_core_set_leaves_room_for_work_at_every_shipped_budget() {
        let reg = ToolRegistry::with_core();
        let core = reg.len();
        assert!(
            core >= 3,
            "core is the escape hatch, the finding recorder and termination"
        );

        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("config")
            .join("profiles");
        let set = crate::profile::ProfileSet::load_dir(&dir).expect("shipped profiles load");
        for id in set.ids() {
            let p = set.get(id).unwrap();
            assert!(
                p.tools.max_exposed_per_step > core,
                "{id} exposes {} tools a step and core alone is {core}; there would be no \
                 slot left for a tool that does the work",
                p.tools.max_exposed_per_step
            );
        }
    }
}
