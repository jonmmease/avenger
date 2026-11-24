//! The chart of a window, without the window: its project, loaded as `render` loads it, its runs,
//! and what each change of the project's files does to it. A change of the chart's own text runs
//! it again, and a change of a data script whose statement the window shows loads the script
//! again first. A change of a data script or a module loads it again, and runs the chart again when
//! a table that it reads changed or it imports a module. A change of a data file loads the data
//! scripts that read it, and runs the chart again on the same terms. A change of a stylesheet
//! restyles the chart without running it, and a change of the manifest opens the project again.
//! A change of another chart does nothing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use avenger_chart::results::QueryResult;
use avenger_sql_project::{
    charts_to_run,
    qmd::Qmd,
    watch::{watched, Watched},
    ProjectStylesheet, Role, MANIFEST,
};

use avenger_chart::results::is_chart_field;
use avenger_results_egui::{RowSource, RowStream};
use datafusion::{arrow::datatypes::SchemaRef, dataframe::DataFrame};
use futures::{future::LocalBoxFuture, StreamExt};

use crate::render::{collect_target, load, plan_target, Failure, Target};
use crate::report::{Report, Severity};
use crate::statement::split_line;

/// What a change did to the chart.
pub enum Outcome {
    /// The chart ran, with its result, which has charts.
    Ran(Arc<QueryResult>),
    /// The chart planned a table without charts, whose rows read as the window shows them.
    Rows(Arc<dyn RowSource>),
    /// The chart did not run, with its errors, each a line as `check` prints it, so the window
    /// keeps the result before.
    Failed(Vec<String>),
    /// The project's stylesheets changed, with their new load.
    Restyled(ProjectStylesheet),
}

/// The chart of a window: the chart's name, as the command took it, and its loaded project.
pub struct Session {
    chart: String,
    target: Target,
}

impl Session {
    /// Opens the project of `chart`, a chart script, `doc.qmd#NAME`, or the statement at a line
    /// of either as `FILE:LINE`, and runs the chart. A chart whose project does not open fails,
    /// and a chart with errors opens with them.
    pub async fn open(chart: &str) -> Result<(Session, Outcome), String> {
        let target = load(chart).await.map_err(|failure| match failure {
            Failure::Usage(error) => error,
            Failure::Reports(reports) => lines(&reports).join("\n"),
        })?;
        let mut session = Session {
            chart: chart.to_string(),
            target,
        };
        let outcome = session.run().await;
        Ok((session, outcome))
    }

    /// The directory that the window watches: the project's root, or the directory of a loose
    /// file.
    pub fn watched_directory(&self) -> PathBuf {
        match &self.target.root {
            Some(root) => root.clone(),
            None => {
                (self.target.file.parent()).map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            }
        }
    }

    /// The stylesheet of the project, as it loaded.
    pub fn stylesheet(&self) -> &ProjectStylesheet {
        &self.target.stylesheet
    }

    /// The window's title of the chart: its file, with the line of its statement, its cell, and
    /// what its statement shows.
    pub fn title(&self) -> String {
        self.target.title()
    }

    /// The chart's file on disk.
    pub fn file(&self) -> &Path {
        &self.target.file
    }

    /// Runs the chart: a result with charts collects, and a table without charts reads its rows
    /// as the window scrolls, so a large table never collects.
    pub async fn run(&mut self) -> Outcome {
        let failed = |failure| match failure {
            Failure::Reports(reports) => Outcome::Failed(lines(&reports)),
            Failure::Usage(error) => Outcome::Failed(vec![error]),
        };
        match plan_target(&self.target).await {
            Ok(frame) => outcome(&self.target, frame).await,
            Err(failure) => failed(failure),
        }
    }

    /// Opens the project again and runs the chart, as after its manifest changes.
    async fn reopen(&mut self) -> Vec<Outcome> {
        // A target at a line opens at its statement's line now.
        if let (Ok((file, Some(_))), Some(line)) = (split_line(&self.chart), self.target.line) {
            self.chart = format!("{file}:{line}");
        }
        match Session::open(&self.chart).await {
            Ok((session, outcome)) => {
                let restyled = Outcome::Restyled(session.target.stylesheet.clone());
                *self = session;
                vec![restyled, outcome]
            }
            Err(error) => vec![Outcome::Failed(vec![error])],
        }
    }

    /// What the changes of the files at `paths`, canonical paths on disk, do to the chart, in
    /// order: none, a restyle, a run, or both.
    pub async fn changed(&mut self, paths: &[PathBuf]) -> Vec<Outcome> {
        let Some(root) = self.target.root.clone() else {
            // A loose file is a project of one file, which opens again when it changes.
            return match paths.contains(&self.target.file) {
                true => self.reopen().await,
                false => Vec::new(),
            };
        };
        if paths.iter().any(|path| *path == root.join(MANIFEST)) {
            return self.reopen().await;
        }
        let (mut run, mut restyle) = (false, false);
        let mut data = Vec::new();
        let mut changes = Vec::new();
        let mut reading = Vec::new();
        for path in paths {
            if *path == self.target.file && self.target.data {
                // A data script's statement shows its table after the script loads again.
                let Ok(text) = std::fs::read_to_string(path) else {
                    return self.reopen().await;
                };
                let target = &mut self.target;
                match target.loader.reload(&target.script, text.clone()).await {
                    Ok((tables, state)) => {
                        let project = target.analysis.project_mut();
                        project.set_texts([(target.script.clone(), text.clone())]);
                        target.analysis.apply(&state);
                        changes.extend(tables);
                    }
                    Err(_) => return self.reopen().await,
                }
                match self.own_text_changed(path) {
                    Ok(()) => run = true,
                    Err(error) => return vec![Outcome::Failed(vec![error])],
                }
                continue;
            }
            if *path == self.target.file {
                match self.own_text_changed(path) {
                    Ok(()) => run = true,
                    Err(error) => return vec![Outcome::Failed(vec![error])],
                }
                continue;
            }
            match watched(&root, &self.target.files, path) {
                Some(Watched::Project(relative)) => {
                    match self.target.files.manifest().role(&relative) {
                        Ok(Some(Role::DataScript | Role::Module(_))) => {
                            let Ok(text) = std::fs::read_to_string(path) else {
                                // A data script or a module that went away changes the
                                // project's files.
                                return self.reopen().await;
                            };
                            let target = &mut self.target;
                            match target.loader.reload(&relative, text.clone()).await {
                                Ok((tables, state)) => {
                                    target.analysis.project_mut().set_texts([(relative, text)]);
                                    target.analysis.apply(&state);
                                    changes.extend(tables);
                                }
                                // A new data script, which the loader lacks, changes the
                                // project's files.
                                Err(_) => return self.reopen().await,
                            }
                        }
                        Ok(Some(Role::Stylesheet)) => restyle = true,
                        _ => {}
                    }
                }
                Some(Watched::Data(relative)) => data.push(relative),
                None => {}
            }
        }
        if !data.is_empty() {
            if let Some((reload, state)) = self.target.loader.reload_files(&data).await {
                self.target.analysis.apply(&state);
                changes.extend(reload.tables);
                reading.extend(reload.charts);
            }
        }
        let target = &self.target;
        // A data script's statement runs again when a load changes a table, since a step builds
        // its table from the script and its files, which a chart's reads don't show.
        run |= target.data && !changes.is_empty();
        if !changes.is_empty() || !reading.is_empty() {
            let schema = target.loader.default_schema().await;
            let chart = [(target.path.as_str(), target.text.as_str())];
            run |= !charts_to_run(chart, &changes, &reading, &schema).is_empty();
        }
        let mut outcomes = Vec::new();
        if restyle {
            match target.files.load_stylesheet().await {
                Ok(stylesheet) => {
                    self.target.stylesheet = stylesheet.clone();
                    outcomes.push(Outcome::Restyled(stylesheet));
                }
                Err(error) => outcomes.push(Outcome::Failed(vec![error.to_string()])),
            }
        }
        if run {
            outcomes.push(self.run().await);
        }
        outcomes
    }

    /// Reads the chart's own file again: a script, or the document of a cell, whose cell keeps
    /// its label or its number. A target's statement is found again by its key, so it follows
    /// edits around it.
    fn own_text_changed(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let target = &mut self.target;
        // A key that finds no statement keeps the statement before, so a later run runs it.
        let before = (target.path.clone(), target.text.clone());
        match &target.document {
            None => {
                let project = target.analysis.project_mut();
                project.set_texts([(target.script.clone(), text.clone())]);
                target.path = target.script.clone();
                target.text = text;
            }
            Some(_) => {
                let document = Qmd::parse(text.as_str());
                let name = target.cell_name().unwrap_or_else(|| "1".to_string());
                let (index, cell) = document.cell(&name).ok_or_else(|| {
                    format!("`{}` has no cell `{name}` any more", target.display_name())
                })?;
                target.text = cell.text.clone();
                target.path = avenger_sql_project::qmd::cell_path(&target.script, index + 1);
                target.cell = Some(index);
                target.analysis.set_document(&target.script, Some(text));
                target.document = Some(document);
            }
        }
        if let Err(error) = target.point_again() {
            (target.path, target.text) = before;
            return Err(error);
        }
        Ok(())
    }
}

/// What the window shows of `frame`, a frame of `target`'s text, unrun: a result with charts
/// collects, and a table without charts reads its rows as the window scrolls, so a large table
/// never collects.
pub(crate) async fn outcome(target: &Target, frame: DataFrame) -> Outcome {
    let charts = (frame.schema().fields().iter()).any(|field| is_chart_field(field));
    if !charts {
        return Outcome::Rows(Arc::new(FrameRows { frame }));
    }
    match collect_target(target, frame).await {
        Ok(result) => Outcome::Ran(Arc::new(result)),
        Err(Failure::Reports(reports)) => Outcome::Failed(lines(&reports)),
        Err(Failure::Usage(error)) => Outcome::Failed(vec![error]),
    }
}

/// The lines of `reports`, as `check` prints the first line of each: `path:line:column:
/// severity: message`, with the path relative to the current directory under it.
pub(crate) fn lines(reports: &[Report]) -> Vec<String> {
    let current = std::env::current_dir()
        .and_then(std::fs::canonicalize)
        .unwrap_or_default();
    (reports.iter())
        .map(|report| {
            let path = report.path.strip_prefix(&current).unwrap_or(&report.path);
            let place = match &report.range {
                Some(range) => format!(":{}:{}", range.start.line, range.start.column),
                None => String::new(),
            };
            let severity = match report.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
            };
            format!("{}{place}: {severity}: {}", path.display(), report.message)
        })
        .collect()
}

/// The rows of a chart script's frame, which a window reads a page at a time: one run of the
/// frame's query read forward, and its count beside it.
struct FrameRows {
    frame: DataFrame,
}

impl RowSource for FrameRows {
    fn schema(&self) -> SchemaRef {
        self.frame.schema().inner().clone()
    }

    fn count(&self) -> LocalBoxFuture<'static, Result<usize, String>> {
        let frame = self.frame.clone();
        Box::pin(async move { frame.count().await.map_err(|error| error.to_string()) })
    }

    fn stream(&self) -> RowStream {
        let frame = self.frame.clone();
        Box::pin(async move {
            let stream = (frame.execute_stream().await).map_err(|error| error.to_string())?;
            Ok(stream
                .map(|batch| batch.map_err(|error| error.to_string()))
                .boxed_local())
        })
    }
}
