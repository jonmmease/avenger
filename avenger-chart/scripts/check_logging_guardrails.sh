#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

# Test-only debug prints currently live in these files.
TEST_ONLY_PRINT_ALLOWLIST=(
  "src/lib.rs"
  "src/theme/color_mix.rs"
  "src/theme/theme.rs"
  "src/utils.rs"
)

EXCLUDE_GLOBS=()
for path in "${TEST_ONLY_PRINT_ALLOWLIST[@]}"; do
  EXCLUDE_GLOBS+=("--glob=!${path}")
done

eprintln_hits="$(rg -n 'eprintln!' src "${EXCLUDE_GLOBS[@]}" || true)"
if [[ -n "${eprintln_hits}" ]]; then
  echo "Found disallowed eprintln! callsites in avenger-chart/src:"
  echo "${eprintln_hits}"
  exit 1
fi

overlay_hits="$(rg -n 'AVENGER_CHART_DEBUG_LAYOUT' src || true)"
disallowed_overlay_hits="$(
  printf '%s\n' "${overlay_hits}" | rg -v 'src/facet/debug.rs|src/plot/compiled/rendering.rs' || true
)"
if [[ -n "${disallowed_overlay_hits}" ]]; then
  echo "Found AVENGER_CHART_DEBUG_LAYOUT usages outside overlay code paths:"
  echo "${disallowed_overlay_hits}"
  exit 1
fi

echo "Logging guardrails check passed."
