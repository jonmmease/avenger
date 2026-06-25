#!/usr/bin/env bash
set -euo pipefail

cargo run --release -p avenger-typst-vendor -- \
  --check-deny-list-only \
  --recipe tools/vendor-typst-math/recipe.toml \
  --out vendor/typst-avenger
