import { useState } from 'react'
import { motion, AnimatePresence } from 'framer-motion'
import { Loader2, RotateCcw, Plus } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { useModelAliases, useModelDeployments, useRollback, useCreateDeployment, useModelVersions } from '@/hooks/useModels'
import { format } from 'date-fns'

interface Props {
  modelId: string
}

const ALIAS_COLORS: Record<string, string> = {
  production: 'bg-pnl-up/15 text-pnl-up border-pnl-up/30',
  candidate: 'bg-purple-500/15 text-purple-400 border-purple-500/30',
  staging: 'bg-blue-500/15 text-blue-400 border-blue-500/30',
  fallback: 'bg-surface-2 text-text-muted border-border',
}

function aliasClass(alias: string) {
  return ALIAS_COLORS[alias] ?? 'bg-surface-2 text-text-muted border-border'
}

function CreateDeploymentModal({
  modelId,
  versions,
  onClose,
}: {
  modelId: string
  versions: number[]
  onClose: () => void
}) {
  const [version, setVersion] = useState(versions[0]?.toString() ?? '')
  const [environment, setEnvironment] = useState('production')
  const [alias, setAlias] = useState('production')
  const [trafficPct, setTrafficPct] = useState(100)
  const createMut = useCreateDeployment(modelId)

  async function handleCreate() {
    await createMut.mutateAsync({
      version: parseInt(version),
      environment,
      alias,
      traffic_pct: trafficPct,
    })
    onClose()
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      <div className="absolute inset-0 bg-black/50 backdrop-blur-sm" onClick={onClose} />
      <motion.div
        initial={{ opacity: 0, scale: 0.95 }}
        animate={{ opacity: 1, scale: 1 }}
        exit={{ opacity: 0, scale: 0.95 }}
        className="relative z-10 w-full max-w-md rounded-xl border border-border bg-surface p-6 shadow-xl mx-4"
      >
        <h3 className="text-base font-semibold text-text mb-4">Create Deployment</h3>

        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-text mb-1.5">Version</label>
            <select
              value={version}
              onChange={(e) => setVersion(e.target.value)}
              className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
            >
              {versions.map((v) => (
                <option key={v} value={v}>
                  v{v}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium text-text mb-1.5">Environment</label>
            <select
              value={environment}
              onChange={(e) => setEnvironment(e.target.value)}
              className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
            >
              {['production', 'staging', 'development'].map((env) => (
                <option key={env} value={env}>
                  {env}
                </option>
              ))}
            </select>
          </div>

          <div>
            <label className="block text-sm font-medium text-text mb-1.5">Alias</label>
            <input
              type="text"
              value={alias}
              onChange={(e) => setAlias(e.target.value)}
              placeholder="production"
              className="w-full h-9 rounded-lg border border-border bg-surface px-3 text-sm text-text focus:outline-none focus:border-accent"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-text mb-1.5">
              Traffic % <span className="font-mono text-accent">{trafficPct}%</span>
            </label>
            <input
              type="range"
              min={0}
              max={100}
              value={trafficPct}
              onChange={(e) => setTrafficPct(parseInt(e.target.value))}
              className="w-full accent-accent"
            />
          </div>
        </div>

        <div className="mt-5 flex gap-3 justify-end">
          <Button variant="outline" onClick={onClose}>
            Cancel
          </Button>
          <Button
            onClick={handleCreate}
            disabled={!version || !alias || createMut.isPending}
          >
            {createMut.isPending ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              'Deploy'
            )}
          </Button>
        </div>
      </motion.div>
    </div>
  )
}

export function ModelDeploymentsTab({ modelId }: Props) {
  const { data: aliases, isLoading: aliasLoading } = useModelAliases(modelId)
  const { data: deployments, isLoading: deploymentsLoading } = useModelDeployments(modelId)
  const { data: versions } = useModelVersions(modelId)
  const rollbackMut = useRollback(modelId)
  const [showCreate, setShowCreate] = useState(false)

  const versionNumbers = versions?.map((v) => v.version).sort((a, b) => b - a) ?? []

  if (aliasLoading || deploymentsLoading) {
    return (
      <div className="flex items-center justify-center py-12 text-text-dim">
        <Loader2 className="h-5 w-5 animate-spin" />
      </div>
    )
  }

  const aliasEntries = aliases ? Object.entries(aliases) : []

  return (
    <>
      {/* Alias chips */}
      {aliasEntries.length > 0 && (
        <div className="mb-6">
          <h3 className="text-sm font-medium text-text mb-3">Aliases</h3>
          <div className="flex flex-wrap gap-3">
            {aliasEntries.map(([alias, version]) => (
              <div
                key={alias}
                className={cn(
                  'inline-flex items-center gap-2 rounded-xl border px-4 py-2.5',
                  aliasClass(alias),
                )}
              >
                <div>
                  <p className="text-xs font-medium">{alias}</p>
                  <p className="text-xs font-mono opacity-80">→ v{version}</p>
                </div>
                <button
                  onClick={() => rollbackMut.mutate(alias)}
                  disabled={rollbackMut.isPending}
                  className="ml-1 opacity-60 hover:opacity-100 transition-opacity"
                  title="Rollback"
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                </button>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Deployments table */}
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium text-text">Deployment history</h3>
        <Button size="sm" variant="outline" onClick={() => setShowCreate(true)}>
          <Plus className="h-3.5 w-3.5" />
          Deploy
        </Button>
      </div>

      {!deployments || deployments.length === 0 ? (
        <div className="flex flex-col items-center justify-center py-12 rounded-xl border border-dashed border-border text-center">
          <p className="text-sm text-text-dim">No deployments yet</p>
          <Button
            size="sm"
            variant="outline"
            className="mt-3"
            onClick={() => setShowCreate(true)}
          >
            <Plus className="h-3.5 w-3.5" />
            Create deployment
          </Button>
        </div>
      ) : (
        <div className="rounded-xl border border-border bg-surface overflow-hidden">
          <table className="w-full">
            <thead>
              <tr className="border-b border-border bg-surface-2">
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Version</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Alias</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Environment</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Status</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Traffic</th>
                <th className="px-4 py-3 text-left text-xs font-medium text-text-muted">Deployed</th>
              </tr>
            </thead>
            <tbody>
              {deployments.map((dep) => (
                <tr key={dep.deployment_id} className="border-b border-border last:border-0">
                  <td className="px-4 py-3 text-sm font-mono font-medium text-text">
                    v{dep.version}
                  </td>
                  <td className="px-4 py-3">
                    <span
                      className={cn(
                        'inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium',
                        aliasClass(dep.alias),
                      )}
                    >
                      {dep.alias}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-sm text-text-muted">{dep.environment}</td>
                  <td className="px-4 py-3">
                    <span
                      className={cn(
                        'inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium',
                        dep.status === 'active'
                          ? 'bg-pnl-up/10 text-pnl-up'
                          : 'bg-surface-2 text-text-muted',
                      )}
                    >
                      {dep.status}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-sm font-mono text-text">
                    {dep.traffic_pct}%
                  </td>
                  <td className="px-4 py-3 text-xs text-text-dim font-mono">
                    {format(new Date(dep.deployed_at), 'MMM d, HH:mm')}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <AnimatePresence>
        {showCreate && (
          <CreateDeploymentModal
            modelId={modelId}
            versions={versionNumbers}
            onClose={() => setShowCreate(false)}
          />
        )}
      </AnimatePresence>
    </>
  )
}
