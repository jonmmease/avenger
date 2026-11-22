// `mount`: the handle of a chart in a container, with the container's loading and error states,
// its canvas, and the observers that load and release the chart as it nears and leaves the screen.

import { Handle, type Host, type View } from "./handle.js";
import { load } from "./loader.js";
import type { ChartHandle, ChartSource, ChartState, MountOptions } from "./types.js";

/** How long a chart loads before its container shows the loading state. */
const LOADING_DELAY_MS = 150;

const STATUS = "avenger-chart-status";

/** The wrapper's styles, of zero specificity, so that a page's own rules win. */
const STYLES = `
:where([data-avenger-chart]) { display: block; position: relative; height: 300px; }
:where([data-avenger-chart]) > canvas { display: block; width: 100%; height: 100%; }
:where([data-avenger-chart]) > .${STATUS} {
  position: absolute; inset: 0; display: flex; align-items: center; justify-content: center;
  border-radius: 4px; pointer-events: none;
  font: 12px/1.4 system-ui, -apple-system, "Segoe UI", sans-serif;
}
:where([data-avenger-chart]) > .${STATUS}::before {
  content: ""; position: absolute; inset: 0; border-radius: inherit;
  background: currentColor; opacity: 0;
}
:where([data-avenger-chart]) > .${STATUS}[data-visible]::before {
  opacity: 0.06; animation: avenger-chart-pulse 1.6s ease-in-out infinite alternate;
}
:where([data-avenger-chart]) > .${STATUS}[data-error]::before { opacity: 0.05; animation: none; }
:where([data-avenger-chart]) > .${STATUS} > span { position: relative; opacity: 0.6; }
@keyframes avenger-chart-pulse { from { opacity: 0.04; } to { opacity: 0.11; } }
@media (prefers-reduced-motion: reduce) {
  :where([data-avenger-chart]) > .${STATUS}[data-visible]::before { animation: none; opacity: 0.07; }
}
`;

let styled = false;

function addStyles(): void {
  if (styled) return;
  styled = true;
  const style = document.createElement("style");
  style.dataset.avengerChartStyles = "";
  style.textContent = STYLES;
  document.head.prepend(style);
}

/** The text of the error state for `error`. */
function errorText(error: Error): string {
  return /WebGPU/.test(error.message) ? "This chart needs WebGPU" : "This chart did not load";
}

function frame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

/** The host of a chart in `container`. */
function host(container: HTMLElement, options: MountOptions): Host {
  let status: HTMLElement | undefined;
  let delay: ReturnType<typeof setTimeout> | undefined;
  const removeStatus = () => {
    clearTimeout(delay);
    status?.remove();
    status = undefined;
  };
  const showStatus = (): HTMLElement => {
    if (!status) {
      status = document.createElement("div");
      status.className = STATUS;
      container.append(status);
    }
    return status;
  };
  return {
    load,
    async fetch(url, signal) {
      const response = await fetch(url, { signal });
      if (!response.ok) {
        throw new Error(`avenger-chart-web: cannot load ${url}: status ${response.status}`);
      }
      return new Uint8Array(await response.arrayBuffer());
    },
    async create(bytes, stylesheet) {
      const core = await load();
      const canvas = document.createElement("canvas");
      if (options.label !== undefined) {
        canvas.setAttribute("role", "img");
        canvas.setAttribute("aria-label", options.label);
      }
      // The view measures its canvas, which stays hidden until the view's first frame.
      canvas.style.visibility = "hidden";
      container.prepend(canvas);
      let view;
      try {
        view = await core.ChartView.create(canvas, bytes, {
          stylesheet: stylesheet ?? undefined,
          wheel: options.wheel,
        });
      } catch (error) {
        canvas.remove();
        throw error;
      }
      await frame();
      await frame();
      canvas.style.visibility = "";
      const created = view;
      const wrapped: View = {
        get diagnostics() {
          return created.diagnostics;
        },
        setChart: (next) => created.setChart(next),
        setStylesheet: (next) => created.setStylesheet(next),
        destroy() {
          created.destroy();
          created.free();
          canvas.remove();
        },
      };
      return wrapped;
    },
    show(state: ChartState | "waiting", error?: Error) {
      container.dataset.avengerChart = state === "waiting" ? "idle" : state;
      if (state === "loading" || state === "waiting") {
        const element = showStatus();
        element.removeAttribute("data-error");
        element.replaceChildren();
        if (!element.hasAttribute("data-visible") && delay === undefined) {
          delay = setTimeout(() => status?.setAttribute("data-visible", ""), LOADING_DELAY_MS);
        }
      } else if (state === "error") {
        const element = showStatus();
        clearTimeout(delay);
        element.removeAttribute("data-visible");
        element.setAttribute("data-error", "");
        const text = document.createElement("span");
        text.textContent = errorText(error ?? new Error(""));
        element.replaceChildren(text);
      } else {
        removeStatus();
        delay = undefined;
      }
    },
    report: options.onError ?? ((error) => console.error(error)),
    clear() {
      removeStatus();
      container.querySelector(":scope > canvas")?.remove();
      delete container.dataset.avengerChart;
    },
  };
}

/** The observers of each margin: one that reports approach, and one that reports release. */
const observers = new Map<string, { near: IntersectionObserver; far: IntersectionObserver }>();
const handles = new WeakMap<Element, { handle: Handle; release: boolean }>();

/** Three times `margin` for a margin in pixels, or `margin` itself. */
function farMargin(margin: string): string {
  return margin.replace(/(-?\d*\.?\d+)px/g, (_, value) => `${Number(value) * 3}px`);
}

function observersOf(margin: string) {
  let found = observers.get(margin);
  if (!found) {
    const near = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) handles.get(entry.target)?.handle.approach();
        }
      },
      { rootMargin: margin },
    );
    const far = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          const mounted = handles.get(entry.target);
          if (!entry.isIntersecting && mounted?.release) mounted.handle.release();
        }
      },
      { rootMargin: farMargin(margin) },
    );
    found = { near, far };
    observers.set(margin, found);
  }
  return found;
}

/**
 * Mounts a chart in `container`, which sets its size. The handle takes `setChart` and
 * `setStylesheet` at any time, and the chart loads as its container nears the screen.
 */
export function mount(container: HTMLElement, source?: ChartSource, options: MountOptions = {}): ChartHandle {
  addStyles();
  const margin = options.margin ?? "200px";
  const release = (options.release ?? "offscreen") === "offscreen";
  const handle = new Handle(host(container, options), source, options.stylesheet ?? null);
  const { near, far } = observersOf(margin);
  handles.set(container, { handle, release });
  container.dataset.avengerChart = "idle";
  near.observe(container);
  far.observe(container);
  if (options.lazy === "eager") handle.approach();
  const destroy = handle.destroy.bind(handle);
  handle.destroy = () => {
    near.unobserve(container);
    far.unobserve(container);
    handles.delete(container);
    destroy();
  };
  return handle;
}
