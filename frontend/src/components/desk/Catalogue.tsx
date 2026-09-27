// The catalogue page (AGENT-012 §9; task S-6.19).
//
// Namespaces as columns, tools as cards, and **simulate exposure**: pick a
// profile and a goal, see the slice the router would expose and *why*. The
// "why" is the part that matters — an exposure list without reasons is a list
// nobody can argue with, and the router's mistakes stay invisible.
//
// Track 23 owns the catalogue's source of truth; this consumes it and does not
// redefine it.
import { useMemo, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Input, SearchInput } from '@/components/primitives/Field'
import { Select } from '@/components/primitives/Overlay'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { conversationsApi } from '@/api/agent'
import { deskApi, type Mode, type ToolCard } from '@/api/desk'
import { Segmented } from '@/components/primitives/Segmented'
import { toolsVisibleInMode, whyHidden } from '@/lib/desk/rings'
import { RiskBadge } from './Shared'

export function Catalogue() {
  const [q, setQ] = useState('')
  const [mode, setMode] = useState<Mode>('research')
  const [profile, setProfile] = useState('')
  const [goal, setGoal] = useState('')
  const [simulated, setSimulated] = useState<{ tools: ToolCard[]; reasons: Record<string, string> } | null>(null)

  const cat = useQuery({ queryKey: ['desk', 'catalogue'], queryFn: () => deskApi.catalogue() })
  const profiles = useQuery({
    queryKey: ['desk', 'profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
    staleTime: 60_000,
  })

  const tools = cat.data?.tools ?? []
  const visible = toolsVisibleInMode(tools, mode)
  const hidden = tools.filter((t) => !visible.includes(t))
  // Keyed off the query result and the two controls, not off `visible`.
  // `toolsVisibleInMode` returns a fresh array every render, so a dep on
  // `visible` changed on every render and the memo never held anything — it was
  // the cost of a memo with none of the benefit. Recomputing the filter inside
  // is cheaper than the array identity it was guarding.
  const namespaces = useMemo(() => {
    const m = new Map<string, ToolCard[]>()
    const needle = q.toLowerCase()
    for (const t of toolsVisibleInMode(cat.data?.tools ?? [], mode)) {
      if (!t.name.toLowerCase().includes(needle)) continue
      const list = m.get(t.namespace) ?? []
      list.push(t)
      m.set(t.namespace, list)
    }
    return [...m.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [cat.data, mode, q])

  return (
    <div className="desk-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-3)' }}>
      <div className="desk-actions">
        <SearchInput value={q} onValueChange={setQ} placeholder="Search tools" />
        <Segmented<Mode>
          value={mode}
          onChange={setMode}
          ariaLabel="Mode"
          options={[
            { value: 'chat', label: 'Chat' },
            { value: 'research', label: 'Research' },
          ]}
        />
      </div>

      {/* `/api/catalogue` has been served since S-6.19, so a null here is the
          request failing, not the endpoint missing. The old copy sent a reader
          away to wait for a phase that had already landed. */}
      {cat.data === null && (
        <EmptyState
          message="The catalogue did not answer"
          detail="/api/catalogue is served (S-6.19), so this is the request failing rather than the endpoint missing — check the API logs. Nothing below is a claim that the registry is empty."
        />
      )}
      {cat.data !== null && cat.data !== undefined && tools.length === 0 && (
        <EmptyState
          message="The catalogue answered with no tools"
          detail="A fact about the registry rather than about this mode or this search."
        />
      )}

      <Panel>
        <PanelHeader title="Simulate exposure" />
        <PanelBody>
          <div className="desk-actions">
            <Select
              value={profile}
              onChange={setProfile}
              ariaLabel="Profile"
              small
              width={220}
              options={(profiles.data?.profiles ?? []).map((p) => ({ value: p.model_id, label: p.model_id }))}
            />
            <Input small aria-label="Goal" placeholder="goal" value={goal} onChange={(e) => setGoal(e.target.value)} />
            <Button
              size="xs"
              onClick={async () => setSimulated(await deskApi.exposure(profile, goal))}
              disabled={!profile || !goal}
            >
              Simulate
            </Button>
          </div>
          {simulated === null ? (
            <p className="sub">
              Nothing simulated yet. The endpoint answers with the same slice the router would choose, plus the
              reason each tool was included.
            </p>
          ) : (
            <ul style={{ fontSize: 'var(--t-12)', paddingLeft: 'var(--s-4)' }}>
              {simulated.tools.map((t) => (
                <li key={t.name}>
                  <span className="mono">{t.name}</span> — {simulated.reasons[t.name] ?? 'no reason given'}
                </li>
              ))}
              {simulated.tools.length === 0 && <span className="sub">the router would expose nothing</span>}
            </ul>
          )}
        </PanelBody>
      </Panel>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(240px, 1fr))', gap: 'var(--s-3)' }}>
        {namespaces.map(([ns, list]) => (
          <Panel key={ns}>
            <PanelHeader title={ns} badges={<Badge tone="neutral">{list.length}</Badge>} />
            <PanelBody>
              {list.map((t) => (
                <div key={t.name} className="desk-card">
                  <div className="desk-card-hd">
                    <strong className="mono" style={{ fontSize: 'var(--t-12)' }}>
                      {t.name}
                    </strong>
                    <RiskBadge value={t.risk} />
                    {t.spends_trial && <Badge tone="warn">spends a trial</Badge>}
                  </div>
                  {t.notes && <p className="sub">{t.notes}</p>}
                  <p className="sub">
                    ring {t.ring}
                    {t.output_budget_tokens ? ` · output budget ${t.output_budget_tokens}` : ''}
                    {t.tiers?.length ? ` · visible to ${t.tiers.join(', ')}` : ''}
                  </p>
                </div>
              ))}
            </PanelBody>
          </Panel>
        ))}
      </div>

      {hidden.length > 0 && (
        <p className="sub">
          {hidden.length} not shown in {mode}: {hidden.map((t) => `${t.name} (${whyHidden(mode, t)})`).join('; ')}
        </p>
      )}
    </div>
  )
}
