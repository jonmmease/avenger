//! `avenger render`: the chart of a chart script, or of a cell of a Quarto document, drawn to
//! PNG, SVG, or PDF by the same chart app as the editor, offscreen, with its widgets.
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

use avenger_chart::{charts_from_arrow, Chart, Decode, EXTENSION_NAME};
use avenger_chart_app::{ChartApp, ChartUpdates};
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_common::canvas::CanvasDimensions;
use avenger_image::{ImageCache, ImageCacheOptions, ImageResolver};
use avenger_scenegraph::{
    marks::{group::SceneGroup, mark::SceneMark, rule::SceneRuleMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_sql_project::{
    charts::{background, label_engine, preferences},
    find_root,
    qmd::{cell_path, is_document, Qmd},
    Analysis, ChartError, FileSource, Loader, ProjectFiles, ProjectStylesheet, Role,
};
use avenger_style::{Cascade, ColorScheme, HostInputs, ParseOptions, Sheet, SourceId};
use avenger_typst_label::{LabelEngine, LabelOptions, LabelWidth, TextStyle};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{
    arrow::{array::RecordBatch, datatypes::Schema, util::display::array_value_to_string},
    common::DataFusionError,
    prelude::SessionContext,
};

use crate::{
    check::{check_text, failure_reports, first_line, same, File},
    report::{Output, Report, Severity},
    Status,
};

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
enum Failure {
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
    let mut targets: Vec<String> = files.chart_scripts().to_vec();
    for (document, qmd) in analysis.documents() {
        targets.extend((1..=qmd.cells.len()).map(|number| cell_path(document, number)));
    }
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
fn image_options() -> Result<ImageCacheOptions, String> {
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
struct Target {
    loader: Loader,
    analysis: Analysis,
    stylesheet: ProjectStylesheet,
    path: String,
    text: String,
    /// The file on disk, and for a cell, the text of its document.
    file: PathBuf,
    document: Option<Qmd>,
    cell: Option<usize>,
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
    let file_text = match &target.document {
        Some(document) => document.text.as_str(),
        None => target.text.as_str(),
    };
    let cell = target
        .document
        .as_ref()
        .zip(target.cell)
        .map(|(d, i)| &d.cells[i]);
    let to_document = |offset| cell.map_or(offset, |cell| cell.to_document(offset));
    let file = File {
        path: &target.file,
        text: file_text,
        place: match cell {
            Some(_) => &to_document,
            None => &same,
        },
        first_line: cell.map(|cell| first_line(file_text, cell)),
    };
    // The script's errors, as `check` reports them, keep it from running.
    let reports = check_text(
        &target.analysis,
        &target.loader,
        &target.path,
        &target.text,
        false,
        &file,
    )
    .await;
    if reports
        .iter()
        .any(|report| report.severity == Severity::Error)
    {
        return Err(Failure::Reports(reports));
    }
    let failed = |error: DataFusionError| Failure::Reports(failure_reports(&error, &file));
    let frame = match target.loader.run_chart(&target.path, &target.text).await {
        Ok(frame) => frame,
        Err(ChartError::Run(error)) | Err(ChartError::Unloaded { error, .. }) => {
            return Err(failed(error))
        }
    };
    // The columns come from the query's schema, so a result without rows still has its header.
    let schema = frame.schema().inner().clone();
    let batches = frame.collect().await.map_err(failed)?;
    let table = ResultTable::of(&schema, &batches)
        .map_err(|error| Failure::Reports(vec![report(&file, error)]))?;
    let cascade = target
        .stylesheet
        .cascade(background(options.color_scheme), renderer.extra.as_ref());
    let scene = match (&options.chart, table.single()) {
        (Some(pick), _) => {
            let chart = table
                .pick(pick)
                .map_err(|error| Failure::Reports(vec![report(&file, error)]))?;
            chart_scene(chart.clone(), &cascade, options, renderer, options.size).await?
        }
        (None, Some(chart)) => {
            chart_scene(chart.clone(), &cascade, options, renderer, options.size).await?
        }
        (None, None) => table_scene(&table, &cascade, options, renderer).await?,
    };
    write(&scene, out, format, options, renderer)
        .await
        .map_err(Failure::Usage)
}

/// The report of the error `message` of the whole file of `file`, or of its cell's first line.
fn report(file: &File, message: String) -> Report {
    match file.first_line {
        Some(line) => Report::at(
            file.path,
            file.text,
            line,
            Severity::Error,
            message,
            "render",
        ),
        None => Report::file(file.path, Severity::Error, message, "render"),
    }
}

/// Loads the project of the chart `chart`, a path of a chart script or `doc.qmd#NAME`, or a
/// loose script or document as a project of one file, and finds its text.
async fn load(chart: &str) -> Result<Target, Failure> {
    let (path, name) = match chart.rsplit_once('#') {
        Some((document, name)) if is_document(document) => (document, Some(name)),
        _ => (chart, None),
    };
    let file = std::fs::canonicalize(path).map_err(|error| format!("{path}: {error}"))?;
    let (files, relative) = match find_root(&file) {
        Some(root) => {
            let files = (ProjectFiles::find(&root).await)
                .map_err(|e| format!("{}: {e}", root.display()))?;
            let relative = (file.strip_prefix(&root).ok())
                .and_then(|path| path.to_str())
                .map(|path| path.replace('\\', "/"))
                .unwrap_or_default();
            let role = files.manifest().role(&relative);
            match role {
                Ok(Some(Role::ChartScript | Role::Document)) => (files, relative),
                _ => loose(&file).await?,
            }
        }
        None => loose(&file).await?,
    };
    let mut analysis = Analysis::new(&files).await.map_err(|e| e.to_string())?;
    let stylesheet = files.load_stylesheet().await.map_err(|e| e.to_string())?;
    let (loader, state) =
        (Loader::load(files, SessionContext::new()).await).map_err(|e| e.to_string())?;
    analysis.apply(&state);
    let (path, text, document, cell) = match analysis.qmd(&relative) {
        Some(document) => {
            let (index, cell) = match name {
                Some(name) => document.cell(name).ok_or_else(|| {
                    format!("{}: no `{{avenger-sql}}` cell `{name}`", file.display())
                })?,
                None if document.cells.len() == 1 => (0, &document.cells[0]),
                None => {
                    return Err(Failure::Usage(format!(
                        "{}: the document has {} cells; name one as `#NUMBER` or `#LABEL`",
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
            (relative, text, None, None)
        }
    };
    Ok(Target {
        loader,
        analysis,
        stylesheet,
        path,
        text,
        file,
        document,
        cell,
    })
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

/// A cell of a result: a chart, a value as text, or a null.
enum Value {
    Chart(Box<Chart>),
    Text(String),
    Null,
}

/// The result of a chart script: its columns, whether each holds charts, and its rows.
struct ResultTable {
    columns: Vec<(String, bool)>,
    rows: Vec<Vec<Value>>,
}

impl ResultTable {
    /// The table of `batches`, of the columns of `schema`.
    fn of(schema: &Schema, batches: &[RecordBatch]) -> Result<ResultTable, String> {
        let columns: Vec<(String, bool)> = (schema.fields().iter())
            .map(|field| {
                let chart = field
                    .metadata()
                    .get("ARROW:extension:name")
                    .map(String::as_str)
                    == Some(EXTENSION_NAME);
                (field.name().clone(), chart)
            })
            .collect();
        let mut rows = Vec::new();
        for batch in batches {
            let mut cells: Vec<Vec<Value>> = (0..batch.num_rows()).map(|_| Vec::new()).collect();
            for (index, (_, chart)) in columns.iter().enumerate() {
                let array = batch.column(index);
                match chart {
                    true => {
                        let charts = charts_from_arrow(array.as_ref(), Decode::Auto)
                            .map_err(|error| error.to_string())?;
                        for (row, chart) in charts.into_iter().enumerate() {
                            cells[row].push(
                                chart.map_or(Value::Null, |chart| Value::Chart(Box::new(chart))),
                            );
                        }
                    }
                    false => {
                        for (row, cells) in cells.iter_mut().enumerate() {
                            cells.push(match array.is_null(row) {
                                true => Value::Null,
                                false => Value::Text(
                                    array_value_to_string(array, row).unwrap_or_default(),
                                ),
                            });
                        }
                    }
                }
            }
            rows.extend(cells);
        }
        Ok(ResultTable { columns, rows })
    }

    /// The chart of a result of one row and one column, which holds it.
    fn single(&self) -> Option<&Chart> {
        match (self.rows.as_slice(), self.columns.len()) {
            ([row], 1) => match &row[0] {
                Value::Chart(chart) => Some(chart),
                _ => None,
            },
            _ => None,
        }
    }

    /// The chart that `pick`, `ROW[.COLUMN]`, names: its row from 1, and its column by name, or
    /// the only column of charts.
    fn pick(&self, pick: &str) -> Result<&Chart, String> {
        let (row, column) = match pick.split_once('.') {
            Some((row, column)) => (row, Some(column)),
            None => (pick, None),
        };
        let row: usize = row
            .parse()
            .ok()
            .filter(|row| (1..=self.rows.len()).contains(row))
            .ok_or_else(|| format!("`--chart {pick}`: the rows are 1 to {}", self.rows.len()))?;
        let charts: Vec<usize> = (self.columns.iter().enumerate())
            .filter(|(_, (_, chart))| *chart)
            .map(|(index, _)| index)
            .collect();
        let index = match column {
            Some(name) => (charts.iter().copied())
                .find(|index| self.columns[*index].0 == name)
                .ok_or_else(|| format!("`--chart {pick}`: no column of charts `{name}`"))?,
            None if charts.len() == 1 => charts[0],
            None if charts.is_empty() => {
                return Err(format!(
                    "`--chart {pick}`: the result has no column of charts"
                ))
            }
            None => {
                let names: Vec<&str> = charts.iter().map(|i| self.columns[*i].0.as_str()).collect();
                return Err(format!(
                    "`--chart {pick}`: name a column of charts, one of {}",
                    names.join(", ")
                ));
            }
        };
        match &self.rows[row - 1][index] {
            Value::Chart(chart) => Ok(chart),
            _ => Err(format!("`--chart {pick}`: the cell holds no chart")),
        }
    }
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
    table: &ResultTable,
    cascade: &Arc<Cascade>,
    options: &Options,
    renderer: &Renderer,
) -> Result<SceneGraph, Failure> {
    let shown = table.rows.len().min(options.rows.max(1));
    let rows = &table.rows[..shown];
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
    let mut widths: Vec<f32> = (table.columns.iter())
        .map(|(name, _)| measure(name, true)[0])
        .collect();
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
    let more = table.rows.len() - shown;
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
    for (c, (name, _)) in table.columns.iter().enumerate() {
        texts.push((x + PADDING, TEXT_PADDING, name.clone(), true));
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
                        chart_marks(chart.as_ref().clone(), cascade, options, renderer, size)
                            .await?;
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
            table.rows.len()
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
