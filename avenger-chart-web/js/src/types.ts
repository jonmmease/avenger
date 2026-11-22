// The types of the wrapper's interface.

import type { StyleDiagnostic } from "../wasm/avenger_chart_web.js";

export type { StyleDiagnostic };

/** The bytes of a chart message, or the URL of the bytes, which the wrapper fetches when the chart needs them. */
export type ChartSource = Uint8Array | ArrayBuffer | string | URL;

/**
 * The state of a chart: `idle` while it waits to load, `loading` while the library, its fonts, or the
 * chart's bytes load, `ready` while its view draws, `error` after a failure, and `destroyed` after
 * `destroy`.
 */
export type ChartState = "idle" | "loading" | "ready" | "error" | "destroyed";

/** Where the shared load finds the library's assets. */
export interface LoaderOptions {
  /** The WebAssembly module. By default, the package's module beside its glue. */
  moduleUrl?: string | URL;
  /**
   * The URLs of the page's own font files, TrueType, OpenType, or collections, which load after
   * the fonts that the module embeds, in order.
   */
  extraFonts?: Array<string | URL>;
}

/** The options of `mount`. */
export interface MountOptions {
  /** `"visible"` loads the chart as its container nears the screen, and `"eager"` loads it at once. */
  lazy?: "visible" | "eager";
  /** How near the screen a container is before its chart loads, as a root margin. `"200px"` by default. */
  margin?: string;
  /**
   * `"offscreen"`, the default, frees the chart's view when its container leaves three times
   * `margin`, and makes a new view from the chart's latest state when it comes back, at its
   * fitted domains. `"never"` keeps the view.
   */
  release?: "offscreen" | "never";
  /** CSS text that styles the chart after the base stylesheet. */
  stylesheet?: string | null;
  /** When wheel events reach the chart: while its canvas has the focus, the default, or always. */
  wheel?: "focus" | "always";
  /** The accessible name of the chart's canvas, which takes `role="img"`. */
  label?: string;
  /** The callback of the chart's failures. By default, each failure is logged with `console.error`. */
  onError?: (error: Error) => void;
}

/** A mounted chart. */
export interface ChartHandle {
  readonly state: ChartState;
  /** Resolves when the chart first becomes interactive, and rejects when it fails before that. */
  readonly ready: Promise<void>;
  /** Replaces the chart. Resolves when the view takes the chart, and rejects with the reader's error. */
  setChart(source: ChartSource): Promise<void>;
  /** Replaces the stylesheet, or returns to the base stylesheet for `null`. Resolves to its diagnostics. */
  setStylesheet(stylesheet: string | null): Promise<StyleDiagnostic[]>;
  /** Removes the chart from its container and ends the handle. */
  destroy(): void;
}
