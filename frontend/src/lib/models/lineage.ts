// The content-hash lineage chain (N-5.9):
//   dataset -> features -> model version -> prediction series -> backtest
//
// Built only from what the registry serves: each version's `metrics` blob and
// `artifact_hash`. A link the registry did not record is drawn as a visible gap
// ("not recorded"), never as an invented node. Two versions that name the same
// hash share one node, which is the point of keying by content.
import type { AliasMap, ModelVersion } from '@/api/mlops'

export type LinkKind = 'dataset' | 'features' | 'model_version' | 'series' | 'backtest'

export const LINK_LAYER: Record<LinkKind, number> = {
  dataset: 0,
  features: 1,
  model_version: 2,
  series: 3,
  backtest: 4,
}

export const LINK_NAMES: Record<LinkKind, string> = {
  dataset: 'Dataset',
  features: 'Features',
  model_version: 'Model version',
  series: 'Prediction series',
  backtest: 'Backtest',
}

export interface HashNode {
  id: string
  kind: LinkKind
  /** The content hash, or null where the registry recorded none. */
  hash: string | null
  label: string
  recorded: boolean
  version?: number
}

export interface HashEdge {
  id: string
  source: string
  target: string
}

function str(m: Record<string, unknown> | undefined, ...keys: string[]): string | null {
  for (const k of keys) {
    const v = m?.[k]
    if (typeof v === 'string' && v.length > 0) return v
  }
  return null
}

function hasBacktest(m: Record<string, unknown> | undefined): boolean {
  return Object.keys(m ?? {}).some((k) => k.startsWith('backtest_'))
}

export function shortHash(h: string): string {
  return h.length > 14 ? `${h.slice(0, 12)}…` : h
}

export function buildHashLineage(
  versions: ModelVersion[],
  aliases: AliasMap | undefined,
): { nodes: HashNode[]; edges: HashEdge[] } {
  const nodes = new Map<string, HashNode>()
  const edges = new Map<string, HashEdge>()

  const put = (n: HashNode) => {
    if (!nodes.has(n.id)) nodes.set(n.id, n)
    return n.id
  }
  const link = (source: string, target: string) => {
    const id = `${source}>${target}`
    if (!edges.has(id)) edges.set(id, { id, source, target })
  }
  const slot = (kind: LinkKind, hash: string | null, v: number, gapLabel: string): string => {
    if (hash) return put({ id: `${kind}:${hash}`, kind, hash, label: shortHash(hash), recorded: true })
    // A gap is per version: it says which version lacks the record.
    return put({ id: `${kind}:gap:v${v}`, kind, hash: null, label: gapLabel, recorded: false, version: v })
  }

  for (const ver of [...versions].sort((a, b) => a.version - b.version)) {
    const m = ver.metrics
    const production = aliases?.production === ver.version
    const ds = slot('dataset', str(m, 'dataset_id', 'dataset_hash', 'dataset_version'), ver.version, 'not recorded')
    const ft = slot(
      'features',
      str(m, 'feature_dag_hash', 'feature_set_hash', 'feature_set'),
      ver.version,
      'not recorded',
    )
    const mv = put({
      id: `model_version:${ver.version}`,
      kind: 'model_version',
      hash: ver.artifact_hash ?? null,
      label: `v${ver.version}${production ? ' · production' : ''}${ver.artifact_hash ? ` · ${shortHash(ver.artifact_hash)}` : ''}`,
      recorded: true,
      version: ver.version,
    })
    const sr = slot('series', str(m, 'prediction_series_id', 'series_hash'), ver.version, 'none linked')
    const bt = hasBacktest(m)
      ? put({ id: `backtest:v${ver.version}`, kind: 'backtest', hash: null, label: 'backtest metrics attached', recorded: true, version: ver.version })
      : slot('backtest', str(m, 'backtest_run_id'), ver.version, 'none linked')
    link(ds, ft)
    link(ft, mv)
    link(mv, sr)
    link(sr, bt)
  }
  return { nodes: [...nodes.values()], edges: [...edges.values()] }
}

export function layoutHashLineage(nodes: HashNode[]): Map<string, { x: number; y: number }> {
  const rows = new Map<number, number>()
  const out = new Map<string, { x: number; y: number }>()
  for (const n of nodes) {
    const col = LINK_LAYER[n.kind]
    const row = rows.get(col) ?? 0
    rows.set(col, row + 1)
    out.set(n.id, { x: col * 230, y: row * 78 })
  }
  return out
}
