//! Compact single-line math layout.
//!
//! These partial files mirror the retained pieces of upstream `typst-layout`'s
//! math directory while staying in one Rust module namespace. Keeping one
//! namespace makes this split traceable without changing behavior or adding a
//! large visibility refactor.
//!
//! Upstream comparison points:
//! `crates/typst-layout/src/math/{mod,line,shaping,text}.rs`,
//! `fraction.rs`, `scripts.rs`, `fenced.rs`, `radical.rs`, `accent.rs`, and
//! `cancel.rs`.

include!("run.rs");
include!("row.rs");
include!("decorate.rs");
include!("fenced.rs");
include!("constructs.rs");
include!("stack.rs");
include!("scripts.rs");
include!("atom.rs");
include!("spacing.rs");
include!("style.rs");
include!("font.rs");
include!("pdf.rs");
include!("svg.rs");
include!("tests.rs");
