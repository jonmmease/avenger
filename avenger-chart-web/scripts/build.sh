#!/usr/bin/env bash
# Builds a package of avenger's WebAssembly libraries, avenger-chart-web or a crate that builds on
# it, such as avenger-query-web, with the release-size profile and `wasm-opt -Oz`, and prints the
# raw, gzip, and brotli sizes of its module and its glue. It sets no budget.
#
# Usage: scripts/build.sh [CRATE] [OUT_DIR]
#
# CRATE is the crate's directory in the repository, `avenger-chart-web` by default, and its files
# take its name, such as `avenger_query_web_bg.wasm`. OUT_DIR is the package directory, `pkg` in
# the crate by default. The script finds wasm-opt in WASM_OPT, on the PATH, or in the cache of
# wasm-pack. On macOS, a crate with C code builds for wasm32 with Homebrew's LLVM, which the
# script uses when it is installed.
set -euo pipefail

repo="$(cd "$(dirname "$0")/../.." && pwd)"
name="${1:-avenger-chart-web}"
crate="$repo/$name"
if [[ ! -f "$crate/Cargo.toml" ]]; then
  echo "build.sh: no crate $name in $repo" >&2
  exit 1
fi
out="$(mkdir -p "${2:-$crate/pkg}" && cd "${2:-$crate/pkg}" && pwd)"
stem="${name//-/_}"

llvm=/opt/homebrew/opt/llvm/bin
if [[ -x "$llvm/clang" ]]; then
  export CC_wasm32_unknown_unknown="$llvm/clang" AR_wasm32_unknown_unknown="$llvm/llvm-ar"
fi

# wasm-pack's own wasm-opt step runs without the features that the module uses, so the script
# runs wasm-opt itself.
wasm-pack build "$crate" --target web --profile release-size --no-opt --out-dir "$out"
wasm_opt="${WASM_OPT:-$(command -v wasm-opt || ls "$HOME"/Library/Caches/.wasm-pack/wasm-opt-*/bin/wasm-opt 2>/dev/null | head -n 1 || true)}"
if [[ -z "$wasm_opt" ]]; then
  echo "build.sh: no wasm-opt; set WASM_OPT" >&2
  exit 1
fi
module="$out/${stem}_bg.wasm"
"$wasm_opt" -Oz --all-features "$module" -o "$module"

size() { wc -c <"$1" | tr -d ' '; }
mb() { awk -v bytes="$1" 'BEGIN { printf "%.2f MB", bytes / 1e6 }'; }
printf '%-26s %10s %10s %10s\n' file raw gzip brotli
for file in "$module" "$out/${stem}.js"; do
  raw=$(size "$file")
  gz=$(gzip -9 -c "$file" | wc -c | tr -d ' ')
  br=$(brotli -q 11 -c "$file" | wc -c | tr -d ' ')
  printf '%-26s %10s %10s %10s\n' "$(basename "$file")" "$(mb "$raw")" "$(mb "$gz")" "$(mb "$br")"
done
