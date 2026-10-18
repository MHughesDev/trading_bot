import { useQueries } from '@tanstack/react-query'
import { modelsApi, type AiModel } from '@/api/mlops'
import { MODEL_KEYS } from '@/hooks/useMlOps'
import { normaliseUsedBy, summariseModel, type ModelSummary } from '@/components/mlops/studio.helpers'

/**
 * One summary per model for the command center: versions (headline metric and
 * sparkline), aliases (production version) and used-by. Three reads per model,
 * bounded by the page size and cached under the same keys the cockpit uses, so
 * opening a model reuses what its card already fetched.
 *
 * A read that has not answered leaves its field unknown rather than empty:
 * `usedBy` stays `null` until its own query resolves, so a chip row is never
 * rendered as "used by nothing" while it is still loading.
 */
export function useModelSummaries(models: AiModel[]): {
  summaries: Record<string, ModelSummary>
  loading: boolean
} {
  const versionQs = useQueries({
    queries: models.map((m) => ({
      queryKey: MODEL_KEYS.versions(m.model_id),
      queryFn: () => modelsApi.listVersions(m.model_id).then((r) => r.data.versions),
      staleTime: 15_000,
    })),
  })
  const aliasQs = useQueries({
    queries: models.map((m) => ({
      queryKey: MODEL_KEYS.aliases(m.model_id),
      queryFn: () => modelsApi.getAliases(m.model_id).then((r) => r.data),
      staleTime: 15_000,
    })),
  })
  const usedByQs = useQueries({
    queries: models.map((m) => ({
      queryKey: MODEL_KEYS.usedBy(m.model_id),
      queryFn: () => modelsApi.usedBy(m.model_id).then((r) => normaliseUsedBy(r.data.strategies)),
      staleTime: 30_000,
    })),
  })

  const summaries: Record<string, ModelSummary> = {}
  models.forEach((m, i) => {
    summaries[m.model_id] = summariseModel({
      versions: versionQs[i]?.data ?? null,
      aliases: aliasQs[i]?.data ?? null,
      usedBy: usedByQs[i]?.data ?? null,
    })
  })
  const loading = [...versionQs, ...aliasQs, ...usedByQs].some((q) => q.isLoading)
  return { summaries, loading }
}
