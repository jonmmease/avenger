//! Conversions between avenger-sql-language's byte offsets and kinds and the protocol's
//! positions and kinds. A position counts its column in UTF-8 bytes or in UTF-16 code units, as
//! the client and the server agree at initialization.

use avenger_sql_language::{
    CompletionKind, Highlight, Offset, Severity, Source, SymbolKind, TextRange,
};
use avenger_sql_syntax::NameRole;
use lsp_types::{
    CompletionItemKind, DiagnosticSeverity, Position, Range, SemanticTokenModifier,
    SemanticTokenType, SymbolKind as LspSymbolKind,
};

/// The unit of the column of a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16,
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
            };
        }
        Offset::from(end as u32)
    }

    pub fn range(&self, range: TextRange) -> Range {
        Range::new(
            self.position(usize::from(range.start())),
            self.position(usize::from(range.end())),
        )
    }

    pub fn text_range(&self, range: Range) -> TextRange {
        let start = self.offset(range.start);
        TextRange::new(start, self.offset(range.end).max(start))
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
    }
}

pub fn completion_kind(kind: CompletionKind) -> CompletionItemKind {
    match kind {
        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
        CompletionKind::Column | CompletionKind::Field => CompletionItemKind::FIELD,
        CompletionKind::RangeVariable | CompletionKind::Variable => CompletionItemKind::VARIABLE,
        CompletionKind::Table | CompletionKind::Query => CompletionItemKind::CLASS,
        CompletionKind::Function(_) => CompletionItemKind::FUNCTION,
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
/// [`NAMESPACE`], or `None` for punctuation, which has no token.
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
