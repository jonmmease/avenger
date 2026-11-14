// The bindings of the chart format over the charts of the gallery, which `build-charts` writes
// into `test-output/site` before the tests run.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { equals } from "@bufbuild/protobuf";
import { ChartSchema, decodeChart, encodeChart } from "../src/proto.js";

const site = new URL("../../site/", import.meta.url);
const manifest: { name: string; title: string }[] = JSON.parse(
  readFileSync(new URL("manifest.json", site), "utf8"),
);

test("each chart of the gallery decodes and encodes to an equal chart", () => {
  assert.equal(manifest.length, 6);
  for (const entry of manifest) {
    const bytes = readFileSync(new URL(`charts/${entry.name}.bin`, site));
    const chart = decodeChart(new Uint8Array(bytes));
    assert.equal(chart.version, 1, entry.name);
    const node = chart.root?.node;
    if (node?.case === "plot") {
      assert.equal(node.value.title?.text, entry.title, entry.name);
    }
    assert.ok(equals(ChartSchema, decodeChart(encodeChart(chart)), chart), entry.name);
  }
});
