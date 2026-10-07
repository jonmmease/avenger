//! Ported from crates/typst-layout/src/inline/linebreak.rs @ v0.15.1, modified for Avenger.
//!
//! avenger: a label's lines end at mandatory breakpoints. There is no optimized line breaking,
//! hyphenation or costs.

use super::line::{Line, line};
use super::prepare::Preparation;
use crate::typst_library::engine::Engine;
use crate::typst_library::layout::Abs;
use crate::typst_library::text::is_default_ignorable;
use typst_utils::Numeric;

/// Breaks the text into lines.
// avenger: in place of `linebreak`. Every line fits an infinite width, so lines end only at
// mandatory breakpoints.
pub fn linebreak<'a>(
    engine: &Engine,
    p: &'a Preparation<'a>,
    width: Abs,
) -> Vec<Line<'a>> {
    debug_assert!(!width.is_finite());
    let mut lines = Vec::with_capacity(4);
    let mut start = 0;
    mandatory_breakpoints(p.text, |end| {
        let attempt = line(engine, p, start..end, Breakpoint::Mandatory, lines.last());
        lines.push(attempt);
        start = end;
    });
    lines
}

/// Calls `f` with the end of each line that a mandatory breakpoint ends: after a character of
/// class BK, CR, LF or NL, except a CR before an LF, and at the end of the text, which is the
/// only breakpoint of empty text (UAX #14 rules LB3 to LB6).
// avenger: the mandatory breakpoints of `breakpoints`.
fn mandatory_breakpoints(text: &str, mut f: impl FnMut(usize)) {
    let mut chars = text.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let end = i + c.len_utf8();
        let crlf = c == '\r' && chars.peek().is_some_and(|&(_, next)| next == '\n');
        if is_mandatory_break(c) && !crlf && end < text.len() {
            f(end);
        }
    }
    f(text.len());
}

/// Whether a character has one of the Unicode line break classes that force a
/// break: BK, CR, LF or NL.
// avenger: the classes' code points, in place of `icu_properties`'s line break data.
pub fn is_mandatory_break(c: char) -> bool {
    matches!(
        c,
        '\u{000A}'
            | '\u{000B}'
            | '\u{000C}'
            | '\u{000D}'
            | '\u{0085}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// A line break opportunity.
// avenger: a label's lines end at mandatory breakpoints. The other kinds stay, so that line
// layout reads as upstream's.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
#[expect(dead_code, reason = "a label's lines end at mandatory breakpoints")]
pub enum Breakpoint {
    /// Just a normal opportunity (e.g. after a space).
    Normal,
    /// A mandatory breakpoint (after '\n' or at the end of the text).
    Mandatory,
    /// An opportunity for hyphenating and how many chars are before/after it
    /// in the word.
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
                // avenger: the classes BK, CR, LF and NL, as a table.
                let trimmed = line.trim_end_matches(is_mandatory_break);
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
