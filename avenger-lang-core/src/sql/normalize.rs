use std::{fmt, ops::Range};

use sqlparser::{
    keywords::Keyword,
    tokenizer::{Span, Token, TokenWithSpan, Whitespace, Word},
};

use crate::{ByteSpan, Diagnostic, SourceLabel, SourceSpan};

use super::{TokenClass, TokenStream, token::valid_identifier};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BindingVersion {
    #[default]
    Current,
    Start,
    Previous,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingOccurrence {
    pub synthetic_identifier: String,
    pub path: Vec<String>,
    pub version: BindingVersion,
    pub span: SourceSpan,
    pub token_range: Range<usize>,
}

/// SQL tokens prepared for sqlparser plus the lossless binding side table.
#[derive(Clone, Debug)]
pub struct NormalizedSql {
    tokens: Vec<TokenWithSpan>,
    origins: Vec<Range<usize>>,
    bindings: Vec<BindingOccurrence>,
}

impl NormalizedSql {
    pub fn tokens(&self) -> &[TokenWithSpan] {
        &self.tokens
    }

    pub fn origins(&self) -> &[Range<usize>] {
        &self.origins
    }

    pub fn bindings(&self) -> &[BindingOccurrence] {
        &self.bindings
    }

    pub(crate) fn original_cursor_after(&self, normalized_count: usize, fallback: usize) -> usize {
        self.origins
            .iter()
            .take(normalized_count)
            .map(|origin| origin.end)
            .max()
            .unwrap_or(fallback)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlFrontendError {
    diagnostic: Diagnostic,
}

impl SqlFrontendError {
    pub(crate) fn new(diagnostic: Diagnostic) -> Self {
        Self { diagnostic }
    }

    pub fn diagnostic(&self) -> &Diagnostic {
        &self.diagnostic
    }

    pub fn into_diagnostic(self) -> Diagnostic {
        self.diagnostic
    }
}

impl fmt::Display for SqlFrontendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.diagnostic.message)
    }
}

impl std::error::Error for SqlFrontendError {}

/// Normalize Avenger bindings in an explicit outer-token range. No name or
/// kind resolution occurs at this stage.
pub fn normalize_bindings(
    stream: &TokenStream,
    range: Range<usize>,
) -> Result<NormalizedSql, SqlFrontendError> {
    if range.start > range.end || range.end > stream.tokens().len() {
        return Err(error(
            "AVENGER-SQL-001",
            "invalid SQL token range",
            SourceSpan::empty(stream.source(), stream.text().len()),
            "the requested SQL token range is outside the shared token stream",
        ));
    }

    let mut tokens = Vec::new();
    let mut origins = Vec::new();
    let mut bindings = Vec::new();
    let mut index = range.start;
    while index < range.end {
        let language_token = &stream.tokens()[index];
        if language_token.class() == TokenClass::Eof {
            break;
        }

        if is_unsupported_positional(language_token.token()) {
            return Err(error(
                "AVENGER-SQL-002",
                "positional SQL placeholders are not supported",
                language_token.span(),
                "use a named Avenger binding such as `$threshold`",
            ));
        }

        if let Token::Placeholder(placeholder) = language_token.token() {
            let Some(base) = placeholder.strip_prefix('$') else {
                return Err(invalid_binding(language_token.span()));
            };
            if !valid_identifier(base) {
                return Err(invalid_binding(language_token.span()));
            }

            let parsed = parse_binding(stream, index, range.end, base)?;
            let synthetic_identifier = unique_synthetic_identifier(stream, bindings.len());
            let sql_span = Span::new(
                language_token.sql().span.start,
                stream.tokens()[parsed.end - 1].sql().span.end,
            );
            tokens.push(TokenWithSpan::new(
                Token::Word(Word {
                    value: synthetic_identifier.clone(),
                    quote_style: Some('"'),
                    keyword: Keyword::NoKeyword,
                }),
                sql_span,
            ));
            origins.push(index..parsed.end);
            bindings.push(BindingOccurrence {
                synthetic_identifier,
                path: parsed.path,
                version: parsed.version,
                span: SourceSpan {
                    source: stream.source(),
                    range: ByteSpan {
                        start: language_token.span().range.start,
                        end: stream.tokens()[parsed.end - 1].span().range.end,
                    },
                },
                token_range: index..parsed.end,
            });
            index = parsed.end;
            continue;
        }

        tokens.push(language_token.sql().clone());
        origins.push(index..index + 1);
        index += 1;
    }

    Ok(NormalizedSql {
        tokens,
        origins,
        bindings,
    })
}

fn unique_synthetic_identifier(stream: &TokenStream, ordinal: usize) -> String {
    let base = format!("__avenger_binding_{ordinal:08}");
    let mut candidate = base.clone();
    let mut collision = 0;
    while stream
        .tokens()
        .iter()
        .any(|token| matches!(token.token(), Token::Word(word) if word.value == candidate))
    {
        collision += 1;
        candidate = format!("{base}_{collision}");
    }
    candidate
}

struct ParsedBinding {
    path: Vec<String>,
    version: BindingVersion,
    end: usize,
}

fn parse_binding(
    stream: &TokenStream,
    start: usize,
    limit: usize,
    base: &str,
) -> Result<ParsedBinding, SqlFrontendError> {
    let mut path = vec![base.to_owned()];
    let mut version = BindingVersion::Current;
    let mut end = start + 1;

    loop {
        let before_trivia = end;
        let significant = skip_trivia(stream, end, limit);
        if significant < limit && matches!(stream.tokens()[significant].token(), Token::Period) {
            let segment_index = skip_trivia(stream, significant + 1, limit);
            if segment_index >= limit || stream.tokens()[segment_index].class() == TokenClass::Eof {
                return Err(invalid_path_segment(stream.tokens()[significant].span()));
            }
            let Some(segment_token) = stream
                .tokens()
                .get(segment_index)
                .filter(|_| segment_index < limit)
            else {
                return Err(invalid_path_segment(stream.tokens()[significant].span()));
            };
            let Token::Word(word) = segment_token.token() else {
                return Err(invalid_path_segment(segment_token.span()));
            };
            if word.quote_style.is_some() {
                return Err(invalid_path_segment(segment_token.span()));
            }
            let (segment, temporal) = split_temporal(&word.value);
            if !valid_identifier(segment) {
                return Err(invalid_path_segment(segment_token.span()));
            }
            path.push(segment.to_owned());
            if let Some(temporal) = temporal {
                version = temporal;
            }
            end = segment_index + 1;
            if temporal.is_some() {
                reject_following_path_or_temporal(stream, end, limit)?;
                break;
            }
            continue;
        }

        if significant < limit {
            match stream.tokens()[significant].token() {
                Token::Number(value, _) if value.starts_with('.') => {
                    return Err(invalid_path_segment(stream.tokens()[significant].span()));
                }
                Token::Word(word) if exact_temporal(&word.value).is_some() => {
                    let temporal = exact_temporal(&word.value).expect("matched temporal suffix");
                    if before_trivia != significant {
                        return Err(error(
                            "AVENGER-SQL-005",
                            "temporal binding suffix must be adjacent",
                            stream.tokens()[significant].span(),
                            "remove whitespace before `@start` or `@previous`",
                        ));
                    }
                    if stream.tokens()[end - 1].span().range.end
                        != stream.tokens()[significant].span().range.start
                    {
                        return Err(error(
                            "AVENGER-SQL-005",
                            "temporal binding suffix must be adjacent",
                            stream.tokens()[significant].span(),
                            "remove whitespace before `@start` or `@previous`",
                        ));
                    }
                    version = temporal;
                    end = significant + 1;
                }
                Token::Word(word)
                    if word.value.starts_with('@')
                        && stream.tokens()[end - 1].span().range.end
                            == stream.tokens()[significant].span().range.start =>
                {
                    return Err(error(
                        "AVENGER-SQL-006",
                        "unknown temporal binding suffix",
                        stream.tokens()[significant].span(),
                        "only `@start` and `@previous` are supported",
                    ));
                }
                _ => {}
            }
        }
        break;
    }

    Ok(ParsedBinding { path, version, end })
}

fn reject_following_path_or_temporal(
    stream: &TokenStream,
    end: usize,
    limit: usize,
) -> Result<(), SqlFrontendError> {
    let next = skip_trivia(stream, end, limit);
    if next < limit && matches!(stream.tokens()[next].token(), Token::Period) {
        return Err(error(
            "AVENGER-SQL-004",
            "binding path cannot continue after a temporal suffix",
            stream.tokens()[next].span(),
            "move `@start` or `@previous` to the end of the binding path",
        ));
    }
    Ok(())
}

fn split_temporal(value: &str) -> (&str, Option<BindingVersion>) {
    if let Some(segment) = value.strip_suffix("@start") {
        (segment, Some(BindingVersion::Start))
    } else if let Some(segment) = value.strip_suffix("@previous") {
        (segment, Some(BindingVersion::Previous))
    } else {
        (value, None)
    }
}

fn exact_temporal(value: &str) -> Option<BindingVersion> {
    match value {
        "@start" => Some(BindingVersion::Start),
        "@previous" => Some(BindingVersion::Previous),
        _ => None,
    }
}

fn skip_trivia(stream: &TokenStream, mut index: usize, limit: usize) -> usize {
    while index < limit
        && matches!(
            stream.tokens()[index].token(),
            Token::Whitespace(
                Whitespace::Space
                    | Whitespace::Newline
                    | Whitespace::Tab
                    | Whitespace::SingleLineComment { .. }
                    | Whitespace::MultiLineComment(_)
            )
        )
    {
        index += 1;
    }
    index
}

fn is_unsupported_positional(token: &Token) -> bool {
    match token {
        Token::Placeholder(value) => {
            value == "?"
                || value.strip_prefix('$').is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|character| character.is_ascii_digit())
                })
        }
        Token::Char('?') => true,
        _ => false,
    }
}

fn invalid_binding(span: SourceSpan) -> SqlFrontendError {
    error(
        "AVENGER-SQL-003",
        "invalid Avenger binding",
        span,
        "bindings begin with `$` followed by an unquoted identifier",
    )
}

fn invalid_path_segment(span: SourceSpan) -> SqlFrontendError {
    error(
        "AVENGER-SQL-004",
        "invalid binding path segment",
        span,
        "binding path segments must be unquoted identifiers",
    )
}

pub(crate) fn error(code: &str, message: &str, span: SourceSpan, label: &str) -> SqlFrontendError {
    SqlFrontendError::new(Diagnostic::error(
        code,
        message,
        SourceLabel::new(span, label),
    ))
}

#[cfg(test)]
mod tests {
    use crate::{SourceFile, SourceId, SourceOrigin, sql::tokenize};

    use super::{BindingVersion, normalize_bindings};

    fn normalized(source: &str) -> super::NormalizedSql {
        let source = SourceFile::new(
            SourceId::new(1),
            SourceOrigin::Memory("binding".into()),
            source,
        );
        let stream = tokenize(&source).unwrap();
        normalize_bindings(&stream, 0..stream.tokens().len()).unwrap()
    }

    #[test]
    fn token_binding_normalization_records_paths_versions_and_spans() {
        let normalized = normalized("$component.alias@start + $plain + $deep . path@previous");
        assert_eq!(normalized.bindings().len(), 3);
        assert_eq!(normalized.bindings()[0].path, ["component", "alias"]);
        assert_eq!(normalized.bindings()[0].version, BindingVersion::Start);
        assert_eq!(normalized.bindings()[1].path, ["plain"]);
        assert_eq!(normalized.bindings()[1].version, BindingVersion::Current);
        assert_eq!(normalized.bindings()[2].path, ["deep", "path"]);
        assert_eq!(normalized.bindings()[2].version, BindingVersion::Previous);
        assert_eq!(normalized.bindings()[0].span.range.start, 0);
        assert_eq!(normalized.bindings()[0].span.range.end, 22);
    }

    #[test]
    fn token_binding_normalization_uses_opaque_quoted_identifiers() {
        let normalized = normalized("$width::double precision");
        assert_eq!(
            normalized.tokens()[0].to_string(),
            "\"__avenger_binding_00000000\""
        );
        assert_eq!(normalized.bindings()[0].path, ["width"]);
    }

    #[test]
    fn token_binding_synthetic_identifiers_cannot_collide_with_source_words() {
        let normalized = normalized("$width + \"__avenger_binding_00000000\"");
        assert_eq!(
            normalized.bindings()[0].synthetic_identifier,
            "__avenger_binding_00000000_1"
        );
    }
}
