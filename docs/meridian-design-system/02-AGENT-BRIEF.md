# Agent Brief — UI Teardown and Rebuild

**Read this first, then `01-DESIGN-SPEC.md` in full before writing any code.**
This brief tells you what to do and in what order. The spec tells you what everything
must look like and how it must behave. Where they appear to disagree, the spec wins.

---

## 1. The job

Replace the entire user interface of the trading platform. Not a reskin — the information
architecture of the Dashboard, Trading and Strategy screens changes, and a light theme is
being introduced that does not currently exist.

Two themes ship together:

- **`paper`** (light) — warm, soft, serif-accented, generous. Calm reading instrument.
- **`terminal`** (dark) — blue-black, cyan, monospaced numerals, hairlines, glow. Precision instrument.

They share layout, spacing, anatomy, motion and behaviour. They share **no colour values,
no radii, no type faces, and no label casing.** The dark theme is not an inversion of the
light theme and must never be generated as one.

The routes stay as they are: `Dashboard · Trading · Automations · Strategy · Back Testing · Settings`.

---

## 2. What you are given

| File | Use it for |
|---|---|
| `tokens.css` | Copy verbatim to `src/styles/tokens.css`. This is the whole colour/shape/space system. Never edit a value without re-running the two validators. |
| `tokens.ts` | Import these types so a typo in a token name is a compile error. |
| `tailwind.theme.css` | Drop in if you use Tailwind v4. |
| `01-DESIGN-SPEC.md` | Everything: foundations, every component, every screen, every behaviour. |
| `03-CONTRAST-REPORT.txt` | Evidence that all 89 fg/bg pairs pass. Re-run on any token change. |
| `dashboard-paper.png`, `dashboard-terminal.png` | Reference render of §4.1 |
| `trading-paper.png`, `trading-terminal.png` | Reference render of §4.2 |
| `strategy-paper.png`, `strategy-terminal.png` | Reference render of §4.3 |

The mockups were rendered from `tokens.css` at 1680×1050 @2×. They are accurate to the token
values but they are reference, not law — the spec's rules govern edge cases the images do not show.

---

## 3. Non-negotiables

If you do nothing else right, do these.

1. **No colour literal outside `tokens.css`.** Add the stylelint rule in step 0 so this can
   never regress.
2. **Components consume semantic tokens** (`--bg-*`, `--fg-*`, `--line-*`, `--chart-*`,
   `--node-*`), **never primitives** (`--p-*`).
3. **Theme switches by one attribute** on `<html>`, resolved before first paint. No JS colour
   objects, no per-component theme branching, no re-render.
4. **Every changing number is tabular** and goes through `src/lib/format.ts`. Nothing formats
   numbers locally.
5. **Zero layout difference between themes.** Switching theme must change paint only.
6. **Direction and state never rely on colour alone** — always a sign, glyph, icon or word.
7. **No market data animates its position.** Tick flash only (spec §5.1).
8. **`--bg-accent` never touches buy/sell.** Buy is `--bg-pos`, sell is `--bg-neg`.
9. **Every interactive element has a visible focus ring.**
10. **Every screen ships its empty, loading, partial, error and stale states.**

---

## 4. Order of work

Each step is independently shippable. Do not skip ahead; step 2 in particular is what makes
everything after it cheap.

### Step 0 — Guardrails (half a day)
- Add `tokens.css`, wire `data-theme` / `data-density` on `<html>` with a pre-paint inline script.
- Add the theme toggle (`⌘\`) and density toggle (`⌘.`) to Settings and the account menu.
- Add the stylelint "no colour literal" rule and the eslint inline-style rule. Let them fail loudly.
- Wire the contrast script and the chart-palette validator into CI.
- **Nothing looks different yet. That is correct.**

### Step 1 — De-hardcode (the tedious one; do it completely)
Sweep every existing component and replace every colour literal with the right semantic token.
Resist the urge to redesign while you are in here. When this step is done the app should look
roughly as it does today in dark mode and become newly usable in light mode.

### Step 2 — Typography and numbers
- Self-host Inter, Source Serif 4, Space Grotesk, JetBrains Mono as woff2 (spec §2.2.1).
- Apply the role table (§2.2.3) and the four `--label-*` tokens (§2.2.4).
- Write `src/lib/format.ts` (§5.3) and route **every** number through it. Instrument precision
  comes from instrument metadata, not from the call site.

### Step 3 — Primitives
Rebuild against spec §3, in this order:
`Panel → Button → IconButton → Badge/Chip → Input/NumberField → Segmented → Tabs →
Select → Toggle → Table → StatTile → Tooltip → Popover → Modal → Toast → Skeleton → EmptyState`.
Each lands in Storybook with **all eight states** in **both themes** before it is used anywhere.

### Step 4 — App shell
Top bar (§3.19) with brand, nav, environment pill, equity and day-P&L readouts, connection
indicator, account. Page headers. Command palette (`⌘K`) and shortcut sheet (`⌘/`).

### Step 5 — Dashboard (§4.1) — biggest visible win
Delete the eight per-asset-class columns entirely. Build: hero strip → equity curve +
allocation → tabbed portfolio detail + automations + risk. The totals row must reconcile to
the hero equity figure.

### Step 6 — Trading (§4.2)
Instrument bar, single primary chart by default with explicit layout modes for 2 / 2×2 / 3+1,
bottom dock with Positions / Open orders / Fills / Automations, full-height order ticket,
order book under the watchlist.

### Step 7 — Strategy (§4.3)
Node taxonomy (§2.1.8) with 3px family rails — remove the saturated headers. Grouped,
searchable block palette. Right-hand inspector. Floating validation console replacing the
"Tips" box. Left→right auto-layout so no edge flows backward. Continuous validation.

### Step 8 — Automations, Back Testing, Settings (§4.4)
No new patterns. Compose from what exists.

### Step 9 — States pass
Walk every screen through empty / filtered-empty / loading / partial / error / stale /
disconnected. Build what is missing (§3.22).

### Step 10 — Accessibility and polish
Spec §6 end to end, both themes, keyboard-only, CVD simulation, 200% zoom, 1280px width.

---

## 5. Per-screen acceptance

### Dashboard
- [ ] No per-asset-class column layout remains anywhere in the codebase.
- [ ] Hero equity figure = Σ of the asset-class table's Equity column, to the cent.
- [ ] Day P&L figure = Σ of the Day P&L column, to the cent.
- [ ] Allocation legend and the stacked bar read from one array in one fixed slot order.
- [ ] Portfolio detail tabs all work: By asset class / Open positions / Recent fills / Orders.
- [ ] Equity curve has a crosshair tooltip, a range selector above the plot, and a table view.
- [ ] Nothing is clipped at 1280px; the right column stacks below at 1024px.

### Trading
- [ ] Default view is **one** chart. Multi-chart is a named layout mode.
- [ ] The focused pane in a multi-chart layout is visibly marked.
- [ ] Order ticket recomputes value / fee / margin / liquidation / buying-power on every keystroke.
- [ ] Submit button label restates side, size and notional.
- [ ] Submit is disabled with an inline, field-adjacent reason whenever the ticket is invalid.
- [ ] Bracket block shows risk in **both** currency and % of equity, plus an R:R badge.
- [ ] Clicking an order-book level writes that price into the ticket.
- [ ] Positions `Source` badge links to the automation that opened the position.
- [ ] Paper mode shows the reminder line; live mode changes the ticket border and button prefix.

### Strategy
- [ ] Seven node families, each with the correct token; `logic` is neutral grey.
- [ ] Family colour appears as a 3px rail + icon + label, never as a filled header.
- [ ] Block palette is grouped by family, has a search field, and shows family dots.
- [ ] No "Tips" box on the canvas; a collapsible validation console is docked bottom-right.
- [ ] Auto-layout guarantees every edge flows left→right across the six stages.
- [ ] Inspector shows properties, connections, backtest preview and risk guardrails.
- [ ] Validation counts in the strategy bar update continuously as the graph changes.
- [ ] `Deploy` is disabled while any error exists; deploy-to-live is a destructive confirm.
- [ ] Below 1024px the canvas is read-only with an explanatory notice.

---

## 6. Definition of done (every screen)

1. No colour literal, no off-scale spacing value, no unlisted font stack.
2. Correct in all four combinations: `paper`/comfortable, `paper`/compact,
   `terminal`/comfortable, `terminal`/compact — with identical layout across themes.
3. All eight interaction states on every interactive element, focus ring always visible.
4. Every number tabular, instrument-precise, formatted through `format.ts`.
5. Empty / loading / partial / error / stale states exist and are reachable in Storybook.
6. Keyboard-only completes the screen's primary task.
7. `axe-core`: zero violations, both themes.
8. No horizontal scroll at 1280px; nothing clipped at 200% zoom.
9. Every chart satisfies spec §2.8.
10. Both-theme screenshots attached to the PR.

---

## 7. Where people usually get this wrong

- **Generating the dark palette from the light one.** The two palettes are different
  temperatures. Inverting produces a muddy warm-grey dark theme that looks like a bug.
- **Letting a chart library bring its own colours.** Every chart colour is a `--chart-*` token.
  If the library cannot be told, wrap it or replace it.
- **Formatting numbers at the call site.** It seems harmless until the same price shows 2dp
  in one place and 5dp in another on the same screen.
- **Animating prices.** It feels premium for a day and is exhausting by week two.
- **Rebuilding the eight-column dashboard "but nicer".** The column layout is the defect.
  The table replaces it.
- **Flooding strategy node headers with family colour** because the old UI did. Twenty
  saturated headers on one canvas is why the old canvas is hard to read.
- **Shipping a component without its empty and error states** and discovering them in production.
- **Treating the environment pill as decoration.** It is the thing standing between a user and
  an accidental live order.

---

## 8. Questions to resolve with the product owner before Step 5

These affect data shape, not visual design, and should not block Steps 0–4.

1. Does the account model support multiple accounts, or is "All accounts" cosmetic?
   (Changes the top bar and the dashboard header.)
2. Are prediction markets and DEX/AMM positions expressible in the same position schema as
   the other six classes? (Changes whether the Open positions tab is one table or several.)
3. Is there a live-trading tier yet, or is the product paper-only for now?
   (Changes how much of §5.4 and §5.5 ships in v1.)
4. Should win rate be per-class as shown, or account-level only?
5. What is the real reconnect behaviour of the market-data feed — does it replay missed
   ticks or snapshot? (Changes the stale/reconnect UX in §5.2.)
