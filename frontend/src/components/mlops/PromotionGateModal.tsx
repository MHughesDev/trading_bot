// Promotion gate (Set H H-5.8).
//
// The checklist the modal shows is the server's. Before confirming it shows what
// the client can already know (evaluated, artifact hash, rollback) and marks the
// regression check as the server's to decide; on confirm it sends the promote
// and, if the server refuses, replaces the list with the server's own
// `gate_checks`. A refusal is HTTP 200 with `promoted: false`, so it is read from
// the body (`parsePromoteResult`) -- never inferred from the status code.
//
// Overriding a failed gate is possible and is not silent: it requires a reason,
// and the reason travels as `override_reason`, which the server records on the
// `promoted` event.
import { useState } from 'react'
import { AlertTriangle, CheckCircle2, CircleHelp, XCircle } from 'lucide-react'
import { Modal } from '@/components/primitives/Modal'
import { Button } from '@/components/primitives/Button'
import { Field, TextArea } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import type { ModelVersion, PromoteResult } from '@/api/mlops'
import {
  failedRequiredChecks,
  gateLabel,
  parsePromoteResult,
  preflightBlocksLive,
  preflightChecks,
  type ParsedPromote,
} from './studio.helpers'

type Env = 'paper' | 'live'

export interface PromotionGateModalProps {
  open: boolean
  version: ModelVersion
  hasProduction: boolean
  onClose: () => void
  /** Sends the promote. Resolves with the server's body; rejects on a transport/HTTP error. */
  onPromote: (args: { environment: Env; overrideReason?: string }) => Promise<PromoteResult>
}

function CheckIcon({ state }: { state: 'pass' | 'fail' | 'server' }) {
  if (state === 'pass') return <CheckCircle2 size={15} aria-hidden style={{ color: 'var(--fg-pos)' }} />
  if (state === 'fail') return <XCircle size={15} aria-hidden style={{ color: 'var(--fg-neg)' }} />
  return <CircleHelp size={15} aria-hidden style={{ color: 'var(--fg-tertiary)' }} />
}

export function PromotionGateModal({
  open,
  version,
  hasProduction,
  onClose,
  onPromote,
}: PromotionGateModalProps) {
  const [environment, setEnvironment] = useState<Env>('paper')
  const [reason, setReason] = useState('')
  const [busy, setBusy] = useState(false)
  const [refused, setRefused] = useState<ParsedPromote | null>(null)
  const [error, setError] = useState<string | null>(null)

  const pre = preflightChecks(version, hasProduction, environment)
  const knownFailure = preflightBlocksLive(pre)
  // An override is needed when this client already sees a failed check, or when
  // the server has just refused.
  const needsOverride = knownFailure || refused !== null
  const canConfirm = !busy && (!needsOverride || reason.trim().length > 0)

  async function confirm() {
    setBusy(true)
    setError(null)
    try {
      const body = await onPromote({
        environment,
        overrideReason: needsOverride ? reason.trim() : undefined,
      })
      const parsed = parsePromoteResult(body)
      if (parsed.promoted) {
        onClose()
        return
      }
      setRefused(parsed)
    } catch {
      setError('The promote request failed. Nothing was changed.')
    } finally {
      setBusy(false)
    }
  }

  const serverFailed = refused ? failedRequiredChecks(refused.checks) : []

  return (
    <Modal
      open={open}
      onClose={onClose}
      title={`Promote v${version.version} to production`}
      description="Moves the production alias. The current production version becomes the fallback."
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button variant="primary" loading={busy} disabled={!canConfirm} onClick={() => void confirm()}>
            {needsOverride ? 'Promote with override' : 'Promote'}
          </Button>
        </>
      }
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
        <Field label="Environment">
          <Segmented<Env>
            ariaLabel="Environment"
            value={environment}
            onChange={(v) => {
              setEnvironment(v)
              setRefused(null)
            }}
            options={[
              { value: 'paper', label: 'Paper' },
              { value: 'live', label: 'Live' },
            ]}
          />
        </Field>

        <ul aria-label="Promotion checks" style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
          {(refused && refused.checks.length > 0
            ? refused.checks.map((c) => ({
                check: c.check,
                label: gateLabel(c.check),
                state: (c.passed ? 'pass' : 'fail') as 'pass' | 'fail',
                note: c.required ? undefined : 'advisory',
              }))
            : pre.map((c) => ({
                ...c,
                note: c.state === 'server' ? 'checked by the server on confirm' : undefined,
              }))
          ).map((c) => (
            <li key={c.check} style={{ display: 'flex', gap: 'var(--s-2)', alignItems: 'center' }}>
              <CheckIcon state={c.state} />
              <span>{c.label}</span>
              {c.note && <span className="mut" style={{ fontSize: 'var(--t-11)' }}>{c.note}</span>}
            </li>
          ))}
        </ul>

        {refused && (
          <div className="callout warn" role="alert">
            <AlertTriangle size={15} aria-hidden style={{ flex: 'none', marginTop: 1 }} />
            <div>
              The server refused this promotion; nothing changed.
              {serverFailed.length > 0 && (
                <> Failed: {serverFailed.map((c) => gateLabel(c.check)).join(', ')}.</>
              )}{' '}
              Give a reason to override.
            </div>
          </div>
        )}

        {needsOverride && (
          <Field
            label="Override reason (recorded on the model's history)"
            error={reason.trim().length === 0 ? 'A reason is required to override a failed check.' : null}
          >
            <TextArea
              value={reason}
              rows={2}
              onChange={(e) => setReason(e.target.value)}
              placeholder="Why is this version being promoted anyway?"
            />
          </Field>
        )}

        {error && (
          <div className="callout neg" role="alert">
            {error}
          </div>
        )}
      </div>
    </Modal>
  )
}
