// The extension end to end: a project in a temporary folder takes the extension by `q2 add` of
// this checkout's folder and the grammar of grammars/, and `q2 render` of
// tests/fixtures/report.qmd, with `avenger` from AVENGER or PATH, writes the page of its cells.
// Run with `node tests/e2e.mjs`.

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
run("q2", ["render", "report.qmd"]);

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
console.log(`ok: ${project}`);
