# avenger-lang-core

Dependency-light source/frontend layer for the Avenger chart language. It owns
source identities and text, byte spans and line indexes, deterministic
diagnostics, explicit capability types, and source-loader/environment traits.
Later phases add strict tokenization, parsing, semantic ASTs, validation,
resolution, printing, formatting, and expansion here.

This crate must not depend on chart construction, rendering, application,
filesystem, Arrow, or DataFusion crates. Its integration boundary test also
prevents chart crates from depending back on any `avenger-lang*` crate.
