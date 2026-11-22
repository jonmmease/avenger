// The shared load of the page: the WebAssembly module, which embeds its fonts, and the page's own
// fonts, once, when the first chart or `preload` needs them.

import type * as Core from "../wasm/avenger_chart_web.js";
import type { LoaderOptions } from "./types.js";

export type CoreModule = typeof Core;

let options: LoaderOptions = {};
let started: Promise<CoreModule> | undefined;

/** Sets where the shared load finds its assets. Throws once the load has started. */
export function configure(next: LoaderOptions): void {
  if (started) {
    throw new Error("avenger-chart-web: configure before the library loads");
  }
  options = { ...options, ...next };
}

/** Starts the shared load, and returns its promise, which a page can ignore. */
export function preload(): Promise<void> {
  const loaded = load().then(() => undefined);
  loaded.catch(() => undefined);
  return loaded;
}

/** The shared load, which the first call starts. */
export function load(): Promise<CoreModule> {
  if (!started) {
    started = start();
    started.catch(() => undefined);
  }
  return started;
}

async function start(): Promise<CoreModule> {
  const core = await import("../wasm/avenger_chart_web.js");
  await core.default(options.moduleUrl === undefined ? undefined : { module_or_path: options.moduleUrl });
  for (const url of options.extraFonts ?? []) {
    await core.loadFonts(String(url));
  }
  return core;
}
