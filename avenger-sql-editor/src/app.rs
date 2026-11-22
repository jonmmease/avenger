//! The editor's window: the open project's files by role in the sidebar on the left, the tabs of
//! its open files and the editor of the active tab over the result pane on the right, and a status
//! bar under them. The editor opens the sample project when it starts, and the result pane's
//! gallery shows the open project's charts.

use avenger_common::time::{Duration, Instant};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use avenger_sql::{catalog::FilesTable, data::Files};
use avenger_style::{Cascade, Preferences};
use avenger_typst_label::LabelEngine;
use eframe::{egui, egui_wgpu::RenderState};

use crate::assist::{Analysis, Target};
use crate::buffer::{Buffer, Running, Shown};
use crate::cards::{Gallery, Start, THUMBNAILS};
use crate::charts::{self, ChartTheme, Images};
use crate::editor::{self, CodeEditor, Text};
use crate::exec::Exec;
use crate::format;
use crate::highlight;
use crate::live::LiveChart;
use crate::query::{self, Functions, Outcome, QueryError};
use crate::result::{QueryResult, Shape};
use crate::sidebar;
use crate::table::{CellKey, CellScene, Cells, Env, Grid, LiveCells};
use crate::workspace::{self, Load, LoadState, Saved, Styles, TabKind, Workspace};
use avenger_sql_language::FileRole;
use avenger_sql_project::TableChange;

/// How long a query runs before the status bar offers to cancel it.
const CANCEL_AFTER: Duration = Duration::from_millis(200);
/// The longest that `--screenshot` waits for the window to settle.
const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);

/// The command-line options of the editor.
#[derive(Debug, Default)]
pub struct Options {
    /// The project to open in place of the sample project: its directory, or a path under it. A
    /// file of the project opens in a tab.
    pub project: Option<PathBuf>,
    /// The file of the project to open in a tab, by its path relative to the project's root, such
    /// as a chart of the sample project, which runs when it opens.
    pub open: Option<String>,
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
    /// The theme that `--theme <system|light|dark>` sets in place of the one that the editor
    /// remembers.
    pub theme: Option<egui::ThemePreference>,
}

impl Options {
    /// The options of `args`, which exclude the program name: `--project <PATH>`,
    /// `--open <PATH>`, `--screenshot <PNG>`, the options that drive the editor before a
    /// screenshot, `--cursor <INDEX>`, `--press <KEYS>`, and `--hover`, and
    /// `--theme <system|light|dark>`.
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
                "--project" => options.project = Some(value("--project")?),
                "--open" => {
                    let path = value("--open")?.to_string_lossy().into_owned();
                    options.open = Some(path);
                }
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
                "--theme" => {
                    let theme = value("--theme")?.to_string_lossy().to_ascii_lowercase();
                    options.theme = Some(match theme.as_str() {
                        "system" => egui::ThemePreference::System,
                        "light" => egui::ThemePreference::Light,
                        "dark" => egui::ThemePreference::Dark,
                        _ => {
                            return Err(format!(
                                "--theme takes system, light, or dark, not {theme}"
                            ))
                        }
                    });
                }
                flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
                _ => {
                    return Err(format!(
                        "{arg} is not an option: the editor opens a project, whose data scripts \
                         read its files, with --project"
                    ))
                }
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
    Ran {
        owner: Owner,
        id: u64,
        outcome: Result<Outcome, QueryError>,
        elapsed: Duration,
    },
    /// The table of each set of files that the loader read, after a run of a chart of the
    /// project that the editor opened as its `generation`th, whose own tables of files can read
    /// files.
    FileTables {
        generation: u64,
        tables: Vec<(Files, FilesTable)>,
    },
    /// The lifted form of the text of a run, after its result.
    Lifted {
        owner: Owner,
        id: u64,
        lifted: Result<query::LiftedText, QueryError>,
    },
    /// The result of the chart of the card `index` of the gallery of the project that the editor
    /// opened as its `generation`th, at the card's `version`.
    Card {
        generation: u64,
        index: usize,
        version: usize,
        result: Result<QueryResult, String>,
    },
    /// A project that opened, and the file of it to open in a tab.
    Opened {
        request: u64,
        opened: Result<(Box<Workspace>, Option<String>), String>,
    },
    /// A load of data scripts that ended, by its key in the workspace's `loading`, or `None` for
    /// changed files that no data script reads, which load nothing.
    Reloaded {
        generation: u64,
        key: String,
        result: Result<Option<(Vec<TableChange>, LoadState)>, String>,
        /// The charts whose own tables of files read files that changed, which run again.
        charts: Vec<String>,
    },
    /// A load of the project's stylesheets that ended.
    Styled {
        generation: u64,
        styles: Result<Styles, String>,
    },
    /// The copies of the data files of Add file that ended, in the project that the editor
    /// opened as its `generation`th.
    #[cfg(not(target_arch = "wasm32"))]
    Copied {
        generation: u64,
        copied: workspace::Copied,
    },
}

/// The buffer that a run belongs to: the tab of a file of the project that the editor opened as
/// its `generation`th.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    generation: u64,
    path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Result,
    Gallery,
    Lifted,
    Plan,
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

/// What waits for the answer of the dialog of the edits that are not saved: a project that
/// opened, by its request, with the file to open in a tab, or the close of the window.
enum Confirm {
    Open {
        request: u64,
        workspace: Box<Workspace>,
        file: Option<String>,
    },
    #[cfg(not(target_arch = "wasm32"))]
    Close,
}

pub struct EditorApp {
    runtime: Exec,
    functions: Functions,
    /// The theme of the charts, which follows the editor's visuals, the reader's preferences of
    /// the theme, and the cascade of the theme with the open project's stylesheets.
    theme: ChartTheme,
    cascade: Arc<Cascade>,
    preferences: Preferences,
    engine: LabelEngine,
    /// How the charts load their images, such as the tiles of a map.
    images: Images,
    /// The cards of the open project's charts.
    gallery: Gallery,
    /// The open project, which is `None` until the first project opens.
    workspace: Option<Workspace>,
    /// The count of the projects that the editor opened, which tells the messages of the open
    /// project from those of a project before it.
    generation: u64,
    /// The count of the projects that the editor began to open, which tells the last from those
    /// that it began before.
    requests: u64,
    /// The name of the project that opens in the background.
    opening: Option<String>,
    messages: (Sender<Message>, Receiver<Message>),
    scenes: (Sender<CellScene>, Receiver<CellScene>),
    next_id: u64,
    /// The dialog that asks whether to save the edits that are not saved, while it shows.
    confirm: Option<Confirm>,
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
    /// The watch of the open project's directory, which loads the data scripts that read a file
    /// that changes. The sample project has none.
    #[cfg(not(target_arch = "wasm32"))]
    watch: Option<crate::watch::Watch>,
    /// The largest size of the window that this session set, in points: the device's largest
    /// texture at the display's scale, since the window draws into a texture of its size.
    #[cfg(not(target_arch = "wasm32"))]
    window_limit: Option<egui::Vec2>,
}

impl EditorApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: Options) -> Result<Self, String> {
        if let Some(theme) = options.theme {
            cc.egui_ctx.set_theme(theme);
        }
        let theme = ChartTheme::of(&cc.egui_ctx.style().visuals);
        let runtime = Exec::new().map_err(|error| error.to_string())?;
        let functions = query::functions();
        let mut app = Self {
            runtime,
            functions,
            theme,
            cascade: theme.cascade(None),
            preferences: theme.preferences(),
            engine: charts::engine(),
            images: Images::new(&cc.egui_ctx)?,
            gallery: Gallery::default(),
            workspace: None,
            generation: 0,
            requests: 0,
            opening: None,
            messages: channel(),
            scenes: channel(),
            next_id: 0,
            confirm: None,
            notice: None,
            tab: Tab::Gallery,
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
            watch: None,
            #[cfg(not(target_arch = "wasm32"))]
            window_limit: None,
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = options.project {
            app.open_project(&cc.egui_ctx, path, options.open);
            return Ok(app);
        }
        app.open_sample(&cc.egui_ctx, options.open);
        Ok(app)
    }

    /// Asks for the directory of a project, or of a new project with `new`, and opens it. A new
    /// project gets the smallest manifest, named after its directory, unless it has one.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_project(&mut self, ctx: &egui::Context, new: bool) {
        let title = match new {
            true => "New project: choose its folder",
            false => "Open project",
        };
        let Some(directory) = rfd::FileDialog::new().set_title(title).pick_folder() else {
            return;
        };
        let directory = match new {
            true => match crate::workspace::create(&directory) {
                Ok(directory) => directory,
                Err(error) => {
                    self.notice = Some(format!("Cannot make the project: {error}"));
                    return;
                }
            },
            false => directory,
        };
        self.open_project(ctx, directory, None);
    }

    /// The web build opens no projects of the user's yet.
    #[cfg(target_arch = "wasm32")]
    fn pick_project(&mut self, _ctx: &egui::Context, _new: bool) {
        self.notice = Some("The browser build opens no projects so far.".to_string());
    }

    /// Asks for data files, and adds them to the open project.
    #[cfg(not(target_arch = "wasm32"))]
    fn pick_files(&mut self, ctx: &egui::Context) {
        let dialog = rfd::FileDialog::new().set_title("Add file");
        let dialog = match self.workspace.as_ref().and_then(Workspace::root) {
            Some(root) => dialog.set_directory(root),
            None => dialog,
        };
        if let Some(paths) = dialog.pick_files() {
            self.add_files(ctx, &paths);
        }
    }

    /// The web build adds no files, since it opens no projects of the user's.
    #[cfg(target_arch = "wasm32")]
    fn pick_files(&mut self, _ctx: &egui::Context) {
        self.notice = Some("The browser build adds no files so far.".to_string());
    }

    /// Adds the data files at `paths` to the open project, as Add file and a drop do: copies the
    /// files from outside the project's root into it in the background, and then
    /// [`EditorApp::added`] writes their statements.
    #[cfg(not(target_arch = "wasm32"))]
    fn add_files(&mut self, ctx: &egui::Context, paths: &[PathBuf]) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let adding = match workspace.adding(paths) {
            Ok(adding) => adding,
            Err(error) => {
                self.notice = Some(format!("Cannot add the files: {error}"));
                return;
            }
        };
        self.notice = Some("Adding the files…".to_string());
        let generation = self.generation;
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        self.runtime.spawn_blocking(move || {
            let copied = adding.copy();
            let _ = sender.send(Message::Copied { generation, copied });
            ctx.request_repaint();
        });
    }

    /// Writes the `CREATE EXTERNAL TABLE` statement of each file of `copied`, the data files of
    /// Add file after their copies, into a data script, which opens in a tab and loads, and says
    /// in the status bar what it added.
    #[cfg(not(target_arch = "wasm32"))]
    fn added(&mut self, ctx: &egui::Context, state: &RenderState, copied: workspace::Copied) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let before = workspace.shown().map(|tab| tab.path.clone());
        let added = workspace.add(copied);
        let shown = workspace.shown().map(|tab| tab.path.clone());
        let root = workspace.root().map(PathBuf::from);
        let added = match added {
            Ok(added) => added,
            Err(error) => {
                self.notice = Some(format!("Cannot add the files: {error}"));
                return;
            }
        };
        if shown != before {
            self.present(state);
        }
        if let (Some(watch), Some(root)) = (&mut self.watch, root) {
            for path in added.copied.iter().chain([&added.script]) {
                watch.own(root.join(path));
            }
        }
        let (noun, pronoun) = match added.tables.len() {
            1 => ("table", "it"),
            _ => ("tables", "them"),
        };
        let mut notice = format!(
            "Added the {noun} {} to {}.",
            added.tables.join(", "),
            added.script
        );
        if added.load.is_none() && added.run.is_none() {
            notice.push_str(&format!(" Save {} to load {pronoun}.", added.script));
        }
        for error in &added.errors {
            notice.push_str(&format!(" Not added: {error}."));
        }
        self.notice = Some(notice);
        if let Some(path) = added.run {
            self.follow_save(ctx, Saved::Chart { path });
        }
        if let Some(load) = added.load {
            self.reload(ctx, load);
        }
    }

    /// Adds the files dropped on the window to the open project, as Add file does, unless it is
    /// the sample project, which is read-only.
    #[cfg(not(target_arch = "wasm32"))]
    fn drop_files(&mut self, ctx: &egui::Context, dropped: Vec<egui::DroppedFile>) {
        if (self.workspace.as_ref()).is_some_and(Workspace::read_only) {
            self.notice = Some(
                "A sample can't take files: open or make a project, and drop its files on the \
                 window to add them."
                    .to_string(),
            );
            return;
        }
        let paths: Vec<PathBuf> = (dropped.into_iter()).filter_map(|file| file.path).collect();
        self.add_files(ctx, &paths);
    }

    /// The web build adds no files dropped on the window, since it opens no projects of the
    /// user's.
    #[cfg(target_arch = "wasm32")]
    fn drop_files(&mut self, _ctx: &egui::Context, _dropped: Vec<egui::DroppedFile>) {
        self.notice = Some("The browser build adds no files so far.".to_string());
    }

    /// Follows the files that changed under the open project's root, once a burst of changes
    /// ends: loads the data scripts that read a changed data file, and reads a changed file of the
    /// project's own again, which then loads, runs, or restyles as after a save.
    #[cfg(not(target_arch = "wasm32"))]
    fn follow_files(&mut self, ctx: &egui::Context) {
        let (Some(watch), Some(workspace)) = (&mut self.watch, &mut self.workspace) else {
            return;
        };
        let (mut files, mut saved) = (Vec::new(), Vec::new());
        for path in watch.changed(ctx) {
            match workspace.watched(&path) {
                Some(workspace::Watched::Data(path)) => files.push(path),
                Some(workspace::Watched::Project(path)) => saved.extend(workspace.reread(&path)),
                None => {}
            }
        }
        if !files.is_empty() {
            self.reload(ctx, Load::Files(files));
        }
        for saved in saved {
            self.follow_save(ctx, saved);
        }
    }

    /// Opens the project that holds `path`, a directory or a file under the project's root, in
    /// the background, in place of the open project. The file of `path`, or else the file `open`
    /// of the project, opens in a tab.
    #[cfg(not(target_arch = "wasm32"))]
    fn open_project(&mut self, ctx: &egui::Context, path: PathBuf, open: Option<String>) {
        let name = path.display().to_string();
        self.request(ctx, name, move || async move {
            let (workspace, file) = workspace::open(&path).await?;
            Ok((Box::new(workspace), file.or(open)))
        });
    }

    /// Opens the sample project in the background, in place of the open project, and its file
    /// `open` in a tab.
    fn open_sample(&mut self, ctx: &egui::Context, open: Option<String>) {
        self.request(ctx, "the sample project".to_string(), move || async move {
            Ok((Box::new(workspace::sample().await?), open))
        });
    }

    /// Opens the project that `open` loads in the background, with the file to open in a tab,
    /// as the newest request, whose name the status bar shows.
    fn request<W, F>(&mut self, ctx: &egui::Context, name: String, open: W)
    where
        W: FnOnce() -> F + Send + 'static,
        F: std::future::Future<Output = Result<(Box<Workspace>, Option<String>), String>> + 'static,
    {
        self.requests += 1;
        let request = self.requests;
        self.opening = Some(name);
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        // A load is never stopped: it changes the session as it goes.
        drop(self.runtime.spawn(move || async move {
            let opened = open().await;
            let _ = sender.send(Message::Opened { request, opened });
            ctx.request_repaint();
        }));
    }

    /// Runs the chart of each card of the gallery in `starts`, in the background.
    fn run_cards(&mut self, ctx: &egui::Context, starts: Vec<Start>) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let generation = self.generation;
        for Start {
            index,
            version,
            path,
            text,
        } in starts
        {
            let (loader, sender, ctx) = (
                workspace.loader().clone(),
                self.messages.0.clone(),
                ctx.clone(),
            );
            self.runtime.spawn(move || async move {
                let result = (loader.run_chart(&path, &text).await)
                    .map(|outcome| outcome.result)
                    .map_err(|error| error.message);
                let _ = sender.send(Message::Card {
                    generation,
                    index,
                    version,
                    result,
                });
                ctx.request_repaint();
            });
        }
    }

    /// The buffer that the editor shows: the active tab of the project, which has none while no
    /// tab is open.
    fn active(&self) -> Option<&Buffer> {
        let workspace = self.workspace.as_ref()?;
        workspace.active().map(|tab| &tab.buffer)
    }

    fn active_mut(&mut self) -> Option<&mut Buffer> {
        let workspace = self.workspace.as_mut()?;
        workspace.active_mut().map(|tab| &mut tab.buffer)
    }

    /// The buffer whose result the result pane shows: the shown tab of the project, which is the
    /// active tab unless a stylesheet's tab is active.
    fn shown(&self) -> Option<&Buffer> {
        let workspace = self.workspace.as_ref()?;
        workspace.shown().map(|tab| &tab.buffer)
    }

    fn shown_mut(&mut self) -> Option<&mut Buffer> {
        let workspace = self.workspace.as_mut()?;
        workspace.shown_mut().map(|tab| &mut tab.buffer)
    }

    /// The owner of the shown buffer's runs.
    fn shown_owner(&self) -> Option<Owner> {
        let workspace = self.workspace.as_ref()?;
        workspace.shown().map(|tab| Owner {
            generation: self.generation,
            path: tab.path.clone(),
        })
    }

    fn buffer_mut(&mut self, owner: &Owner) -> Option<&mut Buffer> {
        if owner.generation != self.generation {
            return None;
        }
        let tab = self.workspace.as_mut()?.tab_mut(&owner.path)?;
        Some(&mut tab.buffer)
    }

    /// Runs the chart script of the active tab, and shows its result.
    fn run(&mut self, ctx: &egui::Context) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let chart = (workspace.active()).filter(|tab| tab.kind == TabKind::Sql(FileRole::Chart));
        if let Some(path) = chart.map(|tab| tab.path.clone()) {
            self.run_tab(ctx, &path);
            self.tab = match self.tab {
                Tab::Gallery => Tab::Result,
                tab => tab,
            };
        }
    }

    /// Runs the chart script of the tab of `path` against the project's tables, in the
    /// background, in place of its run that has not ended.
    fn run_tab(&mut self, ctx: &egui::Context, path: &str) {
        let generation = self.generation;
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let loader = workspace.loader().clone();
        let Some(tab) = workspace.tab_mut(path) else {
            return;
        };
        self.next_id += 1;
        let id = self.next_id;
        let owner = Owner {
            generation,
            path: path.to_string(),
        };
        let text = tab.buffer.text.clone();
        let chart = path.to_string();
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        let task = self.runtime.spawn(move || async move {
            let started = Instant::now();
            let outcome = loader.run_chart(&chart, &text).await;
            let elapsed = started.elapsed();
            let _ = (sender).send(Message::Ran {
                owner: owner.clone(),
                id,
                outcome,
                elapsed,
            });
            // The chart's own tables of files read their files as it ran.
            let tables = loader.file_tables().await;
            let _ = sender.send(Message::FileTables { generation, tables });
            ctx.request_repaint();
            let lifted = loader.lift(&chart, &text).await;
            let _ = sender.send(Message::Lifted { owner, id, lifted });
            ctx.request_repaint();
        });
        tab.buffer.cancel();
        tab.buffer.lifted = None;
        tab.buffer.latest = id;
        tab.buffer.running = Some(Running {
            id,
            task,
            started: Instant::now(),
        });
    }

    /// Saves the active tab to its file, and loads it again when it is a data script, runs it and
    /// its card when it is a chart script, loads the project again when it is a module, and loads
    /// the stylesheets again and restyles the charts when it is a stylesheet. The sample project
    /// can't be saved.
    fn save(&mut self, ctx: &egui::Context) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let Some(path) = workspace.active().map(|tab| tab.path.clone()) else {
            return;
        };
        let saved = workspace.save(&path);
        // The watch of the project's files leaves out the editor's own write.
        #[cfg(not(target_arch = "wasm32"))]
        if let (Ok(_), Some(watch), Some(root)) = (&saved, &mut self.watch, workspace.root()) {
            watch.own(root.join(&path));
        }
        match saved {
            Ok(saved) => self.follow_save(ctx, saved),
            Err(error) => self.notice = Some(format!("Cannot save: {error}")),
        }
    }

    /// Does what a save of a file asks for next, as `saved` tells it: loads a data script or a
    /// module again, with the data scripts downstream of it, runs a chart script and its card,
    /// and loads the stylesheets again and restyles the charts after a stylesheet changes.
    fn follow_save(&mut self, ctx: &egui::Context, saved: Saved) {
        match saved {
            Saved::Data { path, text } | Saved::Module { path, text } => {
                self.reload(ctx, Load::Script { path, text })
            }
            Saved::Chart { path } => {
                let entry = (self.workspace.as_ref()).and_then(|workspace| workspace.entry(&path));
                if let Some(entry) = entry {
                    self.gallery.update(entry);
                }
                self.run_tab(ctx, &path);
            }
            Saved::Stylesheet => self.reload_styles(ctx),
        }
    }

    /// Loads the project's stylesheets again in the background, after which the charts restyle.
    fn reload_styles(&mut self, ctx: &egui::Context) {
        let generation = self.generation;
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        workspace.styling = true;
        let files = workspace.files().clone();
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        drop(self.runtime.spawn(move || async move {
            let styles = Styles::load(&files).await;
            let _ = sender.send(Message::Styled { generation, styles });
            ctx.request_repaint();
        }));
    }

    /// Loads the data scripts of `load` again in the background. The workspace's `loading` holds
    /// the load by its key while it runs: the path of a script, the paths of changed files, or
    /// the empty key for every data script.
    fn reload(&mut self, ctx: &egui::Context, load: Load) {
        let generation = self.generation;
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let key = match &load {
            Load::Script { path, .. } => path.clone(),
            Load::Files(paths) => paths.join(", "),
            Load::All => String::new(),
        };
        workspace.loading.insert(key.clone());
        let loader = workspace.loader().clone();
        let (sender, ctx) = (self.messages.0.clone(), ctx.clone());
        // A load is never stopped: it changes the session as it goes.
        drop(self.runtime.spawn(move || async move {
            let (result, charts) = match load {
                Load::Script { path, text } => (loader.reload(&path, text).await.map(Some), vec![]),
                Load::Files(paths) => match loader.reload_files(&paths).await {
                    Some((reload, state)) => (Ok(Some((reload.tables, state))), reload.charts),
                    None => (Ok(None), Vec::new()),
                },
                Load::All => (loader.reload_all().await.map(Some), Vec::new()),
            };
            let _ = sender.send(Message::Reloaded {
                generation,
                key,
                result,
                charts,
            });
            ctx.request_repaint();
        }));
    }

    /// Shows the tab of the file at `path`, opening it, and runs a chart script that opens, whose
    /// result the result pane shows.
    fn open_tab(&mut self, ctx: &egui::Context, state: &RenderState, path: &str) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let shown = |workspace: &Workspace| workspace.shown().map(|tab| tab.path.clone());
        let before = shown(workspace);
        let opened = workspace.open_tab(path);
        let chart =
            (workspace.tab(path)).is_some_and(|tab| tab.kind == TabKind::Sql(FileRole::Chart));
        if shown(workspace) != before {
            self.present(state);
        }
        if chart && self.tab == Tab::Gallery {
            self.tab = Tab::Result;
        }
        if opened == Some(true) && chart {
            self.run_tab(ctx, path);
        }
    }

    /// Closes the tab of the file at `path`.
    fn close_tab(&mut self, state: &RenderState, path: &str) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let shown = |workspace: &Workspace| workspace.shown().map(|tab| tab.path.clone());
        let before = shown(workspace);
        if let Some(mut tab) = workspace.close_tab(path) {
            tab.buffer.cancel();
        }
        if shown(workspace) != before {
            self.present(state);
        }
    }

    fn cancel(&mut self) {
        if self.shown_mut().is_some_and(Buffer::cancel) {
            self.notice = Some("Canceled the query.".to_string());
        }
    }

    /// Asks before the window closes while a tab holds edits that are not saved, natively, since
    /// a web page has no close of its own. A run that saves a screenshot closes as it is.
    #[cfg(not(target_arch = "wasm32"))]
    fn ask_before_close(&mut self, ctx: &egui::Context) {
        let modified = (self.workspace.as_ref()).is_some_and(Workspace::modified);
        let closing = ctx.input(|input| input.viewport().close_requested());
        if closing && modified && self.screenshot.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Confirm::Close);
        }
    }

    /// The dialog of the edits that are not saved, before another project opens or the window
    /// closes: Save all saves every modified tab, except in the sample project, which can't be
    /// saved, Discard drops their edits, and either goes on, while Cancel keeps the open project
    /// and its edits. A save that fails cancels.
    fn confirm(&mut self, ctx: &egui::Context) {
        if self.confirm.is_none() {
            return;
        }
        let read_only = (self.workspace.as_ref()).is_some_and(Workspace::read_only);
        let (mut save, mut discard, mut cancel) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("unsaved")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label("Some files have changes that are not saved.");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                // The sample project's edits run in memory, and can't be saved.
                save = !read_only && ui.button("Save all").clicked();
                discard = ui.button("Discard").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if cancel || modal.should_close() {
            self.confirm = None;
            return;
        }
        if !save && !discard {
            return;
        }
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if save {
            let modified: Vec<String> = (workspace.tabs().iter())
                .filter(|tab| tab.modified())
                .map(|tab| tab.path.clone())
                .collect();
            for path in modified {
                if let Err(error) = workspace.save(&path) {
                    self.notice = Some(format!("Cannot save: {error}"));
                    self.confirm = None;
                    return;
                }
            }
        } else {
            workspace.discard();
        }
        match self.confirm.take() {
            Some(Confirm::Open {
                request,
                workspace,
                file,
            }) => {
                let opened = Ok((workspace, file));
                let _ = self.messages.0.send(Message::Opened { request, opened });
                ctx.request_repaint();
            }
            #[cfg(not(target_arch = "wasm32"))]
            Some(Confirm::Close) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            None => {}
        }
    }

    fn receive(&mut self, ctx: &egui::Context, state: &RenderState) {
        while let Ok(message) = self.messages.1.try_recv() {
            match message {
                Message::Ran {
                    owner,
                    id,
                    outcome,
                    elapsed,
                } => {
                    let shown = self.shown_owner().as_ref() == Some(&owner);
                    let Some(buffer) = self.buffer_mut(&owner) else {
                        continue;
                    };
                    if buffer.running.as_ref().map(|running| running.id) != Some(id) {
                        // A run that the user canceled or replaced.
                        continue;
                    }
                    buffer.running = None;
                    match outcome {
                        Ok(outcome) => {
                            buffer.error = None;
                            let Outcome { result, plan } = outcome;
                            buffer.shown = Some(Shown {
                                id,
                                result,
                                plan,
                                elapsed,
                            });
                            if shown {
                                self.notice = None;
                                self.show_result(state);
                            }
                        }
                        Err(error) => buffer.error = Some(error),
                    }
                }
                Message::FileTables { generation, tables } => {
                    if let (Some(workspace), true) =
                        (&mut self.workspace, generation == self.generation)
                    {
                        workspace.set_file_tables(tables);
                    }
                }
                Message::Lifted { owner, id, lifted } => {
                    if let Some(buffer) = self.buffer_mut(&owner) {
                        if buffer.latest == id {
                            buffer.lifted = Some(lifted);
                        }
                    }
                }
                Message::Card {
                    generation,
                    index,
                    version,
                    result,
                } => {
                    if generation == self.generation {
                        self.gallery.ran(index, version, result);
                    }
                }
                Message::Opened { request, opened } => {
                    if request != self.requests {
                        continue;
                    }
                    self.opening = None;
                    match opened {
                        Ok((workspace, file)) => {
                            // The edits of the open project that are not saved wait for the
                            // dialog's answer.
                            if (self.workspace.as_ref()).is_some_and(Workspace::modified) {
                                self.confirm = Some(Confirm::Open {
                                    request,
                                    workspace,
                                    file,
                                });
                                continue;
                            }
                            // The runs and the loads of the project before it end unseen.
                            self.generation += 1;
                            if let Some(old) = self.workspace.replace(*workspace) {
                                let runs = (old.tabs().iter())
                                    .filter_map(|tab| tab.buffer.running.as_ref());
                                for running in runs {
                                    running.task.abort();
                                }
                            }
                            // A project opens without the files of the directories that
                            // don't list, which the status bar names.
                            self.notice = (self.workspace.as_ref())
                                .map(|workspace| workspace.files().skipped())
                                .filter(|skipped| !skipped.is_empty())
                                .map(|skipped| {
                                    let skipped: Vec<String> = (skipped.iter())
                                        .map(|skipped| {
                                            format!("{} ({})", skipped.path, skipped.error)
                                        })
                                        .collect();
                                    format!(
                                        "The project leaves out the files of the folders that \
                                         cannot be listed: {}",
                                        skipped.join(", ")
                                    )
                                });
                            #[cfg(not(target_arch = "wasm32"))]
                            self.watch_project(ctx);
                            self.tab = Tab::Gallery;
                            let entries = (self.workspace.as_ref())
                                .map(Workspace::gallery)
                                .unwrap_or_default();
                            self.gallery.set_entries(entries, state);
                            self.restyle(ctx, state);
                            self.present(state);
                            if let Some(file) = file {
                                self.open_tab(ctx, state, &file);
                            }
                        }
                        Err(error) => {
                            self.notice = Some(format!("Cannot open the project: {error}"));
                        }
                    }
                }
                Message::Reloaded {
                    generation,
                    key,
                    result,
                    charts,
                } => {
                    if generation != self.generation {
                        continue;
                    }
                    let Some(workspace) = &mut self.workspace else {
                        continue;
                    };
                    workspace.loading.remove(&key);
                    match result {
                        Ok(None) => {}
                        Ok(Some((changes, load))) => {
                            workspace.apply(load);
                            let reruns = workspace.reruns(&changes, &charts);
                            self.gallery.rerun(|entry| {
                                workspace::reads(&entry.text, &changes)
                                    || charts.contains(&entry.path)
                            });
                            for path in reruns {
                                self.run_tab(ctx, &path);
                            }
                        }
                        Err(error) => {
                            let what = match key.is_empty() {
                                true => "the project".to_string(),
                                false => key,
                            };
                            self.notice = Some(format!("Cannot load {what}: {error}"));
                        }
                    }
                }
                #[cfg(not(target_arch = "wasm32"))]
                Message::Copied { generation, copied } => {
                    if generation == self.generation {
                        self.added(ctx, state, copied);
                    }
                }
                Message::Styled { generation, styles } => {
                    if generation != self.generation {
                        continue;
                    }
                    let Some(workspace) = &mut self.workspace else {
                        continue;
                    };
                    workspace.styling = false;
                    match styles {
                        Ok(styles) => {
                            workspace.set_styles(styles);
                            self.restyle(ctx, state);
                        }
                        Err(error) => {
                            self.notice = Some(format!("Cannot load the stylesheets: {error}"));
                        }
                    }
                }
            }
        }
        while let Ok(scene) = self.scenes.1.try_recv() {
            match scene.result == THUMBNAILS {
                true => self.gallery.receive(scene),
                false => self.cells.receive(scene),
            }
        }
    }

    /// Watches the root of the open project, in place of the watch of the project before it. The
    /// sample project, which has no root, has none.
    #[cfg(not(target_arch = "wasm32"))]
    fn watch_project(&mut self, ctx: &egui::Context) {
        self.watch = None;
        let Some(root) = self.workspace.as_ref().and_then(Workspace::root) else {
            return;
        };
        match crate::watch::Watch::new(root, ctx.clone()) {
            Ok(watch) => self.watch = Some(watch),
            Err(error) => {
                self.notice = Some(format!(
                    "The project's files load again only when its data scripts are saved: \
                     cannot watch {}: {error}",
                    root.display()
                ));
            }
        }
    }

    /// Shows the new result of the shown buffer: the chart of the pane takes a single chart and
    /// keeps its pans and zooms, and the cells of a table start over.
    fn show_result(&mut self, state: &RenderState) {
        let Some(shown) = self.shown().and_then(|buffer| buffer.shown.as_ref()) else {
            return;
        };
        let id = shown.id;
        let chart = match shown.result.shape() {
            Shape::Chart => shown.result.chart(0, 0).cloned(),
            Shape::Table => None,
        };
        self.cells.reset(id, state);
        self.live_cells.reset(id, state);
        if let Some((_, mut opened)) = self.opened.take() {
            opened.clear(state);
        }
        match (chart, &self.pane.chart) {
            (Some(chart), Some(pane)) => pane.set_chart(chart),
            // A new chart of the pane builds when the pane lays out, at the pane's size.
            _ => self.pane.clear(state),
        }
    }

    /// Shows the result of the buffer that became the shown buffer, from the start: its chart
    /// builds again, and its cells draw again.
    fn present(&mut self, state: &RenderState) {
        let id = (self.shown())
            .and_then(|buffer| buffer.shown.as_ref())
            .map_or(0, |shown| shown.id);
        self.cells.reset(id, state);
        self.live_cells.reset(id, state);
        if let Some((_, mut opened)) = self.opened.take() {
            opened.clear(state);
        }
        self.pane.clear(state);
    }

    /// Restyles the charts when the editor's visuals change, as when the theme turns dark, so that
    /// each chart's background stays the panel's color.
    fn follow_theme(&mut self, ctx: &egui::Context, state: &RenderState) {
        let theme = ChartTheme::of(&ctx.style().visuals);
        if theme == self.theme {
            return;
        }
        self.theme = theme;
        self.restyle(ctx, state);
    }

    /// Restyles the charts with the cascade of the theme and the open project's stylesheets, and
    /// the reader's preferences of the theme: the live charts keep their views and selections,
    /// and the static charts of the cells and the gallery evaluate again.
    fn restyle(&mut self, ctx: &egui::Context, state: &RenderState) {
        let sheet = (self.workspace.as_ref()).and_then(|workspace| workspace.styles().sheet());
        self.cascade = self.theme.cascade(sheet);
        self.preferences = self.theme.preferences();
        let panes = std::iter::once(&self.pane).chain(self.opened.as_ref().map(|(_, pane)| pane));
        for chart in panes.filter_map(|pane| pane.chart.as_ref()) {
            chart.restyle(self.cascade.clone(), self.preferences.clone());
        }
        self.live_cells.restyle(&self.cascade, &self.preferences);
        self.cells.restyle(state);
        self.gallery.cells.restyle(state);
        ctx.request_repaint();
    }

    /// Formats the active buffer, unless it has an error of the syntax.
    fn format(&mut self, ctx: &egui::Context) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        let Some((tab, language, _)) = workspace.active_with_context() else {
            return;
        };
        let Some(document) = language.document(&tab.path) else {
            return;
        };
        if document.text() != tab.buffer.text || format::blocked(document).is_some() {
            return;
        }
        if let Some(formatted) = format::formatted(document) {
            (tab.buffer.editor).format(ctx, &mut tab.buffer.text, &formatted);
        }
    }

    /// Why the active buffer can't be formatted, as the Format button says it.
    fn format_blocked(&self) -> Option<&'static str> {
        let workspace = self.workspace.as_ref()?;
        let tab = workspace.active()?;
        if tab.kind == TabKind::Stylesheet {
            return Some("The editor formats SQL, and not stylesheets");
        }
        let document = workspace.language().document(&tab.path)?;
        format::blocked(document)
    }

    fn busy(&self) -> bool {
        let tabs = (self.workspace.iter())
            .flat_map(|workspace| workspace.tabs())
            .any(|tab| tab.buffer.running.is_some());
        tabs || self.opening.is_some()
            || (self.workspace)
                .as_ref()
                .is_some_and(|workspace| !workspace.loading.is_empty() || workspace.styling)
            || (self.tab == Tab::Gallery && self.gallery.busy())
            || self.cells.busy()
            || self.pane.busy()
            || self.opened.as_ref().is_some_and(|(_, pane)| pane.busy())
            || self.live_cells.busy()
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.menu_button("Project", |ui| {
                if ui.button("Open project…").clicked() {
                    ui.close();
                    self.pick_project(ctx, false);
                }
                if ui.button("New project…").clicked() {
                    ui.close();
                    self.pick_project(ctx, true);
                }
                if ui.button("Sample project").clicked() {
                    ui.close();
                    self.open_sample(ctx, None);
                }
                ui.separator();
                let writable =
                    (self.workspace.as_ref()).is_some_and(|workspace| !workspace.read_only());
                let add = ui.add_enabled(writable, egui::Button::new("Add file…"));
                let add = add
                    .on_hover_text(
                        "Add data files to the project: a CREATE EXTERNAL TABLE statement for \
                         each, in a data script",
                    )
                    .on_disabled_hover_text(
                        "A sample can't take files: open or make a project to add files to.",
                    );
                if add.clicked() {
                    ui.close();
                    self.pick_files(ctx);
                }
            });
            ui.separator();
            let title = match &self.workspace {
                Some(workspace) => {
                    let name = match workspace.read_only() {
                        true => format!("{} (read-only)", workspace.name()),
                        false => workspace.name().to_string(),
                    };
                    match workspace.active() {
                        Some(tab) => format!("{name} · {}", tab.path),
                        None => name,
                    }
                }
                None => String::new(),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let runs = (self.workspace.as_ref())
                    .and_then(Workspace::active)
                    .is_some_and(|tab| tab.kind == TabKind::Sql(FileRole::Chart));
                let run = ui.add_enabled(runs, egui::Button::new("Run"));
                let run = run
                    .on_hover_text(format!(
                        "Run the chart ({})",
                        ctx.format_shortcut(&run_shortcut())
                    ))
                    .on_disabled_hover_text(
                        "Save a data script to load it, or a stylesheet to restyle the charts, \
                         and run a chart",
                    );
                if run.clicked() {
                    self.run(ctx);
                }
                if let Some(workspace) = &self.workspace {
                    let modified = workspace.active().is_some_and(|tab| tab.modified());
                    let saves = modified && !workspace.read_only();
                    let save = ui.add_enabled(saves, egui::Button::new("Save"));
                    let save = save.on_hover_text(format!(
                        "Save the file ({})",
                        ctx.format_shortcut(&save_shortcut())
                    ));
                    let save = match workspace.read_only() {
                        true => save.on_disabled_hover_text(
                            "A sample can't be saved: its edits run in memory. Open or make a \
                             project to keep your charts.",
                        ),
                        false => save,
                    };
                    if save.clicked() {
                        self.save(ctx);
                    }
                }
                let blocked = self.format_blocked();
                let formats = blocked.is_none() && self.active().is_some();
                let button = ui.add_enabled(formats, egui::Button::new("Format"));
                let button = button
                    .on_hover_text(format!(
                        "Format the text ({})",
                        ctx.format_shortcut(&format::shortcut())
                    ))
                    .on_disabled_hover_text(blocked.unwrap_or_default());
                if button.clicked() {
                    self.format(ctx);
                }
                // System, Light, or Dark, which egui keeps in its memory across launches.
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
                // The title takes the rest of the bar, and a long one ends in an ellipsis.
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(title).strong()).truncate());
                });
            });
        });
    }

    /// The tabs of the open files of the project, each with its name, a dot while its text
    /// differs from its file, and a button that closes it.
    fn tab_strip(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &RenderState) {
        let Some(workspace) = &self.workspace else {
            return;
        };
        let active = workspace.active().map(|tab| tab.path.clone());
        let mut open = None;
        let mut close = None;
        egui::ScrollArea::horizontal()
            .id_salt("tabs")
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for tab in workspace.tabs() {
                        let name = match tab.modified() {
                            true => format!("● {}", tab.name()),
                            false => tab.name().to_string(),
                        };
                        let selected = active.as_deref() == Some(tab.path.as_str());
                        let label = ui.selectable_label(selected, name).on_hover_text(&tab.path);
                        if label.clicked() {
                            open = Some(tab.path.clone());
                        }
                        let button = ui.small_button("×").on_hover_text(match tab.modified() {
                            true => "Close, and lose the changes that are not saved",
                            false => "Close",
                        });
                        if button.clicked() {
                            close = Some(tab.path.clone());
                        }
                        ui.add_space(6.0);
                    }
                    if workspace.tabs().is_empty() {
                        ui.weak("Open a file of the project from the sidebar.");
                    }
                });
            });
        if let Some(path) = open {
            self.open_tab(ctx, state, &path);
        }
        if let Some(path) = close {
            self.close_tab(state, &path);
        }
    }

    /// The code editor of the active tab, analyzed by the project's document of the tab's file,
    /// or of a stylesheet's tab, colored by its tokens with the errors of the last load of the
    /// stylesheets. A definition in another file opens that file.
    fn code_editor(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &RenderState) {
        let Some(workspace) = &mut self.workspace else {
            return;
        };
        if let Some(path) = workspace.active().map(|tab| tab.path.clone()) {
            workspace.sync(&path);
        }
        let Some((tab, language, styles)) = workspace.active_with_context() else {
            return;
        };
        let text = match tab.kind {
            TabKind::Sql(_) => {
                let Some(analysis) = Analysis::file(language, &tab.path) else {
                    return;
                };
                Text::Sql(analysis)
            }
            TabKind::Stylesheet => Text::Css {
                loaded: styles.text(&tab.path).unwrap_or_default(),
                diagnostics: styles.diagnostics(&tab.path),
            },
        };
        let output = (tab.buffer.editor).show(ui, &mut tab.buffer.text, text);
        if output.changed {
            // The error points into the text before the edit.
            if let Some(error) = self.active_mut().and_then(|buffer| buffer.error.as_mut()) {
                error.range = None;
            }
        }
        if let Some(Target {
            path: Some(path),
            range,
        }) = output.jump
        {
            self.open_tab(ctx, state, &path);
            if let Some(buffer) = self.active_mut() {
                buffer.editor.select(ctx, range);
            }
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let reloading = (self.workspace.as_ref()).and_then(|workspace| {
                let path = workspace.loading.iter().next()?;
                Some(match path.is_empty() {
                    true => "Loading the project…".to_string(),
                    false => format!("Loading {path}…"),
                })
            });
            let buffer = self.shown();
            if let Some(running) = buffer.and_then(|buffer| buffer.running.as_ref()) {
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
            } else if let Some(name) = &self.opening {
                ui.spinner();
                ui.label(format!("Opening {name}…"));
            } else if let Some(reloading) = reloading {
                ui.spinner();
                ui.label(reloading);
            } else if (self.workspace.as_ref()).is_some_and(|workspace| workspace.styling) {
                ui.spinner();
                ui.label("Loading the stylesheets…");
            } else if let Some(error) = buffer.and_then(|buffer| buffer.error.as_ref()) {
                let first = error.message.lines().next().unwrap_or_default();
                ui.add(
                    egui::Label::new(egui::RichText::new(first).color(ui.visuals().error_fg_color))
                        .truncate(),
                )
                .on_hover_text(&error.message);
            } else if let Some(notice) = &self.notice {
                ui.add(egui::Label::new(notice).truncate());
            } else if let Some(shown) = buffer.and_then(|buffer| buffer.shown.as_ref()) {
                let rows = shown.result.num_rows();
                let noun = if rows == 1 { "row" } else { "rows" };
                ui.label(format!("{rows} {noun} · {} ms", shown.elapsed.as_millis()));
            } else if (self.workspace.as_ref()).is_some_and(Workspace::read_only) {
                ui.weak(
                    "Open a chart to run it. A sample's edits run in memory, and Project opens \
                     or makes a project of your own.",
                );
            } else if self.workspace.is_some() {
                ui.weak(
                    "Open a chart to run it, save a data script to load it, and save a \
                     stylesheet to restyle the charts.",
                );
            }
        });
    }

    fn results(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &RenderState) {
        let buffer = self.shown();
        let shown = buffer.and_then(|buffer| buffer.shown.as_ref());
        let failed = buffer.is_some_and(|buffer| buffer.error.is_some());
        let charts_in_table = shown.is_some_and(|shown| {
            shown.result.shape() == Shape::Table && !shown.result.chart_columns().is_empty()
        });
        let stale = failed && shown.is_some();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, Tab::Gallery, "Gallery");
            ui.selectable_value(&mut self.tab, Tab::Result, "Result");
            ui.selectable_value(&mut self.tab, Tab::Lifted, "Lifted SQL");
            ui.selectable_value(&mut self.tab, Tab::Plan, "Plan");
            if stale && self.tab != Tab::Gallery {
                ui.weak("· the last result that ran");
            }
            if self.tab == Tab::Gallery {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.gallery.search_box(ui);
                });
            }
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
            Tab::Gallery => {
                let env = Env {
                    state,
                    runtime: &self.runtime,
                    scenes: &self.scenes.0,
                    cascade: &self.cascade,
                    preferences: &self.preferences,
                    engine: &self.engine,
                    images: &self.images,
                };
                let output = self.gallery.show(ui, &env, self.workspace.is_some());
                self.run_cards(ctx, output.start);
                if let Some(path) = output.open {
                    // A card runs its chart, in a tab that opens now or one already open.
                    let open = (self.workspace.as_ref())
                        .is_some_and(|workspace| workspace.tab(&path).is_some());
                    self.open_tab(ctx, state, &path);
                    if open {
                        self.run_tab(ctx, &path);
                    }
                }
            }
            Tab::Result => self.result_tab(ui, state),
            Tab::Plan => {
                let plan = (self.shown())
                    .and_then(|buffer| buffer.shown.as_ref())
                    .map_or("", |shown| shown.plan.as_str());
                code(ui, plan);
            }
            Tab::Lifted => match self.shown().and_then(|buffer| buffer.lifted.as_ref()) {
                None => {
                    ui.weak("The lifted query shows here after the chart runs.");
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
        let shown = (self.workspace.as_ref())
            .and_then(Workspace::shown)
            .and_then(|tab| tab.buffer.shown.as_ref());
        let Some(shown) = shown else {
            ui.weak("The result of the chart shows here.");
            return;
        };
        let env = Env {
            state,
            runtime: &self.runtime,
            scenes: &self.scenes.0,
            cascade: &self.cascade,
            preferences: &self.preferences,
            engine: &self.engine,
            images: &self.images,
        };
        if shown.result.shape() == Shape::Chart {
            let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
            let chart = shown.result.chart(0, 0).expect("a single chart");
            let id = egui::Id::new(("live chart", shown.id));
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
                let id = egui::Id::new(("opened chart", shown.id, key));
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
        let editor = (self.workspace.as_ref())
            .and_then(Workspace::active)
            .map(|tab| &tab.buffer.editor);
        let driven = match (&mut screenshot.drive, editor) {
            (Some(drive), Some(editor)) => settled && drive.step(ctx, editor),
            _ => true,
        };
        if !screenshot.requested && ((settled && driven) || timed_out) {
            screenshot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
        }
        ctx.request_repaint();
    }
}

impl eframe::App for EditorApp {
    /// egui's memory, with the theme of the theme buttons, persists across launches, except from
    /// a run that saves a screenshot, which leaves the remembered theme as it was.
    fn persist_egui_memory(&self) -> bool {
        self.screenshot.is_none()
    }

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
        self.ask_before_close(ctx);
        self.confirm(ctx);
        self.follow_theme(ctx, &state);
        #[cfg(not(target_arch = "wasm32"))]
        self.limit_window(ctx, &state);
        let dropped: Vec<egui::DroppedFile> = ctx.input(|input| input.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.drop_files(ctx, dropped);
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.follow_files(ctx);
        if ctx.input_mut(|input| input.consume_shortcut(&run_shortcut())) {
            self.run(ctx);
        }
        if ctx.input_mut(|input| input.consume_shortcut(&save_shortcut())) {
            self.save(ctx);
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
                let Some(workspace) = &self.workspace else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.weak("Opening the project…");
                    });
                    return;
                };
                let active = workspace.active().map(|tab| tab.path.as_str());
                let output = sidebar::project(
                    ui,
                    workspace.name(),
                    &workspace.sidebar(),
                    active,
                    &workspace.loading,
                    &self.functions,
                );
                if let Some(path) = output.open {
                    self.open_tab(ctx, &state, &path);
                }
                if let Some(insert) = output.insert {
                    if let Some(buffer) = self.active_mut() {
                        buffer.editor.insert(ctx, &mut buffer.text, &insert);
                    }
                }
            });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| self.status_bar(ui));
        egui::TopBottomPanel::top("editor")
            .resizable(true)
            .default_height(240.0)
            .show(ctx, |ui| {
                self.toolbar(ui, ctx);
                ui.separator();
                self.tab_strip(ui, ctx, &state);
                ui.separator();
                self.code_editor(ui, ctx, &state);
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
                env.preferences.clone(),
                env.engine.clone(),
                env.images,
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

fn run_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Enter)
}

fn save_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)
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
        let text = buffer.as_str();
        let job = editor::layout_job(
            text,
            &highlight::spans(text, &highlight::syntax(text)),
            egui::FontId::monospace(13.0),
            marked,
            background,
            ui.visuals().dark_mode,
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
