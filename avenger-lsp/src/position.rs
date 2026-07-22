use std::{ops::Range as ByteRange, sync::Arc};

use tower_lsp_server::ls_types::{Position, PositionEncodingKind, Range};

#[derive(Clone, Debug)]
pub(crate) struct PositionIndex {
    text: Arc<str>,
    lines: Vec<Line>,
    encoding: PositionEncoding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionEncoding {
    Utf8,
    Utf16,
}

#[derive(Clone, Copy, Debug)]
struct Line {
    start: usize,
    content_end: usize,
    end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PositionError {
    #[error("line {line} is outside the document")]
    InvalidLine { line: u32 },
    #[error("character {character} is outside line {line}")]
    InvalidCharacter { line: u32, character: u32 },
    #[error("position {line}:{character} splits a UTF-8 code point or UTF-16 surrogate pair")]
    SplitCodePoint { line: u32, character: u32 },
    #[error("byte offset {offset} is outside the document or not a character boundary")]
    InvalidOffset { offset: usize },
    #[error("range ends before it starts")]
    ReversedRange,
}

impl PositionEncoding {
    pub(crate) fn from_lsp(kind: &PositionEncodingKind) -> Self {
        if kind == &PositionEncodingKind::UTF8 {
            Self::Utf8
        } else {
            Self::Utf16
        }
    }
}

impl PositionIndex {
    pub(crate) fn new(text: Arc<str>, encoding: PositionEncoding) -> Self {
        let bytes = text.as_bytes();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut cursor = 0;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\n' => {
                    lines.push(Line {
                        start,
                        content_end: cursor,
                        end: cursor + 1,
                    });
                    cursor += 1;
                    start = cursor;
                }
                b'\r' => {
                    let end = if bytes.get(cursor + 1) == Some(&b'\n') {
                        cursor + 2
                    } else {
                        cursor + 1
                    };
                    lines.push(Line {
                        start,
                        content_end: cursor,
                        end,
                    });
                    cursor = end;
                    start = cursor;
                }
                _ => cursor += 1,
            }
        }
        lines.push(Line {
            start,
            content_end: bytes.len(),
            end: bytes.len(),
        });
        Self {
            text,
            lines,
            encoding,
        }
    }

    pub(crate) fn offset(&self, position: Position) -> Result<usize, PositionError> {
        let line = self
            .lines
            .get(position.line as usize)
            .ok_or(PositionError::InvalidLine {
                line: position.line,
            })?;
        let content = &self.text[line.start..line.content_end];
        let relative = match self.encoding {
            PositionEncoding::Utf8 => {
                let relative = position.character as usize;
                if relative > content.len() {
                    return Err(PositionError::InvalidCharacter {
                        line: position.line,
                        character: position.character,
                    });
                }
                if !content.is_char_boundary(relative) {
                    return Err(PositionError::SplitCodePoint {
                        line: position.line,
                        character: position.character,
                    });
                }
                relative
            }
            PositionEncoding::Utf16 => utf16_column_to_byte(content, position)?,
        };
        Ok(line.start + relative)
    }

    pub(crate) fn position(&self, offset: usize) -> Result<Position, PositionError> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return Err(PositionError::InvalidOffset { offset });
        }
        let line_number = self
            .lines
            .partition_point(|line| line.end <= offset && line.end < self.text.len());
        let line = self
            .lines
            .get(line_number)
            .ok_or(PositionError::InvalidOffset { offset })?;
        if offset > line.content_end {
            return Err(PositionError::InvalidOffset { offset });
        }
        let prefix = &self.text[line.start..offset];
        let character = match self.encoding {
            PositionEncoding::Utf8 => prefix.len(),
            PositionEncoding::Utf16 => prefix.encode_utf16().count(),
        };
        Ok(Position::new(line_number as u32, character as u32))
    }

    pub(crate) fn byte_range(&self, range: Range) -> Result<ByteRange<usize>, PositionError> {
        let start = self.offset(range.start)?;
        let end = self.offset(range.end)?;
        if end < start {
            return Err(PositionError::ReversedRange);
        }
        Ok(start..end)
    }

    pub(crate) fn lsp_range(&self, range: ByteRange<usize>) -> Result<Range, PositionError> {
        if range.end < range.start {
            return Err(PositionError::ReversedRange);
        }
        Ok(Range::new(
            self.position(range.start)?,
            self.position(range.end)?,
        ))
    }

    #[cfg(test)]
    fn line_count(&self) -> usize {
        self.lines.len()
    }
}

fn utf16_column_to_byte(content: &str, position: Position) -> Result<usize, PositionError> {
    let target = position.character as usize;
    let mut units = 0;
    for (offset, character) in content.char_indices() {
        if units == target {
            return Ok(offset);
        }
        let width = character.len_utf16();
        if units + width > target {
            return Err(PositionError::SplitCodePoint {
                line: position.line,
                character: position.character,
            });
        }
        units += width;
    }
    if units == target {
        Ok(content.len())
    } else {
        Err(PositionError::InvalidCharacter {
            line: position.line,
            character: position.character,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_round_trips_multibyte_text() {
        let text: Arc<str> = "a😀e\u{301}\t中\nnext".into();
        let index = PositionIndex::new(text.clone(), PositionEncoding::Utf16);
        for offset in text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
        {
            let position = index.position(offset).unwrap();
            assert_eq!(index.offset(position).unwrap(), offset);
        }
        assert_eq!(index.offset(Position::new(0, 3)).unwrap(), 5);
        assert_eq!(
            index.offset(Position::new(0, 2)),
            Err(PositionError::SplitCodePoint {
                line: 0,
                character: 2
            })
        );
    }

    #[test]
    fn utf8_uses_bytes_and_rejects_codepoint_interiors() {
        let index = PositionIndex::new("éx".into(), PositionEncoding::Utf8);
        assert_eq!(index.offset(Position::new(0, 2)).unwrap(), 2);
        assert!(matches!(
            index.offset(Position::new(0, 1)),
            Err(PositionError::SplitCodePoint { .. })
        ));
    }

    #[test]
    fn handles_lf_crlf_bare_cr_and_trailing_empty_lines() {
        let index = PositionIndex::new("a\r\nb\rc\n".into(), PositionEncoding::Utf16);
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.offset(Position::new(0, 1)).unwrap(), 1);
        assert_eq!(index.offset(Position::new(1, 0)).unwrap(), 3);
        assert_eq!(index.offset(Position::new(2, 0)).unwrap(), 5);
        assert_eq!(index.offset(Position::new(3, 0)).unwrap(), 7);
        assert!(index.position(2).is_err());
    }

    #[test]
    fn rejects_invalid_and_reversed_ranges() {
        let index = PositionIndex::new("abc\ndef".into(), PositionEncoding::Utf16);
        assert!(index.offset(Position::new(9, 0)).is_err());
        assert!(index.offset(Position::new(0, 4)).is_err());
        assert_eq!(
            index.byte_range(Range::new(Position::new(1, 1), Position::new(0, 1))),
            Err(PositionError::ReversedRange)
        );
    }
}
