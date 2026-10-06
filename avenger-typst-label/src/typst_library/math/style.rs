//! Ported from crates/typst-library/src/math/style.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: styles are not hashed, so the style functions return `Style`, not `LazyHash`.
//! Upstream's doc examples with bare fences are marked `ignore`, so that rustdoc doesn't run them.

use codex::styling::MathVariant;
use smallvec::smallvec;

use crate::typst_library::foundations::{Content, Style, StyleChain, derive_cast, func};
use crate::typst_library::math::EquationElem;
use crate::typst_library::text::{FontFeatures, Tag, TextElem};

func! {
/// Bold font style in math.
///
/// ```example
/// $ bold(A) := B^+ $
/// ```
#[func(keywords = ["mathbf"])]
pub fn bold(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::bold, true)
}
}

func! {
/// Upright (non-italic) font style in math.
///
/// ```example
/// $ upright(A) != A $
/// ```
#[func(keywords = ["mathup"])]
pub fn upright(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::italic, Some(false))
}
}

func! {
/// Italic font style in math.
///
/// For roman letters and greek lowercase letters, this is already the default.
#[func(keywords = ["mathit"])]
pub fn italic(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::italic, Some(true))
}
}

func! {
/// Serif (roman) font style in math.
///
/// This is already the default.
#[func(keywords = ["mathrm"])]
pub fn serif(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::Plain))
}
}

func! {
/// Sans-serif font style in math.
///
/// ```example
/// $ sans(A B C) $
/// ```
#[func(title = "Sans Serif", keywords = ["mathsf"])]
pub fn sans(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::SansSerif))
}
}

func! {
/// Calligraphic (chancery) font style in math.
///
/// ```example
/// Let $cal(P)$ be the set of ...
/// ```
///
/// This is the default calligraphic/script style for most math fonts. See
/// @math.scr[`scr`] for more on how to get the other style (roundhand).
#[func(title = "Calligraphic", keywords = ["mathcal", "chancery"])]
pub fn cal(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::Chancery))
}
}

func! {
/// Script (roundhand) font style in math.
///
/// ```example
/// $scr(L)$ is not the set of linear
/// maps $cal(L)$.
/// ```
///
/// There are two ways that fonts can support differentiating `cal` and `scr`.
/// The first is using Unicode variation sequences. This works out of the box in
/// Typst, however only a few math fonts currently support this.
///
/// The other way is using @text.features[font features]. For example, the
/// roundhand style might be available in a font through the
/// _@text.stylistic-set[stylistic set] 1_ (`ss01`) feature. To use it in Typst,
/// you could then define your own version of `scr` like in the example below.
///
/// #example(
///   title: "Recreation using stylistic set 1",
///   ```ignore
///   #let scr(it) = text(
///     stylistic-set: 1,
///     $cal(it)$,
///   )
///
///   We establish $cal(P) != scr(P)$.
///   ```
/// )
#[func(title = "Script Style", keywords = ["mathscr", "roundhand"])]
pub fn scr(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::Roundhand))
}
}

func! {
/// Fraktur font style in math.
///
/// ```example
/// $ frak(P) $
/// ```
#[func(title = "Fraktur", keywords = ["mathfrak"])]
pub fn frak(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::Fraktur))
}
}

func! {
/// Monospace font style in math.
///
/// ```example
/// $ mono(x + y = z) $
/// ```
#[func(title = "Monospace", keywords = ["mathtt"])]
pub fn mono(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::Monospace))
}
}

func! {
/// Blackboard bold (double-struck) font style in math.
///
/// For uppercase latin letters, blackboard bold is additionally available
/// through @sym[symbols] of the form `NN` and `RR`.
///
/// ```example
/// $ bb(b) $
/// $ bb(N) = NN $
/// $ f: NN -> RR $
/// ```
#[func(title = "Blackboard Bold", keywords = ["mathbb"])]
pub fn bb(
    /// The content to style.
    body: Content,
) -> Content {
    body.set(EquationElem::variant, Some(MathVariant::DoubleStruck))
}
}

func! {
/// Forced display style in math.
///
/// This is the normal size for block equations.
///
/// ```example
/// $sum_i x_i/2 = display(sum_i x_i/2)$
/// ```
#[func(title = "Display Size", keywords = ["displaystyle"])]
pub fn display(
    /// The content to size.
    body: Content,
    /// Whether to impose a height restriction for exponents, like regular sub-
    /// and superscripts do.
    #[named]
    #[default(false)]
    cramped: bool,
) -> Content {
    body.set(EquationElem::size, MathSize::Display)
        .set(EquationElem::cramped, cramped)
}
}

func! {
/// Forced inline (text) style in math.
///
/// This is the normal size for inline equations.
///
/// ```example
/// $ sum_i x_i/2
///     = inline(sum_i x_i/2) $
/// ```
#[func(title = "Inline Size", keywords = ["textstyle"])]
pub fn inline(
    /// The content to size.
    body: Content,
    /// Whether to impose a height restriction for exponents, like regular sub-
    /// and superscripts do.
    #[named]
    #[default(false)]
    cramped: bool,
) -> Content {
    body.set(EquationElem::size, MathSize::Text)
        .set(EquationElem::cramped, cramped)
}
}

func! {
/// Forced script style in math.
///
/// This is the smaller size used in powers or sub- or superscripts.
///
/// ```example
/// $sum_i x_i/2 = script(sum_i x_i/2)$
/// ```
#[func(title = "Script Size", keywords = ["scriptstyle"])]
pub fn script(
    /// The content to size.
    body: Content,
    /// Whether to impose a height restriction for exponents, like regular sub-
    /// and superscripts do.
    #[named]
    #[default(true)]
    cramped: bool,
) -> Content {
    body.set(EquationElem::size, MathSize::Script)
        .set(EquationElem::cramped, cramped)
}
}

func! {
/// Forced second script style in math.
///
/// This is the smallest size, used in second-level sub- and superscripts
/// (script of the script).
///
/// ```example
/// $sum_i x_i/2 = sscript(sum_i x_i/2)$
/// ```
#[func(title = "Script-Script Size", keywords = ["scriptscriptstyle"])]
pub fn sscript(
    /// The content to size.
    body: Content,
    /// Whether to impose a height restriction for exponents, like regular sub-
    /// and superscripts do.
    #[named]
    #[default(true)]
    cramped: bool,
) -> Content {
    body.set(EquationElem::size, MathSize::ScriptScript)
        .set(EquationElem::cramped, cramped)
}
}

/// The size of elements in an equation.
///
/// See the TeXbook p. 141.
///
/// In MathML Core the attributes `displaystyle` and `scriptlevel` correspond
/// to the CSS properties `math-style` and `math-depth`.
/// - `displaystyle="true"` is equivalent to `math-style: normal`
/// - `displaystyle="false"` is equivalent to `math-style: compact`
/// - `scriptlevel="n"` is equivalent to `math-depth: n`
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum MathSize {
    /// Second-level sub- and superscripts.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `2`.
    ScriptScript,
    /// Sub- and superscripts.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `1`.
    Script,
    /// Math in text.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `false` and `0`.
    Text,
    /// Math on its own line.
    ///
    /// This is equivalent (in MathML Core) to `displaystyle` and `scriptlevel`
    /// as `true` and `0`.
    Display,
}

derive_cast!(MathSize { ScriptScript, Script, Text, Display });

/// Sets flac OpenType feature.
pub fn style_flac() -> Style {
    TextElem::features
        .set(FontFeatures(smallvec![(Tag::from_bytes(b"flac"), 1)]))
        .wrap()
}

/// Sets dtls OpenType feature.
pub fn style_dtls() -> Style {
    TextElem::features
        .set(FontFeatures(smallvec![(Tag::from_bytes(b"dtls"), 1)]))
        .wrap()
}

/// Styles something as cramped.
///
/// This is equivalent (in MathML Core) to the following CSS:
/// ```css
/// math-shift: compact;
/// ```
pub fn style_cramped() -> Style {
    EquationElem::cramped.set(true).wrap()
}

/// The style for superscripts in the current style.
///
/// This is equivalent (in MathML Core) to the following CSS:
/// ```css
/// math-style: compact;
/// math-depth: add(1);
/// ```
pub fn style_for_superscript(styles: StyleChain) -> Style {
    EquationElem::size
        .set(match styles.get(EquationElem::size) {
            MathSize::Display | MathSize::Text => MathSize::Script,
            MathSize::Script | MathSize::ScriptScript => MathSize::ScriptScript,
        })
        .wrap()
}

/// The style for subscripts in the current style.
///
/// This is equivalent (in MathML Core) to the following CSS:
/// ```css
/// math-style: compact;
/// math-depth: add(1);
/// math-shift: compact;
/// ```
pub fn style_for_subscript(styles: StyleChain) -> [Style; 2] {
    [style_for_superscript(styles), style_cramped()]
}

/// The style for numerators in the current style.
///
/// This is equivalent (in MathML Core) to the following CSS:
/// ```css
/// math-style: compact;
/// math-depth: auto-add;
/// ```
pub fn style_for_numerator(styles: StyleChain) -> Style {
    EquationElem::size
        .set(match styles.get(EquationElem::size) {
            MathSize::Display => MathSize::Text,
            MathSize::Text => MathSize::Script,
            MathSize::Script | MathSize::ScriptScript => MathSize::ScriptScript,
        })
        .wrap()
}

/// The style for denominators in the current style.
///
/// This is equivalent (in MathML Core) to the following CSS:
/// ```css
/// math-style: compact;
/// math-depth: auto-add;
/// math-shift: compact;
/// ```
pub fn style_for_denominator(styles: StyleChain) -> [Style; 2] {
    [style_for_numerator(styles), style_cramped()]
}
