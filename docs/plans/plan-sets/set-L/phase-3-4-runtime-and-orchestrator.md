# Phases 3–4 — LLM proxy, container runtime, orchestrator and agent-host

**Completion: components built and tested in isolation; no live agent session has run**

**Requirement IDs:** RT-01…RT-09, RT-22, RT-23 (Phase 3); RT-10…RT-21 (Phase 4);
CX-01…CX-14 partially.
**Spec:** [AGENT-001](../../../specs/AGENT-001-agent-runtime.md), ADR-0024.

---

## What was built

| Piece | Where |
|---|---|
| LLM proxy: credential injection, budget, usage, TTL rewrite | `crates/api/src/llm_proxy.rs` |
| Container image | `infra/agent-image/Dockerfile`, `requirements.txt` |
| Session orchestrator: token minting, lifecycle, container args, event capture | `crates/api/src/orchestrator.rs` |
| In-container host: SDK options, hooks, event stream | `apps/agent-host/` |

## The proxy is what makes "no secrets in the sandbox" true

The container's `ANTHROPIC_BASE_URL` points at `/llm` and its `ANTHROPIC_AUTH_TOKEN`
is the session token, which is useless anywhere else. The proxy authenticates it,
swaps in the real credential from the encrypted store, checks the budget **before**
the upstream call, and records usage after.

Budget is checked in front of the request on purpose: spend reported by the thing
doing the spending is not a control.

**RT-22, the cache TTL rewrite.** The Agent SDK exposes no cache-TTL option
(AGENT-001 §23), so the proxy rewriting `cache_control.ttl` is the only place a 1-hour
TTL can be chosen. It rewrites TTL and nothing else: the CLI decides where its cache
breakpoints go, and moving them would silently change what is cached and what it
costs. `rewriting_moves_no_breakpoints` pins that. Beta headers are merged rather
than replaced, because dropping a beta the client asked for looks like a model bug.

## The sandbox, demonstrated

The image builds (2.9 GB) and the full research stack imports. The containment
properties were then tested directly rather than asserted:

| Property | Result |
|---|---|
| Write to `/` with `--read-only` | `Read-only file system` |
| Reach `api.anthropic.com` on `tbot-agent-net` | `gaierror` — does not resolve |
| Effective capabilities with `--cap-drop ALL` | `CapEff: 0000000000000000` |
| Process user | uid 10001 |

`container_args` is a pure function returning the `docker run` arguments, so those
flags are unit-tested without a daemon — including
`no_provider_credential_reaches_the_container`, which asserts no `ANTHROPIC_API_KEY`,
`CLAUDE_CODE_OAUTH_TOKEN` or `CRED_KEK` appears anywhere in the command, and
`only_the_workspace_volume_is_mounted`, which would catch a docker-socket mount.

## The compaction mechanism the spec did not anticipate

SDK 0.2.152 has **no `PostCompact` hook**. Research state survives a compaction
because `PreCompact` sets summariser instructions and records that one happened, and
the next `UserPromptSubmit` prepends NOTEBOOK §1 read fresh from disk. That is
strictly better than a post-compaction hook: it does not depend on what the
summariser chose to keep. `test_compaction_state_round_trips` covers the pairing,
including that it fires once rather than on every subsequent turn.

## One image bug found by running it

`lightgbm` and `xgboost` install cleanly on `python:3.11-slim` and then fail at
*import* with `libgomp.so.1: cannot open shared object file` — the OpenMP runtime is
not in the slim image. Without the fix this would have first appeared inside a
running research session. `libgomp1` is now installed explicitly, with a comment
saying why.

## What has not happened

**No live agent session has run.** That needs an Anthropic credential in the
encrypted store, which this work did not have and did not ask for. Consequently the
following are written and unit-tested but not exercised end to end:

- the proxy's upstream path (credential injection, real token accounting);
- SDK session start, resume, steering and interrupt;
- the hooks firing inside a real session;
- `run_session` driving a container and capturing its event stream.

The pieces either side of that gap *are* verified: options build against the real
SDK, the image runs, the sandbox holds, tokens mint with the right scopes, and `tbot`
works against the live platform.

**Also not built:** the orchestrator's HTTP surface (`/api/agent/sessions/*`), the
event bridge to the UI, steering and `ask_user`, the approval flow, resume-on-restart,
the `final_report` validator (RT-15), sub-agent definitions, and skill materialisation.
