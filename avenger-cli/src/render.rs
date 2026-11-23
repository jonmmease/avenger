//! `avenger render`: the chart of a chart script, or of a cell of a Quarto document, drawn to
//! PNG, SVG, or PDF by the chart app of `avenger view`, offscreen, with its widgets.
//!
//! The script runs through its project, or a loose script as a project of one file, and its
//! result decides the image: one row of one column of charts is that chart, and any other result
//! is a table, with a header of the column names, a row for each result row, each chart in its
//! cell, and the other columns as text, a row of text alone one line tall. `--size` is the size of a chart,
//! of the chart alone or of each cell of a table, and without it a chart takes its own size or
//! its natural size, and a table's columns and rows the largest natural sizes of their charts.
//! The image waits for the images of its charts, such as the tiles of a map, up to a timeout.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use avenger_chart::{
    results::{QueryResult, Shape},
    Chart,
};
use avenger_chart_app::{ChartApp, ChartUpdates};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_common::canvas::CanvasDimensions;
use avenger_image::{ImageCache, ImageCacheOptions, ImageResolver};
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_sql_language::FileRole;
use avenger_sql_project::{
    charts::{background, label_engine, preferences},
    find_root,
    qmd::{cell_path, is_document, Cell, Qmd},
    Analysis, ChartError, FileSource, Loader, ProjectFiles, ProjectStylesheet, Role,
};
use avenger_style::{Cascade, ColorScheme, HostInputs, ParseOptions, Sheet, SourceId};
use avenger_typst_label::{LabelEngine, LabelOptions, LabelWidth, TextStyle};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{common::DataFusionError, dataframe::DataFrame};

use crate::{
    check::{check_text, failure_reports, first_line, File, Place},
    project::{load_files, targets},
    report::{Output, Report, Severity},
    statement::{at_line, by_key, cell_at_line, data_tables, split_line, Script, Shows, Statement},
    Status,
};

/// The path in the analysis of the text that runs a statement other than a script's value, a
/// chart script of its own.
const STATEMENT_PATH: &str = "<statement>";

/// The options of a render.
pub struct Options {
    /// The size of a chart, either side of which can be unset.
    pub size: [Option<f32>; 2],
    /// The pixel density of PNG.
    pub scale: f32,
    /// The reader's color scheme, light or dark, with its background.
    pub color_scheme: ColorScheme,
    /// A stylesheet after the project's.
    pub style: Option<PathBuf>,
    /// The modes of the chart to turn on.
    pub modes: Vec<String>,
    /// The cell of a table to render alone, as `ROW[.COLUMN]`.
    pub chart: Option<String>,
    /// The most rows of a table.
    pub rows: usize,
    /// How long to wait for the images of the charts.
    pub timeout: Duration,
    /// A multi-thread runtime, on which the chart apps prepare their charts, which the caller
    /// owns, since a runtime starts and ends outside the futures of others.
    pub runtime: tokio::runtime::Handle,
}

/// The format of an image, by the extension of its file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Png,
    Svg,
    Pdf,
}

impl Format {
    /// The format of the file `path`, by its extension.
    pub fn of(path: &Path) -> Result<Format, String> {
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        match extension.to_ascii_lowercase().as_str() {
            "png" => Ok(Format::Png),
            "svg" => Ok(Format::Svg),
            "pdf" => Ok(Format::Pdf),
            _ => Err(format!(
                "{}: the image is PNG, SVG, or PDF, by the extension `.png`, `.svg`, or `.pdf`",
                path.display()
            )),
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Svg => "svg",
            Format::Pdf => "pdf",
        }
    }
}

/// Why a chart did not render: the reports of its errors, which fail the command, or an error
/// that keeps it from running.
pub(crate) enum Failure {
    Reports(Vec<Report>),
    Usage(String),
}

impl From<String> for Failure {
    fn from(error: String) -> Self {
        Failure::Usage(error)
    }
}

/// Renders the chart `chart` to `out`.
pub async fn run(
    chart: &str,
    out: &Path,
    options: &Options,
    output: &Output,
) -> Result<Status, String> {
    let format = Format::of(out)?;
    let renderer = Renderer::new(options)?;
    match render_target(chart, out, format, options, &renderer).await {
        Ok(()) => {
            output.written(&[out.to_path_buf()]);
            Ok(Status::Ok)
        }
        Err(Failure::Reports(reports)) => {
            output.diagnostics(reports);
            Ok(Status::Failed)
        }
        Err(Failure::Usage(error)) => Err(error),
    }
}

/// Renders every chart script and cell of the project of `path` into the directory `out`, one
/// file for each, named after its path, with `/` and `#` as `-`.
pub async fn run_all(
    path: &Path,
    out: &Path,
    format: Format,
    options: &Options,
    output: &Output,
) -> Result<Status, String> {
    let canonical =
        std::fs::canonicalize(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let root = find_root(&canonical)
        .ok_or_else(|| format!("{}: not in an avenger project", path.display()))?;
    let files =
        (ProjectFiles::find(&root).await).map_err(|e| format!("{}: {e}", root.display()))?;
    let analysis = Analysis::new(&files).await.map_err(|e| e.to_string())?;
    let targets = targets(&files, &analysis);
    std::fs::create_dir_all(out).map_err(|error| format!("{}: {error}", out.display()))?;
    let renderer = Renderer::new(options)?;
    let (mut written, mut reports) = (Vec::new(), Vec::new());
    for target in targets {
        let name = format!("{}.{}", target.replace(['/', '#'], "-"), format.extension());
        let file = out.join(name);
        let chart = root.join(&target).to_string_lossy().into_owned();
        match render_target(&chart, &file, format, options, &renderer).await {
            Ok(()) => written.push(file),
            Err(Failure::Reports(failed)) => reports.extend(failed),
            Err(Failure::Usage(error)) => {
                let path = root.join(target.split('#').next().unwrap_or(&target));
                reports.push(Report::file(&path, Severity::Error, error, "render"));
            }
        }
    }
    output.written(&written);
    let failed = !reports.is_empty();
    output.diagnostics(reports);
    Ok(match failed {
        true => Status::Failed,
        false => Status::Ok,
    })
}

/// What every chart of a run shares: the label engine, the extra stylesheet, the image cache, and the
/// runtime that the chart apps prepare their charts on.
struct Renderer {
    engine: LabelEngine,
    extra: Option<Sheet>,
    images: ImageCache,
    runtime: tokio::runtime::Handle,
}

impl Renderer {
    fn new(options: &Options) -> Result<Renderer, String> {
        let extra = match &options.style {
            Some(path) => Some(load_sheet(path)?),
            None => None,
        };
        Ok(Renderer {
            engine: label_engine(),
            extra,
            images: ImageCache::with_options(image_options()?),
            runtime: options.runtime.clone(),
        })
    }
}

/// The stylesheet at `path`, alone, without imports of other files.
fn load_sheet(path: &Path) -> Result<Sheet, String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) =
        Sheet::load(&text, None, SourceId(u32::MAX - 1), options, &mut no_files);
    if let Some(error) = diagnostics.first() {
        return Err(format!("{}: {}", path.display(), error.message));
    }
    Ok(sheet)
}

/// How the charts' images load: with the User-Agent of the program, as the usage policies of tile
/// servers ask, and with a disk cache in the platform's cache directory, when it has one.
pub(crate) fn image_options() -> Result<ImageCacheOptions, String> {
    const USER_AGENT: &str = concat!(
        "avenger-cli/",
        env!("CARGO_PKG_VERSION"),
        " (+https://github.com/jonmmease/avenger)"
    );
    let fetcher = avenger_image::reqwest_fetcher::ReqwestImageFetcher::new(
        USER_AGENT,
        Duration::from_secs(30),
    )
    .map_err(|error| error.to_string())?;
    Ok(ImageCacheOptions {
        fetcher: Arc::new(fetcher),
        disk: cache_dir().map(|dir| avenger_image::DiskCacheOptions::new(dir.join("images"))),
        ..Default::default()
    })
}

/// The directory of the program's caches: `~/Library/Caches/avenger` on macOS,
/// `%LOCALAPPDATA%\avenger` on Windows, and `$XDG_CACHE_HOME/avenger` or `~/.cache/avenger`
/// elsewhere.
fn cache_dir() -> Option<PathBuf> {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home()?.join("Library/Caches")
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
    } else {
        match std::env::var_os("XDG_CACHE_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => home()?.join(".cache"),
        }
    };
    Some(base.join("avenger"))
}

/// A chart script or a cell, ready to run: the analysis path of its text, the text, and its
/// file on disk, with the place of the text in the file.
pub(crate) struct Target {
    pub(crate) loader: Loader,
    pub(crate) analysis: Analysis,
    pub(crate) stylesheet: ProjectStylesheet,
    pub(crate) path: String,
    pub(crate) text: String,
    /// The path in the analysis of the target's file: its script, or its document, which a
    /// target's run keeps when it runs a statement at [`STATEMENT_PATH`].
    pub(crate) script: String,
    /// The file on disk, and for a cell, the text of its document.
    pub(crate) file: PathBuf,
    pub(crate) document: Option<Qmd>,
    pub(crate) cell: Option<usize>,
    /// The files of the project, and its root on disk, or `None` for a loose file, which runs as
    /// a project of one file.
    pub(crate) files: ProjectFiles,
    pub(crate) root: Option<PathBuf>,
    /// The line of the file that the target names, from 1, and its statement, when the target
    /// names one as `FILE:LINE`.
    pub(crate) line: Option<usize>,
    pub(crate) statement: Option<Statement>,
    /// Whether the target's file is a data script, whose statements show their tables from the
    /// project's session.
    pub(crate) data: bool,
}

impl Target {
    /// The report of the error `message` of the target: of its whole file, or of a cell's first
    /// line.
    pub(crate) fn report(&self, message: String) -> Report {
        match (&self.document, self.cell) {
            (Some(document), Some(index)) => {
                let line = first_line(&document.text, &document.cells[index]);
                Report::at(
                    &self.file,
                    &document.text,
                    line,
                    Severity::Error,
                    message,
                    "render",
                )
            }
            _ => Report::file(&self.file, Severity::Error, message, "render"),
        }
    }

    /// Points the target at the statement of its text at the line `line` of its file, from 1: a
    /// line of its script, or of its cell's document. A statement other than the script's value
    /// runs as a chart script of its own, at [`STATEMENT_PATH`], whose text runs the statement.
    pub(crate) fn point_at(&mut self, line: usize) -> Result<(), String> {
        let before = self.lines_before();
        let statement = at_line(&self.text, line - before, before, self.script_kind())?;
        self.show(statement);
        Ok(())
    }

    /// Points the target at its statement again, by the statement's key, after its text changed:
    /// its script's text, or its cell's. The target's line follows the statement.
    pub(crate) fn point_again(&mut self) -> Result<(), String> {
        let Some(statement) = &self.statement else {
            return Ok(());
        };
        let (before, script) = (self.lines_before(), self.script_kind());
        let file = self.display_name();
        let statement = by_key(&self.text, &statement.key, before, script, &file)?;
        self.show(statement);
        Ok(())
    }

    /// Shows the statement `statement` of the target's text: the script's value as the target's
    /// text, and another statement as a chart script of its own at [`STATEMENT_PATH`].
    fn show(&mut self, statement: Statement) {
        if statement.shows != Shows::Value {
            let text = statement.text(&self.text);
            let project = self.analysis.project_mut();
            project.add(STATEMENT_PATH, FileRole::Chart, text.as_str());
            self.path = STATEMENT_PATH.to_string();
            self.text = text;
        }
        self.line = Some(statement.line + self.lines_before());
        self.statement = Some(statement);
    }

    /// The lines of the file before the target's text: none for a script, and those of the
    /// document before its cell.
    fn lines_before(&self) -> usize {
        match (&self.document, self.cell()) {
            (Some(document), Some(cell)) => document.text[..cell.range.start].matches('\n').count(),
            _ => 0,
        }
    }

    /// The kind of the target's script.
    fn script_kind(&self) -> Script {
        match self.data {
            true => Script::Data,
            false => Script::Chart,
        }
    }

    /// The name of the target's file, as a window names it: its path in its project, or the name
    /// of a loose file.
    pub(crate) fn display_name(&self) -> String {
        let name = (self.file.file_name()).map(|name| name.to_string_lossy().to_string());
        match (&self.root, name) {
            (Some(_), _) | (None, None) => self.script.clone(),
            (None, Some(name)) => name,
        }
    }

    /// The name of the target's cell, its label, or its number from 1, which finds it again after
    /// its document changes.
    pub(crate) fn cell_name(&self) -> Option<String> {
        let index = self.cell?;
        let cell = self.cell()?;
        Some(
            cell.label
                .clone()
                .unwrap_or_else(|| (index + 1).to_string()),
        )
    }

    /// The window's title of the target: its file, with the line of its statement, its cell, and
    /// what its statement shows, as `overview.qmd:40 · cell 2 · query 1`.
    pub(crate) fn title(&self) -> String {
        let line = self.line.map(|line| format!(":{line}")).unwrap_or_default();
        let mut parts = vec![format!("{}{line}", self.display_name())];
        parts.extend(self.cell_name().map(|name| format!("cell {name}")));
        if let (Some(_), Some(statement)) = (self.line, &self.statement) {
            parts.push(statement.shows.describe());
        }
        parts.join(" · ")
    }

    /// The reports of the error `error` of a run of the target, at their places in its file.
    pub(crate) fn failure(&self, error: DataFusionError) -> Failure {
        let place = |offset| self.to_file(offset);
        Failure::Reports(failure_reports(&error, &self.on_disk(&place)))
    }

    /// The offset in the file on disk of the offset `offset` of the target's text: the same for
    /// a script, in its document for a cell, and the start of the statement for a data script's
    /// statement, whose query the file doesn't hold.
    fn to_file(&self, offset: usize) -> usize {
        match (self.cell(), &self.statement) {
            (_, Some(statement)) if self.data => statement.start,
            (Some(cell), _) => cell.to_document(offset),
            (None, _) => offset,
        }
    }

    /// The cell of the target, for a cell of a document.
    fn cell(&self) -> Option<&Cell> {
        (self.document.as_ref().zip(self.cell)).map(|(document, index)| &document.cells[index])
    }

    /// The text of the target's script in the analysis, which a statement's run keeps.
    fn source(&self) -> &str {
        (self.analysis.project().document(&self.script))
            .map_or(self.text.as_str(), |document| document.text())
    }

    /// The file on disk of the target, whose reports `check` places with `place`.
    fn on_disk<'a>(&'a self, place: Place<'a>) -> File<'a> {
        let text = match &self.document {
            Some(document) => document.text.as_str(),
            None => self.source(),
        };
        File {
            path: &self.file,
            text,
            place,
            first_line: self.cell().map(|cell| first_line(text, cell)),
        }
    }
}

/// Checks the target as `check` does, runs its chart script, and gives its result, or the reports
/// of its errors.
pub(crate) async fn run_target(target: &Target) -> Result<QueryResult, Failure> {
    collect_target(target, plan_target(target).await?).await
}

/// Runs the frame `frame` of the target's chart script, and gives its result, or the reports of
/// its errors.
pub(crate) async fn collect_target(
    target: &Target,
    frame: DataFrame,
) -> Result<QueryResult, Failure> {
    // The columns come from the query's schema, so a result without rows still has its header.
    let schema = frame.schema().inner().clone();
    let batches = (frame.collect().await).map_err(|error| target.failure(error))?;
    QueryResult::new(schema, batches)
        .map_err(|error| Failure::Reports(vec![target.report(error.to_string())]))
}

/// Checks the target as `check` does, and gives the frame of its chart script, unrun, or the
/// reports of its errors. [`Target::failure`] gives the reports of an error of the frame's run.
pub(crate) async fn plan_target(target: &Target) -> Result<DataFrame, Failure> {
    let (analysis, loader) = (&target.analysis, &target.loader);
    // A data script's errors, such as of its load, keep its tables from showing.
    if target.data {
        let same = |offset| offset;
        let file = target.on_disk(&same);
        let reports = check_text(analysis, loader, &target.script, file.text, false, &file).await;
        if (reports.iter()).any(|report| report.severity == Severity::Error) {
            return Err(Failure::Reports(reports));
        }
    }
    // The script's errors, as `check` reports them, keep it from running.
    let place = |offset| target.to_file(offset);
    let file = target.on_disk(&place);
    let reports = check_text(analysis, loader, &target.path, &target.text, false, &file).await;
    if (reports.iter()).any(|report| report.severity == Severity::Error) {
        return Err(Failure::Reports(reports));
    }
    match target.loader.run_chart(&target.path, &target.text).await {
        Ok(frame) => Ok(frame),
        Err(ChartError::Run(error)) | Err(ChartError::Unloaded { error, .. }) => {
            Err(target.failure(error))
        }
    }
}

/// Renders the chart `chart`, a path with a cell's `#NAME` or not, to `out`.
async fn render_target(
    chart: &str,
    out: &Path,
    format: Format,
    options: &Options,
    renderer: &Renderer,
) -> Result<(), Failure> {
    let target = load(chart).await?;
    let result = run_target(&target).await?;
    let cascade = target
        .stylesheet
        .cascade(background(options.color_scheme), renderer.extra.as_ref());
    let single = (result.shape() == Shape::Chart)
        .then(|| result.chart(0, 0))
        .flatten();
    let scene = match (&options.chart, single) {
        (Some(picked), _) => {
            let chart = pick(&result, picked)
                .map_err(|error| Failure::Reports(vec![target.report(error)]))?;
            chart_scene(chart.clone(), &cascade, options, renderer, options.size).await?
        }
        (None, Some(chart)) => {
            chart_scene(chart.clone(), &cascade, options, renderer, options.size).await?
        }
        (None, None) => table_scene(&result, &cascade, options, renderer).await?,
    };
    write(&scene, out, format, options, renderer)
        .await
        .map_err(Failure::Usage)
}

/// Loads the project of the chart `chart`, a path of a chart script or `doc.qmd#NAME`, or a
/// loose script or document as a project of one file, and finds its text. A target at a line,
/// `FILE:LINE`, names the statement at a line of a script or of a document's cell.
pub(crate) async fn load(chart: &str) -> Result<Target, Failure> {
    let (chart, line) = split_line(chart)?;
    let (path, name) = match chart.rsplit_once('#') {
        Some((document, name)) if is_document(document) => (document, Some(name)),
        _ => (chart, None),
    };
    if let (Some(name), Some(_)) = (name, line) {
        return Err(Failure::Usage(format!(
            "{chart}: a target names a cell by its line or as `#{name}`, not both"
        )));
    }
    let file = std::fs::canonicalize(path).map_err(|error| format!("{path}: {error}"))?;
    let (files, relative, root) = match find_root(&file) {
        Some(root) => {
            let files = (ProjectFiles::find(&root).await)
                .map_err(|e| format!("{}: {e}", root.display()))?;
            let relative = (file.strip_prefix(&root).ok())
                .and_then(|path| path.to_str())
                .map(|path| path.replace('\\', "/"))
                .unwrap_or_default();
            let role = files.manifest().role(&relative);
            match role {
                Ok(Some(Role::ChartScript | Role::Document | Role::DataScript)) => {
                    (files, relative, Some(root))
                }
                Ok(Some(Role::Module(_))) if line.is_some() => {
                    return Err(Failure::Usage(format!(
                        "{}: a statement of a module is no target",
                        file.display()
                    )))
                }
                _ => {
                    let (files, relative) = loose(&file).await?;
                    (files, relative, None)
                }
            }
        }
        None => {
            let (files, relative) = loose(&file).await?;
            (files, relative, None)
        }
    };
    let data = matches!(files.manifest().role(&relative), Ok(Some(Role::DataScript)));
    let (analysis, loader, _) = load_files(&files).await?;
    let stylesheet = files.load_stylesheet().await.map_err(|e| e.to_string())?;
    let (path, text, document, cell) = match analysis.qmd(&relative) {
        Some(document) => {
            let (index, cell) = match (name, line) {
                (_, Some(line)) => (cell_at_line(document, line))
                    .map_err(|error| format!("{chart}:{line}: {error}"))?,
                (Some(name), None) => document.cell(name).ok_or_else(|| {
                    format!("{}: no `{{avenger-sql}}` cell `{name}`", file.display())
                })?,
                (None, None) if document.cells.len() == 1 => (0, &document.cells[0]),
                (None, None) => {
                    return Err(Failure::Usage(format!(
                        "{}: the document has {} cells; name one as `#NUMBER` or `#LABEL`, or \
                         by a line as `:LINE`",
                        file.display(),
                        document.cells.len()
                    )))
                }
            };
            let text = cell.text.clone();
            (
                cell_path(&relative, index + 1),
                text,
                Some(document.clone()),
                Some(index),
            )
        }
        None => {
            let text = (analysis.project().document(&relative))
                .map(|document| document.text().to_string())
                .ok_or_else(|| format!("{}: not a chart script", file.display()))?;
            (relative.clone(), text, None, None)
        }
    };
    let mut target = Target {
        loader,
        analysis,
        stylesheet,
        path,
        text,
        script: relative,
        file,
        document,
        cell,
        files,
        root,
        line: None,
        statement: None,
        data,
    };
    if let (true, None) = (data, line) {
        // A data script has no value: its tables show by their lines.
        let tables: Vec<String> = (data_tables(&target.text).into_iter())
            .map(|(table, line)| format!("{chart}:{line} for `{table}`"))
            .collect();
        let tables = match tables.is_empty() {
            true => "it makes no table that a target shows".to_string(),
            false => format!("name a table by its line: {}", tables.join(", ")),
        };
        return Err(Failure::Usage(format!(
            "{chart}: a data script has no value; {tables}"
        )));
    }
    if let Some(line) = line {
        (target.point_at(line)).map_err(|error| format!("{chart}:{line}: {error}"))?;
    }
    Ok(target)
}

/// A project of the one file `file`, a loose script or document, and its path in the project:
/// a script under `charts/`, and a document at the root.
async fn loose(file: &Path) -> Result<(ProjectFiles, String), Failure> {
    let text =
        std::fs::read_to_string(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let name = (file.file_name().and_then(|name| name.to_str())).unwrap_or("chart.av.sql");
    let relative = match is_document(name) {
        true => name.to_string(),
        false => format!("charts/{name}"),
    };
    let source = FileSource::embedded(
        "loose",
        [
            (
                "avenger.toml".to_string(),
                "[project]\nname = \"loose\"\n".to_string(),
            ),
            (relative.clone(), text),
        ],
    )
    .await
    .map_err(|error| error.to_string())?;
    let files = ProjectFiles::open(source)
        .await
        .map_err(|error| error.to_string())?;
    Ok((files, relative))
}

/// A cell of a result, as a table shows it: a chart, a value as text, or a null.
enum Value<'a> {
    Chart(&'a Chart),
    Text(String),
    Null,
}

/// The cell of `result` at `row` and `column`.
fn value(result: &QueryResult, row: usize, column: usize) -> Value<'_> {
    match result.is_chart_column(column) {
        true => result.chart(row, column).map_or(Value::Null, Value::Chart),
        false => result
            .value_text(row, column)
            .map_or(Value::Null, Value::Text),
    }
}

/// The chart of `result` that `pick`, `ROW[.COLUMN]`, names: its row from 1, and its column by
/// name, or the only column of charts.
fn pick<'a>(result: &'a QueryResult, pick: &str) -> Result<&'a Chart, String> {
    let (row, column) = match pick.split_once('.') {
        Some((row, column)) => (row, Some(column)),
        None => (pick, None),
    };
    let rows = result.num_rows();
    let row: usize = (row.parse().ok())
        .filter(|row| (1..=rows).contains(row))
        .ok_or_else(|| format!("`--chart {pick}`: the rows are 1 to {rows}"))?;
    let fields = result.schema().fields();
    let charts = result.chart_columns();
    let index = match column {
        Some(name) => (charts.iter().copied())
            .find(|index| fields[*index].name() == name)
            .ok_or_else(|| format!("`--chart {pick}`: no column of charts `{name}`"))?,
        None if charts.len() == 1 => charts[0],
        None if charts.is_empty() => {
            return Err(format!(
                "`--chart {pick}`: the result has no column of charts"
            ))
        }
        None => {
            let names: Vec<&str> = charts.iter().map(|i| fields[*i].name().as_str()).collect();
            return Err(format!(
                "`--chart {pick}`: name a column of charts, one of {}",
                names.join(", ")
            ));
        }
    };
    (result.chart(row - 1, index))
        .ok_or_else(|| format!("`--chart {pick}`: the cell holds no chart"))
}

/// The natural size of `chart`, at the sides of `size` that are set.
fn natural_size(
    chart: &Chart,
    cascade: &Arc<Cascade>,
    options: &Options,
    renderer: &Renderer,
    size: [Option<f32>; 2],
) -> Result<[f32; 2], Failure> {
    let mut evaluator = ChartEvaluator::new(
        chart.clone(),
        cascade.clone(),
        renderer.engine.clone(),
        Registry::new(),
    )
    .map_err(|error| error.to_string())?;
    let host = HostInputs {
        width: size[0],
        height: size[1],
        preferences: preferences(options.color_scheme),
        theme: None,
    };
    let evaluated = evaluator
        .evaluate(&host)
        .map_err(|error| error.to_string())?;
    Ok([evaluated.scene.width, evaluated.scene.height])
}

/// The marks of `chart` drawn by its app at `size`, or at its natural size at the sides that
/// `size` leaves unset, and the size.
async fn chart_marks(
    chart: Chart,
    cascade: &Arc<Cascade>,
    options: &Options,
    renderer: &Renderer,
    size: [Option<f32>; 2],
) -> Result<(Vec<SceneMark>, [f32; 2]), Failure> {
    let size = match size {
        [Some(width), Some(height)] => [width, height],
        size => natural_size(&chart, cascade, options, renderer, size)?,
    };
    let modes: Vec<String> = (chart.message().modes.iter())
        .map(|mode| mode.name.clone())
        .collect();
    for mode in &options.modes {
        if !modes.contains(mode) {
            return Err(Failure::Usage(match modes.is_empty() {
                true => format!("the chart has no modes, so no mode `{mode}`"),
                false => format!(
                    "the chart has no mode `{mode}`; its modes are {}",
                    modes.join(", ")
                ),
            }));
        }
    }
    let updates = ChartUpdates::new();
    updates.set_preferences(preferences(options.color_scheme));
    for mode in &options.modes {
        updates.set_mode(mode.clone(), true);
    }
    let mut app = ChartApp::new(chart, cascade.clone(), renderer.engine.clone())
        .images(renderer.images.clone())
        .updates(&updates)
        .size(size)
        .build()
        .await
        .map_err(|error| error.to_string())?
        .with_runtime(renderer.runtime.clone())
        .map_err(|error| error.to_string())?;
    // The app applies the modes and the preferences, then waits for the images of its scenes,
    // which can draw more images, such as the tiles of a map at a new zoom.
    let mut waited = Vec::new();
    for _ in 0..8 {
        app.refresh().await;
        for _ in 0..200 {
            if !updates.pending() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            app.refresh().await;
        }
        if let Some(error) = updates.error() {
            return Err(Failure::Usage(error.to_string()));
        }
        let urls: Vec<String> = (app.scenes().iter())
            .flat_map(|scene| {
                scene
                    .image_urls()
                    .into_iter()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter(|url| !waited.contains(url))
            .collect();
        if urls.is_empty() {
            break;
        }
        let refs: Vec<&str> = urls.iter().map(String::as_str).collect();
        if let Err(timeout) = renderer.images.wait(&refs, options.timeout) {
            eprintln!(
                "warning: {} images did not load in {:?} and draw without their pictures",
                timeout.pending.len(),
                options.timeout
            );
            break;
        }
        waited.extend(urls);
    }
    let marks = (app.scenes().iter())
        .flat_map(|scene| scene.marks.clone())
        .collect();
    Ok((marks, size))
}

/// The scene of `chart` alone.
async fn chart_scene(
    chart: Chart,
    cascade: &Arc<Cascade>,
    options: &Options,
    renderer: &Renderer,
    size: [Option<f32>; 2],
) -> Result<SceneGraph, Failure> {
    let (marks, [width, height]) = chart_marks(chart, cascade, options, renderer, size).await?;
    Ok(SceneGraph {
        width,
        height,
        origin: [0.0, 0.0],
        marks,
    })
}

/// The padding of a cell of a table: on each side of a column, and above and below a row of
/// charts.
const PADDING: f32 = 8.0;
/// The padding above and below a row of text alone, and the header, so that rows of text read
/// as the lines of a table.
const TEXT_PADDING: f32 = 4.0;
/// The widest text column of a table.
const TEXT_WIDTH: f32 = 300.0;
/// The size of the text of a table.
const FONT_SIZE: f32 = 12.0;

/// The scene of a table of charts: a header of the column names, then each row, its charts at
/// their cells' sizes and its other values as text.
async fn table_scene(
    result: &QueryResult,
    cascade: &Arc<Cascade>,
    options: &Options,
    renderer: &Renderer,
) -> Result<SceneGraph, Failure> {
    let shown = result.num_rows().min(options.rows.max(1));
    let names: Vec<&str> = (result.schema().fields().iter())
        .map(|f| f.name().as_str())
        .collect();
    // The cells of the shown rows, by row and column.
    let rows: Vec<Vec<Value>> = (0..shown)
        .map(|row| {
            (0..names.len())
                .map(|column| value(result, row, column))
                .collect()
        })
        .collect();
    let engine = &renderer.engine;
    let measure = |text: &str, bold: bool| -> [f32; 2] {
        let style = TextStyle {
            font_size: FONT_SIZE,
            font_weight: match bold {
                true => avenger_typst_label::FontWeight::BOLD,
                false => avenger_typst_label::FontWeight::REGULAR,
            },
            ..TextStyle::default()
        };
        let label = LabelOptions {
            text: style,
            width: LabelWidth::Max(TEXT_WIDTH),
            max_lines: std::num::NonZeroUsize::new(1),
            ellipsis: true,
            ..LabelOptions::default()
        };
        (engine.measure_text(text, &label)).map_or([0.0, FONT_SIZE], |m| [m.width, m.height])
    };
    // A row of text is one line tall: the height of a line of the table's font, as the label
    // engine lays out one line, which is the same for every text, whatever its letters.
    let line_height = measure("Mg", false)[1];
    // The size of each column and row: the natural sizes of the charts, or `--size`, and the
    // widths of the texts.
    let mut widths: Vec<f32> = names.iter().map(|name| measure(name, true)[0]).collect();
    let mut heights = vec![line_height; rows.len()];
    let mut pads = vec![TEXT_PADDING; rows.len()];
    for (r, row) in rows.iter().enumerate() {
        for (c, value) in row.iter().enumerate() {
            let [width, height] = match value {
                Value::Chart(chart) => {
                    pads[r] = PADDING;
                    match options.size {
                        [Some(width), Some(height)] => [width, height],
                        size => natural_size(chart, cascade, options, renderer, size)?,
                    }
                }
                Value::Text(text) => [measure(text, false)[0], line_height],
                Value::Null => [measure("NULL", false)[0], line_height],
            };
            widths[c] = widths[c].max(width);
            heights[r] = heights[r].max(height);
        }
    }
    let width = widths.iter().map(|w| w + 2.0 * PADDING).sum::<f32>();
    let header_height = line_height + 2.0 * TEXT_PADDING;
    let rows_height: f32 = (heights.iter().zip(&pads))
        .map(|(h, pad)| h + 2.0 * pad)
        .sum();
    let mut height = header_height + rows_height;
    let more = result.num_rows() - shown;
    if more > 0 || rows.is_empty() {
        height += header_height;
    }
    let (color, rule) = match options.color_scheme {
        ColorScheme::Dark => ([0.88, 0.88, 0.88, 1.0], [1.0, 1.0, 1.0, 0.15]),
        ColorScheme::Light => ([0.13, 0.13, 0.13, 1.0], [0.0, 0.0, 0.0, 0.12]),
    };
    let mut marks = Vec::new();
    let mut texts: Vec<(f32, f32, String, bool)> = Vec::new();
    let mut x = 0.0;
    for (c, name) in names.iter().enumerate() {
        texts.push((x + PADDING, TEXT_PADDING, name.to_string(), true));
        x += widths[c] + 2.0 * PADDING;
    }
    let mut y = header_height;
    let mut rules = vec![y];
    for (r, row) in rows.iter().enumerate() {
        let mut x = 0.0;
        let pad = pads[r];
        for (c, value) in row.iter().enumerate() {
            let cell = [widths[c], heights[r]];
            match value {
                Value::Chart(chart) => {
                    let size = [Some(cell[0]), Some(cell[1])];
                    let (chart_marks, _) =
                        chart_marks((*chart).clone(), cascade, options, renderer, size).await?;
                    marks.push(SceneMark::Group(SceneGroup {
                        name: format!("cell-{}-{}", r + 1, c + 1),
                        origin: [x + PADDING, y + pad],
                        marks: chart_marks,
                        ..Default::default()
                    }));
                }
                Value::Text(text) => texts.push((x + PADDING, y + pad, text.clone(), false)),
                Value::Null => texts.push((x + PADDING, y + pad, "NULL".into(), false)),
            }
            x += widths[c] + 2.0 * PADDING;
        }
        y += heights[r] + 2.0 * pad;
        rules.push(y);
    }
    if rows.is_empty() {
        texts.push((PADDING, y + TEXT_PADDING, "No rows".to_string(), false));
    }
    if more > 0 {
        let note = format!("{more} more rows; `--rows` shows more");
        texts.push((PADDING, y + TEXT_PADDING, note, false));
        eprintln!(
            "warning: the image shows {shown} of {} rows",
            result.num_rows()
        );
    }
    marks.push(SceneMark::Rule(SceneRuleMark {
        len: rules.len() as u32,
        x: 0.0.into(),
        x2: width.into(),
        y: rules.clone().into(),
        y2: rules.into(),
        stroke: avenger_color::ColorOrGradient::Color(rule).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }));
    marks.push(SceneMark::Text(Arc::new(SceneTextMark {
        len: texts.len() as u32,
        x: texts.iter().map(|t| t.0).collect::<Vec<_>>().into(),
        y: texts.iter().map(|t| t.1).collect::<Vec<_>>().into(),
        text: texts.iter().map(|t| t.2.clone()).collect::<Vec<_>>().into(),
        font_weight: (texts.iter())
            .map(|t| match t.3 {
                true => avenger_common::types::FontWeight::BOLD,
                false => avenger_common::types::FontWeight::NORMAL,
            })
            .collect::<Vec<_>>()
            .into(),
        // Each text is as wide as it measured, up to the widest column of text, where it ends in an
        // ellipsis, so a text that fits its column never cuts short.
        width: LabelWidth::Max(TEXT_WIDTH).into(),
        max_lines: std::num::NonZeroUsize::new(1),
        ellipsis: true,
        baseline: avenger_common::types::TextBaseline::Top.into(),
        color: color.into(),
        font_size: FONT_SIZE.into(),
        ..Default::default()
    })));
    Ok(SceneGraph {
        width,
        height,
        origin: [0.0, 0.0],
        marks,
    })
}

/// Writes `scene` to `out` in `format`, on the background of the color scheme.
async fn write(
    scene: &SceneGraph,
    out: &Path,
    format: Format,
    options: &Options,
    renderer: &Renderer,
) -> Result<(), String> {
    let error = |error: &dyn std::fmt::Display| format!("{}: {error}", out.display());
    let background = color(background(options.color_scheme));
    let images: Arc<dyn ImageResolver> = Arc::new(renderer.images.clone());
    match format {
        Format::Png => {
            let dimensions = CanvasDimensions {
                size: [scene.width, scene.height],
                scale: options.scale,
            };
            let config = CanvasConfig {
                images: Some(images),
                ..Default::default()
            };
            let mut canvas = (PngCanvas::new(dimensions, config).await)
                .map_err(|e| format!("{e}; without a GPU, `.svg` renders without one"))?;
            canvas.set_background(background);
            (canvas.set_scene(scene, &renderer.engine)).map_err(|e| e.to_string())?;
            let image = canvas.render().await.map_err(|e| e.to_string())?;
            image.save(out).map_err(|e| error(&e))
        }
        Format::Svg => {
            let svg = avenger_svg::SvgRenderer::new()
                .with_options(avenger_svg::SvgRenderOptions {
                    background: Some(background),
                    allow_unready_images: true,
                    ..Default::default()
                })
                .with_images(images)
                .render_scene_graph(scene, &renderer.engine)
                .map_err(|e| error(&e))?;
            std::fs::write(out, svg).map_err(|e| error(&e))
        }
        Format::Pdf => {
            let pdf = avenger_pdf::PdfRenderer::new()
                .with_options(avenger_pdf::PdfRenderOptions {
                    background: Some(background),
                    allow_unready_images: true,
                })
                .with_images(images)
                .render_scene_graph(scene, &renderer.engine)
                .map_err(|e| error(&e))?;
            std::fs::write(out, pdf).map_err(|e| error(&e))
        }
    }
}

/// The sRGB RGBA of a CSS color `#rrggbb`.
fn color(css: &str) -> [f32; 4] {
    let hex = css.trim_start_matches('#');
    let channel = |at: usize| {
        (hex.get(at..at + 2))
            .and_then(|digits| u8::from_str_radix(digits, 16).ok())
            .map_or(1.0, |value| f32::from(value) / 255.0)
    };
    [channel(0), channel(2), channel(4), 1.0]
}
