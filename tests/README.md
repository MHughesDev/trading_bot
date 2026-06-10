# Integration Tests

Cross-crate integration tests. Each test exercises a full data path end-to-end and requires local infra running (`just infra`).

Tests are added as phases complete:

| Phase | Test file | What it proves |
|-------|-----------|----------------|
| Phase 1 | `ingest_to_storage.rs` | collector → bus → storage writer → ClickHouse/Parquet |
| Phase 2 | `manual_order_flow.rs` | REST order → risk gate → paper execution → position update |
| Phase 2 | `reconciliation_halt.rs` | forced divergence halts the instrument |
| Phase 4 | `strategy_end_to_end.rs` | definition → runtime → intent → risk → paper fill |
| Phase 7 | `quarantine_replay.rs` | malformed feed → quarantine → fix → replay → storage |

To run: `just test-integration` (requires `just infra` first)
