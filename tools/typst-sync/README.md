# typst-sync

Keeps `avenger-typst-label`'s vendored Typst sources an exact, checkable subset of upstream.

A generated file is the pinned upstream file minus the whole items that
[`manifest.toml`](manifest.toml) lists, with crate paths rewritten (`crate::` to
`crate::typst_syntax::`, and so on) and doc examples marked `ignore`, under a header that names
its source and says how it was modified. The examples are written against upstream's crates,
and rustdoc tests every example in a crate, private items included. Nothing else may differ. Generated files keep upstream's
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

## Ported files

`upstream_diff.py` compares the crate's hand-ported files, the ones whose first line is
`//! Ported from crates/<path> @ v0.15.1, modified for Avenger.`, with their upstream files. It
undoes what the port changes mechanically (headers and module notes, imports, crate paths, the
`elem!` and `func!` wrappers, doc-fence markers) and formats both sides with the crate's
rustfmt settings, so only real changes remain:

```sh
python3 tools/typst-sync/upstream_diff.py status         # per file: items kept, changed, removed, added
python3 tools/typst-sync/upstream_diff.py diff [FILE...]  # unified diffs from upstream
python3 tools/typst-sync/upstream_diff.py paths          # the upstream paths the crate mirrors
python3 tools/typst-sync/upstream_diff.py bump vNEW      # dry-run upstream's changes since the pin
```

`bump` ports the crate paths in upstream's diff, dry-runs it on each ported file with `patch`,
and reports the hunks that apply, that the file already has, and that fail where the port
changed the code. It reads a Typst checkout (`--typst`, default `../typst`), and `--from`
replaces the pinned revision as the start. `avenger-typst-label/UPSTREAM.md` describes the whole
procedure for a new release.

