// The handle's states and promises, with a host that records what the handle asks of the page.

import assert from "node:assert/strict";
import { test } from "node:test";

import { Handle, type Host, type View } from "../src/handle.js";
import type { ChartState, StyleDiagnostic } from "../src/types.js";

/** A promise that the test settles. */
function later<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

/** The bytes of a chart named `name`, which the fake view reads back. */
const chart = (name: string) => new TextEncoder().encode(name);
const nameOf = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

class FakeView implements View {
  charts: string[] = [];
  stylesheets: (string | null)[] = [];
  destroyed = false;
  constructor(bytes: Uint8Array, stylesheet: string | null) {
    this.charts.push(nameOf(bytes));
    this.stylesheets.push(stylesheet);
  }
  get diagnostics(): StyleDiagnostic[] {
    return diagnosticsOf(this.stylesheets.at(-1) ?? null);
  }
  setChart(bytes: Uint8Array) {
    if (nameOf(bytes) === "invalid") throw new Error("the bytes are not an avenger chart");
    this.charts.push(nameOf(bytes));
  }
  setStylesheet(stylesheet: string | null) {
    this.stylesheets.push(stylesheet);
    return diagnosticsOf(stylesheet);
  }
  destroy() {
    this.destroyed = true;
  }
}

function diagnosticsOf(stylesheet: string | null): StyleDiagnostic[] {
  return stylesheet?.includes("colr")
    ? [{ severity: "warning", message: "unknown property `colr`", line: 1, column: 1 }]
    : [];
}

class FakeHost implements Host {
  shared = later<void>();
  urls = new Map<string, ReturnType<typeof later<Uint8Array>>>();
  views: FakeView[] = [];
  shown: (ChartState | "waiting")[] = [];
  reported: Error[] = [];
  cleared = false;
  createFails = false;
  load() {
    return this.shared.promise;
  }
  fetch(url: string | URL) {
    const key = String(url);
    if (!this.urls.has(key)) this.urls.set(key, later());
    return this.urls.get(key)!.promise;
  }
  async create(bytes: Uint8Array, stylesheet: string | null) {
    if (this.createFails) throw new Error("This browser does not support WebGPU");
    const view = new FakeView(bytes, stylesheet);
    this.views.push(view);
    return view;
  }
  show(state: ChartState | "waiting") {
    this.shown.push(state);
  }
  report(error: Error) {
    this.reported.push(error);
  }
  clear() {
    this.cleared = true;
  }
}

test("a chart loads when it comes near, with the latest chart and stylesheet", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), "chart { color: red; }");
  const first = handle.setChart(chart("second"));
  const styles = handle.setStylesheet("plot { colr: blue; }");
  handle.setChart(chart("third"));
  assert.equal(handle.state, "idle");
  assert.equal(host.views.length, 0);
  handle.approach();
  assert.equal(handle.state, "loading");
  host.shared.resolve();
  await handle.ready;
  assert.equal(handle.state, "ready");
  assert.equal(host.views.length, 1);
  assert.deepEqual(host.views[0].charts, ["third"]);
  assert.deepEqual(host.views[0].stylesheets, ["plot { colr: blue; }"]);
  // A call that a later call replaced settles with the call that the view takes.
  await first;
  assert.equal((await styles).length, 1);
});

test("after ready, calls go to the view and keep it", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  host.shared.resolve();
  await handle.ready;
  await handle.setChart(chart("second"));
  assert.deepEqual(await handle.setStylesheet("plot { colr: x; }"), diagnosticsOf("colr"));
  assert.equal(host.views.length, 1);
  assert.deepEqual(host.views[0].charts, ["first", "second"]);
  await assert.rejects(handle.setChart(chart("invalid")), /not an avenger chart/);
  assert.equal(host.reported.length, 1);
  assert.equal(handle.state, "ready");
});

test("overlapping URL sources settle together with the latest", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  host.shared.resolve();
  await handle.ready;
  const a = handle.setChart("a.bin");
  const b = handle.setChart("b.bin");
  host.urls.get("b.bin")!.resolve(chart("b"));
  await b;
  await a;
  host.urls.get("a.bin")!.resolve(chart("a"));
  await tick();
  assert.deepEqual(host.views[0].charts, ["first", "b"]);
});

test("a chart without a source waits near the screen, and loads when its source comes", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, undefined, null);
  handle.approach();
  assert.equal(handle.state, "idle");
  assert.equal(host.shown.at(-1), "waiting");
  host.shared.resolve();
  handle.setChart(chart("late"));
  await handle.ready;
  assert.deepEqual(host.views[0].charts, ["late"]);
});

test("a failed fetch is an error that a new source retries", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, "missing.bin", null);
  handle.approach();
  host.shared.resolve();
  await tick();
  host.urls.get("missing.bin")!.reject(new Error("status 404"));
  await assert.rejects(handle.ready, /404/);
  assert.equal(handle.state, "error");
  assert.equal(host.shown.at(-1), "error");
  assert.equal(host.reported.length, 1);
  await handle.setChart(chart("found"));
  assert.equal(handle.state, "ready");
  assert.deepEqual(host.views[0].charts, ["found"]);
});

test("a failed shared load fails the chart and stays failed", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  host.shared.reject(new Error("cannot load fonts from brand.ttf"));
  await assert.rejects(handle.ready, /brand\.ttf/);
  await assert.rejects(handle.setChart(chart("second")), /brand\.ttf/);
  assert.equal(handle.state, "error");
  assert.equal(host.views.length, 0);
});

test("a view that fails to create is an error", async () => {
  const host = new FakeHost();
  host.createFails = true;
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  host.shared.resolve();
  await assert.rejects(handle.ready, /WebGPU/);
  assert.equal(handle.state, "error");
});

test("a released chart frees its view and comes back from its latest state", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  host.shared.resolve();
  await handle.ready;
  handle.release();
  assert.equal(handle.state, "idle");
  assert.ok(host.views[0].destroyed);
  handle.setChart(chart("second"));
  handle.setStylesheet("chart { color: red; }");
  assert.equal(host.views.length, 1);
  handle.approach();
  await tick();
  await tick();
  assert.equal(handle.state, "ready");
  assert.equal(host.views.length, 2);
  assert.deepEqual(host.views[1].charts, ["second"]);
  assert.deepEqual(host.views[1].stylesheets, ["chart { color: red; }"]);
});

test("destroy during a load rejects the promises with AbortError and creates no view", async () => {
  const host = new FakeHost();
  const handle = new Handle(host, chart("first"), null);
  handle.approach();
  const styles = handle.setStylesheet("chart { color: red; }");
  handle.destroy();
  host.shared.resolve();
  await tick();
  await assert.rejects(handle.ready, { name: "AbortError" });
  await assert.rejects(styles, { name: "AbortError" });
  await assert.rejects(handle.setChart(chart("second")), { name: "AbortError" });
  assert.equal(handle.state, "destroyed");
  assert.equal(host.views.length, 0);
  assert.ok(host.cleared);
});

test("ignored promises that reject raise no unhandled rejection", async () => {
  const unhandled: unknown[] = [];
  const listener = (reason: unknown) => unhandled.push(reason);
  process.on("unhandledRejection", listener);
  try {
    const host = new FakeHost();
    const handle = new Handle(host, "missing.bin", null);
    handle.setStylesheet("chart { color: red; }");
    handle.approach();
    host.shared.resolve();
    await tick();
    host.urls.get("missing.bin")!.reject(new Error("status 404"));
    await tick();
    handle.setChart(chart("invalid"));
    handle.destroy();
    await new Promise((resolve) => setTimeout(resolve, 20));
  } finally {
    process.off("unhandledRejection", listener);
  }
  assert.deepEqual(unhandled, []);
});
