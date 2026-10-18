// The skills page (Set R-a RA-4.3): the registry, the proposal queue and the notices.
//
// What this page is not: it does not edit a skill, and it has no promote control. Promotion to
// global needs six recorded conditions and is a separate decision (Set R-b RB-2.1). The only
// writes are verify, admit (confirmed) and quarantine (confirmed, with a reason), each of which
// the server refuses without `skills.admit`.
import { useState } from 'react'
import { Segmented } from '@/components/primitives/Segmented'
import { RegistryTab } from '@/components/skills/RegistryTab'
import { ProposalsTab } from '@/components/skills/ProposalsTab'
import { NoticesTab } from '@/components/skills/NoticesTab'

type Tab = 'registry' | 'proposals' | 'notices'

export function SkillsPage() {
  const [tab, setTab] = useState<Tab>('registry')
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Skills</h1>
        <div className="spacer" />
        <Segmented<Tab>
          ariaLabel="Skills section"
          value={tab}
          onChange={setTab}
          options={[
            { value: 'registry', label: 'Registry' },
            { value: 'proposals', label: 'Proposals' },
            { value: 'notices', label: 'Notices' },
          ]}
        />
      </div>
      <div className="page-body">
        {tab === 'registry' && <RegistryTab />}
        {tab === 'proposals' && <ProposalsTab />}
        {tab === 'notices' && <NoticesTab />}
      </div>
    </>
  )
}
