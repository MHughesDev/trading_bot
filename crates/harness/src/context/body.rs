//! A record's content, typed, and the ladder each type shrinks down
//! (AGENT-005 §16; Set S tasks S-9.1 and S-9.3).
//!
//! # Why a type rather than a string
//!
//! Part I holds content as `String` and shrinks it with [`truncate_middle`],
//! which cuts at a byte offset and keeps the head and the tail. That is the best
//! a string can do and it is bad at exactly the content this system sends: a
//! 500-row table cut in the middle is broken JSON with no schema and no row
//! count, and five rows plus the column names plus "495 more" is *smaller* and
//! strictly more informative.
//!
//! So the loss becomes a declared step instead of an offset. Every variant below
//! names its own ladder, cheapest loss first, and [`Body::shrink`] walks one rung
//! and says which rung it walked. A record that reaches the bottom and still does
//! not fit is **dropped, not cut** — and it stays in the manifest (§17), so the
//! model is told it exists and can ask for it back.
//!
//! # The rule this module exists to make true
//!
//! *No record is ever byte-truncated.* `truncate_middle` still exists for the
//! transcript and the tool-result cap, which are not records; it is not reachable
//! from here, and `no_record_is_ever_byte_truncated` in
//! `crates/harness/tests/context_store.rs` is what keeps it that way.
//!
//! [`truncate_middle`]: crate::context::truncate_middle

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One column of a [`Body::Table`].
///
/// `salience` orders column dropping: the first rung sheds the least salient
/// columns first, so an `id` column outlives a `notes` column. Declared by the
/// producer rather than guessed here — the catalogue knows which of its columns
/// carries the answer and this module does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    /// Higher survives longer. Ids and the measured quantity are high; free text
    /// and provenance strings are low.
    pub salience: u8,
}

impl Column {
    #[must_use]
    pub fn new(name: impl Into<String>, salience: u8) -> Self {
        Self {
            name: name.into(),
            salience,
        }
    }

    /// A column nothing declared a salience for.
    ///
    /// Mid-scale rather than zero: an undeclared column is unknown, not
    /// worthless, and defaulting to zero would drop every column of a producer
    /// that had not been taught about salience yet — which is a silent
    /// regression on exactly the tools nobody has looked at.
    #[must_use]
    pub fn plain(name: impl Into<String>) -> Self {
        Self::new(name, 128)
    }
}

/// What a shrink step did, for the `degraded {from, to, how}` marker (§16).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rung {
    /// Table: the least salient columns are gone.
    DroppedColumns,
    /// Table: head, tail and the extreme rows of the salient column.
    SampledRows,
    /// Table: column-wise summary statistics instead of rows.
    Aggregated,
    /// Table: the schema and the row count, and nothing else.
    SchemaOnly,
    /// Series: every n-th point.
    Decimated,
    /// Series: min, max, mean, last, and n.
    Summarised,
    /// Stat: the `detail` field is gone.
    DroppedDetail,
    /// Stat: the number and its ids alone.
    NumberOnly,
    /// Text: first and last paragraph.
    Elided,
    /// Text: the first sentence.
    FirstSentence,
    /// Text: the title alone.
    TitleOnly,
    /// Json: keys below the declared salience are pruned.
    PrunedKeys,
    /// Json: the required keys alone.
    RequiredKeys,
    /// Json: the ids alone.
    IdsOnly,
}

impl Rung {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Rung::DroppedColumns => "dropped_columns",
            Rung::SampledRows => "sampled_rows",
            Rung::Aggregated => "aggregated",
            Rung::SchemaOnly => "schema_only",
            Rung::Decimated => "decimated",
            Rung::Summarised => "summarised",
            Rung::DroppedDetail => "dropped_detail",
            Rung::NumberOnly => "number_only",
            Rung::Elided => "elided",
            Rung::FirstSentence => "first_sentence",
            Rung::TitleOnly => "title_only",
            Rung::PrunedKeys => "pruned_keys",
            Rung::RequiredKeys => "required_keys",
            Rung::IdsOnly => "ids_only",
        }
    }
}

/// Typed content, with a declared shrink ladder per variant (§16).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Body {
    /// Rows under a schema. Rendered delimited, never as JSON (§25.1).
    Table {
        schema: Vec<Column>,
        rows: Vec<Vec<String>>,
        /// Rows the producer had, which stays true across every rung — a sampled
        /// table that forgot how many rows it came from is a table that lies
        /// about its own n.
        total_rows: usize,
    },
    /// An ordered numeric series.
    Series {
        name: String,
        points: Vec<f64>,
        /// 1 until decimation; then the gap between kept points, so a reader
        /// knows the x-axis is no longer unit-spaced.
        stride: u32,
        total_points: usize,
    },
    /// The catalogue's own `ToolResult` shape (AGENT-006 §2.4), which is already
    /// this ladder — so most of the catalogue gets semantic degradation free.
    Stat {
        summary: String,
        ids: BTreeMap<String, String>,
        n: Option<u64>,
        se: Option<f64>,
        detail: Option<String>,
    },
    /// Prose.
    Text {
        title: Option<String>,
        body: String,
    },
    /// A body that has been spilled to the workspace (§26.3).
    ///
    /// The *metadata* stays resident — subject, ids, kind, `tokens_full` — so
    /// scoring and the manifest still see the whole store. Only the bytes leave,
    /// and `expand` reads them back. A store that dropped the metadata too would
    /// be a store that forgot what it had, which is the one thing §14 forbids.
    Elsewhere {
        handle: String,
        /// What it cost before it left, so the manifest can still say.
        tokens: u32,
        /// One line about what it is, so a reader can decide whether to ask.
        summary: String,
    },
    /// Anything else, with the keys that must survive named.
    Json {
        value: serde_json::Value,
        /// Keys that survive to the bottom rung. A producer that names none gets
        /// the id-like keys, which is a guess and is marked as one in
        /// [`Body::shrink`].
        required: Vec<String>,
    },
}

/// Rows kept by [`Rung::SampledRows`]: three from the head, three from the tail.
const SAMPLE_HEAD: usize = 3;
const SAMPLE_TAIL: usize = 3;
/// Points kept by [`Rung::Decimated`] before the ladder gives up on points.
const DECIMATE_TARGET: usize = 24;

impl Body {
    /// Plain prose with no title.
    #[must_use]
    pub fn text(body: impl Into<String>) -> Self {
        Body::Text {
            title: None,
            body: body.into(),
        }
    }

    /// The rendered form, as it would appear in the prompt.
    ///
    /// Tables and series render delimited rather than as JSON: §25.1 measured
    /// ~40% of a 200-row result's tokens going to repeated keys, braces and
    /// quotes, and none of those bytes carry anything the model cannot infer
    /// from the header line.
    #[must_use]
    pub fn render(&self) -> String {
        match self {
            Body::Table {
                schema,
                rows,
                total_rows,
            } => {
                let mut out = String::new();
                out.push_str(
                    &schema
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                for r in rows {
                    out.push('\n');
                    out.push_str(&r.join(" | "));
                }
                if *total_rows > rows.len() {
                    out.push_str(&format!(
                        "\n({} of {total_rows} rows)",
                        rows.len()
                    ));
                }
                out
            }
            Body::Series {
                name,
                points,
                stride,
                total_points,
            } => {
                let mut head = format!("{name} ({total_points} points");
                if *stride > 1 {
                    head.push_str(&format!(", every {stride}"));
                }
                head.push_str(")\n");
                head.push_str(
                    &points
                        .iter()
                        .map(|p| format_sig(*p, 4))
                        .collect::<Vec<_>>()
                        .join(", "),
                );
                head
            }
            Body::Stat {
                summary,
                ids,
                n,
                se,
                detail,
            } => {
                let mut out = summary.clone();
                if let Some(n) = n {
                    out.push_str(&format!("\nn = {n}"));
                }
                if let Some(se) = se {
                    out.push_str(&format!("\nse = {}", format_sig(*se, 2)));
                }
                for (k, v) in ids {
                    out.push_str(&format!("\n{k} = {v}"));
                }
                if let Some(d) = detail {
                    out.push('\n');
                    out.push_str(d);
                }
                out
            }
            Body::Text { title, body } => match title {
                Some(t) if !body.is_empty() => format!("{t}\n{body}"),
                Some(t) => t.clone(),
                None => body.clone(),
            },
            Body::Elsewhere {
                handle,
                tokens,
                summary,
            } => format!("{summary} [{tokens} tokens, held in {handle}; expand to read it]"),
            Body::Json { value, .. } => {
                serde_json::to_string(value).unwrap_or_else(|_| "{}".into())
            }
        }
    }

    /// Estimated tokens of the rendered form.
    #[must_use]
    pub fn tokens(&self) -> u32 {
        super::estimate_tokens(&self.render())
    }

    /// One rung down the ladder, or `None` at the bottom.
    ///
    /// Returning `None` rather than an empty body is the whole contract: the
    /// caller's next move is to **drop** the record and leave its manifest line,
    /// not to cut it. See `degrade::fit`.
    #[must_use]
    pub fn shrink(&self) -> Option<(Body, Rung)> {
        match self {
            Body::Table {
                schema,
                rows,
                total_rows,
            } => shrink_table(schema, rows, *total_rows),
            Body::Series {
                name,
                points,
                stride,
                total_points,
            } => shrink_series(name, points, *stride, *total_points),
            Body::Stat {
                summary,
                ids,
                n,
                se,
                detail,
            } => {
                if detail.is_some() {
                    return Some((
                        Body::Stat {
                            summary: summary.clone(),
                            ids: ids.clone(),
                            n: *n,
                            se: *se,
                            detail: None,
                        },
                        Rung::DroppedDetail,
                    ));
                }
                // The number and its ids. `summary` is the sentence the tool
                // wrote about its own result; the first line of it is the
                // number in context, and the rest is the context.
                let first = summary.lines().next().unwrap_or("").to_string();
                if first != *summary || !ids.is_empty() && n.is_some() {
                    return Some((
                        Body::Stat {
                            summary: first,
                            ids: ids.clone(),
                            n: None,
                            se: *se,
                            detail: None,
                        },
                        Rung::NumberOnly,
                    ));
                }
                None
            }
            Body::Text { title, body } => shrink_text(title.as_deref(), body),
            // Already one line. There is nothing left to shed, and a rung that
            // returned a shorter line would be cutting the handle.
            Body::Elsewhere { .. } => None,
            Body::Json { value, required } => shrink_json(value, required),
        }
    }
}

fn shrink_table(
    schema: &[Column],
    rows: &[Vec<String>],
    total_rows: usize,
) -> Option<(Body, Rung)> {
    // Rung 1: shed the least salient columns, but never the last two — a table
    // of one column is a series and should have been declared one.
    let lowest = schema.iter().map(|c| c.salience).min();
    if let Some(low) = lowest {
        let keep: Vec<usize> = schema
            .iter()
            .enumerate()
            .filter(|(_, c)| c.salience > low)
            .map(|(i, _)| i)
            .collect();
        if keep.len() >= 2 && keep.len() < schema.len() {
            return Some((
                Body::Table {
                    schema: keep.iter().map(|i| schema[*i].clone()).collect(),
                    rows: rows
                        .iter()
                        .map(|r| keep.iter().filter_map(|i| r.get(*i).cloned()).collect())
                        .collect(),
                    total_rows,
                },
                Rung::DroppedColumns,
            ));
        }
    }
    // Rung 2: head and tail. `total_rows` is untouched, so the render still
    // says how many rows the producer had.
    if rows.len() > SAMPLE_HEAD + SAMPLE_TAIL {
        let mut sampled: Vec<Vec<String>> = rows[..SAMPLE_HEAD].to_vec();
        sampled.extend_from_slice(&rows[rows.len() - SAMPLE_TAIL..]);
        return Some((
            Body::Table {
                schema: schema.to_vec(),
                rows: sampled,
                total_rows,
            },
            Rung::SampledRows,
        ));
    }
    // Rung 3: column-wise statistics for the numeric columns.
    if !rows.is_empty() {
        let stats = aggregate(schema, rows);
        if !stats.is_empty() {
            return Some((
                Body::Stat {
                    summary: format!("{total_rows} rows, aggregated"),
                    ids: stats,
                    n: Some(total_rows as u64),
                    se: None,
                    detail: None,
                },
                Rung::Aggregated,
            ));
        }
    }
    // Rung 4: the schema and the count.
    if !rows.is_empty() {
        return Some((
            Body::Table {
                schema: schema.to_vec(),
                rows: vec![],
                total_rows,
            },
            Rung::SchemaOnly,
        ));
    }
    None
}

/// Column-wise min/mean/max for the columns that parse as numbers.
///
/// Non-numeric columns are skipped rather than counted: an aggregate of a text
/// column is a distinct-count at best, and reporting one here would put a number
/// beside a column name that does not mean what a reader assumes it means.
fn aggregate(schema: &[Column], rows: &[Vec<String>]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (i, col) in schema.iter().enumerate() {
        let nums: Vec<f64> = rows
            .iter()
            .filter_map(|r| r.get(i))
            .filter_map(|c| c.parse::<f64>().ok())
            .collect();
        if nums.len() != rows.len() || nums.is_empty() {
            continue;
        }
        let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
        let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mean = nums.iter().sum::<f64>() / nums.len() as f64;
        out.insert(
            col.name.clone(),
            format!(
                "min {} mean {} max {}",
                format_sig(min, 4),
                format_sig(mean, 4),
                format_sig(max, 4)
            ),
        );
    }
    out
}

fn shrink_series(
    name: &str,
    points: &[f64],
    stride: u32,
    total_points: usize,
) -> Option<(Body, Rung)> {
    if points.len() > DECIMATE_TARGET {
        let step = points.len().div_ceil(DECIMATE_TARGET);
        return Some((
            Body::Series {
                name: name.to_string(),
                points: points.iter().step_by(step).copied().collect(),
                stride: stride.saturating_mul(u32::try_from(step).unwrap_or(1)).max(1),
                total_points,
            },
            Rung::Decimated,
        ));
    }
    if points.len() > 1 {
        let min = points.iter().copied().fold(f64::INFINITY, f64::min);
        let max = points.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mean = points.iter().sum::<f64>() / points.len() as f64;
        let last = *points.last().unwrap_or(&0.0);
        let mut ids = BTreeMap::new();
        ids.insert("min".into(), format_sig(min, 4));
        ids.insert("max".into(), format_sig(max, 4));
        ids.insert("mean".into(), format_sig(mean, 4));
        ids.insert("last".into(), format_sig(last, 4));
        return Some((
            Body::Stat {
                summary: name.to_string(),
                ids,
                n: Some(total_points as u64),
                se: None,
                detail: None,
            },
            Rung::Summarised,
        ));
    }
    None
}

fn shrink_text(title: Option<&str>, body: &str) -> Option<(Body, Rung)> {
    let paras: Vec<&str> = body.split("\n\n").filter(|p| !p.trim().is_empty()).collect();
    if paras.len() > 2 {
        return Some((
            Body::Text {
                title: title.map(str::to_string),
                body: format!(
                    "{}\n\n[… {} paragraphs …]\n\n{}",
                    paras[0],
                    paras.len() - 2,
                    paras[paras.len() - 1]
                ),
            },
            Rung::Elided,
        ));
    }
    let first = first_sentence(body);
    if first.len() < body.trim().len() {
        return Some((
            Body::Text {
                title: title.map(str::to_string),
                body: first,
            },
            Rung::FirstSentence,
        ));
    }
    match title {
        Some(t) if !body.is_empty() => Some((
            Body::Text {
                title: Some(t.to_string()),
                body: String::new(),
            },
            Rung::TitleOnly,
        )),
        _ => None,
    }
}

fn first_sentence(s: &str) -> String {
    let t = s.trim();
    for (i, c) in t.char_indices() {
        if matches!(c, '.' | '!' | '?') {
            let end = i + c.len_utf8();
            // A sentence end is punctuation followed by space or the end of the
            // text; `1.4` and `BTC-USD.` mid-token are not sentence ends.
            if t[end..].starts_with(char::is_whitespace) || end == t.len() {
                return t[..end].to_string();
            }
        }
    }
    t.to_string()
}

/// Keys that look like identifiers, for a `Json` body whose producer declared no
/// `required`.
///
/// A guess, and marked as one: the bottom rung of an undeclared JSON body keeps
/// what this finds, and if it finds nothing the body has no bottom rung and the
/// record is dropped whole rather than rendered as a shape with no content.
fn id_like(value: &serde_json::Value) -> Vec<String> {
    value
        .as_object()
        .map(|m| {
            m.keys()
                .filter(|k| *k == "id" || k.ends_with("_id") || *k == "ids")
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn shrink_json(value: &serde_json::Value, required: &[String]) -> Option<(Body, Rung)> {
    let map = value.as_object()?;
    let required: Vec<String> = if required.is_empty() {
        id_like(value)
    } else {
        required.to_vec()
    };

    // Rung 1: prune the values that are themselves large, keeping their keys as
    // a shape line. A pruned key still tells the model the field exists, which
    // is what makes `expand` a sensible next move.
    let big: Vec<String> = map
        .iter()
        .filter(|(k, v)| {
            !required.contains(k)
                && serde_json::to_string(v).map(|s| s.len()).unwrap_or(0) > 64
        })
        .map(|(k, _)| k.clone())
        .collect();
    if !big.is_empty() {
        let mut pruned = map.clone();
        for k in &big {
            pruned.insert(k.clone(), serde_json::Value::String("[pruned]".into()));
        }
        return Some((
            Body::Json {
                value: serde_json::Value::Object(pruned),
                required: required.clone(),
            },
            Rung::PrunedKeys,
        ));
    }
    // Rung 2: the required keys alone.
    if !required.is_empty() && map.len() > required.len() {
        let kept: serde_json::Map<String, serde_json::Value> = map
            .iter()
            .filter(|(k, _)| required.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Some((
            Body::Json {
                value: serde_json::Value::Object(kept),
                required: required.clone(),
            },
            Rung::RequiredKeys,
        ));
    }
    // Rung 3: the ids alone, if any of the kept keys are ids.
    let ids = id_like(value);
    if !ids.is_empty() && ids.len() < map.len() {
        let kept: serde_json::Map<String, serde_json::Value> = map
            .iter()
            .filter(|(k, _)| ids.contains(k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Some((
            Body::Json {
                value: serde_json::Value::Object(kept),
                required: ids,
            },
            Rung::IdsOnly,
        ));
    }
    None
}

/// A number at `sig` significant figures, without a trailing `.0`.
///
/// §25.3: a mean printed as `0.0234871629` against `se = 0.004` spends eight
/// tokens on noise and invites the model to quote a precision the measurement
/// does not support. The `se`-derived version is [`sig_figs_for`].
#[must_use]
pub fn format_sig(v: f64, sig: u32) -> String {
    if !v.is_finite() {
        return format!("{v}");
    }
    if v == 0.0 {
        return "0".into();
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (sig as i32 - 1 - magnitude).clamp(0, 12) as usize;
    let s = format!("{v:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// Significant figures a value earns given its standard error (§25.3).
///
/// The rule: keep one digit past the first significant digit of the error, and
/// never more than six. An `se` of 0.004 against a mean of 0.023 earns two —
/// `0.023`, not `0.0234871629`.
#[must_use]
pub fn sig_figs_for(value: f64, se: f64) -> u32 {
    if !se.is_finite() || se <= 0.0 || !value.is_finite() || value == 0.0 {
        return 4;
    }
    let value_mag = value.abs().log10().floor() as i32;
    let err_mag = se.abs().log10().floor() as i32;
    u32::try_from((value_mag - err_mag + 1).clamp(1, 6)).unwrap_or(4)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn table(rows: usize, cols: &[(&str, u8)]) -> Body {
        Body::Table {
            schema: cols.iter().map(|(n, s)| Column::new(*n, *s)).collect(),
            rows: (0..rows)
                .map(|i| cols.iter().map(|_| i.to_string()).collect())
                .collect(),
            total_rows: rows,
        }
    }

    /// A table renders delimited, never as JSON (§25.1).
    #[test]
    fn a_table_is_not_json() {
        let b = table(3, &[("ts", 200), ("close", 200)]);
        let r = b.render();
        assert!(r.starts_with("ts | close"), "{r}");
        assert!(
            !r.contains('{') && !r.contains('"'),
            "a 200-row result spends ~40% of its tokens on braces and quotes: {r}"
        );
    }

    /// The ladder is walked one rung at a time and each rung says what it did.
    #[test]
    fn a_table_walks_its_declared_ladder() {
        let b = table(50, &[("ts", 200), ("close", 200), ("note", 10)]);
        let (b1, r1) = b.shrink().expect("rung 1");
        assert_eq!(r1, Rung::DroppedColumns);
        match &b1 {
            Body::Table { schema, .. } => {
                assert_eq!(schema.len(), 2, "the low-salience column goes first");
                assert!(schema.iter().all(|c| c.name != "note"));
            }
            other => panic!("still a table, got {other:?}"),
        }

        let (b2, r2) = b1.shrink().expect("rung 2");
        assert_eq!(r2, Rung::SampledRows);
        assert!(
            b2.render().contains("of 50 rows"),
            "a sampled table must still say how many rows it came from: {}",
            b2.render()
        );

        let (b3, r3) = b2.shrink().expect("rung 3");
        assert_eq!(r3, Rung::Aggregated);
        assert!(b3.render().contains("min"));
    }

    /// Nothing on the ladder cuts at a byte offset.
    ///
    /// The property the whole module exists for, stated where it can fail: at
    /// every rung the render is still parseable as the shape it claims to be.
    #[test]
    fn every_rung_is_a_whole_shape() {
        let mut b = table(500, &[("ts", 200), ("close", 200), ("note", 10)]);
        let mut rungs = 0;
        while let Some((next, _)) = b.shrink() {
            let r = next.render();
            match &next {
                Body::Table { schema, .. } => {
                    let header = r.lines().next().unwrap_or("");
                    for c in schema {
                        assert!(header.contains(&c.name), "schema survives every rung: {r}");
                    }
                }
                Body::Stat { .. } => {}
                other => panic!("a table degrades to a table or a stat, got {other:?}"),
            }
            b = next;
            rungs += 1;
            assert!(rungs < 10, "the ladder must terminate");
        }
        assert!(rungs >= 3, "a 500-row table has at least three rungs");
    }

    /// A `ToolResult`-shaped stat is already the ladder.
    #[test]
    fn a_stat_sheds_detail_before_its_number() {
        let b = Body::Stat {
            summary: "sharpe 1.42 over 512 bars".into(),
            ids: BTreeMap::from([("experiment_id".into(), "exp_9".into())]),
            n: Some(512),
            se: Some(0.31),
            detail: Some("a very long per-bar breakdown".repeat(20)),
        };
        let (b1, r1) = b.shrink().expect("detail goes first");
        assert_eq!(r1, Rung::DroppedDetail);
        assert!(b1.render().contains("exp_9"), "the id outlives the detail");
        assert!(b1.render().contains("1.42"), "so does the number");
    }

    /// Text loses its middle as paragraphs, then as sentences — never mid-word.
    #[test]
    fn text_degrades_by_paragraph_then_by_sentence() {
        let body = "One. More one.\n\nTwo.\n\nThree.\n\nFour. More four.";
        let b = Body::Text {
            title: Some("T".into()),
            body: body.into(),
        };
        let (b1, r1) = b.shrink().expect("rung 1");
        assert_eq!(r1, Rung::Elided);
        let r = b1.render();
        assert!(r.contains("One."), "{r}");
        assert!(r.contains("Four."), "the last paragraph is where conclusions live");
        assert!(r.contains("2 paragraphs"), "{r}");
    }

    /// A sentence end is punctuation followed by a space, not any full stop.
    #[test]
    fn a_decimal_point_does_not_end_a_sentence() {
        assert_eq!(
            first_sentence("Sharpe was 1.42 on the sample. Then it fell."),
            "Sharpe was 1.42 on the sample."
        );
    }

    /// JSON prunes large values to a shape line before it drops keys.
    #[test]
    fn json_prunes_the_large_values_first() {
        let b = Body::Json {
            value: json!({"experiment_id": "exp_1", "rows": "x".repeat(400), "n": 512}),
            required: vec!["experiment_id".into()],
        };
        let (b1, r1) = b.shrink().expect("rung 1");
        assert_eq!(r1, Rung::PrunedKeys);
        let r = b1.render();
        assert!(r.contains("[pruned]"));
        assert!(r.contains("exp_1"), "the required key survives");
        assert!(r.contains("\"n\":512"), "a small field is not pruned: {r}");
    }

    /// The ladder bottoms out rather than looping, for every variant.
    #[test]
    fn every_ladder_terminates() {
        let bodies = vec![
            table(40, &[("a", 10), ("b", 10)]),
            Body::Series {
                name: "equity".into(),
                points: (0..500).map(f64::from).collect(),
                stride: 1,
                total_points: 500,
            },
            Body::Stat {
                summary: "line one\nline two".into(),
                ids: BTreeMap::new(),
                n: Some(1),
                se: None,
                detail: Some("d".into()),
            },
            Body::text("a. b. c."),
            Body::Json {
                value: json!({"id": "x", "big": "y".repeat(200), "other": 1}),
                required: vec![],
            },
        ];
        for b in bodies {
            let mut cur = b.clone();
            let mut n = 0;
            while let Some((next, _)) = cur.shrink() {
                assert!(
                    next != cur,
                    "a rung that returns its own input would loop: {cur:?}"
                );
                cur = next;
                n += 1;
                assert!(n < 12, "ladder did not terminate from {b:?}");
            }
        }
    }

    /// Precision follows the error bar (§25.3).
    #[test]
    fn precision_follows_the_error_bar() {
        assert_eq!(sig_figs_for(0.0234871629, 0.004), 2);
        assert_eq!(format_sig(0.0234871629, sig_figs_for(0.0234871629, 0.004)), "0.023");
        // A tight error earns more digits, and only as many as it earns.
        assert_eq!(sig_figs_for(1.42871, 0.00004), 6);
        // No error bar is not a licence for sixteen digits.
        assert_eq!(sig_figs_for(1.42871, 0.0), 4);
    }

    #[test]
    fn significant_figures_do_not_print_trailing_noise() {
        assert_eq!(format_sig(1.0, 4), "1");
        assert_eq!(format_sig(-0.00123456, 3), "-0.00123");
        assert_eq!(format_sig(0.0, 4), "0");
        assert_eq!(format_sig(1234.5678, 4), "1235");
    }

    /// A series decimates before it collapses, and says its stride.
    #[test]
    fn a_series_says_it_is_no_longer_unit_spaced() {
        let b = Body::Series {
            name: "equity".into(),
            points: (0..240).map(f64::from).collect(),
            stride: 1,
            total_points: 240,
        };
        let (b1, r1) = b.shrink().expect("rung 1");
        assert_eq!(r1, Rung::Decimated);
        let r = b1.render();
        assert!(r.contains("every 10"), "a decimated series must say so: {r}");
        assert!(r.contains("240 points"));
    }
}
