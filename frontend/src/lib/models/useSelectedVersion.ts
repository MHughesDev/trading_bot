import { useSearchParams } from 'react-router-dom'
import { useModelAliases, useModelVersions } from '@/hooks/useMlOps'

/**
 * The version a cockpit tab is looking at: `?v=` if it names a real version,
 * else the production alias, else the newest. `null` only when there is none.
 */
export function useSelectedVersion(modelId: string) {
  const versions = useModelVersions(modelId)
  const aliases = useModelAliases(modelId)
  const [params, setParams] = useSearchParams()

  const list = versions.data ?? []
  const nums = list.map((v) => v.version)
  const asked = Number(params.get('v'))
  const production = aliases.data?.production
  let version: number | null = null
  if (Number.isInteger(asked) && nums.includes(asked)) version = asked
  else if (typeof production === 'number' && nums.includes(production)) version = production
  else if (nums.length > 0) version = Math.max(...nums)

  const select = (v: number) => {
    const next = new URLSearchParams(params)
    next.set('v', String(v))
    setParams(next, { replace: true })
  }
  return { version, versions: list, aliases: aliases.data, select, query: versions }
}
