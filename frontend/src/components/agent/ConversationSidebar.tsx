// The conversation list.
//
// Two jobs: start a new chat, and show which existing ones have an agent working in
// them. The second matters more than it looks — the whole point of detaching the run
// from the tab is that a user can leave, and a list that cannot say "this one is
// still going" makes them open every row to find out.

import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { MessageSquarePlus, Loader2, Trash2, Cpu, Cloud } from 'lucide-react'
import { conversationsApi, type AgentProfile, type Conversation } from '@/api/agent'
import { Select } from '@/components/primitives/Overlay'

interface Props {
  selected: string | null
  onSelect: (id: string) => void
}

/** Rough "how long ago", without pulling in a date library for six cases. */
function ago(iso: string): string {
  const secs = Math.max(0, (Date.now() - new Date(iso).getTime()) / 1000)
  if (secs < 60) return 'just now'
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`
  if (secs < 86_400) return `${Math.floor(secs / 3600)}h ago`
  if (secs < 604_800) return `${Math.floor(secs / 86_400)}d ago`
  return new Date(iso).toLocaleDateString()
}

export function ConversationSidebar({ selected, onSelect }: Props) {
  const qc = useQueryClient()

  const { data, isLoading } = useQuery({
    queryKey: ['conversations'],
    queryFn: () => conversationsApi.list().then((r) => r.data.conversations),
    // Polled rather than pushed: the only thing that changes without the user acting
    // is whether an agent is still working, and that is one boolean per row.
    refetchInterval: 5000,
  })

  // Which model the NEXT chat starts on. Per-conversation rather than global: the
  // point of having a local tier is choosing it for the work it suits, not moving
  // every conversation onto it.
  const [model, setModel] = useState<string | null>(null)

  const { data: profileData } = useQuery({
    queryKey: ['agent-profiles'],
    queryFn: () => conversationsApi.listProfiles().then((r) => r.data),
    staleTime: 300_000,
  })
  const profiles: AgentProfile[] = profileData?.profiles ?? []
  const unprofiled = profileData?.unprofiled_local_models ?? []

  // "On this machine" means the backend was asked and said yes — NOT that the
  // profile describes a local model. Those are different claims, and conflating them
  // listed the deployment-target profile (a 35B sized for a 3090) as though it were
  // installed here, next to one that actually was.
  // Local-and-present first: the whole point of the group is answering "what can
  // this machine run right now", so it should not sit below the hosted options.
  const runnable = [
    ...profiles.filter((p) => p.is_local && p.installed === true),
    ...profiles.filter((p) => !p.is_local),
  ]
  const missing = profiles.filter((p) => p.is_local && p.installed === false)
  const unknown = profiles.filter((p) => p.is_local && p.installed === null)

  const chosen =
    runnable.find((p) => p.model_id === model) ??
    runnable.find((p) => p.is_default) ??
    runnable[0] ??
    null

  const groupFor = (p: AgentProfile) => {
    if (!p.is_local) return "Hosted"
    if (p.installed === false) return "Not installed here"
    if (p.installed === null) return "Backend not reachable"
    return p.resident ? "On this machine \u00b7 loaded" : "On this machine"
  }

  const options = [...runnable, ...missing, ...unknown].map((p) => ({
    value: p.model_id,
    text: p.model_id,
    group: groupFor(p),
    // A model this machine does not have cannot be chosen. Shown rather than
    // hidden, so a profile that exists for a machine you do not own yet reads as
    // "not here" instead of vanishing.
    disabled: p.is_local && p.installed !== true,
    label: (
      <span className="flex items-center gap-2">
        {p.is_local ? (
          <Cpu className="h-3 w-3 text-fg-tertiary" />
        ) : (
          <Cloud className="h-3 w-3 text-fg-tertiary" />
        )}
        <span className="truncate">{p.model_id}</span>
      </span>
    ),
  }))

  // Installed models with no profile can be listed but never selected: the platform
  // refuses to run an unprofiled model on purpose. Saying so beats silence, which
  // reads as the model not having been detected.
  const unprofiledOptions = unprofiled.map((m) => ({
    value: `unprofiled:${m.model_id}`,
    text: m.model_id,
    group: "Installed, no profile yet",
    disabled: true,
    label: (
      <span className="flex items-center gap-2 opacity-70">
        <Cpu className="h-3 w-3 text-fg-tertiary" />
        <span className="truncate">{m.model_id}</span>
      </span>
    ),
  }))

  const create = useMutation({
    mutationFn: () =>
      conversationsApi
        .create(chosen ? { provider: chosen.provider, model: chosen.model_id } : undefined)
        .then((r) => r.data),
    onSuccess: (c) => {
      qc.invalidateQueries({ queryKey: ['conversations'] })
      onSelect(c.conversation_id)
    },
  })

  const archive = useMutation({
    mutationFn: (id: string) => conversationsApi.archive(id),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['conversations'] }),
  })

  const items: Conversation[] = data ?? []

  return (
    <div className="flex h-full min-h-0 w-72 shrink-0 flex-col gap-3">
      <button
        type="button"
        onClick={() => create.mutate()}
        disabled={create.isPending}
        className="flex items-center justify-center gap-2 rounded-lg border border-border bg-surface-2 px-3 py-2 text-sm font-medium text-text transition-colors hover:bg-surface disabled:opacity-60"
      >
        {create.isPending ? (
          <Loader2 className="h-4 w-4 animate-spin" />
        ) : (
          <MessageSquarePlus className="h-4 w-4" />
        )}
        New chat
      </button>

      {profiles.length > 1 && (
        <div className="flex flex-col gap-1">
          <Select
            ariaLabel="Model for the next chat"
            small
            value={chosen?.model_id ?? null}
            onChange={setModel}
            options={[...options, ...unprofiledOptions]}
          />
          {/* A tier can only sustain what it has shown it can sustain, so the
              picker says which, rather than presenting the models as equals and
              letting the difference surface as a refusal after the first message. */}
          {chosen && (
            <p className="px-1 text-[11px] leading-tight text-fg-tertiary">
              {chosen.tool_calling === 'multi_step'
                ? `${chosen.tier} · chains up to ${chosen.max_steps} steps`
                : `${chosen.tier} · single lookups only`}
              {/* Said out loud because the cold load is ~170s on an 11GB card, and
                  an unexplained three-minute first reply reads as a hang. */}
              {chosen.is_local &&
                (chosen.resident === true
                  ? ' · loaded, replies immediately'
                  : chosen.resident === false
                    ? ' · not loaded, first reply waits on the model'
                    : '')}
              {chosen.multi_step_evidence &&
                ` · measured ${chosen.multi_step_evidence.clean}/${chosen.multi_step_evidence.trials} on ${chosen.multi_step_evidence.device}`}
            </p>
          )}
        </div>
      )}

      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
        {isLoading && (
          <div className="px-2 py-3 text-xs text-text-dim">loading…</div>
        )}
        {!isLoading && items.length === 0 && (
          <p className="px-2 py-3 text-xs leading-relaxed text-text-dim">
            No conversations yet. Start one and describe what you want researched —
            the agent picks the instruments, the timeframes and how many backtests it
            needs.
          </p>
        )}

        {items.map((c) => {
          const active = c.conversation_id === selected
          return (
            <div
              key={c.conversation_id}
              className={`group relative rounded-lg border px-3 py-2 transition-colors ${
                active
                  ? 'border-accent/40 bg-accent/5'
                  : 'border-transparent hover:border-border hover:bg-surface-2'
              }`}
            >
              <button
                type="button"
                onClick={() => onSelect(c.conversation_id)}
                className="block w-full text-left"
              >
                <div className="flex items-center gap-1.5">
                  {/* The running dot. A row that cannot say "still going" makes
                      someone open every conversation to find out. */}
                  {c.running && (
                    <span className="relative flex h-2 w-2 shrink-0">
                      <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-accent opacity-60" />
                      <span className="relative inline-flex h-2 w-2 rounded-full bg-accent" />
                    </span>
                  )}
                  <span
                    className={`truncate text-sm ${active ? 'text-text' : 'text-text-muted'}`}
                  >
                    {c.title ?? 'New conversation'}
                  </span>
                </div>
                <div className="mt-0.5 flex items-center gap-2 text-[11px] text-text-dim">
                  <span>{ago(c.last_activity_at)}</span>
                  {c.turns > 0 && <span>· {c.turns} turn{c.turns === 1 ? '' : 's'}</span>}
                  {c.running && <span className="text-accent">· working</span>}
                </div>
              </button>

              <button
                type="button"
                title="Archive"
                onClick={(e) => {
                  e.stopPropagation()
                  archive.mutate(c.conversation_id)
                }}
                className="absolute right-2 top-2 hidden rounded p-1 text-text-dim hover:text-neg group-hover:block"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </div>
          )
        })}
      </div>
    </div>
  )
}
