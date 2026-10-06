//! Ported from crates/typst-library/src/math/equation.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: an equation is inline, unnumbered and not referenced, so it has no numbering,
//! number alignment, supplement, alternative description or locale, and is neither
//! synthesized, counted, nor outlined. A label styles its equations through a ghost property,
//! where a document would use a show-set rule.

use codex::styling::MathVariant;

use crate::typst_library::foundations::{
    Content, Packed, ShowSet, StyleChain, Styles, elem,
};
use crate::typst_library::layout::Em;
use crate::typst_library::math::MathSize;
use crate::typst_library::text::{FontFamily, FontList, FontWeight, TextElem, TextSize};
use crate::typst_library::visualize::Paint;

elem! {
/// A mathematical equation.
///
/// Can be displayed inline with text or as a separate block. An equation
/// becomes block-level through the presence of whitespace after the opening
/// dollar sign and whitespace before the closing dollar sign.
///
/// = Example <example>
/// ```example
/// #set text(font: "New Computer Modern")
///
/// Let $a$, $b$, and $c$ be the side
/// lengths of a right-angled triangle.
/// Then, we know that:
/// $ a^2 + b^2 = c^2 $
///
/// Prove by induction:
/// $ sum_(k=1)^n k = (n(n+1)) / 2 $
/// ```
///
/// By default, block-level equations will not break across pages. This can be
/// changed through `{show math.equation: set block(breakable: true)}`.
///
/// = Syntax <syntax>
/// This function also has dedicated syntax: Write mathematical markup within
/// dollar signs to create an equation. Starting and ending the equation with
/// whitespace lifts it into a separate block that is centered horizontally. For
/// more details about math syntax, see the @math[main math page].
#[elem(name = "equation", Locatable, Tagged, ShowSet)]
pub struct EquationElem {
    /// Whether the equation is displayed as a separate block.
    #[default(false)]
    pub block: bool,

    /// The contents of the equation.
    #[required]
    pub body: Content,

    /// The size of the glyphs.
    #[internal]
    #[default(MathSize::Text)]
    #[ghost]
    pub size: MathSize,

    /// The style variant to select.
    #[internal]
    #[ghost]
    pub variant: Option<MathVariant>,

    /// Affects the height of exponents.
    #[internal]
    #[default(false)]
    #[ghost]
    pub cramped: bool,

    /// Whether to use bold glyphs.
    #[internal]
    #[default(false)]
    #[ghost]
    pub bold: bool,

    /// Whether to use italic glyphs.
    #[internal]
    #[ghost]
    pub italic: Option<bool>,

    /// Values of `scriptPercentScaleDown` and `scriptScriptPercentScaleDown`
    /// respectively in the current font's MathConstants table.
    #[internal]
    #[default((70, 50))]
    #[ghost]
    pub script_scale: (i16, i16),

    /// The text style the label sets for its equations.
    // avenger: in place of a `show math.equation: set text(..)` rule, which labels can't
    // write (D15).
    #[internal]
    #[ghost]
    pub label_style: LabelMathStyle,

    /// The font families math falls back to.
    // avenger: in place of the fixed list in `math::families`, as `TextElem::fallbacks` is
    // for text, so that a label falls back to its engine's families.
    #[internal]
    #[default(FontList(
        [
            "new computer modern math",
            "libertinus serif",
            "twitter color emoji",
            "noto color emoji",
            "apple color emoji",
            "segoe ui emoji",
        ]
        .into_iter()
        .map(FontFamily::new)
        .collect()
    ))]
    #[ghost]
    pub fallbacks: FontList,
}
}

/// The text properties a label sets for its equations, which the show-set rule applies over
/// its own. Each unset property keeps the rule's or the inherited value.
// avenger: the math part of a label's style (D15).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct LabelMathStyle {
    /// The font families, in place of the rule's math font.
    pub font: Option<FontList>,
    /// The font weight, in place of the rule's.
    pub weight: Option<FontWeight>,
    /// The font size, relative to the surrounding text's.
    pub size: Option<Em>,
    /// The fill.
    pub fill: Option<Paint>,
}

// upstream: crates/typst-library/src/math/equation.rs::Packed<EquationElem>::show_set @ v0.15.1
// avenger: block equations set only their size, since labels have no blocks, alignment or
// paragraph line numbers.
impl ShowSet for Packed<EquationElem> {
    fn show_set(&self, styles: StyleChain) -> Styles {
        let mut out = Styles::new();
        if self.block.get(styles) {
            out.set(EquationElem::size, MathSize::Display);
        } else {
            out.set(EquationElem::size, MathSize::Text);
        }
        out.set(TextElem::weight, FontWeight::from_number(450));
        out.set(
            TextElem::font,
            FontList(vec![FontFamily::new("New Computer Modern Math")]),
        );

        // avenger: the label's style for equations, as a show-set rule would set it.
        let label = styles.get_ref(EquationElem::label_style);
        if let Some(font) = &label.font {
            out.set(TextElem::font, font.clone());
        }
        if let Some(weight) = label.weight {
            out.set(TextElem::weight, weight);
        }
        if let Some(size) = label.size {
            out.set(TextElem::size, TextSize(size.into()));
        }
        if let Some(fill) = &label.fill {
            out.set(TextElem::fill, fill.clone());
        }
        out
    }
}
