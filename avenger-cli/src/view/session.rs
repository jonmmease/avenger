//! The chart of a window, without the window: its project, loaded as `render` loads it, its runs,
//! and what each change of the project's files does to it. A change of the chart's own text runs
//! it again. A change of a data script or a module loads it again, and runs the chart again when
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

use crate::render::{load, run_target, Failure, Target};
use crate::report::{Report, Severity};
use crate::statement::cell_at_line;

/// What a change did to the chart.
pub enum Outcome {
    /// The chart ran, with its result.
    Ran(Arc<QueryResult>),
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

    /// The chart's file on disk.
    pub fn file(&self) -> &Path {
        &self.target.file
    }

    /// Runs the chart.
    pub async fn run(&mut self) -> Outcome {
        match run_target(&self.target).await {
            Ok(result) => Outcome::Ran(Arc::new(result)),
            Err(Failure::Reports(reports)) => Outcome::Failed(lines(&reports)),
            Err(Failure::Usage(error)) => Outcome::Failed(vec![error]),
        }
    }

    /// Opens the project again and runs the chart, as after its manifest changes.
    async fn reopen(&mut self) -> Vec<Outcome> {
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
    /// its name, or its line. A target at a line points at the statement at the line again.
    fn own_text_changed(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let target = &mut self.target;
        match &target.document {
            None => {
                let project = target.analysis.project_mut();
                project.set_texts([(target.script.clone(), text.clone())]);
                target.path = target.script.clone();
                target.text = text;
            }
            Some(_) => {
                let document = Qmd::parse(text.as_str());
                let (index, cell) = match target.line {
                    Some(line) => (cell_at_line(&document, line))
                        .map_err(|error| format!("{}: {error}", self.chart))?,
                    None => {
                        let name = (self.chart.rsplit_once('#')).map_or("1", |(_, name)| name);
                        (document.cell(name)).ok_or_else(|| {
                            format!("{}: no `{{avenger-sql}}` cell `{name}`", path.display())
                        })?
                    }
                };
                target.text = cell.text.clone();
                target.path = avenger_sql_project::qmd::cell_path(&target.script, index + 1);
                target.cell = Some(index);
                target.analysis.set_document(&target.script, Some(text));
                target.document = Some(document);
            }
        }
        if let Some(line) = target.line {
            (target.point_at(line)).map_err(|error| format!("{}: {error}", self.chart))?;
        }
        Ok(())
    }
}

/// The lines of `reports`, as `check` prints the first line of each: `path:line:column:
/// severity: message`, with the path relative to the current directory under it.
fn lines(reports: &[Report]) -> Vec<String> {
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
