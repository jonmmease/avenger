# Vendored Typst Parser Trimming Plan

## Summary

`avenger-typst` now vendors the upstream Typst parser front end and its immediate
support crates directly into private modules:

- `avenger-typst/src/syntax` copied from `../typst/crates/typst-syntax/src`
- `avenger-typst/src/timing` copied from `../typst/crates/typst-timing/src`
- `avenger-typst/src/utils` copied from `../typst/crates/typst-utils/src`

The copied source came from `../typst` commit
`c98e910391a8544b28bd5c99a6f3b1ac1ada9a84`
(`v0.11.0-1888-gc98e91039` locally). The direct crate dependencies on
`typst-syntax`, `typst-timing`, and `typst-utils` are gone.

The goal of this plan is to shrink the copied parser/support code while
preserving Avenger's public text behavior:

- Canonical Typst parsing for single-line chart labels.
- Typst math syntax inside `$...$`.
- Static non-math markup currently supported by Avenger, such as underline,
  overline, strike, sub/super, highlight, and `#emoji.face`.
- Robust Unicode text handling, bidi/complex script compatibility, and emoji.
- Strict errors for unsupported Typst constructs when Typst markup mode is
  enabled.

This is a size and maintainability effort. It should not change chart rendering
semantics.

## Current State

The copied modules add roughly 15k lines:

- `src/syntax`: about 12.5k lines.
- `src/timing`: about 320 lines.
- `src/utils`: about 2.5k lines.

The copied code currently brings these parser-only dependencies into
`avenger-typst`:

- `ecow`
- `libm`
- `once_cell`
- `parking_lot`
- `portable-atomic`
- `rayon`
- `rustc-hash`
- `semver`
- `serde`
- `serde_json`
- `siphasher`
- `smallvec`
- `thin-vec`
- `toml`
- `unicode-ident`
- `unscanny`

Some of these overlap with existing Avenger needs or are very small. The
largest easy removals are likely the support for package manifests, timing JSON,
source replacement/reparse, syntax highlighting, hash helpers, deferred Rayon
work, Pico strings, and TOML/semver/serde support.

## Constraints

- [ ] Keep `avenger-typst` public APIs stable during this trimming pass.
- [ ] Keep `avenger-text` and chart-facing syntax mode behavior unchanged.
- [ ] Do not reintroduce the Typst evaluator, document model, package loader, or
      frame/render backends.
- [ ] Run release-mode checks and tests after each meaningful step.
- [ ] Commit after each coherent cleanup phase.

## Phase 0 - Establish Parser Parity Tests

- [ ] Add focused `avenger-typst` tests that parse and lower the supported
      syntax examples through the vendored parser:
      - plain text
      - escaped characters and escaped dollar signs
      - unmatched literal dollar behavior
      - inline math
      - nested math groups
      - fractions, roots, scripts, primes, accents, named functions, and
        shorthand symbols
      - static text commands
      - named emoji
      - Unicode, bidi, and complex-script smoke strings
- [ ] Add unsupported-syntax tests for constructs we intentionally reject:
      - display math
      - multiline math
      - imports
      - `#let`
      - unknown function calls outside math
      - unsupported named arguments
      - math matrices/alignment
- [ ] Add a small parser-only oracle test that compares selected raw syntax tree
      summaries against the copied upstream behavior before any trimming.
- [ ] Measure baseline binary size with the existing text size probe in release
      mode.
- [ ] Record current `cargo tree -p avenger-typst --edges normal` output.

Commands:

```sh
cargo test --release -p avenger-typst -- --nocapture
cargo check --release -p avenger-typst
cargo check --release --manifest-path tools/text-size-probe/Cargo.toml --no-default-features --features typst
```

## Phase 1 - Remove Timing Runtime

The parser only uses timing scopes around parse calls. Avenger does not need
Typst's Chrome trace export path.

- [ ] Replace `src/timing/lib.rs` with a minimal no-op `TimingScope` that
      preserves the API used by `src/syntax/parser.rs` and `src/syntax/source.rs`.
- [ ] Keep the `timed!` macro only if copied parser code still references it.
      Otherwise delete it.
- [ ] Remove timing event storage, thread IDs, JSON export, and wasm timer code.
- [ ] Remove dependencies that become unused:
      - `parking_lot`
      - `serde_json`
      - timing-only `serde` usage
- [ ] Verify parser tests and size probe.

Expected result:

- `src/timing` becomes less than 50 lines.
- No runtime timing state remains.

## Phase 2 - Remove Package Manifest Support

`src/syntax/package.rs` is for Typst package manifests and TOML parsing. Avenger
does not load Typst packages.

- [ ] Confirm `PackageSpec`, `PackageManifest`, and related types are not used by
      Avenger lowering or syntax tests.
- [ ] Delete `src/syntax/package.rs`.
- [ ] Remove `pub mod package` from `src/syntax/lib.rs`.
- [ ] Remove package-only dependencies:
      - `toml`
      - `semver`
      - any remaining package-only `serde` usage
- [ ] If `serde` remains only for Avenger's own optional derives, move it back to
      optional feature usage where possible.
- [ ] Verify parser tests and size probe.

Expected result:

- No TOML parser in the Typst text path.
- No Typst version compatibility logic in the parser support path.

## Phase 3 - Remove Editor/IDE-Oriented Syntax Modules

Avenger needs parse trees for one-shot lowering, not editor features.

- [ ] Remove `src/syntax/highlight.rs` and exports for `highlight`,
      `highlight_html`, and `Tag`.
- [ ] Remove `src/syntax/reparser.rs` and exports or imports for incremental
      reparsing.
- [ ] Remove `src/syntax/source.rs` if no parser parity test needs it. Avenger
      calls `parse` and `parse_math` directly and does its own range synthesis.
- [ ] Remove `src/syntax/lines.rs` if it is only used by `Source`.
- [ ] Remove dead public reexports from `src/syntax/lib.rs`.
- [ ] Verify no public Avenger API exposes copied `syntax` types.
- [ ] Verify parser tests and size probe.

Expected result:

- Syntax module exposes only `parse`, `parse_math`, selected AST node types,
  syntax kinds, ranges/spans, and the low-level node machinery needed by lowering.

## Phase 4 - Simplify Span and Path Infrastructure

Avenger currently synthesizes `Span::Range` values by constructing a fake
`FileId` with a `RootedPath`. This preserves upstream AST APIs, but most file
identity machinery is unnecessary for chart labels.

- [ ] Audit which `Span`, `RangeMapper`, `SpanKind`, `FileId`, `RootedPath`,
      `VirtualPath`, and `VirtualRoot` APIs are used by `engine/syntax.rs` and
      `engine/math/syntax.rs`.
- [ ] Replace fake file-path identity with an Avenger-only detached source id or
      direct byte ranges if AST lowering can still call `.range()`.
- [ ] Delete or shrink `src/syntax/path.rs` once package/source support is gone.
- [ ] Keep byte-range accuracy for all lowered plain/math/markup segments.
- [ ] Verify error ranges in existing invalid-syntax tests.
- [ ] Verify parser tests and size probe.

Expected result:

- No virtual path interner for one-line strings.
- Ranges remain accurate enough for Avenger diagnostics and source span metadata.

## Phase 5 - Trim Utility Modules To Parser Needs

After phases 1-4, most of `src/utils` should become unused.

- [ ] Use `cargo check --release -p avenger-typst` plus `rg` to identify live
      utility APIs.
- [ ] Keep only utilities directly needed by parser/AST/node code, likely:
      - `default_math_class`
      - `defer`
      - `debug`
      - `NonZeroExt`
      - small iterator helpers if still referenced
- [ ] Delete unused modules:
      - `bitset`
      - `deferred`
      - `duration`
      - `fat`
      - `hash`
      - `listset`
      - `pico`
      - `protected`
      - `round`
      - `scalar`
      - `version`
- [ ] Remove now-unused dependencies:
      - `once_cell`
      - `rayon`
      - `portable-atomic`
      - `siphasher`
      - `thin-vec`
      - possibly `libm`
- [ ] Verify parser tests and size probe.

Expected result:

- `src/utils` is a small parser helper module, not a copied general utility
  crate.

## Phase 6 - Prune AST Surface Carefully

This phase is higher risk. Do it only after easy module/dependency removals are
complete.

- [ ] Identify AST node structs/enums used by:
      - `engine/syntax.rs`
      - `engine/math/syntax.rs`
      - parser parity tests
- [ ] Remove AST methods and node wrappers that are only for unsupported Typst
      features when the parser can still compile.
- [ ] Prefer deleting unreachable public methods first, not changing parser node
      construction.
- [ ] Keep enough AST to reject unsupported syntax intentionally instead of
      silently misparsing it.
- [ ] Verify all unsupported-syntax diagnostics still point to useful ranges.
- [ ] Verify parser tests and size probe.

Expected result:

- AST remains source-compatible with Avenger lowering, but loses broad Typst
  document/editor API surface.

## Phase 7 - Prune Parser Grammar Only If Needed

Parser surgery is the riskiest step because canonical Typst syntax was the
reason for vendoring the parser.

- [ ] Do not start this phase until binary-size measurements show module-level
      trimming is insufficient.
- [ ] Keep full lexical support for Unicode identifiers, escapes, math
      shorthand, and Typst grouping.
- [ ] Keep robust handling of unsupported constructs so invalid inputs error
      predictably.
- [ ] Consider removing parse branches for:
      - Typst package imports
      - module-level declarations
      - block-level markup that cannot appear in a single chart label
      - tables, lists, headings, raw blocks, references, labels, and citations
      - math alignment/matrix constructs if they are not needed for diagnostics
- [ ] Keep code-expression parsing required for static markup calls and future
      literal options, such as `#underline(stroke: 1.5pt + red, offset: 2pt,
      [care])`.
- [ ] Add upstream-comparison tests before each parser grammar deletion.
- [ ] Verify parser tests, chart visual tests, and size probe.

Expected result:

- Parser still accepts canonical supported Typst syntax, but no longer carries
  large unreachable branches for document features.

## Phase 8 - Final Dependency and Binary Audit

- [ ] Run `cargo tree -p avenger-typst --edges normal` and compare to Phase 0.
- [ ] Confirm these packages are absent unless still justified:
      - `typst-syntax`
      - `typst-timing`
      - `typst-utils`
      - `toml`
      - `semver`
      - `serde_json`
      - `parking_lot`
      - `rayon`
      - `portable-atomic`
      - `siphasher`
      - `thin-vec`
- [ ] Re-run release size probes and record before/after results.
- [ ] Run `cargo bloat` for the text probe if available and summarize remaining
      hotspots.
- [ ] Run chart visual baselines that cover Typst text and math.
- [ ] Commit the final cleanup.

Commands:

```sh
cargo test --release -p avenger-typst -- --nocapture
cargo test --release -p avenger-text -- --nocapture
cargo check --release --workspace
cargo check --release --manifest-path tools/text-size-probe/Cargo.toml --no-default-features --features typst
cargo tree -p avenger-typst --edges normal
```

## Acceptance Criteria

- [ ] `avenger-typst` no longer depends on external `typst-syntax`,
      `typst-timing`, or `typst-utils` crates.
- [ ] Parser/support code is materially smaller than the initial 15k-line copy.
- [ ] Unsupported Typst syntax still produces intentional errors in Typst mode.
- [ ] Supported Typst text/math syntax renders and measures identically to the
      pre-trim implementation.
- [ ] Emoji, bidi text, complex scripts, and math-heavy labels remain covered by
      tests.
- [ ] Release-mode tests and visual baselines pass.
