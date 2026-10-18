// Models and tiers (AGENT-012 §10; task S-6.20).
//
// Providers and keys live in Settings -> AI providers (PROVIDERS track,
// 2026-10-05), where each user connects their own key. This page shows which are
// connected and links there, rather than holding a second form: two places to
// set one key are two places for it to be wrong.
//
// The acceptance that shapes the page: **a profile with no calibration row
// renders `unmeasured` rather than a default.** A default is a claim, and a
// claim about a model nobody has measured is the kind of number that ends up in
// a decision.
import { useQuery } from '@tanstack/react-query'
import { Link } from 'react-router-dom'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { conversationsApi, type AgentProfile } from '@/api/agent'
import { PROVIDERS, PROVIDERS_QUERY_KEY, llmKeysApi } from '@/api/llm'
import { AI_PROVIDERS_SETTINGS_PATH } from '@/components/providers/ConnectProviderCta'
import { cardState } from '@/components/providers/providers.helpers'
import { TierBadge } from './Shared'
import { profileIsMeasured } from '@/components/desk/Shared.helpers'

// The measurement rule lives beside the badge it feeds, in `Shared.helpers.ts`,
// because three surfaces drew this badge and all three had spelled it out
// themselves. It is imported above rather than re-exported here: a module that
// exports components *and* a plain function cannot fast-refresh, and a re-export
// counts (react-refresh/only-export-components).

export function ProfileCard({ p }: { p: AgentProfile }) {
  const measured = profileIsMeasured(p)
  return (
    <Panel>
      <PanelHeader
        title={p.model_id}
        badges={<TierBadge tier={p.tier} measured={measured} detail={p.provider} />}
      />
      <PanelBody>
        <p className="sub">
          tool calling: {p.tool_calling} · max steps {p.max_steps} · {p.is_local ? 'local' : 'hosted'}
        </p>
        <p className="sub">
          hardware admission:{' '}
          {p.is_local
            ? `${p.installed === null ? 'the backend did not answer' : p.installed ? 'installed' : 'not installed'}${
                p.resident ? ', resident' : ''
              }`
            : 'not applicable'}
        </p>
        <p className="sub">
          calibration:{' '}
          {measured
            ? `${p.multi_step_evidence?.eval} · ${p.multi_step_evidence?.clean}/${p.multi_step_evidence?.trials} clean on ${p.multi_step_evidence?.device} (${p.multi_step_evidence?.recorded})`
            : 'unmeasured — no scorecard row exists for this profile'}
        </p>
        <p className="sub">
          decode ladder, exposure budget, context budget and per-suite scorecards arrive with AGENT-009 (Set S
          phase 4). The desk shows what is recorded and nothing more.
        </p>
        {/* **Run scorecard and Run canary were deleted, not left disabled.**
            They were two `disabled` buttons carrying their reason in a `title`,
            which is a tooltip: invisible to a touch screen, invisible to a
            keyboard until focus lands, and invisible to the person who simply
            looked at the card and concluded the feature was broken rather than
            absent. A disabled control with no visible explanation is a promise
            that it will work when some condition clears, and there is no such
            condition here.
            Searched before deleting: no route in `crates/api/src` runs either
            one. `grep -rn "scorecard\|canary"` over `crates/api/src/routes`
            returns a single sentence inside `research_sessions.rs` prose, and
            `routes/mod.rs` mounts `/api/agent/profiles` (GET) and nothing
            beside it. The nearest thing that does exist,
            `POST /api/models/{id}/versions/{v}/evaluate`, belongs to the ML
            model studio and evaluates a *trading* model version by registry id
            — pointing these buttons at it would evaluate a different object
            entirely and report the answer under this profile's name.
            Asserted by `a-run-nobody-can-run-is-not-a-disabled-button`. */}
        <p className="sub" data-testid="no-scorecard-runner">
          Nothing on this platform can run a scorecard or a canary for a capability profile: no route
          serves either, so there is no button here to press. What is shown above is what has been
          recorded, and running a new measurement arrives with AGENT-009 §4.
        </p>
      </PanelBody>
    </Panel>
  )
}

export function Models() {
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
  })
  const creds = useQuery({
    queryKey: PROVIDERS_QUERY_KEY,
    queryFn: () => llmKeysApi.status(),
  })

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <Panel>
        <PanelHeader title="AI providers" />
        <PanelBody>
          <p className="sub" data-testid="providers-live-in-settings">
            Your own Anthropic or OpenAI API key runs the desk. Keys are connected, tested and removed in
            Settings, which is the one place a key is set.
          </p>
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--s-2)', margin: 'var(--s-2) 0' }}>
            {PROVIDERS.filter((m) => m.hosted).map((m) => {
              const s = creds.data?.providers.find((x) => x.provider === m.id) ?? null
              const state = creds.data === undefined ? 'unknown' : cardState(s)
              return (
                <Badge
                  key={m.id}
                  tone={state === 'connected' ? 'pos' : state === 'rejected' ? 'neg' : state === 'unknown' ? 'warn' : 'neutral'}
                >
                  {m.label}:{' '}
                  {state === 'connected'
                    ? `connected${s?.key_last4 ? ` ····${s.key_last4}` : ''}`
                    : state === 'rejected'
                      ? 'key rejected'
                      : state === 'unknown'
                        ? 'state unknown'
                        : 'not connected'}
                </Badge>
              )
            })}
          </div>
          <Link to={AI_PROVIDERS_SETTINGS_PATH}>
            <Button size="xs" variant="secondary">
              Manage AI providers
            </Button>
          </Link>
        </PanelBody>
      </Panel>

      {(profiles.data?.profiles ?? []).map((p) => (
        <ProfileCard key={p.model_id} p={p} />
      ))}
      {/* Three states, because a page that renders nothing at all for a failed
          read is a page that says "no profiles" without having asked. Nothing
          can run without a profile, so "none configured" is an alarming claim to
          make on a request that never came back. */}
      {profiles.isPending ? (
        <p className="sub">Reading the capability profiles…</p>
      ) : profiles.data === undefined ? (
        <EmptyState
          message="The profile list did not answer"
          detail="/api/agent/profiles is served, so this is the request failing rather than the surface being unbuilt. Which models may run is unknown, not none."
        />
      ) : profiles.data.profiles.length === 0 ? (
        <EmptyState message="No capability profile is configured" detail="Nothing can run until one is." />
      ) : null}

      {(profiles.data?.unprofiled_local_models ?? []).length > 0 && (
        <Panel>
          <PanelHeader title="Local models with no profile" />
          <PanelBody>
            <p className="sub">
              These are installed but no capability profile covers them, so nothing may run on them.
            </p>
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {(profiles.data?.unprofiled_local_models ?? []).map((m) => (
                <li key={m.model_id}>{m.model_id}</li>
              ))}
            </ul>
          </PanelBody>
        </Panel>
      )}
    </div>
  )
}
