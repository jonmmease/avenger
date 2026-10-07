//! A label's line limit and ellipses, which Typst has no counterpart for.
//!
//! avenger: `truncate` runs between line breaking and finalization. It keeps a label's first
//! lines and, with an ellipsis, rebuilds each line that is wider than the width, and the last
//! line when lines are dropped, to end in "…".

use std::collections::HashMap;

use unicode_segmentation::UnicodeSegmentation;

use super::LineOptions;
use super::collect::Item;
use super::line::{Line, LogicalIndex, hanging_sign, line};
use super::linebreak::Breakpoint;
use super::prepare::Preparation;
use super::shaping::{ShapedText, shape};
use crate::typst_library::engine::Engine;
use crate::typst_library::foundations::StyleChain;
use crate::typst_library::layout::{Abs, Dir};
use crate::typst_library::text::{TextElem, is_default_ignorable};

/// Keeps the lines that the limit allows, cuts the lines that need it, and returns whether text
/// was cut.
///
/// Text is cut when dropped lines show anything, and, with an ellipsis, when a line is wider
/// than the width. With an ellipsis, each such line, and the last line when dropped lines show
/// anything, ends in "…" and is shortened, at grapheme boundaries, until it fits with the
/// ellipsis. With no width to fit, the last line is kept whole. A width too narrow for the
/// ellipsis alone leaves just the ellipsis.
pub fn truncate<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    lines: &mut Vec<Line<'a>>,
    options: LineOptions,
    width: Abs,
    shared: StyleChain<'a>,
) -> bool {
    let kept = options
        .max_lines
        .map_or(lines.len(), |max| max.get().min(lines.len()));
    let dropped = lines[kept..]
        .iter()
        .any(|line| p.text[line.range.clone()].chars().any(visible));
    lines.truncate(kept);
    if !options.ellipsis {
        return dropped;
    }

    // A hanging sign lies outside the width.
    let fits = |line: &Line<'a>, extra: Abs| {
        let (left, right) = hanging_sign(p, line);
        width.fits(line.width - left - right + extra)
    };
    let mut cut = dropped;
    // In reverse, so that each line is rebuilt after its predecessor as line breaking left it.
    for i in (0..lines.len()).rev() {
        let last = i + 1 == lines.len();
        if fits(&lines[i], Abs::zero()) && !(last && dropped) {
            continue;
        }
        let pred = i.checked_sub(1).map(|i| &lines[i]);
        let shortened = shorten(engine, p, &lines[i], pred, last, &fits, shared);
        lines[i] = shortened;
        cut = true;
    }
    cut
}

/// A line shortened to the longest grapheme boundary after something visible at which it fits
/// with an ellipsis, or to its start, and ended with the ellipsis.
fn shorten<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    original: &Line<'a>,
    pred: Option<&Line<'a>>,
    last: bool,
    fits: &impl Fn(&Line<'a>, Abs) -> bool,
    shared: StyleChain<'a>,
) -> Line<'a> {
    let start = original.range.start;
    let text = &p.text[original.range.clone()];
    // The ends the line can be cut at, shortest first.
    let ends: Vec<usize> = std::iter::once(start)
        .chain(
            text.grapheme_indices(true)
                .map(|(i, grapheme)| start + i + grapheme.len())
                .filter(|&end| ends_visibly(&p.text[start..end])),
        )
        .collect();
    let attempt = |end| line(engine, p, start..end, Breakpoint::Normal, pred);
    // The ellipsis takes the style of the text before it, so it is shaped once per text item.
    let mut widths = HashMap::new();
    let mut ellipsis_width = |end| {
        *widths
            .entry(preceding_text(p, end))
            .or_insert_with(|| ellipsis(engine, p, end, shared).width())
    };

    // A line grows with its end, so the longest end that fits is found by bisection. The start
    // always counts as fitting.
    let (mut low, mut high) = (0, ends.len() - 1);
    while low < high {
        let mid = (low + high).div_ceil(2);
        if fits(&attempt(ends[mid]), ellipsis_width(ends[mid])) {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    let end = ends[low];
    let mut line = attempt(end);
    let ellipsis = ellipsis(engine, p, end, shared);

    // The ellipsis is a neutral at the end of the visible text, so it takes the paragraph's
    // direction, as the bidirectional algorithm resolves trailing neutrals.
    line.width += ellipsis.width();
    let ellipsis = Item::Text(ellipsis);
    if p.config.dir == Dir::RTL {
        line.items.insert(0, (LogicalIndex::ELLIPSIS, ellipsis.into()));
    } else {
        line.items.push(ellipsis, LogicalIndex::ELLIPSIS);
    }

    // The line still ends where line breaking ended it, so an explicit break still ends its
    // text, and a wrapped line's space still separates its text from the next line's.
    line.breakpoint = original.breakpoint;
    if !last && original.breakpoint == Breakpoint::Normal {
        line.cut_space = &text[text.trim_end().len()..];
    }
    line
}

/// Whether a character shows: it isn't whitespace, a soft hyphen or a default ignorable.
fn visible(c: char) -> bool {
    !c.is_whitespace() && c != '\u{ad}' && !is_default_ignorable(c)
}

/// Whether text ends in something visible, which an ellipsis can follow directly.
fn ends_visibly(text: &str) -> bool {
    text.chars().next_back().is_some_and(visible)
}

/// The index of the last text item that starts before a text offset.
fn preceding_text(p: &Preparation, offset: usize) -> Option<usize> {
    p.items
        .iter()
        .take_while(|(range, _)| range.start < offset)
        .enumerate()
        .filter(|(_, (_, item))| item.text().is_some())
        .map(|(i, _)| i)
        .last()
}

/// An ellipsis at a text offset, in the style of the last text before it, or the label's
/// shared style when no text precedes it.
fn ellipsis<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    offset: usize,
    shared: StyleChain<'a>,
) -> ShapedText<'a> {
    let styles = preceding_text(p, offset)
        .and_then(|i| p.items[i].1.text())
        .map_or(shared, |text| text.styles);
    let lang = styles.get(TextElem::lang);
    let region = styles.get(TextElem::region);
    shape(engine, offset, "\u{2026}", styles, p.config.dir, lang, region)
}
