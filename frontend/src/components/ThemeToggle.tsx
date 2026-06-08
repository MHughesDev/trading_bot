import { Sun, Moon } from 'lucide-react'
import { useThemeStore, type Theme } from '@/store/theme'
import { cn } from '@/lib/utils'

const OPTIONS: { value: Theme; label: string; icon: typeof Sun }[] = [
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
]

export function ThemeToggle() {
  const { theme, setTheme } = useThemeStore()

  return (
    <div className="inline-flex items-center rounded-md border border-border-2 bg-background p-1 gap-1">
      {OPTIONS.map(({ value, label, icon: Icon }) => (
        <button
          key={value}
          type="button"
          onClick={() => setTheme(value)}
          aria-pressed={theme === value}
          className={cn(
            'flex items-center gap-1.5 rounded px-3 py-1.5 text-sm font-medium transition-colors',
            theme === value
              ? 'bg-surface-2 text-text shadow-sm'
              : 'text-text-muted hover:text-text'
          )}
        >
          <Icon className="h-4 w-4" />
          {label}
        </button>
      ))}
    </div>
  )
}
