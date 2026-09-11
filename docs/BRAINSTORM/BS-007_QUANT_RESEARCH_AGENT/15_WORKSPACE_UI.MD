# BS-007 / 15 — Research workspace UI

**Spec status:** spec-ready draft · **Req prefix:** UI- · **Formal spec:** COMP-006 ·
**Plan set:** L (v1), then with each phase · **Decisions:** D-04, D-05 · **Replaces:**
`frontend/src/pages/AgentPage.tsx` and `components/agent/*` (01 §8)

---

## 1. Principle

The UI is where humans **watch, steer, approve and inspect**. It is a viewer and editor
over the same APIs the agent uses (D-05). There are no modes (D-04): a quick question and
an overnight campaign use the same workspace.

## 2. Layout

```
┌───────────────┬──────────────────────────────────────┬────────────────────────────────┐
│ Projects      │ Conversation + timeline               │ Notebook │ Artifacts │ Jobs    │
│  ▸ Desk       │  user / agent messages                │ Experiments │ Skills │ Report   │
│  ▸ BTC trend  │  tool/CLI cards (collapsed)           │                                │
│  ▸ SOL swing  │  rich cards: charts, surfaces,        │  Notebook: renders NOTEBOOK.md │
│ Sessions      │   gate ledgers, event studies,        │   + RESEARCH_PLAN.json         │
│               │   sweep progress, dossier             │  Artifacts: chart specs,       │
│ Budget meter  │  checkpoints · approvals inline       │   datasets, models, strategies │
│ $ · compute · │                                       │  Jobs: live tree + states      │
│ wall · cache% │  [ steer / ask … ]                    │  Experiments: funnel, trials   │
└───────────────┴──────────────────────────────────────┴────────────────────────────────┘
          Approvals inbox (global): paper deployments · model promotions · skill promotions ·
          qc waivers · budget increases
```

## 3. Panes

| Pane | Shows | Source |
|---|---|---|
| Projects / sessions | Project list (including the per-user **Desk** project, 06 §3.1), sessions, status, budgets | Orchestrator |
| Conversation / timeline | Messages; tool and CLI cards; rich cards from `for_user` payloads (chart specs, tables); checkpoints; approval cards; sub-agent lanes (collapsible) | SSE `agent_events` |
| Steering input | Send a message into a running session (03 §4); interrupt; end | Orchestrator |
| Notebook | Rendered `NOTEBOOK.md` and `RESEARCH_PLAN.json` (hypotheses with statuses), git history with id links | Workspace via orchestrator |
| Artifacts | Browsable artifacts by type, with manifest and lineage | Artifact store |
| Jobs | Live job tree (sweeps → members), progress, cost, cancel | Job service SSE |
| Experiments | Existing Set J workbench components (funnel, sealed distributions, null picker, gate ledger), trial and effective-N counters, dossier viewer | Set J API |
| Skills | Glossary tree, skill detail and version history, bill-of-materials stats, eval results, promotion queue | Skill registry (13 §9) |
| Report | `final_report` and dossier, with every claim linked to its evidence | Reports API |
| Budget / cost | Dollars, compute, wall clock, cache hit rate, dollars per verdict, burn rate | LLM proxy and job service (04 §9) |

## 4. Approvals

These are the only way consequential things happen (D-12):
- paper-deployment proposals (after G-11);
- model alias promotions;
- global skill promotions (reviewer ≠ author);
- `data_qc` grade-C waivers;
- budget increases;
- `ask_user` questions.

Each approval shows the evidence (dossier, evals, diff) and records who approved it and
when.

## 5. Human editing surfaces (byproducts)

- **Strategy editor** over the SLv2 AST: text plus structured view (08 §8).
- **DatasetSpec editor** (YAML with validation).
- **Experiment workbench** (Set J/K, exists).
- These call the same APIs as the agent. Nothing is UI-only.

## 6. Event stream

SSE event types:
- `message`, `tool_call`, `tool_result` (summary plus `for_user` payload ref);
- `subagent_start|end`, `compaction`;
- `checkpoint`, `approval_request|resolved`;
- `job_state`, `artifact_created`, `report_submitted`;
- `budget`, `skill_notice`.

These replace adaptive polling.

## 7. Requirements

| ID | Requirement | Pri |
|---|---|---|
| UI-01 | One workspace layout for all sessions; no mode switch | MUST |
| UI-02 | Timeline MUST stream over SSE with the §6 event types | MUST |
| UI-03 | Steering, interrupt and `ask_user` answering MUST work in running sessions | MUST |
| UI-04 | The Notebook pane MUST render the workspace NOTEBOOK and plan files | MUST |
| UI-05 | The approvals inbox MUST cover every §4 kind, with evidence and an audit record | MUST |
| UI-06 | The Skills page MUST show the glossary, skill history, stats and promotion queue | MUST (phase 4+) |
| UI-07 | The budget pane MUST show dollars, compute, wall clock, cache hit rate and dollars per verdict | MUST |
| UI-08 | Every claim in a report view MUST link to its evidence artifact | MUST |
| UI-09 | The strategy editor MUST edit the SLv2 AST | MUST (phase 4) |

## 8. Acceptance criteria

1. A user watches a live campaign: jobs update, a checkpoint appears, a steering message
   changes the next action, and an approval card is answered, all without reloads.
2. Every number in a rendered report opens its source artifact.
3. The Desk project answers a live-data question. A research project refuses live data
   with an explanation.
