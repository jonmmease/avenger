//! The places in the text of a query that [`super::lift`] replaces and copies: the byte offsets of
//! its tokens, the range of a pipe operator, and the start of a query. sqlparser-rs 0.62 gives a
//! pipe operator no span, and the span of a query leaves out its pipe operators, so the places come
//! from the tokens.

use std::ops::Range;

use datafusion::{
    common::Result,
    sql::sqlparser::{
        ast::{PipeOperator, Query, SetExpr, Spanned},
        dialect::GenericDialect,
        keywords::Keyword,
        tokenizer::{Location, Token, Tokenizer, Whitespace},
    },
};

use crate::error::at;

/// The text of a query and its tokens, each with its range of bytes in the text.
pub(super) struct Source<'a> {
    text: &'a str,
    tokens: Vec<(Token, Range<usize>)>,
    /// The byte offset of the start of each line.
    lines: Vec<usize>,
}

impl<'a> Source<'a> {
    pub(super) fn new(text: &'a str) -> Result<Self> {
        let lines = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(index, _)| index + 1))
            .collect();
        let mut source = Self {
            text,
            tokens: Vec::new(),
            lines,
        };
        let tokens = Tokenizer::new(&GenericDialect {}, text)
            .tokenize_with_location()
            .map_err(|error| {
                datafusion::common::DataFusionError::SQL(Box::new(error.into()), None)
            })?;
        source.tokens = (tokens.into_iter())
            .filter(|token| token.token != Token::EOF)
            .map(|token| {
                let range = source.offset(token.span.start)..source.offset(token.span.end);
                (token.token, range)
            })
            .collect();
        Ok(source)
    }

    /// The byte offset of `location`, a one-based line and a column of characters.
    fn offset(&self, location: Location) -> usize {
        let Some(&start) = (location.line as usize)
            .checked_sub(1)
            .and_then(|line| self.lines.get(line))
        else {
            return self.text.len();
        };
        let column = (location.column as usize).saturating_sub(1);
        (self.text[start..].char_indices().nth(column))
            .map_or(self.text.len(), |(index, _)| start + index)
    }

    /// The range of the pipe `AGGREGATE` `operator`: from its `|>`, the last before its first item,
    /// to the token before the next `|>` of its query, the parenthesis that closes its query, `;`,
    /// or the end of the text, without the whitespace before that token.
    pub(super) fn operator(&self, operator: &PipeOperator) -> Result<Range<usize>> {
        let PipeOperator::Aggregate {
            full_table_exprs, ..
        } = operator
        else {
            unreachable!("lift replaces a pipe AGGREGATE")
        };
        let span = full_table_exprs[0].expr.expr.span();
        if span.start.line == 0 {
            return Err(at(span, "lift needs the spans of the text of the query"));
        }
        let item = self.offset(span.start);
        let pipe = (self.tokens.iter())
            .rposition(|(token, range)| {
                *token == Token::VerticalBarRightAngleBracket && range.start < item
            })
            .expect("a pipe operator starts with |>");
        let mut depth = 0usize;
        let mut end = self.text.len();
        for (token, range) in &self.tokens[pipe + 1..] {
            match token {
                Token::LParen | Token::LBracket | Token::LBrace => depth += 1,
                Token::RParen | Token::RBracket | Token::RBrace if depth == 0 => {
                    end = range.start;
                    break;
                }
                Token::RParen | Token::RBracket | Token::RBrace => depth -= 1,
                Token::VerticalBarRightAngleBracket | Token::SemiColon if depth == 0 => {
                    end = range.start;
                    break;
                }
                _ => {}
            }
        }
        let start = self.tokens[pipe].1.start;
        Ok(start..(start + self.text[start..end].trim_end().len()))
    }

    /// The byte offset of the `FROM` that starts `query`, the token before its source, after `(`
    /// and `VALUES` for rows of values.
    pub(super) fn query_start(&self, query: &Query) -> Result<usize> {
        let SetExpr::Select(select) = query.body.as_ref() else {
            unreachable!("a pipe query starts with FROM")
        };
        let span = select.from[0].relation.span();
        let source = self.offset(span.start);
        let mut index = (self.tokens.iter())
            .position(|(_, range)| range.start >= source)
            .unwrap_or(self.tokens.len());
        while let Some(previous) = index.checked_sub(1) {
            index = previous;
            match &self.tokens[previous].0 {
                Token::Whitespace(_) | Token::LParen => continue,
                Token::Word(word) if word.keyword == Keyword::VALUES => continue,
                Token::Word(word) if word.keyword == Keyword::FROM => {
                    return Ok(self.tokens[previous].1.start);
                }
                _ => break,
            }
        }
        Err(at(span, "lift needs the FROM of the query"))
    }

    /// The whitespace before `at` on its line, when only whitespace comes before it there.
    pub(super) fn indent(&self, at: usize) -> &'a str {
        let start = self.text[..at].rfind('\n').map_or(0, |index| index + 1);
        let before = &self.text[start..at];
        match before.trim().is_empty() {
            true => before,
            false => "",
        }
    }
}

/// Whether `text` ends in a line comment, which takes the text after it on its line.
pub(super) fn ends_in_comment(text: &str) -> bool {
    let tokens = Tokenizer::new(&GenericDialect {}, text)
        .tokenize()
        .unwrap_or_default();
    matches!(
        tokens.iter().rev().find(|token| !matches!(
            token,
            Token::EOF
                | Token::Whitespace(Whitespace::Space | Whitespace::Tab | Whitespace::Newline)
        )),
        Some(Token::Whitespace(Whitespace::SingleLineComment { .. }))
    )
}

/// The ranges of the runs of `true` in `written`.
pub(super) fn ranges(written: &[bool]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (index, &value) in written.iter().enumerate() {
        match (value, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                ranges.push(from..index);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        ranges.push(from..written.len());
    }
    ranges
}
