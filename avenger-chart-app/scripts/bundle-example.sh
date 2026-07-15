#!/usr/bin/env bash
set -euo pipefail

usage() {
    echo "Usage: $0 <example-name>" >&2
    echo "Example: $0 widget_region_cross_filter" >&2
}

if [[ $# -ne 1 ]]; then
    usage
    exit 2
fi

example_name=$1
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
crate_dir=$(cd -- "$script_dir/.." && pwd)
workspace_dir=$(cd -- "$crate_dir/.." && pwd)
example_source="$crate_dir/examples/$example_name.rs"

if [[ ! -f "$example_source" ]]; then
    echo "Unknown avenger-chart-app example: $example_name" >&2
    echo "Available examples:" >&2
    find "$crate_dir/examples" -maxdepth 1 -type f -name '*.rs' \
        -exec basename {} .rs \; | sort | sed 's/^/  /' >&2
    exit 2
fi

if ! cargo bundle --version >/dev/null 2>&1; then
    echo "cargo-bundle is required. Install it with:" >&2
    echo "  cargo install cargo-bundle --locked" >&2
    exit 1
fi

cd "$workspace_dir"

# cargo-bundle 0.11 fails while formatting warnings under the TERM=dumb and
# NO_COLOR environment used by some agent terminals. Force a capable terminal
# and generate only the .app bundle to avoid a duplicate app inside a DMG.
bundle_log=$(mktemp)
trap 'rm -f "$bundle_log"' EXIT
env -u NO_COLOR TERM=xterm-256color \
    cargo bundle \
    -p avenger-chart-app \
    --example "$example_name" \
    --release \
    --features winit-wgpu \
    --format osx 2>&1 | tee "$bundle_log"

app_path=$(sed -nE 's|^[[:space:]]*(/.*\.app)[[:space:]]*$|\1|p' "$bundle_log" | tail -n 1)

if [[ -z "$app_path" || ! -x "$app_path/Contents/MacOS/$example_name" ]]; then
    echo "Bundle succeeded, but its app path could not be determined." >&2
    exit 1
fi

echo
echo "Computer Use app path: $app_path"
