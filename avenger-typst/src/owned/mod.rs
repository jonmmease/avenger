// Temporary extraction scaffolding. These modules become live when the owned
// backend stops delegating non-empty text/math to the vendored engine.
#[allow(dead_code)]
pub(crate) mod ast;
pub(crate) mod engine;
pub(crate) mod font;
pub(crate) mod inline;
pub(crate) mod math;
pub(crate) mod syntax;
