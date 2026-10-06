//! Ported from crates/typst-library/src/model/emph.rs @ v0.15.1, modified for Avenger.

use crate::typst_library::foundations::{Content, elem};

elem! {
/// Emphasizes content by toggling italics.
///
/// - If the current @text.style[text style] is `{"normal"}`, this turns it into
///   `{"italic"}`.
/// - If it is already `{"italic"}` or `{"oblique"}`, it turns it back to
///   `{"normal"}`.
///
/// = Example <example>
/// ```example
/// This is _emphasized._ \
/// This is #emph[too.]
///
/// #show emph: it => {
///   text(blue, it.body)
/// }
///
/// This is _emphasized_ differently.
/// ```
///
/// = Syntax <syntax>
/// This function also has dedicated syntax: To emphasize content, simply
/// enclose it in underscores (`_`). Note that this only works at word
/// boundaries. To emphasize part of a word, you have to use the function.
#[elem(name = "emph", title = "Emphasis", keywords = ["italic"], Locatable, Tagged)]
pub struct EmphElem {
    /// The content to emphasize.
    #[required]
    pub body: Content,
}
}
