# Quant Research Agent: what it is, how it acts, what it needs

**Brainstorm ID:** BS-007
**Scope:** multi
**Status:** draft
**Author:** Mason Hughes (with Claude), 2026-09-10

---

## One-liner

Design the best quant research agent this platform can support: an agent with its own
workspace UI that answers questions about any instrument, runs long honest research
sessions that build, backtest and model trading algorithms, and is token-efficient
enough to run for hours.

## Problem / opportunity

The platform already has an unusually strong evaluation core: sealed distributions,
automatic trial counting, 7 null families, a 5-gate funnel with DSR and PBO, and a sweep
engine where the LLM can't pick numbers. The agent sitting on top of it is a 38-tool
loop whose memory is its transcript. It can't analyse data, train models or remember past
sessions, and the strategies it can express are limited to EMA/RSI entries with fixed
size. The bottleneck is the agent's reach and harness, not the rigour of evaluation.

## Ground rule

**The platform's exposed capabilities are the truth.** The external ChatGPT context pack
(`Quant_Platform_Agent_Context.md`) was treated as input and checked against the code.
Several of its "highest priorities" already exist (trial accounting, PIT timestamps,
nulls, gates), and several capabilities it assumed don't (Transformers/TFT, AI models
affecting backtests, rich indicators, exits). See 01.

## Reading order

| File | Answers |
|---|---|
| [01_GROUND_TRUTH.MD](01_GROUND_TRUTH.MD) | What can the agent touch today, and what is broken or missing (verified, with paths)? |
| [02_AGENT_DEFINITION.MD](02_AGENT_DEFINITION.MD) | What is the agent? Jobs, modes, behaviour contract, user interaction, UI, how "best" is measured |
| [03_TOOLBOX.MD](03_TOOLBOX.MD) | What tools should it have, and how should they be shaped? |
| [04_HARNESS.MD](04_HARNESS.MD) | How does the loop, memory, token efficiency, sub-agents, long-running work and self-evaluation work? |
| [05_KNOWLEDGE_AND_MEMORY.MD](05_KNOWLEDGE_AND_MEMORY.MD) | Should we use embeddings? What should an embedding represent? |
| [06_QUANT_KNOWLEDGE_CORE.MD](06_QUANT_KNOWLEDGE_CORE.MD) | The quant knowledge (stochastic calculus → toolkit → ML → archetypes → playbook), as seed cards tied to tools |
| [07_ROADMAP.MD](07_ROADMAP.MD) | Build order *(superseded by 08 §10)* |
| [08_AGENT_FIRST_REASSESSMENT.MD](08_AGENT_FIRST_REASSESSMENT.MD) | **Start here after 01.** Keep/change/rebuild verdict per subsystem for an agent-first platform: harness, container, data, features, strategy language, backtest, models, evaluation |
| [09_AGENTIC_TECHNIQUES.MD](09_AGENTIC_TECHNIQUES.MD) | Researched agent techniques (harness, context, search, memory, verification, security, self-improvement) mapped to platform components |
| [10_QUANT_TECHNIQUES_FOR_TOOLS.MD](10_QUANT_TECHNIQUES_FOR_TOOLS.MD) | Researched quant techniques graded by evidence strength, turned into tools and capabilities |
| [11_QUANT_RESEARCH_STANDARD.MD](11_QUANT_RESEARCH_STANDARD.MD) | **The agent's research standard:** the backtesting protocol made mechanical, a 20-topic quant competency map with QC rules, `data_qc`, and the required candidate dossier. Wins over 06/10 where they differ |
| [12_SKILLS.MD](12_SKILLS.MD) | **Agent-created skills:** when the agent writes one (verified-experience triggers T1–T5), how the platform verifies and admits it, how every session for every user gets core and global skills, and how every admitted skill enters the glossary |

> **Decisions 2026-09-10:** frontier models first · sandboxed container for the agent ·
> local embeddings · one agent optimised for long-running work (no modes) · the platform
> is built for the agent, with the UI as a byproduct · the agent creates its own skills,
> which the platform verifies and admits, and which are materialised for every user and
> always listed in the glossary (12). Where 02–07 conflict with these, 08 wins.

---

## Proposal in brief

**What the agent is (02).** One agent, three modes sharing one harness: **Ask** (Q&A,
seconds), **Research** (build or improve one algorithm, hours), **Campaign** (autonomous
search, overnight). Its behaviour contract: numbers only from tools, performance claims
only from Set J, the LLM proposes structure while optimisers pick numbers, hypotheses are
pre-registered, failures are recorded, research authority only (deployment is a
human-approved proposal).

**How it interacts with the platform (03).** Only through the MCP tool layer (ADR-0022),
which wraps `/api/*`. Grow the toolbox from 38 to ~100 tools in groups: session/meta,
knowledge, live data, **server-side market analysis**, authoring, evaluation, models,
text, portfolio, deployment proposals. Principles: answers not data, artifact handles,
progressive disclosure, dual-channel output (rich to the UI, compact to the model),
epistemic labels on every number, jobs for slow work, holdout-aware analysis, loaded by
group.

**Harness (04).** Keep the driver. Replace "transcript as memory" with a harness-owned
`ResearchState` (plan, hypotheses, findings, candidates, budget) plus a rolling summary.
Add prompt caching, parallel tool calls, event-driven zero-token waits, resume on
restart, steering messages, `ask_user`, a structured and citation-checked
`final_report`, sub-agents (Analyst, Author, Critic, Summarizer), model routing, and phase
gates. Measure the agent with a benchmark of planted-edge, pure-noise, leakage-trap and
Q&A tasks.

**Knowledge (05).** Six layers. Embeddings only for L1 reference cards, L2 research
findings and L4 news/social text. Each embedding is one self-contained, citeable unit
(a concept card, a finding, a document). Never transcripts, tool JSON or numbers. Market
knowledge is structured and recomputed; "similar periods" uses explicit regime-feature
vectors, not learned embeddings.

**Quant knowledge (06).** Seed cards from stochastic foundations (filtrations = PIT,
P&L as an Itô sum, optional stopping, Merton's drift-estimation asymmetry, Kelly, SDE
nulls, Girsanov, Feynman–Kac for Kalshi, Malliavin sensitivities) through the practical
toolkit, ML, a 15-archetype library with feasibility today, and a diagnostic→move
playbook.

---

## Workstreams

| # | Name | What it covers | Depends on |
|---|---|---|---|
| 1 | Harness | ResearchState, caching, jobs, resume, steering, final_report, sub-agents, routing | — |
| 2 | Analysis tools | Q&A engine (03 §D) in Rust over ClickHouse | 1 (dual-channel output) |
| 3 | Platform unblockers | Feature library parity, exits, sizing, cost/seed/version, fill timing | — |
| 4 | Models in session | PIT inference in backtests, training/eval tools, meta-labeling, sequence models | 3 |
| 5 | Knowledge & memory | Cards, findings, Milvus collections, local embeddings, instrument profiles | 1 |
| 6 | Workspace UI | Three-pane workspace, notebook, artifacts, approvals, SSE | 1 |
| 7 | Agent evaluation | Synthetic instruments, suites, dashboards | 1 |
| 8 | Campaigns | FEAT-003 Phases 2–3 | 3, 5 |

## Constraints / alignment

- Set J invariants INV-1/2/3 and ADR-0023 P1 are non-negotiable (01 §11).
- No order or automation tools in any agent profile; deployment only via a human-approved proposal.
- Local-model support (Ollama) constrains tool counts per context. Tool groups address this.

## Open questions (for you)

1. **Primary model target.** Optimise the harness for frontier models (Claude/GPT) first,
   or keep local models first-class? This drives tool granularity and context budgets.
2. **Sandbox (D3).** Allow constrained Python execution over PIT-clipped data (recommended)
   or tools only?
3. **Embeddings.** Stay on OpenAI `text-embedding-3-small`, or add a local embedding model
   so research notes never leave the machine (recommended)?
4. **Exploration accounting.** Should analysis calls inside an Experiment count as trials
   (strict), go to a separate exploration ledger reported with the verdict (recommended),
   or not be tracked?
5. **Deployment.** May the agent *propose* paper deployment for human approval?
6. **Equities depth.** Stock Q&A ("how did AAPL react to earnings?") needs fundamentals,
   corporate events and news the platform doesn't collect. Add a source, or scope equities
   to price/volume questions for now?
7. **Priority.** Agree that platform unblockers (Phase 2) should run in parallel with
   harness work, given that today's reachable strategy space is ~2 archetypes?

## Next step

- [ ] **Spike:** Phase 0 + the first three Phase 1 tools (`describe_returns`,
  `volatility_report`, `event_study`) behind dual-channel output, with the pure-noise eval
  task. That validates the harness direction on real sessions within one workstream.

## Links

- [FEAT-003 Strategy Research Agent](../../Specs/FEAT-003-strategy-research-agent.md) · [Strategy search study](../../research/strategy-search-study.md) · [Backtest suite core spec](../../Specs/BACKTEST_SUITE_CORE_SPEC.md) · [INTG-001 MCP server](../../Specs/INTG-001-mcp-server.md)
