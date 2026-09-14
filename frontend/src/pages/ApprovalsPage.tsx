// The global approvals inbox (COMP-006 §2, §4).
//
// A separate route from the workspace because approvals are the one thing a human
// must not have to go looking for. A session blocked on `ask_user` is a session
// spending wall-clock and nothing else, and the person who can unblock it is usually
// not the person staring at that project's timeline.
import { ApprovalsInbox } from '@/components/workspace/ApprovalsInbox'

export function ApprovalsPage() {
  return (
    <>
      <div className="pagehead">
        <h1 className="h1">Approvals</h1>
        <div className="spacer" />
        <span className="lbl">Everything waiting on a human decision, across every project</span>
      </div>
      <div className="read-body">
        <div />
        <div className="read-col">
          <ApprovalsInbox />
        </div>
      </div>
    </>
  )
}
