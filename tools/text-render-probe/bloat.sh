#!/usr/bin/env bash
set -euo pipefail

binary_name="text-render-probe"
results_dir="target/text-render-probe-results"
bloat_target_dir="${BLOAT_TARGET_DIR:-target/text-render-probe-bloat}"

mkdir -p "$results_dir"

if ! cargo bloat --version >/dev/null 2>&1; then
    cat >&2 <<'EOF'
cargo-bloat is required for this probe.

Install it with:

    cargo install cargo-bloat
EOF
    exit 1
fi

cargo tree -p "$binary_name" --no-default-features --features cosmic \
    --prefix none --charset ascii >"$results_dir/cosmic-tree.txt"
cargo tree -p "$binary_name" --no-default-features --features typst \
    --prefix none --charset ascii >"$results_dir/typst-tree.txt"

cargo bloat --release -p "$binary_name" --bin "$binary_name" \
    --no-default-features --features cosmic \
    --target-dir "$bloat_target_dir" \
    --crates -n 0 --message-format json >"$results_dir/bloat-cosmic-crates.json"

cargo bloat --release -p "$binary_name" --bin "$binary_name" \
    --no-default-features --features typst \
    --target-dir "$bloat_target_dir" \
    --crates -n 0 --message-format json >"$results_dir/bloat-typst-crates.json"

python3 tools/text-render-probe/rank_typst_unique_bloat.py "$results_dir"

echo "Wrote:"
echo "  $results_dir/bloat-cosmic-crates.json"
echo "  $results_dir/bloat-typst-crates.json"
echo "  $results_dir/typst-unique-bloat.tsv"
