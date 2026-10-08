# Shared font assets

`avenger-fonts` exposes Brotli-compressed font bytes as named constants, and
`decompress` to decompress them. Callers register the fonts they need.

The assets include Lato Light, Regular, Italic, and Bold, DejaVu Sans Mono, and
Lete Sans Math Regular and Bold. Each font directory contains its license.

Consumers choose default fonts and manage decompression, caching, and
registration. The standalone `avenger-typst-label` engine accepts caller-provided
fonts and uses this crate as a development dependency for tests and examples.
