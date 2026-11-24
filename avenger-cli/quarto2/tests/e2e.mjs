// The extension end to end: a project in a temporary folder takes the extension by `q2 add` of
// this checkout's folder and the grammar of grammars/, and `q2 render` of the documents of
// tests/fixtures/, with `avenger` from AVENGER or PATH, writes the pages of their cells:
// report.qmd's figure, cross-reference, errors, and code, and themed.qmd's chart in the light and
// the dark scheme of its themes. Run with `node tests/e2e.mjs`.

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
run("q2", ["render", "report.qmd"]);
run("q2", ["render", "themed.qmd"]);

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
console.log(`ok: ${project}`);
