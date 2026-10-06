//! Ported from crates/typst-layout/src/math/run.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's math is one row without alignment points, so there is no multiline
//! layout: no row stacking, alignment columns or frame builder, and a run becomes a frame
//! as a single cell.

use unicode_math_class::MathClass;

use super::fragment::MathFragment;
use crate::typst_library::layout::{Abs, Frame, InlineItem, Point, Size};

/// A list of math fragments between alignment points and/or linebreaks.
///
/// For multiline equations this represents a distinct "cell", the list of
/// fragments at a specific row and column. For tables, this represents a
/// "sub-column", one of the parts that make up a cell in a table since
/// alignment points can be used to align fragments within a cell for an
/// individual column.
pub type MathRun = Vec<MathFragment>;

/// Measure the ascent and descent of a row.
// upstream: crates/typst-layout/src/math/run.rs::measure_row @ v0.15.1
pub fn measure_row(cells: &[MathRun]) -> (Abs, Abs) {
    cells
        .iter()
        .flat_map(|sc| sc.iter())
        .map(|f| (f.ascent(), f.descent()))
        .reduce(|(a1, d1), (a2, d2)| (a1.max(a2), d1.max(d2)))
        .unwrap_or_default()
}

pub trait MathFragmentsExt {
    fn into_frame(self) -> Frame;
    fn into_par_items(self) -> Vec<InlineItem>;
}

impl MathFragmentsExt for MathRun {
    // upstream: crates/typst-layout/src/math/run.rs::MathFragmentsExt::into_frame @ v0.15.1
    fn into_frame(self) -> Frame {
        row_into_line_frame(self)
    }

    /// Convert this run of math fragments into a vector of inline items for
    /// paragraph layout. Creates multiple fragments when relation or binary
    /// operators are present to allow for line-breaking opportunities later.
    fn into_par_items(self) -> Vec<InlineItem> {
        let mut items = vec![];

        let mut x = Abs::zero();
        let mut ascent = Abs::zero();
        let mut descent = Abs::zero();
        let mut frame = Frame::soft(Size::zero());
        let mut empty = true;

        let finalize_frame = |frame: &mut Frame, x, ascent, descent| {
            frame.set_size(Size::new(x, ascent + descent));
            frame.set_baseline(Abs::zero());
            frame.translate(Point::with_y(ascent));
        };

        let mut space_is_visible = false;

        let is_space = |f: &MathFragment| matches!(f, MathFragment::Space(_));
        let is_line_break_opportunity = |class, next_fragment| match class {
            // Don't split when two relations are in a row or when preceding a
            // closing parenthesis.
            MathClass::Binary => next_fragment != Some(MathClass::Closing),
            MathClass::Relation => {
                !matches!(next_fragment, Some(MathClass::Relation | MathClass::Closing))
            }
            _ => false,
        };

        let mut iter = self.into_iter().peekable();
        while let Some(fragment) = iter.next() {
            if space_is_visible && is_space(&fragment) {
                items.push(InlineItem::Space(fragment.width(), true));
                continue;
            }

            let class = fragment.class();
            let y = fragment.ascent();

            ascent.set_max(y);
            descent.set_max(fragment.descent());

            let pos = Point::new(x, -y);
            x += fragment.width();
            frame.push_frame(pos, fragment.into_frame());
            empty = false;

            // Split our current frame when we encounter a binary operator or
            // relation so that there is a line-breaking opportunity.
            if is_line_break_opportunity(class, iter.peek().map(|f| f.class())) {
                let mut frame_prev =
                    std::mem::replace(&mut frame, Frame::soft(Size::zero()));

                finalize_frame(&mut frame_prev, x, ascent, descent);
                items.push(InlineItem::Frame(frame_prev));
                empty = true;

                x = Abs::zero();
                ascent = Abs::zero();
                descent = Abs::zero();

                space_is_visible = true;
                if let Some(f_next) = iter.peek()
                    && !is_space(f_next)
                {
                    items.push(InlineItem::Space(Abs::zero(), true));
                }
            } else {
                space_is_visible = false;
            }
        }

        // Don't use `frame.is_empty()` because even an empty frame can
        // contribute width (if it had hidden content).
        if !empty {
            finalize_frame(&mut frame, x, ascent, descent);
            items.push(InlineItem::Frame(frame));
        }

        items
    }
}

/// Build a frame from a row's cell fragments positioned at alignment points.
// upstream: crates/typst-layout/src/math/run.rs::row_into_line_frame @ v0.15.1
// avenger: a row is one cell, without alignment points, measured from its fragments.
fn row_into_line_frame(cell: MathRun) -> Frame {
    let (ascent, descent) = measure_row(std::slice::from_ref(&cell));

    let mut frame = Frame::soft(Size::new(Abs::zero(), ascent + descent));
    frame.set_baseline(ascent);

    let mut x = Abs::zero();
    for frag in cell {
        let y = ascent - frag.ascent();
        let w = frag.width();
        frame.push_frame(Point::new(x, y), frag.into_frame());
        x += w;
    }

    frame.size_mut().x = x;
    frame
}
