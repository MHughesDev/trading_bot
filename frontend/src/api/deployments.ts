// The deployment path's data layer (Set T T-1.1, T-1.3, T-2.1): the overview of
// what is running, the proposals that authorise a deployment, the one-action
// stops and the fleet ceilings.
//
// Every route here is served by `crates/api/src/deployment_gate_routes.rs` except
// `POST /api/deployments`, which is the v3 deployment route and takes the
// `proposal_id` that authorises it. The page sends `proposal_id` on every
// deployment it makes: the gate's trigger refuses a deployment without one once
// it is armed, and a page that did not send it would be what broke the armed
// platform.
//
// Reads go through `Read<T>`, so a list that failed to load is "unknown", never
// "nothing is deployed".
import { api as client } from '@/lib/api'
import { read } from '@/api/ops'

/** One gate's line in a stored verdict. Only the fields the page reads. */
export interface GateLine {
  gate_no?: number
  passed?: boolean
  name?: string
}

export interface ForwardPoint {
  bar: string
  realised: number
  expected: number
}

/** One row of `GET /api/deployment-overview`. */
export interface DeploymentRow {
  deployment_id: string
  strategy_name: string
  revision: string
  instrument_id: string
  environment: string
  venue: string
  starting_capital: string
  started_at: string
  stopped_at: string | null
  stop_reason: string | null
  running: boolean
  /** The authority behind it; null on a deployment that predates the gate. */
  proposal_id: string | null
  profile_id: string | null
  gates_passed: number | null
  verdict: GateLine[] | null
  /** A fact shown beside the numbers, never a permission (ADR-P2-27). */
  legacy_ungated: boolean
  /** The ramp's size as halvings: 0 is full, 4 is zero. */
  size_halvings: number
  review_period_no: number | null
  review_period_ends_at: string | null
  /** Null when no forward recorder feeds this deployment. */
  divergence_halts: number | null
  realised_vs_expected: ForwardPoint[]
  /** Present once the server reports a deployment with no forward recorder. */
  unmeasured?: boolean
}

/** One row of `GET /api/deployment-proposals`. */
export interface DeploymentProposal {
  proposal_id: string
  strategy_name: string
  revision: string
  instrument_id: string
  profile_id: string
  gates_passed: number
  starting_capital: string
  review_period_days: number
  proposed_by_kind: string
  created_at: string
  /** `pending`, `approved` or `rejected`. */
  decision: string
  decided_by: string | null
  decided_at: string | null
  envelope: Record<string, unknown> | null
  /** Set once a deployment was made under this approval (one use). */
  deployment_id: string | null
}

export interface Ceilings {
  declared: Array<{ name: string; value: string; declared_by: string; declared_at: string }>
  undeclared_refuses_every_deployment: boolean
}

export const deploymentsApi = {
  overview: () => read(client.get<DeploymentRow[]>('/api/deployment-overview')),
  proposals: () => read(client.get<DeploymentProposal[]>('/api/deployment-proposals')),
  ceilings: () => read(client.get<Ceilings>('/api/deployment-ceilings')),

  /** `approved` or `rejected`; a person's decision, recorded with the actor. */
  decide: (proposalId: string, decision: 'approved' | 'rejected', note: string) =>
    client.post<{ proposal_id: string; decision: string; envelope: unknown }>(
      `/api/deployment-proposals/${proposalId}/decision`,
      { decision, note },
    ),

  /** Withdraws one deployment and trips its strategy scope at the order gate. */
  revoke: (deploymentId: string) =>
    client.post<{ deployment_id: string; withdrawn: boolean; closed: boolean }>(
      `/api/deployments/${deploymentId}/revoke`,
    ),

  /** The kill switch's action, then every deployment withdrawn. Needs a reason. */
  stopAll: (reason: string) =>
    client.post<{ scope: string; state: string; stopped: string[] }>('/api/deployment-stop-all', {
      reason,
    }),

  /**
   * Make the deployment an approved proposal authorises. `proposal_id` is the
   * point: the proposal's own strategy, instrument and capital are what is
   * sent, so the trigger's check against the proposal cannot disagree with it.
   */
  deploy: (p: DeploymentProposal) =>
    client.post<{ deployment_id?: string }>('/api/deployments', {
      strategy_ref: p.strategy_name,
      instrument_id: p.instrument_id,
      starting_capital: p.starting_capital,
      proposal_id: p.proposal_id,
    }),
}
