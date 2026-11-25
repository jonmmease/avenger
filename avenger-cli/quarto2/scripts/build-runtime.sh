#!/usr/bin/env bash
# Builds avenger-chart-web and writes the runtime that the CLI embeds into runtime/: the
# WebAssembly module as gzip, wasm-bindgen's glue, the `<avenger-chart>` element, and RUNTIME,
# the Git tree of avenger-chart-web, which survives a rebase, and the chart format's version, which
# the CLI's tests check against its checkout. Commit the changes of avenger-chart-web first. The
# files are one snapshot, so the glue, the element, and the module always match.
#
# Usage: scripts/build-runtime.sh
#
# Rebuild after a change of avenger-chart-web or of the chart format. The build needs wasm-pack
# and wasm-opt, as avenger-chart-web/scripts/build.sh finds them.
set -euo pipefail

quarto2="$(cd "$(dirname "$0")/.." && pwd)"
repo="$(cd "$quarto2/../.." && pwd)"
out="$quarto2/runtime"
dirty=$(git -C "$repo" status --porcelain -- avenger-chart-web | head -n 1)
if [[ -n "$dirty" ]]; then
  echo "build-runtime.sh: commit the changes of avenger-chart-web first" >&2
  exit 1
fi
build="$(mktemp -d)"
trap 'rm -rf "$build"' EXIT

"$repo/avenger-chart-web/scripts/build.sh" "$build"
gzip -9n -c "$build/avenger_chart_web_bg.wasm" >"$out/avenger_chart_web_bg.wasm.gz"
cp "$build/avenger_chart_web.js" "$out/avenger_chart_web.js"
cp "$repo/avenger-chart-web/element/avenger-chart.js" "$out/avenger-chart.js"
format=$(sed -n 's/^pub const FORMAT_VERSION: u32 = \([0-9]*\);/\1/p' "$repo/avenger-chart/src/codec.rs")
tree=$(git -C "$repo" rev-parse HEAD:avenger-chart-web)
printf 'avenger-chart-web %s\nformat %s\n' "$tree" "$format" >"$out/RUNTIME"
cat "$out/RUNTIME"
