// Research workspace API (COMP-006).
//
// The same routes the agent uses (D-05). That is not a style preference: a UI with
// its own endpoints drifts from what the agent can actually do, and the first time
// anyone notices is when a human sees something in the workspace that the agent
// swears is not there.
import { api as client, getStoredToken } from '@/lib/api'

// The shared client, not a new axios instance. Its request interceptor attaches the
// bearer token; a second instance would silently send unauthenticated requests and
// the first symptom would be an empty workspace rather than an error.

export type ProjectKind = 'research' | 'desk'

export interface Project {
  project_id: string
  user_id: string
  name: string
  kind: ProjectKind
  goal?: string | null
  instruments: string[]
  /** null on the Desk, which means "now". */
  research_cutoff?: string | null
  created_at: string
}

export interface AgentSession {
  session_id: string
  project_id: string
  state: string
  is_initializer: boolean
  spend_usd: number
  started_at?: string | null
  ended_at?: string | null
  created_at: string
  report_id?: string | null
  abort_reason?: string | null
}

export interface TimelineEvent {
  id: number
  seq: number
  kind: string
  payload: Record<string, unknown>
  created_at: string
}

export interface Approval {
  approval_id: string
  project_id: string
  session_id?: string | null
  kind: string
  payload: Record<string, unknown>
  options?: unknown
  default_option?: string | null
  timeout_at?: string | null
  state: 'pending' | 'answered' | 'defaulted' | 'cancelled'
  answer?: { option?: string; note?: string } | null
  answered_by?: string | null
  answered_at?: string | null
  created_at: string
}

export interface Usage {
  input_tokens: number
  output_tokens: number
  cache_read: number
  cache_write: number
  cost_usd: number
  cache_hit_rate: number
  verdicts: number
  dollars_per_verdict: number | null
}

export interface Job {
  job_id: string
  kind: string
  state: string
  project_id?: string | null
  progress?: { pct?: number; stage?: string; message?: string } | null
  result_summary?: string | null
  error?: { code?: string; fix?: string } | null
  created_at: string
  finished_at?: string | null
}

export const researchApi = {
  projects: () => client.get<{ projects: Project[] }>('/api/projects'),
  project: (id: string) => client.get<Project>(`/api/projects/${id}`),

  sessions: (projectId?: string) =>
    client.get<{ sessions: AgentSession[] }>('/api/agent/sessions', {
      params: projectId ? { project_id: projectId } : undefined,
    }),
  session: (id: string) => client.get<AgentSession>(`/api/agent/sessions/${id}`),

  steer: (
    sessionId: string,
    body: { kind: 'steer' | 'interrupt' | 'stop' | 'answer'; text?: string },
  ) => client.post(`/api/agent/sessions/${sessionId}/steer`, body),

  usage: (params: { project_id?: string; session_id?: string } = {}) =>
    client.get<Usage>('/api/agent/usage', { params }),

  approvals: (params: { project_id?: string; state?: string } = {}) =>
    client.get<{ approvals: Approval[]; kinds: string[] }>('/api/approvals', {
      params,
    }),
  answerApproval: (id: string, option: string, note?: string) =>
    client.post(`/api/approvals/${id}/answer`, { option, note }),

  jobs: (projectId?: string) =>
    client.get<{ jobs: Job[] }>('/api/jobs', {
      params: projectId ? { project_id: projectId } : undefined,
    }),

  // Through the shared client, so the bearer token is attached. A raw `fetch` here
  // silently sends no Authorization header, and the pane renders empty rather than
  // erroring - which is the worst way for an auth bug to present.
  workspaceFile: (projectId: string, path: string) =>
    client.get<{ path: string; handle: string; content: string }>(
      `/api/agent/projects/${projectId}/workspace/files`,
      { params: { path } },
    ),

  report: (reportId: string) => client.get<unknown>(`/api/reports/${reportId}`),

  syntheticCatalogue: () =>
    client.get<{
      instruments: Array<{
        instrument_id: string
        generator: string
        timeframe: string
        length: number
        start: string
        public_meta: Record<string, unknown>
      }>
    }>('/api/data/synthetic'),
}

/**
 * Opens the session timeline (UI-02).
 *
 * **Why not `EventSource`.** The platform authenticates with a bearer token and has
 * no cookie session, and `EventSource` cannot set a request header. The two ways out
 * are putting the token in the query string — where it lands in server logs, proxy
 * logs and browser history — or reading the stream with `fetch`. This does the
 * latter. It costs a hand-rolled reconnect; the other option costs a leaked
 * credential every time anyone opens the timeline.
 *
 * Reconnect resumes from the last event id received, sent as `Last-Event-ID`, which
 * is the same header `EventSource` would have sent. The server reads it and replays
 * only what came after, so a closed laptop lid does not lose the middle of a
 * session.
 */
export function openTimeline(
  sessionId: string,
  onEvent: (e: TimelineEvent) => void,
  onError?: (reason: string) => void,
): () => void {
  const controller = new AbortController()
  let lastId: string | null = null
  let closed = false
  // Backoff, so a server that is down is not hammered by every open tab.
  let backoffMs = 1_000

  const parseFrame = (frame: string) => {
    let data = ''
    for (const line of frame.split('\n')) {
      if (line.startsWith('id:')) lastId = line.slice(3).trim()
      else if (line.startsWith('data:')) data += line.slice(5).trim()
      // `event:` is carried inside the JSON payload as `kind`, so the frame's own
      // event name is redundant here and is skipped rather than parsed twice.
    }
    if (!data) return
    try {
      onEvent(JSON.parse(data) as TimelineEvent)
    } catch {
      // A malformed frame is dropped rather than killing the stream. Losing one
      // event is recoverable; losing the connection loses the rest of the session.
    }
  }

  const run = async () => {
    while (!closed) {
      try {
        const token = getStoredToken()
        const headers: Record<string, string> = {
          Accept: 'text/event-stream',
          Authorization: token ? `Bearer ${token}` : 'Bearer dev-local',
        }
        if (lastId) headers['Last-Event-ID'] = lastId

        const res = await fetch(`/api/agent/sessions/${sessionId}/events`, {
          headers,
          signal: controller.signal,
        })
        if (!res.ok || !res.body) {
          // 404 means the session is not the caller's (or does not exist).
          // Retrying forever would be a polling loop against a permanent answer.
          if (res.status === 404 || res.status === 401) {
            onError?.(
              res.status === 401
                ? 'not signed in'
                : 'this session is not available',
            )
            return
          }
          throw new Error(`status ${res.status}`)
        }

        backoffMs = 1_000
        const reader = res.body.getReader()
        const decoder = new TextDecoder()
        let buffer = ''
        for (;;) {
          const { done, value } = await reader.read()
          if (done) break
          buffer += decoder.decode(value, { stream: true })
          // SSE frames are separated by a blank line. The tail is kept: a chunk
          // boundary lands mid-frame often enough that dropping it loses events.
          let split = buffer.indexOf('\n\n')
          while (split !== -1) {
            parseFrame(buffer.slice(0, split))
            buffer = buffer.slice(split + 2)
            split = buffer.indexOf('\n\n')
          }
        }
      } catch (e) {
        if (closed || (e as Error).name === 'AbortError') return
        onError?.('the event stream dropped; reconnecting')
      }
      if (closed) return
      await new Promise((r) => setTimeout(r, backoffMs))
      backoffMs = Math.min(backoffMs * 2, 15_000)
    }
  }

  void run()
  return () => {
    closed = true
    controller.abort()
  }
}
