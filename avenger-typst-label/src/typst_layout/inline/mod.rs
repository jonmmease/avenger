//! Ported from crates/typst-layout/src/inline/mod.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: inline layout of a label. `layout_label` stands in for `layout_inline_impl`: it runs
//! configuration, collection, preparation, line breaking, the label's line limit and
//! finalization, then stacks the lines as flow stacks the lines of a box's body. Paragraphs, boxes, indents and line numbering are
//! out of scope.

mod collect;
mod deco;
mod finalize;
mod line;
mod linebreak;
mod prepare;
mod shaping;
#[cfg(test)]
mod tests;
mod truncate;

pub(crate) use self::shaping::SaturatingAs;
pub use self::shaping::{SharedShapingContext, create_shape_plan, get_font_and_covers};
pub use self::truncate::LineLimit;

use crate::typst_library::diag::SourceResult;
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{
    Abs, AlignElem, Dir, FixedAlignment, Frame, FrameItem, Point, Size,
};
use crate::typst_library::model::{JustificationLimits, ParElem};
use crate::typst_library::routines::Pair;
use crate::typst_library::text::TextElem;
use typst_utils::Numeric;

use self::collect::{Item, Segment, SpanMapper, collect};
use self::deco::decorate;
use self::finalize::finalize;
use self::line::{Line, apply_shift, commit};
use self::linebreak::{Breakpoint, linebreak};
use self::prepare::{Preparation, prepare};
use self::shaping::{
    BEGIN_PUNCT_PAT, END_PUNCT_PAT, ShapedGlyph, ShapedText, cjk_punct_style,
    is_of_cj_script, shape_range,
};
use self::truncate::truncate;

/// Range of a substring of text.
type Range = std::ops::Range<usize>;

/// A laid-out label.
// avenger: the frame that `layout_inline` lays out and flow stacks, with the label's text.
pub struct LabelLayout {
    /// The label's frame: its lines, stacked.
    pub frame: Frame,
    /// The label's text in reading order: each line's items in logical order, and the text in
    /// laid-out inline content, such as equations, in drawing order. A newline follows each
    /// line that a mandatory breakpoint ends, except the last.
    pub text: String,
    /// Whether the line limit cut text.
    pub truncated: bool,
}

/// Lays out realized content as a label: its lines, broken to fit the region's width, and
/// stacked.
// avenger: in place of `layout_inline` and `layout_inline_impl`.
pub fn layout_label<'a>(
    engine: &mut Engine,
    children: &[Pair<'a>],
    root: StyleChain<'a>,
    region: Size,
    expand: bool,
    limit: LineLimit,
) -> SourceResult<LabelLayout> {
    // The styles that all the content shares, as flow lays out the body of a box.
    let shared =
        StyleChain::trunk(children.iter().map(|&(_, styles)| styles)).unwrap_or(root);

    // Prepare configuration that is shared across the whole inline layout.
    let config = configuration(shared);

    // Collect all text into one string for BiDi analysis.
    let (text, segments, spans) = collect(children, engine, &config)?;

    // Perform BiDi analysis and performs some preparation steps before we
    // proceed to line breaking.
    let p = prepare(engine, &config, &text, segments, spans)?;

    // Break the text into lines.
    let mut lines = linebreak(engine, &p, region.x);

    // Keep the lines that the limit allows.
    let truncated = truncate(engine, &p, &mut lines, limit, region.x, shared);

    // Turn the selected lines into frames.
    let frames = finalize(engine, &p, &lines, region, expand)?;
    Ok(LabelLayout {
        frame: stack(frames, shared.resolve(ParElem::leading)),
        text: lines_text(&lines),
        truncated,
    })
}

/// Stacks a label's lines into one frame, with the leading between them and the first line's
/// baseline.
// upstream: crates/typst-layout/src/flow/collect.rs::Collector::lines @ v0.15.1, with the
// placement of `flow/distribute.rs`. The lines are all as wide as the label, so each sits at
// its start.
fn stack(frames: Vec<Frame>, leading: Abs) -> Frame {
    let width = frames.iter().map(Frame::width).max().unwrap_or_default();
    let height = frames.iter().map(Frame::height).sum::<Abs>()
        + leading * frames.len().saturating_sub(1) as f64;
    let mut output = Frame::soft(Size::new(width, height));
    let mut y = Abs::zero();
    for (i, frame) in frames.into_iter().enumerate() {
        if i == 0 {
            output.set_baseline(frame.baseline());
        }
        let advance = frame.height() + leading;
        output.push_frame(Point::with_y(y), frame);
        y += advance;
    }
    output
}

/// The lines' text, with a newline after each line that a mandatory breakpoint ends, except the
/// last.
fn lines_text(lines: &[Line]) -> String {
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i > 0 && lines[i - 1].breakpoint == Breakpoint::Mandatory {
            text.push('\n');
        }
        text.push_str(&line_text(line));
    }
    text
}

/// The text of a line's items, in logical order, without the embeddings that `collect` puts
/// around text in another direction. The frame keeps visual order within each run of
/// right-to-left text, but a shaped run's text is logical.
fn line_text(line: &Line) -> String {
    fn frame_text(frame: &Frame, text: &mut String) {
        for (_, item) in frame.items() {
            match item {
                FrameItem::Group(group) => frame_text(&group.frame, text),
                FrameItem::Text(item) => text.push_str(&item.text),
                FrameItem::Shape(..) => {}
            }
        }
    }

    let mut items: Vec<_> = line.items.indexed_iter().collect();
    items.sort_by_key(|(index, _)| *index);
    let mut text = String::new();
    for (_, item) in items {
        match &**item {
            Item::Text(shaped) => text.extend(
                shaped
                    .text
                    .chars()
                    .filter(|c| !matches!(c, '\u{202A}' | '\u{202B}' | '\u{202C}')),
            ),
            Item::Frame(frame) => frame_text(frame, &mut text),
            Item::Absolute(..) | Item::Skip(_) => {}
        }
    }
    text
}

/// Determine the inline layout's configuration.
fn configuration(shared: StyleChain) -> Config {
    let dir = shared.resolve(TextElem::dir);

    Config {
        // avenger: a label's paragraph is never justified.
        justify: false,
        justification_limits: shared.get(ParElem::justification_limits),
        align: shared.get(AlignElem::alignment).fix(dir).x,
        dir,
        fallback: shared.get(TextElem::fallback),
        cjk_latin_spacing: shared.get(TextElem::cjk_latin_spacing).is_auto(),
    }
}

/// Shared configuration for the whole inline layout.
// avenger: without line breaking, indents and line numbering.
struct Config {
    /// Whether to justify text.
    justify: bool,
    /// Settings for justification.
    justification_limits: JustificationLimits,
    /// The resolved horizontal alignment.
    align: FixedAlignment,
    /// The dominant direction.
    dir: Dir,
    /// Whether font fallback is enabled.
    fallback: bool,
    /// Whether to add spacing between CJK and Latin characters.
    cjk_latin_spacing: bool,
}
