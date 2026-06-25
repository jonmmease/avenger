#!/usr/bin/env bash
set -euo pipefail

target_dir="target/text-render-probe-results"
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
    cp "target/release/$binary_name" "$binary"
    "$binary" "$png" >/dev/null
    printf '%s,%s,%s\n' "$name" "$(size_bytes "$binary")" "$(size_bytes "$png")"
}

printf 'backend,binary_bytes,png_bytes\n'
build_one cosmic cosmic
build_one typst typst

if cargo tree -p "$binary_name" --no-default-features --features typst -i cosmic-text >/tmp/text-render-probe-cosmic-tree.txt 2>&1; then
    echo "unexpected cosmic-text dependency in typst probe" >&2
    cat /tmp/text-render-probe-cosmic-tree.txt >&2
    exit 1
fi

if ! grep -q 'did not match any packages' /tmp/text-render-probe-cosmic-tree.txt; then
    echo "could not confirm absence of cosmic-text in typst probe" >&2
    cat /tmp/text-render-probe-cosmic-tree.txt >&2
    exit 1
fi
