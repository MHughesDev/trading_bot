# Connect an AI provider (bring your own API key)

Every user of the platform runs the research desk (Chat and Research) on **their own**
Anthropic and/or OpenAI **API key**, billed to their own provider account. A key belongs to
the signed-in user who pasted it: it is tested with a real provider call before it is stored,
encrypted at rest with the platform's envelope encryption, never returned to any browser, and
never written to a log.

> Anthropic and OpenAI only allow API keys in third-party apps; subscriptions (Claude Pro/Max,
> ChatGPT Plus) can't be connected here. Get a key at
> [console.anthropic.com/settings/keys](https://console.anthropic.com/settings/keys) or
> [platform.openai.com/api-keys](https://platform.openai.com/api-keys).

This page has two halves: what a **user** does, and what an **operator** has to configure first.

---

## For users: connect your key

1. Create an API key at your provider:
   - Anthropic: <https://console.anthropic.com/settings/keys>
   - OpenAI: <https://platform.openai.com/api-keys>
2. In the app, open **Settings → AI providers**. The desk's **Connect a provider** button and the
   **Manage AI providers** link on the desk's Models page both go there (`/settings#ai`).
3. On the provider's card, paste the key into **API key** and press **Save & test**. That is one
   step: the platform lists the provider's models with your key, which is a real authenticated
   call that costs nothing, and stores the key only if the provider accepted it.
   - Accepted: the card reads **Connected · key ending ••••abcd · verified just now**.
   - Refused: the card reads **Key rejected: …** with the provider's own reason (your key is cut
     out of it). Nothing is stored, and the key you pasted stays in the field so you can fix it.
4. Pick the **Default model** for that provider. The first provider you connect becomes your
   default; **Make default** on another card changes it.
5. Go to the desk. The composer's model picker shows the provider and model you will run on
   (for example **OpenAI · gpt-5**). Choose Chat or Research and send.

On a connected card:

| Action | What it does |
|---|---|
| **Test again** | Re-checks the stored key with a real call. A key revoked at the provider shows **Key rejected: …** until it passes again. |
| **Replace** | Opens the key field. The old key stays in use until the new one passes **Save & test**. |
| **Remove** | Asks first, then deletes the key from this platform. It stays valid at the provider until you revoke it there. |

If no key is connected (and no local model is reachable), the desk shows **Connect an AI
provider to start** in place of the question box and the send button.

An existing conversation keeps the provider and model it was opened on. If you remove that
provider's key, the conversation shows the call to action instead of a send button until you
connect one again.

---

## For operators: what must be configured

### `CRED_KEK` (required for anyone to save a key)

`CRED_KEK` is the key-encryption key for the envelope encryption of every stored credential
(`crates/api/src/credentials/crypto.rs`): each key is encrypted with its own random data key,
and that data key is wrapped with `CRED_KEK`.

- Format: **exactly 64 hex characters** (32 random bytes).
- Read from the platform process's environment **once at startup**. Setting it later needs a
  restart.
- Without it (or with a malformed value), every save answers `503 cred_kek_missing` and the
  AI providers panel prints the operator's instruction verbatim, for example:
  *"Keys can't be saved: credential encryption isn't configured on this platform. The operator
  must set CRED_KEK to 64 hex characters (32 random bytes, e.g. `openssl rand -hex 32`) in the
  platform's environment and restart it."*
- **Keep it stable and backed up.** Losing it, or changing it, makes every stored key
  undecryptable: the platform then answers `409 undecryptable` and the user must **Replace**
  their key. Keep it in a secret store, never in the repository.

Generate one:

```bash
openssl rand -hex 32
```

```powershell
# Windows, without openssl
$b = New-Object byte[] 32
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b)
($b | ForEach-Object { $_.ToString('x2') }) -join ''
```

### Everything else the platform needs

| Variable | Purpose |
|---|---|
| `DATABASE_URL` | The platform's Postgres, as the owner. Migrations (including `0250_llm_credentials_verification.sql`) run on boot. |
| `PLATFORM_DB_APP_PASSWORD` | Password for the restricted `platform_app` role the platform runs as after migrating. |
| `CLICKHOUSE_URL` | Market data store. |
| `TBOT_PROFILES_DIR` | Optional. Capability profiles, default `config/profiles` (run from the repo root). `gpt-5.yaml` and `claude-opus-5.yaml` are the hosted frontier profiles. |
| `TBOT_AGENT_PROVIDER`, `TBOT_AGENT_MODEL` | Optional server-side defaults for a conversation created **without** a provider. The desk always sends the user's choice, so these only matter for API callers. |

No provider key is configured by the operator. There is no platform-wide Anthropic or OpenAI
key for the desk: each user's runs use that user's key.

### Starting the platform for a live test

From the repository root (Postgres, ClickHouse, NATS and Redis up, for example via
`docker compose up -d`):

```powershell
$env:CRED_KEK = '<64 hex characters, generated as above, kept somewhere safe>'
$env:DATABASE_URL = 'postgres://trading:trading@localhost:5432/trading'
$env:PLATFORM_DB_APP_PASSWORD = '<platform_app password>'
cargo run -p platform        # API on http://127.0.0.1:7080 (config/local.toml)
```

In a second terminal:

```powershell
cd frontend
npm run dev                  # http://localhost:5173, proxies /api and /auth to :7080
```

Sign in, then follow **For users** above.

### Routes

| Route | What it does |
|---|---|
| `GET /api/llm/credentials` | The caller's providers: `configured`, `key_last4`, `verified_at`, `last_checked_at`, `last_error`, `default_model`, `is_default`, plus `encryption_available` and `encryption_problem`. |
| `PUT /api/llm/credentials/{provider}` | `{api_key, base_url?}`. Verifies with a real call, then stores. `422 key_rejected` with the scrubbed reason if refused. |
| `POST /api/llm/credentials/{provider}/verify` | Re-checks the stored key; records the outcome on the row. |
| `DELETE /api/llm/credentials/{provider}` | Removes it. If it was the default, the default moves to the most recently verified remaining provider. |
| `PUT /api/llm/default` | `{provider, model?}`. The provider and model the desk starts on. `404` for a provider the user has not connected. |
| `POST /api/llm/{provider}/models` | The chat models the stored key (or a key in the body) can reach. |
| `POST /api/agent/conversations` | `{provider, model}`. The desk opens every conversation on the user's chosen engine. |

All of them act only on the bearer's own user id. Another user's key cannot be read, listed
with, verified, defaulted to, deleted, or run on; `crates/api/tests/each_user_brings_their_own_key.rs`
holds that, along with a scan of every response body, every stored error and agent message, and
every log line for the keys it used.

---

## Decisions (PROVIDERS track, 2026-10-05)

| ID | Decision | Rejected alternative |
|---|---|---|
| ADR-PROV-01 | **API keys only.** No "connect your Claude membership" (Anthropic prohibits Free/Pro/Max subscription OAuth tokens in third-party products, terms updated 2026-02-19) and no ChatGPT-membership path through a Codex-CLI-impersonating proxy. The panel says so in one line, with both key pages linked. | Offering subscription sign-in: against both providers' terms, and a token that impersonates a first-party client is not a credential this platform may hold. |
| ADR-PROV-02 | **Verification is listing models**, a real authenticated call that costs nothing and needs no model id. | A 1-token completion: spends the user's money on every check, and fails for a key that is fine but lacks access to whichever model the check picked. |
| ADR-PROV-03 | **OpenAI gets its own frontier profile** (`config/profiles/gpt-5.yaml`), and the agent manager's fallback for an unprofiled hosted model now prefers the frontier profile whose `provider` matches. | Keep every hosted model on `claude-opus-5` with the model id swapped in: Claude's window and a provider name ("anthropic-agent-sdk") that is not OpenAI's. |
| ADR-PROV-04 | **GPT-5-family / o-series temperature refusal is absorbed in `llm::openai`**: retry once without `temperature`, and remember the model for the process. | A profile field for "this model takes no temperature": it is a fact about the model, not the profile, and every unprofiled OpenAI model would still fail on its first call. |
| ADR-PROV-05 | **The default provider and model are stored per user on `llm_credentials`** (0250, one default per user by a partial unique index). | Browser `localStorage`: per device, not per user, and invisible to the server. |
| ADR-PROV-06 | **The desk opens the conversation on the chosen engine itself** (`POST /api/agent/conversations {provider, model}`) and hands its id to `POST /api/research/sessions`. | Changing `research_sessions.rs` to infer the provider: outside this track's territory. The server's no-provider default still reads `TBOT_AGENT_PROVIDER`; making it the user's default credential is the follow-up below. |
| ADR-PROV-07 | **`strict: true` structured output only for schemas inside OpenAI's strict subset**; others are sent non-strict. | Always strict: the title generator's schema (open `additionalProperties`) was a 400 on every OpenAI conversation. |
| ADR-PROV-08 | **Provider reasons are scrubbed of the key before they are returned or stored**, and `last_error` is capped at 500 characters by a CHECK. | Returning the provider body: OpenAI's 401 names the key it was sent. |
| ADR-PROV-09 | **A local model is offered on the desk only when its backend answered** (`backend_reachable === true`). | Offering every local profile: a send button that fails after the click on any machine without Ollama running. |
| ADR-PROV-10 | **A key shorter than eight characters is shown with no suffix**, and scrubbing ignores secrets under eight characters. | Always showing the last four: for a very short value that is most of it. |

### Follow-ups outside this track

- `crates/api/src/routes/research_sessions.rs` (`create`, the `None => { let provider = default_provider(); … }`
  branch) and `crates/api/src/agent/routes.rs` (`create_conversation`) still default to
  `TBOT_AGENT_PROVIDER` when no provider is sent. The desk always sends one, so the product path is
  correct; an API caller who omits it gets Anthropic even with only an OpenAI key. The fix is to
  default to the user's `is_default` credential (`LlmCredentialStore::status`).
- `research_sessions.rs`'s continuous-research frontier check looks the model up by exact id, so a
  continuous session on an OpenAI model other than `gpt-5` is refused with "no profile named … is
  loaded". It should resolve through the same provider-matched frontier fallback as
  `AgentManager::profile_for`.
- `config/prices.yaml` has no OpenAI rows, so OpenAI runs are unpriced: token budgets bind, the
  dollar cap does not. Add verified rows to price them.
