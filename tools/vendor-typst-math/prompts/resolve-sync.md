# Typst Vendor Sync Agent Prompt

Use this prompt only after the deterministic vendor pipeline has failed and the
failure cannot be handled by updating the recipe, overlays, or patches directly.

Run Claude non-interactively:

```text
claude -p --model opus --effort high "$(cat tools/vendor-typst-math/prompts/resolve-sync.md)"
```

## Task

You are helping maintain Avenger's vendored Typst math subset. Preserve the
deterministic pipeline:

- Prefer deterministic generator changes over manual edits.
- Prefer overlay or patch files over editing generated vendor files directly.
- Keep generated vendor changes separate from hand-written Avenger changes.
- Do not introduce dependencies denied by `tools/vendor-typst-math/recipe.toml`.
- Do not add `typst`, `typst-html`, `typst-pdf`, `typst-svg`, `typst-render`, or
  `typst-kit` as dependencies of `avenger-typst`.
- Compile and test only in release mode.

## Context To Inspect

- `tools/avenger-typst-vendor/src/main.rs`
- `tools/vendor-typst-math/recipe.toml`
- `vendor/typst-avenger/UPSTREAM_REV`
- `vendor/typst-avenger/VENDOR_MANIFEST.json`
- The current upstream Typst checkout at `../typst`

## Expected Output

Explain the smallest deterministic change needed to complete the sync. If code
changes are needed, edit the generator, recipe, overlays, or patches first, then
regenerate with:

```text
cargo run --release -p avenger-typst-vendor -- \
  --upstream ../typst \
  --rev "$(cat vendor/typst-avenger/UPSTREAM_REV)" \
  --allow-dirty-upstream
```

Then run:

```text
cargo check --release -p avenger-typst --features vendor-typst
cargo test --release -p avenger-typst --features vendor-typst
```
