# avenger-lang-core

Dependency-light source/frontend layer for the Avenger chart language. It owns
source identities and text, byte spans and line indexes, deterministic
diagnostics, explicit capability types, and source-loader/environment traits.
It also owns the shared sqlparser token stream, `AvengerSqlDialect`, doc-comment
capture, lossless `$binding` normalization, and the normative SQL expression
and query-island entry points. Later phases add the strict outer parser,
semantic ASTs, validation, resolution, printing, formatting, and expansion.

This crate must not depend on chart construction, rendering, application,
filesystem, Arrow, or DataFusion crates. Its integration boundary test also
prevents chart crates from depending back on any `avenger-lang*` crate.
DataFusion parsing and planning remain compiler-layer concerns; this crate
depends directly only on the workspace-pinned sqlparser frontend.
