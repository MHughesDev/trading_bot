# ADR-0022: MCP as a thin authenticated client; internal agent over the same tool layer

Date: 2026-09-06
Status: Accepted

## Context

ADR-0010 declared the MCP server an unprivileged front door that calls the same
HTTP API as the UI. The original implementation violated this: `McpContext` held
its own `PgPool`, its own `BacktestManager` (a second manager over the shared
`backtest_jobs` table, whose hydrate pass marks non-terminal rows failed — a
correctness hazard), and an in-memory strategy store, so `create_strategy` never
persisted and backtests ran as `Uuid::nil()`.

Separately, we want an in-app agent: the user picks an LLM provider (OpenAI /
Anthropic / local Ollama), a model, and stores an API token; the agent then
designs strategies, launches backtests, waits out hour-long runs, reads results,
and iterates.

## Decision

1. **MCP server is a thin reqwest client of the platform API** (default
   `http://127.0.0.1:7080`) carrying a bearer **service token**
   (`PLATFORM_API_TOKEN`). It holds no DB pools or managers. Only the
   step-by-step draft builder stays process-local (a scratchpad; persistence
   happens via `POST /api/strategies` on finalize). `apply_strategy` /
   `stop_strategy` were removed — they drove a detached `InstanceManager` and
   could never trade.

2. **Service tokens** are long-lived `sessions` rows (`kind = 'service'`,
   365-day expiry, labeled, prefix-listable, revocable) minted via
   `POST /auth/service-tokens`. The existing `BearerToken` extractor validates
   them unchanged. Chosen over login-per-boot so no password sits in env files
   and tokens are individually revocable.

3. **Long-running tool calls stream keep-alives.** `wait_for_backtest` polls
   server-side (clamped to 600 s per call; agents chain calls) while the MCP
   binary streams SSE frames every 10 s — spec `notifications/progress` when the
   client sent a `progressToken` (resets Claude Code's tool timeout), comment
   frames otherwise. Without this, hour-long waits die on idle timeouts.

4. **The internal agent executes the same tool layer over loopback HTTP.** The
   driver (in `crates/api/src/agent/`) builds an `mcp-server-lib::McpContext`
   pointed at the platform's own API with a **run-scoped** service token
   (deleted on completion), so tool behavior cannot drift between front doors
   and user scoping falls out of normal auth. Backtest waits happen inside the
   driver (poll + persist status messages) so long simulations cost zero LLM
   tokens. Runs follow the `training_runs` job pattern (`agent_runs` +
   `agent_messages`, restart marks non-terminal runs failed).

5. **One unified LLM client** (`crates/llm`): enum-dispatched OpenAI /
   Anthropic / Ollama adapters behind provider-neutral `ChatRequest` /
   `ChatResponse` / tool-call types. Provider keys are stored per-user in
   `llm_credentials`, AES-256-GCM envelope-encrypted with the pre-existing
   `CredentialCrypto` (KEK from env `CRED_KEK`), verified against the live
   provider before saving, and never echoed.

## Consequences

- Strategies created via MCP are real: durable, visible in the UI, backtestable
  by slug, owned by the token's user. The nil-UUID and second-manager hazards
  are gone.
- The MCP process needs a minted token to start (`ApiClient::from_env` fails
  fast without one).
- `strategy_definitions` still has no ownership column; the authoring guide
  mandates versioned slugs (`_v2`, …) to avoid silent overwrites. User-scoped
  strategies remain future work.
- Set J (`/api/backtest/*` experiments) is intentionally not exposed via MCP
  until its executor runs real simulations (Set K).
