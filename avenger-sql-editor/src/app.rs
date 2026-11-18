//! The editor's window: the data sidebar on the left, the query editor over the result pane on the
//! right, and a status bar under them.

use avenger_common::time::{Duration, Instant};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use avenger_style::Cascade;
use avenger_typst_label::LabelEngine;
use datafusion::prelude::SessionContext;
use eframe::{egui, egui_wgpu::RenderState};

use crate::cards::{Gallery, THUMBNAILS};
use crate::charts;
use crate::document::{self, Analyzed};
use crate::editor::{self, CodeEditor};
use crate::exec::{Exec, Task};
use crate::format;
use crate::gallery::{examples, Example};
use crate::highlight;
use crate::live::LiveChart;
use crate::query::{self, Functions, Outcome, QueryError};
use crate::result::{QueryResult, Shape};
use crate::samples::{self, Sample};
use crate::sidebar;
use crate::sources::{table_name, FileKind, Source};
use crate::table::{CellKey, CellScene, Cells, Env, Grid, LiveCells};

/// How long a query runs before the status bar offers to cancel it.
const CANCEL_AFTER: Duration = Duration::from_millis(200);
/// The longest that `--screenshot` waits for the window to settle.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);

/// The command-line options of the editor.
#[derive(Debug, Default)]
pub struct Options {
    /// The CSV and Parquet files to register at startup.
    pub files: Vec<PathBuf>,
    /// A file whose text the editor opens, and runs once the files are registered.
    pub query: Option<PathBuf>,
    /// Where to save a PNG of the window once it settles, after which the editor exits.
    pub screenshot: Option<PathBuf>,
    /// The character index where `--cursor` focuses the editor, before a screenshot.
    pub cursor: Option<usize>,
    /// The keys that each `--press <KEYS>` presses after `--cursor`, in order, such as
    /// `alt+Space` or `cmd+Period`: modifiers `alt`, `ctrl`, `shift`, and `cmd`, and egui's
    /// name of a key.
    pub presses: Vec<(egui::Modifiers, egui::Key)>,
    /// Whether `--hover` holds the pointer over the character after the cursor.
    pub hover: bool,
}

impl Options {
    /// The options of `args`, which exclude the program name: file paths, `--query <FILE>`,
    /// `--screenshot <PNG>`, and the options that drive the editor before a screenshot,
    /// `--cursor <INDEX>`, `--press <KEYS>`, and `--hover`.
    pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let mut value = |flag: &str| {
                args.next()
                    .map(PathBuf::from)
                    .ok_or_else(|| format!("{flag} needs a path"))
            };
            match arg.as_str() {
                "--query" => options.query = Some(value("--query")?),
                "--screenshot" => options.screenshot = Some(value("--screenshot")?),
                "--cursor" => {
                    let index = value("--cursor")?;
                    let index = index.to_string_lossy().parse();
                    options.cursor = Some(index.map_err(|_| "--cursor needs an index")?);
                }
                "--press" => {
                    let keys = value("--press")?.to_string_lossy().into_owned();
                    options.presses.push(press(&keys)?);
                }
                "--hover" => options.hover = true,
                flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
                _ => options.files.push(PathBuf::from(arg)),
            }
        }
        Ok(options)
    }
}

/// The modifiers and the key of `keys`, such as `alt+Space`.
fn press(keys: &str) -> Result<(egui::Modifiers, egui::Key), String> {
    let mut modifiers = egui::Modifiers::NONE;
    let mut parts: Vec<&str> = keys.split('+').collect();
    let name = parts.pop().unwrap_or_default();
    for part in parts {
        modifiers |= match part {
            "alt" => egui::Modifiers::ALT,
            "ctrl" => egui::Modifiers::CTRL,
            "shift" => egui::Modifiers::SHIFT,
            "cmd" => egui::Modifiers::COMMAND,
            _ => return Err(format!("unknown modifier {part} in --press {keys}")),
        };
    }
    let key = egui::Key::from_name(name).ok_or_else(|| format!("unknown key in --press {keys}"))?;
    Ok((modifiers, key))
}

/// What the background tasks send back to the window.
enum Message {
    Registered {
        name: String,
        source: Result<Source, String>,
        path: PathBuf,
    },
    Ran {
        id: u64,
        outcome: Result<Outcome, QueryError>,
        elapsed: Duration,
    },
    /// The lifted form of the query of a run, after its result.
    Lifted {
        id: u64,
        lifted: Result<query::LiftedText, QueryError>,
    },
    Samples(Result<Vec<Sample>, String>),
    /// The result of the query of a card of the gallery.
    Example {
        index: usize,
        result: Result<QueryResult, String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Result,
    Examples,
    Lifted,
    Plan,
}

/// The query that runs.
struct Running {
    id: u64,
    task: Task,
    started: Instant,
}

/// The last result of a query that ran.
struct Shown {
    result: QueryResult,
    plan: String,
    elapsed: Duration,
}

struct Screenshot {
    path: PathBuf,
    started: Instant,
    idle_frames: u32,
    requested: bool,
    /// The steps of `--cursor`, `--press`, and `--hover`, which run once the window settles.
    drive: Option<Drive>,
}

/// The editor's state before a screenshot: the cursor, the keys that it presses one frame apart,
/// and whether the pointer then holds over the character after the cursor.
struct Drive {
    cursor: usize,
    presses: Vec<(egui::Modifiers, egui::Key)>,
    hover: bool,
    frame: u32,
    /// When the pointer moved over the character, for the tooltip's delay.
    hovered: Option<Instant>,
    /// The events of the next frame.
    events: Vec<egui::Event>,
}

impl Drive {
    /// Takes the drive's next step after the editor's last frame, and returns whether the
    /// editor is ready for the screenshot.
    fn step(&mut self, ctx: &egui::Context, editor: &CodeEditor) -> bool {
        self.frame += 1;
        // The cursor at frame 1, a press each second frame from frame 3, and then the hover.
        let hover = 3 + 2 * self.presses.len() as u32;
        match self.frame {
            1 => editor.place(ctx, self.cursor),
            frame if frame >= 3 && frame < hover && frame % 2 == 1 => {
                let (modifiers, key) = self.presses[(frame as usize - 3) / 2];
                self.events
                    .extend([true, false].map(|pressed| egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers,
                    }));
            }
            frame if frame == hover && self.hover => {
                if let Some(caret) = editor.caret() {
                    self.events.push(egui::Event::PointerMoved(caret));
                    self.hovered = Some(Instant::now());
                }
            }
            _ => {}
        }
        let held = (self.hovered).is_none_or(|at| at.elapsed() > Duration::from_secs(1));
        self.frame > hover + 4 && held
    }
}

pub struct EditorApp {
    runtime: Exec,
    session: SessionContext,
    functions: Functions,
    cascade: Arc<Cascade>,
    engine: LabelEngine,
    text: String,
    editor: CodeEditor,
    /// The analysis of `text`, which colors it, underlines its errors, and answers the editor.
    analyzed: Analyzed,
    title: String,
    sources: Vec<Source>,
    pending: BTreeSet<String>,
    /// The sample tables, once the session holds them.
    samples: Option<Vec<Sample>>,
    gallery: Gallery,
    run_when_ready: bool,
    messages: (Sender<Message>, Receiver<Message>),
    scenes: (Sender<CellScene>, Receiver<CellScene>),
    next_id: u64,
    running: Option<Running>,
    shown: Option<Shown>,
    /// The lifted form of the query of the last run, once it arrives.
    lifted: Option<Result<query::LiftedText, QueryError>>,
    error: Option<QueryError>,
    notice: Option<String>,
    tab: Tab,
    cells: Cells,
    /// The live chart of a result that is a single chart, which fills the pane.
    pane: Pane,
    /// A chart cell that the user expanded, which fills the pane over the table until Back.
    opened: Option<(CellKey, Pane)>,
    /// The live charts of the table's chart cells, while `interactive` is on.
    live_cells: LiveCells,
    /// Whether the table's charts take the pointer and the wheel, to pan, zoom, and select. Off,
    /// the charts are images, and the wheel scrolls the table.
    interactive: bool,
    screenshot: Option<Screenshot>,
    /// The largest size of the window that this session set, in points: the device's largest
    /// texture at the display's scale, since the window draws into a texture of its size.
    #[cfg(not(target_arch = "wasm32"))]
    window_limit: Option<egui::Vec2>,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> Result<Self, String> {
        // The charts draw on a light background.
        cc.egui_ctx.set_theme(egui::Theme::Light);
        let runtime = Exec::new().map_err(|error| error.to_string())?;
        let session = query::session().map_err(|error| error.to_string())?;
        let functions = query::functions();
        let (text, title) = match &options.query {
            Some(path) => {
                let text = std::fs::read_to_string(path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
                let title = path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                );
                (text, title)
            }
            None => (String::new(), "Untitled query".to_string()),
        };
        let mut app = Self {
            runtime,
            session,
            functions,
            cascade: charts::cascade(),
            engine: charts::engine(),
            text,
            editor: CodeEditor::new("query"),
            analyzed: Analyzed::new(),
            title,
            sources: Vec::new(),
            pending: BTreeSet::new(),
            samples: None,
            gallery: Gallery::default(),
            run_when_ready: options.query.is_some(),
            messages: channel(),
            scenes: channel(),
            next_id: 0,
            running: None,
            shown: None,
            lifted: None,
            error: None,
            notice: None,
            tab: Tab::Examples,
            cells: Cells::default(),
            pane: Pane::default(),
            opened: None,
            live_cells: LiveCells::default(),
            interactive: false,
            screenshot: options.screenshot.map(|path| Screenshot {
                path,
                started: Instant::now(),
                idle_frames: 0,
                requested: false,
                drive: options.cursor.map(|cursor| Drive {
                    cursor,
                    presses: options.presses,
                    hover: options.hover,
                    frame: 0,
                    hovered: None,
                    events: Vec::new(),
                }),
            }),
            #[cfg(not(target_arch = "wasm32"))]
            window_limit: None,
        };
        app.add_samples(&cc.egui_ctx);
        for path in &options.files {
            app.add_file(&cc.egui_ctx, path);
        }
        Ok(app)
    }

    /// Asks for CSV and Parquet files, and registers each.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_files(&mut self, ctx: &egui::Context) {
        let picked = rfd::FileDialog::new()
            .add_filter("CSV or Parquet", &["csv", "parquet"])
            .pick_files();
        for path in picked.unwrap_or_default() {
            self.add_file(ctx, &path);
        }
    }

    /// The web build reads no files of the user's yet, only the samples.
    #[cfg(target_arch = "wasm32")]
    fn pick_files(&mut self, _ctx: &egui::Context) {
        self.notice = Some("The browser build reads only the sample tables so far.".to_string());
    }

    /// Registers the sample tables, in the background.
    fn add_samples(&mut self, ctx: &egui::Context) {
        let (session, sender, ctx) = (self.session.clone(), self.messages.0.clone(), ctx.clone());
        self.runtime.spawn(move || async move {
            let samples = samples::register(&session)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(Message::Samples(samples));
            ctx.request_repaint();
        });
    }

    /// Analyzes the text against the tables that the session holds now.
    fn refresh_catalog(&mut self) {
        let samples = self.samples.as_deref().unwrap_or_default();
        self.analyzed
            .set_catalog(document::catalog(&self.sources, samples));
    }

    /// Runs the query of each card of the gallery in `indices`, in the background.
    fn run_examples(&mut self, ctx: &egui::Context, indices: Vec<usize>) {
        for index in indices {
            let (session, sender, ctx) =
                (self.session.clone(), self.messages.0.clone(), ctx.clone());
            let text = examples()[index].sql.clone();
            self.runtime.spawn(move || async move {
                let result = query::run(&session, &text)
                    .await
                    .map(|outcome| outcome.result)
                    .map_err(|error| error.message);
                let _ = sender.send(Message::Example { index, result });
                ctx.request_repaint();
            });
        }
    }

    /// Puts the example `example` in the editor, in place of the text, and runs it.
    fn open_example(&mut self, ctx: &egui::Context, example: &Example) {
        let text = example_text(example);
        self.editor.replace(ctx, &mut self.text, &text);
        self.title = example.title.clone();
        self.run(ctx);
    }

    /// Registers the file at `path` as a table, in the background.
    fn add_file(&mut self, ctx: &egui::Context, path: &Path) {
        let Some(kind) = FileKind::of(path) else {
            self.notice = Some(format!("{} is not a CSV or Parquet file.", path.display()));
            return;
        };
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let taken: BTreeSet<String> = (self.sources.iter())
            .map(|source| source.name.clone())
            .chain(self.pending.iter().cloned())
            .collect();
        let name = table_name(&path, &taken);
        self.pending.insert(name.clone());
        let (session, sender, ctx) = (self.session.clone(), self.messages.0.clone(), ctx.clone());
        self.runtime.spawn(move || async move {
            let source = query::register(&session, &name, &path, kind)
                .await
                .map_err(|error| error.to_string());
            let _ = sender.send(Message::Registered { name, source, path });
            ctx.request_repaint();
        });
    }

    /// Runs the query in the background, in place of the one that runs.
    fn run(&mut self, ctx: &egui::Context) {
        if self.text.trim().is_empty() {
            return;
        }
        if self.tab == Tab::Examples {
            self.tab = Tab::Result;
        }
        if let Some(running) = self.running.take() {
            running.task.abort();
        }
        self.next_id += 1;
        let id = self.next_id;
        self.lifted = None;
        let (session, text) = (self.session.clone(), self.text.clone());
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        let task = self.runtime.spawn(move || async move {
            let started = Instant::now();
            let outcome = query::run(&session, &text).await;
            let elapsed = started.elapsed();
            let _ = sender.send(Message::Ran {
                id,
                outcome,
                elapsed,
            });
            ctx.request_repaint();
            // The lifting plans the input of each lifted AGGREGATE, after the result shows.
            let lifted = query::lift(&session, &text).await;
            let _ = sender.send(Message::Lifted { id, lifted });
            ctx.request_repaint();
        });
        self.running = Some(Running {
            id,
            task,
            started: Instant::now(),
        });
    }

    fn cancel(&mut self) {
        if let Some(running) = self.running.take() {
            running.task.abort();
            self.notice = Some("Canceled the query.".to_string());
        }
    }

    fn receive(&mut self, ctx: &egui::Context, state: &RenderState) {
        while let Ok(message) = self.messages.1.try_recv() {
            match message {
                Message::Registered { name, source, path } => {
                    self.pending.remove(&name);
                    match source {
                        Ok(source) => {
                            self.sources.push(source);
                            self.refresh_catalog();
                        }
                        Err(error) => {
                            self.notice = Some(format!("Cannot read {}: {error}", path.display()));
                        }
                    }
                }
                Message::Ran {
                    id,
                    outcome,
                    elapsed,
                } => {
                    if self.running.as_ref().map(|running| running.id) != Some(id) {
                        // A query that the user canceled or replaced.
                        continue;
                    }
                    self.running = None;
                    match outcome {
                        Ok(outcome) => self.show_result(state, id, outcome, elapsed),
                        Err(error) => self.error = Some(error),
                    }
                }
                Message::Lifted { id, lifted } => {
                    if id == self.next_id {
                        self.lifted = Some(lifted);
                    }
                }
                Message::Samples(samples) => {
                    self.samples = Some(samples.unwrap_or_else(|error| {
                        self.notice = Some(format!("Cannot load the samples: {error}"));
                        Vec::new()
                    }));
                    self.refresh_catalog();
                }
                Message::Example { index, result } => self.gallery.ran(index, result),
            }
        }
        while let Ok(scene) = self.scenes.1.try_recv() {
            match scene.result == THUMBNAILS {
                true => self.gallery.receive(scene),
                false => self.cells.receive(scene),
            }
        }
        if self.run_when_ready && self.pending.is_empty() && self.samples.is_some() {
            self.run_when_ready = false;
            self.run(ctx);
        }
    }

    fn show_result(&mut self, state: &RenderState, id: u64, outcome: Outcome, elapsed: Duration) {
        self.error = None;
        self.notice = None;
        self.cells.reset(id, state);
        self.live_cells.reset(id, state);
        if let Some((_, mut opened)) = self.opened.take() {
            opened.clear(state);
        }
        let Outcome { result, plan } = outcome;
        match (result.shape(), &self.pane.chart) {
            // The chart of the pane takes the new chart and keeps its pans and zooms.
            (Shape::Chart, Some(pane)) => {
                let chart = result.chart(0, 0).expect("a single chart").clone();
                pane.set_chart(chart);
            }
            // A new chart of the pane builds when the pane lays out, at the pane's size.
            _ => self.pane.clear(state),
        }
        self.shown = Some(Shown {
            result,
            plan,
            elapsed,
        });
    }

    /// Formats the query, unless it has an error of the syntax.
    fn format(&mut self, ctx: &egui::Context) {
        let document = self.analyzed.document(&self.text);
        if format::blocked(document).is_some() {
            return;
        }
        if let Some(formatted) = format::formatted(document) {
            self.editor.format(ctx, &mut self.text, &formatted);
        }
    }

    fn busy(&self) -> bool {
        self.running.is_some()
            || !self.pending.is_empty()
            || self.samples.is_none()
            || self.run_when_ready
            || (self.tab == Tab::Examples && self.gallery.busy())
            || self.cells.busy()
            || self.pane.busy()
            || self.opened.as_ref().is_some_and(|(_, pane)| pane.busy())
            || self.live_cells.busy()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.strong(&self.title);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let run = ui.button("Run").on_hover_text(format!(
                    "Run the query ({})",
                    ctx.format_shortcut(&run_shortcut())
                ));
                if run.clicked() {
                    self.run(ctx);
                }
                let blocked = format::blocked(self.analyzed.document(&self.text));
                let button = ui.add_enabled(blocked.is_none(), egui::Button::new("Format"));
                let button = button
                    .on_hover_text(format!(
                        "Format the query ({})",
                        ctx.format_shortcut(&format::shortcut())
                    ))
                    .on_disabled_hover_text(blocked.unwrap_or_default());
                if button.clicked() {
                    self.format(ctx);
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if let Some(running) = &self.running {
                ui.spinner();
                ui.label("Running…");
                let waited = running.started.elapsed();
                if waited >= CANCEL_AFTER {
                    if ui.small_button("Cancel").clicked() {
                        self.cancel();
                    }
                } else {
                    ui.ctx().request_repaint_after(CANCEL_AFTER - waited);
                }
            } else if let Some(error) = &self.error {
                let first = error.message.lines().next().unwrap_or_default();
                ui.add(
                    egui::Label::new(egui::RichText::new(first).color(ui.visuals().error_fg_color))
                        .truncate(),
                )
                .on_hover_text(&error.message);
            } else if let Some(notice) = &self.notice {
                ui.add(egui::Label::new(notice).truncate());
            } else if let Some(shown) = &self.shown {
                let rows = shown.result.num_rows();
                let noun = if rows == 1 { "row" } else { "rows" };
                ui.label(format!("{rows} {noun} · {} ms", shown.elapsed.as_millis()));
            } else {
                ui.weak("Write a query and press Run.");
            }
        });
    }

    fn results(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &RenderState) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Result, "Result");
            ui.selectable_value(&mut self.tab, Tab::Examples, "Examples");
            ui.selectable_value(&mut self.tab, Tab::Lifted, "Lifted SQL");
            ui.selectable_value(&mut self.tab, Tab::Plan, "Plan");
            if self.error.is_some() && self.shown.is_some() && self.tab != Tab::Examples {
                ui.weak("· the last result that ran");
            }
            if self.tab == Tab::Examples {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.gallery.search_box(ui);
                });
            }
            let charts_in_table = self.shown.as_ref().is_some_and(|shown| {
                shown.result.shape() == Shape::Table && !shown.result.chart_columns().is_empty()
            });
            if self.tab == Tab::Result && charts_in_table && self.opened.is_none() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let toggle = ui.checkbox(&mut self.interactive, "Interactive charts");
                    let toggle = toggle.on_hover_text(
                        "Pan, zoom, and select in every chart of the table. Off, the wheel \
                         scrolls the table over the charts.",
                    );
                    if toggle.changed() && !self.interactive {
                        self.live_cells.clear(state);
                    }
                });
            }
        });
        ui.separator();
        match self.tab {
            Tab::Examples => {
                let env = Env {
                    state,
                    runtime: &self.runtime,
                    scenes: &self.scenes.0,
                    cascade: &self.cascade,
                    engine: &self.engine,
                };
                let output = self.gallery.show(ui, &env, self.samples.is_some());
                self.run_examples(ctx, output.start);
                if let Some(index) = output.open {
                    self.open_example(ctx, &examples()[index]);
                }
            }
            Tab::Result => self.result_tab(ui, state),
            Tab::Plan => {
                let plan = self.shown.as_ref().map_or("", |shown| shown.plan.as_str());
                code(ui, plan);
            }
            Tab::Lifted => match &self.lifted {
                None => {
                    ui.weak("The lifted query shows here after the query runs.");
                }
                Some(Ok(lifted)) => {
                    ui.weak("The highlighted text computes the statistics of the mark calls.");
                    sql(ui, &lifted.text, &lifted.generated);
                }
                Some(Err(error)) => {
                    ui.colored_label(ui.visuals().error_fg_color, &error.message);
                }
            },
        }
    }

    fn result_tab(&mut self, ui: &mut egui::Ui, state: &RenderState) {
        let Some(shown) = &self.shown else {
            ui.weak("The result of the query shows here.");
            return;
        };
        let env = Env {
            state,
            runtime: &self.runtime,
            scenes: &self.scenes.0,
            cascade: &self.cascade,
            engine: &self.engine,
        };
        if shown.result.shape() == Shape::Chart {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let chart = shown.result.chart(0, 0).expect("a single chart");
            let id = egui::Id::new(("live chart", self.next_id));
            if let Some(error) = self.pane.show(ui, rect, chart, id, &env) {
                self.notice = Some(error);
            }
            return;
        }
        if let Some((key, pane)) = &mut self.opened {
            let key = *key;
            let mut back = false;
            ui.horizontal(|ui| {
                back = ui
                    .button("Back")
                    .on_hover_text("Back to the table (Esc)")
                    .clicked();
                ui.label(row_label(&shown.result, key));
            });
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            if let Some(chart) = shown.result.chart(key.0, key.1) {
                let id = egui::Id::new(("opened chart", self.next_id, key));
                if let Some(error) = pane.show(ui, rect, chart, id, &env) {
                    self.notice = Some(error);
                }
            }
            // Escape goes back unless it is the chart's: the chart hears keys while it has the
            // focus or the pointer.
            let escape = ui.input(|input| input.key_pressed(egui::Key::Escape))
                && ui.memory(|memory| memory.focused().is_none())
                && !ui.rect_contains_pointer(rect);
            if back || escape {
                if let Some((_, mut pane)) = self.opened.take() {
                    pane.clear(state);
                }
            }
            return;
        }
        let mut grid = Grid {
            result: &shown.result,
            cells: &mut self.cells,
            live: &mut self.live_cells,
            interactive: self.interactive,
            env,
            expand: None,
        };
        grid.show(ui);
        let expand = grid.expand;
        self.cells.end_frame(state);
        self.live_cells.end_frame(state);
        if let Some(key) = expand {
            self.opened = Some((key, Pane::default()));
        }
    }

    /// Keeps the window within the device's largest texture at the display's scale, since a
    /// window wider or taller than that fails to draw. A web page sizes its canvas itself.
    #[cfg(not(target_arch = "wasm32"))]
    fn limit_window(&mut self, ctx: &egui::Context, state: &RenderState) {
        let pixels = state.device.limits().max_texture_dimension_2d as f32;
        let limit = egui::Vec2::splat((pixels / ctx.pixels_per_point()).floor());
        if self.window_limit != Some(limit) {
            self.window_limit = Some(limit);
            ctx.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(limit));
        }
    }

    fn screenshot(&mut self, ctx: &egui::Context) {
        let busy = self.busy();
        let Some(screenshot) = &mut self.screenshot else {
            return;
        };
        let image = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let [width, height] = image.size;
            let pixels: Vec<u8> = image
                .pixels
                .iter()
                .flat_map(|pixel| pixel.to_array())
                .collect();
            let saved = image::RgbaImage::from_raw(width as u32, height as u32, pixels)
                .ok_or_else(|| "the screenshot has the wrong size".to_string())
                .and_then(|png| {
                    png.save(&screenshot.path)
                        .map_err(|error| error.to_string())
                });
            if let Err(error) = saved {
                eprintln!(
                    "avenger-sql-editor: cannot save {}: {error}",
                    screenshot.path.display()
                );
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        screenshot.idle_frames = if busy { 0 } else { screenshot.idle_frames + 1 };
        let timed_out = screenshot.started.elapsed() > SCREENSHOT_TIMEOUT;
        let settled = screenshot.idle_frames >= 10;
        let driven = match &mut screenshot.drive {
            Some(drive) => settled && drive.step(ctx, &self.editor),
            None => true,
        };
        if !screenshot.requested && ((settled && driven) || timed_out) {
            screenshot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}

impl eframe::App for EditorApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let drive = (self.screenshot.as_mut()).and_then(|screenshot| screenshot.drive.as_mut());
        if let Some(drive) = drive {
            raw_input.events.append(&mut drive.events);
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(state) = frame.wgpu_render_state().cloned() else {
            return;
        };
        self.receive(ctx, &state);
        #[cfg(not(target_arch = "wasm32"))]
        self.limit_window(ctx, &state);
        let dropped: Vec<PathBuf> = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .collect()
        });
        for path in dropped {
            self.add_file(ctx, &path);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&run_shortcut())) {
            self.run(ctx);
        }
        let formatting = ctx.input_mut(|input| {
            let pressed = input.consume_shortcut(&format::shortcut());
            // ⇧⌥F types `Ï` on macOS, which the shortcut takes too.
            if pressed {
                input
                    .events
                    .retain(|event| !matches!(event, egui::Event::Text(_)));
            }
            pressed
        });
        if formatting {
            self.format(ctx);
        }

        egui::SidePanel::left("data")
            .resizable(true)
            .default_width(240.0)
            .show(ctx, |ui| {
                let output = sidebar::show(
                    ui,
                    &self.sources,
                    self.pending.len(),
                    self.samples.as_deref(),
                    &self.functions,
                );
                if output.add_files {
                    self.pick_files(ctx);
                }
                if let Some(insert) = output.insert {
                    self.editor.insert(ctx, &mut self.text, &insert);
                }
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        egui::TopBottomPanel::top("editor")
            .resizable(true)
            .default_height(240.0)
            .show(ctx, |ui| {
                self.toolbar(ui, ctx);
                ui.separator();
                let analyzed = self.text.clone();
                let document = self.analyzed.document(&analyzed);
                if self.editor.show(ui, &mut self.text, document) {
                    // The error points into the text before the edit.
                    if let Some(error) = &mut self.error {
                        error.range = None;
                    }
                }
            });
        egui::CentralPanel::default().show(ctx, |ui| self.results(ui, ctx, &state));
        self.gallery.end_frame(&state);
        self.screenshot(ctx);
    }
}

/// The live chart that fills the result pane: the chart of a single-chart result, or a chart
/// cell that the user expanded. It builds at the pane's size the first time it shows.
#[derive(Default)]
struct Pane {
    chart: Option<LiveChart>,
    /// Whether the chart failed to build, so that it builds once.
    failed: bool,
}

impl Pane {
    /// Shows `chart` in `rect`, and builds it there the first time. Returns the error of a build
    /// that fails.
    fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        chart: &avenger_chart::Chart,
        id: egui::Id,
        env: &Env<'_>,
    ) -> Option<String> {
        let mut error = None;
        if self.chart.is_none() && !self.failed {
            let size = rect.size();
            let built = LiveChart::new(
                chart.clone(),
                env.cascade.clone(),
                env.engine.clone(),
                [size.x, size.y],
                id,
                env.runtime,
                ui.ctx(),
                env.state,
            );
            match built {
                Ok(chart) => self.chart = Some(chart),
                Err(message) => {
                    self.failed = true;
                    error = Some(format!("The chart cannot run: {message}"));
                }
            }
        }
        if let Some(chart) = &mut self.chart {
            chart.show(ui, rect, env.state, env.runtime);
        }
        error
    }

    fn busy(&self) -> bool {
        self.chart.as_ref().is_some_and(LiveChart::busy)
    }

    /// Drops the chart and its texture.
    fn clear(&mut self, state: &RenderState) {
        if let Some(chart) = self.chart.take() {
            chart.free(state);
        }
        self.failed = false;
    }
}

/// The values of the columns of `row` that are not chart columns, then the name of the chart
/// column of `key`, as the bar over an expanded chart shows them.
fn row_label(result: &QueryResult, (row, column): CellKey) -> String {
    let fields = result.schema().fields();
    let mut parts: Vec<String> = (0..fields.len())
        .filter(|&index| !result.is_chart_column(index))
        .map(|index| {
            let value = result.value_text(row, index);
            format!(
                "{} = {}",
                fields[index].name(),
                value.as_deref().unwrap_or("null")
            )
        })
        .collect();
    parts.push(fields[column].name().clone());
    parts.join(" · ")
}

/// The editor text of `example`: its title and its description as comments, then its query.
fn example_text(example: &Example) -> String {
    let mut text = format!("-- {}\n", example.title);
    for (index, paragraph) in example.description.split("\n\n").enumerate() {
        if index > 0 {
            text.push_str("--\n");
        }
        for line in wrap(paragraph, 96) {
            text.push_str("-- ");
            text.push_str(&line);
            text.push('\n');
        }
    }
    text.push_str(&example.sql);
    text.push('\n');
    text
}

/// The words of `text` in lines of at most `width` characters, except for a longer word.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

fn run_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)
}

fn code(ui: &mut egui::Ui, text: &str) {
    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        let mut text = text;
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .code_editor()
                .desired_width(f32::INFINITY),
        );
    });
}

/// `text`, highlighted as the editor highlights a query, with the ranges of characters `marked`
/// on a background of their own, read-only.
fn sql(ui: &mut egui::Ui, text: &str, marked: &[std::ops::Range<usize>]) {
    let background = match ui.visuals().dark_mode {
        true => egui::Color32::from_rgb(84, 70, 24),
        false => egui::Color32::from_rgb(255, 241, 194),
    };
    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, _wrap: f32| {
        let job = editor::layout_job(
            buffer.as_str(),
            &highlight::syntax(buffer.as_str()),
            egui::FontId::monospace(13.0),
            marked,
            background,
        );
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        let mut text = text;
        ui.add(
            egui::TextEdit::multiline(&mut text)
                .code_editor()
                .desired_width(f32::INFINITY)
                .layouter(&mut layouter),
        );
    });
}
