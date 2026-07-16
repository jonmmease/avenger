# avenger-lang-core

Dependency-light source/frontend layer for the Avenger chart language. It owns
source identities and text, byte spans and line indexes, deterministic
diagnostics, explicit capability types, and source-loader/environment traits.
It also owns the shared sqlparser token stream, `AvengerSqlDialect`, doc-comment
capture, lossless `$binding` normalization, and the normative SQL expression
and query-island entry points. It also owns the strict v1 outer parser, stable
semantic AST and external source map, frozen fourteen-tag JSON interchange
schema, canonical semantic printer, and comment-preserving valid-source
formatter. Later phases add project loading, authoring-schema validation,
resolution, and expansion.

Parse a reviewed fixture and display its canonical JSON and source forms with:

```sh
cargo run --release -p avenger-lang-core --example parse_and_print -- \
  avenger-lang-core/tests/fixtures/parse/chart.avenger
```

This crate must not depend on chart construction, rendering, application,
filesystem, Arrow, or DataFusion crates. Its integration boundary test also
prevents chart crates from depending back on any `avenger-lang*` crate.
DataFusion parsing and planning remain compiler-layer concerns; this crate
depends directly only on the workspace-pinned sqlparser frontend.
