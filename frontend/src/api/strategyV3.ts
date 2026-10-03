// The one authoring API for tbot.strategy/3 (Set U U-7.4, ADR-0045).
//
// The graph editor calls exactly these routes, and so do the agent's
// `*_strategy_v3` tools (crates/mcp-server/src/tools/strategy_v3.rs): a person
// and a model get the same answer to the same question because there is one
// place that answers it. Nothing here parses, validates or compiles a strategy.
import { api as client } from '@/lib/api'

/** A port or output type, as the operator registry serialises it. */
export type Ty =
  | 'num'
  | 'bool'
  | 'text'
  | 'any_dist'
  | 'any_vec'
  | 'any_record'
  | 'num_list'
  | 'bool_list'
  | 'window'
  | 'from_contract'
  | 'from_schema'
  | { var: string }
  | { cat_of: string }
  | { probs_of: string }

export interface Port {
  name: string
  ty: Ty
  optional?: boolean
  literal?: boolean
}

export interface OpSpec {
  name: string
  family: string
  inputs: Port[]
  output: Ty
  stateful: boolean
  open_inputs: boolean
  cross_sectional: boolean
  use_when: string
}

export interface FeatureEntry {
  family: string
  example: string
  warmup_bars?: number
  use_when: string
  alias_of?: string
}

/** A model contract's value type: `{type: 'num'}`, `{type: 'probs', labels}`… */
export interface ValueType {
  type: string
  labels?: string[]
}

/** A contract input, flattened as the contract serialises it: `scalar` inputs
 *  carry a `value_type`; `row` and `window` inputs carry their features. */
export interface ContractInput {
  name: string
  shape: 'scalar' | 'row' | 'window' | string
  value_type?: ValueType
  optional?: boolean
}

export interface ModelEntry {
  reference: string
  kind: string
  contract: {
    inputs: ContractInput[]
    outputs: { name: string; value_type: ValueType }[]
  }
}

export interface Catalogue {
  operators: OpSpec[]
  features: FeatureEntry[]
  models: ModelEntry[]
}

/** One argument of a node: a reference, a literal, a list or an inline node. */
export type Arg = boolean | number | string | { str: string } | InlineNode | Arg[]
export interface InlineNode {
  op: string
  args?: Record<string, Arg>
}

export interface GraphNode {
  id: string
  op: string
  args?: Record<string, Arg>
}

/** The canonical graph. Only `nodes` is edited on the canvas; the rest is
 *  carried through unchanged or edited in forms. */
export interface StrategyGraph {
  format: 'tbot.strategy/3'
  kind: 'trading' | 'selection'
  name: string
  asset_class: string
  timeframe: string
  nodes: GraphNode[]
  [field: string]: unknown
}

export interface Finding {
  code: string
  at: string
  message: string
  fix: string
  example?: string
  see?: string
}

export interface BuildResult {
  valid: boolean
  graph: StrategyGraph
  text: string
  revision?: string
  findings: Finding[]
}

export interface SaveResult {
  strategy_id: string
  revision_hash: string
  actor_kind: string
  parent_revision: string | null
  text: string
}

/** The routes, named once so a test can hold the editor and the agent to them. */
export const V3_ROUTES = {
  catalogue: '/api/v3/catalogue',
  build: '/api/v3/authoring/build',
  validate: '/api/v3/authoring/validate',
  save: '/api/v3/strategies',
  get: (id: string) => `/api/v3/strategies/${encodeURIComponent(id)}`,
} as const

export const strategyV3Api = {
  catalogue: async (): Promise<Catalogue> => (await client.get(V3_ROUTES.catalogue)).data,

  /** Text or a graph in; the graph, its canonical text and every finding out —
   *  even for a strategy that is not finished. What keeps the canvas and the
   *  text view the same strategy. */
  build: async (input: { text: string } | { graph: StrategyGraph }): Promise<BuildResult> => {
    try {
      return (await client.post(V3_ROUTES.build, { ...input, ops: [] })).data
    } catch (e: unknown) {
      const data = (e as { response?: { data?: { findings?: Finding[] } } }).response?.data
      if (data?.findings) {
        throw Object.assign(new Error(data.findings[0]?.message ?? 'not a strategy'), { findings: data.findings })
      }
      throw e
    }
  },

  /** The editor's save — the same call the agent's `save_strategy_v3` makes. */
  save: async (strategyId: string, graph: StrategyGraph): Promise<SaveResult> =>
    (await client.post(V3_ROUTES.save, { strategy_id: strategyId, graph })).data,

  get: async (id: string): Promise<{ graph: StrategyGraph; text: string; revision?: string }> =>
    (await client.get(V3_ROUTES.get(id))).data,
}
