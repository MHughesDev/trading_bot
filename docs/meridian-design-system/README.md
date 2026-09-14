# Meridian Design System — Handoff Package

Read in this order:

1. **02-AGENT-BRIEF.md** — what to build, in what order, and what "done" means.
2. **01-DESIGN-SPEC.md** — the full specification. Everything else defers to it.
3. **tokens.css** — the token layer. Copy verbatim into `src/styles/`.

| File | What it is |
|---|---|
| `01-DESIGN-SPEC.md` | Full specification: foundations, components, screens, behaviour, a11y, implementation |
| `02-AGENT-BRIEF.md` | Execution brief for the coding agent: order of work, acceptance criteria |
| `03-CONTRAST-REPORT.txt` | Machine verification: 89 contrast pairs, 2 chart palettes, token parity, literal audit — all pass |
| `tokens.css` | The complete token layer, both themes. The only file allowed to contain colour values. |
| `tokens.ts` | Typed mirror of the token names + theme application helpers |
| `tailwind.theme.css` | Tailwind v4 `@theme` mapping (optional) |
| `dashboard-paper.png` / `-terminal.png` | Rendered reference — Dashboard, both themes |
| `trading-paper.png` / `-terminal.png` | Rendered reference — Trading terminal, both themes |
| `strategy-paper.png` / `-terminal.png` | Rendered reference — Strategy builder, both themes |
| `reference/*.html` | The live source of those renders. Open in a browser and set `data-theme="paper"` or `"terminal"` on `<html>` to switch. |

The mockups were rendered **from `tokens.css` itself** at 1680×1050 @2×, not drawn
separately — so every colour, radius, spacing value and type size in the images is
a real token you can look up.
