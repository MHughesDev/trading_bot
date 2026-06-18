import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import {
  modelsApi,
  type ModelKind,
  type ModelStatus,
  type TrainDataSelection,
} from '@/api/mlops'

export const MODEL_KEYS = {
  all: ['models'] as const,
  list: (filters?: object) => ['models', 'list', filters] as const,
  detail: (id: string) => ['models', id] as const,
  runs: (id: string) => ['models', id, 'runs'] as const,
  versions: (id: string) => ['models', id, 'versions'] as const,
  evals: (id: string) => ['models', id, 'evals'] as const,
  aliases: (id: string) => ['models', id, 'aliases'] as const,
  deployments: (id: string) => ['models', id, 'deployments'] as const,
  usedBy: (id: string) => ['models', id, 'used-by'] as const,
  forNode: (kind: ModelKind) => ['models', 'for-node', kind] as const,
  testCases: (id: string) => ['models', id, 'test-cases'] as const,
}

export function useModels(filters?: {
  kind?: ModelKind
  status?: ModelStatus
  asset_class?: string
  q?: string
}) {
  return useQuery({
    queryKey: MODEL_KEYS.list(filters),
    queryFn: () => modelsApi.list(filters).then((r) => r.data),
    staleTime: 5000,
  })
}

export function useModel(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.detail(id),
    queryFn: () => modelsApi.get(id).then((r) => r.data),
    enabled: !!id,
  })
}

export function useModelRuns(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.runs(id),
    queryFn: () => modelsApi.listRuns(id).then((r) => r.data.runs),
    enabled: !!id,
    refetchInterval: (query) => {
      const runs = query.state.data
      const hasActive = runs?.some((r) => r.status === 'running')
      return hasActive ? 2000 : false
    },
  })
}

export function useModelVersions(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.versions(id),
    queryFn: () => modelsApi.listVersions(id).then((r) => r.data.versions),
    enabled: !!id,
  })
}

export function useModelEvals(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.evals(id),
    queryFn: () => modelsApi.listEvals(id).then((r) => r.data.evaluations),
    enabled: !!id,
  })
}

export function useModelAliases(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.aliases(id),
    queryFn: () => modelsApi.getAliases(id).then((r) => r.data),
    enabled: !!id,
  })
}

export function useModelDeployments(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.deployments(id),
    queryFn: () => modelsApi.listDeployments(id).then((r) => r.data.deployments),
    enabled: !!id,
  })
}

export function useModelUsedBy(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.usedBy(id),
    queryFn: () => modelsApi.usedBy(id).then((r) => r.data.strategies),
    enabled: !!id,
  })
}

export function useModelsForNode(kind: ModelKind, assetClass?: string) {
  return useQuery({
    queryKey: [...MODEL_KEYS.forNode(kind), assetClass],
    queryFn: () => modelsApi.forNode(kind, assetClass).then((r) => r.data.models),
    staleTime: 30_000,
  })
}

export function useCreateModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: modelsApi.create,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.all })
    },
  })
}

export function usePatchModel(id: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { display_name?: string; description?: string }) =>
      modelsApi.patch(id, body),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.detail(id) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.all })
    },
  })
}

export function useArchiveModel() {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: modelsApi.archive,
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.all })
    },
  })
}

export function useMarketInstruments() {
  return useQuery({
    queryKey: ['market', 'instruments'],
    queryFn: () => modelsApi.marketInstruments().then((r) => r.data.instruments),
    staleTime: 30_000,
  })
}

export function useStartTrain(id: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: {
      dataset_id?: string
      version_note?: string
      hyperparams?: Record<string, unknown>
      data?: TrainDataSelection
    }) => modelsApi.startTrain(id, body),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.runs(id) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.detail(id) })
    },
  })
}

export function useCancelRun(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (runId: string) => modelsApi.cancelRun(modelId, runId),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.runs(modelId) })
    },
  })
}

export function usePromoteVersion(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: ({ version, reason }: { version: number; reason: string }) =>
      modelsApi.promote(modelId, version, reason),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.versions(modelId) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.aliases(modelId) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.detail(modelId) })
    },
  })
}

export function useEvaluateVersion(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (version: number) => modelsApi.evaluate(modelId, version),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.evals(modelId) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.versions(modelId) })
    },
  })
}

export function useRollback(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (alias: string) => modelsApi.rollback(modelId, alias),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.aliases(modelId) })
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.deployments(modelId) })
    },
  })
}

export function useTestCases(id: string) {
  return useQuery({
    queryKey: MODEL_KEYS.testCases(id),
    queryFn: () => modelsApi.listTestCases(id).then((r) => r.data.test_cases),
    enabled: !!id,
  })
}

export function useAddTestCase(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: { name: string; input: unknown; expected?: unknown }) =>
      modelsApi.addTestCase(modelId, body),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.testCases(modelId) })
    },
  })
}

export function useDeleteTestCase(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (caseId: string) => modelsApi.deleteTestCase(modelId, caseId),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.testCases(modelId) })
    },
  })
}

export function useCreateDeployment(modelId: string) {
  const qc = useQueryClient()
  return useMutation({
    mutationFn: (body: {
      version: number
      environment: string
      alias: string
      traffic_pct: number
    }) => modelsApi.createDeployment(modelId, body),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: MODEL_KEYS.deployments(modelId) })
    },
  })
}
