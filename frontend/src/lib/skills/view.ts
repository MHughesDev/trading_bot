// The skills page's wording and rules (Set R-a RA-4.3).
//
// The registry decides what is admitted; this file only gives each word a tone and decides
// when a control is *offered*. A control that is offered is never what authorises the action:
// the server's `skills.admit` check is, and a refusal is shown in its own words.
import type { GlossarySkill, ProposalStatus, SkillNotice, SkillStatus, Verification } from '@/api/skills'

export const GLOSSARY_KEY = ['skills', 'glossary'] as const
export const PROPOSALS_KEY = ['skills', 'proposals'] as const
export const NOTICES_KEY = ['skills', 'notices'] as const

export type Tone = 'pos' | 'neg' | 'warn' | 'info' | 'accent' | 'neutral' | 'outline'

export function statusTone(s: SkillStatus | string): Tone {
  switch (s) {
    case 'admitted':
      return 'pos'
    case 'provisional':
      return 'info'
    case 'quarantined':
      return 'neg'
    case 'deprecated':
    case 'archived':
      return 'outline'
    default:
      // An unrecognised standing is shown as itself, never as a quiet "ok".
      return 'warn'
  }
}

export function proposalTone(s: ProposalStatus | string): Tone {
  switch (s) {
    case 'admitted':
      return 'pos'
    case 'rejected':
      return 'neg'
    case 'verifying':
      return 'info'
    case 'submitted':
      return 'accent'
    default:
      return 'warn'
  }
}

/** The proposals still waiting for a decision, newest first as the server sent them. */
export function openProposals<T extends { status: string }>(ps: T[]): T[] {
  return ps.filter((p) => p.status === 'submitted' || p.status === 'verifying')
}

/** Whether a skill is in the set a session would carry resident. */
export function isResident(skill: Pick<GlossarySkill, 'name'>, resident: string[]): boolean {
  return resident.includes(skill.name)
}

/** Quarantine and deprecate apply to a skill that is still standing, never to one already out. */
export function canQuarantine(skill: Pick<GlossarySkill, 'skill_id' | 'status'>): boolean {
  return skill.skill_id !== null && (skill.status === 'provisional' || skill.status === 'admitted')
}

export interface AdmitControl {
  enabled: boolean
  /** Printed beside the control when it is disabled: the step that is still owed. */
  why: string | null
}

/**
 * Whether admit is offered. It needs a verification that says `admissible`, from this page's
 * own `verify` call, and a proposal that has not already been decided. The server re-checks
 * every step and the scope; this only keeps the control from being a button that can only fail.
 */
export function admitControl(status: ProposalStatus | string, v: Verification | null): AdmitControl {
  if (status === 'admitted') return { enabled: false, why: 'already admitted' }
  if (status === 'rejected') return { enabled: false, why: 'rejected' }
  if (v === null) return { enabled: false, why: 'run verify first: nothing has been checked in this view' }
  if (!v.admissible) {
    return { enabled: false, why: v.first_unmet ? `step \`${v.first_unmet}\` has not passed` : 'not admissible' }
  }
  return { enabled: true, why: null }
}

/** The steps in the order the registry requires them, each with whether it has passed. */
export function stepRows(v: Verification): Array<{ step: string; state: 'passed' | 'failed' | 'unrun' }> {
  return v.required.map((step) => ({
    step,
    state: v.steps[step] === undefined ? 'unrun' : v.steps[step] ? 'passed' : 'failed',
  }))
}

export function noticeWords(n: SkillNotice): string {
  const verb =
    n.kind === 'quarantined' ? 'quarantined' : n.kind === 'deprecated' ? 'deprecated' : n.kind === 'archived' ? 'archived' : n.kind
  return `${n.skill} was ${verb}: ${n.reason}`
}

/** A similarity as the registry measures it (lexical), as a percentage; absent stays absent. */
export function similarityWords(s: number | null): string {
  return s === null ? 'not measured' : `${Math.round(s * 100)}% like the nearest skill`
}

/**
 * Helpfulness balance in words. The balance is the sum of votes from sessions that were
 * exposed to the skill, so a skill nobody was exposed to has no balance, not a zero one.
 */
export function usageWords(u: { exposures: number; activations: number; balance: number }): string {
  if (u.exposures === 0) return 'never exposed: no usage to report'
  const rate = Math.round((u.activations / u.exposures) * 100)
  return `${u.activations} of ${u.exposures} exposures activated (${rate}%), balance ${u.balance >= 0 ? '+' : ''}${u.balance}`
}
