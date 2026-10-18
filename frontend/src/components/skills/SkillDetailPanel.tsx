// One skill: its standing, its usage and its versions, with the quarantine control (RA-4.3).
//
// Quarantine is the one write on this panel. It is confirmed in a dialog that states the
// consequence (the skill leaves every glossary in the statement that quarantines it, and
// live sessions are told), takes the reason the registry requires, and is refused by the
// server unless the session holds `skills.admit`; the refusal is shown in its own words.
import { useState } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { Badge } from '@/components/primitives/Badge'
import { Button } from '@/components/primitives/Button'
import { Modal } from '@/components/primitives/Modal'
import { TextArea } from '@/components/primitives/Field'
import { Panel, PanelBody, PanelHeader } from '@/components/primitives/Panel'
import { ReadGate } from '@/components/ops/ReadGate'
import { refusalWords } from '@/api/ops'
import { skillsApi, type GlossarySkill } from '@/api/skills'
import { GLOSSARY_KEY, NOTICES_KEY, canQuarantine, statusTone, usageWords } from '@/lib/skills/view'
import { dateTime } from '@/lib/format'

export function SkillDetailPanel({ skill }: { skill: GlossarySkill }) {
  const qc = useQueryClient()
  const id = skill.skill_id
  const q = useQuery({
    queryKey: ['skills', 'item', id],
    queryFn: () => skillsApi.item(id as string),
    enabled: id !== null,
  })
  const [open, setOpen] = useState(false)
  const [reason, setReason] = useState('')
  const quarantine = useMutation({
    mutationFn: () => skillsApi.quarantine(id as string, reason.trim()),
    onSuccess: () => {
      setOpen(false)
      setReason('')
      void qc.invalidateQueries({ queryKey: GLOSSARY_KEY })
      void qc.invalidateQueries({ queryKey: NOTICES_KEY })
      void qc.invalidateQueries({ queryKey: ['skills', 'item', id] })
    },
  })

  return (
    <Panel>
      <PanelHeader
        title={skill.name}
        badges={<Badge tone={statusTone(skill.status)}>{skill.status}</Badge>}
        actions={
          <Button
            size="xs"
            variant="danger"
            disabled={!canQuarantine(skill)}
            onClick={() => setOpen(true)}
            data-testid="quarantine-open"
          >
            Quarantine
          </Button>
        }
      />
      <PanelBody>
        <p className="sub">{skill.description}</p>
        {id === null ? (
          <p className="sub">This skill has no id in the registry view, so it cannot be inspected or changed here.</p>
        ) : (
          <ReadGate query={q} route={`/api/skills/item/${id}`} what="this skill's usage">
            {(d) => (
              <>
                <p data-testid="skill-usage">{usageWords(d.usage)}</p>
                {d.resolves_to && (
                  <p className="sub">
                    Deprecated; lookups resolve to <strong>{d.resolves_to.name}</strong>.
                  </p>
                )}
                <table className="tbl">
                  <thead>
                    <tr>
                      <th style={{ textAlign: 'left' }}>version</th>
                      <th style={{ textAlign: 'left' }}>by</th>
                      <th>when</th>
                    </tr>
                  </thead>
                  <tbody>
                    {d.versions.map((v) => (
                      <tr key={v.version}>
                        <td style={{ textAlign: 'left' }}>
                          <code>{v.version.slice(0, 12)}</code>
                          {v.version === d.current_version && <Badge tone="accent"> current</Badge>}
                        </td>
                        <td style={{ textAlign: 'left' }}>{v.created_by}</td>
                        <td>{dateTime(v.created_at)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </>
            )}
          </ReadGate>
        )}
      </PanelBody>
      <Modal
        open={open}
        onClose={() => setOpen(false)}
        dismissible={false}
        title={`Quarantine ${skill.name}`}
        footer={
          <>
            <span className="spacer" />
            <Button onClick={() => setOpen(false)}>Cancel</Button>
            <Button
              variant="danger"
              disabled={reason.trim() === ''}
              loading={quarantine.isPending}
              onClick={() => quarantine.mutate()}
              data-testid="quarantine-confirm"
            >
              Quarantine skill
            </Button>
          </>
        }
      >
        <p>
          The skill leaves every glossary in the same statement, and sessions that have it resident are told. Quarantine is
          not undone from this page.
        </p>
        <TextArea
          label="Reason (recorded, and sent to live sessions)"
          value={reason}
          onChange={(e) => setReason(e.target.value)}
        />
        {quarantine.isError && (
          <p role="alert" className="sub">
            Refused: {refusalWords(quarantine.error)}
          </p>
        )}
      </Modal>
    </Panel>
  )
}
