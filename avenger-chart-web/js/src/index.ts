// avenger-chart-web: charts that load as they near the screen. The page imports this entry, a few
// kilobytes; the WebAssembly module and the fonts load once, when the first chart needs them.

export { configure, preload } from "./loader.js";
export { mount } from "./mount.js";
export type {
  ChartHandle,
  ChartSource,
  ChartState,
  LoaderOptions,
  MountOptions,
  StyleDiagnostic,
} from "./types.js";
