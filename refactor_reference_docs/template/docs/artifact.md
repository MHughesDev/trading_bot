# The Artifact

Fill this in before anything else. It does not need to be complete — it needs to be honest.
The goal is to get the most important things out of your head and into a form that can be designed against.

Each section feeds a downstream artifact: research briefs, plans, specs, or ADRs.

---

## What Are We Building?

[One to three sentences. Name the thing. Say what it does. Avoid feature lists — just describe it.]

---

## Who Uses It?

[The primary person or system that benefits. Include any secondary users if they shape the design.
One paragraph is plenty.]

---

## What Problem Does It Actually Solve?

[Not the feature list — the underlying need. Why does this have to exist?
What breaks or stays broken if it isn't built?]

---

## What Does Good Look Like?

[Three to five observable conditions that tell you the system is working as intended.
Write these so a stranger could verify them without asking you what you meant.

Number each one `SC-N` (Success Condition). Downstream artifacts cite these IDs — a plan
milestone advances an SC, a spec's acceptance criteria satisfy one. Keep the IDs stable;
append new conditions at the end rather than renumbering.]

- SC-1: [Observable condition]
- SC-2: [Observable condition]
- SC-3: [Observable condition]

---

## What Would Make This Fail?

[The most likely ways this goes wrong — technically, from a product perspective, or operationally.
Don't try to be exhaustive. Name the ones that would actually hurt.

Number each one `FM-N` (Failure Mode). Plans cite these as the source of their risks, and
research briefs are often opened to investigate one. Keep the IDs stable.]

- FM-1: [How it fails] — [technical | product | operational]
- FM-2: [How it fails]
- FM-3: [How it fails]

---

## What Don't We Know Yet?

[Open questions that need answers before you can design confidently.
Each one of these is a candidate research brief.]

---

## What Are We Deliberately Not Building?

[Explicit boundaries. "Not this" is as load-bearing as "this."
If you don't write it down, scope will drift toward it anyway.]

---

## What Are We Working Within?

[Timeline, team size, budget, existing stack, compliance requirements, non-negotiable dependencies.
Anything that shapes the design before a decision is made.]
