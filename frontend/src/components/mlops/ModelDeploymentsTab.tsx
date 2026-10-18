// Deployment Control Room (Set H H-5.8): the alias -> version map, per-environment
// traffic allocation, and the deploy / rollback actions.
//
// What the registry lets a client do, and what this tab therefore does not claim:
//   * aliases move by promote (production) and rollback; there is no route that
//     sets candidate/staging/fallback directly, so those slots are read-only;
//   * a deployment, once made, is not editable, so traffic is allocated when a
//     deployment is created and the form refuses an allocation the server would
//     (the same `<= 100` per environment rule, from the active deployments).
import { useState } from 'react'
import { RotateCcw, Plus } from 'lucide-react'
import { Button } from '@/components/primitives/Button'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { Modal, ConfirmDialog } from '@/components/primitives/Modal'
import { Field } from '@/components/primitives/Field'
import { Segmented } from '@/components/primitives/Segmented'
import { EmptyState, ErrorState, PanelLoading } from '@/components/primitives/States'
import {
  useModelAliases,
  useModelDeployments,
  useRollback,
  useCreateDeployment,
  useModelVersions,
} from '@/hooks/useMlOps'
import {
  DEPLOY_ENVIRONMENTS,
  aliasSlots,
  rollbackTarget,
  trafficAllocated,
  validateTraffic,
} from './studio.helpers'
import type { ModelDeployment } from '@/api/mlops'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

type Env = (typeof DEPLOY_ENVIRONMENTS)[number]

export function DeployModal({
  open,
  versions,
  deployments,
  busy,
  error,
  onClose,
  onDeploy,
}: {
  open: boolean
  versions: number[]
  deployments: ModelDeployment[]
  busy: boolean
  error: string | null
  onClose: () => void
  onDeploy: (body: { version: number; environment: Env; traffic_pct: number }) => void
}) {
  const [version, setVersion] = useState<number | null>(null)
  const [environment, setEnvironment] = useState<Env>('paper')
  const [traffic, setTraffic] = useState(100)

  const chosen = version ?? versions[0] ?? null
  const check = validateTraffic(traffic, deployments, environment)
  const allocated = trafficAllocated(deployments, environment)

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Deploy a version"
      footer={
        <>
          <span className="spacer" />
          <Button onClick={onClose}>Cancel</Button>
          <Button
            variant="primary"
            loading={busy}
            disabled={chosen === null || !check.ok}
            onClick={() => chosen !== null && onDeploy({ version: chosen, environment, traffic_pct: traffic })}
          >
            Deploy
          </Button>
        </>
      }
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
        <Field label="Version">
          <select aria-label="Version" value={chosen ?? ''} onChange={(e) => setVersion(Number(e.target.value))}>
            {versions.map((v) => (
              <option key={v} value={v}>v{v}</option>
            ))}
          </select>
        </Field>
        <Field label="Environment">
          <Segmented<Env>
            ariaLabel="Environment"
            value={environment}
            onChange={(e) => {
              setEnvironment(e)
              setTraffic(Math.min(100, Math.max(0, 100 - trafficAllocated(deployments, e))))
            }}
            options={DEPLOY_ENVIRONMENTS.map((e) => ({ value: e, label: e === 'paper' ? 'Paper' : 'Live' }))}
          />
        </Field>
        <Field
          label={<>Traffic <span className="num">{traffic}%</span></>}
          labelAside={<span className="mut num">{check.remaining}% free in {environment}</span>}
          error={check.message}
        >
          <input
            type="range"
            min={0}
            max={100}
            value={traffic}
            aria-label="Traffic percent"
            onChange={(e) => setTraffic(Number(e.target.value))}
          />
        </Field>
        <div className="mut" style={{ fontSize: 'var(--t-11)' }}>
          {allocated}% of {environment} traffic is already routed to active deployments.
        </div>
        {error && <div className="callout neg" role="alert">{error}</div>}
      </div>
    </Modal>
  )
}

export function ModelDeploymentsTab({ modelId }: Props) {
  const aliases = useModelAliases(modelId)
  const deployments = useModelDeployments(modelId)
  const versions = useModelVersions(modelId)
  const rollback = useRollback(modelId)
  const create = useCreateDeployment(modelId)
  const [deploying, setDeploying] = useState(false)
  const [rolling, setRolling] = useState<string | null>(null)

  if (aliases.isLoading || deployments.isLoading) return <PanelLoading lines={6} />
  if (aliases.isError || deployments.isError) {
    return (
      <ErrorState
        message="Deployments could not be loaded."
        detail="This is a failed request, not a model with no deployments."
        onRetry={() => {
          void aliases.refetch()
          void deployments.refetch()
        }}
      />
    )
  }

  const versionNumbers = (versions.data ?? []).map((v) => v.version).sort((a, b) => b - a)
  const slots = aliasSlots(aliases.data)
  const deps = deployments.data ?? []
  const rollingSlot = slots.find((s) => s.alias === rolling)
  const target = rollingSlot ? rollbackTarget(rollingSlot.version, versionNumbers) : null

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
      <Panel>
        <PanelHeader title="Alias map" />
        <PanelBody>
          <div style={{ display: 'grid', gap: 'var(--s-3)', gridTemplateColumns: 'repeat(auto-fill, minmax(190px, 1fr))' }}>
            {slots.map((s) => {
              const canRollback = s.version !== null && rollbackTarget(s.version, versionNumbers) !== null
              return (
                <div key={s.alias} className="panel" style={{ padding: 'var(--s-3)', display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
                  <Badge tone={s.alias === 'production' ? 'pos' : 'outline'}>{s.alias}</Badge>
                  <div className="h2 num">{s.version !== null ? `v${s.version}` : 'not set'}</div>
                  {s.version !== null && (
                    <Button
                      size="xs"
                      disabled={!canRollback}
                      title={canRollback ? undefined : 'There is no earlier version to roll back to'}
                      icon={<RotateCcw size={12} aria-hidden />}
                      onClick={() => setRolling(s.alias)}
                    >
                      Roll back
                    </Button>
                  )}
                </div>
              )
            })}
          </div>
          <p className="mut" style={{ fontSize: 'var(--t-11)', marginTop: 'var(--s-3)' }}>
            Production moves by promoting a version; every other alias is set by the registry.
          </p>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader
          title="Traffic by environment"
          actions={
            <Button size="sm" icon={<Plus size={13} aria-hidden />} onClick={() => setDeploying(true)} disabled={versionNumbers.length === 0}>
              Deploy
            </Button>
          }
        />
        <PanelBody>
          <div style={{ display: 'grid', gap: 'var(--s-4)', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))' }}>
            {DEPLOY_ENVIRONMENTS.map((env) => {
              const used = trafficAllocated(deps, env)
              return (
                <div key={env}>
                  <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                    <strong>{env === 'paper' ? 'Paper' : 'Live'}</strong>
                    <span className="num">{used}% routed</span>
                  </div>
                  <div role="img" aria-label={`${used}% of ${env} traffic allocated`} style={{ height: 6, borderRadius: 3, background: 'var(--bg-sunken)', marginTop: 6, overflow: 'hidden' }}>
                    <div style={{ width: `${Math.min(100, used)}%`, height: '100%', background: used > 100 ? 'var(--fg-neg)' : 'var(--bg-accent)' }} />
                  </div>
                </div>
              )
            })}
          </div>
        </PanelBody>
      </Panel>

      <Panel>
        <PanelHeader title="Deployment history" />
        <PanelBody flush>
          {deps.length === 0 ? (
            <EmptyState message="No deployments yet" detail="Deploy a version to route paper or live traffic to it." />
          ) : (
            <table className="tbl">
              <thead>
                <tr>
                  <th className="l">Version</th>
                  <th>Environment</th>
                  <th>Status</th>
                  <th>Traffic</th>
                  <th>Deployed</th>
                </tr>
              </thead>
              <tbody>
                {deps.map((d) => (
                  <tr key={d.deployment_id}>
                    <td className="l num">v{d.version}</td>
                    <td>{d.environment}</td>
                    <td><Badge tone={d.status === 'active' ? 'pos' : 'neutral'}>{d.status}</Badge></td>
                    <td className="n">{d.traffic_pct}%</td>
                    <td className="n">{format(new Date(d.deployed_at), 'MMM d, HH:mm')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </PanelBody>
      </Panel>

      <DeployModal
        open={deploying}
        versions={versionNumbers}
        deployments={deps}
        busy={create.isPending}
        error={create.isError ? 'The registry refused this deployment. Nothing was changed.' : null}
        onClose={() => setDeploying(false)}
        onDeploy={(body) =>
          create.mutate(
            { ...body, alias: 'production' },
            { onSuccess: () => setDeploying(false) },
          )
        }
      />

      <ConfirmDialog
        open={rolling !== null}
        onCancel={() => setRolling(null)}
        busy={rollback.isPending}
        title={`Roll back ${rolling ?? ''}`}
        confirmLabel="Roll back"
        consequence={
          rollingSlot && target !== null ? (
            <>
              Moves <strong>{rollingSlot.alias}</strong> from v{rollingSlot.version} to v{target} now. The change is
              recorded on the model&apos;s history with who made it.
            </>
          ) : (
            <>There is no earlier version to roll back to.</>
          )
        }
        onConfirm={() => {
          if (rolling && target !== null) rollback.mutate(rolling, { onSuccess: () => setRolling(null) })
        }}
      />
      {rollback.isError && (
        <div className="callout neg" role="alert">The rollback was refused. Nothing was changed.</div>
      )}
    </div>
  )
}
