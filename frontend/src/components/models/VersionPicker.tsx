import type { AliasMap, ModelVersion } from '@/api/mlops'

/** A native select of versions, each labelled with the aliases it holds. */
export function VersionPicker({
  versions,
  aliases,
  value,
  onChange,
}: {
  versions: ModelVersion[]
  aliases: AliasMap | undefined
  value: number | null
  onChange: (v: number) => void
}) {
  if (versions.length === 0) return null
  return (
    <label style={{ display: 'inline-flex', gap: 'var(--s-2)', alignItems: 'center' }}>
      <span className="lbl">Version</span>
      <select
        aria-label="Version"
        value={value ?? ''}
        onChange={(e) => onChange(Number(e.target.value))}
        style={{
          height: 28,
          background: 'var(--bg-sunken)',
          color: 'var(--fg-primary)',
          border: '1px solid var(--line-hairline)',
          borderRadius: 'var(--r-sm)',
        }}
      >
        {[...versions]
          .sort((a, b) => b.version - a.version)
          .map((v) => {
            const held = Object.entries(aliases ?? {})
              .filter(([, n]) => n === v.version)
              .map(([a]) => a)
            return (
              <option key={v.version} value={v.version}>
                v{v.version}
                {held.length > 0 ? ` (${held.join(', ')})` : ''}
              </option>
            )
          })}
      </select>
    </label>
  )
}
