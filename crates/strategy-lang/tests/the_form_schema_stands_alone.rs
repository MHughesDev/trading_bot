//! The clause grammar can be handed to something that does not resolve
//! references (S-5.9; AGENT-011 §3, P7).
//!
//! `form::document_schema` shares definitions through `$ref`. That is fine for a
//! reader and wrong for the two consumers the grammar exists for: a constrained
//! decoder, whose grammar converter should not have to resolve references to be
//! correct, and a tool's `inputSchema`, which is read by more than one consumer.
//! The inlined form is what the MCP tool `compile_strategy_form` declares, so a
//! `select_fill` tier decoding that tool's arguments is decoding under this.

use serde_json::Value;
use strategy_lang::form;

fn mentions_ref(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.contains_key("$ref") || o.values().any(mentions_ref),
        Value::Array(a) => a.iter().any(mentions_ref),
        _ => false,
    }
}

#[test]
fn the_inlined_document_schema_has_no_references_and_every_family() {
    let s = form::document_schema_inlined();
    assert!(!mentions_ref(&s), "a `$ref` survived inlining");
    assert!(s.get("$defs").is_none(), "`$defs` was left behind");
    assert_eq!(s["type"], "object");
    let text = s.to_string();
    // The point of the grammar: the engine's families, and nothing the engine
    // does not serve, are what a feature can be.
    for f in features::registry::families() {
        if f.alias_of.is_some() {
            continue;
        }
        assert!(
            text.contains(&format!("\"{}\"", f.name)),
            "`{}` is in the registry and not in the schema",
            f.name
        );
    }
    assert!(!text.contains("\"not_a_family\""));
}

#[test]
fn every_clause_schema_inlines_on_its_own() {
    for (name, _) in form::clause_schemas() {
        let s = form::clause_schema_inlined(name)
            .unwrap_or_else(|| panic!("`{name}` is a clause and has no standalone schema"));
        assert!(!mentions_ref(&s), "`{name}` still has a `$ref`");
    }
    assert!(form::clause_schema_inlined("not_a_clause").is_none());
}

/// The per-family schema is exact about arity and range and is **large**: an
/// operand carries every family and a filter has up to five operands. That is why
/// a tool declares the compact one. Recorded so a growth is a decision.
#[test]
fn the_per_family_schema_is_too_large_for_a_tool_and_the_compact_one_is_not() {
    let full = form::document_schema_inlined().to_string().len();
    let compact = form::document_schema_compact().to_string().len();
    eprintln!("per-family schema: {full} bytes; compact schema: {compact} bytes");
    assert!(
        compact < 40_000,
        "{compact} bytes is too large to list to every client"
    );
    assert!(
        full > 10 * compact,
        "the compact schema is no longer much smaller"
    );
}

/// What the compact schema keeps is the property the acceptance names: a feature
/// is one of the engine's families and nothing else. What it gives up — arity and
/// range — comes back from the compiler as the same diagnostic free text gets.
#[test]
fn the_compact_schema_names_every_family_and_no_other() {
    let s = form::document_schema_compact();
    assert!(!mentions_ref(&s));
    let text = s.to_string();
    for f in features::registry::families() {
        if f.alias_of.is_some() {
            continue;
        }
        assert!(text.contains(&format!("\"{}\"", f.name)), "`{}`", f.name);
    }
    for (name, _) in form::clause_schemas() {
        let c = form::clause_schema_compact(name).expect("every clause has a compact schema");
        assert!(!mentions_ref(&c), "`{name}`");
    }
    assert!(form::clause_schema_compact("not_a_clause").is_none());
}
