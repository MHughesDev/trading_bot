import { STAGE_ORDER, type Stage } from '@/api/desk'
import { readLayers, type CallLayers, type WireEvent } from '@/lib/desk/events'


/** A line diff, enough to see what changed between two steps' prompts. */
export function diffLines(previous: string, current: string): Array<{ sign: '+' | '-' | ' '; text: string }> {
  const a = previous.split('\n')
  const b = current.split('\n')
  const out: Array<{ sign: '+' | '-' | ' '; text: string }> = []
  const seen = new Set(a)
  const kept = new Set(b)
  for (const line of a) if (!kept.has(line)) out.push({ sign: '-', text: line })
  for (const line of b) out.push({ sign: seen.has(line) ? ' ' : '+', text: line })
  return out
}

/**
 * One model call, as the driver's `decision` row stored it.
 *
 * `Recorder::decision` (`local_driver.rs`) writes, per call, the `system`
 * string, the `prompt` string, the decode `schema`, the `raw_reply`, the
 * settings and the provider's usage — "deliberately the whole prompt rather
 * than an excerpt". It is on `/api/agent/runs/{id}/messages`, which is
 * mounted, while the trajectory route the scrubber was written against is not.
 */
export interface DecisionStep {
  step: number
  seq: number
  phase: string
  tool: string | null
  system: string
  prompt: string
  reply: string
  /**
   * Whether `system` + `prompt` **is** the request. True for the plan /
   * select / fill phases, where `build_request` sends the system string and a
   * single user message equal to `prompt`. False for `native`, where the
   * request carried the compacted transcript and the native tool list — and
   * neither is stored, so the prompt here is the first-turn fallback, not what
   * the model saw.
   */
  faithful: boolean
  inputTokens: number | null
  outputTokens: number | null
  /**
   * The harness's per-layer estimate of this call's prompt, from the row's
   * `layers`. `null` for a row written before the driver recorded it — never a
   * split of the provider's total the desk made up.
   */
  layers: CallLayers | null
  /** The lifecycle stage in force at this call, from the latest `stage` event. */
  stage: Stage | null
}

function strOf(v: unknown): string | null {
  return typeof v === 'string' ? v : null
}
function numOf(v: unknown): number | null {
  return typeof v === 'number' && Number.isFinite(v) ? v : null
}

export function decisionSteps(events: WireEvent[]): DecisionStep[] {
  const out: DecisionStep[] = []
  let stage: Stage | null = null
  for (const e of [...events].sort((a, b) => a.seq - b.seq)) {
    if (e.kind === 'stage') {
      const s = strOf(e.content.stage)
      if (s && (STAGE_ORDER as string[]).includes(s)) stage = s as Stage
      continue
    }
    if (e.kind !== 'decision') continue
    const c = e.content
    const system = strOf(c.system)
    const prompt = strOf(c.prompt)
    // A decision row with no prompt is not a model call this panel can show.
    if (prompt === null) continue
    const phase = strOf(c.phase) ?? 'unknown'
    const usage = (c.usage && typeof c.usage === 'object' ? c.usage : {}) as Record<string, unknown>
    out.push({
      step: numOf(c.step) ?? out.length + 1,
      seq: e.seq,
      phase,
      tool: strOf(c.tool),
      system: system ?? '',
      prompt,
      reply: strOf(c.raw_reply) ?? '',
      faithful: phase !== 'native' && phase !== 'unknown' && system !== null,
      inputTokens: numOf(usage.input_tokens),
      outputTokens: numOf(usage.output_tokens),
      layers: readLayers(c.layers),
      stage,
    })
  }
  return out
}

/** The first call index at each stage, for the stage strip's keyboard jump. */
export function firstCallAtStage(steps: DecisionStep[]): Map<Stage, number> {
  const m = new Map<Stage, number>()
  steps.forEach((s, i) => {
    if (s.stage && !m.has(s.stage)) m.set(s.stage, i)
  })
  return m
}
