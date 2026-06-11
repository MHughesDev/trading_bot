# Agent Spawn Template — Set-C Latency Optimization

Copy this prompt and replace `{AGENT_NUMBER}` with the agent number (01-15). Then spawn a new agent with the customized prompt.

---

## SPAWN AGENT {AGENT_NUMBER}

You are **Agent-{AGENT_NUMBER}** from the Set-C Latency Optimization initiative. Your task is to execute the work package assigned to your agent.

### Your Mission
Complete all optimization work described in your agent documentation:
- **File**: `docs/plans/plan-sets/set-C/agents/agent-{AGENT_NUMBER}.md`
- **Reference**: `docs/plans/plan-sets/set-C/MASTER.md` (master plan context)

### What You Must Do
1. **Read your agent file** (`docs/plans/plan-sets/set-C/agents/agent-{AGENT_NUMBER}.md`) to understand:
   - Your assigned phase and component focus
   - Specific issues you own (see issue numbers in your file)
   - Deliverables and acceptance criteria
   - Dependencies on other agents

2. **Read all your referenced issues** from `docs/plans/plan-sets/set-C/issue-*.md`
   - Extract technical requirements
   - Identify integration points
   - List blockers or prerequisites

3. **Create an execution plan**:
   - Break down work into atomic tasks
   - Identify file changes needed
   - List test coverage requirements
   - Document implementation timeline

4. **Execute the work**:
   - Make all code changes
   - Update tests
   - Create/update documentation
   - Ensure no regressions

5. **Verify completion**:
   - All issues marked complete
   - Tests passing
   - Code reviewed
   - Documentation updated

### Context
- **Codebase**: `/home/user/trading_bot/`
- **Master Plan**: `docs/plans/plan-sets/set-C/MASTER.md`
- **Your Issues**: Listed in `docs/plans/plan-sets/set-C/agents/agent-{AGENT_NUMBER}.md`
- **Phase Goals**: Check MASTER.md for phase {PHASE} objectives

### Success Criteria
✅ All tasks in agent-{AGENT_NUMBER}.md completed
✅ All referenced issues addressed
✅ Code passes CI/CD
✅ Documentation updated
✅ No blockers for dependent agents

### When Complete
Report:
1. Tasks completed
2. Files changed
3. Tests added/updated
4. Any blockers for downstream agents
5. Ready for: [Next dependent agent]

---

## How to Use This Template

**Step 1**: Replace `{AGENT_NUMBER}` with the agent number (01-15)
**Step 2**: Replace `{PHASE}` with the phase letter (A, B, C, D, E, F, G)
**Step 3**: Copy the customized prompt
**Step 4**: Spawn a new agent with this prompt

**Example for Agent-01**:
```
You are Agent-01 from the Set-C Latency Optimization initiative...
File: docs/plans/plan-sets/set-C/agents/agent-01.md
Phase: A
...
```

---

## Quick Reference: Agent Mapping

| Agent | File | Phase | Component |
|-------|------|-------|-----------|
| 01 | agent-01.md | A | JetStream hot path |
| 02 | agent-02.md | A | Binary envelope |
| 03 | agent-03.md | B | Bytecode compiler |
| 04 | agent-04.md | B | Slot array features |
| 05 | agent-05.md | B | Dispatch and universe |
| 06 | agent-06.md | B | Intent filtering |
| 07 | agent-07.md | C | Collector hot path |
| 08 | agent-08.md | C | Scraper and Reddit |
| 09 | agent-09.md | D | Redis and storage |
| 10 | agent-10.md | D | Lock-free registries |
| 11 | agent-11.md | D | Execution and ratelimit |
| 12 | agent-12.md | E | Build config |
| 13 | agent-13.md | E | Subscriptions |
| 14 | agent-14.md | E | API and misc |
| 15 | agent-15.md | F-G | Phase F and G |

---

## Example: Spawning Agent-03

Replace {AGENT_NUMBER} → **03** and {PHASE} → **B**:

```
You are Agent-03 from the Set-C Latency Optimization initiative. 
Your task is to execute the work package assigned to your agent.

Your Mission:
Complete all optimization work described in your agent documentation:
- File: docs/plans/plan-sets/set-C/agents/agent-03.md
- Reference: docs/plans/plan-sets/set-C/MASTER.md (master plan context)

Component Focus: Bytecode Compiler
Phase: B

[Continue with rest of template...]
```

Then spawn the agent with this customized prompt.
