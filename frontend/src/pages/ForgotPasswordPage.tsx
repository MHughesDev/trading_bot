import { useState } from 'react'
import { Link, useNavigate } from 'react-router-dom'
import { authApi } from '@/lib/api'
import { AuthShell } from '@/components/layout/AuthShell'
import { Button } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'

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

  async function submitEmail(e: React.FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.forgotPassword(email)
      setStep('code')
    } catch (err) {
      const msg = (err as { response?: { data?: string } })?.response?.data
      setError(typeof msg === 'string' ? msg : 'Could not send a reset code to that address.')
    } finally {
      setLoading(false)
    }
  }

  async function submitCode(e: React.FormEvent) {
    e.preventDefault()
    setError('')
    setLoading(true)
    try {
      await authApi.verifyResetCode(email, code.trim())
      setStep('password')
    } catch {
      setError('That code is not valid, or it has expired.')
    } finally {
      setLoading(false)
    }
  }

  async function submitPassword(e: React.FormEvent) {
    e.preventDefault()
    setError('')
    if (password !== confirm) {
      setError('The two passwords do not match.')
      return
    }
    setLoading(true)
    try {
      await authApi.resetPassword(email, code.trim(), password)
      navigate('/login', { state: { resetSuccess: true } })
    } catch {
      setError('Could not reset the password. Request a new code and try again.')
    } finally {
      setLoading(false)
    }
  }

  return (
    <AuthShell
      title={step === 'email' ? 'Forgot password' : step === 'code' ? 'Enter your code' : 'Set a new password'}
      subtitle={
        step === 'email'
          ? 'Enter your email and we will send a reset code.'
          : step === 'code'
            ? `We sent a 6-digit code to ${email}.`
            : 'Choose a new password for your account.'
      }
      footer={
        <Link to="/login" style={{ color: 'var(--fg-link)' }}>
          Back to sign in
        </Link>
      }
    >
      {step === 'email' && (
        <form onSubmit={submitEmail} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
          <Input
            label="Email"
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@example.com"
            required
            autoFocus
            autoComplete="email"
            error={error || null}
          />
          <Button type="submit" variant="primary" block loading={loading}>
            Send reset code
          </Button>
        </form>
      )}

      {step === 'code' && (
        <form onSubmit={submitCode} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
          <Input
            label="Reset code"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            placeholder="000000"
            inputMode="numeric"
            maxLength={6}
            required
            autoFocus
            numeric
            error={error || null}
            hint="The code expires shortly after it is sent."
          />
          <Button type="submit" variant="primary" block loading={loading}>
            Verify code
          </Button>
          <Button type="button" variant="ghost" block onClick={() => setStep('email')}>
            Use a different email
          </Button>
        </form>
      )}

      {step === 'password' && (
        <form onSubmit={submitPassword} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
          <Input
            label="New password"
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            required
            autoFocus
            autoComplete="new-password"
            hint="At least 8 characters."
          />
          <Input
            label="Confirm password"
            type="password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            required
            autoComplete="new-password"
            error={error || null}
          />
          <Button type="submit" variant="primary" block loading={loading}>
            Reset password
          </Button>
        </form>
      )}
    </AuthShell>
  )
}
