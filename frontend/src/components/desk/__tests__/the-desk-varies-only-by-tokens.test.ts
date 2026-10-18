/**
 * S-6.1 — "both themes and both densities render without reflow".
 *
 * Theme (`data-theme`) and density (`data-density`) are token swaps in
 * `tokens.css`: they change what `--bg-*`, `--fg-*` and `--s-*` resolve to and
 * nothing else. The desk renders the same boxes under both as long as its own
 * stylesheet never branches on either and never names a colour the tokens do
 * not own. That is what this holds — statically, because `vitest.config.ts`
 * wires no stylesheet into jsdom, so a rendered check would pass with any CSS.
 * It is not a screenshot in a real browser, and does not claim to be one.
 */
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const css = readFileSync(join(__dirname, '..', '..', '..', 'styles', 'desk.css'), 'utf8')

/** Comments out, and the print block out — paper is the one place black is right. */
function screenRules(src: string): string {
  const noComments = src.replace(/\/\*[\s\S]*?\*\//g, '')
  const at = noComments.indexOf('@media print')
  if (at === -1) return noComments
  // Drop the balanced block that starts there.
  let depth = 0
  let end = at
  for (let i = noComments.indexOf('{', at); i < noComments.length; i++) {
    if (noComments[i] === '{') depth++
    if (noComments[i] === '}') depth--
    if (depth === 0) {
      end = i + 1
      break
    }
  }
  return noComments.slice(0, at) + noComments.slice(end)
}

describe('the desk varies by tokens and by nothing else', () => {
  const rules = screenRules(css)

  it('the scan sees the stylesheet, so an empty read cannot pass', () => {
    expect(rules).toMatch(/\.desk-rail/)
    expect(css).toMatch(/@media print/)
  })

  it('no rule branches on the theme or the density', () => {
    expect(rules).not.toMatch(/\[data-theme/)
    expect(rules).not.toMatch(/\[data-density/)
  })

  it('no colour is named outside the tokens', () => {
    expect(rules.match(/#[0-9a-fA-F]{3,8}\b/g) ?? []).toEqual([])
    expect(rules.match(/\brgba?\(/g) ?? []).toEqual([])
    expect(rules.match(/\bhsla?\(/g) ?? []).toEqual([])
  })
})
