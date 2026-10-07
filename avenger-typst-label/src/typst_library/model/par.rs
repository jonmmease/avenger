//! Ported from crates/typst-library/src/model/par.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: the paragraph properties a label's inline layout reads. A label is one paragraph
//! without the paragraph element, so the element itself, line numbering, paragraph breaks and the
//! remaining properties are out of scope.

use ecow::eco_format;

use crate::typst_library::diag::{HintedStrResult, StrResult, bail};
use crate::typst_library::foundations::{
    AlternativeFold, CastInfo, Content, Dict, Fold, FromValue, IntoValue, Reflect, Value,
    cast, dict, elem,
};
use crate::typst_library::layout::{Abs, Em, Length, Ratio, Rel};

elem! {
/// A logical subdivison of textual content.
///
/// Typst automatically collects _inline-level_ elements into paragraphs.
/// Inline-level elements include @text[text], @h[horizontal spacing],
/// @box[boxes], and @math.equation[inline equations].
///
/// To separate paragraphs, use a blank line (or an explicit @parbreak).
/// Paragraphs are also automatically interrupted by any block-level element
/// (like @block, @place, or anything that shows itself as one of these).
///
/// The `par` element is primarily used in set rules to affect paragraph
/// properties, but it can also be used to explicitly display its argument as a
/// paragraph of its own. Then, the paragraph's body may not contain any
/// block-level content.
///
/// = Boxes and blocks <boxes-and-blocks>
/// As explained above, usually paragraphs only contain inline-level content.
/// However, you can integrate any kind of block-level content into a paragraph
/// by wrapping it in a @box.
///
/// Conversely, you can separate inline-level content from a paragraph by
/// wrapping it in a @block. In this case, it will not become part of any
/// paragraph at all. Read the following section for an explanation of why that
/// matters and how it differs from just adding paragraph breaks around the
/// content.
///
/// = What becomes a paragraph? <what-becomes-a-paragraph>
/// When you add inline-level content to your document, Typst will automatically
/// wrap it in paragraphs. However, a typical document also contains some text
/// that is not semantically part of a paragraph, for example in a heading or
/// caption.
///
/// The rules for when Typst wraps inline-level content in a paragraph are as
/// follows:
///
/// - All text at the root of a document is wrapped in paragraphs.
///
/// - Text in a container (like a `block`) is only wrapped in a paragraph if the
///   container holds any block-level content. If all of the contents are
///   inline-level, no paragraph is created.
///
/// In the laid-out document, it's not immediately visible whether text became
/// part of a paragraph. However, it is still important for various reasons:
///
/// - Certain paragraph styling like `first-line-indent` will only apply to
///   proper paragraphs, not any text. Similarly, `par` show rules of course
///   only trigger on paragraphs.
///
/// - A proper distinction between paragraphs and other text helps people who
///   rely on Assistive Technology (AT) (such as screen readers) navigate and
///   understand the document properly.
///
/// - PDF export will generate a `P` tag only for paragraphs.
///
/// - HTML export will generate a `<p>` tag only for paragraphs.
///
/// When creating custom reusable components, you can and should take charge
/// over whether Typst creates paragraphs. By wrapping text in a @block instead
/// of just adding paragraph breaks around it, you can force the absence of a
/// paragraph. Conversely, by adding a @parbreak after some content in a
/// container, you can force it to become a paragraph even if it's just one
/// word. This is, for example, what @list.tight[non-`tight`] lists do to force
/// their items to become paragraphs.
///
/// = Example <example>
/// ```example
/// #set par(
///   first-line-indent: 1em,
///   spacing: 0.65em,
///   justify: true,
/// )
///
/// We proceed by contradiction.
/// Suppose that there exists a set
/// of positive integers $a$, $b$, and
/// $c$ that satisfies the equation
/// $a^n + b^n = c^n$ for some
/// integer value of $n > 2$.
///
/// Without loss of generality,
/// let $a$ be the smallest of the
/// three integers. Then, we ...
/// ```
#[elem(name = "par", scope, title = "Paragraph", Locatable, Tagged)]
pub struct ParElem {
    /// The spacing between lines.
    ///
    /// Leading defines the spacing between the @text.bottom-edge[bottom edge]
    /// of one line and the @text.top-edge[top edge] of the following line. By
    /// default, these two properties are up to the font, but they can also be
    /// configured manually with a text set rule.
    ///
    /// By setting top edge, bottom edge, and leading, you can also configure a
    /// consistent baseline-to-baseline distance. You could, for instance, set
    /// the leading to `{1em}`, the top-edge to `{0.8em}`, and the bottom-edge
    /// to `{-0.2em}` to get a baseline gap of exactly `{2em}`. The exact
    /// distribution of the top- and bottom-edge values affects the bounds of
    /// the first and last line.
    ///
    /// ```preview
    /// // Color palette
    /// #let c = (
    ///   par-line: aqua.transparentize(60%),
    ///   leading-line: blue,
    ///   leading-text: blue.darken(20%),
    ///   spacing-line: orange.mix(red).darken(15%),
    ///   spacing-text: orange.mix(red).darken(20%),
    /// )
    ///
    /// // A sample text for measuring font metrics.
    /// #let sample-text = [A]
    ///
    /// // Number of lines in each paragraph
    /// #let n-lines = (4, 4, 2)
    /// #let annotated-lines = (4, 8)
    ///
    /// // The wide margin is for annotations
    /// #set page(width: 350pt, margin: (x: 20%))
    ///
    /// #context {
    ///   let text-height = measure(sample-text).height
    ///   let line-height = text-height + par.leading.to-absolute()
    ///
    ///   let jumps = n-lines
    ///     .map(n => ((text-height,) * n).intersperse(par.leading))
    ///     .intersperse(par.spacing)
    ///     .flatten()
    ///
    ///   place(grid(
    ///     ..jumps
    ///       .enumerate()
    ///       .map(((i, h)) => if calc.even(i) {
    ///         // Draw a stripe for the line
    ///         block(height: h, width: 100%, fill: c.par-line)
    ///       } else {
    ///         // Put an annotation for the gap
    ///         let sw(a, b) = if h == par.leading { a } else { b }
    ///
    ///         align(end, block(
    ///           height: h,
    ///           outset: (right: sw(0.5em, 1em)),
    ///           stroke: (
    ///             left: none,
    ///             rest: 0.5pt + sw(c.leading-line, c.spacing-line),
    ///           ),
    ///           if i / 2 <= sw(..annotated-lines) {
    ///             place(horizon, dx: 1.3em, text(
    ///               0.8em,
    ///               sw(c.leading-text, c.spacing-text),
    ///               sw([leading], [spacing]),
    ///             ))
    ///           },
    ///         ))
    ///       })
    ///   ))
    ///
    ///   // Mark top and bottom edges
    ///   place(
    ///     // pos: top/bottom edge
    ///     // dy: Δy to the last mark
    ///     // kind: leading/spacing
    ///     for (pos, dy, kind) in (
    ///       (bottom, text-height, "leading"),
    ///       (top, par.leading, "leading"),
    ///       (bottom, (n-lines.first() - 1) * line-height - par.leading, "spacing"),
    ///       (top, par.spacing, "spacing"),
    ///     ) {
    ///       v(dy)
    ///
    ///       let c-text = c.at(kind + "-text")
    ///       let c-line = c.at(kind + "-line")
    ///
    ///       place(end, box(
    ///         height: 0pt,
    ///         grid(
    ///           columns: 2,
    ///           column-gutter: 0.2em,
    ///           align: pos,
    ///           move(
    ///             // Compensate optical illusion
    ///             dy: if pos == top { -0.2em } else { 0.05em },
    ///             text(0.8em, c-text)[#repr(pos) edge],
    ///           ),
    ///           line(length: 1em, stroke: 0.5pt + c-line),
    ///         ),
    ///       ))
    ///     },
    ///   )
    /// }
    ///
    /// #set par(justify: true)
    /// #set text(luma(25%), overhang: false)
    /// #show ". ": it => it + parbreak()
    /// #lorem(55)
    /// ```
    #[default(Em::new(0.65).into())]
    pub leading: Length,

    /// Whether to justify text in its line.
    ///
    /// Hyphenation will be enabled for justified paragraphs if the
    /// @text.hyphenate[text function's `hyphenate` property] is set to `{auto}`
    /// and the current language is known.
    ///
    /// Note that the current @align.alignment[alignment] still has an effect on
    /// the placement of the last line except if it ends with a
    /// @linebreak.justify[justified line break].
    ///
    /// By default, Typst only changes the spacing between words to achieve
    /// justification. However, you can also allow it to adjust the spacing
    /// between individual characters using the
    /// @par.justification-limits[`justification-limits` property].
    #[default(false)]
    pub justify: bool,

    // avenger: the example's fence is marked `ignore`, so that rustdoc doesn't run it.
    /// How much the spacing between words and characters may be adjusted during
    /// justification.
    ///
    /// When justifying text, Typst needs to stretch or shrink a line to the
    /// full width of the measure. To achieve this, by default, it adjusts the
    /// spacing between words. Additionally, it can also adjust the spacing
    /// between individual characters. This property allows you to configure
    /// lower and upper bounds for these adjustments.
    ///
    /// The property accepts a dictionary with two entries, `spacing` and
    /// `tracking`, each containing a dictionary with the keys `min` and `max`.
    /// The `min` keys define down to which lower bound gaps may be shrunk while
    /// the `max` keys define up to which upper bound they may be stretched.
    ///
    /// - The `spacing` entry defines how much the width of spaces between words
    ///   may be adjusted. It is closely related to @text.spacing and its `min`
    ///   and `max` keys accept @relative[relative lengths], just like the
    ///   `spacing` property.
    ///
    ///   A `min` value of `{100%}` means that spaces should retain their normal
    ///   size (i.e. not be shrunk), while a value of `{90% - 0.01em}` would
    ///   indicate that a space can be shrunk to a width of 90% of its normal
    ///   width minus 0.01× the current font size. Similarly, a `max` value of
    ///   `{100% + 0.02em}` means that a space's width can be increased by 0.02×
    ///   the current font size. The ratio part must always be positive. The
    ///   length part, meanwhile, must not be positive for `min` and not be
    ///   negative for `max`.
    ///
    ///   Note that spaces may still be expanded beyond the `max` value if there
    ///   is no way to justify the line otherwise. However, other means of
    ///   justification (e.g. spacing apart characters if the `tracking` entry
    ///   is configured accordingly) are first used to their maximum.
    ///
    /// - The `tracking` entry defines how much the spacing between letters may
    ///   be adjusted. It is closely related to @text.tracking and its `min` and
    ///   `max` keys accept @length[lengths], just like the `tracking` property.
    ///   Unlike `spacing`, it does not accept relative lengths because the base
    ///   of the relative length would vary for each character, leading to an
    ///   uneven visual appearance. The behavior compared to `spacing` is as if
    ///   the base was `{100%}`.
    ///
    ///   Otherwise, the `min` and `max` values work just like for `spacing`. A
    ///   `max` value of `{0.01em}` means that additional spacing amounting to
    ///   0.01× of the current font size may be inserted between every pair of
    ///   characters. Note that this also includes the gaps between spaces and
    ///   characters, so for spaces the values of `tracking` act in addition to
    ///   the values for `spacing`.
    ///
    /// If you only specify one of `spacing` or `tracking`, the other retains
    /// its previously set value (or the default if it was not previously set).
    ///
    /// If you want to enable character-level justification, a good value for
    /// the `min` and `max` keys is around `{0.01em}` to `{0.02em}` (negated for
    /// `min`). Using the same value for both gives a good baseline, but
    /// tweaking the two values individually may produce more balanced results,
    /// as demonstrated in the example below. Be careful not to set the bounds
    /// too wide, as it quickly looks unnatural.
    ///
    /// Using character-level justification is an impactful microtypographical
    /// technique that can improve the appearance of justified text, especially
    /// in narrow columns. Note though that character-level justification does
    /// not work with every font or language. For example, cursive fonts connect
    /// letters. Using character-level justification would lead to jagged
    /// connections.
    ///
    /// #example(
    ///   title: "Character-level justification",
    ///   ```ignore
    ///   #let example(name) = columns(2, gutter: 10pt)[
    ///     #place(top, float: true, scope: "parent", strong(name))
    ///   >>> Anne Christine Bayley (1~June 1934 – 31~December 2024) was an
    ///   >>> English surgeon. She was awarded the Order of the British Empire
    ///   >>> for her research into HIV/AIDS patients in Zambia and for
    ///   >>> documenting the spread of the disease among heterosexual patients in
    ///   >>> Africa. In addition to her clinical work, she was a lecturer and
    ///   >>> head of the surgery department at the University of Zambia School of
    ///   >>> Medicine. In the 1990s, she returned to England, where she was
    ///   >>> ordained as an Anglican priest. She continued to be active in Africa
    ///   >>> throughout her retirement years.
    ///   <<<   /* Text from https://en.wikipedia.org/wiki/Anne_Bayley */
    ///   ]
    ///
    ///   #set page(width: 440pt, height: 21em, margin: 15pt)
    ///   #set par(justify: true)
    ///   #set text(size: 0.8em)
    ///
    ///   #grid(
    ///     columns: (1fr, 1fr),
    ///     gutter: 20pt,
    ///     {
    ///       // These are Typst's default limits.
    ///       set par(justification-limits: (
    ///         spacing: (min: 100% * 2 / 3, max: 150%),
    ///         tracking: (min: 0em, max: 0em),
    ///       ))
    ///       example[Word-level justification]
    ///     },
    ///     {
    ///       // These are our custom character-level limits.
    ///       set par(justification-limits: (
    ///         tracking: (min: -0.01em, max: 0.02em),
    ///       ))
    ///       example[Character-level justification]
    ///     },
    ///   )
    ///   ```
    /// )
    #[fold]
    pub justification_limits: JustificationLimits,

    /// The contents of the paragraph.
    #[required]
    pub body: Content,
}
}
/// Configures how justification may distribute spacing.
#[derive(Debug, Copy, Clone, PartialEq, Hash)]
pub struct JustificationLimits {
    /// Limits for spacing, relative to the space width.
    spacing: Option<Limits<Rel>>,
    /// Limits for tracking, _in addition_ to the glyph width.
    tracking: Option<Limits<Length>>,
}

impl JustificationLimits {
    /// Access the spacing limits.
    pub fn spacing(&self) -> &Limits<Rel> {
        self.spacing.as_ref().unwrap_or(&Limits::SPACING_DEFAULT)
    }

    /// Access the tracking limits.
    pub fn tracking(&self) -> &Limits<Length> {
        self.tracking.as_ref().unwrap_or(&Limits::TRACKING_DEFAULT)
    }
}

cast! {
    JustificationLimits,
    self => {
        let mut dict = Dict::new();
        if let Some(spacing) = &self.spacing {
            dict.insert("spacing".into(), spacing.into_value());
        }
        if let Some(tracking) = &self.tracking {
            dict.insert("tracking".into(), tracking.into_value());
        }
        Value::Dict(dict)
    },
    mut dict: Dict => {
        let spacing = dict
            .take("spacing")
            .ok()
            .map(|v| Limits::cast(v, "spacing"))
            .transpose()?;
        let tracking = dict
            .take("tracking")
            .ok()
            .map(|v| Limits::cast(v, "tracking"))
            .transpose()?;
        dict.finish(&["spacing", "tracking"])?;
        Self { spacing, tracking }
    },
}

impl Fold for JustificationLimits {
    fn fold(self, outer: Self) -> Self {
        Self {
            spacing: self.spacing.fold_or(outer.spacing),
            tracking: self.tracking.fold_or(outer.tracking),
        }
    }
}

impl Default for JustificationLimits {
    fn default() -> Self {
        Self {
            spacing: Some(Limits::SPACING_DEFAULT),
            tracking: Some(Limits::TRACKING_DEFAULT),
        }
    }
}

/// Determines the minimum and maximum size by or to which spacing may be shrunk
/// and stretched.
#[derive(Debug, Copy, Clone, PartialEq, Hash)]
pub struct Limits<T> {
    /// Minimum allowable adjustment.
    pub min: T,
    /// Maximum allowable adjustment.
    pub max: T,
}

impl Limits<Rel> {
    const SPACING_DEFAULT: Self = Self {
        min: Rel::new(Ratio::new(2.0 / 3.0), Length::zero()),
        max: Rel::new(Ratio::new(1.5), Length::zero()),
    };
}

impl Limits<Length> {
    const TRACKING_DEFAULT: Self = Self { min: Length::zero(), max: Length::zero() };
}

impl<T: Reflect> Reflect for Limits<T> {
    fn input() -> CastInfo {
        Dict::input()
    }

    fn output() -> CastInfo {
        Dict::output()
    }

    fn castable(value: &Value) -> bool {
        Dict::castable(value)
    }
}

impl<T: IntoValue> IntoValue for Limits<T> {
    fn into_value(self) -> Value {
        Value::Dict(dict! {
            "min" => self.min,
            "max" => self.max,
        })
    }
}

impl<T> Limits<T> {
    /// Not implementing `FromValue` here because we want to pass the `field`
    /// for the error message. Ideally, the casting infrastructure would be
    /// bit more flexible here.
    fn cast(value: Value, field: &str) -> HintedStrResult<Self>
    where
        T: FromValue + Limit,
    {
        let mut dict: Dict = value.cast()?;
        let mut take = |key, check: fn(T) -> StrResult<T>| {
            dict.take(key)?
                .cast::<T>()
                .map_err(|hinted| hinted.message().clone())
                .and_then(check)
                .map_err(|err| {
                    eco_format!("`{key}` value of `{field}` is invalid ({err})")
                })
        };
        let min = take("min", Limit::checked_min)?;
        let max = take("max", Limit::checked_max)?;
        dict.finish(&["min", "max"])?;
        Ok(Self { min, max })
    }
}

impl<T> Fold for Limits<T> {
    fn fold(self, _: Self) -> Self {
        self
    }
}

/// Validation for limit components.
trait Limit: Sized {
    fn checked_min(self) -> StrResult<Self>;
    fn checked_max(self) -> StrResult<Self>;
}

impl Limit for Length {
    fn checked_min(self) -> StrResult<Self> {
        if self.abs > Abs::zero() || self.em > Em::zero() {
            bail!("length must be negative or zero");
        }
        Ok(self)
    }

    fn checked_max(self) -> StrResult<Self> {
        if self.abs < Abs::zero() || self.em < Em::zero() {
            bail!("length must be positive or zero");
        }
        Ok(self)
    }
}

impl Limit for Rel<Length> {
    fn checked_min(self) -> StrResult<Self> {
        if self.rel <= Ratio::zero() {
            bail!("ratio must be positive");
        }
        self.abs.checked_min()?;
        Ok(self)
    }

    fn checked_max(self) -> StrResult<Self> {
        if self.rel <= Ratio::zero() {
            bail!("ratio must be positive");
        }
        self.abs.checked_max()?;
        Ok(self)
    }
}
