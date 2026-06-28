use crate::error::LabelError;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum UnmatchedDelimiterPolicy {
    TreatAsLiteral,
    Error,
}

impl Default for UnmatchedDelimiterPolicy {
    fn default() -> Self {
        Self::TreatAsLiteral
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathDelimiterOptions {
    pub delimiter: char,
    pub escape: Option<char>,
    pub unmatched: UnmatchedDelimiterPolicy,
    pub allow_display_style: bool,
}

impl Default for MathDelimiterOptions {
    fn default() -> Self {
        Self {
            delimiter: '$',
            escape: Some('\\'),
            unmatched: UnmatchedDelimiterPolicy::TreatAsLiteral,
            allow_display_style: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum MathDisplayHint {
    Inline,
    Display,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MathDelimiterInfo {
    pub full_range: std::ops::Range<usize>,
    pub opening_range: std::ops::Range<usize>,
    pub closing_range: std::ops::Range<usize>,
    pub display_hint: MathDisplayHint,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedSegment {
    Plain {
        text: String,
        range: std::ops::Range<usize>,
    },
    Math {
        source: String,
        source_range: std::ops::Range<usize>,
        delimiter: MathDelimiterInfo,
    },
}

pub(crate) fn parse_segments(
    source: &str,
    options: &MathDelimiterOptions,
) -> Result<Vec<ParsedSegment>, LabelError> {
    let mut segments = Vec::new();
    let mut plain = String::new();
    let mut plain_start = 0usize;
    let mut pos = 0usize;

    while let Some((idx, ch)) = next_char(source, pos) {
        if Some(ch) == options.escape {
            let next_pos = idx + ch.len_utf8();
            if let Some((_, next)) = next_char(source, next_pos) {
                if next == options.delimiter || Some(next) == options.escape {
                    plain.push(next);
                    pos = next_pos + next.len_utf8();
                    continue;
                }
            }
        }

        if ch == options.delimiter {
            let open_end = idx + ch.len_utf8();
            match read_math(source, open_end, options) {
                Some((math, close_start, close_end)) => {
                    push_plain(&mut segments, &mut plain, plain_start, idx);
                    let display_hint = display_hint(&math);
                    segments.push(ParsedSegment::Math {
                        source: math,
                        source_range: open_end..close_start,
                        delimiter: MathDelimiterInfo {
                            full_range: idx..close_end,
                            opening_range: idx..open_end,
                            closing_range: close_start..close_end,
                            display_hint,
                        },
                    });
                    plain_start = close_end;
                    pos = close_end;
                    continue;
                }
                None => match options.unmatched {
                    UnmatchedDelimiterPolicy::TreatAsLiteral => {
                        plain.push(ch);
                    }
                    UnmatchedDelimiterPolicy::Error => {
                        return Err(LabelError::UnmatchedDelimiter { position: idx });
                    }
                },
            }
        } else {
            plain.push(ch);
        }

        pos = idx + ch.len_utf8();
    }

    push_plain(&mut segments, &mut plain, plain_start, source.len());
    Ok(segments)
}

fn read_math(
    source: &str,
    start: usize,
    options: &MathDelimiterOptions,
) -> Option<(String, usize, usize)> {
    let mut math = String::new();
    let mut iter = source[start..]
        .char_indices()
        .map(|(offset, c)| (start + offset, c))
        .peekable();

    while let Some((idx, ch)) = iter.next() {
        if Some(ch) == options.escape {
            if let Some(&(_, next)) = iter.peek() {
                if next == options.delimiter || Some(next) == options.escape {
                    math.push(next);
                    iter.next();
                    continue;
                }
            }
        }

        if ch == options.delimiter {
            return Some((math, idx, idx + ch.len_utf8()));
        }

        math.push(ch);
    }

    None
}

fn push_plain(segments: &mut Vec<ParsedSegment>, plain: &mut String, start: usize, end: usize) {
    if !plain.is_empty() {
        segments.push(ParsedSegment::Plain {
            text: std::mem::take(plain),
            range: start..end,
        });
    }
}

fn next_char(source: &str, start: usize) -> Option<(usize, char)> {
    source[start..]
        .char_indices()
        .next()
        .map(|(offset, ch)| (start + offset, ch))
}

fn display_hint(source: &str) -> MathDisplayHint {
    let starts_with_space = source.chars().next().is_some_and(char::is_whitespace);
    let ends_with_space = source.chars().next_back().is_some_and(char::is_whitespace);
    if starts_with_space && ends_with_space {
        MathDisplayHint::Display
    } else {
        MathDisplayHint::Inline
    }
}
