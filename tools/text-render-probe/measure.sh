#!/usr/bin/env bash
set -euo pipefail

target_dir="target/text-render-probe-results"
cargo_target_dir="${CARGO_TARGET_DIR:-target}"
binary_name="text-render-probe"
mkdir -p "$target_dir"

size_bytes() {
    if stat -f '%z' "$1" >/dev/null 2>&1; then
        stat -f '%z' "$1"
    else
        stat -c '%s' "$1"
    fi
}

build_one() {
    local name="$1"
    local feature="$2"
    local binary="$target_dir/$binary_name-$name"
    local png="$target_dir/hello-$name.png"

    cargo build --release -p "$binary_name" --no-default-features --features "$feature"
    cp "$cargo_target_dir/release/$binary_name" "$binary"
    "$binary" "$png" >/dev/null
    printf '%s,%s,%s\n' "$name" "$(size_bytes "$binary")" "$(size_bytes "$png")"
}

assert_no_cosmic_text() {
    local feature="$1"
    local output="/tmp/text-render-probe-cosmic-tree-$feature.txt"

    if cargo tree -p "$binary_name" --no-default-features --features "$feature" -i cosmic-text >"$output" 2>&1; then
        echo "unexpected cosmic-text dependency in $feature probe" >&2
        cat "$output" >&2
        exit 1
    fi

    if ! grep -q 'did not match any packages' "$output"; then
        echo "could not confirm absence of cosmic-text in $feature probe" >&2
        cat "$output" >&2
        exit 1
    fi
}

printf 'backend,binary_bytes,png_bytes\n'
build_one cosmic cosmic
build_one typst typst

assert_no_cosmic_text typst
