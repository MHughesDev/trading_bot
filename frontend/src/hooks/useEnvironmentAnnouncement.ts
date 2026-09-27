import { useEffect } from 'react'
import { useModeStore } from '@/store/mode'

/** Announces the environment on every route change for screen-reader users. */
export function useEnvironmentAnnouncement() {
  const { mode } = useModeStore()
  useEffect(() => {
    const el = document.getElementById('env-live-region')
    if (el) el.textContent = `Environment: ${mode === 'LIVE' ? 'Live trading' : 'Paper trading'}`
  }, [mode])
}
