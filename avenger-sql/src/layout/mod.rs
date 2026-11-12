//! A layout of SQL for reading, which changes only the whitespace between its tokens: each
//! clause and each pipe operator on its own line, a query in parentheses, such as the target of a
//! join, indented on lines of its own, and a list of items, or the arguments of a call, one to a
//! line when it is long. [`crate::lift`] lays out the text that it writes.

use datafusion::sql::sqlparser::{
    dialect::GenericDialect,
    tokenizer::{Token, Tokenizer, Whitespace},
};

/// The length, in characters, from which a list or the arguments of a call break into lines.
const WIDTH: usize = 60;

/// The clauses that start a line in a query.
const CLAUSES: &[&str] = &[
    "FROM",
    "WHERE",
    "GROUP",
    "HAVING",
    "QUALIFY",
    "WINDOW",
    "ORDER",
    "LIMIT",
    "UNION",
    "INTERSECT",
    "EXCEPT",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "FULL",
    "CROSS",
];

/// The words of a join that come before `JOIN`, which keep `JOIN` on their line.
const JOIN_KINDS: &[&str] = &[
    "LEFT", "RIGHT", "INNER", "FULL", "CROSS", "OUTER", "NATURAL",
];

/// The clauses and the pipe operators whose items a list follows.
const LISTS: &[&str] = &["SELECT", "AGGREGATE", "EXTEND"];

/// A token other than spaces and newlines: its text, its range of characters in the input, and
/// whether whitespace comes before it.
struct Item {
    token: Token,
    text: String,
    start: usize,
    end: usize,
    spaced: bool,
}

impl Item {
    /// The word of the item in uppercase, unless it is quoted or not a word.
    fn word(&self) -> Option<String> {
        match &self.token {
            Token::Word(word) if word.quote_style.is_none() => {
                Some(word.value.to_ascii_uppercase())
            }
            _ => None,
        }
    }

    fn is_pipe(&self) -> bool {
        self.token == Token::VerticalBarRightAngleBracket
    }

    fn is_word_in(&self, words: &[&str]) -> bool {
        self.word()
            .is_some_and(|word| words.contains(&word.as_str()))
    }
}

/// `sql` laid out for reading. Text that the tokenizer cannot read stays as it is.
pub(crate) fn lay_out(sql: &str) -> String {
    let Ok(tokens) = Tokenizer::new(&GenericDialect {}, sql).tokenize_with_location() else {
        return sql.to_string();
    };
    let lines = LineIndex::new(sql);
    let chars: Vec<char> = sql.chars().collect();
    let mut items = Vec::new();
    let mut spaced = false;
    for token in tokens {
        match token.token {
            Token::EOF => {}
            Token::Whitespace(Whitespace::Space | Whitespace::Tab | Whitespace::Newline) => {
                spaced = true;
            }
            _ => {
                let start = lines.index(token.span.start.line, token.span.start.column);
                let end = lines.index(token.span.end.line, token.span.end.column);
                items.push(Item {
                    token: token.token,
                    text: chars[start..end].iter().collect(),
                    start,
                    end,
                    spaced,
                });
                spaced = false;
            }
        }
    }
    Layout::new(&items).run()
}

/// The character index of each line start of a text, which converts the one-based line and
/// column of a location to a character index.
struct LineIndex {
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        let mut len = 0;
        for (index, c) in text.chars().enumerate() {
            if c == '\n' {
                starts.push(index + 1);
            }
            len = index + 1;
        }
        Self { starts, len }
    }

    /// The character index of the one-based `line` and `column`, at most the length of the text.
    fn index(&self, line: u64, column: u64) -> usize {
        let line = (line.max(1) - 1) as usize;
        let Some(start) = self.starts.get(line) else {
            return self.len;
        };
        (start + (column.max(1) - 1) as usize).min(self.len)
    }
}

/// An open parenthesis: the indent of its items, whether they break into lines, and whether it
/// holds a query, whose clauses and pipe operators start lines of their own.
struct Frame {
    indent: usize,
    broken: bool,
    query: bool,
    /// The indent of the line of the parenthesis, where its query closes.
    outer: usize,
    /// Whether the text is in the stages of the pipe query of the parenthesis.
    in_pipe: bool,
}

struct Layout<'a> {
    items: &'a [Item],
    /// The index of the closing parenthesis of each opening one.
    closes: Vec<Option<usize>>,
    out: String,
    /// The indent of the current line, whether nothing follows it yet, and whether it starts with
    /// a pipe operator, whose items line up after `|> `.
    indent: usize,
    at_start: bool,
    pipe_line: bool,
    /// Whether the text is in the stages of the pipe query outside parentheses.
    in_pipe: bool,
    frames: Vec<Frame>,
    /// The list of the current query, when its items break into lines: their indent.
    list: Option<usize>,
}

impl<'a> Layout<'a> {
    fn new(items: &'a [Item]) -> Self {
        let mut closes = vec![None; items.len()];
        let mut open = Vec::new();
        for (index, item) in items.iter().enumerate() {
            match item.token {
                Token::LParen => open.push(index),
                Token::RParen => {
                    if let Some(start) = open.pop() {
                        closes[start] = Some(index);
                    }
                }
                _ => {}
            }
        }
        Self {
            items,
            closes,
            out: String::new(),
            indent: 0,
            at_start: true,
            pipe_line: false,
            in_pipe: false,
            frames: Vec::new(),
            list: None,
        }
    }

    fn run(mut self) -> String {
        for index in 0..self.items.len() {
            self.item(index);
        }
        self.out
    }

    fn newline(&mut self, indent: usize) {
        if self.out.is_empty() {
            return;
        }
        let trimmed = self.out.trim_end_matches(' ').len();
        self.out.truncate(trimmed);
        self.out.push('\n');
        self.out.extend(std::iter::repeat_n(' ', indent));
        self.indent = indent;
        self.at_start = true;
        self.pipe_line = false;
    }

    /// Whether the text is in a query, outside the parentheses of its expressions, with the indent
    /// of the query and whether it is in the stages of a pipe query.
    fn query(&self) -> Option<(usize, bool)> {
        match self.frames.last() {
            None => Some((0, self.in_pipe)),
            Some(frame) if frame.query => Some((frame.indent, frame.in_pipe)),
            Some(_) => None,
        }
    }

    /// Records that the text is in the stages of a pipe query.
    fn enter_pipe(&mut self) {
        match self.frames.last_mut() {
            Some(frame) => frame.in_pipe = true,
            None => self.in_pipe = true,
        }
    }

    /// Whether the item at `index` starts a clause of a query.
    fn starts_clause(&self, index: usize) -> bool {
        let item = &self.items[index];
        let previous = index.checked_sub(1).map(|previous| &self.items[previous]);
        if !item.is_word_in(CLAUSES) || previous.is_some_and(Item::is_pipe) {
            return false;
        }
        // `LEFT JOIN` breaks before `LEFT`, not before `JOIN`.
        let joined = item.word().as_deref() == Some("JOIN")
            && previous.is_some_and(|previous| previous.is_word_in(JOIN_KINDS));
        // The `FROM` that starts a query in parentheses follows the parenthesis, and the `FROM` of
        // `IS [NOT] DISTINCT FROM` is no clause.
        let from = item.word().as_deref() == Some("FROM")
            && previous.is_some_and(|previous| {
                previous.token == Token::LParen || previous.word().as_deref() == Some("DISTINCT")
            });
        !joined && !from
    }

    /// The length of the list that starts at `index`, up to the next clause or pipe operator of
    /// its query.
    fn list_length(&self, index: usize) -> usize {
        let mut depth = 0usize;
        let mut last = None;
        for (at, item) in self.items.iter().enumerate().skip(index) {
            match item.token {
                Token::LParen => depth += 1,
                Token::RParen if depth == 0 => break,
                Token::RParen => depth -= 1,
                _ if depth == 0 && (item.is_pipe() || self.starts_clause(at)) => break,
                _ => {}
            }
            last = Some(item.end);
        }
        last.map_or(0, |end| end.saturating_sub(self.items[index].start))
    }

    fn item(&mut self, index: usize) {
        let items = self.items;
        let item = &items[index];
        let query = self.query();
        if let (Some((indent, _)), true) = (query, item.is_pipe()) {
            self.list = None;
            self.enter_pipe();
            self.newline(indent);
            self.pipe_line = true;
        } else if let (Some((indent, in_pipe)), true) = (query, self.starts_clause(index)) {
            self.list = None;
            // A clause inside a pipe operator, such as the `GROUP BY` of an `AGGREGATE`,
            // lines up after the operator.
            self.newline(if in_pipe { indent + 3 } else { indent });
        } else if item.token == Token::RParen && self.frames.last().is_some_and(|frame| frame.query)
        {
            let outer = self.frames.last().map_or(0, |frame| frame.outer);
            self.newline(outer);
        } else if item.spaced && !self.at_start {
            self.out.push(' ');
        }
        self.out.push_str(&item.text);
        self.at_start = false;
        if let Token::Whitespace(_) = item.token {
            // A comment that ends its line.
            if item.text.ends_with('\n') {
                self.out.extend(std::iter::repeat_n(' ', self.indent));
                self.at_start = true;
            }
        }
        match item.token {
            Token::LParen => {
                let inner = self.closes[index]
                    .map_or(0, |close| self.items[close].start.saturating_sub(item.end));
                // The arguments of a call, but not the clauses of a window.
                let call = index.checked_sub(1).is_some_and(|previous| {
                    let previous = &self.items[previous];
                    matches!(previous.token, Token::Word(_))
                        && previous.word().as_deref() != Some("OVER")
                });
                let opens_query = items
                    .get(index + 1)
                    .is_some_and(|next| next.word().as_deref() == Some("FROM"));
                let base = self.indent + if self.pipe_line { 3 } else { 0 };
                let frame = Frame {
                    indent: base + 2,
                    broken: !opens_query && call && inner > WIDTH,
                    query: opens_query,
                    outer: base,
                    in_pipe: false,
                };
                let (broken, indent) = (frame.broken || frame.query, frame.indent);
                self.frames.push(frame);
                if broken {
                    self.newline(indent);
                }
            }
            Token::RParen => {
                self.frames.pop();
            }
            Token::Comma => match (self.frames.last(), self.list) {
                (Some(frame), _) if frame.broken => {
                    let indent = frame.indent;
                    self.newline(indent);
                }
                (_, Some(indent)) if query.is_some() => self.newline(indent),
                _ => {}
            },
            _ if query.is_some()
                && item.is_word_in(LISTS)
                && index + 1 < items.len()
                && self.list_length(index + 1) > WIDTH =>
            {
                let (base, in_pipe) = query.unwrap_or((0, false));
                let indent = base + if in_pipe { 5 } else { 2 };
                self.list = Some(indent);
                self.newline(indent);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
