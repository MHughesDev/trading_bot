// Null picker (D-7). Surfaces the recommended null and renders every catalog
// entry's preserves/destroys hypothesis BEFORE selection — the null is chosen and
// logged, never an invisible default. Overriding the recommendation requires a
// logged reason, which this component captures and blocks proceeding without.
import { useState } from 'react'
import { Check, Star } from 'lucide-react'
import type { NullCatalogEntry, NullKind, NullPickerView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'

export function NullPicker({
  picker,
  onChoose,
  saving,
}: {
  picker: NullPickerView
  onChoose: (kind: NullKind, overrideReason?: string) => void
  saving: boolean
}) {
  const [selected, setSelected] = useState<NullKind>(picker.chosen?.kind ?? picker.recommended)
  const [reason, setReason] = useState(picker.chosen?.override_reason ?? '')
  const isOverride = selected !== picker.recommended
  const blocked = isOverride && reason.trim().length === 0

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Null picker</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="text-xs text-text-dim">
          Recommended for this strategy type:{' '}
          <span className="font-mono text-text-muted">{picker.recommended}</span>. The null is a
          stated hypothesis — choose deliberately; an override needs a reason.
        </p>

        <div className="flex flex-col gap-2">
          {picker.catalog.map((entry) => (
            <NullOption
              key={entry.kind}
              entry={entry}
              selected={selected === entry.kind}
              onSelect={() => setSelected(entry.kind)}
            />
          ))}
        </div>

        {isOverride && (
          <div className="flex flex-col gap-1">
            <label className="text-xs text-text-dim" htmlFor="null-override-reason">
              Override reason (required — logged forever)
            </label>
            <textarea
              id="null-override-reason"
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              rows={2}
              className="rounded-md border border-border-2 bg-surface px-2 py-1 text-sm text-text"
              placeholder="Why depart from the recommended null?"
            />
          </div>
        )}

        <div className="flex items-center justify-between">
          {picker.chosen ? (
            <span className="text-xs text-text-dim">
              chosen: <span className="font-mono">{picker.chosen.kind}</span>
              {picker.chosen.was_override && ' (override logged)'}
            </span>
          ) : (
            <span className="text-xs text-text-dim">no null chosen yet</span>
          )}
          <Button
            size="sm"
            disabled={saving || blocked}
            onClick={() => onChoose(selected, isOverride ? reason.trim() : undefined)}
          >
            {picker.chosen ? 'Update null' : 'Choose null'}
          </Button>
        </div>
      </CardContent>
    </Card>
  )
}

function NullOption({
  entry,
  selected,
  onSelect,
}: {
  entry: NullCatalogEntry
  selected: boolean
  onSelect: () => void
}) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className={`flex flex-col gap-1 rounded-md border p-3 text-left transition-colors ${
        selected ? 'border-accent bg-accent/10' : 'border-border-2 bg-surface-2 hover:bg-border'
      }`}
    >
      <div className="flex items-center gap-2 text-sm font-medium text-text">
        {selected && <Check className="h-3.5 w-3.5 text-accent" />}
        <span className="font-mono">{entry.kind}</span>
        {entry.recommended && (
          <Badge variant="active" className="gap-1">
            <Star className="h-3 w-3" />
            recommended
          </Badge>
        )}
      </div>
      <div className="grid grid-cols-2 gap-2 text-[11px]">
        <div>
          <span className="text-text-dim">preserves: </span>
          <span className="text-text-muted">{entry.preserves.join(', ')}</span>
        </div>
        <div>
          <span className="text-text-dim">destroys: </span>
          <span className="text-text-muted">{entry.destroys.join(', ')}</span>
        </div>
      </div>
    </button>
  )
}
