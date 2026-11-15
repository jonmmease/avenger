//! The elements of GoogleSQL's format strings, and the escaping of the elements that a type of
//! value lacks.

/// A type of value that a format formats or parses: GoogleSQL's `DATE`, `TIME`, `DATETIME`, or
/// `TIMESTAMP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Date,
    Time,
    Datetime,
    Timestamp,
}

impl Kind {
    /// The elements that a value of this kind lacks, which GoogleSQL's formatting prints as
    /// written: the time of a `DATE`, the date of a `TIME`, and the time zone of both and of a
    /// `DATETIME`.
    pub(crate) fn absent(self) -> &'static [u8] {
        match self {
            Kind::Date => b"cHIklMPpRrSsTXZz",
            Kind::Time => b"AaBbhCcDdeFGgjmQsUuVWwxYyZz",
            Kind::Datetime => b"Zz",
            Kind::Timestamp => b"",
        }
    }
}

/// `format` with each element that `absent` names escaped, so it prints as written, as
/// GoogleSQL's formatting escapes them. An element with a width, such as `%4H`, escapes by its
/// letter, and `%E#S`, `%E*S` and `%E4Y` escape with `%S` and `%Y`, where `#` is one digit.
pub(crate) fn escape_absent(format: &str, absent: &[u8]) -> String {
    let b = format.as_bytes();
    let end = b.len();
    let mut out: Vec<u8> = Vec::with_capacity(end + 4);
    let (mut cur, mut pending) = (0, 0);
    while cur != end {
        while cur != end && b[cur] != b'%' {
            cur += 1;
        }
        let percent = cur;
        while cur != end && b[cur] == b'%' {
            cur += 1;
        }
        if cur != pending {
            out.extend_from_slice(&b[pending..cur]);
            pending = cur;
        }
        // An even run of `%` is text.
        if cur == end || (cur - percent) % 2 == 0 {
            continue;
        }
        while cur != end && b[cur].is_ascii_digit() {
            cur += 1;
        }
        if cur != end && absent.contains(&b[cur]) {
            out.push(b'%');
            cur += 1;
            out.extend_from_slice(&b[pending..cur]);
            pending = cur;
            continue;
        }
        if cur != end {
            if b[cur] != b'E' && b[cur] != b'O' {
                out.push(b[pending]);
                pending += 1;
                continue;
            }
            cur += 1;
            if cur == end {
                out.push(b[pending]);
                pending += 1;
                continue;
            }
            let escape = match b[pending] {
                b'E' => {
                    absent.contains(&b[cur])
                        || (absent.contains(&b'S')
                            && (b[cur] == b'*' || b[cur].is_ascii_digit())
                            && {
                                cur += 1;
                                cur != end && b[cur] == b'S'
                            })
                        || (absent.contains(&b'Y') && b[cur] == b'4' && {
                            cur += 1;
                            cur != end && b[cur] == b'Y'
                        })
                }
                b'O' => absent.contains(&b[cur]),
                _ => false,
            };
            if escape {
                cur += 1;
                out.push(b'%');
            }
        }
        if cur != pending {
            out.extend_from_slice(&b[pending..cur]);
            pending = cur;
        }
    }
    String::from_utf8(out).expect("escaping adds only `%`")
}

/// How an element pads a number: GoogleSQL's flags `-` (no padding), `_` (spaces), and `0`
/// (zeros), or the element's own padding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Pad {
    Default,
    None,
    Space,
    Zero,
}

/// A part of a format string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Token {
    /// Text, which prints as written.
    Text(String),
    /// An element: its letter, its padding, and whether it has the `E` or `O` modifier.
    Element {
        letter: u8,
        pad: Pad,
        modifier: bool,
    },
    /// `%E#S`: the seconds with `#` digits of their fraction.
    Seconds(usize),
    /// `%E*S`: the seconds with the digits of their fraction, without trailing zeros.
    SecondsTrimmed,
    /// `%Ez`: the offset as `+hh:mm`.
    OffsetColon,
    /// `%E*z`: the offset as `+hh:mm:ss`.
    OffsetSeconds,
}

/// The letters of the elements that GoogleSQL documents, which print a part of the value.
pub(crate) const LETTERS: &[u8] = b"AaBbCcDdeFGgHhIJjklMmnPpQRrSsTtUuVWwXxYyZz";

/// A part of a format string as written: text, or a `%` element with its byte offset, its text,
/// and its token, which is `None` for an element that prints as written.
pub(crate) enum Scanned<'a> {
    Text(&'a str),
    Element {
        start: usize,
        written: &'a str,
        token: Option<Token>,
    },
}

/// The parts of `format`. `%%` and a `%` at the end are the text `%`.
pub(crate) fn scan(format: &str) -> Vec<Scanned<'_>> {
    let mut parts = Vec::new();
    let mut at = 0;
    while let Some(offset) = format[at..].find('%') {
        let start = at + offset;
        if offset > 0 {
            parts.push(Scanned::Text(&format[at..start]));
        }
        let rest = &format[start + 1..];
        if rest.is_empty() || rest.starts_with('%') {
            parts.push(Scanned::Text("%"));
            at = (start + 2).min(format.len());
            continue;
        }
        let (token, length) = element(rest);
        let written = &format[start..start + 1 + length];
        parts.push(Scanned::Element {
            start,
            written,
            token,
        });
        at = start + 1 + length;
    }
    if at < format.len() {
        parts.push(Scanned::Text(&format[at..]));
    }
    parts
}

/// The tokens of `format`, after escaping. An element outside GoogleSQL's documented list, or with
/// a width, such as `%10d`, is text as written.
pub(crate) fn tokens(format: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut text = String::new();
    for part in scan(format) {
        match part {
            Scanned::Text(written)
            | Scanned::Element {
                written,
                token: None,
                ..
            } => text.push_str(written),
            Scanned::Element {
                token: Some(token), ..
            } => {
                if !text.is_empty() {
                    tokens.push(Token::Text(std::mem::take(&mut text)));
                }
                tokens.push(token);
            }
        }
    }
    if !text.is_empty() {
        tokens.push(Token::Text(text));
    }
    tokens
}

/// The element at the start of `rest`, the text after a `%`, and its length in bytes: `Some`
/// token, or `None` for an element that prints as written.
fn element(rest: &str) -> (Option<Token>, usize) {
    let b = rest.as_bytes();
    let pad = match b[0] {
        b'-' => Pad::None,
        b'_' => Pad::Space,
        b'0' => Pad::Zero,
        _ => Pad::Default,
    };
    let at = usize::from(pad != Pad::Default);
    // The length of the character at `i`, or 0 at the end.
    let char_at = |i: usize| {
        rest.get(i..)
            .and_then(|r| r.chars().next())
            .map_or(0, char::len_utf8)
    };
    let digits = b[at..].iter().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 {
        // A width, which GoogleSQL does not document: the element prints as written.
        return (None, at + digits + char_at(at + digits));
    }
    let letter = |letter: u8, modifier: bool| {
        Some(Token::Element {
            letter,
            pad,
            modifier,
        })
    };
    match b.get(at) {
        None => (None, at),
        Some(b'E') => {
            let ext = &b[at + 1..];
            let n = ext.iter().take_while(|c| c.is_ascii_digit()).count();
            let (token, length) = match ext {
                [b'z', ..] => (Some(Token::OffsetColon), 1),
                [b'*', b'z', ..] => (Some(Token::OffsetSeconds), 2),
                [b'*', b'S', ..] => (Some(Token::SecondsTrimmed), 2),
                // `%E4Y` writes the year as `%EY` does, in four digits.
                [b'4', b'Y', ..] => (letter(b'Y', true).filter(|_| pad == Pad::Default), 2),
                _ if n > 0 && ext.get(n) == Some(&b'S') => {
                    let digits = rest[at + 1..at + 1 + n].parse().unwrap_or(usize::MAX);
                    ((digits <= 12).then_some(Token::Seconds(digits)), n + 1)
                }
                [l, ..] if b"CcXxYy".contains(l) => (letter(*l, true), 1),
                _ => (None, char_at(at + 1)),
            };
            // Flags apply to the alternative forms of letters only.
            let token = token.filter(|t| pad == Pad::Default || matches!(t, Token::Element { .. }));
            (token, at + 1 + length)
        }
        Some(b'O') => match b.get(at + 1) {
            Some(l) if b"deHIMmSUuVWwy".contains(l) => (letter(*l, true), at + 2),
            _ => (None, at + 1 + char_at(at + 1)),
        },
        // GoogleSQL expands only the exact `%J`, `%Q`, and `%Z`.
        Some(c) if pad != Pad::Default && b"JQZ".contains(c) => (None, at + 1),
        Some(c) if LETTERS.contains(c) => (letter(*c, false), at + 1),
        Some(_) => (None, at + char_at(at)),
    }
}

/// Why the strict check of a format rejects it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Rejection {
    /// An element that GoogleSQL does not document, or with a width, at a byte offset.
    Unknown { element: String, position: usize },
    /// An element of a part that the kind of value lacks, such as the hour of a date.
    Absent { element: String },
    /// `%J`, which GoogleSQL documents but does not format.
    Unsupported,
}

/// Checks that each element of `format` is one that GoogleSQL documents and that a value of `kind`
/// has, as a chart's formats need, where GoogleSQL's functions print the others as written.
pub(crate) fn check(format: &str, kind: Kind) -> Result<(), Rejection> {
    for part in scan(format) {
        let Scanned::Element {
            start,
            written,
            token,
        } = part
        else {
            continue;
        };
        let letter = match token {
            None => {
                return Err(Rejection::Unknown {
                    element: written.to_string(),
                    position: start,
                })
            }
            Some(Token::Element { letter: b'J', .. }) => return Err(Rejection::Unsupported),
            Some(Token::Element { letter, .. }) => letter,
            Some(Token::Seconds(_) | Token::SecondsTrimmed) => b'S',
            Some(Token::OffsetColon | Token::OffsetSeconds) => b'z',
            Some(Token::Text(_)) => unreachable!("an element is not text"),
        };
        if kind.absent().contains(&letter) {
            return Err(Rejection::Absent {
                element: written.to_string(),
            });
        }
    }
    Ok(())
}
