// The extension end to end: a project in a temporary folder takes the extension by `q2 add` of
// this checkout's folder and the grammar of grammars/, and `q2 render` of the documents of
// tests/fixtures/, with `avenger` from AVENGER or PATH, writes the pages of their cells:
// report.qmd's figure, cross-reference, errors, and code, themed.qmd's chart in the light and the
// dark scheme of its themes, and interactive.qmd's charts with tools, which load the runtime of
// interactive charts once, and deck.qmd's chart alone on a slide, which revealjs's auto-stretch
// leaves with its element. Run with `node tests/e2e.mjs`.

import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";

const here = dirname(fileURLToPath(import.meta.url));
const project = mkdtempSync(join(tmpdir(), "avenger-quarto2-"));
const run = (program, args) =>
  execFileSync(program, args, { cwd: project, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });

run("q2", ["add", join(here, ".."), "--no-prompt"]);
cpSync(join(here, "..", "grammars"), join(project, "_quarto", "grammars"), { recursive: true });
cpSync(join(here, "fixtures", "report.qmd"), join(project, "report.qmd"));
cpSync(join(here, "fixtures", "themed.qmd"), join(project, "themed.qmd"));
cpSync(join(here, "fixtures", "interactive.qmd"), join(project, "interactive.qmd"));
cpSync(join(here, "fixtures", "deck.qmd"), join(project, "deck.qmd"));
run("q2", ["render", "report.qmd"]);
run("q2", ["render", "themed.qmd"]);
run("q2", ["render", "interactive.qmd"]);
run("q2", ["render", "deck.qmd"]);

const page = readFileSync(join(project, "report.html"), "utf8");
// The chart's figure, its caption, and the resolved cross-reference.
assert.match(page, /<img src="report_files\/figure-html\/[^"]+\.svg"/);
assert.match(page, /Figure(\s|&nbsp;|\u00a0)1/);
assert.match(page, /Cars by origin/);
assert.doesNotMatch(page, /quarto-unresolved-ref/);
// The failing cell, with `error: true`, shows its error, and `echo: false` hides its code.
assert.match(page, /Unrecognized name: nope/);
// The echoed code of each cell, a highlighted `<code>` of the class `avenger-sql`, as written and
// as text.
const blocks = [...page.matchAll(/<code class="[^"]*\bavenger-sql\b[^"]*">([\s\S]*?)<\/code>/g)].map(
  (m) => m[1],
);
const text = (html) => html.replace(/<[^>]+>/g, "");
assert.ok(!blocks.some((block) => text(block).includes("nope")), "the code of an `echo: false` cell");
// The grammar of avenger-sql highlights the echoed code.
assert.match(blocks[1], /class="hl-keyword"/);
assert.match(blocks[1], /class="hl-function"/);
const figures = readdirSync(join(project, "report_files", "figure-html"));
assert.equal(figures.filter((name) => name.endsWith(".svg")).length, 1);

// A document of a light and a dark theme: one figure, whose caption and cross-reference are its
// own, holds an image of each scheme, which the theme's toggle shows in its scheme.
const themed = readFileSync(join(project, "themed.html"), "utf8");
assert.match(themed, /quarto-color-scheme-toggle/);
const figure = themed.match(/<div id="fig-cars"[\s\S]*?<\/figure>/)?.[0] ?? "";
assert.match(figure, /<div class="light-content[^"]*">\s*<p><img src="themed_files\/figure-html\/[^"]+\.svg"/);
assert.match(figure, /<div class="dark-content[^"]*">\s*<p><img src="themed_files\/figure-html\/[^"]+\.svg"/);
assert.equal(figure.match(/<figcaption/g)?.length, 1);
assert.match(themed, /Figure(\s|&nbsp;|\u00a0)1/);
assert.doesNotMatch(themed, /quarto-unresolved-ref/);
// Charts with tools: each keeps its image and has an element after it, the figure keeps its one
// caption, and the page's header loads the runtime of the project's `_avenger/` once. A page
// without charts with tools loads nothing.
const interactive = readFileSync(join(project, "interactive.html"), "utf8");
assert.equal(interactive.match(/id="avenger-runtime"/g)?.length, 1);
assert.equal(interactive.match(/<avenger-chart /g)?.length, 2);
const interactiveFigure = interactive.match(/<div id="fig-cars"[\s\S]*?<\/figure>/)?.[0] ?? "";
assert.match(interactiveFigure, /<img src="interactive_files\/figure-html\/[^"]+\.svg"[\s\S]*<avenger-chart width="\d+" height="\d+">/);
assert.equal(interactiveFigure.match(/<figcaption/g)?.length, 1);
assert.doesNotMatch(interactive, /quarto-unresolved-ref|requirejs|jquery/i);
assert.deepEqual(readdirSync(join(project, "_avenger")), ["runtime"]);
const config = JSON.parse(interactive.match(/id="avenger-runtime">([\s\S]*?)<\/script>/)[1].replaceAll("<\\/", "</"));
assert.equal(config.runtime, "_avenger/runtime/");
assert.equal(config.version, readFileSync(join(project, "_avenger", "runtime", "HASH"), "utf8"));
assert.ok(readdirSync(join(project, "_avenger", "runtime")).includes("avenger_chart_web_bg.wasm.gz"));
// A page's image keeps no set size, so it shrinks to a narrow page as other images do.
assert.match(interactiveFigure, /<img src="interactive_files\/figure-html\/[^"]+\.svg"[^>]*class="img-fluid"/);
assert.doesNotMatch(page, /avenger-runtime/);
// A slide of one interactive chart keeps its element, since its image has a set size, which
// revealjs's auto-stretch leaves in its figure.
const deck = readFileSync(join(project, "deck.html"), "utf8");
assert.match(deck, /<avenger-chart width="\d+" height="\d+">/);
assert.doesNotMatch(deck, /r-stretch/);
console.log(`ok: ${project}`);
