# Shared font assets

`avenger-fonts` exposes Brotli-compressed font bytes as named constants, and
`decompress` to decompress them.

The assets include Lato Light, Regular, Italic, and Bold, DejaVu Sans Mono, and
Lete Sans Math Regular and Bold. Each font directory contains its license.

Callers choose default fonts, and cache and register the decompressed bytes.
The `avenger-typst-label` engine uses the fonts its caller gives it, and its
`bundled-fonts` feature registers this crate's fonts as the default sans-serif,
monospace and math families.
