//! Ported from crates/typst-layout/src/inline/collect.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label line has no fractional spacing, boxes or introspection tags, and no
//! indents. The span mapper also records where each child's text came from in the source, so
//! that glyphs get exact source ranges.

use crate::typst_library::World;
use crate::typst_library::diag::warning;
use crate::typst_library::layout::{Abs, Dir, Frame};
use crate::typst_library::routines::Pair;
use crate::typst_library::text::{
    LinebreakElem, SmartQuoteElem, SmartQuoter, SmartQuotes, SpaceElem, TextElem,
    is_default_ignorable,
};
use crate::typst_syntax::{Span, SpanKind};

use super::*;

// The characters by which spacing, inline content and pins are replaced in the
// full text.
const SPACING_REPLACE: &str = " "; // Space
const OBJ_REPLACE: &str = "\u{FFFC}"; // Object Replacement Character

// Unicode BiDi control characters.
const LTR_EMBEDDING: &str = "\u{202A}";
const RTL_EMBEDDING: &str = "\u{202B}";
const POP_EMBEDDING: &str = "\u{202C}";
// avenger: the isolates around inline elements arrive with the math layout.

/// A prepared item in a inline layout.
#[derive(Debug)]
pub enum Item<'a> {
    /// A shaped text run with consistent style and direction.
    Text(ShapedText<'a>),
    /// Absolute spacing between other items, and whether it is weak.
    Absolute(Abs, bool),
    // avenger: no `Fractional` spacing, which needs a region to fill.
    /// Layouted inline-level content.
    Frame(Frame),
    // avenger: no `Tag`, since labels have no introspection.
    /// An item that is invisible and needs to be skipped, e.g. a Unicode
    /// isolate.
    Skip(&'static str),
}

impl<'a> Item<'a> {
    /// If this a text item, return it.
    pub fn text(&self) -> Option<&ShapedText<'a>> {
        match self {
            Self::Text(shaped) => Some(shaped),
            _ => None,
        }
    }

    /// If this a text item, return it mutably.
    pub fn text_mut(&mut self) -> Option<&mut ShapedText<'a>> {
        match self {
            Self::Text(shaped) => Some(shaped),
            _ => None,
        }
    }

    /// Return the textual representation of this item: Either just itself (for
    /// a text item) or a replacement string (for any other item).
    pub fn textual(&self) -> &str {
        match self {
            Self::Text(shaped) => shaped.text,
            Self::Absolute(_, _) => SPACING_REPLACE,
            Self::Frame(_) => OBJ_REPLACE,
            Self::Skip(s) => s,
        }
    }

    /// The text length of the item.
    pub fn textual_len(&self) -> usize {
        self.textual().len()
    }

    /// The natural layouted width of the item.
    pub fn natural_width(&self) -> Abs {
        match self {
            Self::Text(shaped) => shaped.width(),
            Self::Absolute(v, _) => *v,
            Self::Frame(frame) => frame.width(),
            Self::Skip(_) => Abs::zero(),
        }
    }
}

/// An item or not-yet shaped text. We can't shape text until we have collected
/// all items because only then we can compute BiDi, and we need to split shape
/// runs at level boundaries.
#[derive(Debug)]
pub enum Segment<'a> {
    /// One or multiple collapsed text children. Stores how long the segment is
    /// (in bytes of the full text string).
    Text(usize, StyleChain<'a>),
    /// An already prepared item.
    Item(Item<'a>),
}

impl Segment<'_> {
    /// The text length of the item.
    pub fn textual_len(&self) -> usize {
        match self {
            Self::Text(len, _) => *len,
            Self::Item(item) => item.textual_len(),
        }
    }
}

/// Collects all text into one string and a collection of segments that
/// correspond to pieces of that string. This also performs string-level
/// preprocessing like case transformations.
// avenger: no locator or region, since a label line has no introspection and no region to
// fill, and no indents.
pub fn collect<'a>(
    children: &[Pair<'a>],
    engine: &mut Engine<'_>,
    config: &Config,
) -> SourceResult<(String, Vec<Segment<'a>>, SpanMapper)> {
    let mut collector = Collector::new(2 + children.len());
    let mut quoter = SmartQuoter::new();

    for &(child, styles) in children {
        let prev_len = collector.full.len();
        // avenger: where the child's own text lies in the full text, without
        // embedding characters, and the text it was made from.
        let mut payload = None;
        let mut original = None;

        if child.is::<SpaceElem>() {
            collector.push_text(" ", styles);
        } else if let Some(elem) = child.to_packed::<TextElem>() {
            collector.build_text(styles, |full| {
                let dir = styles.resolve(TextElem::dir);
                if dir != config.dir {
                    // Insert "Explicit Directional Embedding".
                    match dir {
                        Dir::LTR => full.push_str(LTR_EMBEDDING),
                        Dir::RTL => full.push_str(RTL_EMBEDDING),
                        _ => {}
                    }
                }

                let start = full.len();
                if let Some(case) = styles.get(TextElem::case) {
                    full.push_str(&case.apply(&elem.text));
                } else {
                    full.push_str(&elem.text);
                }
                payload = Some(start..full.len());
                original = Some(elem.text.as_str());

                if dir != config.dir {
                    // Insert "Pop Directional Formatting".
                    full.push_str(POP_EMBEDDING);
                }
            });
        // avenger: no `HElem`, which labels cannot produce.
        } else if let Some(elem) = child.to_packed::<LinebreakElem>() {
            collector.push_text(
                if elem.justify.get(styles) { "\u{2028}" } else { "\n" },
                styles,
            );
        } else if let Some(elem) = child.to_packed::<SmartQuoteElem>() {
            let double = elem.double.get(styles);
            if elem.enabled.get(styles) {
                let quotes = SmartQuotes::get(
                    elem.quotes.get_ref(styles),
                    styles.get(TextElem::lang),
                    styles.get(TextElem::region),
                    elem.alternative.get(styles),
                );
                let before =
                    collector.full.chars().rev().find(|&c| !is_default_ignorable(c));
                let quote = quoter.quote(before, &quotes, double);
                collector.push_text(quote, styles);
            } else {
                collector.push_text(SmartQuotes::fallback(double), styles);
            }
        // avenger: inline elements (equations) arrive with the math layout. There are no
        // boxes or tags.
        } else {
            // Non-paragraph inline layout should never trigger this since it
            // only won't be triggered if we see any non-inline content.
            engine.sink.warn(warning!(
                child.span(),
                "{} may not occur inside of a paragraph and was ignored",
                child.func().name(),
            ));
        };

        let len = collector.full.len() - prev_len;
        collector.spans.push(len, child.span());

        // avenger: record where the child's text came from.
        let payload = payload.unwrap_or(prev_len..collector.full.len());
        let original = original.unwrap_or(&collector.full[payload.clone()]);
        collector.spans.push_source(
            payload,
            original,
            &collector.full,
            child.span(),
            engine.world,
        );
    }

    Ok((collector.full, collector.segments, collector.spans))
}

/// Collects segments.
struct Collector<'a> {
    full: String,
    segments: Vec<Segment<'a>>,
    spans: SpanMapper,
}

impl<'a> Collector<'a> {
    fn new(capacity: usize) -> Self {
        Self {
            full: String::new(),
            segments: Vec::with_capacity(capacity),
            spans: SpanMapper::new(),
        }
    }

    fn push_text(&mut self, text: &str, styles: StyleChain<'a>) {
        self.build_text(styles, |full| full.push_str(text));
    }

    fn build_text<F>(&mut self, styles: StyleChain<'a>, f: F)
    where
        F: FnOnce(&mut String),
    {
        let prev = self.full.len();
        f(&mut self.full);
        let segment_len = self.full.len() - prev;

        // Merge adjacent text segments with the same styles.
        if let Some(Segment::Text(last_len, last_styles)) = self.segments.last_mut()
            && *last_styles == styles
        {
            *last_len += segment_len;
            return;
        }

        self.segments.push(Segment::Text(segment_len, styles));
    }

    fn push_item(&mut self, item: Item<'a>) {
        match (self.segments.last_mut(), &item) {
            // Merge adjacent weak spacing by taking the maximum.
            (
                Some(Segment::Item(Item::Absolute(prev_amount, true))),
                Item::Absolute(amount, true),
            ) => {
                *prev_amount = (*prev_amount).max(*amount);
            }

            _ => {
                self.full.push_str(item.textual());
                self.segments.push(Segment::Item(item));
            }
        }
    }
}

/// Maps byte offsets back to spans.
// avenger: also maps byte ranges back to source ranges, through `sources`.
#[derive(Default)]
pub struct SpanMapper(Vec<(usize, Span)>, Vec<SourceEntry>);

/// Where a child's text lies in the full text and where it came from in the source.
// avenger: the source side table.
struct SourceEntry {
    /// The byte range of the child's own text in the full text.
    payload: Range,
    /// The byte range of the child's node in the source.
    source: Range,
    /// Whether the child's text maps byte for byte to its source text: it was made from the
    /// verbatim source text, unchanged or by a case change that kept every character's
    /// length.
    verbatim: bool,
}

impl SpanMapper {
    /// Create a new span mapper.
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a span for a segment with the given length.
    pub fn push(&mut self, len: usize, span: Span) {
        self.0.push((len, span));
    }

    /// Record where the text in `payload` came from, if its span has a source
    /// range. `original` is the text the payload was made from.
    // avenger: fills the source side table.
    pub fn push_source(
        &mut self,
        payload: Range,
        original: &str,
        full: &str,
        span: Span,
        world: &dyn World,
    ) {
        let SpanKind::Range { id, range } = span.get() else { return };
        let text = &full[payload.clone()];
        let verbatim = world.source(id).and_then(|source| source.get(range.clone()))
            == Some(original)
            && text
                .chars()
                .map(char::len_utf8)
                .eq(original.chars().map(char::len_utf8));
        self.1.push(SourceEntry { payload, source: range, verbatim });
    }

    /// Determine the span at the given byte offset.
    ///
    /// May return a detached span.
    pub fn span_at(&self, offset: usize) -> (Span, u16) {
        let mut cursor = 0;
        for &(len, span) in &self.0 {
            if (cursor..cursor + len).contains(&offset) {
                return (span, u16::try_from(offset - cursor).unwrap_or(0));
            }
            cursor += len;
        }
        (Span::detached(), 0)
    }

    /// Determine the source range of the text in the given byte range: the union
    /// of the exact ranges of verbatim text and the node ranges of the rest.
    ///
    /// Is empty if no source text produced the range.
    // avenger: for exact glyph source ranges.
    pub fn source_range(&self, range: Range) -> Range {
        let mut union: Option<Range> = None;
        for entry in &self.1 {
            let start = range.start.max(entry.payload.start);
            let end = range.end.min(entry.payload.end);
            if start >= end {
                continue;
            }
            let source = if entry.verbatim {
                entry.source.start + (start - entry.payload.start)
                    ..entry.source.start + (end - entry.payload.start)
            } else {
                entry.source.clone()
            };
            union = Some(match union {
                Some(union) => union.start.min(source.start)..union.end.max(source.end),
                None => source,
            });
        }
        union.unwrap_or(0..0)
    }
}
