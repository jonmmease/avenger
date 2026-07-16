use std::{collections::BTreeMap, fmt, ops::Range, path::PathBuf, sync::Arc};

use serde::{Deserialize, Serialize};

/// Opaque identity for one loaded source within a compilation attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceId(u32);

impl SourceId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "source:{}", self.0)
    }
}

/// Canonical origin used for source identity and deterministic diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SourceOrigin {
    Memory(String),
    File(PathBuf),
    Std(String),
    Http(String),
}

impl SourceOrigin {
    pub fn display_name(&self) -> String {
        match self {
            Self::Memory(name) => format!("memory:{name}"),
            Self::File(path) => path.to_string_lossy().into_owned(),
            Self::Std(path) => format!("std:{path}"),
            Self::Http(url) => url.clone(),
        }
    }
}

impl fmt::Display for SourceOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display_name())
    }
}

/// Half-open byte range into UTF-8 source text.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ByteSpan {
    pub start: usize,
    pub end: usize,
}

impl ByteSpan {
    pub fn new(start: usize, end: usize) -> Result<Self, SourceError> {
        if start > end {
            return Err(SourceError::ReversedSpan { start, end });
        }
        Ok(Self { start, end })
    }

    pub const fn empty(offset: usize) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    pub const fn len(self) -> usize {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    pub const fn as_range(self) -> Range<usize> {
        self.start..self.end
    }
}

/// A byte span paired with its owning source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SourceSpan {
    pub source: SourceId,
    pub range: ByteSpan,
}

impl SourceSpan {
    pub fn new(source: SourceId, start: usize, end: usize) -> Result<Self, SourceError> {
        Ok(Self {
            source,
            range: ByteSpan::new(start, end)?,
        })
    }

    pub const fn empty(source: SourceId, offset: usize) -> Self {
        Self {
            source,
            range: ByteSpan::empty(offset),
        }
    }
}

/// Zero-based position in source text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceLocation {
    /// Zero-based physical line.
    pub line: usize,
    /// Zero-based Unicode-scalar column.
    pub column: usize,
    /// Zero-based UTF-8 byte column.
    pub byte_column: usize,
    /// Zero-based rendered column with tabs advanced to the configured stops.
    pub display_column: usize,
}

/// Index from byte offsets to source positions.
#[derive(Clone, Debug)]
pub struct LineIndex {
    text: Arc<str>,
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: Arc<str>) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self { text, line_starts }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn location(&self, offset: usize) -> Result<SourceLocation, SourceError> {
        self.location_with_tab_width(offset, 4)
    }

    pub fn location_with_tab_width(
        &self,
        offset: usize,
        tab_width: usize,
    ) -> Result<SourceLocation, SourceError> {
        if offset > self.text.len() {
            return Err(SourceError::OffsetOutOfBounds {
                offset,
                len: self.text.len(),
            });
        }
        if !self.text.is_char_boundary(offset) {
            return Err(SourceError::NotCharBoundary { offset });
        }
        if tab_width == 0 {
            return Err(SourceError::ZeroTabWidth);
        }

        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        let line_start = self.line_starts[line];
        let content_end = self.line_content_end(line);
        let position_end = offset.min(content_end);
        let prefix = &self.text[line_start..position_end];
        let column = prefix.chars().count();
        let display_column = prefix.chars().fold(0, |column, character| {
            if character == '\t' {
                column + (tab_width - (column % tab_width))
            } else {
                column + 1
            }
        });

        Ok(SourceLocation {
            line,
            column,
            byte_column: position_end - line_start,
            display_column,
        })
    }

    pub fn line_text(&self, line: usize) -> Option<&str> {
        let start = *self.line_starts.get(line)?;
        let end = self.line_content_end(line);
        Some(&self.text[start..end])
    }

    pub fn line_range(&self, line: usize) -> Option<Range<usize>> {
        let start = *self.line_starts.get(line)?;
        Some(start..self.line_content_end(line))
    }

    fn line_content_end(&self, line: usize) -> usize {
        let next_start = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.text.len());
        let mut end = next_start;
        if end > 0 && self.text.as_bytes()[end - 1] == b'\n' {
            end -= 1;
            if end > 0 && self.text.as_bytes()[end - 1] == b'\r' {
                end -= 1;
            }
        }
        end
    }
}

/// Immutable loaded source and its line index.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub id: SourceId,
    pub origin: SourceOrigin,
    text: Arc<str>,
    line_index: LineIndex,
}

impl SourceFile {
    pub fn new(id: SourceId, origin: SourceOrigin, text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let line_index = LineIndex::new(text.clone());
        Self {
            id,
            origin,
            text,
            line_index,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    pub fn slice(&self, span: ByteSpan) -> Result<&str, SourceError> {
        if span.end > self.text.len() {
            return Err(SourceError::OffsetOutOfBounds {
                offset: span.end,
                len: self.text.len(),
            });
        }
        if !self.text.is_char_boundary(span.start) {
            return Err(SourceError::NotCharBoundary { offset: span.start });
        }
        if !self.text.is_char_boundary(span.end) {
            return Err(SourceError::NotCharBoundary { offset: span.end });
        }
        Ok(&self.text[span.as_range()])
    }
}

/// Immutable source collection returned by compilation and analysis.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    sources: BTreeMap<SourceId, SourceFile>,
}

impl SourceMap {
    pub fn insert(&mut self, source: SourceFile) -> Result<(), SourceError> {
        let id = source.id;
        if self.sources.contains_key(&id) {
            return Err(SourceError::DuplicateSourceId(id));
        }
        self.sources.insert(id, source);
        Ok(())
    }

    pub fn get(&self, id: SourceId) -> Option<&SourceFile> {
        self.sources.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SourceId, &SourceFile)> {
        self.sources.iter()
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("span start {start} is after end {end}")]
    ReversedSpan { start: usize, end: usize },
    #[error("byte offset {offset} is outside source length {len}")]
    OffsetOutOfBounds { offset: usize, len: usize },
    #[error("byte offset {offset} is not on a UTF-8 character boundary")]
    NotCharBoundary { offset: usize },
    #[error("tab width must be greater than zero")]
    ZeroTabWidth,
    #[error("duplicate source id {0}")]
    DuplicateSourceId(SourceId),
}
