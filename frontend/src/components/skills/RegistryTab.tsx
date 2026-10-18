// The registry list: every skill the caller can see and which are resident (RA-4.3).
//
// Read from `GET /api/skills/glossary`, the view a quarantined skill is absent from in the
// statement that quarantined it, so this list cannot show a skill the registry has pulled.
// The search box queries the full `SKILL.md` bodies; the response says its mode is lexical and
// the page prints that rather than implying a semantic ranking.
import { useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { EmptyState } from '@/components/primitives/States'
import { SearchInput } from '@/components/primitives/Field'
import { ReadGate } from '@/components/ops/ReadGate'
import { skillsApi, type GlossarySkill } from '@/api/skills'
import { GLOSSARY_KEY, isResident, statusTone } from '@/lib/skills/view'
import { SkillDetailPanel } from './SkillDetailPanel'

export function RegistryTab() {
  const [selected, setSelected] = useState<string | null>(null)
  const [term, setTerm] = useState('')
  const glossary = useQuery({ queryKey: GLOSSARY_KEY, queryFn: () => skillsApi.glossary() })
  const search = useQuery({
    queryKey: ['skills', 'search', term],
    queryFn: () => skillsApi.search(term),
    enabled: term.trim().length > 1,
  })

  return (
    <div className="grid-2">
      <Panel>
        <PanelHeader title="Registry" />
        <PanelBody>
          <SearchInput
            value={term}
            onValueChange={setTerm}
            placeholder="Search skill bodies"
            aria-label="Search skill bodies"
          />
          {term.trim().length > 1 ? (
            <ReadGate query={search} route="/api/skills/search" what="the search">
              {(r) => (
                <>
                  <p className="sub" data-testid="search-mode">
                    {r.mode} search. {r.note}
                  </p>
                  {r.results.length === 0 ? (
                    <EmptyState message="No skill body matches" />
                  ) : (
                    <ul>
                      {r.results.map((h) => (
                        <li key={h.skill_id}>
                          <button className="linklike" onClick={() => setSelected(h.name)}>
                            {h.name}
                          </button>{' '}
                          <Badge tone={statusTone(h.status)}>{h.status}</Badge> <span className="sub">{h.description}</span>
                        </li>
                      ))}
                    </ul>
                  )}
                </>
              )}
            </ReadGate>
          ) : (
            <ReadGate query={glossary} route="/api/skills/glossary" what="the registry">
              {(g) =>
                g.skills.length === 0 ? (
                  <EmptyState
                    message="No skill is visible to you"
                    detail="The registry is empty for this session, or every skill is quarantined."
                  />
                ) : (
                  <table className="tbl" data-testid="registry-table">
                    <thead>
                      <tr>
                        <th style={{ textAlign: 'left' }}>name</th>
                        <th>scope</th>
                        <th>status</th>
                        <th>resident</th>
                      </tr>
                    </thead>
                    <tbody>
                      {g.skills.map((s: GlossarySkill) => (
                        <tr
                          key={`${s.scope}/${s.name}`}
                          data-testid={`skill-${s.name}`}
                          aria-selected={selected === s.name}
                        >
                          <td style={{ textAlign: 'left' }}>
                            <button className="linklike" onClick={() => setSelected(s.name)}>
                              {s.name}
                            </button>
                            <div className="sub">{s.category}</div>
                          </td>
                          <td>{s.scope}</td>
                          <td>
                            <Badge tone={statusTone(s.status)}>{s.status}</Badge>
                          </td>
                          <td>{isResident(s, g.resident) ? 'resident' : 'on demand'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )
              }
            </ReadGate>
          )}
        </PanelBody>
      </Panel>
      <div>
        {selected === null ? (
          <EmptyState message="Select a skill" detail="Its usage, versions and the quarantine control appear here." />
        ) : (
          <ReadGate query={glossary} route="/api/skills/glossary" what="the registry">
            {(g) => {
              const s = g.skills.find((x) => x.name === selected)
              return s ? (
                <SkillDetailPanel skill={s} />
              ) : (
                <EmptyState message={`${selected} is not in the glossary`} detail="It may have been quarantined." />
              )
            }}
          </ReadGate>
        )}
      </div>
    </div>
  )
}
