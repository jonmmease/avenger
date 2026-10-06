//! Ported from crates/typst-library/src/text/mod.rs @ v0.15.1, modified for Avenger.
//!
//! Text styling and shaping infrastructure.
//!
//! avenger: a partial port so far. `TextElem` has the properties the geometry kernel resolves
//! through (`size`, `lang`, `region`, `dir`); the text library adds the rest of upstream's.

mod lang;

pub use self::lang::*;

use std::fmt::{self, Debug, Formatter};

use ecow::{EcoString, eco_format};

use crate::typst_library::diag::bail;
use crate::typst_library::foundations::{
    Content, Fold, NativeElement, Repr, Resolve, Smart, StyleChain, cast, elem,
};
use crate::typst_library::layout::{Abs, Axis, Dir, Em, Length};
use crate::typst_library::math::{EquationElem, MathSize};

elem! {
/// Customizes the look and layout of text in a variety of ways.
///
/// This function is used frequently, both with set rules and directly. While
/// the set rule is often the simpler choice, calling the `text` function
/// directly can be useful when passing text as an argument to another function.
///
/// = Example <example>
/// ```example
/// #set text(18pt)
/// With a set rule.
///
/// #emph(text(blue)[
///   With a function call.
/// ])
/// ```
#[elem(name = "text", Debug, Construct, PlainText, Repr)]
pub struct TextElem {
    /// The size of the glyphs. This value forms the basis of the `em` unit:
    /// `{1em}` is equivalent to the font size.
    ///
    /// You can also give the font size itself in `em` units. Then, it is
    /// relative to the previous font size.
    ///
    /// When used with a suitable variable font, Typst will automatically
    /// configure the `opsz` (optical size) @text.variations[font variation]
    /// based on this property, optimizing legibility for the specific size.
    ///
    /// ```example
    /// #set text(size: 20pt)
    /// very #text(1.5em)[big] text
    /// ```
    #[parse(args.named_or_find("size")?)]
    #[fold]
    #[default(TextSize(Abs::pt(11.0).into()))]
    #[ghost]
    pub size: TextSize,

    /// An #link("https://en.wikipedia.org/wiki/ISO_639")[ISO 639-1/2/3 language code.]
    ///
    /// Setting the correct language affects various parts of Typst:
    ///
    /// - The text processing pipeline can make more informed choices.
    /// - Hyphenation will use the correct patterns for the language.
    /// - @smartquote[Smart quotes] turns into the correct quotes for the
    ///   language.
    /// - And all other things which are language-aware.
    ///
    /// Choosing the correct language is important for accessibility. For
    /// example, screen readers will use it to choose a voice that matches the
    /// language of the text. If your document is in another language than
    /// English (the default), you should set the text language at the start of
    /// your document, before any other content. You can, for example, put it
    /// right after the `[#set document(/* ... */)]` rule that
    /// @document.title[sets your document's title].
    ///
    /// If your document contains passages in a different language than the main
    /// language, you should locally change the text language just for those
    /// parts, either with a set rule
    /// @reference:scripting:blocks[scoped to a block] or using a direct text
    /// function call such as `[#text(lang: "de")[...]]`.
    ///
    /// If multiple codes are available for your language, you should prefer the
    /// two-letter code (ISO 639-1) over the three-letter codes (ISO 639-2/3).
    /// When you have to use a three-letter code and your language differs
    /// between ISO 639-2 and ISO 639-3, use ISO 639-2 for PDF 1.7 (Typst's
    /// default for PDF export) and below and ISO 639-3 for PDF 2.0 and HTML
    /// export.
    ///
    /// The language code is case-insensitive, and will be lowercased when
    /// accessed through @reference:context[context].
    ///
    /// #example(
    ///   title: "Setting the text language to German",
    ///   ```
    ///   #set text(lang: "de")
    ///   #outline()
    ///
    ///   = Einleitung
    ///   In diesem Dokument, ...
    ///   ```
    /// )
    #[default(Lang::ENGLISH)]
    #[ghost]
    pub lang: Lang,

    /// An #link("https://en.wikipedia.org/wiki/ISO_3166-1_alpha-2")[ISO 3166-1
    /// alpha-2 region code.]
    ///
    /// This lets the text processing pipeline make more informed choices.
    ///
    /// The region code is case-insensitive, and will be uppercased when
    /// accessed through @reference:context[context].
    #[ghost]
    pub region: Option<Region>,

    /// The dominant direction for text and inline objects. Possible values are:
    ///
    /// - `{auto}`: Automatically infer the direction from the `lang` property.
    /// - `{ltr}`: Layout text from left to right.
    /// - `{rtl}`: Layout text from right to left.
    ///
    /// When writing in right-to-left scripts like Arabic or Hebrew, you should
    /// set the @text.lang[text language] or direction. While individual runs of
    /// text are automatically layouted in the correct direction, setting the
    /// dominant direction gives the bidirectional reordering algorithm the
    /// necessary information to correctly place punctuation and inline objects.
    /// Furthermore, setting the direction affects the alignment values `start`
    /// and `end`, which are equivalent to `left` and `right` in `ltr` text and
    /// the other way around in `rtl` text.
    ///
    /// If you set this to `rtl` and experience bugs or in some way bad looking
    /// output, please get in touch with us through the
    /// #link("https://forum.typst.app/")[Forum],
    /// #link("https://discord.gg/2uDybryKPe")[Discord server], or our
    /// #link("https://typst.app/contact")[contact form].
    ///
    /// ```example
    /// #set text(dir: rtl)
    /// هذا عربي.
    /// ```
    #[ghost]
    pub dir: TextDir,

    /// The text.
    #[required]
    pub text: EcoString,
}
}

impl TextElem {
    /// Creates a new text element and directly packs it into type-erased
    /// content.
    pub fn packed(text: impl Into<EcoString>) -> Content {
        Self::new(text.into()).pack()
    }
}

impl Debug for TextElem {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "Text({})", self.text)
    }
}

impl Repr for TextElem {
    fn repr(&self) -> EcoString {
        eco_format!("[{}]", self.text)
    }
}

// avenger: no `Construct`, since evaluation styles text directly, and no `PlainText`, since
// only outlines, bibliographies, footnotes and links read plain text.

/// The size of text.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct TextSize(pub Length);

impl Fold for TextSize {
    fn fold(self, outer: Self) -> Self {
        // Multiply the two linear functions.
        Self(Length {
            em: Em::new(self.0.em.get() * outer.0.em.get()),
            abs: self.0.em.get() * outer.0.abs + self.0.abs,
        })
    }
}

impl Resolve for TextSize {
    type Output = Abs;

    fn resolve(self, styles: StyleChain) -> Self::Output {
        let factor = match styles.get(EquationElem::size) {
            MathSize::Display | MathSize::Text => 1.0,
            MathSize::Script => styles.get(EquationElem::script_scale).0 as f64 / 100.0,
            MathSize::ScriptScript => {
                styles.get(EquationElem::script_scale).1 as f64 / 100.0
            }
        };
        factor * self.0.resolve(styles)
    }
}

cast! {
    TextSize,
    self => self.0.into_value(),
    v: Length => Self(v),
}

/// The direction of text and inline objects in their line.
#[derive(Debug, Default, Copy, Clone, Eq, PartialEq, Hash)]
pub struct TextDir(pub Smart<Dir>);

cast! {
    TextDir,
    self => self.0.into_value(),
    v: Smart<Dir> => {
        if v.is_custom_and(|dir| dir.axis() == Axis::Y) {
            bail!("text direction must be horizontal");
        }
        Self(v)
    },
}

impl Resolve for TextDir {
    type Output = Dir;

    fn resolve(self, styles: StyleChain) -> Self::Output {
        match self.0 {
            Smart::Auto => styles.get(TextElem::lang).dir(),
            Smart::Custom(dir) => dir,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_elem_size() {
        assert_eq!(std::mem::size_of::<TextElem>(), std::mem::size_of::<EcoString>());
    }

    // avenger: `test_text_tag_parsing` arrives with font features.
}
