use std::{fmt, sync::Arc};

use sqlparser::tokenizer::{Location, Token, TokenWithSpan, Tokenizer, Whitespace, Word};

use crate::{
    ByteSpan, Diagnostic, SourceFile, SourceLabel, SourceSpan,
    sql::{AvengerSqlDialect, is_unquoted_identifier},
};

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

/// Token classification used by the tolerant, lossless editing frontend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LosslessTokenKind {
    Token(TokenClass),
    Error,
    Eof,
}

/// One valid or recovered lexical range in a lossless token stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LosslessToken {
    token: Option<Token>,
    kind: LosslessTokenKind,
    span: SourceSpan,
}

impl LosslessToken {
    pub fn token(&self) -> Option<&Token> {
        self.token.as_ref()
    }

    pub const fn kind(&self) -> LosslessTokenKind {
        self.kind
    }

    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

/// Best-effort tokenization that owns every source byte, including malformed
/// ranges that the strict tokenizer rejects.
#[derive(Clone, Debug)]
pub struct LosslessTokenStream {
    source: crate::SourceId,
    text: Arc<str>,
    tokens: Vec<LosslessToken>,
    diagnostics: Vec<Diagnostic>,
}

impl LosslessTokenStream {
    pub const fn source(&self) -> crate::SourceId {
        self.source
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tokens(&self) -> &[LosslessToken] {
        &self.tokens
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn raw(&self, token: &LosslessToken) -> &str {
        &self.text[token.span.range.as_range()]
    }
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
    diagnostic: Box<Diagnostic>,
}

impl TokenizeError {
    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        *self.diagnostic
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
        validate_token_policy(source, &sql.token, span)?;
        let class = if matches!(&sql.token, Token::Word(word) if matches!(word.value.as_str(), "start" | "previous"))
            && tokens.last().is_some_and(|previous: &LanguageToken| {
                matches!(previous.token(), Token::AtSign)
                    && previous.span().range.end == span.range.start
            }) {
            TokenClass::TemporalVersion
        } else {
            classify(&sql.token)
        };
        tokens.push(LanguageToken { class, sql, span });
    }
    validate_token_sequences(source, &tokens)?;
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

/// Tokenize for editor analysis, retaining malformed ranges and continuing
/// after recoverable lexical failures.
pub fn tokenize_lossless(source: &SourceFile) -> LosslessTokenStream {
    let dialect = AvengerSqlDialect::new();
    let mut tokens = Vec::new();
    let mut diagnostics = Vec::new();
    let mut base = 0;

    while base < source.text().len() {
        let remaining = &source.text()[base..];
        let local_source = SourceFile::new(
            source.id,
            source.origin.clone(),
            Arc::<str>::from(remaining),
        );
        let mut sql_tokens = Vec::new();
        let result =
            Tokenizer::new(&dialect, remaining).tokenize_with_location_into_buf(&mut sql_tokens);
        let mut consumed = 0;

        for sql in sql_tokens {
            let local_span = sql_span_to_source(&local_source, sql.span.start, sql.span.end)
                .expect("sqlparser locations must map into the tokenized suffix");
            let span = SourceSpan::new(
                source.id,
                base + local_span.range.start,
                base + local_span.range.end,
            )
            .expect("translated token span is ordered");
            consumed = local_span.range.end;
            match validate_token_policy(source, &sql.token, span) {
                Ok(()) => {
                    let class = if matches!(&sql.token, Token::Word(word) if matches!(word.value.as_str(), "start" | "previous"))
                        && tokens.last().is_some_and(|previous: &LosslessToken| {
                            matches!(previous.token(), Some(Token::AtSign))
                                && previous.span().range.end == span.range.start
                        }) {
                        TokenClass::TemporalVersion
                    } else {
                        classify(&sql.token)
                    };
                    tokens.push(LosslessToken {
                        kind: LosslessTokenKind::Token(class),
                        token: Some(sql.token),
                        span,
                    })
                }
                Err(error) => {
                    diagnostics.push(error.into_diagnostic());
                    tokens.push(LosslessToken {
                        kind: LosslessTokenKind::Error,
                        token: Some(sql.token),
                        span,
                    });
                }
            }
        }

        let Err(error) = result else {
            break;
        };
        let reported =
            sql_location_to_offset(&local_source, error.location).unwrap_or(remaining.len());
        let error_start = consumed.min(remaining.len());
        let mut error_end = if reported < remaining.len() {
            reported
                + remaining[reported..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
        } else {
            remaining.len()
        };
        if error_end <= error_start && error_start < remaining.len() {
            error_end = error_start
                + remaining[error_start..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8);
        }
        let span = SourceSpan::new(source.id, base + error_start, base + error_end)
            .expect("recovered token error span is ordered");
        diagnostics.push(Diagnostic::error(
            "AVENGER-TOKEN-001",
            error.message.clone(),
            SourceLabel::new(span, error.message),
        ));
        if !span.range.is_empty() {
            tokens.push(LosslessToken {
                token: None,
                kind: LosslessTokenKind::Error,
                span,
            });
        }
        if error_end == 0 {
            break;
        }
        base += error_end;
    }

    tokens.push(LosslessToken {
        token: Some(Token::EOF),
        kind: LosslessTokenKind::Eof,
        span: SourceSpan::empty(source.id, source.text().len()),
    });
    LosslessTokenStream {
        source: source.id,
        text: Arc::from(source.text()),
        tokens,
        diagnostics,
    }
}

fn validate_token_sequences(
    source: &SourceFile,
    tokens: &[LanguageToken],
) -> Result<(), TokenizeError> {
    for window in tokens.windows(2) {
        let [prefix, string] = window else {
            unreachable!()
        };
        if matches!(prefix.token(), Token::Word(word) if matches!(word.value.to_ascii_lowercase().as_str(), "q" | "nq"))
            && matches!(string.token(), Token::SingleQuotedString(_))
            && prefix.span().range.end == string.span().range.start
        {
            let span = SourceSpan::new(
                source.id,
                prefix.span().range.start,
                string.span().range.end,
            )
            .expect("ordered adjacent token span");
            return Err(unsupported_token_error(
                span,
                "use an ordinary or `E'...'` string",
            ));
        }
    }
    for window in tokens.windows(3) {
        let [prefix, ampersand, string] = window else {
            unreachable!()
        };
        if matches!(prefix.token(), Token::Word(word) if word.value.eq_ignore_ascii_case("u"))
            && matches!(ampersand.token(), Token::Ampersand)
            && matches!(string.token(), Token::SingleQuotedString(_))
            && prefix.span().range.end == ampersand.span().range.start
            && ampersand.span().range.end == string.span().range.start
        {
            let span = SourceSpan::new(
                source.id,
                prefix.span().range.start,
                string.span().range.end,
            )
            .expect("ordered adjacent token span");
            return Err(unsupported_token_error(
                span,
                "use an ordinary or `E'...'` string",
            ));
        }
    }
    Ok(())
}

fn validate_token_policy(
    source: &SourceFile,
    token: &Token,
    span: SourceSpan,
) -> Result<(), TokenizeError> {
    let raw = &source.text()[span.range.as_range()];
    if raw.starts_with("'''") || raw.starts_with("\"\"\"") {
        return Err(unsupported_token_error(
            span,
            "use a dollar-quoted raw string for multiline text",
        ));
    }
    let replacement = match token {
        Token::Word(Word {
            quote_style: Some('"') | None,
            ..
        })
        | Token::SingleQuotedString(_)
        | Token::EscapedStringLiteral(_) => return Ok(()),
        Token::DollarQuotedString(value) => {
            if value.tag.as_deref().is_none_or(valid_dollar_tag) {
                return Ok(());
            }
            "use `$$...$$` or an ASCII `$tag$...$tag$` raw string"
        }
        Token::HexStringLiteral(_) if raw.starts_with("X'") || raw.starts_with("x'") => {
            return Ok(());
        }
        Token::Word(Word {
            quote_style: Some(_),
            ..
        }) => "use a double-quoted identifier",
        Token::Char('`') => "use a double-quoted identifier",
        Token::DoubleQuotedString(_) => "use a single-quoted string",
        Token::TripleSingleQuotedString(_) | Token::TripleDoubleQuotedString(_) => {
            "use a dollar-quoted raw string for multiline text"
        }
        Token::NationalStringLiteral(_)
        | Token::UnicodeStringLiteral(_)
        | Token::QuoteDelimitedStringLiteral(_)
        | Token::NationalQuoteDelimitedStringLiteral(_) => "use an ordinary or `E'...'` string",
        Token::SingleQuotedByteStringLiteral(_)
        | Token::DoubleQuotedByteStringLiteral(_)
        | Token::TripleSingleQuotedByteStringLiteral(_)
        | Token::TripleDoubleQuotedByteStringLiteral(_)
        | Token::SingleQuotedRawStringLiteral(_)
        | Token::DoubleQuotedRawStringLiteral(_)
        | Token::TripleSingleQuotedRawStringLiteral(_)
        | Token::TripleDoubleQuotedRawStringLiteral(_) => {
            "use `X'...'` for binary or a dollar-quoted raw string for text"
        }
        Token::HexStringLiteral(_) => "use the canonical `X'...'` binary literal",
        _ => return Ok(()),
    };
    Err(unsupported_token_error(span, replacement))
}

fn unsupported_token_error(span: SourceSpan, replacement: &str) -> TokenizeError {
    TokenizeError {
        diagnostic: Box::new(Diagnostic::error(
            "AVENGER-TOKEN-002",
            "unsupported Avenger SQL literal or identifier form",
            SourceLabel::new(span, replacement),
        )),
    }
}

fn valid_dollar_tag(tag: &str) -> bool {
    let mut bytes = tag.bytes();
    matches!(bytes.next(), Some(first) if first.is_ascii_alphabetic() || first == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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
        diagnostic: Box::new(Diagnostic::error(
            "AVENGER-TOKEN-001",
            error.message.clone(),
            SourceLabel::new(span, error.message),
        )),
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
    is_unquoted_identifier(value)
}

fn positional_dollar(value: &str) -> bool {
    value.strip_prefix('$').is_some_and(|rest| {
        !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit())
    })
}

fn is_string(token: &Token) -> bool {
    matches!(
        token,
        Token::SingleQuotedString(_)
            | Token::DollarQuotedString(_)
            | Token::EscapedStringLiteral(_)
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
        Token::DollarQuotedString(value) => value.value.clone(),
        _ => token.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{SourceFile, SourceId, SourceOrigin};

    use super::{LosslessTokenKind, TokenClass, WhitespaceKind, tokenize, tokenize_lossless};

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

    #[test]
    fn lossless_tokenization_retains_unterminated_string_and_suffix() {
        let source = SourceFile::new(
            SourceId::new(2),
            SourceOrigin::Memory("incomplete".into()),
            "x: 'unterminated\ny: 2;",
        );
        let stream = tokenize_lossless(&source);
        let reconstructed = stream
            .tokens()
            .iter()
            .filter(|token| token.kind() != LosslessTokenKind::Eof)
            .map(|token| stream.raw(token))
            .collect::<String>();
        assert_eq!(reconstructed, source.text());
        assert!(
            stream
                .tokens()
                .iter()
                .any(|token| token.kind() == LosslessTokenKind::Error)
        );
        assert!(!stream.diagnostics().is_empty());
    }

    #[test]
    fn lossless_tokenization_recovers_after_invalid_character() {
        let source = SourceFile::new(
            SourceId::new(3),
            SourceOrigin::Memory("recovery".into()),
            "x: 1; § y: 2;",
        );
        let stream = tokenize_lossless(&source);
        let reconstructed = stream
            .tokens()
            .iter()
            .filter(|token| token.kind() != LosslessTokenKind::Eof)
            .map(|token| stream.raw(token))
            .collect::<String>();
        assert_eq!(reconstructed, source.text());
        assert!(stream.tokens().iter().any(|token| stream.raw(token) == "y"));
    }
}
