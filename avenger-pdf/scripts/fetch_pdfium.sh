#!/usr/bin/env bash
set -euo pipefail

# Pin the rasterizer so local and CI image comparisons use the same PDF implementation.
case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    artifact=pdfium-mac-arm64.tgz
    digest=9acf49e46c68992cd40810e88264b1ad171805d02fd41c4cca336aad6653b333
    library=libpdfium.dylib
    ;;
  Linux-x86_64)
    artifact=pdfium-linux-x64.tgz
    digest=e3f0c66b2daad710cb6c8edd4a8c45c8902995e359dc0775917fc16e2e56349d
    library=libpdfium.so
    ;;
  *)
    echo 'This helper supports macOS ARM64 and Linux x86_64.' >&2
    exit 1
    ;;
esac
output_dir="${1:-target/pdfium}"
archive="$(mktemp)"
trap 'rm -f "$archive"' EXIT
curl -fsSL "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium/7763/$artifact" -o "$archive"
printf '%s  %s\n' "$digest" "$archive" | shasum -a 256 --check
mkdir -p "$output_dir"
tar -xzf "$archive" -C "$output_dir"
printf 'PDFium library: %s/lib/%s\n' "$(cd "$output_dir" && pwd)" "$library"
