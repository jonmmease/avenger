//! Ported from crates/typst-library/src/math/equation.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a partial port so far. `EquationElem` has the fields text sizing resolves through;
//! the math library adds the rest of upstream's, and its capabilities.

use crate::typst_library::foundations::{Content, elem};
use crate::typst_library::math::MathSize;

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
#[elem(name = "equation")]
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
}
}
