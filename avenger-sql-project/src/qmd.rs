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
//!
//! A cell's leading `#|` lines are its options, a YAML block, as Quarto reads them: Quarto takes
//! the prefix of a cell's options from the comment syntax of its language, and a language that its
//! table doesn't name, as `avenger-sql`, takes `#`, which avenger-sql reads as a comment. Avenger reads the options that change what a cell runs or
//! how its chart draws, [`CellOptions`], and leaves the others to Quarto. A value of the wrong
//! type is an error of the cell's options, at the value.

use std::collections::BTreeSet;
use std::ops::Range;

use avenger_sql_language::{Diagnostic, FileRole, Project, Severity, Source};
use avenger_sql_syntax::{Offset, TextRange};
use saphyr::{LoadableYamlNode, MarkedYaml, Marker, Scalar, YamlData};

use crate::manifest::DOCUMENT_SUFFIX;

/// The class of the fences of avenger cells.
const CELL_CLASS: &str = "{avenger-sql";

/// The prefix of the lines of a cell's options.
const OPTION_PREFIX: &str = "#|";

/// A Quarto document: its text and its cells.
#[derive(Clone, Debug, Default)]
pub struct Qmd {
    pub text: String,
    pub cells: Vec<Cell>,
}

/// A cell of a document: its text, and where its lines are in the document.
#[derive(Clone, Debug, PartialEq)]
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
    /// The options of the cell's leading `#|` lines that avenger reads.
    pub options: CellOptions,
    /// The errors of the cell's options, at their ranges in the cell's text.
    pub option_errors: Vec<OptionError>,
    lines: Vec<Line>,
}

/// The options of a cell that avenger reads, from its leading `#|` lines. An option that a cell
/// doesn't set, or sets to a value of the wrong type, is `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CellOptions {
    /// `label`, which Quarto's cross-references name the cell by, such as `fig-weather`.
    pub label: Option<String>,
    /// `eval`: `false` keeps the cell from running.
    pub eval: Option<bool>,
    /// `fig-width`, the width of the cell's chart in inches.
    pub fig_width: Option<f64>,
    /// `fig-height`, the height of the cell's chart in inches.
    pub fig_height: Option<f64>,
    /// `fig-format`, the format of the cell's image.
    pub fig_format: Option<FigFormat>,
    /// `fig-dpi`, the dots per inch of a PNG image.
    pub fig_dpi: Option<f64>,
}

/// The format of a cell's image, as its `fig-format` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FigFormat {
    Svg,
    Png,
    /// PNG at twice the density.
    Retina,
}

/// An error of a cell's options: its range in the cell's text, and its message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OptionError {
    pub range: Range<usize>,
    pub message: String,
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
    /// The cell that `name` names and its index, from 0: its number, from 1, or its label.
    pub fn cell(&self, name: &str) -> Option<(usize, &Cell)> {
        if let Ok(number) = name.parse::<usize>() {
            let index = number.checked_sub(1)?;
            return Some((index, self.cells.get(index)?));
        }
        (self.cells.iter().enumerate())
            .find(|(_, cell)| cell.options.label.as_deref() == Some(name))
    }

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
    /// The errors of the cell's options, as diagnostics of its text, with the source
    /// [`Source::Options`].
    pub fn option_diagnostics(&self) -> Vec<Diagnostic> {
        (self.option_errors.iter())
            .map(|error| Diagnostic {
                range: TextRange::new(
                    Offset::from(error.range.start as u32),
                    Offset::from(error.range.end as u32),
                ),
                message: error.message.clone(),
                help: None,
                severity: Severity::Error,
                source: Source::Options,
                code: None,
                fixes: Vec::new(),
            })
            .collect()
    }

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
    let (options, option_errors) = options(&cell_text);
    Cell {
        options,
        option_errors,
        text: cell_text,
        indent: indent.to_string(),
        range,
        closed,
        lines,
    }
}

/// The options of the cell's text `text`, from its leading `#|` lines, and their errors.
fn options(text: &str) -> (CellOptions, Vec<OptionError>) {
    let block = OptionBlock::of(text);
    let mut options = CellOptions::default();
    let mut errors = Vec::new();
    if block.yaml.is_empty() {
        return (options, errors);
    }
    let documents = match MarkedYaml::load_from_str(&block.yaml) {
        Ok(documents) => documents,
        Err(error) => {
            let at = block.to_cell(*error.marker());
            errors.push(OptionError {
                range: at..at,
                message: format!("the cell's options are not YAML: {}", error.info()),
            });
            return (options, errors);
        }
    };
    let Some(document) = documents.first() else {
        return (options, errors);
    };
    let entries = match &document.data {
        YamlData::Mapping(entries) => entries,
        YamlData::Value(Scalar::Null) => return (options, errors),
        _ => {
            errors.push(OptionError {
                range: block.range(document),
                message: "the cell's options are a YAML mapping of names to values, as \
                          `#| label: fig-cars`"
                    .to_string(),
            });
            return (options, errors);
        }
    };
    for (key, value) in entries {
        let Some(name) = key.data.as_str() else {
            continue;
        };
        let mut error = |message: String| {
            errors.push(OptionError {
                range: block.range(value),
                message,
            })
        };
        match name {
            "label" => match &value.data {
                YamlData::Value(Scalar::String(label)) => options.label = Some(label.to_string()),
                YamlData::Value(Scalar::Integer(label)) => options.label = Some(label.to_string()),
                YamlData::Value(Scalar::Null) => {}
                _ => error("`label` is a name, as `label: fig-cars`".to_string()),
            },
            "eval" => match &value.data {
                YamlData::Value(Scalar::Boolean(eval)) => options.eval = Some(*eval),
                _ => error("`eval` is `true` or `false`".to_string()),
            },
            "fig-width" | "fig-height" | "fig-dpi" => {
                let unit = match name {
                    "fig-dpi" => "dots per inch, as `fig-dpi: 192`",
                    "fig-width" => "inches, as `fig-width: 6`",
                    _ => "inches, as `fig-height: 4`",
                };
                let number = match &value.data {
                    YamlData::Value(Scalar::Integer(number)) => Some(*number as f64),
                    YamlData::Value(Scalar::FloatingPoint(number)) => Some(number.into_inner()),
                    _ => None,
                };
                match number {
                    Some(number) if number > 0.0 && number.is_finite() => match name {
                        "fig-width" => options.fig_width = Some(number),
                        "fig-height" => options.fig_height = Some(number),
                        _ => options.fig_dpi = Some(number),
                    },
                    _ => error(format!("`{name}` is a number of {unit}, more than 0")),
                }
            }
            "fig-format" => {
                let format = match &value.data {
                    YamlData::Value(Scalar::String(format)) => match format.as_ref() {
                        "svg" => Some(FigFormat::Svg),
                        "png" => Some(FigFormat::Png),
                        "retina" => Some(FigFormat::Retina),
                        _ => None,
                    },
                    _ => None,
                };
                match format {
                    Some(format) => options.fig_format = Some(format),
                    None => error("the avenger engine draws `svg`, `png`, or `retina`".to_string()),
                }
            }
            _ => {}
        }
    }
    (options, errors)
}

/// The leading `#|` lines of a cell's text as a YAML block: each line without its indent, its
/// `#|`, and the one space after it, and where each line of the block starts in the cell's text.
struct OptionBlock {
    yaml: String,
    /// The start of each line of the block in the block, and in the cell's text.
    lines: Vec<(usize, usize)>,
}

impl OptionBlock {
    fn of(text: &str) -> OptionBlock {
        let mut yaml = String::new();
        let mut lines = Vec::new();
        let mut at = 0;
        for line in text.split_inclusive('\n') {
            let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
            let Some(rest) = line[indent..].strip_prefix(OPTION_PREFIX) else {
                break;
            };
            let skip = indent + OPTION_PREFIX.len() + usize::from(rest.starts_with(' '));
            lines.push((yaml.len(), at + skip));
            yaml.push_str(&line[skip..]);
            if !yaml.ends_with('\n') {
                yaml.push('\n');
            }
            at += line.len();
        }
        OptionBlock { yaml, lines }
    }

    /// The offset in the cell's text of `marker`, a line from 1 and a column in characters of
    /// the block.
    fn to_cell(&self, marker: Marker) -> usize {
        let index = marker.line().saturating_sub(1).min(self.lines.len() - 1);
        let (start, cell) = self.lines[index];
        let end = (self.lines.get(index + 1)).map_or(self.yaml.len(), |(next, _)| *next);
        let line = &self.yaml[start..end];
        let column = (line.char_indices().nth(marker.col())).map_or(line.len(), |(at, _)| at);
        cell + column.min(line.trim_end_matches('\n').len())
    }

    /// The range in the cell's text of the node `node`.
    fn range(&self, node: &MarkedYaml) -> Range<usize> {
        let start = self.to_cell(node.span.start);
        let end = self.to_cell(node.span.end).max(start);
        start..end
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
