// The extension's execute with a fake `avenger`: the record of its cells by q2's unit ids, the
// check of the cells against the units, a document without cells, a q2 that doesn't write the
// cells, a failed request, and the image of each output format.

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { test } from "node:test";

import { colorSchemesFor, execute, imageFor, record } from "../_extensions/avenger/src/avenger-engine.ts";

const here = dirname(fileURLToPath(import.meta.url));
process.env.AVENGER = join(here, "fake-avenger.mjs");

/** The options of an execute of the Markdown `markdown`, with `units` and q2's offer. */
function options(markdown: string, units: unknown[], extra: Record<string, unknown> = {}) {
  return {
    target: { source: "report.qmd", input: "/project/report.qmd", markdown: { value: markdown } },
    format: { pandoc: { to: "html" }, execute: { error: false } },
    units,
    presentedBy: "core",
    executeEnv: {},
    ...extra,
  } as never;
}

const unit = (id: string, line: number, owned = true) =>
  ({ id, index: 0, kind: "cell", language: owned ? "avenger-sql" : "python", line, owned });

const svg = { output_type: "display_data", data: { "image/svg+xml": "<svg/>" }, metadata: {} };

test("the record holds each cell's outputs by its unit's id", async () => {
  const requested = join(mkdtempSync(join(tmpdir(), "avenger-")), "request.json");
  process.env.FAKE_REQUEST = requested;
  process.env.FAKE_STATUS = "0";
  process.env.FAKE_RESPONSE = JSON.stringify({
    protocol: 1,
    cells: [
      { number: 1, line: 5, outputs: [] },
      { number: 2, line: 9, outputs: [svg] },
    ],
  });
  const units = [unit("0-a-unlabeled", 5), unit("1-b-py", 7, false), unit("2-c-fig", 9)];
  const result = (await execute(options("# Text", units))) as unknown as {
    markdown: string;
    notebook: { cells: { metadata: { quarto_cell_id: string }; outputs: unknown[] }[] };
    recordInfo: { presentedBy: string; alignment: string };
  };
  assert.equal(result.markdown, "");
  assert.deepEqual(
    result.notebook.cells.map((cell) => [cell.metadata.quarto_cell_id, cell.outputs.length]),
    [["0-a-unlabeled", 0], ["2-c-fig", 1]],
  );
  assert.equal(result.recordInfo.presentedBy, "core");
  assert.equal(result.recordInfo.alignment, "ids");
  const request = JSON.parse(readFileSync(requested, "utf8"));
  assert.deepEqual(
    [request.protocol, request.document, request.text, request.image, request.error],
    [1, "/project/report.qmd", "# Text", { format: "svg", scale: 1 }, false],
  );
  assert.deepEqual(request.colorSchemes, ["light"]);
});

test("a theme of a dark and a light variant asks for light, then dark", async () => {
  const requested = join(mkdtempSync(join(tmpdir(), "avenger-")), "request.json");
  process.env.FAKE_REQUEST = requested;
  process.env.FAKE_STATUS = "0";
  process.env.FAKE_RESPONSE = JSON.stringify({ protocol: 1, cells: [{ number: 1, line: 5, outputs: [] }] });
  const format = {
    pandoc: { to: "html" },
    execute: { error: false },
    metadata: { theme: { dark: "darkly", light: "cosmo" } },
  };
  await execute(options("# Text", [unit("a", 5)], { format }));
  assert.deepEqual(JSON.parse(readFileSync(requested, "utf8")).colorSchemes, ["light", "dark"]);
});

test("a response of many chunks keeps its characters of several bytes", async () => {
  // 300 000 bytes of `é`, two bytes each, after a prefix of each parity, so that the end of some
  // chunk of the pipe, whose chunks have an even length, falls inside a character.
  for (const prefix of ["x", "xy"]) {
    const svg = `<svg>${prefix}` + "\u00e9".repeat(150_000) + "</svg>";
    const output = { output_type: "display_data", data: { "image/svg+xml": svg }, metadata: {} };
    const response = { protocol: 1, cells: [{ number: 1, line: 5, outputs: [output] }] };
    const file = join(mkdtempSync(join(tmpdir(), "avenger-")), "response.json");
    writeFileSync(file, JSON.stringify(response));
    process.env.FAKE_RESPONSE_FILE = file;
    process.env.FAKE_STATUS = "0";
    delete process.env.FAKE_REQUEST;
    try {
      const result = (await execute(options("# Text", [unit("a", 5)]))) as unknown as {
        notebook: { cells: { outputs: { data: Record<string, string> }[] }[] };
      };
      assert.equal(result.notebook.cells[0].outputs[0].data["image/svg+xml"], svg);
    } finally {
      delete process.env.FAKE_RESPONSE_FILE;
    }
  }
});

test("the cells must be the units, by count and by line", () => {
  assert.throws(
    () => record([unit("a", 5)], [{ number: 1, line: 6, outputs: [] }]),
    /q2's at lines 5, avenger's at lines 6/,
  );
  assert.throws(() => record([unit("a", 5)], []), /different cells/);
});

test("a document without avenger cells passes its Markdown through", async () => {
  const result = await execute(options("# Prose", [unit("p", 3, false)]));
  assert.equal(result.markdown, "# Prose");
});

test("a q2 that doesn't write the cells of a record is an error", async () => {
  await assert.rejects(
    execute(options("# Text", [unit("a", 5)], { presentedBy: undefined })),
    /needs a q2 that writes the cells/,
  );
});

test("a failed request fails the execute with avenger's error", async () => {
  process.env.FAKE_STATUS = "1";
  process.env.FAKE_RESPONSE = JSON.stringify({ protocol: 1, error: "report.qmd:6:1: error: nope" });
  await assert.rejects(execute(options("# Text", [unit("a", 5)])), /report.qmd:6:1: error: nope/);
  process.env.FAKE_RESPONSE = "not JSON";
  await assert.rejects(execute(options("# Text", [unit("a", 5)])), /avenger answered no response/);
});

test("a missing avenger names how to find it", async () => {
  const fake = process.env.AVENGER;
  process.env.AVENGER = join(here, "no-such-avenger");
  await assert.rejects(execute(options("# Text", [unit("a", 5)])), /`avenger` on PATH, or its path in AVENGER/);
  process.env.AVENGER = fake;
});

test("a theme and an output format pick the color schemes of the charts", () => {
  const both = { light: ["cosmo", "theme.scss"], dark: "darkly" };
  // Light, then dark, whatever the order of the theme's keys, which q2 doesn't keep.
  assert.deepEqual(colorSchemesFor(both, "html"), ["light", "dark"]);
  assert.deepEqual(colorSchemesFor({ dark: "darkly", light: "cosmo" }, "html5"), ["light", "dark"]);
  assert.deepEqual(colorSchemesFor(both, "dashboard"), ["light", "dark"]);
  // q2 adds its default light variant to a dark one alone, and toggles between them.
  assert.deepEqual(colorSchemesFor({ dark: "darkly" }, "html"), ["light", "dark"]);
  assert.deepEqual(colorSchemesFor({ light: "cosmo" }, "html"), ["light"]);
  // A theme's name says nothing of its scheme.
  assert.deepEqual(colorSchemesFor("darkly", "html"), ["light"]);
  assert.deepEqual(colorSchemesFor(undefined, "html"), ["light"]);
  // Formats without q2's toggle show one image.
  assert.deepEqual(colorSchemesFor(both, "revealjs"), ["light"]);
  assert.deepEqual(colorSchemesFor(both, "typst"), ["light"]);
});

test("an output format picks the image of cells without their own", () => {
  assert.deepEqual(imageFor("html"), { format: "svg", scale: 1 });
  assert.deepEqual(imageFor("revealjs"), { format: "svg", scale: 1 });
  assert.deepEqual(imageFor("gfm+yaml_metadata_block"), { format: "svg", scale: 1 });
  assert.deepEqual(imageFor("docx"), { format: "png", scale: 2 });
  assert.deepEqual(imageFor("pptx"), { format: "png", scale: 2 });
});
