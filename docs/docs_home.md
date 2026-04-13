# Docs Home (Generalized Cursor/Agent Setup)

This repository now uses a generalized docs layout suitable for Cursor/agent-driven repos:

- `docs/foundation/` — orientation, conventions, commentary
- `docs/architecture/` — system architecture and technical design
- `docs/operations/` — runbooks, setup, deployment, operator flows
- `docs/governance/` — audits, promotion policy, release/process controls
- `docs/backlog/` — roadmap/backlog references outside queue system
- `docs/reports/` — generated audit/review artifacts
- `docs/QUEUE*` + queue skill docs — queue system canonical runtime for agents

## Agent-first operating pattern

1. Read `AGENTS.md`.
2. Read only top Open row in `docs/QUEUE_STACK.csv`.
3. Follow `.cursor/skills/queue-one-at-a-time/SKILL.md`.
4. Implement one item, update docs touched by behavior changes, commit.

## Compatibility

Queue-system canonical paths remain stable at the docs root (`QUEUE.MD`, `QUEUE_ARCHIVE.MD`, `QUEUE_STACK.csv`, `QUEUE_SCHEMA.md`).
