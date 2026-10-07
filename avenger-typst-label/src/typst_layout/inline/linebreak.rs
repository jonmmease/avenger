//! Ported from crates/typst-layout/src/inline/linebreak.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's paragraph is never justified, so its lines break greedily, without
//! optimized line breaking, hyphenation or costs. Break opportunities come from
//! `unicode-linebreak` in place of ICU4X's segmenters (D27).

use typst_syntax::link_prefix;
use unicode_linebreak::{BreakClass, break_property, linebreaks};

use super::line::{Line, line};
use super::prepare::Preparation;
use crate::typst_library::engine::Engine;
use crate::typst_library::layout::Abs;
use crate::typst_library::text::is_default_ignorable;
use typst_utils::Numeric;

// avenger: no segmenters or line break data, which `unicode-linebreak` provides (D27).

/// A line break opportunity.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Breakpoint {
    /// Just a normal opportunity (e.g. after a space).
    Normal,
    /// A mandatory breakpoint (after '\n' or at the end of the text).
    Mandatory,
    /// An opportunity for hyphenating and how many chars are before/after it
    /// in the word.
    // avenger: labels don't hyphenate. The variant stays, so that line layout reads as
    // upstream's.
    #[expect(dead_code, reason = "labels don't hyphenate")]
    Hyphen(u8, u8),
}

impl Breakpoint {
    /// Trim a line before this breakpoint.
    pub fn trim(self, start: usize, line: &str) -> Trim {
        match self {
            // Trailing whitespace should be shaped, but the glyphs should have
            // their advance width zeroed. This way, they are available for copy
            // paste, but don't influence layout.
            // Also trim Unicode `Default_Ignorable`s, since they may interfere
            // with end-of-line adjustments in CJK layout and since they are not
            // rendered, they will be included in another glyph cluster. If they
            // aren't trimmed here and they attach to a space glyph that is
            // covered by a font, the space may be considered relevant for
            // inline layout, which is undesirable. Unicode `Default_Ignorable`s
            // include among others:
            // - `\u{200B}` zero width space
            // - `\u{202A}` LTR embedding
            // - `\u{202B}` RTL embedding
            // - `\u{202C}` POP embedding
            // - `\u{2066}` LTR isolate
            // - `\u{2069}` POP isolate
            Self::Normal => {
                let trimmed = line.trim_end_matches(|c: char| {
                    c.is_whitespace() || is_default_ignorable(c)
                });
                Trim {
                    layout: start + trimmed.len(),
                    shaping: start + line.len(),
                }
            }

            // Trim linebreaks.
            Self::Mandatory => {
                let trimmed = line.trim_end_matches(|c: char| {
                    matches!(
                        break_property(c as u32),
                        BreakClass::Mandatory
                            | BreakClass::CarriageReturn
                            | BreakClass::LineFeed
                            | BreakClass::NextLine
                    )
                });
                Trim::uniform(start + trimmed.len())
            }

            // Trim nothing.
            Self::Hyphen(..) => Trim::uniform(start + line.len()),
        }
    }

    /// Whether this is a hyphen breakpoint.
    pub fn is_hyphen(self) -> bool {
        matches!(self, Self::Hyphen(..))
    }
}

/// How to trim the end of a line.
///
/// It's an invariant that `self.layout <= self.shaping`.
pub struct Trim {
    /// The text in the range `layout..shaping` should be shaped but should not
    /// affect layout. This ensures that we trim spaces for layout purposes, but
    /// still render zero-advance space glyphs for copy paste.
    pub layout: usize,
    /// The text should only be shaped up until the given text offset. Newlines
    /// are already trimmed here.
    pub shaping: usize,
}

impl Trim {
    /// Create an instance with equal layout and shaping trim.
    fn uniform(trim: usize) -> Self {
        Self { layout: trim, shaping: trim }
    }
}

/// Breaks the text into lines.
pub fn linebreak<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    width: Abs,
) -> Vec<Line<'a>> {
    // avenger: always simple, since a label's paragraph is never justified, and upstream
    // optimizes only justified paragraphs by default.
    linebreak_simple(engine, p, width)
}

/// Performs line breaking in simple first-fit style. This means that we build
/// lines greedily, always taking the longest possible line. This may lead to
/// very unbalanced line, but is fast and simple.
fn linebreak_simple<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    width: Abs,
) -> Vec<Line<'a>> {
    let mut lines = Vec::with_capacity(16);
    let mut start = 0;
    let mut last = None;

    breakpoints(p, |end, breakpoint| {
        // avenger: every line fits an infinite width, so only mandatory breakpoints end
        // lines there, and the attempts at the others, whose cost grows quadratically with
        // the line, are skipped.
        if !width.is_finite() && breakpoint != Breakpoint::Mandatory {
            return;
        }

        // Compute the line and its size.
        let mut attempt = line(engine, p, start..end, breakpoint, lines.last());

        // If the line doesn't fit anymore, we push the last fitting attempt
        // into the stack and rebuild the line from the attempt's end. The
        // resulting line cannot be broken up further.
        if !width.fits(attempt.width)
            && let Some((last_attempt, last_end)) = last.take()
        {
            lines.push(last_attempt);
            start = last_end;
            attempt = line(engine, p, start..end, breakpoint, lines.last());
        }

        // Finish the current line if there is a mandatory line break (i.e. due
        // to "\n") or if the line doesn't fit horizontally already since then
        // no shorter line will be possible.
        if breakpoint == Breakpoint::Mandatory || !width.fits(attempt.width) {
            lines.push(attempt);
            start = end;
            last = None;
        } else {
            last = Some((attempt, end));
        }
    });

    if let Some((line, _)) = last {
        lines.push(line);
    }

    lines
}

// avenger: no optimized line breaking, since a label's paragraph is never justified.

/// Yields for each breakpoint the text index, whether the break is mandatory
/// (after `\n`) and whether a hyphen is required (when breaking inside of a
/// word).
///
/// This is an internal instead of an external iterator because it makes the
/// code much simpler and the consumers of this function don't need the
/// composability and flexibility of external iteration anyway.
// avenger: the opportunities of `unicode-linebreak`, the same for every language, and no
// hyphenation (D27).
fn breakpoints(p: &Preparation, mut f: impl FnMut(usize, Breakpoint)) {
    let text = p.text;

    // Single breakpoint at the end for empty text.
    if text.is_empty() {
        f(0, Breakpoint::Mandatory);
        return;
    }

    let mut last = 0;
    let mut iter = linebreaks(text).map(|(point, _)| point).peekable();

    loop {
        // Special case for links. UAX #14 doesn't handle them well.
        let (head, tail) = text.split_at(last);
        if head.ends_with("://") || tail.starts_with("www.") {
            let (link, _) = link_prefix(tail);
            linebreak_link(link, |i| f(last + i, Breakpoint::Normal));
            last += link.len();
            while iter.peek().is_some_and(|&p| p < last) {
                iter.next();
            }
        }

        // Get the next UAX #14 linebreak opportunity.
        let Some(point) = iter.next() else { break };

        // Skip breakpoint if there is no char before it. icu4x generates one
        // at offset 0, but we don't want it.
        let Some(c) = text[..point].chars().next_back() else { continue };

        // Find out whether the last break was mandatory by checking against
        // rules LB4 and LB5, special-casing the end of text according to LB3.
        // See also: https://docs.rs/icu_segmenter/latest/icu_segmenter/struct.LineSegmenter.html
        let breakpoint = if point == text.len() {
            Breakpoint::Mandatory
        } else {
            const OBJ_REPLACE: char = '\u{FFFC}';
            match break_property(c as u32) {
                BreakClass::Mandatory
                | BreakClass::CarriageReturn
                | BreakClass::LineFeed
                | BreakClass::NextLine => Breakpoint::Mandatory,

                // https://github.com/typst/typst/issues/5489
                //
                // OBJECT-REPLACEMENT-CHARACTERs provide Contingent Break
                // opportunities before and after by default. This behaviour
                // is however tailorable, see:
                // https://www.unicode.org/reports/tr14/#CB
                // https://www.unicode.org/reports/tr14/#TailorableBreakingRules
                // https://www.unicode.org/reports/tr14/#LB20
                //
                // Don't provide a line breaking opportunity between a LTR-
                // ISOLATE (or any other Combining Mark) and an OBJECT-
                // REPLACEMENT-CHARACTER representing an inline item, if the
                // LTR-ISOLATE could end up as the only character on the
                // previous line.
                BreakClass::CombiningMark
                    if text[point..].starts_with(OBJ_REPLACE)
                        && last + c.len_utf8() == point =>
                {
                    continue;
                }

                _ => Breakpoint::Normal,
            }
        };

        // Call `f` for the UAX #14 break opportunity.
        f(point, breakpoint);
        last = point;
    }
}

/// Produce linebreak opportunities for a link.
fn linebreak_link(link: &str, mut f: impl FnMut(usize)) {
    #[derive(PartialEq)]
    enum Class {
        Alphabetic,
        Digit,
        Open,
        Other,
    }

    impl Class {
        fn of(c: char) -> Self {
            if c.is_alphabetic() {
                Class::Alphabetic
            } else if c.is_numeric() {
                Class::Digit
            } else if matches!(c, '(' | '[') {
                Class::Open
            } else {
                Class::Other
            }
        }
    }

    let mut offset = 0;
    let mut prev = Class::Other;

    for (end, c) in link.char_indices() {
        let class = Class::of(c);

        // Emit opportunities when going from
        // - other -> other
        // - alphabetic -> numeric
        // - numeric -> alphabetic
        // Never before/after opening delimiters.
        if end > 0
            && prev != Class::Open
            && if class == Class::Other { prev == Class::Other } else { class != prev }
        {
            let piece = &link[offset..end];
            if piece.len() < 16 {
                // For bearably long segments, emit them as one.
                offset = end;
                f(offset);
            } else {
                // If it gets very long (e.g. a hash in the URL), just allow a
                // break at every char.
                for c in piece.chars() {
                    offset += c.len_utf8();
                    f(offset);
                }
            }
        }

        prev = class;
    }
}
