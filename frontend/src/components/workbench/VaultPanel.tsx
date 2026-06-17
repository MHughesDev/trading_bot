// Vault panel. Shows the one-shot holdout state (`spent`), whether the action is
// reachable (Gate-3 passed, unspent, not unsafe), and the full access log
// (who + when), forever. The action disables itself once `spent` — a second
// attempt is refused at the API level, and the UI does not offer it.
import { Lock, ShieldCheck, Loader2 } from 'lucide-react'
import type { VaultView } from '@/api/experiments'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'

export function VaultPanel({
  vault,
  onRun,
  running,
}: {
  vault: VaultView
  onRun: () => void
  running: boolean
}) {
  return (
    <Card>
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="flex items-center gap-2 text-sm">
          <Lock className="h-4 w-4 text-accent" />
          Holdout vault
        </CardTitle>
        {vault.spent ? (
          <Badge variant="inactive">spent</Badge>
        ) : vault.can_run ? (
          <Badge variant="active">unlocked · one shot</Badge>
        ) : (
          <Badge variant="outline">locked</Badge>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <p className="text-xs text-text-dim">
          The vault grants exactly one logged evaluation against data no Study has touched.
          {!vault.gate3_passed && ' It is reachable only after Gate 3 passes.'}
          {vault.unsafe && ' This experiment is flagged unsafe and cannot be validated through the vault.'}
        </p>

        <div className="flex items-center justify-between">
          <span className="text-xs text-text-dim">
            {vault.spent
              ? 'This holdout is spent — continuing requires a new experiment with genuinely new data.'
              : vault.can_run
                ? 'Ready for its single evaluation.'
                : 'Not yet reachable.'}
          </span>
          <Button
            size="sm"
            variant={vault.spent ? 'secondary' : 'default'}
            disabled={running || vault.spent || !vault.can_run}
            onClick={onRun}
          >
            {running && <Loader2 className="h-3 w-3 animate-spin" />}
            {vault.spent ? 'Spent' : 'Spend vault (once)'}
          </Button>
        </div>

        <div className="border-t border-border pt-3">
          <div className="flex items-center gap-2 text-xs text-text-dim">
            <ShieldCheck className="h-3.5 w-3.5" />
            Access log ({vault.access_log.length})
          </div>
          {vault.access_log.length === 0 ? (
            <div className="mt-1 text-xs text-text-dim">never accessed</div>
          ) : (
            <ul className="mt-2 space-y-1 text-xs">
              {vault.access_log.map((a, i) => (
                <li key={`${a.run_id}-${i}`} className="flex items-center justify-between gap-2">
                  <span className="truncate font-mono text-text-muted" title={a.run_id}>
                    {a.run_id}
                  </span>
                  <span className="shrink-0 text-text-dim">
                    {a.by} · {new Date(a.when).toLocaleString()}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </CardContent>
    </Card>
  )
}
