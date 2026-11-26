//! A project of avenger-sql in a page: its files in memory, the data scripts loaded into a session
//! of DataFusion, the charts of its chart scripts, and the features of an editor at a position of
//! a file. Positions and ranges count UTF-16 code units, as the strings of JavaScript do.

use std::cell::RefCell;

use avenger_chart::results::is_chart_field;
use avenger_sql_language::{
    CompletionKind, Diagnostic as LanguageDiagnostic, FileRole, Offset, Severity,
    Symbol as Outline, TextRange,
};
use avenger_sql_project::{Analysis, FileSource, Loader, ProjectFiles};
use datafusion::{
    arrow::array::{Array, AsArray, RecordBatch},
    arrow::datatypes::DataType,
    prelude::{SessionConfig, SessionContext},
};
use serde::Serialize;

/// A project of avenger-sql whose files a page holds in memory.
pub struct SqlProject {
    loader: Loader,
    analysis: RefCell<Analysis>,
    charts: Vec<String>,
}

/// A range of a file's text, in UTF-16 code units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Range {
    pub start: u32,
    pub end: u32,
}

/// An error or a warning of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub range: Range,
    /// `error` or `warning`.
    pub severity: &'static str,
    pub message: String,
    pub help: Option<String>,
    /// What found it: `syntax`, `analysis`, `project`, or `typst`, as avenger-sql-language's
    /// `Source` names it, in lower case.
    pub source: String,
}

/// The completions at a position, which replace `range`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Completions {
    pub range: Range,
    pub items: Vec<CompletionItem>,
}

/// A completion.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CompletionItem {
    pub label: String,
    /// The kind, as avenger-sql-language's `CompletionKind` names it, such as `Column` or
    /// `Function(Aggregate)`.
    pub kind: String,
    pub detail: Option<String>,
    /// Markdown.
    pub documentation: Option<String>,
    /// The text that replaces the range.
    pub insert: String,
    /// The text that an editor matches what the reader typed against, when it is not the label.
    pub filter: Option<String>,
}

/// The hover at a position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Hover {
    pub range: Range,
    pub markdown: String,
}

/// The forms of the call at a position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SignatureHelp {
    pub signatures: Vec<Signature>,
    pub active_signature: usize,
}

/// A form of a function.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Signature {
    pub label: String,
    pub documentation: Option<String>,
    /// The range of each parameter in `label`, in UTF-16 code units.
    pub parameters: Vec<Range>,
    pub active_parameter: Option<usize>,
}

/// An item of a file's outline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub name: String,
    /// The kind, as avenger-sql-language's `SymbolKind` names it, such as `Query`.
    pub kind: String,
    pub detail: Option<String>,
    pub range: Range,
    pub name_range: Range,
    pub children: Vec<Symbol>,
}

/// The byte offset of the UTF-16 position `position` of `text`, or the text's end past it.
fn offset(text: &str, position: u32) -> Offset {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= position {
            return Offset::from(index as u32);
        }
        units += character.len_utf16() as u32;
    }
    Offset::from(text.len() as u32)
}

/// The UTF-16 position of the byte offset `offset` of `text`.
fn position(text: &str, offset: usize) -> u32 {
    let offset = offset.min(text.len());
    text[..offset].encode_utf16().count() as u32
}

fn range(text: &str, range: TextRange) -> Range {
    Range {
        start: position(text, range.start().into()),
        end: position(text, range.end().into()),
    }
}

fn kind(kind: CompletionKind) -> String {
    format!("{kind:?}")
}

fn symbol(text: &str, symbol: Outline) -> Symbol {
    Symbol {
        name: symbol.name,
        kind: format!("{:?}", symbol.kind),
        detail: symbol.detail,
        range: range(text, symbol.range),
        name_range: range(text, symbol.name_range),
        children: (symbol.children.into_iter())
            .map(|child| self::symbol(text, child))
            .collect(),
    }
}

fn diagnostic(text: &str, diagnostic: LanguageDiagnostic) -> Diagnostic {
    Diagnostic {
        range: range(text, diagnostic.range),
        severity: match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        },
        message: diagnostic.message,
        help: diagnostic.help,
        source: format!("{:?}", diagnostic.source).to_lowercase(),
    }
}

/// The bytes of each chart of the column of charts of `batches`, by row.
fn charts(batches: &[RecordBatch]) -> Vec<Vec<u8>> {
    let mut charts = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        let Some(column) = (schema.fields().iter()).position(|field| is_chart_field(field)) else {
            continue;
        };
        let column = batch.column(column);
        for row in 0..column.len() {
            if column.is_null(row) {
                continue;
            }
            charts.push(match column.data_type() {
                DataType::BinaryView => column.as_binary_view().value(row).to_vec(),
                DataType::LargeBinary => column.as_binary::<i64>().value(row).to_vec(),
                _ => column.as_binary::<i32>().value(row).to_vec(),
            });
        }
    }
    charts
}

impl SqlProject {
    /// Opens the project of `files`, each a path relative to the project's root and its bytes,
    /// in a store in memory named `name`, such as `sales`, and loads its data scripts, in a
    /// session of one partition, since a page runs one task at a time.
    pub async fn open(name: &str, files: Vec<(String, Vec<u8>)>) -> Result<SqlProject, String> {
        let source = FileSource::embedded(name, files)
            .await
            .map_err(|error| error.to_string())?;
        let files = ProjectFiles::open(source)
            .await
            .map_err(|error| error.to_string())?;
        let charts = files.chart_scripts().to_vec();
        let mut analysis = Analysis::new(&files)
            .await
            .map_err(|error| error.to_string())?;
        let ctx = SessionContext::new_with_config(SessionConfig::new().with_target_partitions(1));
        let (loader, state) = Loader::load(files, ctx)
            .await
            .map_err(|error| error.to_string())?;
        analysis.apply(&state);
        Ok(SqlProject {
            loader,
            analysis: RefCell::new(analysis),
            charts,
        })
    }

    /// The paths of the project's chart scripts.
    pub fn chart_scripts(&self) -> &[String] {
        &self.charts
    }

    /// The text of the file at `path`, as the editor's last change left it.
    pub fn text(&self, path: &str) -> Option<String> {
        let analysis = self.analysis.borrow();
        let document = analysis.project().document(path)?;
        Some(document.text().to_string())
    }

    /// The bytes of the charts of the last query of the chart script at `path`, with its text, one
    /// for each row of the query's rows.
    pub async fn charts(&self, path: &str) -> Result<Vec<Vec<u8>>, String> {
        let text = self
            .text(path)
            .ok_or_else(|| format!("the project has no chart script {path}"))?;
        let query =
            (self.loader.run_chart(path, &text).await).map_err(|error| error.to_string())?;
        let batches = query.collect().await.map_err(|error| error.to_string())?;
        Ok(charts(&batches))
    }

    /// Sets the text of the file at `path` to `text`, as an editor changes it: the analysis of
    /// every file reads it, and a data script or a module loads again, with the scripts
    /// downstream of it.
    pub async fn set_text(&self, path: &str, text: String) -> Result<(), String> {
        let role = self.analysis.borrow().project().role(path).cloned();
        let Some(role) = role else {
            return Err(format!("the project has no script {path}"));
        };
        (self.analysis.borrow_mut().project_mut()).set_texts([(path, text.clone())]);
        if matches!(role, FileRole::Data | FileRole::Module { .. }) {
            let (_, state) =
                (self.loader.reload(path, text).await).map_err(|error| error.to_string())?;
            self.analysis.borrow_mut().apply(&state);
        }
        Ok(())
    }

    /// The errors and the warnings of the file at `path`.
    pub fn diagnostics(&self, path: &str) -> Vec<Diagnostic> {
        let analysis = self.analysis.borrow();
        let Some(text) = self.text(path) else {
            return Vec::new();
        };
        (analysis.project().diagnostics(path).into_iter())
            .map(|found| diagnostic(&text, found))
            .collect()
    }

    /// The completions at the position `at` of the file at `path`.
    pub fn completions(&self, path: &str, at: u32) -> Option<Completions> {
        let text = self.text(path)?;
        let analysis = self.analysis.borrow();
        let completions = analysis.project().completions(path, offset(&text, at));
        Some(Completions {
            range: range(&text, completions.range),
            items: (completions.items.into_iter())
                .map(|item| CompletionItem {
                    label: item.label,
                    kind: kind(item.kind),
                    detail: item.detail,
                    documentation: item.documentation,
                    insert: item.insert,
                    filter: item.filter,
                })
                .collect(),
        })
    }

    /// The hover at the position `at` of the file at `path`.
    pub fn hover(&self, path: &str, at: u32) -> Option<Hover> {
        let text = self.text(path)?;
        let analysis = self.analysis.borrow();
        let hover = analysis.project().hover(path, offset(&text, at))?;
        Some(Hover {
            range: range(&text, hover.range),
            markdown: hover.markdown,
        })
    }

    /// The forms of the call at the position `at` of the file at `path`.
    pub fn signature_help(&self, path: &str, at: u32) -> Option<SignatureHelp> {
        let text = self.text(path)?;
        let analysis = self.analysis.borrow();
        let help = analysis.project().signature_help(path, offset(&text, at))?;
        Some(SignatureHelp {
            signatures: (help.signatures.into_iter())
                .map(|form| Signature {
                    parameters: (form.parameters.iter())
                        .map(|parameter| Range {
                            start: position(&form.label, parameter.range.start),
                            end: position(&form.label, parameter.range.end),
                        })
                        .collect(),
                    label: form.label,
                    documentation: form.documentation,
                    active_parameter: form.active_parameter,
                })
                .collect(),
            active_signature: help.active_signature,
        })
    }

    /// The outline of the file at `path`.
    pub fn symbols(&self, path: &str) -> Vec<Symbol> {
        let Some(text) = self.text(path) else {
            return Vec::new();
        };
        let analysis = self.analysis.borrow();
        let Some(document) = analysis.project().document(path) else {
            return Vec::new();
        };
        (document.symbols().into_iter())
            .map(|item| symbol(&text, item))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_code_units() {
        let text = "é𝄞x";
        assert_eq!(position(text, 2), 1);
        assert_eq!(position(text, 6), 3);
        assert_eq!(offset(text, 3), Offset::from(6));
        assert_eq!(offset(text, 9), Offset::from(text.len() as u32));
    }
}
