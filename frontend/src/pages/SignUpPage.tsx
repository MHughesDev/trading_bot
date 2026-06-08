import { useState } from 'react'
import { Link, Navigate, useNavigate } from 'react-router-dom'
import { useAuthStore } from '@/store/auth'
import { authApi } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

type Step = 'credentials' | 'alpaca' | 'coinbase' | 'done'

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

  if (user) return <Navigate to="/dashboard" replace />

  const handleCredentials = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.register(email, password)
      await login(email, password)
      setStep('alpaca')
    } catch (err: unknown) {
      const msg = (err as { response?: { data?: { detail?: string } } })?.response?.data?.detail
      setError(msg ?? 'Registration failed.')
    } finally {
      setLoading(false)
    }
  }

  const handleVenueKeys = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      const creds: Record<string, string> = {}
      if (step === 'alpaca') {
        if (alpacaKey) creds.alpaca_api_key = alpacaKey
        if (alpacaSecret) creds.alpaca_api_secret = alpacaSecret
      } else {
        if (coinbaseKey) creds.coinbase_api_key = coinbaseKey
        if (coinbaseSecret) creds.coinbase_api_secret = coinbaseSecret
      }
      if (Object.keys(creds).length) await authApi.putVenueCredentials(creds)
      if (step === 'alpaca') setStep('coinbase')
      else { setStep('done'); setTimeout(() => navigate('/dashboard'), 1500) }
    } catch {
      setError('Failed to save credentials.')
    } finally {
      setLoading(false)
    }
  }

  const stepNum = { credentials: 1, alpaca: 2, coinbase: 3, done: 4 }[step]

  return (
    <div className="flex h-screen items-center justify-center bg-background">
      <div className="w-full max-w-sm space-y-6 rounded-xl border border-border bg-surface p-8">
        <div className="text-center space-y-1">
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-lg bg-blue-500 text-white font-bold text-lg mb-3">TB</div>
          <h1 className="text-xl font-semibold text-text">Create account</h1>
          <p className="text-sm text-text-muted">Step {stepNum} of 3</p>
        </div>

        {step === 'credentials' && (
          <form onSubmit={handleCredentials} className="space-y-4">
            <div className="space-y-1.5">
              <Label>Email</Label>
              <Input type="email" value={email} onChange={(e) => setEmail(e.target.value)} required autoFocus />
            </div>
            <div className="space-y-1.5">
              <Label>Password</Label>
              <Input type="password" value={password} onChange={(e) => setPassword(e.target.value)} required minLength={8} />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Creating…' : 'Continue'}
            </Button>
          </form>
        )}

        {step === 'alpaca' && (
          <form onSubmit={handleVenueKeys} className="space-y-4">
            <p className="text-sm text-text-muted">
              Add your{' '}
              <a href="https://app.alpaca.markets/login" target="_blank" rel="noopener noreferrer" className="text-blue-400 hover:underline">
                Alpaca
              </a>{' '}
              API keys (optional — skip to continue).
            </p>
            <div className="space-y-1.5">
              <Label>Alpaca API Key</Label>
              <Input value={alpacaKey} onChange={(e) => setAlpacaKey(e.target.value)} placeholder="PK…" />
            </div>
            <div className="space-y-1.5">
              <Label>Alpaca Secret Key</Label>
              <Input type="password" value={alpacaSecret} onChange={(e) => setAlpacaSecret(e.target.value)} />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Saving…' : 'Continue'}
            </Button>
            <Button type="button" variant="ghost" className="w-full" onClick={() => setStep('coinbase')}>
              Skip
            </Button>
          </form>
        )}

        {step === 'coinbase' && (
          <form onSubmit={handleVenueKeys} className="space-y-4">
            <p className="text-sm text-text-muted">
              Add your{' '}
              <a href="https://www.coinbase.com/signin" target="_blank" rel="noopener noreferrer" className="text-blue-400 hover:underline">
                Coinbase
              </a>{' '}
              API keys (optional — skip to continue).
            </p>
            <div className="space-y-1.5">
              <Label>Coinbase API Key</Label>
              <Input value={coinbaseKey} onChange={(e) => setCoinbaseKey(e.target.value)} />
            </div>
            <div className="space-y-1.5">
              <Label>Coinbase API Secret</Label>
              <Input type="password" value={coinbaseSecret} onChange={(e) => setCoinbaseSecret(e.target.value)} />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Saving…' : 'Finish setup'}
            </Button>
            <Button type="button" variant="ghost" className="w-full" onClick={() => { setStep('done'); setTimeout(() => navigate('/dashboard'), 1500) }}>
              Skip
            </Button>
          </form>
        )}

        {step === 'done' && (
          <div className="text-center space-y-3">
            <p className="text-emerald-400 font-medium">Account created!</p>
            <p className="text-sm text-text-muted">Redirecting to dashboard…</p>
          </div>
        )}

        <p className="text-center text-sm text-text-muted">
          Already have an account?{' '}
          <Link to="/login" className="text-blue-400 hover:underline">Sign in</Link>
        </p>
      </div>
    </div>
  )
}
