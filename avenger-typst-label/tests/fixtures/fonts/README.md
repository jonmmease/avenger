# Audit font fixtures

These OFL-licensed fonts make the upstream comparison independent of system fonts.
The adjacent license files cover the original and modified fonts.

Noto Sans Hebrew and Noto Sans Devanagari come from google/fonts revision
`809e4d8b8d7e9364a914909bb777679606c178b8`, under `ofl/notosanshebrew` and
`ofl/notosansdevanagari`. Their uncompressed SHA-256 hashes are:

- Hebrew: `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88`
- Devanagari: `14ec4af41f27482216d1c2229f417ff9b1425e1babb014e57d1d40d03229853e`

The `Audit*` families are modified fixtures with distinct names:

- `AuditNoScriptMetrics`: bundled Lato Medium with its OS/2 table removed.
- `AuditScriptOffsets`: bundled Lato Medium with superscript X offset 400 and subscript X offset -200 font units.
- `AuditHebrewRegular`: Noto Sans Hebrew instantiated at weight 400 and width 100.

To rebuild the modified fixtures, install Python `fonttools` and `brotli`, then run
`python avenger-typst-label/tests/fixtures/fonts/rebuild.py` from the repository root.
The Rust tests and reference generator read the committed Brotli files directly.
No system emoji font is distributed here. Bitmap painter-order tests construct an
image in memory.
