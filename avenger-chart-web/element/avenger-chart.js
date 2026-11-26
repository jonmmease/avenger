// The `<avenger-chart>` element: a chart that the WebAssembly library draws in a canvas, with the
// tools that the chart declares. Plain JavaScript, with no build step.
//
// A page calls `start(config)` once, which loads the library, adds the fonts, and defines the
// element, and every `<avenger-chart>` of the page then upgrades. An element holds its chart's
// bytes as base64 in an inner `<script type="application/vnd.avenger.chart+base64">`, and its
// `width` and `height` in CSS pixels, its chart's natural size, of which it takes less width on a
// narrower page:
//
//   <avenger-chart width="300" height="268">
//     <script type="application/vnd.avenger.chart+base64">…</script>
//   </avenger-chart>
//
// An element creates its view as it nears the screen and frees it as it leaves, and keeps the
// state of its selections and modes for its next view. Inside a Quarto figure or cell, it hides
// the chart's images once its canvas draws, and shows them again when the view goes away, so a
// page without WebGPU keeps its images.
//
// Once the element is defined, a page listens to the reader's changes as `avenger-change` events,
// whose `detail` is the view's change, `{ bytes, selections, modes, settled }`, and updates the
// element with `setChart(bytes, options)`, `setState(bytes, parts)`, `setMode(name, on)`,
// `clearSelection(name)`, and `resetViews()`, which apply at once or when the element's next view
// appears. `state(parts)` gives the bytes of the element's state: its selections, its modes, and
// its views. The `reports` attribute picks the changes that the events carry: `settled`, by default,
// the changes when a gesture ends, or `every`, such as each move of a brush's drag.
//
// An element can take its chart from a document's graph instead of its own bytes: `graph` names the
// graph's file, relative to `start`'s `graphs`, and `output` the graph's output whose chart it
// draws. The elements of one graph share one decoded graph and their selections: a change of one
// element's selections restyles the others at once, and when the graph reads the selection, as a
// chart filtered by `WHERE brush` does, the graph runs again and each element whose chart changed
// draws its new chart with its views and its brushes. Such an element reports every change, at most
// once in its tool's throttle during a gesture, unless its `reports` attribute says otherwise.
// Graphs run in avenger-query-web's library, whose module holds this library's exports too. With
// avenger-chart-web's own library, an element of a graph keeps its images, and the page warns once.

const CHART_TYPE = "application/vnd.avenger.chart+base64";

/** The page's configuration, which `start` sets. */
let config;
/** The library, once it loads. */
let library;
/** Whether the page warned that its library runs no graphs. */
let graphless = false;

/**
 * Loads the library and the fonts of `options`, and defines `<avenger-chart>`. A second call
 * returns the first one's promise. `options` holds:
 *
 * - `glue`: the URL of wasm-bindgen's module of the library, `avenger_chart_web.js`, or
 *   `avenger_query_web.js` for a page whose elements draw from graphs.
 * - `wasm`: the URL of the library's WebAssembly module, `.wasm`, or `.wasm.gz` as gzip.
 * - `fonts`: the URLs of the font files, the faces that `avenger-fonts` bundles first, in their
 *   order, then the page's own; a URL that ends in `.gz` is gzip.
 * - `styles`: the project's stylesheets, `{ entry, files }`, or none.
 * - `colorSchemes`: `["light"]`, `["dark"]`, or `["light", "dark"]`. With one scheme, a chart
 *   draws in it on its background. With two, it draws on a transparent canvas and follows the
 *   `quarto-light` and `quarto-dark` classes of `body`, as Quarto's light and dark themes switch.
 * - `graphs`: the URL of the folder of the graphs' files that elements name with `graph`, the
 *   page's own URL by default.
 */
export function start(options) {
  if (!start.started) {
    config = { colorSchemes: ["light"], fonts: [], ...options };
    start.started = load().then(() => {
      // With two schemes, the charts follow the scheme of the page.
      if (config.colorSchemes.length > 1) {
        new MutationObserver(() => connected.forEach((element) => element.schemeChanged())).observe(
          document.body,
          { attributes: true, attributeFilter: ["class"] },
        );
      }
      if (!customElements.get("avenger-chart")) customElements.define("avenger-chart", AvengerChart);
    });
    start.started.catch((error) => console.warn("avenger-chart: the library did not load", error));
  }
  return start.started;
}

/** The response of `url`, decompressed when its name ends in `.gz`. */
async function fetchBody(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`cannot load ${url}: status ${response.status}`);
  if (!new URL(url, document.baseURI).pathname.endsWith(".gz")) return response.body;
  return response.body.pipeThrough(new DecompressionStream("gzip"));
}

async function load() {
  const glue = await import(new URL(config.glue, document.baseURI).href);
  // The module compiles while it downloads and decompresses.
  const wasm = new Response(await fetchBody(config.wasm), {
    headers: { "Content-Type": "application/wasm" },
  });
  await glue.default({ module_or_path: wasm });
  const fonts = await Promise.all(
    config.fonts.map(async (url) => new Uint8Array(await new Response(await fetchBody(url)).arrayBuffer())),
  );
  for (const font of fonts) glue.addFonts(font);
  library = glue;
}

/** The scheme of the page: dark while `body` has `quarto-dark`. */
function pageScheme() {
  return document.body.classList.contains("quarto-dark") ? "dark" : "light";
}

/** The observers that mark an element near the screen as it nears it, and far as it leaves. */
const near = new IntersectionObserver(
  (entries) =>
    entries.forEach((entry) => {
      if (entry.isIntersecting) entry.target.setNear(true);
    }),
  { rootMargin: "200px" },
);
const far = new IntersectionObserver(
  (entries) =>
    entries.forEach((entry) => {
      if (!entry.isIntersecting) entry.target.setNear(false);
    }),
  { rootMargin: "600px" },
);

/** The connected elements, which a change of revealjs's slide or of the page's scheme updates. */
const connected = new Set();
// revealjs lays every slide out in one place and shows one, so an element of another slide counts
// as near the screen. A change of slide decides again which elements hold views.
document.addEventListener("slidechanged", () => connected.forEach((element) => element.update()));
document.addEventListener("ready", () => connected.forEach((element) => element.update()));

function frame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

/**
 * The graphs of the page, by the URLs of their files: each with its elements, its decoded graph and
 * the charts of its last run once it loads, and whether a run is in progress or due again.
 */
const graphs = new Map();

/** The graph that `element` names, which it joins, and which loads on first use. */
function graphOf(element) {
  const url = new URL(element.getAttribute("graph"), config.graphs ?? document.baseURI).href;
  let group = graphs.get(url);
  if (!group) {
    group = { elements: new Set(), graph: undefined, charts: [], running: false, again: false };
    group.loaded = (async () => {
      const response = await fetch(url);
      if (!response.ok) throw new Error(`cannot load ${url}: status ${response.status}`);
      group.graph = new library.ChartGraph(new Uint8Array(await response.arrayBuffer()));
      group.charts = await group.graph.run();
    })();
    graphs.set(url, group);
  }
  group.elements.add(element);
  return group;
}

/**
 * Gives a change of `from`'s selections to the other elements of its graph, which restyle by them,
 * and runs the graph again when the change writes a selection that the graph reads.
 */
function changed(group, from, change) {
  for (const other of group.elements) {
    if (other !== from) other.setState(change.bytes, { modes: false, views: false });
  }
  if (change.selections.length && group.graph.change(change.bytes)) rerun(group);
}

/** Runs `group`'s graph, once more for each change that arrives meanwhile, and gives each element whose chart changed its new chart. */
async function rerun(group) {
  if (group.running) {
    group.again = true;
    return;
  }
  group.running = true;
  try {
    do {
      group.again = false;
      const charts = await group.graph.run();
      for (const element of group.elements) {
        const index = Number(element.getAttribute("output") ?? 0);
        const bytes = charts[index];
        if (bytes && !same(bytes, group.charts[index])) element.setChart(bytes);
      }
      group.charts = charts;
    } while (group.again);
  } catch (error) {
    console.warn("avenger-chart: the graph did not run", error);
  } finally {
    group.running = false;
  }
}

/** Whether the byte arrays `a` and `b` hold the same bytes. */
function same(a, b) {
  if (!a || !b || a.length !== b.length) return false;
  for (let at = 0; at < a.length; at++) if (a[at] !== b[at]) return false;
  return true;
}

/** Applies a call of `setState`, `setMode`, `clearSelection`, or `resetViews` to `view`. */
function apply(view, call) {
  if (call.state) view.setState(call.state, call.parts);
  else if (call.mode !== undefined) view.setMode(call.mode, call.on);
  else if (call.clear !== undefined) view.clearSelection(call.clear);
  else if (call.reset) view.resetViews();
}

/** The base64 text of `bytes`. */
function encodeBase64(bytes) {
  if (bytes.toBase64) return bytes.toBase64();
  let text = "";
  for (let at = 0; at < bytes.length; at += 0x8000) {
    text += String.fromCharCode(...bytes.subarray(at, at + 0x8000));
  }
  return btoa(text);
}

/** The bytes of the base64 text `text`. */
function decodeBase64(text) {
  const clean = text.replace(/\s+/g, "");
  if (Uint8Array.fromBase64) return Uint8Array.fromBase64(clean);
  return Uint8Array.from(atob(clean), (char) => char.charCodeAt(0));
}

class AvengerChart extends HTMLElement {
  constructor() {
    super();
    this.view = undefined;
    this.canvas = undefined;
    this.creating = undefined;
    /** Whether the element is near the screen, as the observers last said. */
    this.near = false;
    /** Whether the element wants a view: near the screen, and on the present slide of a deck. */
    this.wanted = false;
    this.hiddenOutputs = [];
    /** The bytes of the state of the last view, which the next view applies. */
    this.saved = undefined;
    /**
     * The calls of `setState` and `setMode` since, in order, which the next view applies after it,
     * until a view has drawn and holds them in its state.
     */
    this.pending = [];
    /** Whether `setChart` replaced the chart, whose images then show another chart. */
    this.replaced = false;
    /** Whether the view drew its first frame, after it applied the saved state and the calls. */
    this.drawn = false;
    /** The graph that the element takes its chart from, once it joins it. */
    this.group = undefined;
  }

  connectedCallback() {
    // Until its canvas draws, the element takes no room, and the chart's images show.
    this.style.display = "block";
    this.style.height = "0";
    this.style.overflow = "hidden";
    connected.add(this);
    near.observe(this);
    far.observe(this);
  }

  disconnectedCallback() {
    connected.delete(this);
    this.group?.elements.delete(this);
    this.group = undefined;
    near.unobserve(this);
    far.unobserve(this);
    this.leave();
  }

  size() {
    const width = Number(this.getAttribute("width")) || 300;
    const height = Number(this.getAttribute("height")) || 200;
    return [width, height];
  }

  setNear(near) {
    this.near = near;
    this.update();
  }

  /** Whether the element is on the present slide of a revealjs deck, or in no deck. */
  shown() {
    const slide = this.closest(".reveal .slides section");
    return !slide || slide.classList.contains("present");
  }

  /** Creates the element's view when it is near the screen and shown, and frees it otherwise. */
  update() {
    if (this.near && this.shown()) this.approach();
    else this.leave();
  }

  approach() {
    this.wanted = true;
    if (this.view || this.creating) return;
    this.creating = this.create().finally(() => {
      this.creating = undefined;
    });
  }

  /** The bytes of the element's chart: its own, or the chart of its output of its graph. */
  async chartBytes() {
    const script = this.querySelector(`script[type="${CHART_TYPE}"]`);
    if (script) return decodeBase64(script.textContent);
    if (!this.hasAttribute("graph")) return undefined;
    if (!library.ChartGraph) {
      if (!graphless) {
        graphless = true;
        console.warn("avenger-chart: the library runs no graphs, so its graphs' charts stay images; load avenger-query-web");
      }
      return undefined;
    }
    this.group ??= graphOf(this);
    await this.group.loaded;
    return this.group.charts[Number(this.getAttribute("output") ?? 0)] ?? undefined;
  }

  async create() {
    let bytes;
    try {
      bytes = await this.chartBytes();
    } catch (error) {
      console.warn("avenger-chart: the chart stays an image", error);
      return;
    }
    if (!bytes) return;
    const schemes = config.colorSchemes;
    const transparent = schemes.length > 1;
    const canvas = document.createElement("canvas");
    const [width, height] = this.size();
    // The canvas takes the chart's width, or less on a narrower page, where the chart lays out
    // again at the width it gets, as its image would shrink.
    canvas.style.width = "100%";
    canvas.style.maxWidth = `${width}px`;
    canvas.style.height = `${height}px`;
    canvas.style.display = "block";
    this.append(canvas);
    let view;
    try {
      view = await library.ChartView.create(canvas, bytes, {
        styles: config.styles ?? undefined,
        colorScheme: transparent ? pageScheme() : schemes[0],
        transparent,
        reports: this.getAttribute("reports") ?? (this.group ? "every" : "settled"),
        onChange: (change) => {
          if (this.group?.graph) changed(this.group, this, change);
          this.dispatchEvent(new CustomEvent("avenger-change", { detail: change, bubbles: true }));
        },
      });
    } catch (error) {
      canvas.remove();
      console.warn("avenger-chart: the chart stays an image", error);
      return;
    }
    // The element left the screen, or the page, while its view was created.
    if (!this.wanted || !this.isConnected) {
      view.free();
      canvas.remove();
      return;
    }
    this.view = view;
    this.canvas = canvas;
    // The state of the last view, and the calls since, apply before the first frame shows.
    try {
      if (this.saved) view.setState(this.saved);
      for (const call of this.pending) apply(view, call);
    } catch (error) {
      console.warn("avenger-chart: the saved state did not apply", error);
    }
    // The view's first frame draws before the images give way to the canvas.
    await frame();
    await frame();
    if (this.view !== view) return;
    // The view has applied the saved state and the calls, which its state now holds.
    this.drawn = true;
    this.pending = [];
    this.style.height = "";
    this.style.overflow = "";
    this.hideImages();
  }

  /** Draws the element's chart in the page's scheme. */
  schemeChanged() {
    this.view?.setColorScheme(pageScheme());
  }

  leave() {
    this.wanted = false;
    if (!this.view) return;
    // A view that has not drawn may not have applied the saved state and the calls yet, so they
    // stay for the next view.
    if (this.drawn) {
      try {
        this.saved = this.view.state();
        this.pending = [];
      } catch (error) {
        console.warn("avenger-chart: the state did not save", error);
      }
    }
    this.drawn = false;
    this.view.free();
    this.view = undefined;
    this.canvas?.remove();
    this.canvas = undefined;
    if (this.replaced) {
      this.reserve();
    } else {
      this.style.height = "0";
      this.style.overflow = "hidden";
      this.showImages();
    }
  }

  /** Keeps the room of the element's chart while it has no view, in place of images of another chart. */
  reserve() {
    const [, height] = this.size();
    this.style.height = `${height}px`;
    this.style.overflow = "hidden";
  }

  /**
   * Replaces the element's chart with the chart message `bytes`, which its view draws now, with
   * `options` of `ChartView.setChart`, or its next view draws. The images of the first chart stay
   * hidden after it.
   */
  setChart(bytes, options) {
    // The view checks the bytes and the options first, so an error leaves the element as it was.
    this.view?.setChart(bytes, options);
    let script = this.querySelector(`script[type="${CHART_TYPE}"]`);
    if (!script) {
      script = document.createElement("script");
      script.type = CHART_TYPE;
      this.append(script);
    }
    script.textContent = encodeBase64(bytes);
    this.replaced = true;
    if (options?.selections === "reset") {
      this.saved = undefined;
      this.pending = this.pending.filter((call) => !call.state && call.clear === undefined);
    }
    if (!this.view) {
      this.hideImages();
      this.reserve();
    }
  }

  /**
   * The bytes of the state of the element's chart: its selections, the modes that are on, and its
   * views, or the parts of `parts`, as `ChartView.state` takes them. Without a view, it is the whole
   * state of the last view, without the calls since, or null before any view.
   */
  state(parts) {
    return this.view ? this.view.state(parts) : (this.saved ?? null);
  }

  /**
   * Applies a change message, such as `state()` or an event's `detail.bytes` of another chart, or
   * the parts of it that `parts` names, as `ChartView.setState` takes them.
   */
  setState(bytes, parts) {
    this.call({ state: bytes, parts });
  }

  /** Turns the chart's mode `name` on or off. */
  setMode(name, on) {
    this.call({ mode: name, on });
  }

  /** Clears the selection `name`. */
  clearSelection(name) {
    this.call({ clear: name });
  }

  /** Returns every scale of the chart to its fit. */
  resetViews() {
    this.call({ reset: true });
  }

  /** Applies `call` to the view, and keeps it for the next view until a view has drawn. */
  call(call) {
    if (this.view) apply(this.view, call);
    if (!this.drawn) this.pending.push(call);
  }

  /**
   * Hides the chart's images: each image of the element's figure, or of its Quarto cell, with the
   * wrapper that holds it, the image's ancestor just below the one that holds the element too, so
   * it hides the image's output whether q2 writes each output in its own `.cell-output-display`,
   * as an HTML page does, or a figure's outputs in one, as a dashboard does.
   */
  hideImages() {
    const scope = this.closest("figure") ?? this.closest(".cell");
    if (!scope) return;
    for (const image of scope.querySelectorAll("img")) {
      if (this.contains(image)) continue;
      let output = image;
      while (output.parentElement && !output.parentElement.contains(this)) output = output.parentElement;
      if (output.style.display === "none") continue;
      output.style.display = "none";
      this.hiddenOutputs.push(output);
    }
  }

  showImages() {
    for (const output of this.hiddenOutputs) output.style.display = "";
    this.hiddenOutputs = [];
  }
}
