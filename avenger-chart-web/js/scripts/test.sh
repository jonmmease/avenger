#!/usr/bin/env bash
# Writes the gallery's charts, which the tests of the bindings read, compiles the tests, and runs
# them.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run --release --quiet -p chart-web --bin build-charts -- "$PWD/test-output/site" >/dev/null
rm -rf test-output/build
npx tsc -p tsconfig.test.json
node --test test-output/build/test/
