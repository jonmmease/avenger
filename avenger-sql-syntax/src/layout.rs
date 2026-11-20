//! The layout of avenger-sql text in lines of at most [`WIDTH`] characters, which the formatter
//! and the printer of lifted calls share.
//!
//! A [`Doc`] is text with the places where it can break: a group of a [`Doc`] stays on one line
//! when it fits in the rest of its line, and otherwise breaks at each of its own lines, while the
//! groups inside it decide again. A line that breaks starts at the indentation of its nesting, so
//! a part that breaks onto lines of its own is two spaces deeper than the line that opens it.
//! [`list`] lays out a call or an array by the style's rule: on one line, or with each item on a
//! line of its own, followed by a comma, and the closer at the depth of the line that opens it.
//!
//! The layout follows Wadler's "prettier printer": a group fits when the text up to the next line
//! that breaks outside it fits in the width.

use crate::{lexer::tokenize, SyntaxKind};

/// The longest line of the layout, in characters.
pub const WIDTH: usize = 100;

/// The columns that a part that breaks onto lines of its own goes deeper than the line that opens
/// it, as the items of a list or the operators of a chain.
pub const INDENT: usize = 2;

/// The columns of `|> `, which the keyword of a pipe operator and the lines of its body that break
/// go deeper than the depth of its query.
pub const PIPE: usize = 3;

/// Text with the places where it can break.
#[derive(Clone, Debug)]
pub enum Doc {
    Text(String),
    /// A space when its group stays on one line, and a line break when it breaks.
    Line,
    /// Nothing when its group stays on one line, and a line break when it breaks.
    SoftLine,
    /// A line break in every layout, which breaks each group around it, as after a line comment.
    HardLine,
    /// A doc whose lines that break start `n` columns deeper.
    Nest(usize, Box<Doc>),
    /// A doc that stays on one line when it fits, unless it breaks, as `broken` says, because it
    /// holds a [`Doc::HardLine`] or a group that breaks.
    Group {
        doc: Box<Doc>,
        broken: bool,
    },
    Concat(Vec<Doc>),
    /// The first doc when its group breaks, and the second when the group stays on one line.
    IfBreak(Box<Doc>, Box<Doc>),
    /// Text written at the end of its line, before the next line break, as a comment after the
    /// last token of a line.
    LineSuffix(String),
    /// Nothing, which breaks each group around it, as a comment at the end of a line does.
    BreakParent,
}

impl Doc {
    pub fn text(text: impl Into<String>) -> Doc {
        Doc::Text(text.into())
    }

    pub fn concat(docs: impl IntoIterator<Item = Doc>) -> Doc {
        Doc::Concat(docs.into_iter().collect())
    }

    pub fn nest(n: usize, doc: Doc) -> Doc {
        Doc::Nest(n, Box::new(doc))
    }

    /// A group of `doc`, which breaks when `doc` holds a line that always breaks.
    pub fn group(doc: Doc) -> Doc {
        let broken = doc.breaks();
        Doc::Group {
            doc: Box::new(doc),
            broken,
        }
    }

    pub fn if_break(broken: Doc, flat: Doc) -> Doc {
        Doc::IfBreak(Box::new(broken), Box::new(flat))
    }

    /// The doc, with its outermost group broken, as by a comma after the last item of a list.
    pub fn broken(self) -> Doc {
        match self {
            Doc::Group { doc, .. } => Doc::Group { doc, broken: true },
            doc => doc,
        }
    }

    /// Whether the doc holds a line that breaks in every layout.
    fn breaks(&self) -> bool {
        match self {
            Doc::HardLine | Doc::BreakParent => true,
            Doc::Group { broken, .. } => *broken,
            Doc::Nest(_, doc) => doc.breaks(),
            Doc::Concat(docs) => docs.iter().any(Doc::breaks),
            Doc::Text(_) | Doc::Line | Doc::SoftLine | Doc::IfBreak(..) | Doc::LineSuffix(_) => {
                false
            }
        }
    }

    /// The doc laid out from `column` of its first line, with lines of at most [`WIDTH`]
    /// characters where its groups can break. A line that breaks has no trailing spaces, so a
    /// blank line is empty. Text that holds a line break, as a statement kept as written, counts
    /// its last line toward the column.
    pub fn render(&self, column: usize) -> String {
        let mut out = String::new();
        let mut column = column;
        // The indentation of the line that the last break started, written before its first text.
        let mut pending: Option<usize> = None;
        // The text at the end of the current line, written before its line break.
        let mut suffixes: Vec<&str> = Vec::new();
        let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, self)];
        while let Some((indent, mode, doc)) = stack.pop() {
            match doc {
                Doc::Text(text) => {
                    if let Some(indent) = pending.take() {
                        out.extend(std::iter::repeat_n(' ', indent));
                    }
                    out.push_str(text);
                    column = match text.rfind('\n') {
                        Some(end) => width(&text[end + 1..]),
                        None => column + width(text),
                    };
                }
                Doc::Line | Doc::SoftLine if mode == Mode::Flat => {
                    if matches!(doc, Doc::Line) {
                        out.push(' ');
                        column += 1;
                    }
                }
                Doc::Line | Doc::SoftLine | Doc::HardLine => {
                    for suffix in suffixes.drain(..) {
                        out.push_str(suffix);
                    }
                    out.push('\n');
                    pending = Some(indent);
                    column = indent;
                }
                Doc::Nest(n, doc) => stack.push((indent + n, mode, doc)),
                Doc::Group { doc, broken } => {
                    let flat = mode == Mode::Flat
                        || (!broken && fits(WIDTH as isize - column as isize, doc, &stack));
                    let mode = match flat {
                        true => Mode::Flat,
                        false => Mode::Break,
                    };
                    stack.push((indent, mode, doc));
                }
                Doc::Concat(docs) => {
                    stack.extend(docs.iter().rev().map(|doc| (indent, mode, doc)));
                }
                Doc::IfBreak(broken, flat) => {
                    let doc = match mode {
                        Mode::Break => broken,
                        Mode::Flat => flat,
                    };
                    stack.push((indent, mode, doc));
                }
                Doc::LineSuffix(text) => suffixes.push(text),
                Doc::BreakParent => {}
            }
        }
        for suffix in suffixes {
            out.push_str(suffix);
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// Whether `doc` on one line, then the text that follows it up to the next line that breaks, fits
/// in `remaining` characters. `rest` is the stack of the docs after `doc`, in their own modes.
fn fits(mut remaining: isize, doc: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
    let mut stack: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
    let mut rest = rest.iter().rev();
    loop {
        if remaining < 0 {
            return false;
        }
        let Some((mode, doc)) = stack
            .pop()
            .or_else(|| rest.next().map(|(_, mode, doc)| (*mode, *doc)))
        else {
            return true;
        };
        match doc {
            Doc::Text(text) => match text.split_once('\n') {
                Some((first, _)) => return remaining >= width(first) as isize,
                None => remaining -= width(text) as isize,
            },
            Doc::LineSuffix(_) | Doc::BreakParent => {}
            Doc::Line | Doc::SoftLine if mode == Mode::Flat => {
                if matches!(doc, Doc::Line) {
                    remaining -= 1;
                }
            }
            Doc::Line | Doc::SoftLine | Doc::HardLine => return true,
            Doc::Nest(_, doc) => stack.push((mode, doc)),
            Doc::Group { doc, broken } => {
                let mode = match broken {
                    true => Mode::Break,
                    false => mode,
                };
                stack.push((mode, doc));
            }
            Doc::Concat(docs) => stack.extend(docs.iter().rev().map(|doc| (mode, doc))),
            Doc::IfBreak(broken, flat) => stack.push((
                mode,
                match mode {
                    Mode::Break => broken,
                    Mode::Flat => flat,
                },
            )),
        }
    }
}

/// The number of characters of `text`.
pub fn width(text: &str) -> usize {
    text.chars().count()
}

/// An item of a list: its doc, whether a blank line comes before it, and the doc that follows
/// its comma, such as a comment at the end of its line.
#[derive(Clone, Debug)]
pub struct Item {
    pub doc: Doc,
    pub blank: bool,
    pub after_comma: Option<Doc>,
}

impl Item {
    pub fn new(doc: Doc) -> Item {
        Item {
            doc,
            blank: false,
            after_comma: None,
        }
    }
}

/// A call or an array, `open` with its `items` and `clauses` and `close`: on one line, with the
/// items separated by `, ` and each clause after a space, or broken after `open`, with each item
/// on a line of its own, two spaces deeper, followed by a comma, then each clause on a line of
/// its own, and `close` back at the depth of the line that opens it.
pub fn list(open: impl Into<String>, items: Vec<Doc>, clauses: Vec<Doc>, close: &str) -> Doc {
    let items = items.into_iter().map(Item::new).collect();
    list_of(
        Doc::Text(open.into()),
        items,
        clauses,
        None,
        Doc::text(close),
    )
}

/// The list of [`list`], with items that keep a blank line before them, which breaks the list,
/// and `tail`, such as the comments before the closer, on lines of their own after the clauses,
/// at the depth of the items.
pub fn list_of(
    open: Doc,
    items: Vec<Item>,
    clauses: Vec<Doc>,
    tail: Option<Doc>,
    close: Doc,
) -> Doc {
    if items.is_empty() && clauses.is_empty() && tail.is_none() {
        return Doc::concat([open, close]);
    }
    let mut inner = vec![Doc::SoftLine];
    let count = items.len();
    for (index, item) in items.into_iter().enumerate() {
        if item.blank && index > 0 {
            inner.push(Doc::HardLine);
        }
        inner.push(item.doc);
        match index + 1 == count {
            true => inner.push(Doc::if_break(Doc::text(","), Doc::text(""))),
            false => inner.push(Doc::text(",")),
        }
        inner.extend(item.after_comma);
        if index + 1 < count {
            inner.push(Doc::Line);
        }
    }
    for clause in clauses {
        if inner.len() > 1 {
            inner.push(Doc::Line);
        }
        inner.push(clause);
    }
    if let Some(tail) = tail {
        if inner.len() > 1 {
            inner.push(Doc::HardLine);
        }
        inner.push(tail);
    }
    Doc::group(Doc::concat([
        open,
        Doc::nest(INDENT, Doc::Concat(inner)),
        Doc::SoftLine,
        close,
    ]))
}

/// `text`, an expression, on one line: its spaces collapsed outside its literals, without spaces
/// inside its brackets or a comma before a closing bracket. Text with a comment stays as written.
pub fn one_line(text: &str) -> String {
    let (tokens, _) = tokenize(text);
    if (tokens.iter()).any(|token| token.kind == SyntaxKind::COMMENT) {
        return text.to_string();
    }
    let solid: Vec<(SyntaxKind, &str, bool)> = {
        let mut solid = Vec::new();
        let mut spaced = false;
        for token in &tokens {
            match token.kind {
                SyntaxKind::WHITESPACE => spaced = true,
                kind => {
                    solid.push((kind, &text[token.range], spaced));
                    spaced = false;
                }
            }
        }
        solid
    };
    let closing = |kind: SyntaxKind| matches!(kind, SyntaxKind::R_PAREN | SyntaxKind::R_BRACK);
    let mut out = String::new();
    let mut previous: Option<SyntaxKind> = None;
    for (index, (kind, token, spaced)) in solid.iter().enumerate() {
        let next = solid.get(index + 1).map(|(kind, _, _)| *kind);
        if *kind == SyntaxKind::COMMA && next.is_some_and(closing) {
            continue;
        }
        let opened = matches!(previous, Some(SyntaxKind::L_PAREN | SyntaxKind::L_BRACK));
        if *spaced && !out.is_empty() && !opened && !closing(*kind) && *kind != SyntaxKind::COMMA {
            out.push(' ');
        }
        out.push_str(token);
        previous = Some(*kind);
    }
    out
}
