// The handle of a chart: its latest chart and stylesheet, its state, its view, and the promises
// that it returns. The handle reaches the page only through its `Host`, so tests run it in Node.

import type { ChartHandle, ChartSource, ChartState, StyleDiagnostic } from "./types.js";

/** A chart's view, which the host creates in the chart's container. */
export interface View {
  readonly diagnostics: StyleDiagnostic[];
  /** Throws the reader's error for bytes that are not a chart. */
  setChart(bytes: Uint8Array): void;
  setStylesheet(stylesheet: string | null): StyleDiagnostic[];
  destroy(): void;
}

/** What the handle does in the page. */
export interface Host {
  /** The shared load of the library and its fonts. */
  load(): Promise<unknown>;
  /** The bytes at `url`. */
  fetch(url: string | URL, signal: AbortSignal): Promise<Uint8Array>;
  /** A view of `bytes` in the container, styled by `stylesheet`. */
  create(bytes: Uint8Array, stylesheet: string | null): Promise<View>;
  /** Shows `state` in the container, with the failure of the `error` state. */
  show(state: ChartState | "waiting", error?: Error): void;
  /** Reports a failure, by default with `console.error`. */
  report(error: Error): void;
  /** Removes what the host added to the container. */
  clear(): void;
}

/** A promise with its own settle functions, whose rejection never goes unhandled. */
class Deferred<T> {
  readonly promise: Promise<T>;
  resolve!: (value: T) => void;
  reject!: (error: unknown) => void;
  settled = false;

  constructor() {
    this.promise = new Promise<T>((resolve, reject) => {
      this.resolve = (value) => {
        this.settled = true;
        resolve(value);
      };
      this.reject = (error) => {
        this.settled = true;
        reject(error);
      };
    });
    this.promise.catch(() => undefined);
  }
}

function aborted(): DOMException {
  return new DOMException("the chart was destroyed", "AbortError");
}

function asError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}

export class Handle implements ChartHandle {
  private current: ChartState = "idle";
  private near = false;
  private source: ChartSource | undefined;
  private stylesheet: string | null;
  private view: View | undefined;
  /** The bytes of the latest source, once it has bytes, so a view made again needs no fetch. */
  private bytes: { source: ChartSource; bytes: Uint8Array } | undefined;
  /** Grows with each load and each `destroy`, so a load whose result comes late does nothing. */
  private generation = 0;
  private abort: AbortController | undefined;
  private readonly first = new Deferred<void>();
  private charts: Deferred<void>[] = [];
  private styles: Deferred<StyleDiagnostic[]>[] = [];

  constructor(
    private readonly host: Host,
    source: ChartSource | undefined,
    stylesheet: string | null,
  ) {
    this.source = source;
    this.stylesheet = stylesheet;
  }

  get state(): ChartState {
    return this.current;
  }

  get ready(): Promise<void> {
    return this.first.promise;
  }

  /** The container came near the screen. */
  approach(): void {
    if (this.current === "destroyed" || this.near) return;
    this.near = true;
    this.render();
    void this.load();
  }

  /** The container left the release margin: the chart frees its view and waits to load again. */
  release(): void {
    if (this.current === "destroyed" || !this.near) return;
    this.near = false;
    if (this.current === "loading") {
      this.generation++;
      this.abort?.abort();
    }
    this.view?.destroy();
    this.view = undefined;
    if (this.current !== "error") this.current = "idle";
    this.render();
  }

  setChart(source: ChartSource): Promise<void> {
    const call = new Deferred<void>();
    if (this.current === "destroyed") {
      call.reject(aborted());
      return call.promise;
    }
    this.source = source;
    this.charts.push(call);
    if (this.view) {
      void this.apply(this.view, source);
    } else {
      if (this.current === "error") this.current = "idle";
      this.render();
      void this.load();
    }
    return call.promise;
  }

  setStylesheet(stylesheet: string | null): Promise<StyleDiagnostic[]> {
    const call = new Deferred<StyleDiagnostic[]>();
    if (this.current === "destroyed") {
      call.reject(aborted());
      return call.promise;
    }
    this.stylesheet = stylesheet;
    if (this.view) {
      call.resolve(this.view.setStylesheet(stylesheet));
      this.settleStyles(this.view.diagnostics);
    } else {
      this.styles.push(call);
    }
    return call.promise;
  }

  destroy(): void {
    if (this.current === "destroyed") return;
    this.generation++;
    this.abort?.abort();
    this.view?.destroy();
    this.view = undefined;
    this.current = "destroyed";
    for (const call of [...this.charts, ...this.styles]) call.reject(aborted());
    this.charts = [];
    this.styles = [];
    if (!this.first.settled) this.first.reject(aborted());
    this.host.clear();
  }

  /** Makes the chart's view when the chart is near, has a source, and has no view or load. */
  private async load(): Promise<void> {
    if (!this.near || this.source === undefined || this.view || this.current === "loading") return;
    if (this.current === "destroyed" || this.current === "error") return;
    const generation = ++this.generation;
    const abort = new AbortController();
    this.abort = abort;
    this.current = "loading";
    this.render();
    try {
      await this.host.load();
      // The latest source and stylesheet when each step starts, so the view starts from them.
      let source = this.source;
      let bytes = await this.bytesOf(source, abort.signal);
      if (generation !== this.generation) return;
      while (source !== this.source) {
        source = this.source;
        bytes = await this.bytesOf(source, abort.signal);
        if (generation !== this.generation) return;
      }
      const stylesheet = this.stylesheet;
      const view = await this.host.create(bytes, stylesheet);
      if (generation !== this.generation) {
        view.destroy();
        return;
      }
      this.view = view;
      this.current = "ready";
      this.render();
      if (this.stylesheet !== stylesheet) view.setStylesheet(this.stylesheet);
      this.settleStyles(view.diagnostics);
      if (this.source !== source) {
        await this.apply(view, this.source!);
      } else {
        this.settleCharts();
      }
      if (!this.first.settled) this.first.resolve();
    } catch (error) {
      if (generation !== this.generation) return;
      this.fail(asError(error));
    }
  }

  /**
   * Gives the view the chart of `source`, and settles every pending call with its result. A source
   * that a later call replaces while its bytes load does nothing: the later call settles them all.
   */
  private async apply(view: View, source: ChartSource): Promise<void> {
    const generation = this.generation;
    let bytes: Uint8Array;
    try {
      bytes = await this.bytesOf(source, new AbortController().signal);
    } catch (error) {
      if (generation === this.generation && this.view === view && this.source === source) {
        this.failCharts(asError(error));
      }
      return;
    }
    if (generation !== this.generation || this.view !== view || this.source !== source) return;
    try {
      view.setChart(bytes);
    } catch (error) {
      this.failCharts(asError(error));
      return;
    }
    this.settleCharts();
  }

  /** Rejects the pending calls of `setChart` with `error`, and reports it. */
  private failCharts(error: Error): void {
    for (const call of this.charts) call.reject(error);
    this.charts = [];
    this.host.report(error);
  }

  private async bytesOf(source: ChartSource, signal: AbortSignal): Promise<Uint8Array> {
    if (this.bytes?.source === source) return this.bytes.bytes;
    let bytes: Uint8Array;
    if (source instanceof Uint8Array) {
      bytes = source;
    } else if (source instanceof ArrayBuffer) {
      bytes = new Uint8Array(source);
    } else {
      bytes = await this.host.fetch(source, signal);
    }
    this.bytes = { source, bytes };
    return bytes;
  }

  private settleCharts(): void {
    for (const call of this.charts) call.resolve();
    this.charts = [];
  }

  private settleStyles(diagnostics: StyleDiagnostic[]): void {
    for (const call of this.styles) call.resolve(diagnostics);
    this.styles = [];
  }

  private fail(error: Error): void {
    this.current = "error";
    this.abort = undefined;
    for (const call of [...this.charts, ...this.styles]) call.reject(error);
    this.charts = [];
    this.styles = [];
    if (!this.first.settled) this.first.reject(error);
    this.host.show("error", error);
    this.host.report(error);
  }

  private render(): void {
    if (this.current === "destroyed") return;
    // A chart near the screen that waits for its source shows the loading state too.
    const waiting = this.current === "idle" && this.near && this.source === undefined;
    this.host.show(waiting ? "waiting" : this.current);
  }
}
