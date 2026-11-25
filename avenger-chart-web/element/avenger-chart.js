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
// An element creates its view as it nears the screen and frees it as it leaves. Inside a Quarto
// figure or cell, it hides the chart's images once its canvas draws, and shows them again when
// the view goes away, so a page without WebGPU keeps its images.

const CHART_TYPE = "application/vnd.avenger.chart+base64";

/** The page's configuration, which `start` sets. */
let config;
/** The library, once it loads. */
let library;

/**
 * Loads the library and the fonts of `options`, and defines `<avenger-chart>`. A second call
 * returns the first one's promise. `options` holds:
 *
 * - `glue`: the URL of wasm-bindgen's module of the library, `avenger_chart_web.js`.
 * - `wasm`: the URL of the library's WebAssembly module, `.wasm`, or `.wasm.gz` as gzip.
 * - `fonts`: the URLs of the font files, the faces that `avenger-fonts` bundles first, in their
 *   order, then the page's own; a URL that ends in `.gz` is gzip.
 * - `styles`: the project's stylesheets, `{ entry, files }`, or none.
 * - `colorSchemes`: `["light"]`, `["dark"]`, or `["light", "dark"]`. With one scheme, a chart
 *   draws in it on its background. With two, it draws on a transparent canvas and follows the
 *   `quarto-light` and `quarto-dark` classes of `body`, as Quarto's light and dark themes switch.
 */
export function start(options) {
  if (!start.started) {
    config = { colorSchemes: ["light"], fonts: [], ...options };
    start.started = load().then(() => {
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

/** The elements that follow the page's scheme, and the observer of `body`'s classes. */
const followers = new Set();
let schemeObserver;

function follow(element) {
  followers.add(element);
  if (!schemeObserver) {
    schemeObserver = new MutationObserver(() => {
      for (const follower of followers) follower.schemeChanged();
    });
    schemeObserver.observe(document.body, { attributes: true, attributeFilter: ["class"] });
  }
}

/** The observers that create a view as its element nears the screen and free it as it leaves. */
const near = new IntersectionObserver(
  (entries) => entries.forEach((entry) => entry.isIntersecting && entry.target.approach()),
  { rootMargin: "200px" },
);
const far = new IntersectionObserver(
  (entries) => entries.forEach((entry) => !entry.isIntersecting && entry.target.leave()),
  { rootMargin: "600px" },
);

function frame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
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
    /** Whether the element is near the screen, so a view that finishes creating stays. */
    this.wanted = false;
    this.hiddenOutputs = [];
  }

  connectedCallback() {
    // Until its canvas draws, the element takes no room, and the chart's images show.
    this.style.display = "block";
    this.style.height = "0";
    this.style.overflow = "hidden";
    near.observe(this);
    far.observe(this);
  }

  disconnectedCallback() {
    near.unobserve(this);
    far.unobserve(this);
    this.leave();
  }

  size() {
    const width = Number(this.getAttribute("width")) || 300;
    const height = Number(this.getAttribute("height")) || 200;
    return [width, height];
  }

  approach() {
    this.wanted = true;
    if (this.view || this.creating) return;
    this.creating = this.create().finally(() => {
      this.creating = undefined;
    });
  }

  async create() {
    const script = this.querySelector(`script[type="${CHART_TYPE}"]`);
    if (!script) return;
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
      view = await library.ChartView.create(canvas, decodeBase64(script.textContent), {
        styles: config.styles ?? undefined,
        colorScheme: transparent ? pageScheme() : schemes[0],
        transparent,
      });
    } catch (error) {
      canvas.remove();
      console.warn("avenger-chart: the chart stays an image", error);
      return;
    }
    // The element left the screen, or the page, while its view was created.
    if (!this.wanted || !this.isConnected) {
      view.destroy();
      view.free();
      canvas.remove();
      return;
    }
    this.view = view;
    this.canvas = canvas;
    if (transparent) follow(this);
    // The view's first frame draws before the images give way to the canvas.
    await frame();
    await frame();
    if (this.view !== view) return;
    this.style.height = "";
    this.style.overflow = "";
    this.hideImages();
  }

  schemeChanged() {
    this.view?.setColorScheme(pageScheme());
  }

  leave() {
    this.wanted = false;
    if (!this.view) return;
    followers.delete(this);
    this.view.destroy();
    this.view.free();
    this.view = undefined;
    this.canvas?.remove();
    this.canvas = undefined;
    this.style.height = "0";
    this.style.overflow = "hidden";
    this.showImages();
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
