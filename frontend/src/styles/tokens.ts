/**
 * Typed mirror of the semantic token layer in `tokens.css`.
 *
 * Purpose: make a mistyped token name a COMPILE ERROR rather than a silently
 * transparent element. Import `cssVar` (or the `T` map) anywhere you need a token
 * in TS/JSX — never write the string by hand.
 *
 *   import { cssVar } from "@/styles/tokens";
 *   <div style={{ background: cssVar("bg-surface") }} />
 *
 * This file contains NO colour values. Values live only in `tokens.css`.
 */

/* ------------------------------------------------------------------ themes */

export const THEMES = ["paper", "terminal"] as const;
export type Theme = (typeof THEMES)[number];

export const DENSITIES = ["comfortable", "compact"] as const;
export type Density = (typeof DENSITIES)[number];

/** Default density per theme. A user setting overrides this. */
export const DEFAULT_DENSITY: Record<Theme, Density> = {
  paper: "comfortable",
  terminal: "compact",
};

/* ------------------------------------------------------- semantic surfaces */

export const SURFACE_TOKENS = [
  "bg-canvas",
  "bg-surface",
  "bg-surface-raised",
  "bg-sunken",
  "bg-inset",
  "bg-hover",
  "bg-active",
  "bg-selected",
  "bg-overlay",
  "bg-scrim",
  "bg-accent",
  "bg-accent-hover",
  "bg-accent-active",
  "bg-accent-subtle",
  "bg-pos",
  "bg-pos-hover",
  "bg-pos-subtle",
  "bg-neg",
  "bg-neg-hover",
  "bg-neg-subtle",
  "bg-warn-subtle",
  "bg-info-subtle",
  "bg-neutral-subtle",
  "bg-skeleton",
] as const;

export const FOREGROUND_TOKENS = [
  "fg-primary",
  "fg-secondary",
  "fg-tertiary",
  "fg-disabled",
  "fg-on-accent",
  "fg-on-pos",
  "fg-on-neg",
  "fg-accent",
  "fg-pos",
  "fg-neg",
  "fg-warn",
  "fg-info",
  "fg-link",
] as const;

export const LINE_TOKENS = [
  "line-hairline",
  "line-default",
  "line-strong",
  "line-accent",
  "line-pos",
  "line-neg",
  "line-focus",
] as const;

export const ELEVATION_TOKENS = [
  "shadow-0",
  "shadow-1",
  "shadow-2",
  "shadow-3",
  "shadow-4",
  "shadow-focus",
  "glow-accent",
  "glow-pos",
  "glow-neg",
  "ring-inset",
] as const;

/* -------------------------------------------------------------- chart / viz */

export const CHART_TOKENS = [
  "chart-surface",
  "chart-grid",
  "chart-grid-major",
  "chart-axis",
  "chart-crosshair",
  "chart-up",
  "chart-up-line",
  "chart-down",
  "chart-down-line",
  "chart-volume",
  "chart-last",
  "chart-tag-bg",
  "chart-tag-fg",
  "chart-ma-fast",
  "chart-ma-slow",
  "chart-equity",
  "chart-area-from",
  "chart-area-to",
  "chart-1", "chart-2", "chart-3", "chart-4",
  "chart-5", "chart-6", "chart-7", "chart-8",
  "chart-seq-1", "chart-seq-2", "chart-seq-3", "chart-seq-4", "chart-seq-5",
  "chart-div-n2", "chart-div-n1", "chart-div-0", "chart-div-p1", "chart-div-p2",
] as const;

/* ------------------------------------------------------------ strategy graph */

export const GRAPH_TOKENS = [
  "canvas-bg",
  "canvas-dot",
  "canvas-dot-major",
  "node-bg",
  "node-line",
  "node-shadow",
  "edge",
  "edge-active",
  "edge-invalid",
  "port",
  "port-line",
  "node-data",
  "node-indicator",
  "node-signal",
  "node-logic",
  "node-intent",
  "node-risk",
  "node-ai",
  "chip-bg",
  "chip-fg",
  "chip-line",
] as const;

/* ------------------------------------------- scale / shape / motion / layout */

export const SCALE_TOKENS = [
  "s-px", "s-1", "s-2", "s-3", "s-4", "s-5", "s-6", "s-7", "s-8",
  "s-10", "s-12", "s-14", "s-16", "s-20", "s-24", "s-32",
  "t-10", "t-11", "t-12", "t-13", "t-14", "t-16", "t-18", "t-20",
  "t-24", "t-28", "t-32", "t-40", "t-48", "t-60",
  "r-xs", "r-sm", "r-md", "r-lg", "r-xl", "r-2xl", "r-pill", "r-circle",
  "ctl-h-xs", "ctl-h-sm", "ctl-h-md", "ctl-h-lg", "ctl-h-xl",
  "row-h", "row-pad-x", "panel-pad", "tile-gap",
  "icon-sm", "icon-md", "icon-lg", "icon-xl",
  "nav-h", "subnav-h", "rail-w",
  "panel-w-sm", "panel-w-md", "panel-w-lg", "content-max",
  "dur-1", "dur-2", "dur-3", "dur-4", "dur-5",
  "ease-out", "ease-in-out", "ease-spring", "ease-linear",
  "font-display", "font-ui", "font-mono", "font-numeric",
  "label-size", "label-weight", "label-transform", "label-tracking",
  "w-regular", "w-medium", "w-semibold", "w-bold",
  "z-sticky", "z-nav", "z-dropdown", "z-tooltip", "z-modal", "z-toast",
] as const;

/* ------------------------------------------------------------------- public */

export type SurfaceToken = (typeof SURFACE_TOKENS)[number];
export type ForegroundToken = (typeof FOREGROUND_TOKENS)[number];
export type LineToken = (typeof LINE_TOKENS)[number];
export type ElevationToken = (typeof ELEVATION_TOKENS)[number];
export type ChartToken = (typeof CHART_TOKENS)[number];
export type GraphToken = (typeof GRAPH_TOKENS)[number];
export type ScaleToken = (typeof SCALE_TOKENS)[number];

export type Token =
  | SurfaceToken
  | ForegroundToken
  | LineToken
  | ElevationToken
  | ChartToken
  | GraphToken
  | ScaleToken;

/** `cssVar("bg-surface")` → `"var(--bg-surface)"` */
export const cssVar = (t: Token): string => `var(--${t})`;

/** `cssVarWithFallback("bg-surface", "#fff")` → `"var(--bg-surface, #fff)"` */
export const cssVarWithFallback = (t: Token, fallback: string): string =>
  `var(--${t}, ${fallback})`;

/* --------------------------------------------------- domain → token mapping */

/** Asset classes in their FIXED chart-slot order. Slot 1 is index 0. Never reorder. */
export const ASSET_CLASSES = [
  "crypto-spot",
  "equities",
  "etf",
  "fx",
  "perpetuals",
  "options",
  "dex-amm",
  "prediction",
] as const;
export type AssetClass = (typeof ASSET_CLASSES)[number];

/**
 * The categorical slot for an asset class. Fixed for the life of the product:
 * filtering a chart must never repaint the surviving series.
 */
export const assetClassChartToken = (c: AssetClass): ChartToken =>
  `chart-${(ASSET_CLASSES.indexOf(c) + 1) as 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8}` as ChartToken;

/** Strategy block families. `logic` is intentionally neutral. */
export const NODE_FAMILIES = [
  "data",
  "indicator",
  "signal",
  "logic",
  "ai",
  "intent",
  "risk",
] as const;
export type NodeFamily = (typeof NODE_FAMILIES)[number];

export const nodeFamilyToken = (f: NodeFamily): GraphToken =>
  `node-${f}` as GraphToken;

/** Direction of a numeric change. `flat` is deliberately colourless. */
export type Direction = "up" | "down" | "flat";

export const directionForegroundToken = (d: Direction): ForegroundToken =>
  d === "up" ? "fg-pos" : d === "down" ? "fg-neg" : "fg-secondary";

/** The glyph that must accompany a direction when no explicit sign is shown. */
export const directionGlyph = (d: Direction): string =>
  d === "up" ? "▲" : d === "down" ? "▼" : "";

/** Status families used by badges, dots and inline messages. */
export const STATUSES = ["ok", "warn", "error", "info", "neutral"] as const;
export type Status = (typeof STATUSES)[number];

export const statusForegroundToken = (s: Status): ForegroundToken =>
  s === "ok" ? "fg-pos"
  : s === "warn" ? "fg-warn"
  : s === "error" ? "fg-neg"
  : s === "info" ? "fg-info"
  : "fg-secondary";

export const statusSurfaceToken = (s: Status): SurfaceToken =>
  s === "ok" ? "bg-pos-subtle"
  : s === "warn" ? "bg-warn-subtle"
  : s === "error" ? "bg-neg-subtle"
  : s === "info" ? "bg-info-subtle"
  : "bg-neutral-subtle";

/* -------------------------------------------------------- theme application */

const STORAGE_THEME = "meridian.theme";
const STORAGE_DENSITY = "meridian.density";

export function resolveInitialTheme(): Theme {
  if (typeof window === "undefined") return "paper";
  const saved = window.localStorage.getItem(STORAGE_THEME);
  if (saved === "paper" || saved === "terminal") return saved;
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches
    ? "terminal"
    : "paper";
}

export function applyTheme(theme: Theme, density?: Density): void {
  const el = document.documentElement;
  el.setAttribute("data-theme", theme);
  el.setAttribute("data-density", density ?? DEFAULT_DENSITY[theme]);
  try {
    window.localStorage.setItem(STORAGE_THEME, theme);
    if (density) window.localStorage.setItem(STORAGE_DENSITY, density);
  } catch {
    /* private mode — theme still applies for this session */
  }
}

/**
 * Inline this (minified) in <head> BEFORE any stylesheet to prevent a flash of
 * the wrong theme on first paint:
 *
 * <script>(function(){var t=localStorage.getItem('meridian.theme');
 * if(t!=='paper'&&t!=='terminal'){t=matchMedia('(prefers-color-scheme: dark)').matches?'terminal':'paper'}
 * var d=localStorage.getItem('meridian.density')||(t==='terminal'?'compact':'comfortable');
 * var e=document.documentElement;e.setAttribute('data-theme',t);e.setAttribute('data-density',d)})()</script>
 */
export const PREPAINT_SCRIPT = `(function(){var t=localStorage.getItem('${STORAGE_THEME}');if(t!=='paper'&&t!=='terminal'){t=matchMedia('(prefers-color-scheme: dark)').matches?'terminal':'paper'}var d=localStorage.getItem('${STORAGE_DENSITY}')||(t==='terminal'?'compact':'comfortable');var e=document.documentElement;e.setAttribute('data-theme',t);e.setAttribute('data-density',d)})()`;
