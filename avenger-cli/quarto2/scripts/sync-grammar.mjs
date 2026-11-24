// Copies tree-sitter-avenger-sql's grammar into grammars/avenger-sql/, as q2 loads a grammar from
// a project's `_quarto/grammars/`: the wasm as `avenger-sql.wasm`, and `highlights.scm` with the
// names of q2's palette, and writes UPSTREAM, the grammar's commit and the sha256 of each source.
//
//   node scripts/sync-grammar.mjs --grammar-root PATH [--wasm PATH] [--check]
//
// The wasm is the grammar's build, `node scripts/build_wasm.mjs` in its checkout, which writes
// `target/tree-sitter-avenger-sql.wasm`. `--check` compares the files that a sync would write
// with the committed ones, and fails when they differ.
//
// q2 names a grammar's function after its file, `tree_sitter_avenger-sql`, which no C grammar
// can export, so the sync adds that export to the wasm, for the grammar's function
// `tree_sitter_avenger_sql`.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const out = resolve(here, "..", "grammars", "avenger-sql");

/** The export of the grammar's function, and the name that q2 looks for. */
const FUNCTION = "tree_sitter_avenger_sql";
const ALIAS = "tree_sitter_avenger-sql";

/** The captures of the query that q2's palette names otherwise, each with its count. */
const CAPTURES = [
  ["@variable.special", "@variable.builtin", 2],
  ["@comment.doc", "@comment.documentation", 1],
];

/** The value of the option `name`, or `fallback`. */
function option(name, fallback) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : fallback;
}

/** An unsigned LEB128 number of `bytes` at `at`, and the offset after it. */
function readLeb(bytes, at) {
  let value = 0;
  let shift = 0;
  for (;;) {
    const byte = bytes[at++];
    value += (byte & 0x7f) * 2 ** shift;
    shift += 7;
    if (byte < 0x80) return [value, at];
  }
}

/** `value` as unsigned LEB128. */
function leb(value) {
  const out = [];
  do {
    let byte = value & 0x7f;
    value = Math.floor(value / 128);
    if (value > 0) byte |= 0x80;
    out.push(byte);
  } while (value > 0);
  return Buffer.from(out);
}

/** The wasm module `wasm` with an export `alias` of the export `name`, of the same kind. */
export function withAlias(wasm, name, alias) {
  if (!wasm.subarray(0, 8).equals(Buffer.from([0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]))) {
    throw new Error("not a wasm module of version 1");
  }
  const parts = [wasm.subarray(0, 8)];
  let at = 8;
  let found = false;
  while (at < wasm.length) {
    const id = wasm[at];
    const [size, start] = readLeb(wasm, at + 1);
    let body = wasm.subarray(start, start + size);
    if (id === 7) {
      const [count, first] = readLeb(body, 0);
      const entries = [];
      let target;
      let k = first;
      for (let index = 0; index < count; index++) {
        const [length, nameAt] = readLeb(body, k);
        const exported = body.subarray(nameAt, nameAt + length).toString("utf8");
        const kind = body[nameAt + length];
        const [, end] = readLeb(body, nameAt + length + 1);
        if (exported === alias) throw new Error(`the wasm already exports ${alias}`);
        if (exported === name) target = body.subarray(nameAt + length, end);
        entries.push(body.subarray(k, end));
        k = end;
      }
      if (target === undefined) throw new Error(`the wasm exports no ${name}`);
      const aliasName = Buffer.from(alias, "utf8");
      entries.push(Buffer.concat([leb(aliasName.length), aliasName, target]));
      body = Buffer.concat([leb(count + 1), ...entries]);
      found = true;
    }
    parts.push(Buffer.from([id]), leb(body.length), body);
    at = start + size;
  }
  if (!found) throw new Error("the wasm has no export section");
  return Buffer.concat(parts);
}

/** The query `text` with the captures of q2's palette. */
function withCaptures(text) {
  for (const [from, to, count] of CAPTURES) {
    const pattern = new RegExp(`${from.replace(/\./g, "\\.")}(?![\\w.])`, "g");
    const found = text.match(pattern)?.length ?? 0;
    if (found !== count) {
      throw new Error(`the query has ${found} of ${from}, where the sync expects ${count}`);
    }
    text = text.replace(pattern, to);
  }
  return text;
}

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function main() {
  const root = option("--grammar-root");
  if (!root) {
    console.error("usage: node scripts/sync-grammar.mjs --grammar-root PATH [--wasm PATH] [--check]");
    process.exit(2);
  }
  const wasmPath = option("--wasm", join(root, "target", "tree-sitter-avenger-sql.wasm"));
  const queryPath = join(root, "queries", "highlights.scm");
  const wasm = readFileSync(wasmPath);
  const query = readFileSync(queryPath, "utf8");
  const commit = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
  const files = {
    "avenger-sql.wasm": withAlias(wasm, FUNCTION, ALIAS),
    "highlights.scm": Buffer.from(withCaptures(query), "utf8"),
    UPSTREAM: Buffer.from(
      [
        "avenger-vis/tree-sitter-avenger-sql",
        `commit ${commit}`,
        `${sha256(wasm)}  target/tree-sitter-avenger-sql.wasm`,
        `${sha256(Buffer.from(query, "utf8"))}  queries/highlights.scm`,
        "",
      ].join("\n"),
      "utf8",
    ),
  };
  if (process.argv.includes("--check")) {
    const stale = Object.entries(files).filter(([name, bytes]) => {
      try {
        return !readFileSync(join(out, name)).equals(bytes);
      } catch {
        return true;
      }
    });
    if (stale.length > 0) {
      console.error(`grammars/avenger-sql/ differs from a sync: ${stale.map(([name]) => name).join(", ")}`);
      process.exit(1);
    }
    console.log("grammars/avenger-sql/ is in sync");
    return;
  }
  mkdirSync(out, { recursive: true });
  for (const [name, bytes] of Object.entries(files)) {
    writeFileSync(join(out, name), bytes);
  }
  console.log(`grammars/avenger-sql/ synced from ${commit}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main();
}
