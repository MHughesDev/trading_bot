import { useEffect, useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Bell, Trash2 } from 'lucide-react'
import { Modal } from '@/components/primitives/Modal'
import { Button, IconButton } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Field, Input, NumberField } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState, PanelLoading } from '@/components/primitives/States'
import { Label } from '@/components/primitives/Num'
import { price as fmtPrice, relativeTime, instrumentMeta } from '@/lib/format'
import { createAlert, deleteAlert, fetchAlerts, type PriceAlert } from '@/api/portfolio'
import { useToast } from '@/hooks/useToast'

/* =============================================================================
   Price alerts.

   Stored on the platform and evaluated server-side on its sampler tick, so an
   alert fires whether or not this tab is open — which is the only version of an
   alert worth having.
   ============================================================================= */

const KINDS = [
  { value: 'price_above' as const, label: 'Price rises above' },
  { value: 'price_below' as const, label: 'Price falls below' },
  { value: 'pct_change' as const, label: 'Moves by %' },
]

export function useAlerts() {
  return useQuery({
    queryKey: ['alerts'],
    queryFn: fetchAlerts,
    refetchInterval: 30_000,
    retry: 1,
  })
}

export function AlertDialog({
  open,
  onClose,
  instrument,
  lastPrice,
}: {
  open: boolean
  onClose: () => void
  instrument: string
  lastPrice: number | null
}) {
  const toast = useToast()
  const qc = useQueryClient()
  const meta = instrumentMeta(instrument)
  const alerts = useAlerts()
  const [kind, setKind] = useState<PriceAlert['kind']>('price_above')
  const [value, setValue] = useState('')
  const [note, setNote] = useState('')

  useEffect(() => {
    if (open) {
      setValue(lastPrice != null ? lastPrice.toFixed(meta.priceDp) : '')
      setNote('')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open])

  const create = useMutation({
    mutationFn: () => createAlert({ instrumentId: instrument, kind, value, note: note || undefined }),
    onSuccess: () => {
      toast({
        title: 'Alert armed',
        description: `${instrument} ${KINDS.find((k) => k.value === kind)?.label.toLowerCase()} ${value}`,
      })
      setNote('')
      void qc.invalidateQueries({ queryKey: ['alerts'] })
    },
    onError: () => toast({ title: 'Could not set that alert', variant: 'error' }),
  })

  const remove = useMutation({
    mutationFn: (id: string) => deleteAlert(id),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ['alerts'] }),
    onError: () => toast({ title: 'Could not delete that alert', variant: 'error' }),
  })

  const mine = (alerts.data ?? []).filter((a) => a.instrumentId === instrument)

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Alerts — ${instrument}`}
      description="Get told when this market does something, without watching it."
      footer={
        <>
          <Button onClick={onClose}>Done</Button>
          <Button
            variant="primary"
            disabled={!value}
            loading={create.isPending}
            onClick={() => create.mutate()}
            icon={<Bell size={13} aria-hidden />}
          >
            Set alert
          </Button>
        </>
      }
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
        <Field label="Trigger when">
          <Segmented
            ariaLabel="Alert kind"
            wide
            value={kind}
            onChange={(k) => setKind(k as PriceAlert['kind'])}
            options={KINDS.map((k) => ({
              value: k.value,
              label: k.label,
              disabled: k.value === 'pct_change',
              title:
                k.value === 'pct_change'
                  ? 'Percentage alerts need a reference price the platform does not retain per alert yet.'
                  : undefined,
            }))}
          />
        </Field>

        <NumberField
          label="Price"
          unit={meta.quote ?? 'USD'}
          value={value}
          onValueChange={setValue}
          dp={meta.priceDp}
          step={Math.pow(10, -meta.priceDp)}
          hint={lastPrice != null ? `Last traded ${fmtPrice(instrument, lastPrice)}` : undefined}
        />

        <Input
          label="Note (optional)"
          value={note}
          onChange={(e) => setNote(e.target.value)}
          placeholder="Why does this level matter?"
        />

        <div className="callout neutral">
          <Bell size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
          <span>
            Alerts are evaluated on the platform, not in this tab. They fire with the browser closed and
            disarm themselves once they trip.
          </span>
        </div>

        <div>
          <Label>Alerts on {instrument}</Label>
          <div style={{ marginTop: 'var(--s-2)' }}>
            {alerts.isLoading ? (
              <PanelLoading lines={2} />
            ) : mine.length === 0 ? (
              <EmptyState message="No alerts on this market yet" />
            ) : (
              mine.map((a) => (
                <div
                  key={a.id}
                  className="between"
                  style={{ padding: '8px 0', borderBottom: '1px solid var(--line-hairline)' }}
                >
                  <div style={{ minWidth: 0 }}>
                    <div className="row" style={{ gap: 'var(--s-2)' }}>
                      <Badge tone={a.active ? 'accent' : 'neutral'}>{a.active ? 'Armed' : 'Fired'}</Badge>
                      <span style={{ fontSize: 'var(--t-12)' }}>
                        {KINDS.find((k) => k.value === a.kind)?.label}{' '}
                        <span className="num">{a.value}</span>
                      </span>
                    </div>
                    <div className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 2 }}>
                      {a.note ? `${a.note} · ` : ''}
                      {a.triggeredAt
                        ? `Fired ${relativeTime(a.triggeredAt)}${a.triggeredPrice ? ` at ${a.triggeredPrice}` : ''}`
                        : relativeTime(a.createdAt)}
                    </div>
                  </div>
                  <IconButton label="Delete alert" bare onClick={() => remove.mutate(a.id)}>
                    <Trash2 size={13} aria-hidden />
                  </IconButton>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </Modal>
  )
}
