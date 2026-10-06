//! Ported from crates/typst-layout/src/math/mod.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: so far, the font helpers of inline equation layout.

use crate::typst_library::World;
use crate::typst_library::diag::{At, SourceResult};
use crate::typst_library::engine::Tracked;
use crate::typst_library::foundations::{Style, StyleChain};
use crate::typst_library::math::{EquationElem, families};
use crate::typst_library::text::{FontInstance, TextElem, variant};
use crate::typst_syntax::Span;

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
