//! A label's line limit, which Typst has no counterpart for.
//!
//! avenger: `truncate` runs between line breaking and finalization. It keeps a label's first
//! lines and, with an ellipsis, rebuilds the last one to end in "…" when text is cut: when
//! lines are dropped, or when the last line is wider than the width.

use std::num::NonZeroUsize;

use unicode_segmentation::UnicodeSegmentation;

use super::collect::Item;
use super::line::{Line, LogicalIndex, line};
use super::linebreak::Breakpoint;
use super::prepare::Preparation;
use super::shaping::{ShapedText, shape};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{Abs, Dir};
use crate::typst_library::text::{TextElem, is_default_ignorable};

/// How many lines a label keeps, and whether an ellipsis marks cut text.
#[derive(Debug, Copy, Clone, Default)]
pub struct LineLimit {
    /// The most lines to keep, or all of them.
    pub max_lines: Option<NonZeroUsize>,
    /// Whether the last line ends in an ellipsis when text is cut.
    pub ellipsis: bool,
}

/// Keeps the lines that the limit allows, and returns whether text was cut.
///
/// Text is cut when dropped lines show anything, and, with an ellipsis, when the last line is
/// wider than the width. The last line then ends in "…" and is shortened, at
/// grapheme boundaries, until it fits with the ellipsis. With no width to fit, it is kept
/// whole. A width too narrow for the ellipsis alone leaves just the ellipsis.
pub fn truncate<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    lines: &mut Vec<Line<'a>>,
    limit: LineLimit,
    width: Abs,
    shared: StyleChain<'a>,
) -> bool {
    let kept = limit.max_lines.map_or(lines.len(), |max| max.get().min(lines.len()));
    let dropped = lines[kept..]
        .iter()
        .any(|line| p.text[line.range.clone()].chars().any(visible));
    lines.truncate(kept);
    let Some(last) = lines.last() else { return dropped };
    let cut = dropped || !width.fits(last.width);
    if !(limit.ellipsis && cut) {
        return dropped;
    }

    let start = last.range.start;
    let text = &p.text[last.range.clone()];
    let pred = lines.len().checked_sub(2).map(|i| &lines[i]);
    let ends = text
        .grapheme_indices(true)
        .rev()
        .map(|(i, grapheme)| start + i + grapheme.len())
        .filter(|&end| ends_visibly(&p.text[start..end]))
        .chain([start]);
    let mut shortened = None;
    for end in ends {
        let attempt = line(engine, p, start..end, Breakpoint::Normal, pred);
        let ellipsis = ellipsis(engine, p, end, shared);
        if width.fits(attempt.width + ellipsis.width()) || end == start {
            shortened = Some((attempt, ellipsis));
            break;
        }
    }
    let (mut line, ellipsis) = shortened.expect("the empty line ends the candidates");

    // The ellipsis is a neutral at the end of the visible text, so it takes the paragraph's
    // direction, as the bidirectional algorithm resolves trailing neutrals.
    line.width += ellipsis.width();
    let ellipsis = Item::Text(ellipsis);
    if p.config.dir == Dir::RTL {
        line.items.insert(0, (LogicalIndex::ELLIPSIS, ellipsis.into()));
    } else {
        line.items.push(ellipsis, LogicalIndex::ELLIPSIS);
    }
    *lines.last_mut().expect("a line") = line;
    true
}

/// Whether a character shows: it isn't whitespace, a soft hyphen or a default ignorable.
fn visible(c: char) -> bool {
    !c.is_whitespace() && c != '\u{ad}' && !is_default_ignorable(c)
}

/// Whether text ends in something visible, which an ellipsis can follow directly.
fn ends_visibly(text: &str) -> bool {
    text.chars().next_back().is_some_and(visible)
}

/// An ellipsis at a text offset, in the style of the last text before it, or the label's
/// shared style when no text precedes it.
fn ellipsis<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    offset: usize,
    shared: StyleChain<'a>,
) -> ShapedText<'a> {
    let styles = p
        .items
        .iter()
        .take_while(|(range, _)| range.start < offset)
        .filter_map(|(_, item)| item.text())
        .last()
        .map_or(shared, |text| text.styles);
    let lang = styles.get(TextElem::lang);
    let region = styles.get(TextElem::region);
    shape(engine, offset, "\u{2026}", styles, p.config.dir, lang, region)
}
