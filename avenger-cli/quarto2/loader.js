// The loader of the interactive charts of a Quarto 2 page, which `avenger quarto2-engine` puts in
// the page's header once, inline, after the page's configuration, a JSON script `avenger-runtime`:
// the URL of the runtime's folder relative to the page, the runtime's version, which each URL
// carries so a browser never reads a cached file of another build, the names of its fonts, the
// project's stylesheets, and the color schemes. A module script runs after the page parses, so the page
// never waits for it. When the first `<avenger-chart>` nears the screen, it imports the runtime's
// element once, which loads the library and the fonts and draws the charts.
const config = JSON.parse(document.getElementById("avenger-runtime").textContent);
const base = new URL(config.runtime, document.baseURI);
const url = (path) => {
  const href = new URL(path, base);
  href.searchParams.set("v", config.version);
  return href.href;
};
let started = false;
const observer = new IntersectionObserver(
  (entries) => {
    if (started || !entries.some((entry) => entry.isIntersecting)) return;
    started = true;
    observer.disconnect();
    import(url("avenger-chart.js"))
      .then(({ start }) =>
        start({
          glue: url("avenger_query_web.js"),
          wasm: url("avenger_query_web_bg.wasm.gz"),
          fonts: config.fonts.map((font) => url(`fonts/${font}.gz`)),
          styles: config.styles ?? undefined,
          colorSchemes: config.colorSchemes,
          // The graphs' files are named by their content, beside the runtime's folder.
          graphs: new URL("../graphs/", base).href,
        }),
      )
      .catch((error) => console.warn("avenger-chart: the runtime did not load", error));
  },
  { rootMargin: "200px" },
);
for (const element of document.querySelectorAll("avenger-chart")) observer.observe(element);
