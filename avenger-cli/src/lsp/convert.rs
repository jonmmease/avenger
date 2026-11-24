//! Conversions between avenger-sql-language's byte offsets and kinds and the protocol's
//! positions and kinds. A position counts its column in UTF-8 bytes or in UTF-16 code units, as
//! the client and the server agree at initialization. A [`Place`] converts the offsets of a file
//! of the analysis, a script, a module, or a cell, to the positions of the client's file.

use avenger_sql_language::{
    CompletionKind, Highlight, Offset, Severity, Source, SymbolKind, TextRange,
};
use avenger_sql_syntax::NameRole;

use avenger_sql_project::qmd::Cell;
use lsp_types::{
    CompletionItemKind, DiagnosticSeverity, Position, Range, SemanticTokenModifier,
    SemanticTokenType, SymbolKind as LspSymbolKind,
};

/// The unit of the column of a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
    /// Characters, as the command-line program counts its columns.
    Utf32,
}

/// The starts of the lines of a text, which convert between byte offsets and positions.
pub struct LineIndex<'a> {
    text: &'a str,
    /// The byte offset of the start of each line.
    starts: Vec<usize>,
    encoding: Encoding,
}

impl<'a> LineIndex<'a> {
    pub fn new(text: &'a str, encoding: Encoding) -> LineIndex<'a> {
        let starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        LineIndex {
            text,
            starts,
            encoding,
        }
    }

    /// The spaces and tabs at the start of the line `line`, which are one unit each in every
    /// encoding.
    pub fn indent(&self, line: u32) -> &'a str {
        let Some(&start) = self.starts.get(line as usize) else {
            return "";
        };
        let rest = &self.text[start..];
        &rest[..rest.len() - rest.trim_start_matches([' ', '\t']).len()]
    }

    /// The position of the byte offset `offset`, which is at a character boundary or past the
    /// end.
    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(self.text.len());
        let line = self.starts.partition_point(|start| *start <= offset) - 1;
        let start = self.starts[line];
        let before = &self.text[start..offset];
        let character = match self.encoding {
            Encoding::Utf8 => before.len(),
            Encoding::Utf16 => before.encode_utf16().count(),
            Encoding::Utf32 => before.chars().count(),
        };
        Position::new(line as u32, character as u32)
    }

    /// The byte offset of `position`, clamped to its line, and to the end of the text.
    pub fn offset(&self, position: Position) -> Offset {
        let Some(&start) = self.starts.get(position.line as usize) else {
            return Offset::from(self.text.len() as u32);
        };
        let end = self
            .starts
            .get(position.line as usize + 1)
            .map_or(self.text.len(), |next| {
                // Before the line's `\n`, and its `\r`.
                let end = next - 1;
                match self.text[..end].ends_with('\r') {
                    true => end - 1,
                    false => end,
                }
            });
        let line = &self.text[start..end];
        let mut units = 0;
        for (at, c) in line.char_indices() {
            if units >= position.character as usize {
                return Offset::from((start + at) as u32);
            }
            units += match self.encoding {
                Encoding::Utf8 => c.len_utf8(),
                Encoding::Utf16 => c.len_utf16(),
                Encoding::Utf32 => 1,
            };
        }
        Offset::from(end as u32)
    }

    pub fn text_range(&self, range: Range) -> TextRange {
        let start = self.offset(range.start);
        TextRange::new(start, self.offset(range.end).max(start))
    }
}

/// Where a file of the analysis is in a file of the client: all of it, for a script or a module,
/// or a cell of a document.
pub struct Place<'a> {
    index: LineIndex<'a>,
    cell: Option<&'a Cell>,
}

impl<'a> Place<'a> {
    /// The place of the file `cell` of the client's text `text`, or of all of it without one.
    pub fn new(text: &'a str, cell: Option<&'a Cell>, encoding: Encoding) -> Place<'a> {
        Place {
            index: LineIndex::new(text, encoding),
            cell,
        }
    }

    pub fn cell(&self) -> Option<&'a Cell> {
        self.cell
    }

    /// The position of the offset `offset` of the file of the analysis.
    pub fn position(&self, offset: usize) -> Position {
        match self.cell {
            Some(cell) => self.index.position(cell.to_document(offset)),
            None => self.index.position(offset),
        }
    }

    pub fn range(&self, range: TextRange) -> Range {
        Range::new(
            self.position(usize::from(range.start())),
            self.position(usize::from(range.end())),
        )
    }

    /// The spaces and tabs at the start of the line `line` of the client's file.
    pub fn line_indent(&self, line: u32) -> &'a str {
        self.index.indent(line)
    }

    /// The position of the offset `offset` of the client's file.
    pub fn document_position(&self, offset: usize) -> Position {
        self.index.position(offset)
    }

    /// The offset in the file of the analysis of `position`, or `None` outside the cell.
    pub fn offset(&self, position: Position) -> Option<Offset> {
        let offset = self.index.offset(position);
        match self.cell {
            Some(cell) => Some(Offset::from(cell.to_cell(usize::from(offset))? as u32)),
            None => Some(offset),
        }
    }

    /// The range in the file of the analysis of `range`, cut to the cell, or `None` when the
    /// range misses it.
    pub fn text_range(&self, range: Range) -> Option<TextRange> {
        let Some(cell) = self.cell else {
            return Some(self.index.text_range(range));
        };
        let range = self.index.text_range(range);
        let (start, end) = (usize::from(range.start()), usize::from(range.end()));
        if end < cell.range.start || cell.range.end < start {
            return None;
        }
        let start = cell.to_cell(start.max(cell.range.start)).unwrap_or(0);
        let end = cell
            .to_cell(end.min(cell.range.end))
            .unwrap_or(cell.text.len());
        Some(TextRange::new(
            Offset::from(start as u32),
            Offset::from(end.max(start) as u32),
        ))
    }

    /// The text `replacement` of an edit of the file of the analysis, with the indent of the
    /// cell before each line with text after the first, and at its end after a line break,
    /// before the line of the document that follows.
    pub fn replacement(&self, replacement: &str) -> String {
        let indent = self.cell.map_or("", |cell| cell.indent.as_str());
        if indent.is_empty() || !replacement.contains('\n') {
            return replacement.to_string();
        }
        let mut indented = String::with_capacity(replacement.len());
        for (index, line) in replacement.split_inclusive('\n').enumerate() {
            let empty = line.trim_end_matches(['\n', '\r']).is_empty();
            if index > 0 && !empty {
                indented.push_str(indent);
            }
            indented.push_str(line);
        }
        if replacement.ends_with('\n') {
            indented.push_str(indent);
        }
        indented
    }
}

pub fn severity(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Error => DiagnosticSeverity::ERROR,
        Severity::Warning => DiagnosticSeverity::WARNING,
    }
}

pub fn source(source: Source) -> &'static str {
    match source {
        Source::Syntax => "avenger-sql syntax",
        Source::Analysis => "avenger-sql",
        Source::Project => "avenger project",
        Source::Loader => "avenger loader",
        Source::Typst => "typst",
        Source::Options => "avenger cell options",
    }
}

pub fn completion_kind(kind: CompletionKind) -> CompletionItemKind {
    match kind {
        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        CompletionKind::Column | CompletionKind::Field => CompletionItemKind::FIELD,
        CompletionKind::RangeVariable | CompletionKind::Variable => CompletionItemKind::VARIABLE,
        CompletionKind::Table | CompletionKind::Query => CompletionItemKind::CLASS,
        CompletionKind::Function(_) => CompletionItemKind::FUNCTION,
        CompletionKind::Constant => CompletionItemKind::CONSTANT,
        CompletionKind::Parameter | CompletionKind::Option => CompletionItemKind::PROPERTY,
        CompletionKind::EnumValue => CompletionItemKind::ENUM_MEMBER,
        CompletionKind::Type => CompletionItemKind::TYPE_PARAMETER,
        CompletionKind::Window => CompletionItemKind::VARIABLE,
        CompletionKind::File => CompletionItemKind::FILE,
        CompletionKind::Folder => CompletionItemKind::FOLDER,
    }
}

pub fn symbol_kind(kind: SymbolKind) -> LspSymbolKind {
    match kind {
        SymbolKind::Variable => LspSymbolKind::VARIABLE,
        SymbolKind::Table | SymbolKind::View => LspSymbolKind::CLASS,
        SymbolKind::Schema | SymbolKind::Module => LspSymbolKind::NAMESPACE,
        SymbolKind::Function => LspSymbolKind::FUNCTION,
        SymbolKind::Query => LspSymbolKind::OBJECT,
        SymbolKind::Operator => LspSymbolKind::OPERATOR,
    }
}

/// The token types of the semantic tokens, the standard types alone, in the order of their
/// indices in the legend.
pub const TOKEN_TYPES: [SemanticTokenType; 12] = [
    SemanticTokenType::KEYWORD,
    SemanticTokenType::COMMENT,
    SemanticTokenType::STRING,
    SemanticTokenType::ENUM_MEMBER,
    SemanticTokenType::NUMBER,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::CLASS,
    SemanticTokenType::FUNCTION,
    SemanticTokenType::TYPE,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::PROPERTY,
];

/// The namespace type, after [`TOKEN_TYPES`].
pub const NAMESPACE: SemanticTokenType = SemanticTokenType::NAMESPACE;

/// The token modifiers, in the order of their bits.
pub const TOKEN_MODIFIERS: [SemanticTokenModifier; 1] = [SemanticTokenModifier::DECLARATION];

/// The index of the token type of `highlight` in the legend, which is [`TOKEN_TYPES`] and then
/// [`NAMESPACE`], or `None` for punctuation and Typst markup, which have no tokens.
pub fn token_type(highlight: Highlight) -> Option<u32> {
    let index = |kind: SemanticTokenType| {
        (TOKEN_TYPES.iter().position(|t| *t == kind)).unwrap_or(TOKEN_TYPES.len()) as u32
    };
    Some(match highlight {
        Highlight::Keyword => index(SemanticTokenType::KEYWORD),
        Highlight::Comment => index(SemanticTokenType::COMMENT),
        Highlight::String | Highlight::Uri => index(SemanticTokenType::STRING),
        Highlight::EnumValue | Highlight::Format => index(SemanticTokenType::ENUM_MEMBER),
        Highlight::Number => index(SemanticTokenType::NUMBER),
        Highlight::QueryParameter => index(SemanticTokenType::PARAMETER),
        Highlight::Operator => index(SemanticTokenType::OPERATOR),
        Highlight::Punctuation => return None,
        // A string of Typst markup takes no tokens: the injection of `tree-sitter-avenger-typst`
        // into avenger-sql's grammar highlights it, and the server leaves it to the injection.
        Highlight::Typst(_) => return None,
        Highlight::Name(role) => match role {
            NameRole::Table | NameRole::Query => index(SemanticTokenType::CLASS),
            NameRole::Function(_) => index(SemanticTokenType::FUNCTION),
            NameRole::Type => index(SemanticTokenType::TYPE),
            NameRole::Parameter => index(SemanticTokenType::PARAMETER),
            NameRole::Column | NameRole::Field => index(SemanticTokenType::PROPERTY),
            NameRole::Schema | NameRole::Module => TOKEN_TYPES.len() as u32,
            NameRole::DatePart => index(SemanticTokenType::KEYWORD),
            NameRole::Option => index(SemanticTokenType::PROPERTY),
            NameRole::Other => return None,
            NameRole::Alias
            | NameRole::Window
            | NameRole::PatternVariable
            | NameRole::Variable
            | NameRole::RangeVariable => index(SemanticTokenType::VARIABLE),
        },
    })
}
