//! The indentation of a line by the style of avenger-sql's `STYLE.md`, as an editor needs it on
//! Enter, when a line's first token is typed, and on Tab.
//!
//! The indentation reads the lexer's tokens, not the syntax tree. The parser gives every text a
//! tree, but on the unfinished text of an edit, the tree's shape is the parser's recovery: a
//! bracket without its closer ends where the recovery ends it, and a word that it can't place goes
//! into an `ERROR` node. The formatter keeps such a statement as written. The tokens give what is
//! open at the start of a line directly.
//!
//! The indentation follows the depths of the formatter's layout: the innermost open bracket gives
//! its items [`INDENT`] deeper than the depth of its line, and its closer the depth of its line;
//! `|>` takes the depth of its query; `GROUP BY`, `WINDOW`, and a join's `ON` and `USING` take the
//! depth of the operator's keyword; the items of a pipe operator's list go [`INDENT`] deeper than
//! its `|>`; and a line that starts with a binary operator, or that follows a line that ends with
//! one, continues the chain of the line before it. A line's depth is the column of its first
//! token, and of a pipe operator's line the column of its keyword, after `|> `, so that what breaks
//! under a pipe operator's line, or under its clause's line, goes [`PIPE_STEP`] deeper than the
//! depth of the line, and so [`INDENT`] deeper than the `|>`.
//!
//! The rules read what the formatter reads: the depths [`INDENT`], [`PIPE`], and [`PIPE_STEP`] of
//! [`layout`](crate::layout), the formatter's tables of the clauses that go on lines of their
//! own, by their first keywords, and the parser's levels of the binary operators.
//!
//! The first token of a line decides its depth, so a line without one, as after Enter at the end
//! of a line, takes the depth of what most likely comes next: an item after a comma or after a
//! keyword that starts a list, and otherwise the next pipe operator.

use crate::format::{Clause, BRACKET_CLAUSES, OPERATOR_CLAUSES};
use crate::layout::{INDENT, PIPE, PIPE_STEP};
use crate::lexer::{tokenize, Token};
use crate::parser::binary_level;
use crate::SyntaxKind;

use SyntaxKind as K;

/// An open bracket, or an open `CASE`, and the depths of its items and of its closer.
struct Frame {
    /// The depth of the bracket's items, or of `CASE`'s `WHEN` and `ELSE`.
    items: usize,
    /// The depth of the bracket's closer, or of `CASE`'s `END`: that of the line that opens it.
    close: usize,
    /// Whether a query starts the bracket, so that its lines follow the rules of a query.
    query: bool,
    case: bool,
    /// Whether no token has come after the opener yet.
    empty: bool,
}

/// The lines of `text`.
struct Lines<'a> {
    text: &'a str,
}

impl Lines<'_> {
    /// The start of the line that holds `offset`.
    fn start(&self, offset: usize) -> usize {
        self.text[..offset].rfind('\n').map_or(0, |end| end + 1)
    }
}

/// Whether `token` is the word `word`, in any case.
fn is_word(text: &str, token: &Token, word: &str) -> bool {
    token.kind == K::WORD && text[token.range].eq_ignore_ascii_case(word)
}

/// Whether `token` is the operator of a binary expression, which the formatter puts at the start
/// of a line when its chain breaks. `>>` and `<>` start with `>` and `<`, which are operators too.
fn is_operator(text: &str, token: &Token) -> bool {
    binary_level(&[(token.kind, &text[token.range])]).is_some()
}

/// The keywords of a join's clauses, which the formatter keeps on the join's line, and which take
/// the depth of the operator's keyword, as its other clauses do, when a user starts a line with
/// them.
const JOIN_CLAUSES: [&str; 2] = ["ON", "USING"];

/// Whether `word` starts a clause of a pipe operator, at the depth of the operator's keyword.
pub fn starts_operator_clause(word: &str) -> bool {
    (OPERATOR_CLAUSES.iter().flat_map(|clause| clause.keywords))
        .chain(JOIN_CLAUSES.iter())
        .any(|keyword| word.eq_ignore_ascii_case(keyword))
}

/// Whether `token` is a keyword that starts one of `clauses`.
fn starts(clauses: &[Clause], text: &str, token: &Token) -> bool {
    token.kind == K::WORD
        && (clauses.iter().flat_map(|clause| clause.keywords))
            .any(|keyword| text[token.range].eq_ignore_ascii_case(keyword))
}

/// Whether `token` is a keyword that starts a clause whose items go on the lines after its
/// keywords, as `GROUP BY` and `ORDER BY` do.
fn starts_list(text: &str, token: &Token) -> bool {
    (OPERATOR_CLAUSES.iter().chain(BRACKET_CLAUSES))
        .filter(|clause| clause.list)
        .any(|clause| starts(std::slice::from_ref(clause), text, token))
}

/// The depth of the line that starts at `line_start`, a byte offset of `text` at the start of a
/// line, by the style. `None` inside a string or a comment that holds the line's start.
pub fn depth(text: &str, line_start: usize) -> Option<usize> {
    let (tokens, _) = tokenize(text);
    if (tokens.iter()).any(|token| {
        matches!(token.kind, K::STRING | K::BYTES | K::COMMENT)
            && usize::from(token.range.start()) < line_start
            && usize::from(token.range.end()) > line_start
    }) {
        return None;
    }
    let lines = Lines { text };
    let solid = |token: &&Token| !token.kind.is_trivia() && token.kind != K::EOF;
    // The first token of the line, past the comments on lines of their own, which take the depth
    // of the line after them, except that a comment before a closer takes the depth of the items.
    let commented = (tokens.iter())
        .filter(|token| usize::from(token.range.start()) >= line_start)
        .find(|token| token.kind != K::WHITESPACE)
        .is_some_and(|token| token.kind == K::COMMENT);
    // A blank line has no first token, though a token follows on a later line.
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |end| line_start + end);
    let blank = text[line_start..line_end].trim().is_empty();
    let first = (tokens.iter())
        .filter(|token| usize::from(token.range.start()) >= line_start)
        .find(solid)
        .filter(|_| !blank);
    let before: Vec<&Token> = (tokens.iter())
        .filter(|token| usize::from(token.range.end()) <= line_start)
        .filter(solid)
        .collect();

    // The depth of the line of `offset`, as written.
    let line_depth = |offset: usize| {
        let start = lines.start(offset);
        let rest = &text[start..];
        let indent = rest.chars().take_while(|c| *c == ' ' || *c == '\t').count();
        match rest.trim_start().starts_with("|>") {
            true => indent + PIPE,
            false => indent,
        }
    };
    // How much deeper than the line of `offset` a part goes that breaks under it: [`PIPE_STEP`]
    // under a pipe operator's line or its clause's line, at the top of the operator's body, and
    // [`INDENT`] under any other line.
    let step = |offset: usize| {
        let start = lines.start(offset);
        let rest = text[start..].trim_start_matches([' ', '\t']);
        let word = rest
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .next()
            .unwrap_or("");
        match rest.starts_with("|>") || starts_operator_clause(word) {
            true => PIPE_STEP,
            false => INDENT,
        }
    };
    // The start of the line that opens what holds `offset`: the line of `offset`, or, when that
    // line starts with a closer, the line of the closer's bracket, and so on, since what follows a
    // closer on its line continues what its bracket's line starts.
    let opening_line = |offset: usize| {
        let mut start = lines.start(offset);
        loop {
            let at = (before.iter()).position(|token| usize::from(token.range.start()) >= start);
            let Some(at) = at.filter(|at| matches!(before[*at].kind, K::R_PAREN | K::R_BRACK))
            else {
                return start;
            };
            let mut open = 0usize;
            let opener = (0..=at).rev().find(|index| {
                match before[*index].kind {
                    K::R_PAREN | K::R_BRACK => open += 1,
                    K::L_PAREN | K::L_BRACK => open = open.saturating_sub(1),
                    _ => {}
                }
                open == 0
            });
            match opener {
                Some(index) => start = lines.start(usize::from(before[index].range.start())),
                None => return start,
            }
        }
    };
    // The frame of a bracket or a `CASE` that opens at `start`.
    let open = |start: usize, case: bool| {
        let line = opening_line(start);
        Frame {
            items: line_depth(line) + step(line),
            close: line_depth(line),
            query: false,
            case,
            empty: !case,
        }
    };
    let mut frames: Vec<Frame> = Vec::new();
    for token in &before {
        let start = usize::from(token.range.start());
        if let Some(frame) = frames.last_mut() {
            if frame.empty {
                frame.empty = false;
                frame.query = !frame.case && is_word(text, token, "FROM");
            }
        }
        match token.kind {
            K::SEMICOLON => frames.clear(),
            K::L_PAREN | K::L_BRACK => frames.push(open(start, false)),
            K::R_PAREN | K::R_BRACK => {
                // A `CASE` without its `END` closes with the bracket around it.
                while frames.last().is_some_and(|frame| frame.case) {
                    frames.pop();
                }
                frames.pop();
            }
            K::WORD if is_word(text, token, "CASE") => frames.push(open(start, true)),
            K::WORD
                if is_word(text, token, "END") && frames.last().is_some_and(|frame| frame.case) =>
            {
                frames.pop();
            }
            _ => {}
        }
    }
    let frame = frames.last();
    // The depth of the query that holds the line.
    let base = frame.map_or(0, |frame| frame.items);
    let prev = before.last().copied();

    if let Some(first) = first {
        match first.kind {
            K::R_PAREN | K::R_BRACK => {
                let frame = frames.iter().rev().find(|frame| !frame.case);
                return Some(frame.map_or(0, |frame| match commented {
                    true => frame.items,
                    false => frame.close,
                }));
            }
            K::PIPE => {
                let query = frames.iter().rev().find(|frame| !frame.case);
                return Some(query.map_or(0, |frame| frame.items));
            }
            K::WORD if is_word(text, first, "END") && frame.is_some_and(|frame| frame.case) => {
                return frame.map(|frame| frame.close);
            }
            _ => {}
        }
    }
    let Some(prev) = prev else {
        return Some(0);
    };
    if prev.kind == K::SEMICOLON {
        return Some(0);
    }
    // The depth of a part that breaks under the line that opens what ends at `prev`.
    let under_previous = || {
        let start = opening_line(usize::from(prev.range.start()));
        line_depth(start) + step(start)
    };
    // Whether the line of `prev` holds only keywords after its `|>` or its clause's keyword, as a
    // pipe operator's keyword or `GROUP BY` does, so that its list goes on the lines after it.
    let head = || {
        let start = lines.start(usize::from(prev.range.start()));
        let on_line: Vec<&&Token> = (before.iter())
            .filter(|token| usize::from(token.range.start()) >= start)
            .collect();
        let Some(first) = on_line.first() else {
            return false;
        };
        let opens = first.kind == K::PIPE || starts_list(text, first);
        let keyword = |token: &&&Token| {
            token.kind == K::WORD && SyntaxKind::keyword(&text[token.range]).is_some()
        };
        opens && on_line.iter().skip(1).all(keyword)
    };
    // The depth of the line of the item that `prev` ends, past the lines of its chain of
    // operators, as for a comma after a chain that breaks.
    let item_line = || {
        let mut start = lines.start(usize::from(prev.range.start()));
        loop {
            let line_first =
                (before.iter()).find(|token| usize::from(token.range.start()) >= start);
            match line_first {
                Some(token) if is_operator(text, token) && start > 0 => {
                    start = lines.start(start - 1);
                }
                _ => return line_depth(start),
            }
        }
    };
    let opened = matches!(prev.kind, K::L_PAREN | K::L_BRACK);
    let item = prev.kind == K::COMMA || opened || head();
    // Whether `prev` is a binary operator that ends its line, as `AND` in `|> WHERE x AND`, so
    // that the chain continues on the line. A `*` continues one only after an operand, since
    // after a keyword, such as `SELECT`, or after `(`, `,`, or `.`, it is the star of all the
    // columns.
    let operand = |token: &Token| match token.kind {
        K::WORD => SyntaxKind::keyword(&text[token.range]).is_none(),
        kind => matches!(
            kind,
            K::QUOTED_IDENT
                | K::STRING
                | K::INTEGER
                | K::FLOAT
                | K::PARAM
                | K::R_PAREN
                | K::R_BRACK
        ),
    };
    let trailing = is_operator(text, prev)
        && (prev.kind != K::STAR || before.len() >= 2 && operand(before[before.len() - 2]));
    let starts_operator = first.is_some_and(|first| is_operator(text, first));
    if (starts_operator || trailing) && !item {
        let start = lines.start(usize::from(prev.range.start()));
        let line_first = (before.iter()).find(|token| usize::from(token.range.start()) >= start);
        return Some(match line_first {
            Some(token) if is_operator(text, token) => line_depth(start),
            _ => under_previous(),
        });
    }
    if let Some(frame) = frame.filter(|frame| !frame.query) {
        let clause = first.is_some_and(|first| starts(BRACKET_CLAUSES, text, first));
        if clause {
            return Some(frame.items);
        }
        if !opened && head() {
            return Some(under_previous());
        }
        if prev.kind == K::COMMA {
            return Some(frame.items.max(item_line()));
        }
        return Some(frame.items);
    }
    if first
        .is_some_and(|first| first.kind == K::WORD && starts_operator_clause(&text[first.range]))
    {
        return Some(base + PIPE);
    }
    // The items of a pipe operator's list, or of its clause's, [`INDENT`] deeper than the `|>`.
    if prev.kind == K::COMMA || head() {
        return Some(base + INDENT);
    }
    Some(base)
}
