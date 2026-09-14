# Meridian Design System — Full Specification

**Status:** Approved for implementation · **Version:** 1.0 · **Date:** 12 September 2026
**Scope:** Complete visual and interaction specification for a multi-asset trading platform
(equities, ETFs, FX, futures/perpetuals, crypto spot, options, DEX/AMM, prediction markets).
**Intended reader:** the coding agent performing the full UI teardown and rebuild.

---

## 0. How to use this document

### 0.1 File manifest

| File | What it is | Authority |
|---|---|---|
| `01-DESIGN-SPEC.md` | This document. The single source of truth for everything visual and behavioural. | Normative |
| `02-AGENT-BRIEF.md` | Execution order, migration plan, definition of done, QA checklist. | Normative |
| `tokens.css` | The complete token layer, both themes. Copy verbatim into the codebase. | Normative — **do not retype values from this doc; import the file** |
| `tokens.ts` | Typed TS mirror of the semantic token names, for authoring safety. | Normative |
| `tailwind.theme.css` | Tailwind v4 `@theme` mapping over `tokens.css`. | Normative if Tailwind is used |
| `*-paper.png` / `*-terminal.png` | Rendered reference mockups of the three primary screens in both themes. | Illustrative — pixel reference, not pixel law |

The mockups were produced **from `tokens.css` itself**, not drawn separately. Anything in the
images that disagrees with this document is a rendering artefact; the document wins.

### 0.2 Normative language

**MUST** / **MUST NOT** are hard requirements — a build that violates one is not done.
**SHOULD** is a strong default; deviating requires a written reason in the PR.
**MAY** is genuine latitude.

---

## 1. Design intent

### 1.1 The problem with the current UI

The rebuild is not a reskin. These are the concrete defects the new system is designed to fix,
drawn from the existing Dashboard, Trading and Strategy screens:

| # | Defect | Where | Fix in this spec |
|---|---|---|---|
| D1 | Eight near-empty per-asset-class columns; ~75% of the dashboard viewport is dead space; the eighth column is clipped off-screen. | Dashboard | §4.1 — one dense table replaces eight columns; hero strip carries the headline numbers. |
| D2 | Nav pills float alone in a wide empty bar; no account context, no equity readout, no connection state, no user. | All screens | §3.19 — top bar with brand, nav, environment pill, equity/P&L readout, latency, account. |
| D3 | Four tiny charts compete; none is readable; no focal instrument. | Trading | §4.2 — one primary chart plus a tabbed dock; multi-chart is an explicit layout mode, not the default. |
| D4 | Strategy nodes use eight arbitrary saturated hues with no semantic meaning (purple indicator, orange condition, green action, teal size, red exit). | Strategy | §2.1.8 — a seven-family node taxonomy where colour *means* something, carried as a 3px rail rather than a saturated header block. |
| D5 | Type is a mix of mono and sans with no scale; labels and values compete. | All screens | §2.2 — one scale, explicit roles, tabular numerals everywhere a number can change. |
| D6 | No light mode at all. | All screens | §1.2 — two complete, separately designed themes. |
| D7 | Numbers are not tabular; columns jitter as prices tick. | All screens | §2.2.5 — `tnum` mandatory on every changing figure. |
| D8 | "PAPER" badge is the only safety affordance and it reads as decoration. | All screens | §5.4 — environment is a first-class, always-visible, colour-and-text state. |

### 1.2 Two themes, two personalities

This product ships **two deliberately different themes**, not one theme and its inversion.
Generating the dark palette by flipping the light one is explicitly forbidden (§2.1.2).

**`paper` — the light theme. "Soft entropic."**
Warm ivory ground, generous whitespace, a single clay accent, a transitional serif for
display type, soft directional shadows, large radii. It should feel like a well-set document
that happens to contain live markets — calm, literate, unhurried. Reading it should lower the
user's heart rate. This is the theme for planning, reviewing, reading research, building
strategies, and for anyone who looks at the screen for eight hours.

**`terminal` — the dark theme. "Instrument-grade."**
Blue-black ground, cyan brand light, hairline rules, tight radii, uppercase tracked-out HUD
labels, monospaced numerals, glow instead of shadow. It should feel like a precision instrument
under low light — cold, fast, dense, slightly electric. This is the theme for execution,
for the night session, and for the multi-monitor desk.

**What is shared between them (MUST be identical):**
the information architecture, the layout grid, the spacing scale, the type *scale*,
component anatomy and geometry, control heights, motion durations and easings,
interaction behaviour, keyboard map, copy, and every semantic token *name*.

**What differs between them (MUST be theme-scoped):**
colour values, radii, type *faces*, label casing and tracking, the elevation model,
texture/background treatment, and the chart palettes.

A user MUST be able to switch themes mid-session without any layout reflow — only paint changes.

### 1.3 Principles

1. **The number is the product.** Chrome recedes; figures dominate. If a decorative element
   competes with a price for attention, the decoration is wrong.
2. **Colour carries meaning or it doesn't appear.** Every hue in the interface is either
   brand, direction (up/down), state (ok/warn/error/info), or a chart series identity.
   There is no decorative colour.
3. **Never colour alone.** Direction and state MUST always carry a second cue — a sign,
   an arrow glyph, an icon, or a text label. ~8% of male users cannot rely on red/green.
4. **Calm by default, loud on exception.** Saturation is a budget. A screen at rest should
   be almost monochrome; a filled order, a breached limit or a stopped automation earns colour.
5. **Density is a user setting, not a taste.** Both themes support both densities (§2.3.4).
6. **Destructive and irreversible actions look different and take one more step.** (§5.5)
7. **Every state is designed.** Loading, empty, partial, error, stale, reconnecting,
   halted, and after-hours are not afterthoughts; see §3.22 and §5.2.

---

## 2. Foundations

### 2.1 Colour

#### 2.1.1 Three-tier architecture

```
Tier 1  PRIMITIVES   --p-<ramp>-<step>     raw values, theme-independent, both palettes present
Tier 2  SEMANTIC     --bg-*, --fg-*, ...   remapped per theme; THIS is what components consume
Tier 3  COMPONENT    --btn-*, --node-*     only where a component needs its own contract
```

**Rules:**
- R1. A raw colour value (`#…`, `rgb()`, `hsl()`, `oklch()`) MUST NOT appear anywhere in the
  codebase outside `tokens.css`. This is enforced in CI (§8.6).
- R2. Components MUST NOT reference `--p-*` primitives. Only semantic tokens.
- R3. Any new colour need creates a **new semantic token**, not a one-off value.
- R4. Opacity MUST NOT be used to create a new colour on an opaque surface. Use the
  `-subtle` token that already exists, or add one.

#### 2.1.2 Why the dark theme is not an inversion

`paper` is built on a warm neutral ramp (hue ≈ 60°, chroma ≈ 0.01) and a clay accent
(hue ≈ 40°). `terminal` is built on a cool neutral ramp (hue ≈ 250°, chroma ≈ 0.02) and a
cyan accent (hue ≈ 200°). Inverting one produces the other's *lightness* but not its
*temperature*, and temperature is the entire personality difference. Each theme's palette
was designed and validated independently against its own surfaces.

#### 2.1.3 Semantic surface tokens

| Token | Role | `paper` | `terminal` |
|---|---|---|---|
| `--bg-canvas` | The page behind everything | `#FAF9F5` | `#06090E` |
| `--bg-surface` | Default panel / card | `#FFFFFF` | `#0B1017` |
| `--bg-surface-raised` | Panel that sits above another panel | `#FFFFFF` + shadow-2 | `#111925` |
| `--bg-sunken` | Wells, chart grounds, segmented tracks, code | `#F4F2EB` | `#080D13` |
| `--bg-inset` | Inputs on dark, deepest recess | `#EEEBE1` | `#0E151E` |
| `--bg-hover` | Row/menu hover (translucent, composites) | `rgba(23,22,19,.045)` | `rgba(147,196,255,.055)` |
| `--bg-active` | Pressed | `rgba(23,22,19,.075)` | `rgba(147,196,255,.10)` |
| `--bg-selected` | Selected row / active watchlist item | `#FBF3EF` | `rgba(22,196,222,.13)` |
| `--bg-overlay` | Modal, popover, menu, floating console | `#FFFFFF` | `#111925` |
| `--bg-scrim` | Behind modals | `rgba(30,26,20,.32)` | `rgba(2,5,9,.72)` |
| `--bg-accent` | Primary button fill | `#B85B39` | `#16C4DE` |
| `--bg-pos` / `--bg-neg` | Buy / sell button fills | `#2D7751` / `#B0382E` | `#12C286` / `#FF5470` |
| `--bg-*-subtle` | Tinted badge/callout grounds | see `tokens.css` | see `tokens.css` |

`--bg-hover` and `--bg-active` are **translucent by design** so they compose correctly over
any surface. Do not replace them with opaque values.

#### 2.1.4 Semantic foreground tokens

| Token | Role | `paper` | `terminal` | Min contrast achieved |
|---|---|---|---|---|
| `--fg-primary` | Values, headings, body | `#171613` | `#E4EBF3` | 12.9 : 1 |
| `--fg-secondary` | Supporting text, column headers | `#57534A` | `#93A4B8` | 6.1 : 1 |
| `--fg-tertiary` | Micro-labels, units, timestamps | `#6E695E` | `#758497` | 4.5 : 1 |
| `--fg-disabled` | Disabled text (non-informational only) | `#AEA89A` | `#47566A` | 2.3 : 1 * |
| `--fg-accent` | Links, accent text | `#A64E2F` | `#3FD9F0` | 5.1 : 1 |
| `--fg-pos` | Gains, long, bid, filled-buy | `#2D7751` | `#2BE3A2` | 4.6 : 1 |
| `--fg-neg` | Losses, short, ask, filled-sell | `#B0382E` | `#FF5470` | 5.0 : 1 |
| `--fg-warn` | Caution, paper mode, paused | `#886314` | `#FFB13D` | 4.6 : 1 |
| `--fg-info` | Neutral informational | `#2F6B9E` | `#58B6FF` | 4.9 : 1 |
| `--fg-on-accent` | Text on `--bg-accent` | `#FFFFFF` | `#04161C` | 4.6 / 8.8 : 1 |
| `--fg-on-pos` / `--fg-on-neg` | Text on buy/sell fills | `#FFFFFF` | `#032118` / `#2B0008` | ≥ 4.5 : 1 |

\* `--fg-disabled` is exempt from 4.5:1 per WCAG 1.4.3 (inactive controls), but a disabled
control MUST NOT be the only place a piece of information exists.

**All 89 foreground/background pairs in the system were verified programmatically;
0 failures.** The verification script and its output are in `03-CONTRAST-REPORT.txt`.
Re-run it in CI whenever a colour token changes.

#### 2.1.5 Financial semantics — the rules

- **Positive / long / bid / buy → `--fg-pos` family. Negative / short / ask / sell →
  `--fg-neg` family.** No exceptions, no per-market inversion in v1. (If you later ship an
  Asia-market "red = up" preference, it MUST be a single token remap, not a per-component
  branch — which this architecture already supports.)
- **A P&L figure MUST carry an explicit sign** (`+` or `−`, U+2212 minus, not a hyphen)
  **in addition to colour.** A directional change MUST additionally carry `▲`/`▼` when it
  appears without a sign.
- **Zero is neutral.** `0.00` renders in `--fg-secondary` with no sign and no colour.
- **`--fg-pos` / `--fg-neg` are reserved.** They MUST NOT be reused as chart series colours,
  as a category hue, or as decoration.

#### 2.1.6 The clay / sell-red separation (important)

In `paper`, the brand accent is clay (`#B85B39`) — a warm orange-red. The sell colour is
ember (`#B0382E`) — a cooler crimson. They are close in hue and MUST be kept apart:

- Clay is used **only** for: brand mark, active nav, focus rings, links, selection tint,
  the non-financial primary button (`Deploy`, `New order`, `Save`).
- Clay MUST NOT appear on, adjacent to, or inside any element that expresses direction —
  no clay P&L text, no clay candles, no clay in the order ticket's side toggle.
- The Buy and Sell buttons use `--bg-pos` / `--bg-neg`, never `--bg-accent`.
- In `terminal` this tension does not exist (cyan vs rose), but the same rule is applied for
  consistency.

#### 2.1.7 Asset-class colour policy

There are eight asset classes and that number will grow. Therefore:

- Asset class **identity in chrome** (cards, rows, chips, headers) MUST be expressed by a
  **neutral chip + label**, never by a unique hue. This scales to a ninth class for free.
- Asset class **identity in data visualisation** (allocation bars, multi-series charts,
  legends) uses the **Instrument scale** (§2.8.3), assigned in a **fixed order that never
  changes** — slot 1 is Crypto Spot forever, whether or not Crypto Spot is on screen.
- The same class MUST map to the same slot on every chart in the product.

Fixed assignment:

| Slot | Asset class | `paper` | `terminal` |
|---|---|---|---|
| 1 | Crypto Spot | `#3A83C2` sky | `#009FA7` teal |
| 2 | Equities | `#8F5E00` bronze | `#A08200` brass |
| 3 | ETF | `#008F95` cactus | `#00B074` jade |
| 4 | FX | `#863864` mulberry | `#C55CA6` orchid |
| 5 | Perpetuals / futures | `#6C862C` olive | `#8D82F1` periwinkle |
| 6 | Options | `#664D99` heather | `#E36D3B` ember |
| 7 | DEX / AMM | `#B46239` clay-tan | `#1190E4` azure |
| 8 | Prediction | `#46549E` indigo | `#689A18` lime |

A ninth class does **not** get a generated hue. It folds into "Other", or the chart becomes
small multiples. (§2.8.3)

#### 2.1.8 Strategy node taxonomy

Replaces the current arbitrary rainbow. Seven families; colour encodes *what kind of thing
this block is*, which is the only property worth colour-coding on a canvas.

| Family | Token | Contains | `paper` | `terminal` |
|---|---|---|---|---|
| `data` | `--node-data` | Price series, volume, order book, funding | indigo | azure |
| `indicator` | `--node-indicator` | EMA, SMA, RSI, ATR, MACD, Bollinger | sky | teal |
| `signal` | `--node-signal` | Crosses above/below, >, <, is rising/falling | bronze | brass |
| `logic` | `--node-logic` | AND, OR, NOT, If/Then | neutral grey | neutral grey |
| `ai` | `--node-ai` | AI Forecast, regime detector, sentiment | heather | periwinkle |
| `intent` | `--node-intent` | Buy/Sell, position size, scale-in | kelp | mint |
| `risk` | `--node-risk` | Stop loss, take profit, trailing, daily loss cap | ember | rose |

**Presentation rule:** the family colour appears as a **3px left rail + icon + uppercase family
label** on the node header. It MUST NOT flood the whole header. A canvas of twenty nodes with
saturated headers is unreadable; a canvas of twenty nodes with rails is scannable.

`logic` is deliberately neutral: gates are plumbing, not meaning.

---

### 2.2 Typography

#### 2.2.1 Faces

| Role token | `paper` | `terminal` | Purpose |
|---|---|---|---|
| `--font-display` | **Source Serif 4** | **Space Grotesk** | Page titles, hero equity figure, brand |
| `--font-ui` | **Inter** | **Inter** | Everything else |
| `--font-mono` | **JetBrains Mono** | **JetBrains Mono** | IDs, code, timestamps, raw payloads |
| `--font-numeric` | **Inter** (tabular) | **JetBrains Mono** | Every number that can change |

All four are open-licensed (SIL OFL). Self-host as `woff2`, `font-display: swap`, subset to
`latin` + the arrows/math glyphs in §2.2.6. Do **not** load them from a third-party CDN —
a font request that blocks first paint on a trading screen is a latency bug.

`--font-numeric` differing between themes is intentional and is a large part of why the two
themes feel unrelated: proportional-but-tabular Inter reads as typeset prose; JetBrains Mono
reads as instrumentation.

#### 2.2.2 Scale

Fixed pixel scale. **MUST NOT** be rem-scaled with a root font-size multiplier — data density
must not drift when a user changes browser font size; use the density modifier instead.

`10 · 11 · 12 · 13 · 14 · 16 · 18 · 20 · 24 · 28 · 32 · 40 · 48 · 60`

Base UI size is **13px**. Table and dense-panel text is **12px**. Micro-labels are
**12px** in `paper` and **10px uppercase** in `terminal`.

#### 2.2.3 Roles

| Role | Size / weight / tracking | Face | Notes |
|---|---|---|---|
| Hero figure | 32 / 600 / −0.015em | display | Account equity only |
| Page title (`h1`) | 20 / 600 / −0.015em | display | `terminal`: 16, uppercase, +0.12em |
| Panel title | 12 / 600 | ui | `terminal`: 10, uppercase, +0.1em, tertiary |
| Section label | `--label-*` tokens | ui | See §2.2.4 |
| Body | 13 / 400 / 1.45 | ui | |
| Table cell | 12 / 400 | ui / numeric | |
| Table header | `--label-*` | ui | |
| Instrument ticker | 13 / 600 | ui | Never abbreviated |
| Price (primary) | 20 / 600 | numeric | |
| Stat tile value | 20 / 600 | numeric | |
| Node title | 12 / 600 | ui | |
| Badge | 10 / 600 / +0.02em, uppercase | ui | Both themes |
| Code / ID | 11–12 / 400 | mono | |

#### 2.2.4 The micro-label contract

Micro-labels ("Day P&L", "24h vol", "Limit price") are the highest-frequency text in the
product and are the clearest tell between the two themes. They are driven by four tokens so
that one rule change propagates everywhere:

```css
/* paper */                         /* terminal */
--label-size: 12px;                 --label-size: 10px;
--label-weight: 500;                --label-weight: 500;
--label-transform: none;            --label-transform: uppercase;
--label-tracking: 0;                --label-tracking: 0.1em;
```

Every label MUST consume all four. Hand-written `text-transform: uppercase` on a label is a bug.

#### 2.2.5 Numerals — mandatory rules

- N1. **Every number that can change MUST use `--font-numeric` with
  `font-variant-numeric: tabular-nums`.** Non-negotiable: proportional digits make columns
  jitter on every tick, which is physically tiring and makes misreads likely.
- N2. Decimal places are **fixed per instrument**, from instrument metadata, never
  "as many as needed". BTC-USD is always 2dp; EURUSD is always 4dp (5dp for pip display);
  a 400-share NVDA position is `400`, not `400.0`.
- N3. Thousands separators on every figure ≥ 1000. Locale-aware.
- N4. Large aggregate figures MAY abbreviate (`$1.24M`, `18.4k BTC`) **only** outside the
  execution path. Order tickets, fills, positions and balances MUST show full precision.
- N5. Percentages: 2dp for returns, 2dp for allocation, 0dp for win rate.
- N6. Minus sign is **U+2212 (−)**, not a hyphen. Plus is a literal `+`.
- N7. Right-align every numeric column. Left-align the first (identity) column. Nothing else.
- N8. A number that is stale (§5.2) MUST render at 60% opacity with a stale indicator —
  it MUST NOT be hidden or zeroed.

#### 2.2.6 Glyphs

`▲ ▼ · − + × ≥ ≤ →` are part of the type system. `▲`/`▼` accompany directional percentages
where no sign is shown. `·` is the standard separator in meta lines. `×` is the multiplication
sign in "ATR ×1.5" and "3×" leverage, never the letter x.

---

### 2.3 Space, sizing and layout

#### 2.3.1 Space scale

4px base: `0 · 1 · 4 · 8 · 12 · 16 · 20 · 24 · 28 · 32 · 40 · 48 · 56 · 64 · 80 · 96 · 128`
(`--s-px` through `--s-32`). Every margin, padding and gap MUST come from this scale.

#### 2.3.2 Control heights

| Token | Comfortable | Compact | Used for |
|---|---|---|---|
| `--ctl-h-xs` | 24 | 20 | Inline chips, tag buttons |
| `--ctl-h-sm` | 30 | 26 | Toolbar buttons, segmented, search |
| `--ctl-h-md` | 36 | 30 | Default button, input |
| `--ctl-h-lg` | 44 | 36 | Emphasis input |
| `--ctl-h-xl` | 52 | 44 | Order submit button |
| `--row-h` | 40 | 28 | Table row |

#### 2.3.3 Layout rails

| Token | Value | Meaning |
|---|---|---|
| `--nav-h` | 56 / 48 | Global top bar |
| `--subnav-h` | 44 / 36 | Instrument bar, page header |
| `--panel-w-sm` | 280 | Watchlist, list rails |
| `--panel-w-md` | 340 | Order ticket, inspector |
| `--panel-w-lg` | 400 | Wide inspector, allocation |
| `--content-max` | 1680 | Max content width for reading-oriented pages only; trading screens are full-bleed |

#### 2.3.4 Density

`data-density="comfortable" | "compact"` on any container. It remaps control heights, row
height, panel padding and nav heights — **never type sizes, never colour**.

- `paper` defaults to **comfortable**. `terminal` defaults to **compact**.
- This is a default, not a lock. The user MUST be able to set density independently of theme
  in Settings, and the setting persists per device.
- A single panel MAY opt into compact inside a comfortable page (e.g. a dense fills table).

#### 2.3.5 Panel grid

All screens are built from a gap-`--s-3` (12px) grid of `.panel` elements inside a
`--s-5` (20px) page gutter. Panels MUST NOT nest more than two deep. A panel is:

```
┌ panel ───────────────────────────────┐
│ panel-hd  38px, hairline bottom      │   title · badges · spacer · meta
├──────────────────────────────────────┤
│ panel-bd  padding --panel-pad        │   or .flush for tables/charts
└──────────────────────────────────────┘
```

Tables and charts MUST use `.flush` (zero body padding) and bleed to the panel edge.

---

### 2.4 Shape

| Token | `paper` | `terminal` | Applied to |
|---|---|---|---|
| `--r-xs` | 6 | 2 | Badges, chips, swatches, node field pills |
| `--r-sm` | 8 | 3 | Inputs, small buttons, segmented, list rows |
| `--r-md` | 12 | 5 | Buttons, nodes, menus, toasts |
| `--r-lg` | 16 | 8 | Panels, cards |
| `--r-xl` | 22 | 12 | Modals, large sheets |
| `--r-2xl` | 28 | 16 | Marketing / empty-state illustration frames |
| `--r-pill` | 999 | 999 | Nav pills (`paper` only), progress bars |
| `--r-circle` | 50% | 50% | Avatars (`paper`), status dots, graph ports (`paper`) |

`terminal` additionally squares off things `paper` rounds: avatars become `--r-sm`, nav pills
become `--r-md`, graph ports become 2px squares. This is one of the strongest theme tells and
is already encoded in `tokens.css` — do not "unify" it.

---

### 2.5 Elevation

The two themes use **different physics**. This is deliberate and MUST be preserved.

**`paper` — light and shadow.** A warm, directional, low-alpha shadow (`rgba(72,60,42,…)`,
never neutral black). Four levels:

| Token | Use |
|---|---|
| `--shadow-1` | Resting panel, segmented active thumb |
| `--shadow-2` | Raised panel, hovered card, toolbar button |
| `--shadow-3` | Popover, menu, floating console, selected node |
| `--shadow-4` | Modal |

**`terminal` — luminance and glow.** Shadows are nearly invisible on near-black; depth comes
from surface lightness stepping up (`#06090E → #0B1017 → #111925 → #16202C`), a 1px inner
top highlight (`--ring-inset`), and glow on emphasis:

| Token | Use |
|---|---|
| `--glow-accent` | Primary button, brand mark, selected node, focused input |
| `--glow-pos` / `--glow-neg` | Buy / sell submit buttons only |
| `--shadow-3` / `--shadow-4` | Floating surfaces, to separate from the canvas |

**MUST NOT:** apply `paper`'s drop shadows in `terminal`, or glow in `paper`. Both look cheap
in the wrong theme. The tokens already resolve correctly — just use `--shadow-*` and
`--glow-*` and let the theme decide.

---

### 2.6 Motion

| Token | Value | Use |
|---|---|---|
| `--dur-1` | 80ms | Hover, press, colour change |
| `--dur-2` | 140ms | Toggle, tab switch, tooltip |
| `--dur-3` | 220ms | Popover, dropdown, panel expand |
| `--dur-4` | 320ms | Modal, drawer, route transition |
| `--dur-5` | 480ms | Onboarding, celebratory (rare) |
| `--ease-out` | `cubic-bezier(.22,.61,.36,1)` | Default — things entering |
| `--ease-in-out` | `cubic-bezier(.65,.05,.36,1)` | Things moving between two states |
| `--ease-spring` | `cubic-bezier(.34,1.56,.64,1)` | Toggles, the segmented thumb — **only** |

**Rules:**
- M1. **Market data MUST NOT animate its position.** No sliding rows, no animated number
  counters, no easing on a price change. A price is either the old one or the new one.
- M2. The only permitted data animation is the **tick flash** (§5.1): a 1-frame background
  tint that decays over `--dur-3`. It MUST be disableable in Settings.
- M3. Charts MUST NOT animate on data append. They MAY animate on range change (`--dur-3`).
- M4. `prefers-reduced-motion: reduce` sets every duration to 0 (already in `tokens.css`).
  Tick flash becomes a static 1px left border for `--dur-3` worth of real time instead.
- M5. Nothing in the execution path (order ticket → confirm → fill) may be gated on an
  animation completing.

---

### 2.7 Iconography

- 16×16 grid, **1.5px stroke**, round caps and joins, no fills except status dots.
- Sizes: `--icon-sm` 14 · `--icon-md` 16 · `--icon-lg` 20 · `--icon-xl` 24.
- Icons inherit `currentColor`. An icon MUST NOT carry its own colour except a family colour
  inside a strategy node header.
- Icon-only controls MUST have `aria-label` and a tooltip (§3.16).
- Ship as a single sprite or as React components from one source; do not mix icon libraries.
- Stroke width does **not** change between themes. (Everything else does; this shouldn't.)

---

### 2.8 Data visualisation

#### 2.8.1 Chart tokens

| Token | Meaning |
|---|---|
| `--chart-surface` | Ground the plot sits on |
| `--chart-grid` / `--chart-grid-major` | Gridlines; major = axis-aligned emphasis |
| `--chart-axis` | Axis labels |
| `--chart-crosshair` | Crosshair line |
| `--chart-up` / `--chart-down` | Candle bodies and wicks |
| `--chart-up-line` / `--chart-down-line` | Candle outlines / high-contrast variants |
| `--chart-volume` | Volume bars (neutral — see rule V2) |
| `--chart-last` | Last-price line |
| `--chart-tag-bg` / `--chart-tag-fg` | Last-price tag fill and its text |
| `--chart-ma-fast` / `--chart-ma-slow` | The two default moving averages |
| `--chart-equity` | The equity-curve series |
| `--chart-area-from` / `--chart-area-to` | Area gradient stops |
| `--chart-1` … `--chart-8` | Categorical Instrument scale |
| `--chart-seq-1` … `-5` | Sequential (magnitude) ramp, one hue |
| `--chart-div-n2` … `-p2` | Diverging (polarity) ramp, two hues + neutral midpoint |

#### 2.8.2 Candlestick rules

- V1. Body 62% of slot width, wick 1.15px, 0.8px corner radius in `paper`, 0 in `terminal`.
  A doji renders a minimum 1.2px body so it never disappears.
- V2. **Volume bars are neutral** (`--chart-volume`), not up/down coloured. Colouring volume
  doubles the red/green noise for no information gain.
- V3. The last-price line is dashed `3 3` with a solid tag at the axis. Tag fill is
  `--chart-tag-bg`, text `--chart-tag-fg` — never white-on-mid-green (that pair fails contrast).
- V4. Position annotations: entry line = `--chart-ma-fast` dashed `5 4` with a label;
  stop = `--chart-down` dashed; target = `--chart-up` dashed. All three MUST be labelled with
  the price, not just drawn.
- V5. Gridlines are recessive: `--chart-grid` at ~7% alpha in `terminal`, a hairline ivory in
  `paper`. Horizontal gridlines only by default; vertical only at major time boundaries.
- V6. **Never a dual-axis chart.** Two measures of different scale → two stacked panes with a
  shared x-axis, or index both to a common base.

#### 2.8.3 Categorical palette — validated, fixed, not cycled

Both Instrument scales were generated and machine-validated against the six computable
data-viz checks (OKLCH lightness band, chroma floor, protan/deutan ΔE separation on adjacent
pairs, normal-vision ΔE floor, and WCAG contrast against the theme's chart surface):

| | `paper` (on `#FFFFFF`) | `terminal` (on `#0B1017`) |
|---|---|---|
| Lightness band | PASS, all 8 in L 0.43–0.77 | PASS, all 8 in L 0.48–0.67 |
| Chroma floor | PASS, all ≥ 0.10 | PASS, all ≥ 0.10 |
| CVD separation (adjacent) | PASS, worst ΔE 12.5 (deutan) | PASS, worst ΔE 9.3 (deutan) |
| Normal-vision floor | PASS, worst ΔE 20.1 | PASS, worst ΔE 15.2 |
| Contrast vs surface | PASS, all ≥ 3 : 1 | PASS, all ≥ 3 : 1 |

**Rules:**
- V7. Hues are assigned in **fixed slot order** (§2.1.7) and are **never cycled**. Filtering a
  chart down to three series MUST NOT repaint the survivors.
- V8. A ninth series is never a generated hue — fold into "Other", facet into small multiples,
  or add a second encoding.
- V9. Sequential = one hue, light→dark. Diverging = two hues plus a **neutral grey** midpoint.
  Never a rainbow, never a hue at the diverging midpoint.
- V10. **Status colours are reserved** and never appear as a series.
- V11. If you change a value in an Instrument scale, re-run the validator before merging.

#### 2.8.4 Chart anatomy and interaction

- Legend is **always present for ≥ 2 series**; a single-series chart needs none (the title
  names it). Up to 4 series are **also** direct-labelled at the line end.
- Text in and around a chart wears text tokens (`--fg-primary/secondary/tertiary`), **never**
  the series colour. The mark beside the label carries the identity.
- Line series 2px; markers ≥ 8px; a 2px surface-coloured gap between adjacent stacked fills
  and a 2px surface ring on overlapping marks.
- Every line/area chart ships a **crosshair + tooltip**; every bar/dot/cell chart ships a
  per-mark tooltip. Hit targets are larger than the marks.
- Every chart has a **table view** toggle (keyboard-reachable) exposing the same data.
- Filters and range selectors live in **one row above** the chart, never inside the plot.

---

## 3. Components

For each: anatomy → sizes → states → tokens → accessibility. Every component MUST implement
**all** of: default, hover, active/pressed, focus-visible, selected, disabled, loading, error
(where applicable). A component with an undesigned state is not complete.

### 3.1 Button

**Variants:** `primary` · `secondary` (default) · `ghost` · `danger` · `buy` · `sell`.
**Sizes:** `sm` (`--ctl-h-sm`) · `md` (`--ctl-h-md`, default) · `xl` (`--ctl-h-xl`, submit only).

| Variant | Fill | Text | Border |
|---|---|---|---|
| primary | `--bg-accent` | `--fg-on-accent` | none (+`--glow-accent` in `terminal`) |
| secondary | `--bg-surface` | `--fg-primary` | `--line-default` |
| ghost | transparent | `--fg-secondary` | none |
| danger | `--bg-neg` | `--fg-on-neg` | none |
| buy | `--bg-pos` | `--fg-on-pos` | none (+`--glow-pos`) |
| sell | `--bg-neg` | `--fg-on-neg` | none (+`--glow-neg`) |

**States:** hover → `--bg-*-hover` (filled) or `--bg-hover` overlay (secondary/ghost);
active → `--bg-*-active` + `translateY(0.5px)`; focus-visible → `--shadow-focus`
(2px ring, 2px offset); disabled → 45% opacity, `cursor: not-allowed`, no hover;
loading → spinner replaces the leading icon, **label text stays** (never collapse the button
width mid-flight), `aria-busy="true"`, pointer-events none.

**MUST:** icon-only buttons get `aria-label`. Minimum hit target 30×30 even at `sm`
(use padding, not size). The submit button's label states the actual action and amount —
`Buy 1.250 BTC · $80,312.50`, not `Submit`.

### 3.2 Icon button
`--ctl-h-sm` square, `--r-sm`, `--line-hairline` border, `--fg-secondary` icon.
Same state model as `ghost`. Always `aria-label` + tooltip.

### 3.3 Segmented control
Track `--bg-sunken` + `--line-hairline`, 2px padding, 2px gap.
Thumb in `paper` = `--bg-surface` + `--shadow-1`; in `terminal` = `--bg-accent-subtle` +
`--fg-accent`, no shadow. Thumb slides `--dur-2 --ease-spring`.
**Buy/Sell variant** (`.seg.bs`): active buy = `--bg-pos-subtle` / `--fg-pos` / 1px
`--line-pos` inset; sell mirrors with the neg family. Role `radiogroup`; ←/→ move selection.

### 3.4 Tabs
36px tall, `--s-4` gap, 2px `--line-accent` bottom border on the active tab, `--fg-primary`
active / `--fg-tertiary` inactive. `terminal` uppercases at 11px/+0.08em. Counts ride as a
neutral badge. Role `tablist`; ←/→ + Home/End; `aria-selected`.

### 3.5 Input / number field
`--ctl-h-md`, `--r-sm`, `--line-default` border, `--bg-surface` (`paper`) / `--bg-inset`
(`terminal`). Value in `--font-numeric` at 14px; unit suffix in `--fg-tertiary` at 12px.
Focus → `--line-focus` border + `--shadow-focus`.
Error → `--line-neg` border + message below in `--fg-neg` at 11px with a warning icon.
**Number fields MUST:** accept paste with separators, step with ↑/↓ (×10 with Shift),
clamp to instrument tick size on blur, and never reformat mid-typing.

### 3.6 Percent-of-buying-power row
5 equal buttons (`10% · 25% · 50% · 75% · Max`), `--ctl-h-xs`+2, `--bg-sunken`,
active = `--bg-accent-subtle` + `--line-accent`. Selecting one writes the size field;
typing in the size field clears the selection.

### 3.7 Select / dropdown
Trigger matches Input. Menu = `--bg-overlay`, `--r-md`, `--shadow-3`, 4px padding,
30px items, `--bg-hover` on highlight, `--bg-selected` + check glyph on the selected item.
Full keyboard: ↑/↓, type-ahead, Enter, Esc. `role="listbox"`.

### 3.8 Toggle / switch
36×20 track (`paper`, `--r-pill`) or 32×18 (`terminal`, `--r-xs`). Off = `--bg-inset`;
on = `--bg-accent`. Knob `--bg-surface`, `--dur-2 --ease-spring`.
A toggle that changes money behaviour (e.g. *Reduce only*, *Live trading*) MUST have a
text label and MUST NOT be the only confirmation (§5.5).

### 3.9 Badge and chip
20px tall, `--r-xs`, 10px/600/+0.02em uppercase.
`badge` variants: `pos · neg · warn · info · accent · neutral` (subtle fill + matching fg).
`chip` = neutral identity token (`--chip-bg/fg/line`) used for asset-class labels (§2.1.7).
A badge MUST contain a word, never colour alone.

### 3.10 Panel
See §2.3.5. Border `--line-hairline`, radius `--r-lg`, `--shadow-1` in `paper` /
`--ring-inset` in `terminal`. Header 38px. Body scrolls; header and footer do not.

### 3.11 Table / data grid

The most important component in the product.

- Header row 30px, `--label-*` styling, sticky, `--bg-surface` ground, hairline bottom.
- Body rows `--row-h`, hairline bottom, last row borderless.
- Hover `--bg-hover`; selected `--bg-selected` + 2px `--line-accent` left border.
- First column left-aligned (identity); **every** other column right-aligned.
- Identity cell = `swatch? + ticker (13/600) + name (11/tertiary)` stacked.
- Numeric cells carry `.n` → `--font-numeric` + `tnum`.
- Footer/total row: `--bg-sunken`, 1px `--line-default` top, semibold key figures.
- Sort: click header toggles asc/desc; active sort shows a caret and `aria-sort`.
- Column resize, reorder and show/hide MUST be supported and persisted per table per user.
- **Virtualise** any table that can exceed 100 rows. Row height is fixed, which makes this easy.
- Empty state (§3.22) replaces the body, never the header.
- Loading: skeleton rows at `--bg-skeleton`, same height, **never** a spinner over the table.

### 3.12 Stat tile
`label (--label-*)` over `value (20/600, numeric)` over optional `meta (11, tertiary)`.
In a hero strip, tiles are separated by 1px `--line-hairline` verticals, not gaps.
The value MUST be the largest thing in the tile.

### 3.13 Order ticket
Fixed anatomy, top to bottom:
1. Side segmented (`buy`/`sell`) — full width, tinted.
2. Order-type segmented — `Market · Limit · Stop · Bracket`.
3. Price field (hidden for Market).
4. Size field + percent row + unit toggle (base/quote).
5. Secondary row: Time in force · Leverage (perps/margin only).
6. **Cost summary** in a `--bg-sunken` well: order value, est. fee with the rate shown,
   margin required, est. liquidation (in `--fg-neg`), and buying power after.
7. **Attached bracket** block (when Bracket): tinted `--bg-pos-subtle` with a `R : R` badge,
   take-profit, stop-loss, and *risk on this trade in both currency and % of equity*.
8. Submit button (`xl`, `buy`/`sell`) whose label restates side, size and value.
9. Environment reminder line (§5.4).

**MUST:** every derived figure recomputes on every input change within one frame;
the submit button is disabled with an inline reason whenever the ticket is invalid;
invalid reasons appear next to the offending field, not only at the bottom.

### 3.14 Order book
Three columns (price · size · cumulative total), 19px rows, `--font-numeric`, 11px.
Depth bar is an absolutely-positioned tinted rect anchored to the **right** edge, width
proportional to cumulative size, `--bg-neg-subtle` for asks / `--bg-pos-subtle` for bids.
Price column coloured; size and total stay `--fg-primary`.
Mid block between the sides: last price (14/600, direction-coloured) + mid.
Clicking a level writes that price into the ticket.

### 3.15 Watchlist row
Two columns: identity (ticker 13/600 + name 11/tertiary) and right-aligned
(price 12/500 + change % 11 direction-coloured). Active row = `--bg-selected` + 1px
`--line-accent` inset. Grouped by asset class with a swatch + label header.

### 3.16 Tooltip
`--bg-overlay`, `--r-sm`, `--shadow-3`, 11px, 6/8px padding, 300ms open delay / 0 close,
8px offset, arrow optional. Never contains interactive content. `role="tooltip"`.

### 3.17 Popover / menu
`--bg-overlay`, `--r-md`, `--shadow-3`, 6px padding. Focus trapped, Esc closes,
returns focus to the trigger, `--dur-3 --ease-out` scale-from-0.98 + fade.

### 3.18 Modal
`--r-xl`, `--shadow-4`, scrim `--bg-scrim`, max-width 560 (or 720 for forms).
Header / body / footer; footer actions right-aligned, primary last.
Focus trapped; Esc closes unless the modal is a destructive confirm (§5.5).

### 3.19 Top bar
`--nav-h`, `--bg-surface`, hairline bottom. Left: brand mark + wordmark.
Centre: nav pills. Right, in this order: **environment pill** (§5.4) → equity readout →
day P&L readout → connection/latency indicator → account avatar.
The equity and P&L readouts are `label` over `value` at 13/600, right-aligned.
The nav MUST show the active route with both fill and colour, never colour alone.

### 3.20 Graph node (strategy canvas)
Width 156–200px depending on zoom. Anatomy:
`header (26px: 3px family rail + icon + uppercase family label)` →
`title (12/600)` → `fields (label left in --fg-tertiary, value right in a --bg-sunken pill)`.
Selected = `--line-accent` border + `--shadow-3` + accent ring (`paper`) or `--glow-accent`
(`terminal`). Invalid = `--line-neg` border + a warning badge in the header.
**Ports:** 9px, `--r-circle` in `paper` / 2px square in `terminal`; unconnected =
`--port` fill + `--port-line` border; connected = family colour.
Inputs on the left edge, outputs on the right, distributed evenly by count.

### 3.21 Graph edge
Cubic bezier, horizontal control points at `max(52, Δx × 0.5)`.
1.5px `--edge` at 85% opacity; the edge into/out of the selected node is 2px `--edge-active`
at 100%. Invalid connection = `--edge-invalid` dashed.
**Layout rule:** the canvas auto-layout MUST place nodes so every edge flows left→right.
A backward edge produces a loop that reads as a bug. Stages: data → indicators →
signals → logic → intent → risk.

### 3.22 Empty, loading, error and stale states

| State | Treatment |
|---|---|
| Empty (no data yet) | Centred icon (20px, `--fg-tertiary`), one-line explanation, one primary action. Never a bare "No data". |
| Empty (filtered to nothing) | "No results for *<query>*" + a **Clear filters** button. |
| Loading | Skeletons at `--bg-skeleton` matching the final layout's shape and height. No spinners over content; a spinner is only allowed inside a button. |
| Partial | Render what has arrived; skeleton the rest. Never block the whole panel on the slowest feed. |
| Error | Inline in the affected panel: icon, plain-language cause, **Retry**. Never a toast for a panel-scoped failure. |
| Stale | 60% opacity on the figures + a `warn` badge naming the age ("12s old"). (§5.2) |
| Disconnected | Top-bar indicator turns `warn`→`neg`, plus one persistent banner. Data keeps its last value, marked stale. |

---

## 4. Screen blueprints

### 4.1 Dashboard (`/dashboard`)

**Purpose:** answer "where do I stand and what is running" in under five seconds.

```
topbar 56
pagehead 52   Portfolio · account chip · [range segmented] · Export · New order
┌──────────────────────────────────────────────────────────┬──────────────────┐
│ HERO STRIP (full width, 6 cells, hairline dividers)      │                  │
│ equity(300, hero+sparkline) │ day │ unreal │ real │ win │ buying power      │
├──────────────────────────────────────────────────────────┼──────────────────┤
│ EQUITY CURVE  (312px)                                    │ ALLOCATION       │
│ area chart, deposit markers, dd/sharpe/best-day in hd    │ stacked bar +    │
│                                                          │ 8-row legend     │
├──────────────────────────────────────────────────────────┼──────────────────┤
│ PORTFOLIO DETAIL (tabbed, fills remaining height)        │ ACTIVE AUTOMATIONS│
│ [By asset class] [Open positions 4] [Recent fills] [Orders]│ 4 rows          │
│ 8 rows + totals row                                      ├──────────────────┤
│                                                          │ RISK & EXPOSURE  │
└──────────────────────────────────────────────────────────┴──────────────────┘
```

**This directly replaces the eight-column layout.** The asset-class table columns are:
`Asset class · Equity · Cash · Day P&L · Total P&L · Win rate · Trades · Open · Allocation`,
with a totals row that MUST reconcile to the hero equity figure.
The allocation cell pairs a mini bar (scaled to the largest class, not to 100%) with the
exact percentage.

**MUST:** the hero equity figure equals the sum of the table's Equity column;
day P&L equals the sum of the Day P&L column. If they cannot reconcile, that is a data bug
to surface, not to hide.

### 4.2 Trading terminal (`/trading`)

```
topbar 56
instrument bar 44   [asset chip] BTC-USD ▾ | last +chg | bid ask hi lo vol funding |
                    ⟶ [timeframe segmented] [Indicators] [Alert]
┌───────────┬──────────────────────────────────────────┬────────────────┐
│ WATCHLIST │ PRIMARY CHART (fills)                    │ ORDER TICKET   │
│ 280       │ candles + volume + EMAs + last-price tag │ 340            │
│ grouped   │ entry/stop/target annotations            │ (full height)  │
│ by class  ├──────────────────────────────────────────┤                │
│           │ DOCK 262                                 │                │
├───────────┤ [Positions 4][Open orders 2][Fills][Automations]           │
│ ORDER BOOK│ net exposure · margin used in the tab bar│                │
│ 332       │                                          │                │
└───────────┴──────────────────────────────────────────┴────────────────┘
```

**Multi-chart** (the current app's 4-pane view) becomes an explicit **layout mode** in the
chart panel header (`1 · 2 · 2×2 · 3+1`). Default is 1. In any multi-chart mode the
instrument bar reflects the **focused** pane, and the focused pane carries a 1px
`--line-accent` border.

**MUST:** the order ticket is always visible at ≥1440px and never behind a tab.
Positions table columns: `Instrument · Side · Size · Entry · Mark · Unrealized · Return ·
Liq. price · Stop · Source`. `Source` is a badge — `neutral` for Manual, `accent` for an
automation, and it links to that automation.

### 4.3 Strategy builder (`/strategy`)

```
topbar 56
strategy bar 52   [name field ✎] [Draft] version/edited · ⟶ [✓ n valid][n warnings] |
                  Validate · Run backtest · Save · Deploy to paper
┌───────────┬───────────────────────────────────────────┬─────────────────┐
│ BLOCKS    │ CANVAS (dot grid)                         │ INSPECTOR 340   │
│ 268       │ nodes + bezier edges, left→right stages   │ selected node   │
│ search    │                                           │ properties      │
│ 7 families│  data → indicator → signal → logic →      ├─────────────────┤
│ w/ family │         intent → risk                     │ connections     │
│ dots      │                                           ├─────────────────┤
│           │ ┌ zoom tools ┐        ┌ VALIDATION ─────┐ │ backtest preview│
│           │ └────────────┘        └─────────────────┘ ├─────────────────┤
└───────────┴───────────────────────────────────────────┤ risk guardrails │
                                                        └─────────────────┘
```

**Improvements over the current builder, all required:**
- Blocks panel gains a **search field** and **family grouping with a colour dot**; the
  flat alphabetical list goes away.
- The "Tips" box that occupied prime canvas real estate is replaced by a **floating
  Validation console** docked bottom-right, showing typed messages (ok / warning / info)
  with timestamps. It is collapsible and remembers its state.
- The inspector replaces in-node editing of long values: nodes show a **summary**, the
  inspector holds the full form. Nodes stay small so the graph stays readable.
- The inspector carries a **backtest preview** (net P&L, win rate, trades, max DD, Sharpe,
  exposure + an equity sparkline) so the user never has to leave the canvas to judge a change.
- A **risk guardrails** block shows the account-level limits that will clamp this strategy —
  the current UI lets users build strategies that silently violate account rules.
- Validation is **continuous**, not on-demand: the strategy bar's `✓ n valid / n warnings`
  updates as the graph changes; the `Validate` button forces a full re-run.

**MUST:** `Deploy` is disabled while any error exists. `Deploy to live` (as opposed to paper)
is a destructive-class action (§5.5).

### 4.4 Screens not mocked

`Automations`, `Back Testing` and `Settings` are built from the same parts and MUST NOT
invent new patterns:

- **Automations** — a full-width table (`Name · Strategy · Markets · State · Today P&L ·
  Total P&L · Win rate · Trades · Last signal · Actions`) + a detail drawer. State is a badge:
  `Running` (accent) · `Paused` (warn) · `Stopped` (neutral) · `Error` (neg).
- **Back Testing** — a run configuration panel (left 340), a results area with the equity
  curve on top and a tabbed dock (Trades · Monthly returns · Drawdown · Statistics).
  Monthly returns use the **diverging** ramp (§2.8.3), never the categorical one.
- **Settings** — a two-column layout: 240px section nav + a `--content-max` reading column.
  This is the one place `paper`'s reading typography leads. Theme, density, tick-flash,
  number-format and risk-limit controls live here.

---

## 5. Behaviour

### 5.1 Real-time data
- Tick flash: on a price change, the cell background flashes `--bg-pos-subtle` /
  `--bg-neg-subtle` and decays to transparent over `--dur-3`. No movement, no counting.
- Throttle UI updates to **one paint per animation frame**, regardless of feed rate.
  A 200 Hz feed MUST NOT cause 200 renders.
- Row identity is keyed by instrument id; a re-sort MUST NOT re-key rows (no flash storm).

### 5.2 Staleness and connection
- A quote older than **2s** (streaming) or **15s** (polled) is stale: 60% opacity + a
  `warn` badge with the age.
- Connection states: `connected` (pos dot + latency ms) · `degraded` (warn dot + "delayed") ·
  `reconnecting` (warn, animated dot) · `disconnected` (neg dot + persistent banner).
- On reconnect, reconcile and briefly flash changed cells; do not full-clear the screen.

### 5.3 Number formatting
Centralise in one `format` module. No component formats numbers itself.
`price(instrument, v)` · `qty(instrument, v)` · `money(v, ccy)` · `pct(v, dp)` ·
`signed(v)` · `compact(v)`. All respect §2.2.5.

### 5.4 Environment (paper vs live)
- The environment pill in the top bar is **always visible**: `Paper` = `warn` family,
  `Live` = `pos` family with a solid dot.
- In **Live**, the order ticket gains a 2px `--line-accent` top border and the submit button
  text is prefixed `LIVE — `.
- Switching paper→live is a destructive-class action (§5.5) and MUST re-confirm on every
  session, not once ever.
- The paper reminder line under the submit button ("Paper account — no live funds at risk")
  is required in paper mode.

### 5.5 Destructive and irreversible actions
Applies to: submitting a live order, cancelling all orders, closing all positions,
deploying a strategy to live, deleting a strategy or automation, and switching to live.

- Use the `danger` button variant.
- Require a confirmation modal that **names the exact consequence with numbers**
  ("Close 4 positions with a combined notional of $412,880?").
- For the highest-severity three (live switch, live deploy, close all), require typing a
  confirmation word or holding the button for 800ms.
- Esc MUST NOT dismiss these modals; the cancel button must be clicked.
- Always offer the reversible alternative where one exists (pause instead of delete).

### 5.6 Keyboard
Global: `⌘K` command palette · `⌘/` shortcut sheet · `g d/t/a/s/b` go to section ·
`⌘\` toggle theme · `⌘.` toggle density.
Trading: `b` focus buy ticket · `s` focus sell ticket · `Esc` clear ticket ·
`⌘↵` submit (with confirm) · `1–6` timeframe · `/` focus watchlist search.
Canvas: `Space+drag` pan · `⌘0` fit · `⌘+/-` zoom · `Del` delete selection ·
`⌘D` duplicate node · `Tab` cycle nodes.
Every shortcut MUST be discoverable in the `⌘/` sheet.

---

## 6. Accessibility

- **A1.** All text meets WCAG AA (4.5:1 body, 3:1 for ≥18.66px bold / ≥24px). Verified: 89/89.
- **A2.** Non-text UI (borders of inputs, focus rings, chart marks) meets 3:1 against its
  adjacent surface.
- **A3.** Focus is **always visible**: 2px `--line-focus` ring at 2px offset. Never
  `outline: none` without a replacement.
- **A4.** Colour is never the sole carrier of meaning (§1.3.3). Direction gets a sign or
  arrow; state gets a word; chart series get a legend and/or direct labels.
- **A5.** Full keyboard operability, logical tab order, visible skip-to-content link.
- **A6.** Live regions: order fills and connection changes announce via `aria-live="polite"`;
  errors via `aria-live="assertive"`. Streaming price cells are `aria-live="off"` —
  announcing every tick is hostile.
- **A7.** Tables use real `<table>` semantics with `<caption>`, `scope`, and `aria-sort`.
- **A8.** Charts have `role="img"` with a summarising `aria-label`, plus the table view (§2.8.4).
- **A9.** `prefers-reduced-motion` honoured (§2.6 M4).
- **A10.** Minimum 30×30 hit target for every interactive element; 44×44 on touch layouts.
- **A11.** Zoom to 200% MUST NOT break layout or clip content.
- **A12.** Test both themes under protan and deutan simulation before release.

---

## 7. Responsive

| Breakpoint | Layout |
|---|---|
| ≥ 1680 | Full three-rail trading layout; dashboard 2-column. |
| 1440–1679 | Watchlist narrows to 240; order ticket stays 340. |
| 1280–1439 | Order book moves into the dock as a tab; watchlist collapses to an icon rail with a flyout. |
| 1024–1279 | Single content column; order ticket becomes a right drawer opened by `b`/`s`; dashboard right column stacks below. |
| < 1024 | Mobile/tablet: bottom tab bar; chart-first trading view; order ticket is a full-height sheet; strategy canvas is **read-only** with a "edit on desktop" notice. |

The strategy canvas MUST NOT attempt full editing below 1024px. A degraded read-only view
is the correct answer; a cramped editor is not.

---

## 8. Implementation

### 8.1 Recommended stack

Since the brief is a complete teardown, use the stack that makes this spec cheapest to hold
correct over time:

- **React 19 + TypeScript (strict)** — you are already in React.
- **Vite** for the app shell.
- **Tailwind CSS v4** mapped onto `tokens.css` via `@theme` (`tailwind.theme.css`).
  v4's CSS-first config means the tokens file *is* the theme — no JS config drift.
- **Radix UI primitives** (unstyled) for Dialog, Popover, Select, Tabs, Tooltip, Toggle,
  Slider, DropdownMenu. They give you the accessibility requirements in §6 for free.
  Style them with the tokens; do not adopt any pre-styled component library.
- **TanStack Table v8** for every table (sorting, column sizing/order/visibility, virtualisation).
- **TanStack Virtual** for long lists.
- **Charts:** `lightweight-charts` (TradingView) for candlestick/price panes — it is
  canvas-based and handles streaming appends without re-render churn. Everything else
  (equity curve, allocation, monthly returns, sparklines) in **hand-rolled SVG** or
  **visx**. Do **not** use a chart library that ships its own colour theme;
  every chart colour comes from `--chart-*`.
- **React Flow (xyflow)** for the strategy canvas — it already does ports, bezier edges,
  pan/zoom, selection and minimap. Restyle its CSS variables from `--node-*` / `--edge-*`.
  Use its `nodeTypes` to implement §3.20 exactly; do not use its default node.
- **Zustand** (or your existing store) for UI state; **TanStack Query** for request state.
- **Motion:** CSS transitions with `--dur-*` / `--ease-*`. Reach for a JS animation library
  only for the canvas, and even then, sparingly.

If you prefer to keep the current stack, everything above §8 still applies unchanged —
the spec is framework-agnostic. Only this section is advisory.

### 8.2 File structure

```
src/
  styles/
    tokens.css            ← the file shipped with this spec, verbatim
    tailwind.theme.css    ← @theme mapping
    base.css              ← reset + element defaults only
  lib/
    format.ts             ← §5.3, the ONLY place numbers are formatted
    theme.ts              ← theme + density resolution and persistence
  components/
    primitives/           ← Button, Input, Select, Segmented, Tabs, Badge, Panel, Table…
    charts/               ← CandleChart, EquityCurve, AllocationBar, Sparkline, Legend
    trading/              ← OrderTicket, OrderBook, Watchlist, PositionsTable
    strategy/             ← Canvas, Node, Port, Inspector, BlockPalette, ValidationConsole
  screens/
    Dashboard/ Trading/ Automations/ Strategy/ BackTesting/ Settings/
```

### 8.3 Theme mechanism

```html
<html data-theme="paper" data-density="comfortable">
```

- Resolve on the server or in a tiny blocking inline script **before first paint** to avoid
  a flash. Read `localStorage.theme`, fall back to `prefers-color-scheme`.
- Switching sets the attribute. Nothing else. No class toggling on components, no JS colour
  objects, no re-render required.
- `color-scheme` is set per theme so native scrollbars and form controls follow.

### 8.4 Tailwind v4 mapping (excerpt — full file in `tailwind.theme.css`)

```css
@import "tailwindcss";
@import "./tokens.css";

@theme inline {
  --color-canvas:        var(--bg-canvas);
  --color-surface:       var(--bg-surface);
  --color-sunken:        var(--bg-sunken);
  --color-fg:            var(--fg-primary);
  --color-fg-secondary:  var(--fg-secondary);
  --color-fg-tertiary:   var(--fg-tertiary);
  --color-accent:        var(--bg-accent);
  --color-pos:           var(--fg-pos);
  --color-neg:           var(--fg-neg);
  --color-line:          var(--line-default);
  --radius-sm:           var(--r-sm);
  --radius-md:           var(--r-md);
  --radius-lg:           var(--r-lg);
  --font-ui:             var(--font-ui);
  --font-display:        var(--font-display);
  --font-numeric:        var(--font-numeric);
}
```

`@theme inline` is required so the utilities resolve to the *variable*, not its value at
build time — otherwise theme switching stops working.

### 8.5 Migration order

Do it in this sequence. Each step is independently shippable.

1. **Token layer.** Drop in `tokens.css`, wire `data-theme`/`data-density`, add the theme
   toggle. Nothing looks different yet.
2. **Kill hardcoded colour.** Sweep every component; replace literals with semantic tokens.
   Turn on the CI lint (§8.6). This is the highest-value step and the only truly tedious one.
3. **Typography + `format.ts`.** Load the four faces, apply the role table, route every
   number through the formatter. Tabular numerals land here.
4. **Primitives.** Rebuild Button, Input, Select, Segmented, Tabs, Badge, Panel, Table
   against §3. Replace usages screen by screen.
5. **Shell.** Top bar, environment pill, connection indicator, page headers, keyboard map.
6. **Dashboard.** The eight-column layout is deleted and replaced per §4.1. Biggest visible win.
7. **Trading.** Instrument bar, single-chart default + layout modes, dock, ticket, book.
8. **Strategy.** Node taxonomy, palette grouping + search, inspector, validation console,
   left→right auto-layout.
9. **Automations / Back Testing / Settings** per §4.4.
10. **States pass.** Walk every screen through empty / loading / partial / error / stale /
    disconnected and build what is missing.
11. **A11y pass.** §6 checklist, both themes, keyboard-only run-through, CVD simulation.

### 8.6 CI gates (all MUST pass to merge)

- `stylelint` rule: no colour literal outside `src/styles/tokens.css`.
- `eslint` rule: no `style={{ color / background }}` with a literal; no inline hex in TSX.
- Contrast script re-runs on every change to `tokens.css` and fails on any regression.
- Chart palette validator re-runs if any `--chart-1..8` value changes.
- Visual regression (Playwright screenshots) on the three primary screens **× both themes**
  × both densities — 12 snapshots, the same harness that produced this spec's mockups.
- `axe-core` automated pass on every screen in both themes, zero violations.

### 8.7 Definition of done

A screen is done when **all** of the following are true:

1. It contains no colour literal, no hardcoded px outside the space scale, and no font stack
   not named in §2.2.1.
2. It renders correctly in `paper`/comfortable, `paper`/compact, `terminal`/comfortable and
   `terminal`/compact, with **zero layout difference between themes**.
3. Every interactive element has all eight states (§3) and a visible focus ring.
4. Every number is tabular, correctly precise per instrument, and formatted through
   `format.ts`.
5. Empty, loading, partial, error and stale states exist and are reachable in Storybook.
6. Keyboard-only operation completes the screen's primary task.
7. `axe-core` reports zero violations in both themes.
8. No horizontal scroll at 1280px; no clipped content at 200% zoom.
9. Every chart on it passes §2.8 (legend, direct labels, table view, tooltip, no dual axis).
10. Screenshots in both themes are attached to the PR.

---

## Appendix A — What must never happen

A short list of the failure modes this system exists to prevent. Treat each as a P1 bug.

1. A hex value outside `tokens.css`.
2. A dark theme generated by inverting the light theme.
3. Proportional (non-tabular) digits on a changing number.
4. A price or P&L animating its position or counting up.
5. Red/green as the only indicator of direction.
6. A dual-axis chart.
7. Cycled or re-assigned categorical chart colours when a filter changes.
8. Clay (`paper` brand accent) used on anything expressing buy/sell direction.
9. A saturated full-width node header on the strategy canvas.
10. A backward-flowing edge on the strategy canvas.
11. A spinner covering a table that could show skeleton rows.
12. An irreversible action behind a single unconfirmed click.
13. A live order submitted from a UI that does not visibly say "Live".
14. `outline: none` without a replacement focus indicator.
15. A ninth asset class given an invented hue.
