# ADR-P7-01 … ADR-P7-05: web access, untrusted content, the skill library, the skill sandbox, and evidence tools

**Status:** Accepted (reconstructed 2026-09-14)
**Why this file exists:** these five identifiers are cited by `crates/mcp-server/src/{lib,taxonomy}.rs`, `crates/api/src/routes/{mod,web_and_skills,evidence}.rs`, `crates/backtest/src/forensics.rs`, `crates/harness/tests/{conformance,failure_injection}.rs` and every `config/profiles/*.yaml`, and had no written record. The decisions below are reconstructed from that code and its comments, dated to the code's own date (2026-09-14). Where the code is the only source, the reasoning quotes it. They are appended to `ADR-INDEX.md` in the P-series.

---

## ADR-P7-01: The platform gets an outbound channel — two tools, classified `Outbound`, egress-checked

**Decision.** `web_search` and `fetch_url` are added to the catalogue (namespace `web`) and to the internal agent's profile. They are the only two routes in the platform that reach the internet (`routes/mod.rs:99-102`). They are classified `Risk::Outbound`, a fourth risk class introduced for them, and a test asserts they are the only outbound tools (`taxonomy.rs` `the_only_outbound_tools_are_the_two_web_tools`).

**Rejected alternative.** Keep the outbound leg of the lethal trifecta cut (the state before 2026-09-14: private data + untrusted content, no exfiltration path).

**Reason.** From `taxonomy.rs:108-117`: "a research agent that cannot read a fee schedule or check whether its discovery is a 2011 paper is worth much less." The leg is restored *narrowed rather than cut*: every destination passes `webfetch::egress::check`, a pure deny-by-default function over a parsed URL and its resolved addresses — private, loopback, link-local and unique-local ranges refused in both address families (IPv4-mapped included), redirects re-checked hop by hop, credentials in a URL refused, the client pinned to the checked addresses to close DNS rebinding. Profiles record `outbound_channel: true` with this mitigation text; the conformance test that asserted "no profile holds all three legs" was moved, not deleted (`conformance.rs:255`).

**Consequences.** Every profile's trifecta audit now reads all three legs live. The residual, accepted risk is persuasion by fetched content (see P7-02). AGENT-006 §2.1 fixes the namespace at two tools.

## ADR-P7-02: Fetched content is a distinct untrusted type that cannot be printed by accident

**Decision.** Everything the web tools return is `webfetch::Untrusted`: a type with no `Display`, one deliberately long accessor, and a single way into a prompt — inside a provenance fence that states it is data (`harness::provenance::Tagged`, `Provenance::Untrusted`). Untrusted content is refused from the `Charter` and `Subtask` sections (`context::provenance_allowed`) and may only appear in working state and reference blocks.

**Rejected alternative.** Return fetched text as a string like any other tool result and rely on the prompt to say "this is untrusted".

**Reason.** From the profile mitigation: "that makes the boundary legible at every call site and cannot make a model that ignores the distinction ignore it less." The type makes forgetting the fence a compile error; the section rule makes an injection unable to reach the instructions layer.

**Consequences.** Publishing still happens only by writing to `outputs/` and calling `deliver`, which a human approves. AGENT-008 §3.4 rung 2 is this decision applied to every tool result.

## ADR-P7-03: A Python skill library — pure transformations, no network, admitted by scan, every run recorded

**Decision.** `create_skill`, `list_skills`, `run_skill` (namespace `skills`). A skill is a Python file defining `run(payload: dict) -> dict`, with a declared contract (`outputs` non-empty), stored by content hash (`crates/skills` `SkillHash`), executed by a runner (P7-04), and every run — including failures — written to `mlops.skill_run`. Skills are classified `Write` (they append a run record) and explicitly **not** `Outbound`: "a skill has no network and no database" (`taxonomy.rs:118-125`), asserted by a test.

**Rejected alternative.** Give the agent a general code-execution tool immediately.

**Reason.** A bounded contract (dict in, dict out, standard library + numpy + pandas) is auditable and reusable; a run record per execution makes a skill's history evidence. The static import scan on admission (`crates/skills/src/lib.rs:207-274`) refuses `socket`, `requests`, `subprocess`, `ctypes`, `multiprocessing` and friends — and says of itself "this is not the sandbox."

**Consequences.** ADR-0028's verify job (unit tests, A/B replay, reviewer) remains unbuilt; AGENT-007 §5 specifies it. AGENT-005 §8 and AGENT-006 §5.9 generalise the runner into `run_python` behind the `code_execution` profile tier.

## ADR-P7-04: The skill sandbox is two layers plus one that is not a boundary — and says so

**Decision.** The runner (`crates/skills/src/runner.rs`, `apps/skill-runner/skill_runner.py`) applies: (1) in the child, POSIX rlimits — address space, CPU, file size, `RLIMIT_NPROC=0`; (2) in the parent, the wall clock, kill-then-reap, because "a child asked to police its own deadline can be prevented from doing so by the code it is policing"; (3) a monkeypatched socket layer that is "real enough to stop an accident and a casual attempt, and not a boundary." Limits: 30 s wall, 25 s CPU, 512 MB, 4 MB output. `env_clear()` with `PATH` only; the skill's `print` goes to a captured buffer so it "cannot forge" the result channel. On Windows the `resource` module is absent and the runner reports that resource limits are unavailable.

**Rejected alternative.** Claim the runner is a security boundary.

**Reason.** From `skill_runner.py:22-41`: "Three layers, and only two of them are real … a skill is *sandboxed against mistakes* by this file and *sandboxed against intent* by the container it runs in." The decision is to be honest about which layer does what, so nobody builds on the wrong one.

**Consequences.** On a Windows dev box only the wall clock holds. AGENT-010 §10 makes the container the executor for the `full` tier and keeps the host runner for templated/denied tiers, which is the container the docstring assumed. HARNESS_CONFORMANCE item 5 ("destructive-shell-pattern denial is not implemented") stays open until then.

## ADR-P7-05: Evidence tools — the falsification vocabulary gets the numbers behind it

**Decision.** `get_trade_list`, `probe_data_quality`, `find_similar_trials` (namespace `falsification`, all `Read`, all cheap), backed by `crates/backtest/src/forensics.rs` and `crates/api/src/routes/evidence.rs`. Each answers one defect class of `report_kill` with a measurement: P&L concentration in the top-N trades (`data_artifact` kills — "a strategy whose edge disappears when you remove its best six bars did not have an edge, it had those six bars"), impossible/zero-volume/stale/reverting bars with quotable timestamps, and prior evaluations of the same config hash or neighbouring experiments ("re-running it produces no new market evidence, still spends trial budget, and inflates the effective trial count"). None returns a verdict.

**Rejected alternative.** Ship `report_kill` with its closed defect vocabulary and let the agent argue from the backtest summary.

**Reason.** From `routes/evidence.rs:1-4` and `taxonomy.rs:101-107`: the falsification sprint "shipped a defect vocabulary and no way to gather evidence for it." A kill that cannot cite a measurement is an opinion; these tools make each defect class checkable in under ten minutes by a human, which is `report_kill`'s own reproduction requirement.

**Consequences.** `find_similar_trials` is the typed core of AGENT-007's hypothesis dedup; `get_trade_list` is the base of AGENT-006's `ablation_sweep`; `probe_data_quality` is step 1 of the `leakage_audit` workflow.
