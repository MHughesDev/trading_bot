/**
 * The comment thread renders in every state, including the empty one (S-8.7).
 *
 * The dossier's mandatory-section rule — a section with nothing in it says "none
 * found" rather than being omitted — exists because an absent heading and an
 * empty one look identical on the page and mean opposite things. The same
 * argument applies one level down: a reviewer who sees no comments must be able
 * to tell "nobody has commented" from "the comments did not load", and a
 * component that renders nothing in both cases has told them the first.
 */
import { describe, expect, it } from 'vitest'

import { commentPlaceholder } from '../Dossier'

describe('commentPlaceholder', () => {
  it('distinguishes loading from empty', () => {
    expect(commentPlaceholder(true, 0, false)).toContain('Loading')
    expect(commentPlaceholder(false, 0, false)).toContain('None yet')
    expect(commentPlaceholder(true, 0, false)).not.toEqual(commentPlaceholder(false, 0, false))
  })

  it('distinguishes a failed load from an empty thread', () => {
    const failed = commentPlaceholder(false, 0, true)
    expect(failed).toContain('not the same as none')
    expect(failed).not.toEqual(commentPlaceholder(false, 0, false))
  })

  it('renders the comments themselves once there are any', () => {
    // `null` is the signal to render the list. A placeholder here would hide
    // real comments behind a summary of them.
    expect(commentPlaceholder(false, 1, false)).toBeNull()
    expect(commentPlaceholder(false, 9, false)).toBeNull()
  })

  it('reports loading even when an earlier fetch left comments behind', () => {
    // React Query keeps the previous data while refetching. Saying "loading"
    // is the honest answer about what is on screen.
    expect(commentPlaceholder(true, 3, false)).toContain('Loading')
  })
})
