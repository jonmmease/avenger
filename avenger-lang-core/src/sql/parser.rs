use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, Query, Select, SelectFlavor, SelectItem, Statement, Visit, Visitor},
    parser::{Parser, ParserError},
    tokenizer::Token,
};

use crate::{ByteSpan, SourceSpan, sql::normalize::error};

use super::{
    AvengerSqlDialect, BindingOccurrence, SqlFrontendError, TokenClass, TokenStream,
    normalize_bindings,
};

pub const RESERVED_HELPER_NAMES: &[&str] = &["EXISTS", "INTERVAL", "STRUCT", "TRIM"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlParseLimits {
    pub max_tokens: usize,
    pub max_recursion_depth: usize,
}

impl Default for SqlParseLimits {
    fn default() -> Self {
        Self {
            max_tokens: 50_000,
            max_recursion_depth: 128,
        }
    }
}

pub fn is_reserved_helper_name(name: &str) -> bool {
    RESERVED_HELPER_NAMES
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

#[derive(Clone, Debug)]
pub struct ParsedSqlIsland<T> {
    pub ast: T,
    /// Exact source range owned by the SQL island. Outer DSL delimiters are
    /// excluded even when sqlparser uses them to determine that the island is
    /// complete.
    pub span: SourceSpan,
    /// First outer token not consumed by sqlparser.
    pub next_token: usize,
    /// Binding occurrences consumed by this island only.
    pub bindings: Vec<BindingOccurrence>,
}

pub fn parse_sql_expression(
    stream: &TokenStream,
    start_token: usize,
) -> Result<ParsedSqlIsland<Expr>, SqlFrontendError> {
    parse_sql_expression_with_limits(stream, start_token, SqlParseLimits::default())
}

pub fn parse_sql_expression_with_limits(
    stream: &TokenStream,
    start_token: usize,
    limits: SqlParseLimits,
) -> Result<ParsedSqlIsland<Expr>, SqlFrontendError> {
    parse_island(stream, start_token, limits, |parser| parser.parse_expr())
}

pub fn parse_sql_projection(
    stream: &TokenStream,
    start_token: usize,
) -> Result<ParsedSqlIsland<Vec<SelectItem>>, SqlFrontendError> {
    parse_sql_projection_with_limits(stream, start_token, SqlParseLimits::default())
}

pub fn parse_sql_projection_with_limits(
    stream: &TokenStream,
    start_token: usize,
    limits: SqlParseLimits,
) -> Result<ParsedSqlIsland<Vec<SelectItem>>, SqlFrontendError> {
    let parsed = parse_island(stream, start_token, limits, |parser| {
        parser.parse_projection()
    })?;
    validate_explicit_projection_aliases(stream, start_token, &parsed)?;
    Ok(parsed)
}

fn validate_explicit_projection_aliases(
    stream: &TokenStream,
    start_token: usize,
    parsed: &ParsedSqlIsland<Vec<SelectItem>>,
) -> Result<(), SqlFrontendError> {
    let mut item_has_as = vec![false];
    let mut item_starts = vec![start_token];
    let mut depth = 0usize;
    for index in start_token..parsed.next_token {
        let Some(token) = stream.token(index) else {
            break;
        };
        match token.token() {
            Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
            Token::RParen | Token::RBracket | Token::RBrace => {
                depth = depth.saturating_sub(1);
            }
            Token::Comma if depth == 0 => {
                item_has_as.push(false);
                item_starts.push(index.saturating_add(1));
            }
            Token::Word(word) if depth == 0 && word.value.eq_ignore_ascii_case("as") => {
                if let Some(has_as) = item_has_as.last_mut() {
                    *has_as = true;
                }
            }
            _ => {}
        }
    }
    for (index, item) in parsed.ast.iter().enumerate() {
        if matches!(
            item,
            SelectItem::ExprWithAlias { .. } | SelectItem::ExprWithAliases { .. }
        ) && !item_has_as.get(index).copied().unwrap_or(false)
        {
            let span = item_starts
                .get(index)
                .and_then(|index| stream.token(*index))
                .map_or(parsed.span, |token| token.span());
            return Err(error(
                "AVENGER-SQL-012",
                "projection aliases require `AS`",
                span,
                "write `expression AS output_name`",
            ));
        }
    }
    Ok(())
}

pub fn parse_sql_query(
    stream: &TokenStream,
    start_token: usize,
) -> Result<ParsedSqlIsland<Box<Query>>, SqlFrontendError> {
    parse_sql_query_with_limits(stream, start_token, SqlParseLimits::default())
}

pub fn parse_sql_query_with_limits(
    stream: &TokenStream,
    start_token: usize,
    limits: SqlParseLimits,
) -> Result<ParsedSqlIsland<Box<Query>>, SqlFrontendError> {
    let parsed = parse_island(stream, start_token, limits, |parser| {
        parser.parse_statement()
    })?;
    let Statement::Query(query) = parsed.ast else {
        let span = stream.token(start_token).map_or(
            SourceSpan::empty(stream.source(), stream.text().len()),
            |token| token.span(),
        );
        return Err(error(
            "AVENGER-SQL-008",
            "SQL property requires one query statement",
            span,
            "expected SELECT, WITH, VALUES, or FROM",
        ));
    };
    if contains_from_first_without_select(&query) {
        let span = stream.token(start_token).map_or(
            SourceSpan::empty(stream.source(), stream.text().len()),
            |token| token.span(),
        );
        return Err(error(
            "AVENGER-SQL-009",
            "FROM-first query requires an explicit SELECT clause",
            span,
            "add SELECT after the FROM clause",
        ));
    }
    Ok(ParsedSqlIsland {
        ast: query,
        span: parsed.span,
        next_token: parsed.next_token,
        bindings: parsed.bindings,
    })
}

fn parse_island<T>(
    stream: &TokenStream,
    start_token: usize,
    limits: SqlParseLimits,
    parse: impl FnOnce(&mut Parser<'_>) -> Result<T, ParserError>,
) -> Result<ParsedSqlIsland<T>, SqlFrontendError> {
    if start_token >= stream.tokens().len() {
        return Err(error(
            "AVENGER-SQL-001",
            "SQL island starts outside the token stream",
            SourceSpan::empty(stream.source(), stream.text().len()),
            "invalid SQL island start",
        ));
    }
    let range_end = start_token
        .saturating_add(limits.max_tokens.saturating_add(1))
        .min(stream.tokens().len());
    let truncated = range_end < stream.tokens().len();
    let normalized = normalize_bindings(stream, start_token..range_end)?;
    let dialect = AvengerSqlDialect::new();
    let mut parser = Parser::new(&dialect)
        .with_recursion_limit(limits.max_recursion_depth)
        .with_tokens_with_locations(normalized.tokens().to_vec());
    let ast = parse(&mut parser).map_err(|parser_error| {
        let token = parser.peek_token();
        let span = normalized
            .tokens()
            .iter()
            .position(|candidate| candidate == &token)
            .and_then(|index| normalized.origins().get(index))
            .and_then(|origin| stream.token(origin.start))
            .map_or_else(
                || SourceSpan::empty(stream.source(), stream.text().len()),
                |token| token.span(),
            );
        if matches!(parser_error, ParserError::RecursionLimitExceeded) {
            error(
                "AVENGER-SQL-011",
                "SQL recursion limit exceeded",
                span,
                &format!(
                    "SQL nesting exceeds the configured depth of {}",
                    limits.max_recursion_depth
                ),
            )
        } else if truncated
            && parser.get_current_index().saturating_add(1) >= normalized.tokens().len()
        {
            sql_token_limit_error(stream, start_token, limits.max_tokens)
        } else {
            error(
                "AVENGER-SQL-007",
                "invalid SQL island",
                span,
                &parser_error.to_string(),
            )
        }
    })?;

    let normalized_count = parser.get_current_index().saturating_add(1);
    let parsed_next_token = normalized.original_cursor_after(normalized_count, start_token);
    let next_token = (parsed_next_token..stream.tokens().len())
        .find(|index| {
            !matches!(
                stream.tokens()[*index].class(),
                TokenClass::Whitespace(_) | TokenClass::Comment(_)
            )
        })
        .unwrap_or(stream.tokens().len());
    if next_token.saturating_sub(start_token) > limits.max_tokens {
        return Err(sql_token_limit_error(
            stream,
            start_token,
            limits.max_tokens,
        ));
    }
    let bindings = normalized
        .bindings()
        .iter()
        .filter(|binding| binding.token_range.start < next_token)
        .cloned()
        .collect();
    let start = stream
        .token(start_token)
        .map_or(stream.text().len(), |token| token.span().range.start);
    let end = stream
        .token(next_token)
        .map_or(stream.text().len(), |token| token.span().range.start);
    Ok(ParsedSqlIsland {
        ast,
        span: SourceSpan {
            source: stream.source(),
            range: ByteSpan { start, end },
        },
        next_token,
        bindings,
    })
}

fn sql_token_limit_error(
    stream: &TokenStream,
    start_token: usize,
    max_tokens: usize,
) -> SqlFrontendError {
    let span = stream.token(start_token).map_or(
        SourceSpan::empty(stream.source(), stream.text().len()),
        |token| token.span(),
    );
    error(
        "AVENGER-SQL-010",
        "SQL token limit exceeded",
        span,
        &format!("SQL island exceeds the configured limit of {max_tokens} tokens"),
    )
}

fn contains_from_first_without_select(query: &Query) -> bool {
    struct FromFirstNoSelectVisitor;

    impl Visitor for FromFirstNoSelectVisitor {
        type Break = ();

        fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
            if select.flavor == SelectFlavor::FromFirstNoSelect {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        }
    }

    query.visit(&mut FromFirstNoSelectVisitor).is_break()
}

#[cfg(test)]
mod tests {
    use sqlparser::ast::{SelectFlavor, SetExpr};

    use crate::{SourceFile, SourceId, SourceOrigin, sql::tokenize};

    use super::{parse_sql_expression, parse_sql_projection, parse_sql_query};

    fn stream(source: &str) -> super::TokenStream {
        let source = SourceFile::new(SourceId::new(1), SourceOrigin::Memory("sql".into()), source);
        tokenize(&source).unwrap()
    }

    #[test]
    fn token_expression_parser_returns_outer_cursor_at_dsl_block() {
        let stream = stream("$width@start::double + CAST(1 AS DOUBLE) { scale: linear; }");
        let parsed = parse_sql_expression(&stream, 0).unwrap();
        assert_eq!(parsed.bindings.len(), 1);
        assert!(
            matches!(
                stream.tokens()[parsed.next_token].token(),
                sqlparser::tokenizer::Token::Whitespace(_)
            ) || matches!(
                stream.tokens()[parsed.next_token].token(),
                sqlparser::tokenizer::Token::LBrace
            )
        );
        let next_significant = stream.tokens()[parsed.next_token..]
            .iter()
            .find(|token| !matches!(token.token(), sqlparser::tokenizer::Token::Whitespace(_)))
            .unwrap();
        assert!(matches!(
            next_significant.token(),
            sqlparser::tokenizer::Token::LBrace
        ));
    }

    #[test]
    fn projection_parser_returns_select_items_and_outer_cursor() {
        let stream = stream("sum(x) AS total, avg(y) AS average;");
        let parsed = parse_sql_projection(&stream, 0).unwrap();
        assert_eq!(parsed.ast.len(), 2);
        assert_eq!(parsed.ast[0].to_string(), "sum(x) AS total");
        assert_eq!(parsed.ast[1].to_string(), "avg(y) AS average");
        assert!(matches!(
            stream.tokens()[parsed.next_token].token(),
            sqlparser::tokenizer::Token::SemiColon
        ));
    }

    #[test]
    fn projection_parser_rejects_implicit_aliases() {
        let stream = stream("sum(x) total;");
        let error = parse_sql_projection(&stream, 0).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-012");
    }

    #[test]
    fn token_query_parser_accepts_standard_and_from_first_select() {
        let standard = stream("SELECT m.x FROM movies AS m;");
        let standard = parse_sql_query(&standard, 0).unwrap();
        assert!(matches!(standard.ast.body.as_ref(), SetExpr::Select(_)));

        let from_first = stream("FROM movies AS m SELECT m.x;");
        let from_first = parse_sql_query(&from_first, 0).unwrap();
        let SetExpr::Select(select) = from_first.ast.body.as_ref() else {
            panic!("expected select");
        };
        assert_eq!(select.flavor, SelectFlavor::FromFirst);
    }

    #[test]
    fn token_query_parser_rejects_from_first_without_select() {
        let root_stream = stream("FROM movies;");
        let error = parse_sql_query(&root_stream, 0).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-009");

        let stream = stream("WITH invalid AS (FROM movies) SELECT * FROM invalid;");
        let error = parse_sql_query(&stream, 0).unwrap_err();
        assert_eq!(error.diagnostic().code.as_str(), "AVENGER-SQL-009");
    }
}
