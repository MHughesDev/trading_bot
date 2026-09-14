import { Moon, Sun } from 'lucide-react'
import { useThemeStore } from '@/store/theme'
import { THEME_DESCRIPTION, type Theme } from '@/lib/theme'
import { Segmented } from '@/components/primitives/Segmented'

/* Spec §1.2 — two deliberately different themes, not one theme and its
   inversion. The names are part of the product vocabulary. */

const OPTIONS: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: 'paper', label: 'Paper', icon: Sun },
  { value: 'terminal', label: 'Terminal', icon: Moon },
]

export function ThemeToggle() {
  const { theme, setTheme } = useThemeStore()

  return (
    <Segmented
      ariaLabel="Theme"
      value={theme}
      onChange={setTheme}
      options={OPTIONS.map((o) => ({
        value: o.value,
        title: THEME_DESCRIPTION[o.value],
        label: (
          <span className="row" style={{ gap: 6 }}>
            <o.icon size={13} aria-hidden />
            {o.label}
          </span>
        ),
      }))}
    />
  )
}
