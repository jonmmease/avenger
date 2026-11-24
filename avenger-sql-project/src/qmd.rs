//! The `{avenger-sql}` cells of Quarto documents (`.qmd`). A document's cells, in order, form one
//! chart script of its project, its [`DocumentScript`], a file of the language project at the
//! document's path, so a cell sees what the cells before it define: temporary tables and
//! functions, imports, variables, and tables of files. A cell's value is its last query, and a
//! cell without a query has none. A cell can start with `DECLARE`s, as a block of GoogleSQL can.
//! A document outside every project is a project of its own.
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
use std::sync::Arc;

use avenger_sql_language::{Diagnostic, FileRole, Project, Severity, Source};
use avenger_sql_syntax::{Offset, TextRange};
use saphyr::{LoadableYamlNode, MarkedYaml, Marker, Scalar, YamlData};

use crate::manifest::DOCUMENT_SUFFIX;

/// The class of the fences of avenger cells.
const CELL_CLASS: &str = "{avenger-sql";

/// The prefix of the lines of a cell's options.
const OPTION_PREFIX: &str = "#|";

/// A Quarto document: its text, its cells, and the script of its cells.
#[derive(Clone, Debug, Default)]
pub struct Qmd {
    pub text: String,
    pub cells: Vec<Cell>,
    pub script: DocumentScript,
}

/// The script of a document: its cells, in order, as one chart script, with a line for each line
/// of the document, so line `L` of the script is line `L` of the document. A line of a cell is the
/// cell's line without the fence's indent. The line of a closing fence holds a `;` when the cell's
/// last statement has none, so that no statement spans two cells. Every other line is empty: of
/// prose, of other blocks, of opening fences, and of the cells that `eval: false` keeps from
/// running, which are not part of the script.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentScript {
    pub text: String,
    /// The range of each cell in the script, from the start of its first line to the end of the
    /// line of its closing fence, by the cell's index, or `None` for a cell that is not part of
    /// the script.
    pub cells: Vec<Option<Range<usize>>>,
    lines: Vec<ScriptLine>,
}

/// A line of a document's script: its start in the script, the start of its line in the
/// document, and for a line of a cell, the start of its text in the document and its length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ScriptLine {
    script: usize,
    document: usize,
    text: Option<(usize, usize)>,
}

impl DocumentScript {
    fn new(text: &str, cells: &[Cell]) -> DocumentScript {
        let starts: Vec<usize> = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(at, _)| at + 1))
            .collect();
        let line_of = |offset: usize| starts.partition_point(|start| *start <= offset) - 1;
        // Each line of the document: the start and the length of its text in a cell, and the
        // line of the script.
        let mut lines: Vec<(Option<(usize, usize)>, String)> =
            vec![(None, String::new()); starts.len()];
        let mut ranges = Vec::with_capacity(cells.len());
        for cell in cells {
            if cell.options.eval == Some(false) || cell.lines.is_empty() {
                ranges.push(None);
                continue;
            }
            for (index, line) in cell.lines.iter().enumerate() {
                let end = (cell.lines.get(index + 1)).map_or(cell.text.len(), |next| next.cell);
                let content = cell.text[line.cell..end].trim_end_matches(['\n', '\r']);
                lines[line_of(line.document)] =
                    (Some((line.document, content.len())), content.to_string());
            }
            let last = match cell.closed {
                true => line_of(cell.range.end),
                false => starts.len() - 1,
            };
            if needs_semicolon(&cell.text) {
                lines[last].1.push(';');
            }
            ranges.push(Some((line_of(cell.lines[0].document), last)));
        }
        let mut script = String::with_capacity(text.len());
        let mut map = Vec::with_capacity(starts.len());
        for (index, (cell_text, content)) in lines.into_iter().enumerate() {
            map.push(ScriptLine {
                script: script.len(),
                document: starts[index],
                text: cell_text,
            });
            script.push_str(&content);
            if index + 1 < starts.len() {
                script.push('\n');
            }
        }
        let cells = (ranges.into_iter())
            .map(|range| {
                let (first, last) = range?;
                let end = (map.get(last + 1)).map_or(script.len(), |line| line.script);
                Some(map[first].script..end)
            })
            .collect();
        DocumentScript {
            text: script,
            cells,
            lines: map,
        }
    }

    /// The starts of the cells in the script, in order, for the cells that are part of it.
    pub fn cell_starts(&self) -> Arc<[Offset]> {
        (self.cells.iter().flatten())
            .map(|range| Offset::from(range.start as u32))
            .collect()
    }

    /// The index of the cell whose range in the script holds the offset `offset`.
    pub fn cell_at(&self, offset: usize) -> Option<usize> {
        (self.cells.iter()).position(|range| {
            range
                .as_ref()
                .is_some_and(|range| range.start <= offset && offset <= range.end)
        })
    }

    /// The offset in the document of the offset `offset` of the script. An offset in a line that
    /// holds no text of a cell, such as a `;` that the script adds, is the start of its line, or
    /// the end of the cell's text on its line.
    pub fn to_document(&self, offset: usize) -> usize {
        let index = self.lines.partition_point(|line| line.script <= offset) - 1;
        let line = self.lines[index];
        match line.text {
            Some((start, len)) => start + (offset - line.script).min(len),
            None => line.document,
        }
    }

    /// Whether the offset `offset` of the script is in the text of a cell, and not in a `;` that
    /// the script adds or in a line of no cell.
    pub fn holds(&self, offset: usize) -> bool {
        let index = self.lines.partition_point(|line| line.script <= offset) - 1;
        let line = self.lines[index];
        line.text.is_some_and(|(_, len)| offset - line.script < len)
    }

    /// The offset in the script of the offset `offset` of the document, or `None` outside the
    /// text of a cell of the script. An offset in the indent of a line is the start of the line.
    pub fn to_script(&self, offset: usize) -> Option<usize> {
        let index = self.lines.partition_point(|line| line.document <= offset) - 1;
        let line = self.lines[index];
        let (start, len) = line.text?;
        Some(line.script + offset.saturating_sub(start).min(len))
    }
}

/// Whether the cell's text `text` ends with a statement that no `;` closes: its last token that
/// is not a space or a comment is not a `;`.
fn needs_semicolon(text: &str) -> bool {
    let (tokens, _) = avenger_sql_syntax::lexer::tokenize(text);
    (tokens.iter().rev())
        .find(|token| !token.kind.is_trivia())
        .is_some_and(|token| token.kind != avenger_sql_syntax::SyntaxKind::SEMICOLON)
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
        let script = DocumentScript::new(&text, &cells);
        Qmd {
            text,
            cells,
            script,
        }
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

/// Sets the script of the document at `path` in `project` to the script of `new`, or removes it
/// without one, and returns the files that the change reaches.
pub(crate) fn set_script(project: &mut Project, path: &str, new: Option<&Qmd>) -> BTreeSet<String> {
    match new {
        Some(new) => project.add(path, role(new), new.script.text.clone()),
        None => project.remove(path),
    }
}

/// The role of the script of the document `document` in a language project.
pub(crate) fn role(document: &Qmd) -> FileRole {
    FileRole::Cells {
        cells: document.script.cell_starts(),
    }
}
