# research/

Research briefs, technology evaluations, and findings from discovery work. This folder feeds into plans, specs, and ADRs — it is the evidence layer.

> To add a new research brief, follow the skill: `skills/create-research-brief.md` and procedure: `procedures/add-research.md`.

---

## What Belongs Here

- Technology comparison briefs ("Why Rust over Go or Python for this workload?")
- Prior art surveys ("how do other systems solve X?")
- Broker, venue, and API evaluations
- Stack component evaluations (runtime, event fabric, storage engines)
- Performance benchmark summaries

## What Does Not Belong Here

- Decisions — record those in `adr/`
- Plans that act on the research — record those in `plans/`
- Specifications derived from research — record those in `specs/`

---

## Conventions

- File names: `kebab-case.md` — be descriptive: `rust-trading-stack-evaluation.md`
- Each research brief has:
  - **Question** — what were you trying to find out?
  - **Status** — Complete or In Progress
  - **Outcome** — one sentence summary of what was recommended and whether it was adopted
  - **ADR(s)** — which ADRs this brief directly informed
  - **Method** — how was the research done?
  - **Findings** — what was learned?
  - **Recommendation** — what does this research suggest?
  - **References** — sources
- Research files are not modified once written. Add a new file if follow-up research changes the picture, and cross-reference the earlier file.

---

## Index

| Brief | Question | Status | Outcome |
|-------|----------|--------|---------|
| [rust-trading-stack-evaluation.md](./rust-trading-stack-evaluation.md) | Why Rust and which libraries? | Complete | Rust with tokio/axum/NATS/sqlx/ClickHouse/parquet — adopted (ADR-0001, ADR-0003, ADR-0004) |
| [broker-venue-selection.md](./broker-venue-selection.md) | Which brokers and venues for v1? | Complete | Coinbase=live, Alpaca=paper+equity-data, Kraken=crypto-data, market_simulator=backtest — adopted (ADR-0006, ADR-0011) |
