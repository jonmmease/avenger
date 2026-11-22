//! `avenger check`: the errors and warnings of the selected files. A project loads as an editor
//! loads it, so the data scripts run and their tables have their schemas, and each file has the
//! diagnostics of its analysis, the project's, and the loader's. A stylesheet has the errors of
//! the load of the entry stylesheet. Each `{avenger-sql}` cell of a Quarto document is a chart
//! script of the document's project, whose diagnostics are at their places in the document. With
//! `--run`, each chart script and cell without errors runs too. A loose script, module, or cell
//! analyzes alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library, ScriptError};
use avenger_sql_language::{Diagnostic, Document, Role as TextRole, Severity as LanguageSeverity};
use avenger_sql_project::{
    qmd::{cell_path, is_document, Cell, Qmd},
    Analysis, ChartError, Loader, ProjectStylesheet, Role, MODULE_SUFFIX,
};
use datafusion::{common::DataFusionError, prelude::SessionContext};

use crate::{
    files::{select, ProjectSelection},
    report::{Output, Report, Severity},
    Status,
};

/// The offsets of a file of the language analysis in the file on disk that holds it: the same
/// for a script or a module, and in its document for a cell.
type Place<'a> = &'a dyn Fn(usize) -> usize;

fn same(offset: usize) -> usize {
    offset
}

/// Checks the files of `paths`, and runs the charts when `run`.
pub async fn run(paths: &[PathBuf], run: bool, output: &Output) -> Result<Status, String> {
    let selection = select(paths).await?;
    let mut reports = Vec::new();
    for project in &selection.projects {
        reports.extend(check_project(project, run).await?);
    }
    for path in &selection.loose {
        reports.extend(check_loose(path)?);
    }
    let failed = reports
        .iter()
        .any(|report| report.severity == Severity::Error);
    output.diagnostics(reports);
    Ok(match failed {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// The reports of the selected files of a project, after its load.
async fn check_project(project: &ProjectSelection, run: bool) -> Result<Vec<Report>, String> {
    let files = &project.files;
    let error = |error: &dyn std::fmt::Display| format!("{}: {error}", project.root.display());
    let mut analysis = Analysis::new(files).await.map_err(|e| error(&e))?;
    let (loader, state) =
        (Loader::load(files.clone(), SessionContext::new()).await).map_err(|e| error(&e))?;
    analysis.apply(&state);
    let stylesheets = (project.paths.iter()).any(|(_, role)| *role == Role::Stylesheet);
    let stylesheet = match stylesheets {
        true => Some(files.load_stylesheet().await.map_err(|e| error(&e))?),
        false => None,
    };

    let mut reports = Vec::new();
    for (path, role) in &project.paths {
        let file = project.root.join(path);
        match role {
            Role::Stylesheet => {
                let text = (files.source().read_text(path).await).map_err(|e| error(&e))?;
                let stylesheet = stylesheet.as_ref().expect("the loaded stylesheet");
                reports.extend(style_reports(stylesheet, path, &file, &text));
            }
            Role::DataScript | Role::ChartScript | Role::Module(_) => {
                let Some(document) = analysis.project().document(path) else {
                    continue;
                };
                let text = document.text();
                let chart = run && *role == Role::ChartScript;
                let file = File {
                    path: &file,
                    text,
                    place: &same,
                    first_line: None,
                };
                reports.extend(check_text(&analysis, &loader, path, text, chart, &file).await);
            }
            Role::Document => {
                let Some(document) = analysis.qmd(path) else {
                    continue;
                };
                for (index, cell) in document.cells.iter().enumerate() {
                    let place = |offset| cell.to_document(offset);
                    let file = File {
                        path: &file,
                        text: &document.text,
                        place: &place,
                        first_line: Some(first_line(&document.text, cell)),
                    };
                    let path = cell_path(path, index + 1);
                    let text = &cell.text;
                    reports.extend(check_text(&analysis, &loader, &path, text, run, &file).await);
                }
            }
        }
    }
    Ok(reports)
}

/// A file on disk, whose text holds the text of a file of the analysis at `place`.
struct File<'a> {
    path: &'a Path,
    text: &'a str,
    place: Place<'a>,
    /// The bytes of the first line of a cell, where the errors of the whole cell are, or `None`
    /// for a whole file.
    first_line: Option<(usize, usize)>,
}

/// The bytes of the first line of `cell` in its document's text `text`.
fn first_line(text: &str, cell: &Cell) -> (usize, usize) {
    let start = cell.range.start;
    let line = text[start..cell.range.end]
        .lines()
        .next()
        .unwrap_or_default();
    (start, start + line.len())
}

/// The reports of the file at `path` of the analysis, with `text`, in `file`, and when `run` and
/// it has no errors, of a run of it as a chart script.
async fn check_text(
    analysis: &Analysis,
    loader: &Loader,
    path: &str,
    text: &str,
    run: bool,
    file: &File<'_>,
) -> Vec<Report> {
    let diagnostics = analysis.project().diagnostics(path);
    let errors =
        (diagnostics.iter()).any(|diagnostic| diagnostic.severity == LanguageSeverity::Error);
    let mut reports: Vec<Report> = (diagnostics.iter())
        .map(|diagnostic| language_report(file, diagnostic))
        .collect();
    if run && !errors {
        reports.extend(run_chart(loader, path, text, file).await);
    }
    reports
}

/// The reports of the stylesheet at `path`, the file `file` with `text`, from the load of the
/// entry stylesheet.
fn style_reports(
    stylesheet: &ProjectStylesheet,
    path: &str,
    file: &Path,
    text: &str,
) -> Vec<Report> {
    (stylesheet.diagnostics.iter())
        .filter(|diagnostic| stylesheet.path(diagnostic.span.source) == Some(path))
        .map(|diagnostic| {
            let severity = match diagnostic.severity {
                avenger_style::Severity::Error => Severity::Error,
                avenger_style::Severity::Warning => Severity::Warning,
            };
            let span = (diagnostic.span.start, diagnostic.span.end);
            Report::at(
                file,
                text,
                span,
                severity,
                diagnostic.message.clone(),
                "style",
            )
            .with_help(diagnostic.help.clone())
        })
        .collect()
}

/// The report of the diagnostic `diagnostic`, at its place in `file`.
fn language_report(file: &File, diagnostic: &Diagnostic) -> Report {
    let severity = match diagnostic.severity {
        LanguageSeverity::Error => Severity::Error,
        LanguageSeverity::Warning => Severity::Warning,
    };
    let source = match diagnostic.source {
        avenger_sql_language::Source::Syntax => "syntax",
        avenger_sql_language::Source::Analysis => "analysis",
        avenger_sql_language::Source::Project => "project",
        avenger_sql_language::Source::Loader => "loader",
        avenger_sql_language::Source::Typst => "typst",
    };
    let range = diagnostic.range;
    let bytes = (
        (file.place)(usize::from(range.start())),
        (file.place)(usize::from(range.end())),
    );
    let message = diagnostic.message.clone();
    Report::at(file.path, file.text, bytes, severity, message, source)
        .with_help(diagnostic.help.clone())
}

/// The errors of a run of the chart script at `path`, with `text`, in `file`: the errors of its
/// script, at their ranges, or the error of its query, of the whole file. A read of a table that
/// did not load has the project's error already, so it reports nothing more.
async fn run_chart(loader: &Loader, path: &str, text: &str, file: &File<'_>) -> Vec<Report> {
    let error = match loader.run_chart(path, text).await {
        Ok(frame) => match frame.collect().await {
            Ok(_) => return Vec::new(),
            Err(error) => error,
        },
        Err(ChartError::Unloaded { .. }) => return Vec::new(),
        Err(ChartError::Run(error)) => error,
    };
    if let DataFusionError::External(external) = &error {
        if let Some(script) = external.downcast_ref::<ScriptError>() {
            return (script.diagnostics.iter())
                .map(|diagnostic| {
                    let range = diagnostic.range;
                    let bytes = (
                        (file.place)(usize::from(range.start())),
                        (file.place)(usize::from(range.end())),
                    );
                    let message = diagnostic.message.clone();
                    Report::at(file.path, file.text, bytes, Severity::Error, message, "run")
                })
                .collect();
        }
    }
    let message = format!("the chart's query fails: {error}");
    vec![match file.first_line {
        Some(line) => Report::at(file.path, file.text, line, Severity::Error, message, "run"),
        None => Report::file(file.path, Severity::Error, message, "run"),
    }]
}

/// The reports of the loose script, module, or document at `path`, which analyzes alone: a
/// script or a cell as a chart script and a module as a module, with no tables but the ones that
/// it defines.
fn check_loose(path: &Path) -> Result<Vec<Report>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.to_string_lossy();
    if is_document(&name) {
        let document = Qmd::parse(text.as_str());
        let mut reports = Vec::new();
        for cell in &document.cells {
            let place = |offset| cell.to_document(offset);
            let file = File {
                path,
                text: &text,
                place: &place,
                first_line: Some(first_line(&text, cell)),
            };
            reports.extend(loose_reports(&cell.text, TextRole::Script, &file));
        }
        return Ok(reports);
    }
    let role = match name.ends_with(MODULE_SUFFIX) {
        true => TextRole::Module,
        false => TextRole::Script,
    };
    let file = File {
        path,
        text: &text,
        place: &same,
        first_line: None,
    };
    Ok(loose_reports(&text, role, &file))
}

/// The reports of `text` of `role`, which analyzes alone, in `file`.
fn loose_reports(text: &str, role: TextRole, file: &File) -> Vec<Report> {
    let document = Document::new(
        text,
        role,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    let help = "The file is in no avenger project, or has no role in its manifest, so it reads \
                no tables of a project: run `avenger check` on it inside a project";
    (document.diagnostics().iter())
        .map(|diagnostic| {
            let report = language_report(file, diagnostic);
            match diagnostic.message.starts_with("Table not found") {
                true => report.with_help(Some(help.to_string())),
                false => report,
            }
        })
        .collect()
}
