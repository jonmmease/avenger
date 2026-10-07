//! Ported from crates/typst-layout/src/math/text.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: text in math is laid out as a label of unbounded width, as upstream's unbounded
//! inline layout lays it out.

use codex::styling::{MathStyle, to_style};
use ecow::EcoString;
use unicode_math_class::MathClass;

use super::MathContext;
use super::fragment::{FrameFragment, GlyphFragment};
use super::run::MathFragmentsExt;
use crate::typst_layout::inline::{LineOptions, layout_label};
use crate::typst_library::diag::SourceResult;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{Abs, Size};
use crate::typst_library::math::ir::{GlyphItem, MathProperties, NumberItem, TextItem};
use crate::typst_library::math::{EquationElem, style_dtls, style_flac};
use crate::typst_library::text::{FontInstance, TextElem};

/// Lays out a [`TextItem`].
// upstream: crates/typst-layout/src/math/text.rs::layout_text @ v0.15.1
pub fn layout_text(
    item: &TextItem,
    ctx: &mut MathContext,
    styles: StyleChain,
    props: &MathProperties,
) -> SourceResult<()> {
    let text = &item.text;
    let span = props.span;
    let elem = TextElem::packed(text).spanned(span);

    // There isn't a natural width for a paragraph in a math environment;
    // because it will be placed somewhere probably not at the left margin
    // it will overflow. So emulate an `hbox` instead and allow the
    // paragraph to extend as far as needed.
    let region = Size::splat(Abs::inf());
    let children = [(&elem, styles)];
    let options = LineOptions::default();
    let frame =
        layout_label(ctx.engine, &children, styles, region, false, options)?.frame;
    ctx.push(FrameFragment::new(props, styles, frame).with_text_like(true));
    Ok(())
}

/// Lays out a [`NumberItem`].
pub fn layout_number(
    item: &NumberItem,
    ctx: &mut MathContext,
    styles: StyleChain,
    props: &MathProperties,
) -> SourceResult<()> {
    let text = &item.text;
    let span = props.span;
    // Small optimization for numbers. Note that this lays out slightly
    // differently to normal text and is worth re-evaluating in the future.
    let mut fragments = vec![];
    for c in text.chars() {
        if let Some(glyph) = GlyphFragment::synthetic(ctx.engine, styles, c, span) {
            fragments.push(glyph.into());
        }
    }
    let frame = fragments.into_frame();
    ctx.push(FrameFragment::new(props, styles, frame).with_text_like(true));
    Ok(())
}

/// Layout a single character in the math font.
pub fn layout_glyph(
    item: &GlyphItem,
    ctx: &mut MathContext,
    styles: StyleChain,
    props: &MathProperties,
) -> SourceResult<()> {
    let flac;
    let styles = if item.flac.get() {
        flac = style_flac();
        styles.chain(&flac)
    } else {
        styles
    };

    let dtls;
    let (styles, text): (_, EcoString) =
        if item.text.chars().any(|c| try_dotless(c).is_some())
            && has_dtls_feat(ctx.font())
        {
            dtls = style_dtls();
            let variant = styles.get(EquationElem::variant);
            let bold = styles.get(EquationElem::bold);
            let italic = styles.get(EquationElem::italic);
            let text = item
                .text
                .chars()
                .flat_map(|mut c| {
                    if let Some(d) = try_dotless(c) {
                        c = d;
                    }
                    to_style(c, MathStyle::select(c, variant, bold, italic))
                })
                .collect();
            (styles.chain(&dtls), text)
        } else {
            (styles, item.text.clone())
        };

    if let Some(mut glyph) = GlyphFragment::new(
        ctx.engine,
        &text,
        &item.stretch.get(),
        styles,
        props,
        item.class,
    ) {
        if glyph.class == MathClass::Large {
            // TeXbook p 155. Large operators are always vertically centered on
            // the axis.
            glyph.center_on_axis();
        }

        glyph.class = props.class();
        ctx.push(glyph);
    }
    Ok(())
}

/// Whether the given font has the dtls OpenType feature.
fn has_dtls_feat(font: &FontInstance) -> bool {
    font.ttf()
        .tables()
        .gsub
        .and_then(|gsub| gsub.features.index(ttf_parser::Tag::from_bytes(b"dtls")))
        .is_some()
}

/// The non-dotless version of a dotless character that can be used with the
/// `dtls` OpenType feature.
fn try_dotless(c: char) -> Option<char> {
    match c {
        'ı' | '𝚤' => Some('i'),
        'ȷ' | '𝚥' => Some('j'),
        _ => None,
    }
}
