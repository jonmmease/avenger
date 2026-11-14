//! The text of quoted tokens: string and bytes literals, and names in backticks.

/// The parts of a closed quoted token.
pub(crate) struct Quoted<'a> {
    /// The text between the quotes.
    pub body: &'a str,
    /// The offset of the body in the token.
    pub body_offset: usize,
    /// Whether an `r` prefix makes the token raw, so it keeps its backslashes.
    pub raw: bool,
    /// Whether a `b` prefix makes the token a bytes literal.
    pub bytes: bool,
}

/// The parts of the quoted token `text`, or `None` when its closing quote is missing, which the
/// lexer reports. As in the lexer, a backslash escapes the next byte, in a raw token too.
pub(crate) fn split(text: &str) -> Option<Quoted<'_>> {
    let bytes = text.as_bytes();
    let prefix = bytes
        .iter()
        .position(|b| matches!(b, b'\'' | b'"' | b'`'))?;
    let quote = bytes[prefix];
    let triple = quote != b'`' && bytes[prefix..].starts_with(&[quote, quote, quote]);
    let quote_len = if triple { 3 } else { 1 };
    let body_offset = prefix + quote_len;
    let mut index = body_offset;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == quote && (!triple || bytes[index..].starts_with(&[quote, quote, quote])) {
            if index + quote_len != bytes.len() {
                return None;
            }
            let prefix = &text[..prefix];
            return Some(Quoted {
                body: &text[body_offset..index],
                body_offset,
                raw: prefix.bytes().any(|b| b.eq_ignore_ascii_case(&b'r')),
                bytes: prefix.bytes().any(|b| b.eq_ignore_ascii_case(&b'b')),
            });
        }
        index += match byte == b'\\' && index + 1 < bytes.len() {
            true => 2,
            false => 1,
        };
    }
    None
}

/// The value of the quoted token `text`, after its escapes, or `None` when it is unclosed or
/// its escapes are wrong.
pub(crate) fn value(text: &str) -> Option<Vec<u8>> {
    let quoted = split(text)?;
    unescape(quoted.body, quoted.raw, quoted.bytes).ok()
}

/// An error in the escapes of a quoted token: its offset in the token's body, its length, and
/// its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EscapeError {
    pub offset: usize,
    pub len: usize,
    pub message: String,
}

/// The value of the body of a quoted token, the text between its quotes, after its escapes. A
/// raw body keeps its backslashes, and a bytes body takes no `\u` or `\U`.
pub(crate) fn unescape(body: &str, raw: bool, bytes: bool) -> Result<Vec<u8>, EscapeError> {
    let source = body.as_bytes();
    let mut value = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        let byte = source[index];
        if byte != b'\\' || index + 1 >= source.len() {
            value.push(byte);
            index += 1;
            continue;
        }
        let next = source[index + 1];
        if raw {
            value.extend([b'\\', next]);
            index += 2;
            continue;
        }
        let start = index;
        index += 2;
        let digits = |index: &mut usize, count: usize, radix: u32| -> Result<u32, EscapeError> {
            let mut code = 0u32;
            for _ in 0..count {
                let digit = source
                    .get(*index)
                    .and_then(|b| (*b as char).to_digit(radix))
                    .ok_or_else(|| EscapeError {
                        offset: start,
                        len: *index - start,
                        message: format!("this escape takes {count} digits"),
                    })?;
                code = code * radix + digit;
                *index += 1;
            }
            Ok(code)
        };
        match next {
            b'a' => value.push(0x07),
            b'b' => value.push(0x08),
            b'f' => value.push(0x0c),
            b'n' => value.push(b'\n'),
            b'r' => value.push(b'\r'),
            b't' => value.push(b'\t'),
            b'v' => value.push(0x0b),
            b'\\' | b'?' | b'"' | b'\'' | b'`' => value.push(next),
            b'0'..=b'7' => {
                let rest = digits(&mut index, 2, 8)?;
                let code = u32::from(next - b'0') * 64 + rest;
                let code = u8::try_from(code).map_err(|_| EscapeError {
                    offset: start,
                    len: index - start,
                    message: "an octal escape is at most \\377".to_string(),
                })?;
                value.push(code);
            }
            b'x' | b'X' => value.push(digits(&mut index, 2, 16)? as u8),
            b'u' | b'U' if !bytes => {
                let count = if next == b'u' { 4 } else { 8 };
                let code = digits(&mut index, count, 16)?;
                let character = char::from_u32(code).ok_or_else(|| EscapeError {
                    offset: start,
                    len: index - start,
                    message: "this escape names no Unicode character".to_string(),
                })?;
                let mut buffer = [0; 4];
                value.extend(character.encode_utf8(&mut buffer).as_bytes());
            }
            _ => {
                return Err(EscapeError {
                    offset: start,
                    len: 2,
                    message: format!("`\\{}` is not an escape", next as char),
                })
            }
        }
    }
    Ok(value)
}
