use std::{fmt, sync::Arc};

use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer, Whitespace, Word};

use crate::{ByteSpan, Diagnostic, SourceFile, SourceLabel, SourceSpan, sql::AvengerSqlDialect};

/// Stable, DSL-relevant classification over sqlparser's complete token enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenClass {
    Word,
    QuotedIdentifier,
    String,
    Number,
    Punctuation,
    Binding,
    TemporalVersion,
    PositionalPlaceholder,
    OtherPlaceholder,
    Comment(CommentKind),
    Whitespace(WhitespaceKind),
    Eof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    Line,
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WhitespaceKind {
    Space,
    Newline,
    Tab,
}

/// One sqlparser token paired with an exact UTF-8 byte span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageToken {
    sql: TokenWithSpan,
    class: TokenClass,
    span: SourceSpan,
}

impl LanguageToken {
    pub fn sql(&self) -> &TokenWithSpan {
        &self.sql
    }

    pub fn token(&self) -> &Token {
        &self.sql.token
    }

    pub const fn class(&self) -> TokenClass {
        self.class
    }

    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// A contiguous group of `-- |` documentation lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocCommentBlock {
    pub text: String,
    pub span: SourceSpan,
    pub token_range: std::ops::Range<usize>,
}

/// The one authoritative tokenization of an Avenger source file.
#[derive(Clone, Debug)]
pub struct TokenStream {
    source: crate::SourceId,
    text: Arc<str>,
    tokens: Vec<LanguageToken>,
}

impl TokenStream {
    pub const fn source(&self) -> crate::SourceId {
        self.source
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tokens(&self) -> &[LanguageToken] {
        &self.tokens
    }

    pub fn token(&self, index: usize) -> Option<&LanguageToken> {
        self.tokens.get(index)
    }

    pub fn raw(&self, token: &LanguageToken) -> &str {
        &self.text[token.span.range.as_range()]
    }

    pub fn sql_tokens(&self) -> Vec<TokenWithSpan> {
        self.tokens
            .iter()
            .filter(|token| token.class != TokenClass::Eof)
            .map(|token| token.sql.clone())
            .collect()
    }

    pub fn doc_comments(&self) -> Vec<DocCommentBlock> {
        let mut blocks = Vec::new();
        let mut index = 0;
        while index < self.tokens.len() {
            let Some(line) = doc_line(&self.tokens[index]) else {
                index += 1;
                continue;
            };

            let first_index = index;
            let first_span = self.tokens[index].span;
            let mut last_span = first_span;
            let mut previous_line = line_number(&self.text, first_span.range.start);
            let mut lines = vec![line];
            let mut next = index + 1;

            loop {
                let trivia_start = next;
                while matches!(
                    self.tokens.get(next).map(LanguageToken::class),
                    Some(TokenClass::Whitespace(
                        WhitespaceKind::Space | WhitespaceKind::Tab
                    ))
                ) {
                    next += 1;
                }
                let Some(token) = self.tokens.get(next) else {
                    break;
                };
                let Some(next_line_text) = doc_line(token) else {
                    next = trivia_start;
                    break;
                };
                let next_line = line_number(&self.text, token.span.range.start);
                if next_line != previous_line + 1 {
                    next = trivia_start;
                    break;
                }
                lines.push(next_line_text);
                previous_line = next_line;
                last_span = token.span;
                next += 1;
            }

            blocks.push(DocCommentBlock {
                text: lines.join("\n"),
                span: SourceSpan {
                    source: self.source,
                    range: ByteSpan {
                        start: first_span.range.start,
                        end: last_span.range.end,
                    },
                },
                token_range: first_index..next,
            });
            index = next.max(first_index + 1);
        }
        blocks
    }

    /// Stable review representation used by the checked-in token corpus.
    pub fn snapshot(&self) -> String {
        let mut output = String::new();
        for (index, token) in self.tokens.iter().enumerate() {
            let raw = self.raw(token).escape_default().to_string();
            let value = normalized_value(token.token()).escape_default().to_string();
            use std::fmt::Write;
            writeln!(
                output,
                "{index:04} {:?} {}..{} raw=\"{raw}\" value=\"{value}\"",
                token.class, token.span.range.start, token.span.range.end
            )
            .expect("writing to a String cannot fail");
        }
        output
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenizeError {
    diagnostic: Diagnostic,
}

impl TokenizeError {
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        self.diagnostic
    }
}

impl fmt::Display for TokenizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic.message)
    }
}

impl std::error::Error for TokenizeError {}

/// Tokenize a complete source once under the shared Avenger SQL dialect.
pub fn tokenize(source: &SourceFile) -> Result<TokenStream, TokenizeError> {
    let dialect = AvengerSqlDialect::new();
    let sql_tokens = Tokenizer::new(&dialect, source.text())
        .tokenize_with_location()
        .map_err(|error| tokenizer_error(source, error))?;

    let mut tokens = Vec::with_capacity(sql_tokens.len() + 1);
    for sql in sql_tokens {
        let span = sql_span_to_source(source, sql.span.start, sql.span.end)
            .expect("sqlparser locations must map back into the tokenized source");
        tokens.push(LanguageToken {
            class: classify(&sql.token),
            sql,
            span,
        });
    }
    let eof_span = SourceSpan::empty(source.id, source.text().len());
    tokens.push(LanguageToken {
        sql: TokenWithSpan::new_eof(),
        class: TokenClass::Eof,
        span: eof_span,
    });
    Ok(TokenStream {
        source: source.id,
        text: Arc::from(source.text()),
        tokens,
    })
}

fn tokenizer_error(
    source: &SourceFile,
    error: sqlparser::tokenizer::TokenizerError,
) -> TokenizeError {
    let start = sql_location_to_offset(source, error.location).unwrap_or(source.text().len());
    let end = source.text()[start..]
        .chars()
        .next()
        .map_or(start, |character| start + character.len_utf8());
    let span = SourceSpan::new(source.id, start, end).expect("ordered token error span");
    TokenizeError {
        diagnostic: Diagnostic::error(
            "AVENGER-TOKEN-001",
            error.message.clone(),
            SourceLabel::new(span, error.message),
        ),
    }
}

fn sql_span_to_source(
    source: &SourceFile,
    start: Location,
    end: Location,
) -> Result<SourceSpan, crate::SourceError> {
    SourceSpan::new(
        source.id,
        sql_location_to_offset(source, start)?,
        sql_location_to_offset(source, end)?,
    )
}

fn sql_location_to_offset(
    source: &SourceFile,
    location: Location,
) -> Result<usize, crate::SourceError> {
    if location.line == 0 {
        return Ok(source.text().len());
    }
    source.line_index().byte_offset(
        usize::try_from(location.line - 1).expect("line fits usize"),
        usize::try_from(location.column - 1).expect("column fits usize"),
    )
}

fn classify(token: &Token) -> TokenClass {
    match token {
        Token::EOF => TokenClass::Eof,
        Token::Word(word) if is_temporal_spelling(&word.value) => TokenClass::TemporalVersion,
        Token::Word(Word {
            quote_style: Some(_),
            ..
        }) => TokenClass::QuotedIdentifier,
        Token::Word(_) => TokenClass::Word,
        Token::Number(..) => TokenClass::Number,
        Token::Placeholder(value) if value == "?" || positional_dollar(value) => {
            TokenClass::PositionalPlaceholder
        }
        Token::Placeholder(value) if named_binding(value) => TokenClass::Binding,
        Token::Placeholder(_) => TokenClass::OtherPlaceholder,
        Token::Whitespace(Whitespace::Space) => TokenClass::Whitespace(WhitespaceKind::Space),
        Token::Whitespace(Whitespace::Newline) => TokenClass::Whitespace(WhitespaceKind::Newline),
        Token::Whitespace(Whitespace::Tab) => TokenClass::Whitespace(WhitespaceKind::Tab),
        Token::Whitespace(Whitespace::SingleLineComment { .. }) => {
            TokenClass::Comment(CommentKind::Line)
        }
        Token::Whitespace(Whitespace::MultiLineComment(_)) => {
            TokenClass::Comment(CommentKind::Block)
        }
        token if is_string(token) => TokenClass::String,
        _ => TokenClass::Punctuation,
    }
}

pub(crate) fn named_binding(value: &str) -> bool {
    let Some(name) = value.strip_prefix('$') else {
        return false;
    };
    valid_identifier(name)
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    matches!(characters.next(), Some(first) if first.is_alphabetic() || first == '_')
        && characters.all(|character| character.is_alphanumeric() || character == '_')
}

fn positional_dollar(value: &str) -> bool {
    value.strip_prefix('$').is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit())
    })
}

pub(crate) fn is_temporal_spelling(value: &str) -> bool {
    value == "@start"
        || value == "@previous"
        || value.strip_suffix("@start").is_some_and(valid_identifier)
        || value
            .strip_suffix("@previous")
            .is_some_and(valid_identifier)
}

fn is_string(token: &Token) -> bool {
    matches!(
        token,
        Token::SingleQuotedString(_)
            | Token::DoubleQuotedString(_)
            | Token::TripleSingleQuotedString(_)
            | Token::TripleDoubleQuotedString(_)
            | Token::DollarQuotedString(_)
            | Token::SingleQuotedByteStringLiteral(_)
            | Token::DoubleQuotedByteStringLiteral(_)
            | Token::TripleSingleQuotedByteStringLiteral(_)
            | Token::TripleDoubleQuotedByteStringLiteral(_)
            | Token::SingleQuotedRawStringLiteral(_)
            | Token::DoubleQuotedRawStringLiteral(_)
            | Token::TripleSingleQuotedRawStringLiteral(_)
            | Token::TripleDoubleQuotedRawStringLiteral(_)
            | Token::NationalStringLiteral(_)
            | Token::QuoteDelimitedStringLiteral(_)
            | Token::NationalQuoteDelimitedStringLiteral(_)
            | Token::EscapedStringLiteral(_)
            | Token::UnicodeStringLiteral(_)
            | Token::HexStringLiteral(_)
    )
}

fn doc_line(token: &LanguageToken) -> Option<String> {
    let Token::Whitespace(Whitespace::SingleLineComment { prefix, comment }) = token.token() else {
        return None;
    };
    if prefix != "--" {
        return None;
    }
    let content = comment.strip_prefix(" |")?;
    let content = content.strip_prefix(' ').unwrap_or(content);
    Some(content.trim_end_matches(['\r', '\n']).to_owned())
}

fn line_number(text: &str, offset: usize) -> usize {
    text[..offset].bytes().filter(|byte| *byte == b'\n').count()
}

fn normalized_value(token: &Token) -> String {
    match token {
        Token::Word(word) => word.value.clone(),
        Token::Number(value, _) | Token::Placeholder(value) => value.clone(),
        Token::SingleQuotedString(value)
        | Token::DoubleQuotedString(value)
        | Token::TripleSingleQuotedString(value)
        | Token::TripleDoubleQuotedString(value)
        | Token::SingleQuotedByteStringLiteral(value)
        | Token::DoubleQuotedByteStringLiteral(value)
        | Token::TripleSingleQuotedByteStringLiteral(value)
        | Token::TripleDoubleQuotedByteStringLiteral(value)
        | Token::SingleQuotedRawStringLiteral(value)
        | Token::DoubleQuotedRawStringLiteral(value)
        | Token::TripleSingleQuotedRawStringLiteral(value)
        | Token::TripleDoubleQuotedRawStringLiteral(value)
        | Token::NationalStringLiteral(value)
        | Token::EscapedStringLiteral(value)
        | Token::UnicodeStringLiteral(value)
        | Token::HexStringLiteral(value) => value.clone(),
        _ => token.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{SourceFile, SourceId, SourceOrigin};

    use super::{TokenClass, WhitespaceKind, tokenize};

    #[test]
    fn token_stream_preserves_utf8_byte_spans_and_trivia() {
        let source = SourceFile::new(
            SourceId::new(7),
            SourceOrigin::Memory("utf8".into()),
            "café\t+ 1\n",
        );
        let stream = tokenize(&source).unwrap();
        assert_eq!(stream.raw(&stream.tokens()[0]), "café");
        assert_eq!(stream.tokens()[0].span().range.end, 5);
        assert_eq!(
            stream.tokens()[1].class(),
            TokenClass::Whitespace(WhitespaceKind::Tab)
        );
        assert_eq!(stream.tokens().last().unwrap().class(), TokenClass::Eof);
        assert_eq!(stream.tokens().last().unwrap().span().range.start, 10);
    }

    #[test]
    fn token_doc_comments_join_only_contiguous_doc_lines() {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("docs".into()),
            "-- | first\n  -- | second\n\n-- ordinary\n-- | third\n",
        );
        let stream = tokenize(&source).unwrap();
        let docs = stream.doc_comments();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0].text, "first\nsecond");
        assert_eq!(docs[1].text, "third");
    }
}
