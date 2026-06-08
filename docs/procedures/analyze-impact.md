# Procedure: Analyze Impact

## Purpose
Before changing or removing a design element, compute its full downstream blast radius — every
element that cites it, transitively — so the cost of the change is known before it is made.

This is the reverse of normal traceability: links point from idea toward implementation, but this
procedure walks them *backward from the source of a change toward everything that depends on it*.
It complements `verify-traceability.md` (which checks integrity after the fact) by predicting
consequences before you act.

## Trigger
Use this procedure when:
- Considering a change to an artifact success condition (`SC-N`) or failure mode (`FM-N`)
- Considering editing, deprecating, or superseding a spec or a spec section (`§N.M.X`)
- Considering reversing or superseding an ADR
- Resolving an open question in a way that may invalidate work already done
- Estimating the cost of a proposed change before committing to it

## Inputs
- The **target element**: the ID of the thing that might change (`SC-2`, `FEAT-001 §3.1.C`,
  `ADR-0001`, `Q-5`, etc.)
- Read access to the whole `docs/` tree

---

## Steps

### 1. Name the target precisely
State the exact ID being changed. "Change the auth spec" is too coarse — `FEAT-001 §3.1.C`
(a single requirement) has a far smaller blast radius than `FEAT-001` (the whole spec).

### 2. Find direct citations (one hop)
Search `docs/` for every reference to the target ID. Record each citing element with its own ID
and file. Look in all the places references live:
- Plans (Derived From, milestone Source columns, task source traces)
- Specs (§1.3 Related, acceptance criteria that cite requirements, cross-spec references)
- ADRs (References sections)
- Research briefs (forward links)
- `open-questions.md`, `architecture.md`

### 3. Expand transitively to closure
For each element found in Step 2, repeat Step 2 on *its* ID. Keep expanding until no new elements
appear. The result is the transitive closure — the complete set of elements that depend on the
target directly or indirectly.

> Example: editing `SC-2` → cited by milestone M2 and `FEAT-002 §2.1` → `FEAT-002` is cited by
> plan tasks T-4, T-5 and by `FEAT-003 §1.3` → and so on. The blast radius is everything reached.

### 4. Classify each downstream element by required action
For every element in the closure, record what the change forces:

| Downstream element | Action the change forces |
|--------------------|--------------------------|
| A formal plan | Mark `Derivation Status: STALE`; re-derive (`add-plan.md`) |
| A plan task already `COMPLETE` | Re-verify — its source moved; the prior verification may no longer hold |
| A spec | Review against the change; may need a new version or supersession |
| A spec acceptance criterion | Re-check it still maps to the changed requirement |
| An ADR | Review whether its rationale still holds; may need superseding |
| An open question | Reopen if the change reintroduces a settled fork |

### 5. Produce the impact report
Summarize:
- **Target:** the ID being changed
- **Blast radius:** the count of downstream elements (a coupling signal — a large number means the
  target is load-bearing and the change is expensive)
- **By element:** the closure list with each element's required action from Step 4
- **Keystone flag:** if the blast radius is large, say so plainly — the change is high-cost and
  should be made deliberately, not casually

### 6. Decide, then act
Use the report to decide whether the change is worth its blast radius. If you proceed:
- Apply the change to the target.
- Fire the actions from Step 4 — mark dependent plans STALE, flag specs for review, etc.
  (This generalizes the one-hop re-derivation trigger in `add-spec.md` / `add-adr.md` to the
  full closure.)
- Re-run `verify-traceability.md` afterward to confirm nothing was left dangling.

---

## Outputs
- An impact report: target, blast-radius count, and the classified downstream closure
- (If the change proceeds) STALE marks and review flags applied across the closure

## Checklist
- [ ] Target named at the most precise ID level possible
- [ ] Direct citations found across all reference locations
- [ ] Expanded to transitive closure (no new elements on the last pass)
- [ ] Each downstream element classified with its required action
- [ ] Blast-radius count reported; keystone flagged if large
- [ ] If the change proceeded, all forced actions fired and `verify-traceability` re-run

## Related
- Skill: `skills/analyze-impact.md`
- Procedure: `procedures/verify-traceability.md` (integrity check, run after a change)
- Procedure: `procedures/add-plan.md` (re-derivation the impact forces on plans)
