import { useState } from 'react'
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query'
import { Eye, EyeOff, Save, KeyRound } from 'lucide-react'
import { authApi } from '@/lib/api'
import { useAuthStore } from '@/store/auth'
import { toast } from '@/hooks/useToast'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ThemeToggle } from '@/components/ThemeToggle'
import { cn, formatDate } from '@/lib/utils'

function MaskedInput({ label, name, value, masked, error, onChange }: {
  label: string
  name: string
  value: string
  masked?: string | null
  error?: string | null
  onChange: (v: string) => void
}) {
  const [show, setShow] = useState(false)
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <div className="relative">
        <Input
          type={show ? 'text' : 'password'}
          name={name}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={masked ? `Saved: ${masked} — leave blank to keep` : 'Not set'}
          className={cn('pr-9', error && 'border-red-500/60 focus-visible:ring-red-500/40')}
          aria-invalid={!!error}
        />
        <button
          type="button"
          onClick={() => setShow(!show)}
          className="absolute right-2.5 top-1/2 -translate-y-1/2 text-text-dim hover:text-text-muted"
        >
          {show ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
        </button>
      </div>
      {error && <p className="text-xs text-red-400">{error}</p>}
    </div>
  )
}

export function AccountPage() {
  const { user } = useAuthStore()
  const qc = useQueryClient()

  const { data: creds } = useQuery({
    queryKey: ['venue-creds'],
    queryFn: () => authApi.getVenueCredentials().then((r) => r.data),
  })

  const [alpacaKey, setAlpacaKey] = useState('')
  const [alpacaSecret, setAlpacaSecret] = useState('')
  const [coinbaseKey, setCoinbaseKey] = useState('')
  const [coinbaseSecret, setCoinbaseSecret] = useState('')
  const [fieldErrors, setFieldErrors] = useState<Record<string, string>>({})
  const [verifying, setVerifying] = useState(false)

  const saveMut = useMutation({
    mutationFn: (data: Record<string, string>) => authApi.putVenueCredentials(data),
    onSuccess: () => {
      toast({ title: 'Credentials saved', variant: 'success' })
      qc.invalidateQueries({ queryKey: ['venue-creds'] })
      setAlpacaKey('')
      setAlpacaSecret('')
      setCoinbaseKey('')
      setCoinbaseSecret('')
      setFieldErrors({})
    },
    onError: () => toast({ title: 'Save failed', variant: 'error' }),
  })

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault()
    setFieldErrors({})
    const data: Record<string, string> = {}
    if (alpacaKey) data.alpaca_api_key = alpacaKey
    if (alpacaSecret) data.alpaca_api_secret = alpacaSecret
    if (coinbaseKey) data.coinbase_api_key = coinbaseKey
    if (coinbaseSecret) data.coinbase_api_secret = coinbaseSecret
    if (Object.keys(data).length === 0) {
      toast({ title: 'No changes to save', variant: 'default' })
      return
    }

    // A venue is checkable when it ends up with a *complete* key+secret pair —
    // either freshly typed or merged with what's already saved on the server —
    // and at least one of those fields was actually edited just now. This mirrors
    // the backend's merge-then-verify behavior so a partial edit (e.g. typing
    // garbage into just the Alpaca key while leaving the secret as "keep existing")
    // still gets fully live-verified rather than silently skipped.
    const alpacaEdited = !!(alpacaKey || alpacaSecret)
    const alpacaComplete = !!((alpacaKey || creds?.alpaca_key_set) && (alpacaSecret || creds?.alpaca_secret_set))
    const coinbaseEdited = !!(coinbaseKey || coinbaseSecret)
    const coinbaseComplete = !!((coinbaseKey || creds?.coinbase_key_set) && (coinbaseSecret || creds?.coinbase_secret_set))
    const checkable = (alpacaEdited && alpacaComplete) || (coinbaseEdited && coinbaseComplete)
    if (checkable) {
      setVerifying(true)
      try {
        const res = await authApi.verifyVenueCredentials(data)
        const v = res.data ?? {}
        const errors: Record<string, string> = {}
        const collect = (result: { ok: boolean; key_error?: string | null; secret_error?: string | null; error?: string | null } | null | undefined, keyField: string, secretField: string) => {
          if (!result || result.ok) return
          if (result.key_error) errors[keyField] = result.key_error
          if (result.secret_error) errors[secretField] = result.secret_error
          if (result.error && !result.key_error && !result.secret_error) {
            errors[keyField] = result.error
            errors[secretField] = result.error
          }
        }
        collect(v.alpaca, 'alpaca_key', 'alpaca_secret')
        collect(v.coinbase, 'cb_key', 'cb_secret')

        if (Object.keys(errors).length > 0) {
          setFieldErrors(errors)
          toast({
            title: "Couldn't verify your credentials",
            description: 'Check the highlighted field(s) below — nothing was saved.',
            variant: 'error',
          })
          return
        }
      } catch {
        toast({ title: "Couldn't verify your credentials — try again", variant: 'error' })
        return
      } finally {
        setVerifying(false)
      }
    }

    saveMut.mutate(data)
  }

  const credStatus = (key: string) =>
    creds?.[key] ? (
      <span className="text-xs text-emerald-400">Connected</span>
    ) : (
      <span className="text-xs text-text-dim">Not set</span>
    )

  return (
    <div className="p-6 max-w-2xl space-y-6">
      <h1 className="text-xl font-semibold text-text">Account</h1>

      {/* Profile */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Profile</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3 text-sm">
          <div className="flex justify-between">
            <span className="text-text-muted">Email</span>
            <span className="font-mono text-text">{user?.email}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">Account created</span>
            <span className="text-text-muted">{user?.created_at ? formatDate(user.created_at) : '—'}</span>
          </div>
          <div className="flex justify-between">
            <span className="text-text-muted">User ID</span>
            <span className="font-mono text-xs text-text-dim">{user?.id}</span>
          </div>
        </CardContent>
      </Card>

      {/* Appearance */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm">Appearance</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="flex items-center justify-between gap-4">
            <div>
              <p className="text-sm text-text">Theme</p>
              <p className="text-xs text-text-dim mt-0.5">Choose how TradingBot looks on this device.</p>
            </div>
            <ThemeToggle />
          </div>
        </CardContent>
      </Card>

      {/* Venue credentials */}
      <Card>
        <CardHeader className="pb-2">
          <CardTitle className="text-sm flex items-center gap-2">
            <KeyRound className="h-4 w-4 text-blue-400" />
            Venue API Keys
          </CardTitle>
        </CardHeader>
        <CardContent>
          {/* Status summary */}
          <div className="flex gap-6 mb-4 pb-4 border-b border-border text-sm">
            <div className="space-y-0.5">
              <p className="text-text-dim text-xs uppercase tracking-widest">Alpaca</p>
              {credStatus('alpaca_key_set')}
            </div>
            <div className="space-y-0.5">
              <p className="text-text-dim text-xs uppercase tracking-widest">Coinbase</p>
              {credStatus('coinbase_key_set')}
            </div>
          </div>

          <form onSubmit={handleSave} className="space-y-4">
            <div className="space-y-3">
              <p className="text-xs font-semibold uppercase tracking-widest text-text-dim">
                <a href="https://app.alpaca.markets/login" target="_blank" rel="noopener noreferrer" className="hover:text-text-muted hover:underline">
                  Alpaca
                </a>
              </p>
              <MaskedInput label="API Key" name="alpaca_key" value={alpacaKey} masked={creds?.alpaca_key_masked} error={fieldErrors.alpaca_key} onChange={(v) => { setAlpacaKey(v); setFieldErrors((f) => ({ ...f, alpaca_key: '' })) }} />
              <MaskedInput label="Secret Key" name="alpaca_secret" value={alpacaSecret} masked={creds?.alpaca_secret_masked} error={fieldErrors.alpaca_secret} onChange={(v) => { setAlpacaSecret(v); setFieldErrors((f) => ({ ...f, alpaca_secret: '' })) }} />
            </div>

            <div className="border-t border-border pt-4 space-y-3">
              <p className="text-xs font-semibold uppercase tracking-widest text-text-dim">
                <a href="https://www.coinbase.com/signin" target="_blank" rel="noopener noreferrer" className="hover:text-text-muted hover:underline">
                  Coinbase
                </a>
              </p>
              <MaskedInput label="API Key" name="cb_key" value={coinbaseKey} masked={creds?.coinbase_key_masked} error={fieldErrors.cb_key} onChange={(v) => { setCoinbaseKey(v); setFieldErrors((f) => ({ ...f, cb_key: '' })) }} />
              <MaskedInput label="API Secret" name="cb_secret" value={coinbaseSecret} masked={creds?.coinbase_secret_masked} error={fieldErrors.cb_secret} onChange={(v) => { setCoinbaseSecret(v); setFieldErrors((f) => ({ ...f, cb_secret: '' })) }} />
            </div>

            <Button type="submit" disabled={saveMut.isPending || verifying}>
              <Save className="h-4 w-4" />
              {verifying ? 'Verifying keys…' : saveMut.isPending ? 'Saving…' : 'Save credentials'}
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  )
}
