//! Ported from crates/typst-layout/src/inline/finalize.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's lines have no fractional spacing or hanging indent, and they are committed
//! without a locator.

use crate::typst_library::layout::Size;

use super::*;

/// Turns the selected lines into frames.
// avenger: one frame per line, in place of a `Fragment`.
pub fn finalize(
    engine: &mut Engine,
    p: &Preparation,
    lines: &[Line],
    region: Size,
    expand: bool,
) -> SourceResult<Vec<Frame>> {
    // Determine the resulting width: Full width of the region if we should
    // expand, fit-to-width otherwise.
    let width = if !region.x.is_finite() || !expand {
        region
            .x
            .min(lines.iter().map(|line| line.width).max().unwrap_or_default())
    } else {
        region.x
    };

    // Stack the lines into one frame per region.
    lines.iter().map(|line| commit(engine, p, line, width)).collect()
}
