//! Ported from crates/typst-layout/src/inline/mod.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: inline layout of a single label line. `layout_label_line` stands in for
//! `layout_inline_impl`: it runs configuration, collection and preparation, then builds the one
//! line that ends at the mandatory break at the end of the text, and commits it at its natural
//! width, as `linebreak` and `finalize` do for the last line of a paragraph in an infinitely
//! wide region. Paragraphs, boxes, indents, line numbering and line breaking are out of scope.

mod collect;
mod deco;
mod line;
mod linebreak;
mod prepare;
mod shaping;
#[cfg(test)]
mod tests;

pub(crate) use self::shaping::SaturatingAs;
pub use self::shaping::{SharedShapingContext, create_shape_plan, get_font_and_covers};

use crate::typst_library::diag::{SourceResult, bail};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{
    Dir, FixAlignment, FixedAlignment, Frame, FrameItem, HAlignment,
};
use crate::typst_library::model::{JustificationLimits, ParElem};
use crate::typst_library::routines::Pair;
use crate::typst_library::text::TextElem;
use typst_syntax::Span;
use typst_utils::Numeric;

use self::collect::{Item, Segment, SpanMapper, collect};
use self::deco::decorate;
use self::line::{apply_shift, commit, line};
use self::linebreak::{Breakpoint, is_mandatory_break};
use self::prepare::{Preparation, prepare};
use self::shaping::{
    BEGIN_PUNCT_PAT, END_PUNCT_PAT, ShapedGlyph, ShapedText, cjk_punct_style,
    is_of_cj_script, shape_range,
};

/// Range of a substring of text.
type Range = std::ops::Range<usize>;

/// A laid-out label line.
// avenger: the frame `layout_inline` returns, with the line's text.
pub struct LabelLine {
    /// The line's frame.
    pub frame: Frame,
    /// The line's text in reading order: its items in logical order, and the text in laid-out
    /// inline content, such as equations, in drawing order.
    pub text: String,
}

/// Lays out realized content as a single line of inline layout.
// avenger: in place of `layout_inline` and `layout_inline_impl`.
pub fn layout_label_line<'a>(
    engine: &mut Engine,
    children: &[Pair<'a>],
    shared: StyleChain<'a>,
) -> SourceResult<LabelLine> {
    // Prepare configuration that is shared across the whole inline layout.
    let config = configuration(shared);

    // Collect all text into one string for BiDi analysis.
    let (text, segments, spans) = collect(children, engine, &config)?;

    // A label is a single line: evaluation turns line breaks in data into spaces and rejects
    // explicit ones, so none may remain before the end of the text.
    let end = text.trim_end_matches(is_mandatory_break).len();
    if text[..end].contains(is_mandatory_break) {
        bail!(Span::detached(), "a label must be a single line");
    }

    // Perform BiDi analysis and performs some preparation steps before we
    // proceed to line breaking.
    let p = prepare(engine, &config, &text, segments, spans)?;

    // The line spans the whole text.
    let line = line(engine, &p, 0..text.len(), Breakpoint::Mandatory, None);

    // Turn the line into a frame as wide as the line.
    let frame = commit(engine, &p, &line, line.width)?;
    Ok(LabelLine { frame, text: line_text(&line) })
}

/// The text of a line's items, in logical order, without the embeddings that `collect` puts
/// around text in another direction. The frame keeps visual order within each run of
/// right-to-left text, but a shaped run's text is logical.
fn line_text(line: &line::Line) -> String {
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
        // avenger: a label line is never justified.
        justify: false,
        justification_limits: shared.get(ParElem::justification_limits),
        // avenger: `AlignElem`'s default, start alignment. It moves a line that is exactly as
        // wide as its content only by the hanging punctuation `commit` adds.
        align: HAlignment::Start.fix(dir),
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
