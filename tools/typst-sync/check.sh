#!/usr/bin/env bash
# Checks that avenger-typst-label's vendored Typst sources equal the pinned
# upstream sources minus the items listed in manifest.toml.
#
#   tools/typst-sync/check.sh                       # crates.io packages, sha256-checked and cached
#   TYPST_DIR=../typst tools/typst-sync/check.sh    # a Typst git checkout, read at the pinned rev
#
# Needs python3 (3.11 or later), curl and shasum.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
crate="${CRATE_DIR:-$here/../../avenger-typst-label}"
manifest="$here/manifest.toml"
field() {
  python3 -c 'import sys, tomllib; v = tomllib.load(open(sys.argv[1], "rb"))["upstream"]
for k in sys.argv[2:]: v = v[k]
print(v)' "$manifest" "$@"
}

if [[ -n "${TYPST_DIR:-}" ]]; then
  src="$TYPST_DIR"
else
  version="$(field version)"
  src="${TYPST_SYNC_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/avenger-typst-sync}"
  mkdir -p "$src"
  for c in typst-syntax typst-utils; do
    file="$src/$c-$version.crate"
    [[ -f "$file" ]] || curl -sSfL -o "$file" "https://static.crates.io/crates/$c/$c-$version.crate"
    echo "$(field sha256 "$c")  $file" | shasum -a 256 -c --status - \
      || { echo "checksum mismatch for $file" >&2; exit 2; }
    [[ -d "$src/$c-$version" ]] || tar -xzf "$file" -C "$src"
  done
fi
exec python3 "$here/sync.py" check --typst "$src" --crate "$crate" --manifest "$manifest"
