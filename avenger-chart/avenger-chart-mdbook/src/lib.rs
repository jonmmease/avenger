//! Documentation test harness for the Avenger Chart mdBook.
//!
//! This crate mirrors the Markdown sources in `avenger-chart/book/src`
//! so that `cargo test --doc -p avenger-chart-mdbook` runs rustdoc
//! checks against the same content `mdbook` renders.

include!(concat!(env!("OUT_DIR"), "/book_docs.rs"));

pub mod render_snippets {
    include!(concat!(env!("OUT_DIR"), "/render_snippets.rs"));
}
