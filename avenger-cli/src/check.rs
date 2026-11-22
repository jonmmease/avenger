//! `avenger check`: the errors and warnings of the selected files. A project loads as an editor
//! loads it, so the data scripts run and their tables have their schemas, and each file has the
//! diagnostics of its analysis, the project's, and the loader's. A stylesheet has the errors of
//! the load of the entry stylesheet. With `--run`, each chart script without errors runs too.
//! A loose script or module analyzes alone.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_sql::{analysis::Options, catalog::MemoryCatalog, library::Library, ScriptError};
use avenger_sql_language::{Diagnostic, Document, Role as TextRole, Severity as LanguageSeverity};
use avenger_sql_project::{Analysis, ChartError, Loader, ProjectStylesheet, Role, MODULE_SUFFIX};
use datafusion::{common::DataFusionError, prelude::SessionContext};

use crate::{
    files::{select, ProjectSelection},
    report::{Output, Report, Severity},
    Status,
};

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
                let diagnostics = analysis.project().diagnostics(path);
                let errors = (diagnostics.iter())
                    .any(|diagnostic| diagnostic.severity == LanguageSeverity::Error);
                reports.extend(diagnostics.iter().map(|d| language_report(&file, text, d)));
                if run && *role == Role::ChartScript && !errors {
                    reports.extend(run_chart(&loader, path, &file, text).await);
                }
            }
        }
    }
    Ok(reports)
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

/// The report of the diagnostic `diagnostic` of the file `file` with `text`.
fn language_report(file: &Path, text: &str, diagnostic: &Diagnostic) -> Report {
    let severity = match diagnostic.severity {
        LanguageSeverity::Error => Severity::Error,
        LanguageSeverity::Warning => Severity::Warning,
    };
    let source = match diagnostic.source {
        avenger_sql_language::Source::Syntax => "syntax",
        avenger_sql_language::Source::Analysis => "analysis",
        avenger_sql_language::Source::Project => "project",
        avenger_sql_language::Source::Loader => "loader",
    };
    let range = diagnostic.range;
    let bytes = (usize::from(range.start()), usize::from(range.end()));
    Report::at(
        file,
        text,
        bytes,
        severity,
        diagnostic.message.clone(),
        source,
    )
    .with_help(diagnostic.help.clone())
}

/// The errors of a run of the chart script at `path`, the file `file` with `text`: the errors
/// of its script, at their ranges, or the error of its query, of the whole file. A read of a
/// table that did not load has the project's error already, so it reports nothing more.
async fn run_chart(loader: &Loader, path: &str, file: &Path, text: &str) -> Vec<Report> {
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
                    let bytes = (usize::from(range.start()), usize::from(range.end()));
                    let message = diagnostic.message.clone();
                    Report::at(file, text, bytes, Severity::Error, message, "run")
                })
                .collect();
        }
    }
    let message = format!("the chart's query fails: {error}");
    vec![Report::file(file, Severity::Error, message, "run")]
}

/// The reports of the loose script or module at `path`, which analyzes alone, as a chart script
/// or a module, with no tables but the ones that it defines.
fn check_loose(path: &Path) -> Result<Vec<Report>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let role = match path.to_string_lossy().ends_with(MODULE_SUFFIX) {
        true => TextRole::Module,
        false => TextRole::Script,
    };
    let document = Document::new(
        &text,
        role,
        Arc::new(MemoryCatalog::new()),
        Arc::new(Library::new()),
        Options::default(),
    );
    let help = "The file is in no avenger project, or has no role in its manifest, so it reads \
                no tables of a project: run `avenger check` on it inside a project";
    Ok((document.diagnostics().iter())
        .map(|diagnostic| {
            let report = language_report(path, &text, diagnostic);
            match diagnostic.message.starts_with("Table not found") {
                true => report.with_help(Some(help.to_string())),
                false => report,
            }
        })
        .collect())
}
