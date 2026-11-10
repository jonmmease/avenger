//! The token pass: rewrites of the tokens of the text, before parsing, for the syntax that
//! sqlparser-rs 0.62 does not parse. It reads no schemas and no functions:
//!
//! - A word with the reserved prefix `__avenger_` is an error at the word.
//! - `|> CHART <items> [GROUP BY ...]` becomes `|> AGGREGATE __avenger_chart() AS __avenger_chart,
//!   <items> [GROUP BY ...]`, the `AGGREGATE` of a marker and the items, which the syntax pass
//!   consumes. `CHART` is an operator of avenger-sql, so no upstream change removes the item.

use datafusion::{
    common::Result,
    sql::sqlparser::tokenizer::{Token, TokenWithSpan, Whitespace},
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
    chart_markers(tokens)
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
