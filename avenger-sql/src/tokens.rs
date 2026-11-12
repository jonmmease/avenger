//! The token pass: rewrites of the tokens of the text, before parsing, for the syntax that
//! sqlparser-rs 0.62 does not parse. It reads no schemas and no functions:
//!
//! - A word with the reserved prefix `__avenger_` is an error at the word.
//! - A trailing comma becomes a space: a comma before `)`, `]`, `}`, `|>`, `;`, the end of the
//!   text, or `GROUP BY` or `ORDER BY`, the clauses inside a pipe operator and a call. A comma
//!   before one of these words as a column, as in `greatest(a, order)`, or as the name of an
//!   argument, as in `order => year`, stays. So a call, a list, or the items of an operator can end in a comma. Each is a parser
//!   error of sqlparser-rs 0.62, which takes a trailing comma only in a `SELECT` list. Removed by
//!   sqlparser-rs parsing trailing commas in these lists, before `|>`, and not before the name of
//!   an argument, which its `trailing_commas` option does not do.
//! - A query in parentheses that starts with `FROM`, such as the subquery of `x IN (FROM t |>
//!   SELECT k)`, starts with `SELECT * FROM`, because the parser reads a subquery of an expression
//!   only when it starts with `SELECT` or `WITH`. The inserted tokens have an empty span, which
//!   marks them for the grammar check ([`crate::grammar`]). Removed by sqlparser-rs reading a
//!   subquery that starts with `FROM` in a dialect of FROM-first queries.

use datafusion::{
    common::Result,
    sql::sqlparser::{
        keywords::Keyword,
        tokenizer::{Span, Token, TokenWithSpan, Whitespace},
    },
};

use crate::error::at;

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
    Ok(from_first_subqueries(trailing_commas(tokens)))
}

/// `tokens` with each trailing comma written as a space, which keeps the spans of the others. A
/// comma is trailing before a closer, `)`, `]`, `}`, `|>`, `;`, or the end of the text, and before
/// `GROUP BY` or `ORDER BY`, unquoted. In a pipe query, the other clauses start after `|>`. `group`
/// and `order` can also be columns, as in `greatest(a, order)`, or the names of arguments, as in
/// `order => year`, so a comma before such a word stays. A comma right after an opener or another
/// comma stays, so `f(,)` is still an error.
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

/// `tokens` with `SELECT *` before the `FROM` of each `(` that `FROM` and a source follow, a word
/// or `(`. A column named `from`, as in `greatest(from, a)`, has no source after it. The inserted
/// tokens have an empty span.
fn from_first_subqueries(tokens: Vec<TokenWithSpan>) -> Vec<TokenWithSpan> {
    let mut out = Vec::with_capacity(tokens.len());
    for (index, token) in tokens.iter().enumerate() {
        out.push(token.clone());
        if token.token != Token::LParen {
            continue;
        }
        let from = next(&tokens, index + 1);
        let from = matches!(
            tokens.get(from).map(|t| &t.token),
            Some(Token::Word(word)) if word.quote_style.is_none() && word.keyword == Keyword::FROM
        ) && matches!(
            tokens.get(next(&tokens, from + 1)).map(|t| &t.token),
            Some(Token::Word(_) | Token::LParen)
        );
        if from {
            let inserted = |token: Token| TokenWithSpan::new(token, Span::empty());
            out.extend(
                [
                    Token::make_keyword("SELECT"),
                    Token::Whitespace(Whitespace::Space),
                    Token::Mul,
                    Token::Whitespace(Whitespace::Space),
                ]
                .map(inserted),
            );
        }
    }
    out
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
