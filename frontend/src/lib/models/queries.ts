// Typed query hooks for the Model Studio routes (N-5.1).
//
// The project's convention is react-query over the shared axios client, with
// keys under `['models', ...]` so the existing `MODEL_KEYS.all` invalidation
// reaches these too. Hooks for routes the registry already served live in
// `@/hooks/useMlOps`; only the Studio's own routes are here.
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { studioApi, type DatasetDeclaration, type StudioTrainBody } from '@/api/modelsStudio'

export const STUDIO_KEYS = {
  families: ['models', 'families'] as const,
  fleet: ['models', 'studio', 'fleet'] as const,
  verdict: (id: string, v: number) => ['models', id, 'verdict', v] as const,
  evidence: (id: string, v: number) => ['models', id, 'promotion-evidence', v] as const,
  proposals: (id: string) => ['models', id, 'promotion-proposals'] as const,
  importance: (id: string, v: number) => ['models', id, 'importance', v] as const,
}

export function useFamilies() {
  return useQuery({
    queryKey: STUDIO_KEYS.families,
    queryFn: () => studioApi.families().then((r) => r.data.families),
    staleTime: 60_000,
  })
}

/** The fleet, polled while anything in it is training or evaluating. */
export function useFleet() {
  return useQuery({
    queryKey: STUDIO_KEYS.fleet,
    queryFn: () => studioApi.fleet().then((r) => r.data),
    refetchInterval: (q) => {
      if (q.state.status === 'error') return 10_000
      const c = q.state.data?.counts
      return c && (c.training > 0 || c.evaluating > 0) ? 3000 : 15_000
    },
  })
}

export function useVerdict(id: string, version: number | null) {
  return useQuery({
    queryKey: STUDIO_KEYS.verdict(id, version ?? -1),
    queryFn: () => studioApi.verdict(id, version as number).then((r) => r.data),
    enabled: !!id && version !== null,
  })
}

export function usePromotionEvidence(id: string, version: number | null) {
  return useQuery({
    queryKey: STUDIO_KEYS.evidence(id, version ?? -1),
    queryFn: () => studioApi.promotionEvidence(id, version as number).then((r) => r.data),
    enabled: !!id && version !== null,
  })
}

export function usePromotionProposals(id: string) {
  return useQuery({
    queryKey: STUDIO_KEYS.proposals(id),
    queryFn: () => studioApi.proposals(id).then((r) => r.data.proposals),
    enabled: !!id,
  })
}

export function useImportance(id: string, version: number | null) {
  return useQuery({
    queryKey: STUDIO_KEYS.importance(id, version ?? -1),
    queryFn: () => studioApi.importance(id, version as number).then((r) => r.data),
    enabled: !!id && version !== null,
  })
}

/** Validation is a POST that changes nothing, so it is a mutation the wizard calls on demand. */
export function useValidateDeclaration() {
  return useMutation({
    mutationFn: (body: { declaration: DatasetDeclaration; cutoff?: string }) =>
      studioApi.validateDeclaration(body).then((r) => r.data),
  })
}

export function useProposePromotion(id: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (a: { version: number; environment?: string; rationale: string }) =>
      studioApi
        .propose(id, a.version, { environment: a.environment, rationale: a.rationale })
        .then((r) => r.data),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: STUDIO_KEYS.proposals(id) })
      void qc.invalidateQueries({ queryKey: ['models', id, 'promotion-evidence'] })
    },
  })
}

export function useDecideProposal(id: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (a: { proposalId: string; decision: 'approved' | 'rejected'; note: string }) =>
      studioApi.decide(a.proposalId, { decision: a.decision, note: a.note }).then((r) => r.data),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['models', id] })
    },
  })
}

export function useStudioTrain(id: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: StudioTrainBody) => studioApi.train(id, body).then((r) => r.data),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: ['models'] })
    },
  })
}
