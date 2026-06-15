import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { authApi } from '@/lib/api'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

type Step = 'email' | 'code' | 'password'

export function ForgotPasswordPage() {
  const navigate = useNavigate()
  const [step, setStep] = useState<Step>('email')
  const [email, setEmail] = useState('')
  const [code, setCode] = useState('')
  const [password, setPassword] = useState('')
  const [confirm, setConfirm] = useState('')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')

  // ── Step 1: send code ───────────────────────────────────────────────────────
  const submitEmail = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.forgotPassword(email)
      setStep('code')
    } catch (err: unknown) {
      const msg = (err as { response?: { data?: string } })?.response?.data
      setError(typeof msg === 'string' ? msg : 'No account found with that email.')
    } finally {
      setLoading(false)
    }
  }

  // ── Step 2: verify code ─────────────────────────────────────────────────────
  const submitCode = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.verifyResetCode(email, code.trim())
      setStep('password')
    } catch {
      setError('Invalid or expired code. Check your email and try again.')
    } finally {
      setLoading(false)
    }
  }

  // ── Step 3: set new password ────────────────────────────────────────────────
  const submitPassword = async (e: React.FormEvent) => {
    e.preventDefault()
    setError('')
    if (password !== confirm) { setError('Passwords do not match.'); return }
    if (password.length < 8) { setError('Password must be at least 8 characters.'); return }
    setLoading(true)
    try {
      await authApi.resetPassword(email, code.trim(), password)
      navigate('/login', { replace: true, state: { resetSuccess: true } })
    } catch {
      setError('Something went wrong. Try requesting a new code.')
    } finally {
      setLoading(false)
    }
  }

  return (
    <div className="flex h-screen items-center justify-center bg-background">
      <div className="w-full max-w-sm space-y-6 rounded-xl border border-border bg-surface p-8">

        {/* Header */}
        <div className="text-center space-y-1">
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-lg bg-blue-500 text-white font-bold text-lg mb-3">TB</div>
          <h1 className="text-xl font-semibold text-text">
            {step === 'email' && 'Forgot password'}
            {step === 'code'  && 'Enter your code'}
            {step === 'password' && 'Set new password'}
          </h1>
          <p className="text-sm text-text-muted">
            {step === 'email'    && "Enter your email and we'll send you a reset code."}
            {step === 'code'     && `We sent a 6-digit code to ${email}.`}
            {step === 'password' && 'Choose a new password for your account.'}
          </p>
        </div>

        {/* Step 1 — email */}
        {step === 'email' && (
          <form onSubmit={submitEmail} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                placeholder="you@example.com"
                value={email}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => setEmail(e.target.value)}
                required
                autoFocus
              />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Sending…' : 'Send reset code'}
            </Button>
          </form>
        )}

        {/* Step 2 — code */}
        {step === 'code' && (
          <form onSubmit={submitCode} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="code">Reset code</Label>
              <Input
                id="code"
                type="text"
                inputMode="numeric"
                pattern="[0-9]{6}"
                maxLength={6}
                placeholder="123456"
                value={code}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => setCode(e.target.value)}
                required
                autoFocus
              />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Verifying…' : 'Verify code'}
            </Button>
            <button
              type="button"
              className="w-full text-sm text-text-muted hover:text-blue-400"
              onClick={() => { setStep('email'); setCode(''); setError('') }}
            >
              Use a different email
            </button>
          </form>
        )}

        {/* Step 3 — new password */}
        {step === 'password' && (
          <form onSubmit={submitPassword} className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="password">New password</Label>
              <Input
                id="password"
                type="password"
                placeholder="At least 8 characters"
                value={password}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => setPassword(e.target.value)}
                required
                autoFocus
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="confirm">Confirm password</Label>
              <Input
                id="confirm"
                type="password"
                placeholder="••••••••"
                value={confirm}
                onChange={(e: React.ChangeEvent<HTMLInputElement>) => setConfirm(e.target.value)}
                required
              />
            </div>
            {error && <p className="text-sm text-red-400">{error}</p>}
            <Button type="submit" className="w-full" disabled={loading}>
              {loading ? 'Saving…' : 'Reset password'}
            </Button>
          </form>
        )}

        <p className="text-center text-sm text-text-muted">
          <Link to="/login" className="text-blue-400 hover:underline">
            Back to sign in
          </Link>
        </p>
      </div>
    </div>
  )
}
