# Skill: Analyze Impact

## Purpose
Predict the full downstream consequence of a proposed change before making it, by walking the
traceability graph backward from the target to everything that depends on it.

## When to Use
- Before editing or removing any element with an ID: `SC-N`, `FM-N`, a spec or spec section,
  an `ADR-NNNN`, or a `Q-N`
- When a stakeholder asks "how hard would it be to change X?"
- When deciding between two designs and you want to know which one is cheaper to change later
- Before resolving an open question in a way that might invalidate completed work

---

## Procedures Used
- `procedures/analyze-impact.md`

---

## Workflow

### 1. Run impact analysis *before* the change, not after
The whole point is foresight. Running it after you have already edited the target tells you what
you broke; running it before tells you what it will cost. Always run it first.

### 2. Be precise about the target
The blast radius of one requirement (`FEAT-001 §3.1.C`) is far smaller than the whole spec
(`FEAT-001`). Analyze at the finest ID that captures the actual change — overstating the target
inflates the blast radius and may scare you off a cheap change.

### 3. Read the blast-radius number as a coupling signal
A small closure means the target is loosely coupled and safe to change. A large closure means it
is load-bearing — a keystone — and the change is expensive. This number is design feedback:
if a single success condition touches dozens of elements, the design may be over-coupled.

### 4. Decide before you act
Use the report to make a real decision: is the change worth the blast radius? If yes, make it and
fire every forced action (STALE marks, spec reviews). If the cost is surprising, that itself is
worth surfacing to the user before proceeding.

### 5. Verify after
After applying a change and its forced actions, run `skills/verify-traceability.md` to confirm the
closure is clean and nothing was left dangling.

---

## Tips

- Impact analysis and provenance (`skills/explain-provenance.md`) are inverse walks of the same
  graph: provenance walks *up* (why does this exist?), impact walks *down* (what depends on this?).
- The re-derivation triggers in `add-spec.md` / `add-adr.md` are a one-hop special case of this
  procedure. Use full impact analysis when a change is large or its reach is unclear.
- A change with a zero blast radius on an element that *should* have dependents is a smell — it may
  mean the element is an orphan that nothing actually uses.
