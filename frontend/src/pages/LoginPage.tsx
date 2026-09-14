import { useState } from 'react'
import { Link, Navigate, useLocation, useNavigate } from 'react-router-dom'
import { CheckCircle2 } from 'lucide-react'
import { useAuthStore } from '@/store/auth'
import { AuthShell } from '@/components/layout/AuthShell'
import { Button } from '@/components/primitives/Button'
import { Input } from '@/components/primitives/Field'

export function LoginPage() {
  const { user, login, loading } = useAuthStore()
  const navigate = useNavigate()
  const location = useLocation()
  const resetSuccess = (location.state as { resetSuccess?: boolean } | null)?.resetSuccess
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')

  if (user) return <Navigate to="/dashboard" replace />

  async function handleSubmit(e: React.FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError('')
    try {
      await login(email, password)
      navigate('/dashboard')
    } catch {
      setError('That email and password do not match an account.')
    }
  }

  return (
    <AuthShell
      title="Sign in"
      subtitle="Multi-asset trading, research and automation."
      footer={
        <>
          No account?{' '}
          <Link to="/signup" style={{ color: 'var(--fg-link)' }}>
            Create one
          </Link>
        </>
      }
    >
      {resetSuccess && (
        <div className="callout pos" style={{ marginBottom: 'var(--s-4)' }}>
          <CheckCircle2 size={14} aria-hidden style={{ flex: 'none', marginTop: 1 }} />
          <span>Password reset. Sign in with your new password.</span>
        </div>
      )}

      <form onSubmit={handleSubmit} style={{ display: 'flex', flexDirection: 'column', gap: 'var(--s-4)' }}>
        <Input
          id="email"
          label="Email"
          type="email"
          placeholder="you@example.com"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          required
          autoFocus
          autoComplete="email"
        />
        <Input
          id="password"
          label="Password"
          labelAside={
            <Link to="/forgot-password" style={{ color: 'var(--fg-link)', fontSize: 'var(--t-11)' }}>
              Forgot password?
            </Link>
          }
          type="password"
          placeholder="••••••••"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          required
          autoComplete="current-password"
          error={error || null}
        />
        <Button type="submit" variant="primary" block loading={loading}>
          Sign in
        </Button>
      </form>
    </AuthShell>
  )
}
