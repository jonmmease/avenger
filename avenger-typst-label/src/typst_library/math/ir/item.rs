//! Ported from crates/typst-library/src/math/ir/item.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: an equation in a label is one inline line, so there are no multiline items,
//! tables, boxes, MathML or external content, and no tags, since labels have no
//! introspection. Without line breaks and alignment points there is no `RawMathItem`: the
//! resolver works on math items. Only tags and placed content are ignorant and only aligned
//! rows are infix at an alignment point, so no item is either.

#![allow(clippy::too_many_arguments)]
use std::cell::Cell;
use std::ops::MulAssign;

use ecow::EcoString;
use unicode_math_class::MathClass;
use unicode_segmentation::UnicodeSegmentation;

use crate::typst_library::foundations::{Smart, StyleChain};
use crate::typst_library::layout::{Abs, Axes, Axis, Em, Length, Ratio, Rel};
use crate::typst_library::math::{CancelAngle, EquationElem, Limits, MathSize};
use crate::typst_library::visualize::FixedStroke;
use crate::typst_syntax::Span;
use crate::typst_utils::{Get, default_math_class};

/// The top-level item in the math IR.
// avenger: no `Tag`.
#[derive(Debug)]
pub enum MathItem<'a> {
    /// A layoutable component with associated properties and styles.
    Component(MathComponent<'a>),
    /// Explicit spacing with the font size at the point of creation. The
    /// boolean indicates whether the spacing is weak.
    Spacing(Length, Abs, bool),
    /// A regular space.
    Space,
}

impl<'a> From<MathComponent<'a>> for MathItem<'a> {
    fn from(comp: MathComponent<'a>) -> Self {
        Self::Component(comp)
    }
}

impl<'a> MathItem<'a> {
    /// Wraps the given items into a group item, or returns the single item if
    /// there is only one.
    pub(crate) fn wrap(
        mut items: Vec<MathItem<'a>>,
        styles: StyleChain<'a>,
    ) -> MathItem<'a> {
        if items.len() == 1 {
            items.pop().unwrap()
        } else {
            GroupItem::create(items, styles)
        }
    }

    /// Returns the limit placement configuration for this item.
    pub(crate) fn limits(&self) -> Limits {
        match self {
            Self::Component(comp) => comp.props.limits,
            _ => Limits::Never,
        }
    }

    /// Returns the math class of this item.
    pub(crate) fn class(&self) -> MathClass {
        self.raw_class().unwrap_or(MathClass::Normal)
    }

    pub(crate) fn raw_class(&self) -> Option<MathClass> {
        match self {
            Self::Component(comp) => comp.props.class,
            Self::Spacing(..) | Self::Space => Some(MathClass::Space),
        }
    }

    /// Returns the effective math class on the right side of this item.
    ///
    /// For fenced items with a closing delimiter and no explicit class, this
    /// returns the closing class instead of the item's overall class.
    pub(crate) fn rclass(&self) -> MathClass {
        match self {
            Self::Component(MathComponent {
                kind: MathKind::Fenced(fence),
                props: MathProperties { class: None, .. },
                ..
            }) if fence.close.is_some() => MathClass::Closing,
            _ => self.class(),
        }
    }

    /// Returns the effective math class on the left side of this item.
    ///
    /// For fenced items with an opening delimiter and no explicit class, this
    /// returns the opening class instead of the item's overall class.
    pub(crate) fn lclass(&self) -> MathClass {
        match self {
            Self::Component(MathComponent {
                kind: MathKind::Fenced(fence),
                props: MathProperties { class: None, .. },
                ..
            }) if fence.open.is_some() => MathClass::Opening,
            _ => self.class(),
        }
    }

    /// Returns the math size of this item, if it is a component.
    pub(crate) fn size(&self) -> Option<MathSize> {
        match self {
            Self::Component(comp) => Some(comp.props.size),
            _ => None,
        }
    }

    /// Whether this item should have explicit spaces around it.
    pub(crate) fn is_spaced(&self) -> bool {
        if self.class() == MathClass::Fence {
            return true;
        }

        if let Self::Component(comp) = self
            && comp.props.spaced
            && matches!(comp.props.class(), MathClass::Normal | MathClass::Alphabetic)
        {
            true
        } else {
            false
        }
    }

    // avenger: no `is_ignorant`, since nothing is ignorant.

    /// Returns the source span of this item.
    pub fn span(&self) -> Span {
        match self {
            Self::Component(comp) => comp.props.span,
            _ => Span::detached(),
        }
    }

    /// Returns the style chain of this item, if it is a component.
    pub fn styles(&self) -> Option<StyleChain<'a>> {
        match self {
            Self::Component(comp) => Some(comp.styles),
            _ => None,
        }
    }

    /// Returns whether this glyph has been stretched as a middle delimiter.
    pub fn mid_stretched(&self) -> Option<bool> {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.mid_stretched.get()
        } else {
            None
        }
    }

    /// Returns the inner items if this is a group, or a slice containing
    /// just this item otherwise.
    pub fn as_slice(&self) -> &[MathItem<'a>] {
        if let MathItem::Component(comp) = self
            && let MathKind::Group(group) = &comp.kind
        {
            &group.items
        } else {
            core::slice::from_ref(self)
        }
    }

    /// Sets the limit placement configuration for this item.
    pub(crate) fn set_limits(&mut self, limits: Limits) {
        if let Self::Component(comp) = self {
            comp.props.limits = limits;
        }
    }

    /// Sets the effective math class of this item.
    pub(crate) fn set_class(&mut self, class: MathClass) {
        if let Self::Component(comp) = self {
            comp.props.class = Some(class);
        }
    }

    /// Sets the effective math class and applies it to glyph layout.
    pub(crate) fn set_explicit_class(&mut self, class: MathClass) {
        self.set_class(class);
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &mut comp.kind
        {
            glyph.class = class;

            // Small hack to ensure the non-explicit stretch gets added, as the
            // class is not recursive. This applies an equivalent stretch to
            // the one in `resolve_symbol`.
            if class == MathClass::Large
                && comp.props.size == MathSize::Display
                && !glyph.stretch.get().is_explicit(Axis::Y)
            {
                let info = StretchInfo::default();
                glyph.stretch.update(|stretch| stretch.with_y(info));
            }
        }
    }

    /// Sets the left spacing for this item if not already set.
    pub(crate) fn set_lspace(&mut self, lspace: Option<Em>) {
        if let Self::Component(comp) = self
            && comp.props.lspace.is_none()
        {
            comp.props.lspace = lspace;
        }
    }

    /// Sets the right spacing for this item if not already set.
    pub(crate) fn set_rspace(&mut self, rspace: Option<Em>) {
        if let Self::Component(comp) = self
            && comp.props.rspace.is_none()
        {
            comp.props.rspace = rspace;
        }
    }

    /// Sets whether this glyph has been stretched as a middle delimiter.
    pub(crate) fn set_mid_stretched(&self, mid_stretched: Option<bool>) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.mid_stretched.set(mid_stretched);
        }
    }

    /// Sets the stretch configuration for this glyph, marking it as explicit.
    pub(crate) fn set_stretch(&self, mut stretch: Stretch) {
        if let Some(info) = &mut stretch.0.x {
            info.explicit = true;
        }
        if let Some(info) = &mut stretch.0.y {
            info.explicit = true;
        }
        self.replace_stretch(stretch);
    }

    /// Sets the stretch configuration for this glyph
    pub(crate) fn replace_stretch(&self, stretch: Stretch) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.stretch.replace(stretch);
        }
    }

    /// Updates the vertical stretch info for this glyph.
    pub(crate) fn set_y_stretch(&self, mut info: StretchInfo) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            info.explicit = true;
            glyph.stretch.update(|stretch| stretch.with_y(info));
        }
    }

    /// Updates the stretch info for both axes of this glyph.
    pub(crate) fn update_stretch(&self, info: StretchInfo) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.stretch.update(|stretch| stretch.update(info));
        }
    }

    /// Sets the reference size for relative stretching on the given axis.
    pub fn set_stretch_relative_to(&self, relative_to: Abs, axis: Axis) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.stretch.update(|stretch| stretch.relative_to(relative_to, axis));
        }
    }

    /// Sets the font size to use for short-fall calculations on the given axis.
    pub fn set_stretch_font_size(&self, font_size: Abs, axis: Axis) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.stretch.update(|stretch| stretch.font_size(font_size, axis));
        }
    }

    /// Enables the flac OpenType feature for this glyph.
    pub fn set_flac(&self) {
        if let Self::Component(comp) = self
            && let MathKind::Glyph(glyph) = &comp.kind
        {
            glyph.flac.set(true);
        }
    }
}

/// A generic component that bundles a specific math item kind with common
/// properties and styles.
#[derive(Debug)]
pub struct MathComponent<'a> {
    /// The specific kind of math item.
    pub kind: MathKind<'a>,
    /// The properties attached to this component.
    pub props: MathProperties,
    /// The item's styles.
    pub styles: StyleChain<'a>,
}

/// The specific kind of a layoutable math item.
///
/// Recursive or large variants are boxed.
// avenger: no `Multiline`, `Table`, `Box`, `Mathml` or `External`.
#[derive(Debug)]
pub enum MathKind<'a> {
    /// A group of math items laid out horizontally.
    Group(GroupItem<'a>),
    /// A radical (square root or nth root).
    Radical(Box<RadicalItem<'a>>),
    /// An item enclosed in delimiters.
    Fenced(Box<FencedItem<'a>>),
    /// A vertical fraction.
    Fraction(Box<FractionItem<'a>>),
    /// An inline skewed fraction.
    SkewedFraction(Box<SkewedFractionItem<'a>>),
    /// A base with scripts (subscripts/superscripts) and/or limits attached.
    Scripts(Box<ScriptsItem<'a>>),
    /// A base with an accent mark above or below.
    Accent(Box<AccentItem<'a>>),
    /// A base with a line overlaid.
    Cancel(Box<CancelItem<'a>>),
    /// A base with a line drawn above or below.
    Line(Box<LineItem<'a>>),
    /// Grouped prime symbols.
    Primes(Box<PrimesItem>),
    /// A text string.
    Text(TextItem),
    /// A number.
    Number(NumberItem),
    /// A single glyph (grapheme cluster).
    Glyph(Box<GlyphItem>),
}

/// Shared properties for all layoutable math components.
// avenger: no `ignorant` or `align_form_infix`.
#[derive(Debug, Copy, Clone)]
pub struct MathProperties {
    /// How attachments should be positioned.
    pub(crate) limits: Limits,
    /// The math class.
    pub class: Option<MathClass>,
    /// The current math size.
    pub size: MathSize,
    /// Whether this item is in a cramped style.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "upstream reads it in HTML export; the IR oracle compares it"
        )
    )]
    pub cramped: bool,
    /// Whether this item should have explicit spaces around it.
    pub(crate) spaced: bool,
    /// The amount of spacing to the left of this item.
    pub lspace: Option<Em>,
    /// The amount of spacing to the right of this item.
    pub rspace: Option<Em>,
    /// The source span.
    pub span: Span,
}

impl MathProperties {
    /// Creates properties with an explicit class, avoiding the style lookup.
    fn new(styles: StyleChain, class: Option<MathClass>, span: Span) -> MathProperties {
        Self {
            limits: Limits::Never,
            class,
            size: styles.get(EquationElem::size),
            cramped: styles.get(EquationElem::cramped),
            spaced: false,
            lspace: None,
            rspace: None,
            span,
        }
    }

    /// Creates default properties from the given styles.
    ///
    /// This gets the math size from the styles.
    pub fn default(styles: StyleChain, span: Span) -> MathProperties {
        Self::new(styles, None, span)
    }

    /// Returns the class, using the default normal class if None.
    pub fn class(&self) -> MathClass {
        self.class.unwrap_or(MathClass::Normal)
    }

    /// Sets how attachments should be positioned for this item.
    fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Sets whether this item should have explicit spaces around it.
    fn with_spaced(mut self, spaced: bool) -> Self {
        self.spaced = spaced;
        self
    }
}

/// A group of math items laid out horizontally.
#[derive(Debug)]
pub struct GroupItem<'a> {
    /// The items in the group.
    pub items: Vec<MathItem<'a>>,
}

impl<'a> GroupItem<'a> {
    /// Creates a new group item.
    pub(crate) fn create(
        items: Vec<MathItem<'a>>,
        styles: StyleChain<'a>,
    ) -> MathItem<'a> {
        let props = MathProperties::default(styles, Span::detached());
        let kind = MathKind::Group(Self { items });
        MathComponent { kind, props, styles }.into()
    }
}

/// A radical (square root or nth root).
#[derive(Debug)]
pub struct RadicalItem<'a> {
    /// The item under the radical symbol.
    pub radicand: MathItem<'a>,
    /// The index for nth roots. `None` for square roots.
    pub index: Option<MathItem<'a>>,
    /// The radical symbol.
    ///
    /// Only used in paged export.
    pub sqrt: MathItem<'a>,
}

impl<'a> RadicalItem<'a> {
    /// Creates a new radical item.
    pub(crate) fn create(
        radicand: MathItem<'a>,
        index: Option<MathItem<'a>>,
        sqrt: MathItem<'a>,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind = MathKind::Radical(Box::new(Self { radicand, index, sqrt }));
        let props = MathProperties::default(styles, span);
        MathComponent { kind, props, styles }.into()
    }
}

/// An item enclosed in delimiters.
#[derive(Debug)]
pub struct FencedItem<'a> {
    /// The optional opening delimiter.
    pub open: Option<MathItem<'a>>,
    /// The optional closing delimiter.
    pub close: Option<MathItem<'a>>,
    /// The item between the delimiters.
    // avenger: always owned, since only the segments of a fence split across lines share a
    // body.
    pub body: MathItem<'a>,
    /// How the target height for the delimiters should be calculated.
    ///
    /// If true, the height for each body item is two times the maximum of its
    /// ascent and descent. If false, the height for each body item is simply
    /// its height.
    ///
    /// Only used in paged export.
    pub balanced: bool,
}

impl<'a> FencedItem<'a> {
    /// Creates a new fenced item.
    pub(crate) fn create(
        open: Option<MathItem<'a>>,
        close: Option<MathItem<'a>>,
        body: MathItem<'a>,
        balanced: bool,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind = MathKind::Fenced(Box::new(Self { open, close, body, balanced }));
        let props = MathProperties::default(styles, span);
        MathComponent { kind, props, styles }.into()
    }
}

/// A vertical fraction.
#[derive(Debug)]
pub struct FractionItem<'a> {
    /// The item in the top part of the fraction.
    pub numerator: MathItem<'a>,
    /// The item in the bottom part of the fraction.
    pub denominator: MathItem<'a>,
    /// Whether to draw a fraction line between the numerator and denominator.
    pub line: bool,
    /// The amount of padding added before and after the fraction.
    pub padding: Em,
}

impl<'a> FractionItem<'a> {
    /// Creates a new fraction item.
    pub(crate) fn create(
        numerator: MathItem<'a>,
        denominator: MathItem<'a>,
        line: bool,
        padding: Em,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind =
            MathKind::Fraction(Box::new(Self { numerator, denominator, line, padding }));
        let props = MathProperties::default(styles, span);
        MathComponent { kind, props, styles }.into()
    }
}

/// An inline skewed fraction.
#[derive(Debug)]
pub struct SkewedFractionItem<'a> {
    /// The item in the top-left part of the fraction.
    pub numerator: MathItem<'a>,
    /// The item in the bottom-right part of the fraction.
    pub denominator: MathItem<'a>,
    /// The fraction slash symbol.
    ///
    /// Only used in paged export.
    pub slash: MathItem<'a>,
}

impl<'a> SkewedFractionItem<'a> {
    /// Creates a new skewed fraction item.
    pub(crate) fn create(
        numerator: MathItem<'a>,
        denominator: MathItem<'a>,
        slash: MathItem<'a>,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind =
            MathKind::SkewedFraction(Box::new(Self { numerator, denominator, slash }));
        let props = MathProperties::default(styles, span);
        MathComponent { kind, props, styles }.into()
    }
}

/// A base with scripts (subscripts/superscripts) and/or limits attached.
#[derive(Debug)]
pub struct ScriptsItem<'a> {
    /// The base item.
    pub base: MathItem<'a>,
    /// The top attachment (limit above).
    pub top: Option<MathItem<'a>>,
    /// The bottom attachment (limit below).
    pub bottom: Option<MathItem<'a>>,
    /// The top-left attachment (pre-superscript).
    pub top_left: Option<MathItem<'a>>,
    /// The bottom-left attachment (pre-subscript).
    pub bottom_left: Option<MathItem<'a>>,
    /// The top-right attachment (post-superscript).
    pub top_right: Option<MathItem<'a>>,
    /// The bottom-right attachment (post-subscript).
    pub bottom_right: Option<MathItem<'a>>,
}

impl<'a> ScriptsItem<'a> {
    /// Creates a new scripts item.
    ///
    /// The resulting item inherits its math class from the base.
    pub(crate) fn create(
        base: MathItem<'a>,
        top: Option<MathItem<'a>>,
        bottom: Option<MathItem<'a>>,
        top_left: Option<MathItem<'a>>,
        bottom_left: Option<MathItem<'a>>,
        top_right: Option<MathItem<'a>>,
        bottom_right: Option<MathItem<'a>>,
        styles: StyleChain<'a>,
    ) -> MathItem<'a> {
        let props = MathProperties::new(styles, base.raw_class(), Span::detached());
        let kind = MathKind::Scripts(Box::new(Self {
            base,
            top,
            bottom,
            top_left,
            bottom_left,
            top_right,
            bottom_right,
        }));
        MathComponent { kind, props, styles }.into()
    }
}

/// A base with an accent mark above or below.
#[derive(Debug)]
pub struct AccentItem<'a> {
    /// The base item.
    pub base: MathItem<'a>,
    /// The accent mark item.
    pub accent: MathItem<'a>,
    /// Whether this is a top or bottom accent.
    pub position: Position,
    /// Whether dotless styles have been added.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "upstream reads it in HTML export; the IR oracle compares it"
        )
    )]
    pub dotless: bool,
    /// Whether the item's width should include the accent's width.
    ///
    /// Only used in paged export.
    pub exact_frame_width: bool,
}

impl<'a> AccentItem<'a> {
    /// Creates a new accent item.
    ///
    /// The resulting item inherits its math class from the base.
    pub(crate) fn create(
        base: MathItem<'a>,
        accent: MathItem<'a>,
        position: Position,
        dotless: bool,
        exact_frame_width: bool,
        styles: StyleChain<'a>,
    ) -> MathItem<'a> {
        let props = MathProperties::new(styles, base.raw_class(), Span::detached());
        let kind = MathKind::Accent(Box::new(Self {
            base,
            accent,
            position,
            dotless,
            exact_frame_width,
        }));
        MathComponent { kind, props, styles }.into()
    }
}

/// A base with a line overlaid.
#[derive(Debug)]
pub struct CancelItem<'a> {
    /// The base item.
    pub base: MathItem<'a>,
    /// The length of the line.
    pub length: Rel<Abs>,
    /// The stroke for the line.
    pub stroke: FixedStroke,
    /// Whether a cross (two lines) is drawn instead of a single line.
    pub cross: bool,
    /// Whether to invert the angle of the first line.
    pub invert_first_line: bool,
    /// The angle of the line.
    pub angle: Smart<CancelAngle>,
}

impl<'a> CancelItem<'a> {
    /// Creates a new cancel item.
    ///
    /// The resulting item inherits its math class from the base.
    pub(crate) fn create(
        base: MathItem<'a>,
        length: Rel<Abs>,
        stroke: FixedStroke,
        cross: bool,
        invert_first_line: bool,
        angle: Smart<CancelAngle>,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let props = MathProperties::new(styles, base.raw_class(), span);
        let kind = MathKind::Cancel(Box::new(Self {
            base,
            length,
            stroke,
            cross,
            invert_first_line,
            angle,
        }));
        MathComponent { kind, props, styles }.into()
    }
}

/// A base with a line drawn above or below.
#[derive(Debug)]
pub struct LineItem<'a> {
    /// The base item.
    pub base: MathItem<'a>,
    /// Whether the line is drawn above or below the base.
    pub position: Position,
}

impl<'a> LineItem<'a> {
    /// Creates a new line item.
    ///
    /// The resulting item inherits its math class from the base.
    pub(crate) fn create(
        base: MathItem<'a>,
        position: Position,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let props = MathProperties::new(styles, base.raw_class(), span);
        let kind = MathKind::Line(Box::new(Self { base, position }));
        MathComponent { kind, props, styles }.into()
    }
}

/// The prime character used by [`PrimesItem`].
pub const PRIME_CHAR: char = '′';

/// Grouped prime symbols.
///
/// This is for more than four prime symbols, since there are only dedicated
/// Unicode codepoints up to four.
#[derive(Debug)]
pub struct PrimesItem {
    /// The number of primes to display. Always at least five.
    pub count: usize,
}

impl PrimesItem {
    /// Creates a new primes item.
    pub(crate) fn create<'a>(count: usize, styles: StyleChain<'a>) -> MathItem<'a> {
        let kind = MathKind::Primes(Box::new(Self { count }));
        let props = MathProperties::default(styles, Span::detached());
        MathComponent { kind, props, styles }.into()
    }
}

/// A text string.
// avenger: no locator, since labels have no introspection.
#[derive(Debug)]
pub struct TextItem {
    /// The text content.
    pub text: EcoString,
}

impl TextItem {
    /// Creates a new text item.
    ///
    /// The resulting item is spaced and has alphabetic math class.
    pub(crate) fn create<'a>(
        text: EcoString,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind = MathKind::Text(Self { text });
        let props = MathProperties::new(styles, Some(MathClass::Alphabetic), span)
            .with_spaced(true);
        MathComponent { kind, props, styles }.into()
    }
}

/// A number.
#[derive(Debug)]
pub struct NumberItem {
    /// The number's text content.
    pub text: EcoString,
}

impl NumberItem {
    /// Creates a new number item.
    pub(crate) fn create<'a>(
        text: EcoString,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        let kind = MathKind::Number(Self { text });
        let props = MathProperties::default(styles, span);
        MathComponent { kind, props, styles }.into()
    }
}

/// A single glyph (grapheme cluster).
#[derive(Debug)]
pub struct GlyphItem {
    /// The text content.
    pub text: EcoString,
    /// The math class to use for layout.
    ///
    /// When the math class is large, the glyph is centered vertically and, in
    /// display style, stretched vertically. This value is not necessarily the
    /// same as the item's associated `MathProperties::class`, which is used
    /// for determining spacing between items.
    pub class: MathClass,
    /// How the glyph should be stretched.
    pub stretch: Cell<Stretch>,
    /// Whether this glyph has been stretched as a middle delimiter.
    pub mid_stretched: Cell<Option<bool>>,
    /// Whether to apply the flac OpenType feature.
    pub flac: Cell<bool>,
}

impl GlyphItem {
    /// Creates a new glyph item.
    ///
    /// The `dtls` parameter indicates that a dotless character was converted
    /// to its non-dotless version.
    pub(crate) fn create<'a>(
        text: EcoString,
        styles: StyleChain<'a>,
        span: Span,
    ) -> MathItem<'a> {
        assert!(text.graphemes(true).count() == 1);

        let c = text.chars().next().unwrap();

        let class = default_math_class(c);
        let limits = Limits::for_char_with_class(c, class);

        let kind = MathKind::Glyph(Box::new(Self {
            text,
            class: class.unwrap_or(MathClass::Normal),
            stretch: Cell::new(Stretch::new()),
            mid_stretched: Cell::new(None),
            flac: Cell::new(false),
        }));
        let props = MathProperties::new(styles, class, span).with_limits(limits);
        MathComponent { kind, props, styles }.into()
    }
}

/// Stretch configuration for a glyph on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Stretch(Axes<Option<StretchInfo>>);

impl Stretch {
    /// Creates a new empty stretch configuration.
    pub(crate) fn new() -> Self {
        Self(Axes::splat(None))
    }

    /// Adds horizontal stretch information.
    pub(crate) fn with_x(mut self, info: StretchInfo) -> Self {
        self.0.x = Some(info);
        self
    }

    /// Adds vertical stretch information.
    pub(crate) fn with_y(mut self, info: StretchInfo) -> Self {
        self.0.y = Some(info);
        self
    }

    /// Updates stretch info for both axes, combining with existing info and
    /// marking them as explicit.
    pub(crate) fn update(mut self, mut info: StretchInfo) -> Self {
        info.explicit = true;
        match &mut self.0.x {
            Some(val) => *val *= info,
            None => self.0.x = Some(info),
        }
        match &mut self.0.y {
            Some(val) => *val *= info,
            None => self.0.y = Some(info),
        }
        self
    }

    /// Sets the reference size for relative stretching on the given axis.
    ///
    /// Only sets the value if not already set.
    pub(crate) fn relative_to(mut self, relative_to: Abs, axis: Axis) -> Self {
        if let Some(info) = self.0.get_mut(axis)
            && info.relative_to.is_none()
        {
            info.relative_to = Some(relative_to);
        }
        self
    }

    /// Sets the font size for short-fall calculations on the given axis.
    ///
    /// Only sets the value if not already set.
    pub(crate) fn font_size(mut self, font_size: Abs, axis: Axis) -> Self {
        if let Some(info) = self.0.get_mut(axis)
            && info.font_size.is_none()
        {
            info.font_size = Some(font_size);
        }
        self
    }

    /// Returns the stretch info for the given axis, if any.
    pub fn resolve(mut self, axis: Axis) -> Option<StretchInfo> {
        if let Some(info) = self.0.get_mut(axis)
            && let Some(buffer) = info.buffer
        {
            // Sort out the buffer before returning the info to use.
            if info.relative_to.is_some() {
                info.target = buffer;
            } else {
                info.target = Rel::new(
                    info.target.rel * buffer.rel,
                    buffer.rel.of(info.target.abs) + buffer.abs,
                );
            }
        }
        self.0.get(axis)
    }

    /// Returns the user-requested stretch target for the given axis, if any.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "upstream calls it in HTML export; the IR oracle compares it"
        )
    )]
    pub fn resolve_requested(self, axis: Axis) -> Option<Rel<Length>> {
        self.0
            .get(axis)
            .and_then(|info| info.requested_target)
            .filter(|target| !target.is_one())
    }

    /// Whether the stretch along the given axis should be represented
    /// explicitly.
    pub fn is_explicit(self, axis: Axis) -> bool {
        self.0.get(axis).is_some_and(|info| info.explicit)
    }
}

/// Information about how to stretch a glyph on one axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StretchInfo {
    /// The target size to stretch to.
    pub target: Rel<Abs>,
    /// A buffer to store the latest stretch added, in case it needs to be
    /// relative to something else.
    buffer: Option<Rel<Abs>>,
    /// Whether this stretch is explicit. That is, the stretch was not from a
    /// large operator in display math.
    pub(crate) explicit: bool,
    /// The user-requested stretch target, if any.
    pub(crate) requested_target: Option<Rel<Length>>,
    /// The short-fall amount for glyph assembly.
    pub short_fall: Em,
    /// The reference size for relative targets.
    ///
    /// Only used in paged export.
    pub relative_to: Option<Abs>,
    /// The font size to use for short-fall.
    ///
    /// Only used in paged export.
    pub font_size: Option<Abs>,
}

impl StretchInfo {
    /// Creates new stretch info with the given target and short-fall.
    pub(crate) fn new(target: Rel<Abs>, short_fall: Em) -> Self {
        Self {
            target,
            buffer: None,
            explicit: false,
            requested_target: None,
            short_fall,
            relative_to: None,
            font_size: None,
        }
    }

    /// Creates stretch info from a user-specified size.
    pub(crate) fn from_size(size: Rel<Length>, short_fall: Em, font_size: Abs) -> Self {
        Self {
            target: size.map(|l| l.at(font_size)),
            buffer: None,
            explicit: false,
            requested_target: (!size.is_one()).then_some(size),
            short_fall,
            relative_to: None,
            font_size: None,
        }
    }
}

impl Default for StretchInfo {
    fn default() -> Self {
        let target = Rel::new(Ratio::one(), Abs::zero());
        Self::new(target, Em::zero())
    }
}

impl MulAssign for StretchInfo {
    fn mul_assign(&mut self, rhs: Self) {
        if let Some(buffer) = self.buffer {
            self.target = Rel::new(
                self.target.rel * buffer.rel,
                buffer.rel.of(self.target.abs) + buffer.abs,
            );
        }
        self.buffer = Some(rhs.target);

        if let Some(requested) = rhs.requested_target {
            self.requested_target =
                Some(self.requested_target.map_or(requested, |target| {
                    Rel::new(
                        target.rel * requested.rel,
                        requested.rel.of(target.abs) + requested.abs,
                    )
                }));
        }

        self.explicit = self.explicit || rhs.explicit;
        self.short_fall = rhs.short_fall;
    }
}

/// A marker representing the positioning of something above or below a base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// Placed above the base.
    Above,
    /// Placed below the base.
    Below,
}
