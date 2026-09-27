import { useQuery } from '@tanstack/react-query'
import { fetchAlerts } from '@/api/portfolio'

export function useAlerts() {
  return useQuery({
    queryKey: ['alerts'],
    queryFn: fetchAlerts,
    refetchInterval: 30_000,
    retry: 1,
  })
}
