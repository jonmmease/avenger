//! The `{avenger-sql}` cells of Quarto documents (`.qmd`). Each cell is a chart script of the
//! document's project, a file of the language project at the document's path and the cell's
//! number, such as `reports/overview.qmd#3`, and a cell of a document outside every project is
//! a loose script.
//!
//! A line scanner finds the fenced code blocks of a document, as Pandoc does: a fence of three
//! or more backticks or tildes, which a line of the same character, at least as long, closes.
//! A block whose attributes are `{avenger-sql}`, or start with `{avenger-sql` and a space or a
//! comma before their closing brace, as Quarto reads the cells of a language, is a cell. Other
//! blocks, such as `{.avenger-sql}` blocks and the cells of other languages, are not, nor are the
//! fences inside them or inside an HTML comment that starts a line. A fence that is indented, as
//! in a list, drops its indent from each line of its cell, so the offsets of a cell map to the
//! document's line by line. A tab indents to the next multiple of four columns.
//!
//! The scanner reads no containers: it takes a fence at any indent, so a fence indented four or
//! more columns outside a list, which Pandoc reads as an indented code block, is a fence, and a
//! fence in a block quote is not one.

use std::collections::BTreeSet;
use std::ops::Range;

use avenger_sql_language::{FileRole, Project};

use crate::manifest::DOCUMENT_SUFFIX;

/// The class of the fences of avenger cells.
const CELL_CLASS: &str = "{avenger-sql";

/// A Quarto document: its text and its cells.
#[derive(Clone, Debug, Default)]
pub struct Qmd {
    pub text: String,
    pub cells: Vec<Cell>,
}

/// A cell of a document: its text, and where its lines are in the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    /// The text between the fences, without the fence's indent.
    pub text: String,
    /// The spaces and tabs before the opening fence, as many columns of which each line of the
    /// cell drops.
    pub indent: String,
    /// The bytes of the document between the fences: from the start of the cell's first line to
    /// the start of the closing fence, or to the end of the document without one.
    pub range: Range<usize>,
    /// Whether a fence closes the cell.
    pub closed: bool,
    lines: Vec<Line>,
}

/// A line of a cell: its start in the cell, its start in the document, and the bytes of the
/// fence's indent that it drops, before its start in the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    cell: usize,
    document: usize,
    dropped: usize,
}

impl Qmd {
    /// The document with `text`, and its cells.
    pub fn parse(text: impl Into<String>) -> Qmd {
        let text = text.into();
        let cells = cells(&text);
        Qmd { text, cells }
    }

    /// The text of the document with the text of each cell formatted by `format`, and the rest
    /// of the document as it is.
    pub fn format(&self, format: impl Fn(&str) -> String) -> String {
        let mut formatted = String::with_capacity(self.text.len());
        let mut at = 0;
        for cell in &self.cells {
            formatted.push_str(&self.text[at..cell.range.start]);
            let text = format(&cell.text);
            match text == cell.text {
                true => formatted.push_str(&self.text[cell.range.clone()]),
                false => formatted.push_str(&cell.indented(&text)),
            }
            at = cell.range.end;
        }
        formatted.push_str(&self.text[at..]);
        formatted
    }
}

impl Cell {
    /// The offset in the document of the offset `offset` of the cell's text.
    pub fn to_document(&self, offset: usize) -> usize {
        let at = self.lines.partition_point(|line| line.cell <= offset);
        match at.checked_sub(1).map(|at| self.lines[at]) {
            Some(line) => line.document + (offset - line.cell),
            None => self.range.start,
        }
    }

    /// The offset in the cell's text of the offset `offset` of the document, or `None` outside
    /// the cell. An offset in the indent of a line is the start of the line.
    pub fn to_cell(&self, offset: usize) -> Option<usize> {
        let inside = match self.closed {
            true => self.range.contains(&offset),
            false => self.range.start <= offset && offset <= self.range.end,
        };
        if !inside {
            return None;
        }
        let at = (self.lines).partition_point(|line| line.document - line.dropped <= offset);
        let Some(index) = at.checked_sub(1) else {
            return Some(0);
        };
        let line = self.lines[index];
        let end = self
            .lines
            .get(index + 1)
            .map_or(self.text.len(), |next| next.cell);
        Some((line.cell + offset.saturating_sub(line.document)).min(end))
    }

    /// The text `text` of the cell with the fence's indent before each line that is not empty,
    /// and a line break at its end when a fence closes the cell.
    pub fn indented(&self, text: &str) -> String {
        let indent = &self.indent;
        let mut indented = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            if !line.trim_end_matches(['\n', '\r']).is_empty() {
                indented.push_str(indent);
            }
            indented.push_str(line);
        }
        if self.closed && !indented.is_empty() && !indented.ends_with('\n') {
            indented.push('\n');
        }
        indented
    }
}

/// The cells of the document `text`.
fn cells(text: &str) -> Vec<Cell> {
    /// What the line before ended in.
    enum State<'a> {
        Text,
        /// An HTML comment that a line started, until a line that holds `-->`.
        Comment,
        /// A fenced block, of its character and its length, after its opening line, whose
        /// indent `indent` its lines drop, and whether it is a cell.
        Fenced {
            fence: char,
            length: usize,
            indent: &'a str,
            start: usize,
            cell: bool,
        },
    }
    let mut cells = Vec::new();
    let mut state = State::Text;
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        let end = start + line.len();
        let content = line.trim_end_matches(['\n', '\r']);
        let rest = content.trim_start_matches([' ', '\t']);
        let indent = &content[..content.len() - rest.len()];
        let fence = rest.chars().next().filter(|c| *c == '`' || *c == '~');
        let length = fence.map_or(0, |fence| rest.len() - rest.trim_start_matches(fence).len());
        state = match state {
            State::Text if rest.starts_with("<!--") => match rest[4..].contains("-->") {
                true => State::Text,
                false => State::Comment,
            },
            State::Text => match fence {
                Some(fence) if length >= 3 => {
                    let info = rest[length..].trim();
                    match fence == '`' && info.contains('`') {
                        // Not a fence, but backticks of inline code.
                        true => State::Text,
                        false => State::Fenced {
                            fence,
                            length,
                            indent,
                            start: end,
                            cell: is_cell(info),
                        },
                    }
                }
                _ => State::Text,
            },
            State::Comment => match content.contains("-->") {
                true => State::Text,
                false => State::Comment,
            },
            State::Fenced {
                fence: open,
                length: opened,
                indent: fence_indent,
                start: block,
                cell: is_cell,
            } => {
                let closes =
                    fence == Some(open) && length >= opened && rest[length..].trim().is_empty();
                match closes {
                    true => {
                        if is_cell {
                            cells.push(cell(text, block..start, fence_indent, true));
                        }
                        State::Text
                    }
                    false => State::Fenced {
                        fence: open,
                        length: opened,
                        indent: fence_indent,
                        start: block,
                        cell: is_cell,
                    },
                }
            }
        };
        start = end;
    }
    if let State::Fenced {
        indent,
        start,
        cell: true,
        ..
    } = state
    {
        cells.push(cell(text, start..text.len(), indent, false));
    }
    cells
}

/// Whether the attributes `info` of a fence are those of a cell: `{avenger-sql}`, or
/// `{avenger-sql` and a space or a comma before more attributes and the closing brace.
fn is_cell(info: &str) -> bool {
    let Some(after) = info.strip_prefix(CELL_CLASS) else {
        return false;
    };
    after == "}" || (after.ends_with('}') && after.starts_with([' ', '\t', ',']))
}

/// The columns of the spaces and tabs `whitespace`, where a tab indents to the next multiple of
/// four columns.
fn columns(whitespace: &str) -> usize {
    whitespace.chars().fold(0, |columns, c| match c {
        '\t' => columns + 4 - columns % 4,
        _ => columns + 1,
    })
}

/// The cell of the lines `range` of `text`, whose fence's indent is `indent`: each line drops its
/// spaces and tabs up to as many columns.
fn cell(text: &str, range: Range<usize>, indent: &str, closed: bool) -> Cell {
    let width = columns(indent);
    let mut cell_text = String::with_capacity(range.len());
    let mut lines = Vec::new();
    let mut start = range.start;
    for line in text[range.clone()].split_inclusive('\n') {
        let mut dropped = 0;
        for (at, c) in line.char_indices() {
            if !matches!(c, ' ' | '\t') || columns(&line[..at + 1]) > width {
                break;
            }
            dropped = at + 1;
        }
        lines.push(Line {
            cell: cell_text.len(),
            document: start + dropped,
            dropped,
        });
        cell_text.push_str(&line[dropped..]);
        start += line.len();
    }
    Cell {
        text: cell_text,
        indent: indent.to_string(),
        range,
        closed,
        lines,
    }
}

/// Whether the file at `path` is named as a Quarto document.
pub fn is_document(path: &str) -> bool {
    path.ends_with(DOCUMENT_SUFFIX)
}

/// The path in a language project of the cell numbered `number`, from 1, of the document at
/// `document`.
pub fn cell_path(document: &str, number: usize) -> String {
    format!("{document}#{number}")
}

/// The document's path and the index, from 0, of the cell at the path `path` of a language
/// project, or `None` for another file.
pub fn split_cell_path(path: &str) -> Option<(&str, usize)> {
    let (document, number) = path.rsplit_once('#')?;
    let number: usize = number.parse().ok()?;
    match is_document(document) && number > 0 {
        true => Some((document, number - 1)),
        false => None,
    }
}

/// Sets the cells of the document at `path` in `project` to the cells of `new`, after the cells
/// of `old`, as chart scripts, and returns the files that the change reaches.
pub(crate) fn set_cells(
    project: &mut Project,
    path: &str,
    old: &[Cell],
    new: &[Cell],
) -> BTreeSet<String> {
    let mut reached = BTreeSet::new();
    for number in new.len() + 1..=old.len() {
        reached.extend(project.remove(&cell_path(path, number)));
    }
    let mut texts = Vec::new();
    for (index, cell) in new.iter().enumerate() {
        let path = cell_path(path, index + 1);
        match old.get(index) {
            Some(old) if old.text == cell.text => {}
            Some(_) => texts.push((path, cell.text.clone())),
            None => reached.extend(project.add(path, FileRole::Chart, cell.text.clone())),
        }
    }
    reached.extend(project.set_texts(texts));
    reached
}
