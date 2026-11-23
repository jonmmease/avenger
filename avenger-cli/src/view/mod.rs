//! `avenger view`: a native window on the result of a chart script, or of a cell of a Quarto
//! document, which stays current as the project's files change. The window shows the result with
//! avenger-results-egui: the chart alone, live, or the table of the result, whose chart cells open
//! their charts. Two buttons switch the color scheme, which starts from the
//! system's, or from `--color-scheme`.
//!
//! A thread of its own keeps the [`Session`]: it opens the project, runs the chart, watches the
//! project's files, and sends what each burst of changes does to the window, with the window's
//! title, which follows a statement's line. A run that fails keeps the result before, and its
//! errors show in a bar at the bottom of the window and on standard error. The window opens at
//! once, in front, and says that the project loads until it opens.

mod session;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use avenger_results_egui::{Exec, Images, ResultView, Shared, Surface};
use avenger_sql_project::charts::label_engine;
use avenger_sql_project::watch::{Burst, Watch};
use avenger_style::{Cascade, ColorScheme, Preferences, Sheet};
use avenger_typst_label::LabelEngine;
use eframe::egui;

pub use session::{Outcome, Session};

/// What the session's thread sends the window.
enum Message {
    /// The project opened, with its stylesheet, or not.
    Opened(Result<Option<Sheet>, String>),
    Outcome(Outcome),
    /// The window's title: the chart's file, with its statement's line now, its cell, and what
    /// its statement shows.
    Title(String),
}

/// Opens a window on `chart`, a chart script, `doc.qmd#NAME`, or a statement as `FILE:LINE`, in
/// the color scheme `scheme`, or the system's, until the window closes.
pub fn run(chart: String, scheme: Option<ColorScheme>) -> Result<(), String> {
    let title = format!("avenger view {chart}");
    let options = eframe::NativeOptions {
        // The window opens in front, with focus, though a terminal starts it.
        viewport: egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size([960.0, 720.0])
            .with_active(true),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            let window = Window::new(cc, chart, scheme).map_err(|error| error.to_string())?;
            Ok(Box::new(window))
        }),
    )
    .map_err(|error| error.to_string())
}

/// The window.
struct Window {
    runtime: Exec,
    view: ResultView,
    images: Images,
    engine: LabelEngine,
    /// The project's entry stylesheet.
    sheet: Option<Sheet>,
    surface: Surface,
    cascade: Arc<Cascade>,
    preferences: Preferences,
    /// The errors of the last run, or of a stylesheet, which the bar shows.
    errors: Vec<String>,
    /// Whether the project has not opened yet.
    opening: bool,
    messages: Receiver<Message>,
    /// Whether the window closed, which ends the session's thread.
    closed: Arc<AtomicBool>,
    /// Whether the window asked for focus, which it does on its first frame.
    focused: bool,
}

impl Window {
    fn new(
        cc: &eframe::CreationContext<'_>,
        chart: String,
        scheme: Option<ColorScheme>,
    ) -> Result<Self, String> {
        match scheme {
            Some(ColorScheme::Dark) => cc.egui_ctx.set_theme(egui::Theme::Dark),
            Some(ColorScheme::Light) => cc.egui_ctx.set_theme(egui::Theme::Light),
            None => cc.egui_ctx.set_theme(egui::ThemePreference::System),
        }
        let runtime = Exec::new().map_err(|error| error.to_string())?;
        let (sender, messages) = channel();
        let closed = Arc::new(AtomicBool::new(false));
        {
            let (ctx, closed) = (cc.egui_ctx.clone(), closed.clone());
            std::thread::Builder::new()
                .name("avenger-view".into())
                .spawn(move || follow(chart, sender, ctx, closed))
                .map_err(|error| error.to_string())?;
        }
        let surface = Surface::of(&cc.egui_ctx.style().visuals);
        Ok(Self {
            runtime,
            view: ResultView::new(),
            images: Images::new(&cc.egui_ctx, crate::render::image_options()?),
            engine: label_engine(),
            sheet: None,
            cascade: surface.cascade([]),
            preferences: surface.preferences(),
            surface,
            errors: Vec::new(),
            opening: true,
            messages,
            closed,
            focused: false,
        })
    }

    /// The charts' shared state on `state`.
    fn shared<'a>(&'a self, state: &'a eframe::egui_wgpu::RenderState) -> Shared<'a> {
        Shared {
            state,
            runtime: &self.runtime,
            cascade: &self.cascade,
            preferences: &self.preferences,
            engine: &self.engine,
            images: &self.images,
        }
    }

    /// Restyles the charts with the surface of the window's visuals and the project's stylesheet.
    fn restyle(&mut self, state: &eframe::egui_wgpu::RenderState) {
        self.cascade = self.surface.cascade(self.sheet.iter());
        self.preferences = self.surface.preferences();
        let mut view = std::mem::take(&mut self.view);
        view.restyle(&self.shared(state));
        self.view = view;
    }

    /// Takes the messages of the session's thread.
    fn receive(&mut self, ctx: &egui::Context, state: &eframe::egui_wgpu::RenderState) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::Title(title) => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
                }
                Message::Opened(Ok(sheet)) => {
                    self.opening = false;
                    self.sheet = sheet;
                    self.restyle(state);
                }
                Message::Opened(Err(error)) => {
                    self.opening = false;
                    self.report(vec![error]);
                }
                Message::Outcome(Outcome::Ran(result)) => {
                    self.errors.clear();
                    self.view.set_result(result, state);
                }
                Message::Outcome(Outcome::Failed(errors)) => self.report(errors),
                Message::Outcome(Outcome::Restyled(stylesheet)) => {
                    let errors: Vec<String> = (stylesheet.diagnostics.iter())
                        .map(|diagnostic| match stylesheet.path(diagnostic.span.source) {
                            Some(path) => format!("{path}: error: {}", diagnostic.message),
                            None => format!("error: {}", diagnostic.message),
                        })
                        .collect();
                    if !errors.is_empty() {
                        self.report(errors);
                    }
                    self.sheet = stylesheet.sheet;
                    self.restyle(state);
                }
            }
        }
    }

    /// Shows `errors` in the bar and on standard error.
    fn report(&mut self, errors: Vec<String>) {
        for error in &errors {
            eprintln!("{error}");
        }
        self.errors = errors;
    }
}

impl eframe::App for Window {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let Some(state) = frame.wgpu_render_state() else {
            return;
        };
        if !self.focused {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.focused = true;
        }
        self.receive(ctx, state);
        // The charts follow the window's visuals: their background and their color scheme.
        let surface = Surface::of(&ctx.style().visuals);
        if surface != self.surface {
            self.surface = surface;
            self.restyle(state);
        }
        egui::TopBottomPanel::top("bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.view.shows_chart_cells() {
                    ui.checkbox(&mut self.view.interactive, "Interactive cells")
                        .on_hover_text("The chart cells of a table pan, zoom, and show tooltips");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let dark = ctx.style().visuals.dark_mode;
                    if ui
                        .selectable_label(dark, "🌙")
                        .on_hover_text("Dark")
                        .clicked()
                    {
                        ctx.set_theme(egui::Theme::Dark);
                    }
                    if ui
                        .selectable_label(!dark, "☀")
                        .on_hover_text("Light")
                        .clicked()
                    {
                        ctx.set_theme(egui::Theme::Light);
                    }
                });
            });
        });
        if !self.errors.is_empty() {
            egui::TopBottomPanel::bottom("errors").show(ctx, |ui| {
                let color = ui.visuals().error_fg_color;
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for error in &self.errors {
                            ui.label(egui::RichText::new(error).monospace().color(color));
                        }
                    });
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.opening {
                // A project's data scripts can take a while to load.
                ui.centered_and_justified(|ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading the project…");
                    });
                });
                return;
            }
            let mut view = std::mem::take(&mut self.view);
            let error = view.show(ui, &self.shared(state));
            self.view = view;
            if let Some(error) = error {
                self.report(vec![error]);
            }
        });
        if self.view.busy() {
            ctx.request_repaint();
        }
    }
}

impl Drop for Window {
    /// Ends the session's thread with the window.
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// The session's thread: opens the project of `chart`, runs the chart, and follows the changes of
/// its files, sending each outcome to the window and waking it, until the window closes.
fn follow(chart: String, sender: Sender<Message>, ctx: egui::Context, closed: Arc<AtomicBool>) {
    let send = |message: Message| {
        let sent = sender.send(message).is_ok();
        ctx.request_repaint();
        sent
    };
    // A loaded project's futures are not `Send`, so they run on this thread.
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        send(Message::Opened(Err(
            "the session's runtime does not start".into()
        )));
        return;
    };
    let (mut session, outcome) = match runtime.block_on(Session::open(&chart)) {
        Ok(opened) => opened,
        Err(error) => {
            send(Message::Opened(Err(error)));
            return;
        }
    };
    let sheet = session.stylesheet().sheet.clone();
    let mut title = session.title();
    if !send(Message::Opened(Ok(sheet)))
        || !send(Message::Title(title.clone()))
        || !send(Message::Outcome(outcome))
    {
        return;
    }
    let (wake, wakes) = crossbeam_channel::unbounded();
    let Ok(mut watch) = Watch::new(&session.watched_directory(), move || {
        let _ = wake.send(());
    }) else {
        send(Message::Outcome(Outcome::Failed(vec![format!(
            "{}: the window cannot watch its files, so it shows the chart as it first ran",
            session.watched_directory().display()
        )])));
        return;
    };
    while !closed.load(Ordering::SeqCst) {
        let wait = match watch.changed() {
            Burst::Ended(paths) => {
                let paths: Vec<PathBuf> = (paths.iter()).map(|path| canonical(path)).collect();
                for outcome in runtime.block_on(session.changed(&paths)) {
                    if !send(Message::Outcome(outcome)) {
                        return;
                    }
                }
                // The statement's line follows edits around it.
                if session.title() != title {
                    title = session.title();
                    if !send(Message::Title(title.clone())) {
                        return;
                    }
                }
                continue;
            }
            Burst::Waiting(time) => time,
            Burst::Quiet => Duration::from_millis(500),
        };
        let _ = wakes.recv_timeout(wait);
    }
}

/// The canonical path of `path`, or for a file that went away, its directory's with its name.
fn canonical(path: &std::path::Path) -> PathBuf {
    match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(_) => match (
            path.parent().and_then(|p| std::fs::canonicalize(p).ok()),
            path.file_name(),
        ) {
            (Some(parent), Some(name)) => parent.join(name),
            _ => path.to_path_buf(),
        },
    }
}
