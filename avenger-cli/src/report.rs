//! The results of the commands, as text for people or as JSON for programs: diagnostics as
//! `path:line:column: severity: message` with the line of the text, and the files that
//! formatting changes.

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use avenger_sql_syntax::{line_column, Offset};
use serde_json::json;

/// The version of the JSON output, which changes when its shape does.
const JSON_VERSION: u32 = 1;

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    fn name(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// A place in a text: its line and its column, each from 1, the column in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: usize,
    pub column: usize,
}

/// The range of a diagnostic in its file, with the line of its start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: Position,
    pub end: Position,
    /// The bytes of the range in the file.
    pub bytes: (usize, usize),
    /// The text of the line of the start, which the text output shows.
    pub line: String,
}

/// An error or a warning of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    /// The file on disk.
    pub path: PathBuf,
    /// Where in the file, or `None` for an error of the whole file.
    pub range: Option<Range>,
    pub severity: Severity,
    pub message: String,
    pub help: Option<String>,
    /// What found it: `syntax`, `analysis`, `project`, `loader`, `style`, or `run`.
    pub source: &'static str,
}

impl Report {
    /// A report at the bytes `start..end` of `text`, the text of the file at `path`.
    pub fn at(
        path: &Path,
        text: &str,
        (start, end): (usize, usize),
        severity: Severity,
        message: String,
        source: &'static str,
    ) -> Report {
        let position = |offset: usize| {
            let (line, column) = line_column(text, Offset::from(offset.min(text.len()) as u32));
            Position { line, column }
        };
        let line_start = text[..start.min(text.len())]
            .rfind('\n')
            .map_or(0, |at| at + 1);
        let line_end = text[line_start..]
            .find('\n')
            .map_or(text.len(), |at| line_start + at);
        Report {
            path: path.to_path_buf(),
            range: Some(Range {
                start: position(start),
                end: position(end),
                bytes: (start, end),
                line: text[line_start..line_end]
                    .trim_end_matches('\r')
                    .to_string(),
            }),
            severity,
            message,
            help: None,
            source,
        }
    }

    /// A report of the whole file at `path`.
    pub fn file(path: &Path, severity: Severity, message: String, source: &'static str) -> Report {
        Report {
            path: path.to_path_buf(),
            range: None,
            severity,
            message,
            help: None,
            source,
        }
    }

    pub fn with_help(mut self, help: Option<String>) -> Report {
        self.help = help;
        self
    }
}

/// Where the results go, and how.
pub struct Output {
    json: bool,
    color: bool,
    current: PathBuf,
}

impl Output {
    /// The output to standard output: JSON when `json`, and otherwise text, with colors on a
    /// terminal unless `NO_COLOR` is set.
    pub fn new(json: bool) -> Output {
        let color = !json
            && std::io::stdout().is_terminal()
            && std::env::var_os("NO_COLOR").is_none_or(|value| value.is_empty());
        let current = std::env::current_dir()
            .and_then(std::fs::canonicalize)
            .unwrap_or_default();
        Output {
            json,
            color,
            current,
        }
    }

    /// The path `path` as the output writes it: relative to the current directory when it is
    /// under it, and otherwise as it is.
    fn display(&self, path: &Path) -> String {
        let path = path.strip_prefix(&self.current).unwrap_or(path);
        path.to_string_lossy().replace('\\', "/")
    }

    fn paint(&self, text: &str, code: &str) -> String {
        match self.color {
            true => format!("\x1b[{code}m{text}\x1b[0m"),
            false => text.to_string(),
        }
    }

    /// Prints `reports`, in the order of their paths and their positions, and as text, a count
    /// of the errors and the warnings on standard error.
    pub fn diagnostics(&self, mut reports: Vec<Report>) {
        reports.sort_by(|a, b| {
            (
                self.display(&a.path),
                a.range.as_ref().map(|r| r.start),
                a.severity,
            )
                .cmp(&(
                    self.display(&b.path),
                    b.range.as_ref().map(|r| r.start),
                    b.severity,
                ))
        });
        if self.json {
            let diagnostics: Vec<serde_json::Value> = (reports.iter())
                .map(|report| {
                    let range = report.range.as_ref().map(|range| {
                        let position = |p: Position| json!({"line": p.line, "column": p.column});
                        json!({
                            "start": position(range.start),
                            "end": position(range.end),
                            "bytes": [range.bytes.0, range.bytes.1],
                        })
                    });
                    json!({
                        "path": self.display(&report.path),
                        "range": range,
                        "severity": report.severity.name(),
                        "message": report.message,
                        "help": report.help,
                        "source": report.source,
                    })
                })
                .collect();
            let value = json!({"version": JSON_VERSION, "diagnostics": diagnostics});
            println!("{value}");
            return;
        }
        for report in &reports {
            let severity = match report.severity {
                Severity::Error => self.paint("error", "1;31"),
                Severity::Warning => self.paint("warning", "1;33"),
            };
            let place = match &report.range {
                Some(range) => format!(
                    "{}:{}:{}",
                    self.display(&report.path),
                    range.start.line,
                    range.start.column
                ),
                None => self.display(&report.path),
            };
            println!(
                "{}: {severity}: {}",
                self.paint(&place, "1"),
                report.message
            );
            if let Some(range) = &report.range {
                let number = range.start.line.to_string();
                let gutter = " ".repeat(number.len());
                let width = match range.end.line == range.start.line {
                    true => range.end.column.saturating_sub(range.start.column).max(1),
                    false => 1,
                };
                let marker = format!(
                    "{}{}",
                    " ".repeat(range.start.column - 1),
                    "^".repeat(width)
                );
                println!(" {number} | {}", range.line);
                println!(" {gutter} | {}", self.paint(&marker, "1;34"));
            }
            if let Some(help) = &report.help {
                println!("  = help: {help}");
            }
        }
        let count = |severity| reports.iter().filter(|r| r.severity == severity).count();
        let (errors, warnings) = (count(Severity::Error), count(Severity::Warning));
        if errors + warnings > 0 {
            let plural = |n: usize, word: &str| match n {
                1 => format!("1 {word}"),
                n => format!("{n} {word}s"),
            };
            eprintln!(
                "{}, {}",
                plural(errors, "error"),
                plural(warnings, "warning")
            );
        }
    }

    /// Prints the images `paths` that a render wrote.
    pub fn written(&self, paths: &[PathBuf]) {
        let paths: Vec<String> = paths.iter().map(|path| self.display(path)).collect();
        match self.json {
            true => println!("{}", json!({"version": JSON_VERSION, "written": paths})),
            false => paths.iter().for_each(|path| println!("{path}")),
        }
    }

    /// Prints the files `paths` that formatting changed, or with `check`, would change.
    pub fn formatted(&self, paths: &[PathBuf], check: bool) {
        let paths: Vec<String> = paths.iter().map(|path| self.display(path)).collect();
        if self.json {
            let key = match check {
                true => "unformatted",
                false => "formatted",
            };
            println!("{}", json!({"version": JSON_VERSION, key: paths}));
            return;
        }
        for path in &paths {
            println!("{path}");
        }
        if check && !paths.is_empty() {
            let files = match paths.len() {
                1 => "1 file needs".to_string(),
                n => format!("{n} files need"),
            };
            eprintln!("{files} formatting: run `avenger fmt`");
        }
    }
}
