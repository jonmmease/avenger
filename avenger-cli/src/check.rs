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
    qmd::{is_document, Cell, Qmd},
    Analysis, ChartError, Loader, ProjectStylesheet, Role, MODULE_SUFFIX,
};
use datafusion::{common::DataFusionError, prelude::SessionContext};

use crate::{
    files::{select, ProjectSelection},
    render::{cell_aim, Pick},
    report::{Output, Report, Severity},
    Status,
};

/// The offsets of a file of the language analysis in the file on disk that holds it: the same
/// for a script or a module, and in its document for a cell.
pub(crate) type Place<'a> = &'a dyn Fn(usize) -> usize;

pub(crate) fn same(offset: usize) -> usize {
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
                let place = |offset| document.script.to_document(offset);
                let script = File {
                    path: &file,
                    text: &document.text,
                    place: &place,
                    first_line: None,
                };
                let text = &document.script.text;
                let checked = check_text(&analysis, &loader, path, text, false, &script).await;
                let errors = (checked.iter()).any(|report| report.severity == Severity::Error);
                reports.extend(checked);
                for cell in &document.cells {
                    reports.extend(option_reports(cell, &file, &document.text));
                }
                if run && !errors {
                    reports.extend(run_cells(&loader, path, document, &file).await);
                }
            }
        }
    }
    Ok(reports)
}

/// The reports of the runs of the values of the cells of `document`, at `path` in the project,
/// whose file on disk is `file`: each runs on its document's script masked to what it reads, and
/// a failure is at the cell's first line or at the errors of its script.
async fn run_cells(loader: &Loader, path: &str, document: &Qmd, file: &Path) -> Vec<Report> {
    let mut reports = Vec::new();
    let statements = crate::cells::statements(document);
    for (index, cell) in document.cells.iter().enumerate() {
        // A cell without a value, which only defines what later cells read, runs nothing.
        if crate::cells::value(&statements, index).is_none() {
            continue;
        }
        let line = first_line(&document.text, cell);
        let pick = Pick::Name((index + 1).to_string());
        let aim = match cell_aim(document, path, &pick) {
            Ok(aim) => aim,
            Err(error) => {
                let text = &document.text;
                reports.push(Report::at(file, text, line, Severity::Error, error, "run"));
                continue;
            }
        };
        let place = |offset| document.script.to_document(offset);
        let on_disk = File {
            path: file,
            text: &document.text,
            place: &place,
            first_line: Some(line),
        };
        reports.extend(run_chart(loader, path, &aim.masked, &on_disk).await);
    }
    reports
}

/// A file on disk, whose text holds the text of a file of the analysis at `place`.
pub(crate) struct File<'a> {
    pub path: &'a Path,
    pub text: &'a str,
    pub place: Place<'a>,
    /// The bytes of the first line of a cell, where the errors of the whole cell are, or `None`
    /// for a whole file.
    pub first_line: Option<(usize, usize)>,
}

/// The bytes of the first line of `cell` in its document's text `text`.
pub(crate) fn first_line(text: &str, cell: &Cell) -> (usize, usize) {
    let start = cell.range.start;
    let line = text[start..cell.range.end]
        .lines()
        .next()
        .unwrap_or_default();
    (start, start + line.len())
}

/// The reports of the file at `path` of the analysis, with `text`, in `file`, and when `run` and
/// it has no errors, of a run of it as a chart script.
pub(crate) async fn check_text(
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

/// The reports of the errors of the options of `cell`, at their places in its document's file
/// `file`, whose text is `text`.
fn option_reports(cell: &Cell, file: &Path, text: &str) -> Vec<Report> {
    let place = |offset| cell.to_document(offset);
    let file = File {
        path: file,
        text,
        place: &place,
        first_line: None,
    };
    (cell.option_diagnostics().iter())
        .map(|diagnostic| language_report(&file, diagnostic))
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
        avenger_sql_language::Source::Options => "options",
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
        Ok(query) => match query.collect().await {
            Ok(_) => return Vec::new(),
            Err(error) => error,
        },
        Err(ChartError::Unloaded { .. }) => return Vec::new(),
        Err(ChartError::Run(error)) => error,
    };
    failure_reports(&error, file)
}

/// The reports of the error `error` of a run of a chart in `file`: the errors of its script, at
/// their ranges, or the error of its query, of the whole file or of a cell's first line.
pub(crate) fn failure_reports(error: &DataFusionError, file: &File<'_>) -> Vec<Report> {
    if let DataFusionError::External(external) = error {
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
        let place = |offset| document.script.to_document(offset);
        let file = File {
            path,
            text: &text,
            place: &place,
            first_line: None,
        };
        let script = Document::new_cells(
            document.script.text.as_str(),
            document.script.cell_starts(),
            Arc::new(MemoryCatalog::new()),
            Arc::new(Library::new()),
            Options::default(),
        );
        let mut reports = loose_reports(&script, &file);
        for cell in &document.cells {
            reports.extend(option_reports(cell, path, &text));
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
    let document = Document::new(
        text.as_str(),
        role,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    Ok(loose_reports(&document, &file))
}

/// The reports of `document`, which analyzes alone, in `file`.
fn loose_reports(document: &Document, file: &File) -> Vec<Report> {
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
