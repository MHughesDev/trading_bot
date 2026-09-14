import { useState } from 'react'
import { Link, Navigate, useNavigate } from 'react-router-dom'
import { Check, ChevronRight } from 'lucide-react'
import { authApi } from '@/lib/api'
import { useAuthStore } from '@/store/auth'
import { AuthShell } from '@/components/layout/AuthShell'
import { Button } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'
import { Badge } from '@/components/primitives/Badge'
import { Label } from '@/components/primitives/Num'

type Step = 'credentials' | 'alpaca' | 'coinbase' | 'done'

const STEPS: { id: Step; label: string }[] = [
  { id: 'credentials', label: 'Account' },
  { id: 'alpaca', label: 'Alpaca' },
  { id: 'coinbase', label: 'Coinbase' },
  { id: 'done', label: 'Done' },
]

export function SignUpPage() {
  const { user, login } = useAuthStore()
  const navigate = useNavigate()
  const [step, setStep] = useState<Step>('credentials')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [alpacaKey, setAlpacaKey] = useState('')
  const [alpacaSecret, setAlpacaSecret] = useState('')
  const [coinbaseKey, setCoinbaseKey] = useState('')
  const [coinbaseSecret, setCoinbaseSecret] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  if (user && step === 'credentials') return <Navigate to="/dashboard" replace />

  async function handleCredentials(e: React.FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.register(email, password)
      await login(email, password)
      setStep('alpaca')
    } catch (err) {
      const msg = (err as { response?: { data?: { detail?: string } } })?.response?.data?.detail
      setError(msg ?? 'That email is already registered, or the password is too short.')
    } finally {
      setLoading(false)
    }
  }

  async function saveVenueKeys(next: Step) {
    setError('')
    setLoading(true)
    try {
      const creds: Record<string, string> = {}
      if (alpacaKey) creds.alpaca_api_key = alpacaKey
      if (alpacaSecret) creds.alpaca_api_secret = alpacaSecret
      if (coinbaseKey) creds.coinbase_api_key = coinbaseKey
      if (coinbaseSecret) creds.coinbase_api_secret = coinbaseSecret
      if (Object.keys(creds).length) await authApi.putVenueCredentials(creds)
      setStep(next)
      if (next === 'done') window.setTimeout(() => navigate('/dashboard'), 900)
    } catch {
      setError('Those credentials were refused by the venue. You can add them later in Settings.')
    } finally {
      setLoading(false)
    }
  }

  const stepIndex = STEPS.findIndex((s) => s.id === step)

  return (
    <AuthShell
      title={
        step === 'credentials'
          ? 'Create your account'
          : step === 'done'
            ? 'You are set up'
            : `Connect ${step === 'alpaca' ? 'Alpaca' : 'Coinbase'}`
      }
      subtitle={
        step === 'credentials'
          ? 'Paper trading is on by default. Nothing you do here risks real funds.'
          : step === 'done'
            ? 'Taking you to your dashboard.'
            : 'Optional. You can connect venues later in Settings, and everything is verified before it is saved.'
      }
      footer={
        <>
          Already have an account?{' '}
          <Link to="/login" style={{ color: 'var(--fg-link)' }}>
            Sign in
          </Link>
        </>
      }
    >
      <div className="row" style={{ gap: 6, marginBottom: 'var(--s-5)' }}>
        {STEPS.map((s, i) => (
          <div key={s.id} className="row" style={{ gap: 6, flex: 1, minWidth: 0 }}>
            <span
              aria-hidden
              style={{
                height: 3,
                flex: 1,
                borderRadius: 999,
                background: i <= stepIndex ? 'var(--bg-accent)' : 'var(--bg-inset)',
              }}
            />
          </div>
        ))}
      </div>
      <div className="between" style={{ marginBottom: 'var(--s-4)' }}>
        <Label>
          Step {stepIndex + 1} of {STEPS.length}
        </Label>
        <Label>{STEPS[stepIndex]?.label}</Label>
      </div>

      {step === 'credentials' && (
        <form onSubmit={handleCredentials} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
          <Input
            label="Email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            required
            autoFocus
            autoComplete="email"
          />
          <Input
            label="Password"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoComplete="new-password"
            hint="At least 8 characters."
            error={error || null}
          />
          <Button type="submit" variant="primary" block loading={loading} trailing={<ChevronRight size={14} aria-hidden />}>
            Create account
          </Button>
        </form>
      )}

      {(step === 'alpaca' || step === 'coinbase') && (
        <form
          onSubmit={(e) => {
            e.preventDefault()
            void saveVenueKeys(step === 'alpaca' ? 'coinbase' : 'done')
          }}
          style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}
        >
          <Input
            label="API key"
            type="password"
            autoComplete="off"
            value={step === 'alpaca' ? alpacaKey : coinbaseKey}
            onChange={(e) => (step === 'alpaca' ? setAlpacaKey(e.target.value) : setCoinbaseKey(e.target.value))}
          />
          <Input
            label="API secret"
            type="password"
            autoComplete="off"
            value={step === 'alpaca' ? alpacaSecret : coinbaseSecret}
            onChange={(e) => (step === 'alpaca' ? setAlpacaSecret(e.target.value) : setCoinbaseSecret(e.target.value))}
            error={error || null}
          />
          <div className="row" style={{ gap: 'var(--s-2)' }}>
            <Button
              type="button"
              onClick={() => setStep(step === 'alpaca' ? 'coinbase' : 'done')}
            >
              Skip
            </Button>
            <Button type="submit" variant="primary" loading={loading} className="flex-1">
              {step === 'alpaca' ? 'Save and continue' : 'Finish'}
            </Button>
          </div>
        </form>
      )}

      {step === 'done' && (
        <div className="empty" style={{ minHeight: 120 }}>
          <Badge tone="pos">
            <Check size={9} aria-hidden />
            Ready
          </Badge>
          <span className="msg">Your paper accounts are funded and waiting.</span>
        </div>
      )}
    </AuthShell>
  )
}
