//! Ported from crates/typst-layout/src/math/fenced.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a fence's body is never shared, since only fences split across lines share one,
//! so the body is always measured from its own layout.

use crate::typst_library::diag::SourceResult;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{Abs, Axis};
use crate::typst_library::math::ir::{FencedItem, MathProperties};

use super::MathContext;
use super::fragment::MathFragment;

/// Lays out a [`FencedItem`].
// upstream: crates/typst-layout/src/math/fenced.rs::layout_fenced @ v0.15.1
pub fn layout_fenced(
    item: &FencedItem,
    ctx: &mut MathContext,
    styles: StyleChain,
    // avenger: upstream only names the timing span with the properties.
    _props: &MathProperties,
) -> SourceResult<()> {
    // Compute relative_to for delimiter sizing.
    let initial_body = ctx.layout_into_fragments(&item.body, styles)?;
    let body_styles = item.body.styles().unwrap_or(styles);
    let relative_to =
        relative_to_from_fragments(&initial_body, ctx, body_styles, item.balanced);

    // Set stretch info for stretched mid items.
    let mut has_mid_stretched = false;
    for body_item in item.body.as_slice() {
        if body_item.mid_stretched().is_some_and(|x| x) {
            has_mid_stretched = true;
            body_item.set_stretch_relative_to(relative_to, Axis::Y);
        }
    }

    // Layout the opening delimiter if present.
    if let Some(open) = &item.open {
        open.set_stretch_relative_to(relative_to, Axis::Y);
        let open = ctx.layout_into_fragment(open, styles)?;
        ctx.push(open);
    }

    // Check if the body needs re-layout, since stretch info was updated.
    let body = if !has_mid_stretched {
        initial_body
    } else {
        ctx.layout_into_fragments(&item.body, styles)?
    };
    ctx.extend(body);

    // Layout the closing delimiter if present.
    if let Some(close) = &item.close {
        close.set_stretch_relative_to(relative_to, Axis::Y);
        let close = ctx.layout_into_fragment(close, styles)?;
        ctx.push(close);
    }

    Ok(())
}

fn relative_to_from_fragments(
    fragments: &[MathFragment],
    ctx: &MathContext,
    styles: StyleChain,
    balanced: bool,
) -> Abs {
    fragments
        .iter()
        .map(|f| {
            if balanced {
                let (font, size) = f.font(ctx, styles);
                let axis = font.math().axis_height.at(size);
                2.0 * (f.ascent() - axis).max(f.descent() + axis)
            } else {
                f.height()
            }
        })
        .max()
        .unwrap_or_default()
}
