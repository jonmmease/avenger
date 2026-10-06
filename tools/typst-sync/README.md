# typst-sync

Keeps `avenger-typst-label`'s vendored Typst sources an exact, checkable subset of upstream.

A generated file is the pinned upstream file minus the whole items that
[`manifest.toml`](manifest.toml) lists, with crate paths rewritten (`crate::` to
`crate::typst_syntax::`, and so on) and doc examples marked `ignore`. The examples are written
against upstream's crates, and rustdoc tests every example in a crate, private items included.
Nothing else may differ. Generated files keep upstream's
formatting: their `mod` declarations carry `#[rustfmt::skip]`, and each generated directory has
a `rustfmt.toml` that disables formatting for editors.

```sh
tools/typst-sync/check.sh                      # crates.io packages, sha256-checked and cached
TYPST_DIR=../typst tools/typst-sync/check.sh   # a Typst checkout, read at the pinned rev
```

CI runs the first form. It fails on any hand edit, and on files in a generated directory that
the manifest doesn't list.

## Moving to a new upstream release

1. Update `[upstream]` in `manifest.toml`: `rev`, `version`, and the packages' `sha256`.
2. Regenerate:
   `python3 tools/typst-sync/sync.py generate --typst <sources> --crate avenger-typst-label`.
   A selector that no longer matches exactly one item fails the run, so renamed or removed
   upstream items show up here. Fix the manifest and rerun.
3. Build, test, and review the diff of the generated files.

`rust_items.py` is the item model: a line and brace scanner for rustfmt-formatted Rust, which
names each item by its header (`fn name`, `impl Trait for Type`, `impl Type :: fn method`,
`node! Name`, `use path;`).
