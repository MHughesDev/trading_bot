# COMP-006: Research Workspace UI

**Status:** Proposed (Phase 0 contract; not implemented)
**Version:** 0.1
**ADR(s):** ADR-0024, ADR-0025
**Derived from:** BS-007 [15_WORKSPACE_UI](../BRAINSTORM/BS-007_QUANT_RESEARCH_AGENT/15_WORKSPACE_UI.MD)
**Plan set:** L (v1); later sets add panes (Skills: R-a; strategy AST editor: O)
**Code:** `frontend/src/pages/ResearchWorkspacePage.tsx` (new, replacing
`AgentPage.tsx`), `frontend/src/components/workspace/*`; reuses the Set J/K workbench
components (`components/workbench/*`) and the Set K unified workspace where it has landed

---

## 1. Purpose

Where humans **watch, steer, approve and inspect** the agent's work. It uses the same
APIs as the agent (D-05). There are no modes: a Desk question and an overnight campaign
share one layout.

## 2. Routes and layout

- `/research` shows the project list plus the Desk (default landing).
- `/research/:projectId` opens the project workspace. `…/sessions/:sessionId` focuses a
  session.
- `/approvals` is the global approvals inbox.
- The workspace has three columns (collapsible on narrow screens):
  1. **Left:** projects and sessions, and the budget meter (dollars, compute, wall clock,
     cache hit rate).
  2. **Centre:** conversation and timeline, with the steering input.
  3. **Right:** tabbed panes (§3).

## 3. Panes

| Pane | Content | Data source |
|---|---|---|
| Timeline | Messages; tool/CLI cards (collapsed, expandable to full output via artifact); rich cards rendered from `for_user` chart and table specs; sub-agent lanes (collapsible); checkpoint cards; approval cards inline; compaction markers | `GET /api/agent/sessions/{id}/events` (SSE; AGENT-001 §18) |
| Steering | Text input; interrupt; stop | AGENT-001 §16 |
| Notebook | Rendered `NOTEBOOK.md`; `RESEARCH_PLAN.json` as a hypothesis table with status chips; git log with id links | `GET /api/agent/projects/{id}/workspace/files?path=` (read-only proxy through the orchestrator) |
| Artifacts | Browse by type; manifest and lineage viewer; Parquet preview (first 100 rows, server-side); chart-spec renderer | COMP-005 §8 |
| Jobs | Live tree (sweep → members); state, progress, cost; cancel | COMP-005 §6 |
| Experiments | Set J workbench components (funnel, sealed distributions, null picker, gate ledger); trials, effective N, `post_hoc` flags; dossier viewer | Set J API, BACKTEST_SUITE_CORE_SPEC v2 |
| Report | `final_report` with claim → evidence links; the dossier | `GET /api/reports/{id}` |
| Skills (R-a) | Glossary tree; skill detail, versions, bill-of-materials stats, eval results; promotion queue | AGENT-003 §8 |
| Cost | Per-session `llm_usage` aggregates: dollars, cache hit rate, rewrites, dollars per verdict, burn rate | `GET /api/agent/usage` |

## 4. Approvals inbox

All kinds from `approval_requests` (AGENT-001 §5):
- `ask_user`;
- `plan`;
- `budget`;
- `qc_waiver`;
- `model_promotion`;
- `skill_promotion`;
- `paper_deployment` (after G-11).

Each card shows the evidence (dossier, eval results, diff, qc report), options, the
default and the time left. Answers record `answered_by` and the time. Skill promotion
cards enforce that the reviewer isn't the author.

## 5. Rich-card contract

Tool outputs may carry `for_user_ref` (an artifact of type `chart_spec` or
`table_spec`). The UI renders:
- **charts** with Vega-Lite (`chart_spec` is a Vega-Lite JSON document with data inlined
  or referenced by artifact handle);
- **tables** as sortable grids.

The model never sees these payloads (AGENT-002 §2 P3).

## 6. Human editing surfaces

- Strategy editor over the SLv2 AST (text plus structured views) — FEAT-004, Set O.
- DatasetSpec editor (YAML with live validation) — FEAT-006, Set N.
- Experiment workbench (exists).

All of them call the same routes the agent uses.

## 7. Retirement

- `AgentPage.tsx` and `components/agent/*` are removed at Set L exit. The
  adaptive-polling timeline is replaced by SSE.
- `/backtesting`, `/proving-ground` and `/workbench` consolidation follows Set K. The
  Experiments pane embeds the unified workbench.

## 8. Requirements mapping

| BS-007 ID | Where |
|---|---|
| UI-01 | §2 |
| UI-02 | §3 timeline (SSE) |
| UI-03 | §3 steering |
| UI-04 | §3 notebook |
| UI-05 | §4 |
| UI-06 | §3 Skills |
| UI-07 | §2 budget meter + §3 cost |
| UI-08 | §3 report |
| UI-09 | §6 (FEAT-004) |

## 9. Acceptance (Set L)

1. Watching a live synthetic campaign: jobs update, a checkpoint appears, a steering
   message changes the next action, and an approval card is answered, all without page
   reloads.
2. Every number in a rendered report opens its source artifact.
3. The Desk answers a live-data question. A research project shows the
   `live_data_desk_only` error with a link to the Desk.
4. The cost pane matches `llm_usage` totals for the session.
