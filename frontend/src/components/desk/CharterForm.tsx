// The charter form — what the Continuous toggle swaps the question field for
// (AGENT-012 §2b; AGENT-014 §2.1; ADR-0037 §2).
//
// It is a *form for the same object*, not a different screen. There is no
// separate page for continuous research and no third word for it anywhere in
// this file or any other under `components/desk/`; a test greps for that.
import { Badge } from '@/components/primitives/Badge'
import { Field, Input, NumberField, Switch } from '@/components/primitives/Field'
import { Select } from '@/components/primitives/Overlay'
import type { Autonomy, Charter, Objective, Workflow } from '@/api/desk'
import { DEFAULT_DAY_BUDGET_USD, WORKFLOWS } from '@/api/desk'

const OBJECTIVES: Array<{ value: Objective; label: string }> = [
  { value: 'find_deployable', label: 'find deployable' },
  { value: 'build_knowledge', label: 'build knowledge' },
  { value: 'maintain', label: 'maintain' },
  { value: 'mixed', label: 'mixed' },
]

const AUTONOMIES: Array<{ value: Autonomy; label: string }> = [
  { value: 'supervised', label: 'supervised' },
  { value: 'notify', label: 'notify' },
  { value: 'autonomous', label: 'autonomous' },
]

function listField(
  label: string,
  value: string[],
  onChange: (next: string[]) => void,
  hint?: string,
) {
  return (
    <Field label={label} hint={hint}>
      <Input
        small
        aria-label={label}
        value={value.join(', ')}
        placeholder="comma separated"
        onChange={(e) =>
          onChange(
            e.target.value
              .split(',')
              .map((s) => s.trim())
              .filter(Boolean),
          )
        }
      />
    </Field>
  )
}

const DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']

/**
 * The 7 × 24 schedule grid (AGENT-014 §2.1), Monday first — the order
 * `desk_charter::windows_from_grid` reads it in, and ISO weekday 1.
 *
 * Empty is **any time**, matching `Schedule::default`: the server reads an empty
 * grid as always open, not as never. So the grid starts empty and says so, and
 * the first click materialises the full 7 × 24 — sending a partial grid is a
 * 400, because a grid the wrong size is refused rather than guessed at.
 */
export function ScheduleGrid({
  value,
  onChange,
}: {
  value: boolean[][]
  onChange: (next: boolean[][]) => void
}) {
  const full = value.length === 7 && value.every((d) => d.length === 24)
  const grid = full ? value : Array.from({ length: 7 }, () => Array<boolean>(24).fill(false))
  const open = full && value.some((d) => d.some(Boolean))
  function toggle(day: number, hour: number) {
    onChange(grid.map((d, i) => (i === day ? d.map((h, j) => (j === hour ? !h : h)) : d.slice())))
  }
  return (
    <div data-testid="schedule-grid">
      <p className="sub" style={{ margin: 0 }} data-testid="schedule-reading">
        {open
          ? `${grid.flat().filter(Boolean).length} of 168 hours open, in the charter's timezone`
          : 'any time — no hour is marked, and an empty schedule is always open'}
        {open && (
          <>
            {' '}
            <button type="button" className="chip" onClick={() => onChange([])}>
              clear (any time)
            </button>
          </>
        )}
      </p>
      <div
        role="grid"
        aria-label="Schedule"
        style={{ display: 'grid', gridTemplateColumns: 'auto repeat(24, minmax(8px, 1fr))', gap: 1, marginTop: 'var(--s-1)' }}
      >
        {grid.map((hours, day) => (
          <div role="row" key={DAYS[day]} style={{ display: 'contents' }}>
            <span className="lbl" role="rowheader" style={{ paddingRight: 'var(--s-1)' }}>
              {DAYS[day]}
            </span>
            {hours.map((on, hour) => (
              <button
                key={hour}
                type="button"
                role="gridcell"
                aria-pressed={on}
                aria-label={`${DAYS[day]} ${String(hour).padStart(2, '0')}:00`}
                title={`${DAYS[day]} ${String(hour).padStart(2, '0')}:00–${String(hour + 1).padStart(2, '0')}:00`}
                onClick={() => toggle(day, hour)}
                style={{
                  height: 12,
                  padding: 0,
                  border: '1px solid var(--line-default)',
                  background: on ? 'var(--bg-accent)' : 'transparent',
                  cursor: 'pointer',
                }}
              />
            ))}
          </div>
        ))}
      </div>
    </div>
  )
}

export function CharterForm({
  value,
  onChange,
  /** Set when the chosen profile's judgement score is below threshold. */
  autonomousDisabledReason,
  replenishDisabledReason,
}: {
  value: Charter
  onChange: (next: Charter) => void
  autonomousDisabledReason?: string
  /**
   * Set while the server refuses `replenish` (it does until S-11.11 builds the
   * survey run that refills an agenda). Offering a switch the route answers 400
   * to is a control that reaches a refusal, so it is drawn disabled with why.
   */
  replenishDisabledReason?: string
}) {
  const set = <K extends keyof Charter>(k: K, v: Charter[K]) => onChange({ ...value, [k]: v })

  return (
    <div className="charter-form" data-testid="charter-form">
      {listField('Instruments', value.instruments, (v) => set('instruments', v))}
      {listField('Asset classes', value.asset_classes, (v) => set('asset_classes', v))}
      {listField('Families', value.families, (v) => set('families', v))}
      {listField('Timeframes', value.timeframes, (v) => set('timeframes', v))}

      <Field label="Objective">
        <Select<Objective>
          value={value.objective}
          onChange={(v) => set('objective', v)}
          ariaLabel="Objective"
          small
          options={OBJECTIVES.map((o) => ({ value: o.value, label: o.label }))}
        />
      </Field>

      <Field label="Autonomy" hint={autonomousDisabledReason}>
        <Select<Autonomy>
          value={value.autonomy}
          onChange={(v) => set('autonomy', v)}
          ariaLabel="Autonomy"
          small
          options={AUTONOMIES.map((a) => ({
            value: a.value,
            label: a.label,
            disabled: a.value === 'autonomous' && !!autonomousDisabledReason,
          }))}
        />
      </Field>

      <Field
        label="Day budget"
        hint={`platform default $${DEFAULT_DAY_BUDGET_USD}/day; never required`}
      >
        <NumberField
          small
          aria-label="Day budget in dollars"
          value={String(value.budget_usd_day)}
          onValueChange={(n) => set('budget_usd_day', Number(n) || DEFAULT_DAY_BUDGET_USD)}
        />
      </Field>

      <Field label="Total budget" hint="blank means only the day budget applies">
        <NumberField
          small
          aria-label="Total budget in dollars"
          value={value.budget_usd_total === null ? '' : String(value.budget_usd_total)}
          onValueChange={(n) => set('budget_usd_total', n === '' ? null : Number(n))}
        />
      </Field>

      <Field label="Concurrency">
        <NumberField
          small
          aria-label="Concurrency"
          value={String(value.concurrency)}
          onValueChange={(n) => set('concurrency', Number(n) || 1)}
        />
      </Field>

      <Field label="Review every (runs)">
        <NumberField
          small
          aria-label="Review every"
          value={value.review_every === null ? '' : String(value.review_every)}
          onValueChange={(n) => set('review_every', n === '' ? null : Number(n))}
        />
      </Field>

      <Field label="Timezone">
        <Input small aria-label="Timezone" value={value.timezone} onChange={(e) => set('timezone', e.target.value)} />
      </Field>

      <div className="full">
        <Field label="Permitted workflows" hint="defaults to everything the profile supports">
          <div style={{ display: 'flex', flexWrap: 'wrap', gap: 'var(--s-2)' }}>
            {WORKFLOWS.map((w) => {
              const on = value.workflows.includes(w)
              return (
                <button
                  key={w}
                  type="button"
                  className="chip"
                  aria-pressed={on}
                  style={{ opacity: on ? 1 : 0.5, cursor: 'pointer' }}
                  onClick={() =>
                    set(
                      'workflows',
                      on ? value.workflows.filter((x) => x !== w) : ([...value.workflows, w] as Workflow[]),
                    )
                  }
                >
                  {w}
                </button>
              )
            })}
          </div>
        </Field>
      </div>

      <div className="full">
        <Field label="Schedule" hint="when the session may start runs; leave empty for any time">
          <ScheduleGrid value={value.schedule} onChange={(g) => set('schedule', g)} />
        </Field>
      </div>

      <div className="full">{listField('Seeds', value.seeds, (v) => set('seeds', v), 'the questions to start from')}</div>
      <div className="full">{listField('Exclusions', value.exclusions, (v) => set('exclusions', v))}</div>
      <div className="full">
        {listField('Standing constraints', value.standing_constraints, (v) => set('standing_constraints', v))}
      </div>

      <div className="full">
        <Field
          label="delta_practical"
          hint="a suggestion; each run seals its own"
          labelAside={<Badge tone="outline">suggestion</Badge>}
        >
          <Input
            small
            aria-label="delta_practical"
            value={value.delta_practical}
            onChange={(e) => set('delta_practical', e.target.value)}
          />
        </Field>
      </div>

      <div className="full" style={{ display: 'flex', alignItems: 'center', gap: 'var(--s-2)' }}>
        <Switch
          checked={value.replenish}
          onCheckedChange={(v) => set('replenish', v)}
          label="Keep generating questions"
          disabled={!!replenishDisabledReason && !value.replenish}
        />
        <span style={{ fontSize: 'var(--t-12)' }}>Keep generating questions</span>
        <span className="sub" data-testid="replenish-hint">
          {replenishDisabledReason ?? 'when the agenda empties, the session writes more rather than ending'}
        </span>
      </div>
    </div>
  )
}
