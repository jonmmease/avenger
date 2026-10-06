# typst-upstream

Compares `avenger-typst-label`'s hand-ported files with the upstream Typst files they come from.

A ported file names its source on its first line:
`//! Ported from crates/<path> @ v0.15.1, modified for Avenger.` `upstream_diff.py` undoes what
the port changes mechanically (headers and module notes, imports, crate paths, the `elem!` and
`func!` wrappers, doc-fence markers) and formats both sides with the crate's rustfmt settings, so
only real changes remain:

```sh
python3 tools/typst-upstream/upstream_diff.py status         # per file: items kept, changed, removed, added
python3 tools/typst-upstream/upstream_diff.py diff [FILE...]  # unified diffs from upstream
python3 tools/typst-upstream/upstream_diff.py paths          # the upstream paths the crate mirrors
python3 tools/typst-upstream/upstream_diff.py bump vNEW      # dry-run upstream's changes since the pin
```

It reads a Typst checkout (`--typst`, default `../typst`) at the commit pinned in
`avenger-typst-label/tests/fixtures/typst-pin.toml`. `bump` ports the crate paths in upstream's
diff, dry-runs it on each ported file with `patch`, and reports the hunks that apply, that the
file already has, and that fail where the port changed the code; `--from` replaces the pinned
commit as the start. `avenger-typst-label/UPSTREAM.md` describes the whole procedure for a new
release.

`rust_items.py` is the item model: a line and brace scanner for rustfmt-formatted Rust, which
names each item by its header (`fn name`, `impl Trait for Type`, `impl Type :: fn method`,
`node! Name`, `use path;`).
