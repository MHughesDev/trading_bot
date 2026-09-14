import type { ReactNode } from 'react'
import { Panel, PanelBody } from '@/components/primitives/Panel'
import { ThemeToggle } from '@/components/ThemeToggle'

/* The signed-out shell. Same token layer, same type, same geometry as the app —
   the first screen a user sees should already be the product. */

export function AuthShell({
  title,
  subtitle,
  children,
  footer,
}: {
  title: string
  subtitle: ReactNode
  children: ReactNode
  footer?: ReactNode
}) {
  return (
    <div className="auth-shell">
      <div className="auth-card">
        <div className="row" style={{ justifyContent: 'center', gap: 'var(--s-3)', marginBottom: 'var(--s-2)' }}>
          <div className="brand-mark" aria-hidden>
            <svg width="14" height="14" viewBox="0 0 16 16" fill="none">
              <path
                d="M2 11.5 6 6l3 3.2L14 3.5"
                stroke="currentColor"
                strokeWidth="1.9"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          </div>
          <span className="brand-name">Meridian</span>
        </div>

        <Panel>
          <PanelBody style={{ padding: 'var(--s-6)' }}>
            <div style={{ textAlign: 'center', marginBottom: 'var(--s-5)' }}>
              <h1 className="h1" style={{ marginBottom: 4 }}>
                {title}
              </h1>
              <p className="sec" style={{ fontSize: 'var(--t-13)' }}>
                {subtitle}
              </p>
            </div>
            {children}
          </PanelBody>
        </Panel>

        <div className="row" style={{ justifyContent: 'space-between' }}>
          <div style={{ fontSize: 'var(--t-12)' }} className="mut">
            {footer}
          </div>
          <ThemeToggle />
        </div>
      </div>
    </div>
  )
}
