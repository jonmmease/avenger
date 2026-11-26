#!/usr/bin/env bash
# Builds avenger-query-web, which holds avenger-chart-web's library and runs the documents' graphs,
# and writes the runtime that the CLI embeds into runtime/: the WebAssembly module as gzip,
# wasm-bindgen's glue, avenger-chart-web's `<avenger-chart>` element, and RUNTIME, the Git trees of
# the libraries and of the crates that read the CLI's graphs, which survive a rebase, DataFusion's
# version, and the chart format's version, which the CLI's tests check against its checkout. Commit
# the changes of those crates first. The files are one snapshot, so the glue, the element, and the
# module always match.
#
# Usage: scripts/build-runtime.sh
#
# Rebuild after a change of one of those crates or of the chart format. The build needs wasm-pack
# and wasm-opt, as avenger-chart-web/scripts/build.sh finds them.
set -euo pipefail

quarto2="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$quarto2/../.." && pwd)"
out="$quarto2/runtime"
# The library, and the crates that read the graphs that the CLI writes, whose trees RUNTIME records.
trees=(avenger-chart-web avenger-query-web avenger-query-graph avenger-chart-datafusion avenger-selection-datafusion avenger-datafusion-shared avenger-datafusion-transforms avenger-sql avenger-chart)
dirty=$(git -C "$repo" status --porcelain -- "${trees[@]}" | head -n 1)
if [[ -n "$dirty" ]]; then
  echo "build-runtime.sh: commit the changes of ${trees[*]} first" >&2
  exit 1
fi
build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT

# The runtime runs the documents' graphs, so its charts filter by their selections without a server.
"$repo/avenger-chart-web/scripts/build.sh" avenger-query-web "$build"
gzip -9n -c "$build/avenger_query_web_bg.wasm" >"$out/avenger_query_web_bg.wasm.gz"
cp "$build/avenger_query_web.js" "$out/avenger_query_web.js"
cp "$repo/avenger-chart-web/element/avenger-chart.js" "$out/avenger-chart.js"
format=$(sed -n 's/^pub const FORMAT_VERSION: u32 = \([0-9]*\);/\1/p' "$repo/avenger-chart/src/codec.rs")
datafusion=$(awk '/^name = "datafusion"$/ { getline; gsub(/version = |"/, ""); print; exit }' "$repo/Cargo.lock")
: >"$out/RUNTIME"
for name in "${trees[@]}"; do
  printf '%s %s\n' "$name" "$(git -C "$repo" rev-parse "HEAD:$name")" >>"$out/RUNTIME"
done
printf 'datafusion %s\nformat %s\n' "$datafusion" "$format" >>"$out/RUNTIME"
cat "$out/RUNTIME"
