"""Model families (FEAT-006 section 4, Set N Phase 2).

`registry.json` is the one declaration of every `(family, variant)` the trainer
can run; `registry.py` resolves it. The Rust crate `crates/model-registry`
embeds the same JSON at build time and validates a train request against it
before dispatching.
"""
