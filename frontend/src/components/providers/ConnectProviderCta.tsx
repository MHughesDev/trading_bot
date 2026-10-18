// The desk's call to action when nothing can run (PROVIDERS track).
//
// Drawn in place of the question box and the send button, never beside them: a
// send button over no engine is a click that fails after the person has typed
// their question, which is the dead control this replaces.
//
// One sentence and one button. It names no provider: the person goes to the
// API keys section of Settings and connects whichever key they have.
import { KeyRound } from 'lucide-react'
import { Link } from 'react-router-dom'
import { Button } from '@/components/primitives/Button'

export const AI_PROVIDERS_SETTINGS_PATH = '/settings#ai'

export function ConnectProviderCta({ detail }: { detail?: string }) {
  return (
    <div className="callout info" role="status" data-testid="connect-provider-cta">
      <KeyRound size={13} aria-hidden style={{ flex: 'none', marginTop: 2 }} />
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-2)' }}>
        <span>{detail ?? 'Add an API key to start chatting.'}</span>
        <span>
          <Link to={AI_PROVIDERS_SETTINGS_PATH}>
            <Button size="sm" variant="primary">
              Add an API key
            </Button>
          </Link>
        </span>
      </div>
    </div>
  )
}
