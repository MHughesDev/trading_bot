// The skill registry's data layer (Set R-a RA-4.3; routes in crates/api/src/routes/skills.rs).
//
// Reads resolve to a named `Read<T>` (see `api/ops.ts`) so a registry that failed to load is
// never drawn as an empty registry. Writes are plain posts: a refusal is the server's own
// sentence (`refusalWords`), because the registry is the authority on what may be admitted and
// this client holds no second copy of its rules.
//
// Every path below is a literal the server mounts; `the-skills-client-calls-only-routes-the-
// server-serves.test.ts` checks that against the router's `.route(...)` table.
import { api as client } from '@/lib/api'
import { read } from '@/api/ops'

export type SkillScope = 'core' | 'global' | 'user' | 'project'
export type SkillStatus = 'provisional' | 'admitted' | 'quarantined' | 'deprecated' | 'archived'

/** One row of the glossary: what a session could see. Quarantined skills are not in it. */
export interface GlossarySkill {
  /** Addresses the item, quarantine and promote routes. */
  skill_id: string | null
  name: string
  scope: SkillScope
  status: SkillStatus
  category: string
  description: string
  /** The admitted version's content hash. */
  version: string
  pinned: boolean
}

export interface Glossary {
  skills: GlossarySkill[]
  /** Names a session would carry resident. */
  resident: string[]
  /** What the 40-skill cap cut, as the materialiser reports it. */
  cut: unknown
}

export interface SkillVersion {
  version: string
  parent: string | null
  description: string
  created_by: string
  created_at: string
}

export interface SkillDetail {
  skill_id: string
  name: string
  scope: SkillScope
  status: SkillStatus
  category: string
  current_version: string | null
  pinned: boolean
  alias_of: string | null
  resolves_to: { skill_id: string; name: string; version: string } | null
  usage: { exposures: number; activations: number; balance: number }
  versions: SkillVersion[]
}

export type ProposalStatus = 'submitted' | 'verifying' | 'rejected' | 'admitted'

export interface Proposal {
  proposal_id: string
  name: string
  project_id: string
  proposer: string
  /** The trajectory kind it was proposed from: T1 to T4. */
  reason: string
  status: ProposalStatus
  /** Cosine over word counts to the nearest existing skill; lexical, and the registry says so. */
  similarity: number | null
  created_at: string
}

/** What `POST .../verify` reports: which steps passed and which is still owed. */
export interface Verification {
  proposal_id: string
  steps: Record<string, boolean>
  required: string[]
  first_unmet: string | null
  admissible: boolean
}

export interface SkillNotice {
  notice_id: number
  skill: string
  kind: 'quarantined' | 'deprecated' | 'archived' | string
  reason: string
}

export interface SearchHit {
  name: string
  scope: SkillScope
  status: SkillStatus
  description: string
  rank: number
  skill_id: string
}

export interface SearchResult {
  mode: string
  embedder: boolean
  note: string
  results: SearchHit[]
}

export const skillsApi = {
  glossary: () => read(client.get<Glossary>('/api/skills/glossary')),
  search: (q: string) => read(client.get<SearchResult>('/api/skills/search', { params: { q } })),
  item: (id: string) => read(client.get<SkillDetail>(`/api/skills/item/${encodeURIComponent(id)}`)),
  proposals: (status?: ProposalStatus) =>
    read(
      client.get<{ proposals: Proposal[] }>('/api/skills/proposals', {
        params: status ? { status } : undefined,
      }),
    ),
  notices: (after?: number) =>
    read(
      client.get<{ notices: SkillNotice[] }>('/api/skills/notices', {
        params: after === undefined ? undefined : { after },
      }),
    ),
  /** Runs what the platform can run and reports the first unmet step. Needs `skills.admit`. */
  verify: (proposalId: string) =>
    client.post<Verification>(`/api/skills/proposals/${encodeURIComponent(proposalId)}/verify`),
  /** Admits a verified proposal as **provisional**. Needs `skills.admit`; the page confirms first. */
  admit: (proposalId: string) =>
    client.post<{ skill_id: string; status: 'provisional' }>(
      `/api/skills/proposals/${encodeURIComponent(proposalId)}/admit`,
    ),
  /** Out of every glossary in one statement, with a notice for live sessions. */
  quarantine: (skillId: string, reason: string) =>
    client.post<{ skill_id: string; status: string }>(
      `/api/skills/${encodeURIComponent(skillId)}/quarantine`,
      { reason },
    ),
}
