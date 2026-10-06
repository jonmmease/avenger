//! Ported from crates/typst-layout/src/math/mod.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's equations are inline and one line, so there is no block or multiline
//! layout, and no tables, boxes, MathML, external content or tags. Layout needs no region or
//! locator. Upstream memoizes layout, which keeps repeated layouts of the same item cheap;
//! without memoization, each equation has a budget of item layouts instead (D12).

mod accent;
mod cancel;
mod fenced;
mod fraction;
mod fragment;
mod line;
mod radical;
mod run;
mod scripts;
mod shaping;
#[cfg(test)]
mod tests;
mod text;

use crate::typst_library::World;
use crate::typst_library::diag::{At, SourceResult, bail, warning};
use crate::typst_library::engine::{Engine, Tracked};
use crate::typst_library::foundations::{Packed, Resolve, Style, StyleChain};
use crate::typst_library::layout::{Frame, InlineItem, Point, Size};
use crate::typst_library::math::ir::{
    MathItem, MathKind, MathProperties, resolve_equation,
};
use crate::typst_library::math::{EquationElem, families};
use crate::typst_library::model::ParElem;
use crate::typst_library::routines::Arenas;
use crate::typst_library::text::{
    Font, FontFlags, FontInstance, TextEdgeBounds, TextElem, variant,
};
use typst_syntax::Span;
use typst_utils::Numeric;

use self::accent::layout_accent;
use self::cancel::layout_cancel;
use self::fenced::layout_fenced;
use self::fraction::{layout_fraction, layout_skewed_fraction};
use self::fragment::{FrameFragment, MathFragment};
use self::line::layout_line;
use self::radical::layout_radical;
use self::run::{MathFragmentsExt, MathRun};
use self::scripts::{layout_primes, layout_scripts};
use self::text::{layout_glyph, layout_number, layout_text};

/// How many items an equation may lay out, counting each layout of an item.
// avenger: bounds the repeated layout that upstream's memoization keeps cheap, such as a
// fence's re-layout of a body with mid delimiters, which doubles with each nested fence
// (D12).
pub(crate) const MATH_WORK_LIMIT: usize = 50_000;

/// The error for an equation that exceeds [`MATH_WORK_LIMIT`].
pub(crate) const MATH_TOO_COMPLEX: &str = "equation is too complex to lay out";

/// Layout an inline equation (in a paragraph).
// upstream: crates/typst-layout/src/math/mod.rs::layout_equation_inline @ v0.15.1
pub fn layout_equation_inline(
    elem: &Packed<EquationElem>,
    engine: &mut Engine,
    styles: StyleChain,
) -> SourceResult<Vec<InlineItem>> {
    assert!(!elem.block.get(styles));

    let span = elem.span();
    let font = get_font(engine.world, styles, span)?;
    warn_non_math_font(&font, engine, span);

    let scale_style = style_for_script_scale(&font);
    let styles = styles.chain(&scale_style);

    let arenas = Arenas::default();
    let item = resolve_equation(elem, engine, &arenas, styles)?;

    let mut ctx = MathContext::new(engine, font.clone(), span);
    let mut items = ctx.layout_into_fragments(&item, styles)?.into_par_items();

    // An empty equation should have a height, so we still create a frame
    // (which is then resized in the loop).
    if items.is_empty() {
        items.push(InlineItem::Frame(Frame::soft(Size::zero())));
    }

    for item in &mut items {
        let InlineItem::Frame(frame) = item else { continue };

        let slack = styles.resolve(ParElem::leading) * 0.7;

        let (t, b) = font.edges(
            styles.get(TextElem::top_edge),
            styles.get(TextElem::bottom_edge),
            styles.resolve(TextElem::size),
            TextEdgeBounds::Frame(frame),
        );

        let ascent = t.max(frame.ascent() - slack);
        let descent = b.max(frame.descent() - slack);
        frame.translate(Point::with_y(ascent - frame.baseline()));
        frame.size_mut().y = ascent + descent;
    }

    Ok(items)
}

/// The context for math layout.
// avenger: no region, which only boxes and external content use, and a count of item
// layouts against the budget.
struct MathContext<'v, 'e> {
    // External.
    engine: &'v mut Engine<'e>,
    // Mutable.
    fonts_stack: Vec<FontInstance>,
    fragments: MathRun,
    /// The number of items laid out so far.
    work: usize,
    /// The equation's span, for the budget's error.
    span: Span,
}

impl<'v, 'e> MathContext<'v, 'e> {
    /// Create a new math context.
    fn new(engine: &'v mut Engine<'e>, font: FontInstance, span: Span) -> Self {
        Self {
            engine,
            fonts_stack: vec![font],
            fragments: vec![],
            work: 0,
            span,
        }
    }

    /// Get the current base font.
    #[inline]
    fn font(&self) -> &FontInstance {
        // Will always be at least one font in the stack.
        self.fonts_stack.last().unwrap()
    }

    /// Push a fragment.
    fn push(&mut self, fragment: impl Into<MathFragment>) {
        self.fragments.push(fragment.into());
    }

    /// Push multiple fragments.
    fn extend(&mut self, fragments: impl IntoIterator<Item = MathFragment>) {
        self.fragments.extend(fragments);
    }

    /// Layout the given math item and return the resulting [`MathFragment`]s.
    fn layout_into_fragments(
        &mut self,
        item: &MathItem,
        styles: StyleChain,
    ) -> SourceResult<MathRun> {
        let start = self.fragments.len();
        self.layout_into_self(item, styles)?;
        Ok(self.fragments.drain(start..).collect())
    }

    /// Layout the given math item and return the resulting [`MathFragment`]s.
    fn layout_into_fragment(
        &mut self,
        item: &MathItem,
        styles: StyleChain,
    ) -> SourceResult<MathFragment> {
        let fragments = self.layout_into_fragments(item, styles)?;
        if fragments.len() == 1 {
            return Ok(fragments.into_iter().next().unwrap());
        }

        // Fragments without a math_size are ignored: the notion of size does
        // not apply to them, so their text-likeness is meaningless.
        let text_like = fragments
            .iter()
            .filter(|e| e.math_size().is_some())
            .all(|e| e.is_text_like());

        let styles = item.styles().unwrap_or(styles);
        let props = MathProperties::default(styles, Span::detached());
        let frame = fragments.into_frame();
        Ok(FrameFragment::new(&props, styles, frame)
            .with_text_like(text_like)
            .into())
    }

    fn layout_into_self(
        &mut self,
        item: &MathItem,
        styles: StyleChain,
    ) -> SourceResult<()> {
        let outer_styles = item.styles().unwrap_or(styles);
        let outer_font = outer_styles.get_ref(TextElem::font);

        for item in item.as_slice() {
            let styles = item.styles().unwrap_or(outer_styles);

            // Whilst this check isn't exact, it more or less suffices as a
            // change in font variant probably won't have an effect on metrics.
            if styles != outer_styles && styles.get_ref(TextElem::font) != outer_font {
                self.fonts_stack
                    .push(get_font(self.engine.world, styles, item.span())?);
                let scale_style = style_for_script_scale(self.font());
                layout_realized(item, self, styles.chain(&scale_style))?;
                self.fonts_stack.pop();
            } else {
                layout_realized(item, self, styles)?;
            }
        }

        Ok(())
    }
}

/// Lays out a single math item.
// upstream: crates/typst-layout/src/math/mod.rs::layout_realized @ v0.15.1
fn layout_realized(
    item: &MathItem,
    ctx: &mut MathContext,
    styles: StyleChain,
) -> SourceResult<()> {
    // avenger: count the layout against the equation's budget.
    ctx.work += 1;
    if ctx.work > MATH_WORK_LIMIT {
        bail!(ctx.span, "{MATH_TOO_COMPLEX}");
    }

    // Handle non-component items first.
    let comp = match item {
        MathItem::Component(comp) => comp,
        MathItem::Spacing(amount, font_size, _) => {
            ctx.push(MathFragment::Space(amount.at(*font_size)));
            return Ok(());
        }
        MathItem::Space => {
            ctx.push(MathFragment::Space(ctx.font().math().space_width.resolve(styles)));
            return Ok(());
        }
    };

    let props = &comp.props;

    // Insert left spacing.
    // avenger: no alignment forms, whose spacing only multiline layout handles.
    if let Some(lspace) = props.lspace
        && !lspace.is_zero()
    {
        let width = lspace.at(styles.resolve(TextElem::size));
        ctx.push(MathFragment::Space(width));
    }

    // Dispatch based on item kind to the appropriate layout function.
    match &comp.kind {
        MathKind::Glyph(item) => layout_glyph(item, ctx, styles, props)?,
        MathKind::Cancel(item) => layout_cancel(item, ctx, styles, props)?,
        MathKind::Radical(item) => layout_radical(item, ctx, styles, props)?,
        MathKind::Line(item) => layout_line(item, ctx, styles, props)?,
        MathKind::Accent(item) => layout_accent(item, ctx, styles, props)?,
        MathKind::Scripts(item) => layout_scripts(item, ctx, styles, props)?,
        MathKind::Primes(item) => layout_primes(item, ctx, styles, props)?,
        MathKind::Fraction(item) => layout_fraction(item, ctx, styles, props)?,
        MathKind::SkewedFraction(item) => {
            layout_skewed_fraction(item, ctx, styles, props)?
        }
        MathKind::Text(item) => layout_text(item, ctx, styles, props)?,
        MathKind::Number(item) => layout_number(item, ctx, styles, props)?,
        MathKind::Fenced(item) => layout_fenced(item, ctx, styles, props)?,
        MathKind::Group(_) => {
            let fragment = ctx.layout_into_fragment(item, styles)?;
            let italics = fragment.italics_correction();
            let accent_attach = fragment.accent_attach();
            ctx.push(
                FrameFragment::new(props, styles, fragment.into_frame())
                    .with_italics_correction(italics)
                    .with_accent_attach(accent_attach),
            );
        }
    }

    // Insert right spacing.
    if let Some(rspace) = props.rspace
        && !rspace.is_zero()
    {
        let width = rspace.at(styles.resolve(TextElem::size));
        ctx.push(MathFragment::Space(width));
    }

    Ok(())
}

/// Styles to add font constants to the style chain.
pub(crate) fn style_for_script_scale(font: &FontInstance) -> Style {
    EquationElem::script_scale
        .set((
            font.math().script_percent_scale_down,
            font.math().script_script_percent_scale_down,
        ))
        .wrap()
}

/// Get the current base font.
pub(crate) fn get_font(
    world: Tracked<dyn World + '_>,
    styles: StyleChain,
    span: Span,
) -> SourceResult<FontInstance> {
    let variant = variant(styles);
    let size = styles.resolve(TextElem::size);
    let variations = styles.get_cloned(TextElem::variations);
    families(styles)
        .find_map(|family| {
            world
                .book()
                .select(family.as_str(), variant)
                .and_then(|id| world.font(id))
                .filter(|_| family.covers().is_none())
                .map(|font| font.instantiate(variant, size, &variations))
        })
        .ok_or("no font could be found")
        .at(span)
}

/// Check if the top-level base font has a MATH table.
fn warn_non_math_font(font: &Font, engine: &mut Engine, span: Span) {
    if !font.info().flags.contains(FontFlags::MATH) {
        engine.sink.warn(warning!(
            span,
            "current font is not designed for math";
            hint: "rendering may be poor";
        ))
    }
}
