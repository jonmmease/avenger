#!/usr/bin/env bash
set -euo pipefail

manifest="tools/text-size-probe/Cargo.toml"
target_dir="target/text-size-probe-results"
binary_name="text-size-probe"

mkdir -p "$target_dir"

size_bytes() {
    if stat -f '%z' "$1" >/dev/null 2>&1; then
        stat -f '%z' "$1"
    else
        stat -c '%s' "$1"
    fi
}

measure_one() {
    local name="$1"
    local features="$2"
    local raw="$target_dir/$name"
    local stripped="$target_dir/$name.stripped"

    if [[ -n "$features" ]]; then
        cargo build --release --manifest-path "$manifest" --no-default-features --features "$features"
    else
        cargo build --release --manifest-path "$manifest" --no-default-features
    fi
    cp "tools/text-size-probe/target/release/$binary_name" "$raw"
    cp "$raw" "$stripped"
    strip "$stripped"

    printf '| `%s` | %s B | %s B |\n' "$name" "$(size_bytes "$raw")" "$(size_bytes "$stripped")"
}

printf '| Probe | Raw file size | Stripped size |\n'
printf '| --- | ---: | ---: |\n'
measure_one "baseline" ""
measure_one "typst" "typst"
