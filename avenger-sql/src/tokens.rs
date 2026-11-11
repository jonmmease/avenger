//! The token pass: rewrites of the tokens of the text, before parsing, for the syntax that
//! sqlparser-rs 0.62 does not parse. It reads no schemas and no functions:
//!
//! - A word with the reserved prefix `__avenger_` is an error at the word.
//! - A trailing comma becomes a space: a comma before `)`, `]`, `}`, `|>`, `;`, the end of the
//!   text, or a clause keyword that starts its clause, `GROUP BY`, `ORDER BY`, or `FROM`, `WHERE`,
//!   `HAVING`, or `LIMIT` before the start of its body. A comma before one of these words as a
//!   column, as in `greatest(a, order)`, or as the name of an argument, as in `order => year`,
//!   stays. So a call, a list, or the items of an operator can end in a comma. Each is a parser
//!   error of sqlparser-rs 0.62, which takes a trailing comma only in a `SELECT` list. Removed by
//!   sqlparser-rs parsing trailing commas in these lists, before `|>`, and not before the name of
//!   an argument, which its `trailing_commas` option does not do.
//! - `|> CHART <items> [GROUP BY ...]` becomes `|> AGGREGATE __avenger_chart() AS __avenger_chart,
//!   <items> [GROUP BY ...]`, the `AGGREGATE` of a marker and the items, which the syntax pass
//!   consumes. `CHART` is an operator of avenger-sql, so no upstream change removes the item.

use datafusion::{
    common::Result,
    sql::sqlparser::{
        keywords::Keyword,
        tokenizer::{Token, TokenWithSpan, Whitespace},
    },
};

use crate::{chart::MARKER, error::at};

/// The prefix of the names that the passes generate, which the text of a query cannot hold.
pub(crate) const RESERVED: &str = "__avenger_";

/// The tokens of a query after the token pass.
pub(crate) fn rewrite(tokens: Vec<TokenWithSpan>) -> Result<Vec<TokenWithSpan>> {
    for token in &tokens {
        if let Token::Word(word) = &token.token {
            if word.value.to_lowercase().starts_with(RESERVED) {
                return Err(at(
                    token.span,
                    format!(
                        "`{}` starts with `{RESERVED}`, a prefix that avenger-sql reserves",
                        word.value
                    ),
                ));
            }
        }
    }
    chart_markers(trailing_commas(tokens))
}

/// `tokens` with each trailing comma written as a space, which keeps the spans of the others. A
/// comma is trailing before a closer, `)`, `]`, `}`, `|>`, `;`, or the end of the text, and before
/// a clause keyword that starts its clause, unquoted: `GROUP BY` and `ORDER BY`, and `FROM`,
/// `WHERE`, `HAVING`, and `LIMIT` before a token that can start the body of the clause. Each of
/// these words can also be a column, as in `greatest(a, order)` or `ORDER BY x, limit + 1`, or
/// the name of an argument, as in `order => year`, so a comma before such a word stays. A comma
/// right after an opener or another comma stays, so `f(,)` is still an error.
fn trailing_commas(mut tokens: Vec<TokenWithSpan>) -> Vec<TokenWithSpan> {
    for index in 0..tokens.len() {
        if tokens[index].token != Token::Comma {
            continue;
        }
        let empty = matches!(
            previous(&tokens, index).map(|t| &t.token),
            None | Some(Token::LParen | Token::LBracket | Token::LBrace | Token::Comma)
        );
        let after = next(&tokens, index + 1);
        let following = tokens.get(next(&tokens, after + 1)).map(|t| &t.token);
        let trailing = match tokens.get(after).map(|t| &t.token) {
            None | Some(Token::EOF) => true,
            Some(
                Token::RParen
                | Token::RBracket
                | Token::RBrace
                | Token::VerticalBarRightAngleBracket
                | Token::SemiColon,
            ) => true,
            Some(Token::Word(word)) if word.quote_style.is_none() => match word.keyword {
                Keyword::GROUP | Keyword::ORDER => {
                    matches!(following, Some(Token::Word(by)) if by.keyword == Keyword::BY)
                }
                Keyword::FROM | Keyword::WHERE | Keyword::HAVING | Keyword::LIMIT => {
                    following.is_some_and(starts_body)
                }
                _ => false,
            },
            Some(_) => false,
        };
        if trailing && !empty {
            tokens[index].token = Token::Whitespace(Whitespace::Space);
        }
    }
    tokens
}

/// Whether `token` can start the body of a clause after its keyword: a word other than `AS`, a
/// literal, or `(`. An operator, `=>`, `:=`, a closer, or `,` follows a column of the name.
fn starts_body(token: &Token) -> bool {
    match token {
        Token::Word(word) => word.keyword != Keyword::AS,
        Token::Number(..)
        | Token::SingleQuotedString(_)
        | Token::DoubleQuotedString(_)
        | Token::LParen => true,
        _ => false,
    }
}

/// The last token before `index` that is not blank.
fn previous(tokens: &[TokenWithSpan], index: usize) -> Option<&TokenWithSpan> {
    tokens[..index].iter().rev().find(|t| !blank(&t.token))
}

/// Whether `token` is whitespace or a comment, which the parser skips.
fn blank(token: &Token) -> bool {
    matches!(token, Token::Whitespace(_))
}

/// The index of the first token at or after `index` that is not blank.
fn next(tokens: &[TokenWithSpan], index: usize) -> usize {
    (index..tokens.len())
        .find(|i| !blank(&tokens[*i].token))
        .unwrap_or(tokens.len())
}

/// Writes each `|> CHART` as `|> AGGREGATE __avenger_chart() AS __avenger_chart,`, so the parser
/// reads the items and the `GROUP BY` of the `CHART` as those of an `AGGREGATE`. The inserted
/// tokens take the span of `CHART`, and the tokens after it keep theirs.
fn chart_markers(tokens: Vec<TokenWithSpan>) -> Result<Vec<TokenWithSpan>> {
    let mut out = Vec::with_capacity(tokens.len());
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        out.push(token.clone());
        index += 1;
        if token.token != Token::VerticalBarRightAngleBracket {
            continue;
        }
        let chart = next(&tokens, index);
        let is_chart = matches!(
            tokens.get(chart).map(|t| &t.token),
            Some(Token::Word(word)) if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("chart")
        );
        if !is_chart {
            continue;
        }
        let span = tokens[chart].span;
        let item = next(&tokens, chart + 1);
        let ends = matches!(
            tokens.get(item).map(|t| &t.token),
            None | Some(
                Token::EOF | Token::VerticalBarRightAngleBracket | Token::RParen | Token::SemiColon
            )
        );
        if ends {
            return Err(at(
                span,
                "CHART takes a mark call, such as `|> CHART bar(x => category, y => count(*))`",
            ));
        }
        out.extend(tokens[index..chart].iter().cloned());
        let inserted = |token: Token| TokenWithSpan::new(token, span);
        out.extend(
            [
                Token::make_keyword("AGGREGATE"),
                Token::Whitespace(Whitespace::Space),
                Token::make_word(MARKER, None),
                Token::LParen,
                Token::RParen,
                Token::Whitespace(Whitespace::Space),
                Token::make_keyword("AS"),
                Token::Whitespace(Whitespace::Space),
                Token::make_word(MARKER, None),
                Token::Comma,
                Token::Whitespace(Whitespace::Space),
            ]
            .map(inserted),
        );
        index = chart + 1;
    }
    Ok(out)
}
