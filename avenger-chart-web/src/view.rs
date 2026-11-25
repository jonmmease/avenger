//! The JavaScript interface: `addFonts`, and `ChartView`, which runs the app of a chart in a
//! canvas, takes the canvas's DOM events, and draws through the page's one WebGPU device.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap, VecDeque},
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
};

use avenger_app::app::{AvengerApp, Changes};
use avenger_chart::{ChangeMessage, Chart};
use avenger_chart_app::{
    Carry, ChartApp, ChartAppState, ChartChange, ChartUpdate, ChartUpdates, Fit, Reports, Views,
};
use avenger_common::{canvas::CanvasDimensions, time::Instant};
use avenger_eventstream::window::{ElementState, WindowEvent, WindowResizeEvent};
use avenger_selection::Clause;
use avenger_style::{ColorScheme, Sheet};
use avenger_typst_label::{LabelEngine, RegisteredFont};
use avenger_wgpu::{
    canvas::CanvasConfig,
    html_canvas::{BrowserGpu, HtmlCanvasCanvas},
};
use js_sys::{Array, Function, Object, Promise, Reflect, Uint8Array};
use serde::Deserialize;
use serde_wasm_bindgen::{from_value, to_value};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};
use web_sys::{
    AddEventListenerOptions, ClipboardEvent, Event, FocusOptions, HtmlCanvasElement, KeyboardEvent,
    MouseEvent, PointerEvent, ResizeObserver, ResizeObserverEntry, TouchEvent, WheelEvent,
};

use crate::{
    fonts::{label_engine, read_fonts},
    input::{self, Touches, Wheel},
    styles::{cascade, load, ProjectStyles},
};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** The options of `ChartView.create`. */
export interface ViewOptions {
  /** The project's stylesheets, which style the chart after the base stylesheet. */
  styles?: ProjectStyles;
  /** The reader's color scheme, which sets the chart's background and its scheme's styles. */
  colorScheme?: "light" | "dark";
  /** Whether the canvas and the chart's own background are transparent, so the page shows
   * through, or filled with the scheme's background. */
  transparent?: boolean;
  /** When wheel events reach the chart: while its canvas has the focus, or always. */
  wheel?: "focus" | "always";
  /** Hears each change of the chart by its reader, after the view applies it. */
  onChange?: (change: ChartChange) => void;
  /** Which changes `onChange` hears: each one, such as each move of a brush's drag, or the
   * changes when a gesture settles, with the selections that the gesture wrote before it. */
  reports?: "every" | "settled";
}

/** A change of a chart by its reader: the bytes of its change message, which a server reads
 * with avenger-chart's `ChangeMessage` and `setState` applies, the names of the selections that
 * it wrote, the modes that are on when it holds the modes, and whether its gesture ended. */
export interface ChartChange {
  bytes: Uint8Array;
  selections: string[];
  modes: string[] | null;
  settled: boolean;
}

/** What a new chart of `setChart` carries over from the chart that it replaces. */
export interface UpdateOptions {
  /** The domains of the scales: fitted to the new rows, the old ones, or both. */
  fit?: "refit" | "keep" | "extend";
  /** The views of pans and zooms: kept, or dropped for the fitted domains. */
  views?: "keep" | "drop";
  /** The clauses of the selections: kept, or reset to the new chart's initial clauses. */
  selections?: "keep" | "reset";
}

/** A project's stylesheets: the path of the entry stylesheet, and the text of each stylesheet
 * by its path relative to the project's root, which the entry stylesheet imports by paths
 * relative to itself. */
export interface ProjectStyles {
  entry: string;
  files: Record<string, string>;
}

/** A diagnostic of a project's stylesheet, at a line and a column of its file, both from 1. */
export interface StyleDiagnostic {
  severity: "error" | "warning";
  message: string;
  file: string;
  line: number;
  column: number;
}
"#;

/// The options of `ChartView.create`, as `ViewOptions` declares them. Each is optional, so an
/// option that is missing, `undefined`, or `null` takes its default.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ViewOptions {
    styles: Option<ProjectStyles>,
    color_scheme: Option<Scheme>,
    transparent: Option<bool>,
    wheel: Option<Wheel>,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    on_change: JsValue,
    reports: Option<ReportsOption>,
}

/// Which changes a view reports, as the page names them.
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReportsOption {
    Every,
    Settled,
}

/// The options of `setChart`, as `UpdateOptions` declares them.
#[derive(Default, Deserialize)]
#[serde(default)]
struct UpdateOptionsJs {
    fit: Option<FitOption>,
    views: Option<ViewsOption>,
    selections: Option<CarryOption>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FitOption {
    Refit,
    Keep,
    Extend,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ViewsOption {
    Keep,
    Drop,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum CarryOption {
    Keep,
    Reset,
}

/// A color scheme as the page names it, `"light"` or `"dark"`.
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Scheme {
    #[default]
    Light,
    Dark,
}

impl From<Scheme> for ColorScheme {
    fn from(scheme: Scheme) -> Self {
        match scheme {
            Scheme::Light => ColorScheme::Light,
            Scheme::Dark => ColorScheme::Dark,
        }
    }
}

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// What the page shares among its views: the fonts that it added, the label engine of the
/// fonts, and the WebGPU device.
#[derive(Default)]
struct Page {
    fonts: RefCell<Vec<RegisteredFont>>,
    /// The engine of the fonts, with the number of fonts that it holds.
    engine: RefCell<Option<(usize, LabelEngine)>>,
    /// The promise of the device, which resolves once it opens, and the device then.
    gpu: RefCell<Option<Promise>>,
    device: RefCell<Option<BrowserGpu>>,
    /// The views by id, for the wakers of their apps, which must be `Send` and so name a view by
    /// its id.
    views: RefCell<HashMap<u32, Weak<View>>>,
    next_id: Cell<u32>,
}

thread_local! {
    static PAGE: Page = Page::default();
}

/// Adds the fonts of `file`, one TrueType, OpenType, or collection font, for the views made after
/// it. The module embeds no fonts, so a page adds the faces that `avenger-fonts` bundles first, in
/// their order, and its own fonts after them.
#[wasm_bindgen(js_name = addFonts)]
pub fn add_fonts(file: &[u8]) -> Result<(), JsError> {
    let fonts = read_fonts(file).map_err(JsError::new)?;
    PAGE.with(|page| page.fonts.borrow_mut().extend(fonts));
    Ok(())
}

/// The engine of the page's fonts, which the views share until the page adds fonts.
fn engine() -> LabelEngine {
    PAGE.with(|page| {
        let fonts = page.fonts.borrow();
        let mut engine = page.engine.borrow_mut();
        match &*engine {
            Some((count, engine)) if *count == fonts.len() => engine.clone(),
            _ => {
                let built = label_engine(fonts.clone());
                *engine = Some((fonts.len(), built.clone()));
                built
            }
        }
    })
}

/// The page's WebGPU device, which the first view opens.
async fn gpu() -> Result<BrowserGpu, JsValue> {
    let opening = PAGE.with(|page| {
        page.gpu
            .borrow_mut()
            .get_or_insert_with(|| {
                future_to_promise(async {
                    let gpu = BrowserGpu::new()
                        .await
                        .map_err(|error| JsError::new(&error.to_string()))?;
                    PAGE.with(|page| *page.device.borrow_mut() = Some(gpu));
                    Ok(JsValue::UNDEFINED)
                })
            })
            .clone()
    });
    JsFuture::from(opening).await?;
    Ok(PAGE.with(|page| page.device.borrow().clone().expect("the device opened")))
}

/// What a view's driver handles, in order.
enum Input {
    Event(WindowEvent),
    /// The app's waker asks for a refresh.
    Refresh,
    /// The canvas has a new size in CSS pixels, at the display's pixel ratio.
    Resize([f32; 2], f32),
    /// A frame of the browser to draw in.
    Draw,
}

/// The app of a chart, and the canvas that it draws in.
struct Core {
    app: AvengerApp<ChartAppState>,
    canvas: HtmlCanvasCanvas,
    max_texture: u32,
    /// The dimensions of a resize that the drawing buffer takes at the next frame. A new size
    /// clears the buffer, so the frame that draws into it takes the size, and until then the
    /// browser stretches the last frame instead of showing a cleared canvas.
    resized: Option<CanvasDimensions>,
}

struct View {
    id: u32,
    canvas: HtmlCanvasElement,
    wheel: Wheel,
    updates: ChartUpdates,
    /// The changes that the app reported, which the driver passes to `on_change` after the step
    /// that made them. The app's callback must be `Send`, so it queues them here.
    changes: Arc<Mutex<Vec<ChartChange>>>,
    on_change: Option<Function>,
    /// The state of the app after its last step, which `state()` reads while the driver holds the
    /// app.
    state: RefCell<State>,
    /// The project's stylesheet, loaded once, the color scheme, and whether the view is
    /// transparent, which a change of the stylesheet or of the scheme styles the chart with again.
    sheet: RefCell<Option<Sheet>>,
    scheme: Cell<ColorScheme>,
    transparent: bool,
    /// The app and its canvas, which the driver takes while it awaits the app, and which
    /// `destroy` drops.
    core: RefCell<Option<Core>>,
    queue: RefCell<VecDeque<Input>>,
    driving: Cell<bool>,
    frame_requested: Cell<bool>,
    destroyed: Cell<bool>,
    touches: RefCell<Touches>,
    listeners: RefCell<Vec<Listener>>,
    observer: RefCell<Option<Observer>>,
}

/// What `state()` reads of an app: the clauses of each selection, and the modes that are on, or
/// none for a chart that declares no modes, whose state then turns off no toggle of another chart.
#[derive(Default)]
struct State {
    selections: BTreeMap<String, Vec<Clause>>,
    modes: Option<Vec<String>>,
}

impl State {
    fn of(app: &ChartAppState) -> Self {
        Self {
            selections: (app.evaluator().selections().iter())
                .map(|(name, state)| (name.clone(), state.clauses.clone()))
                .collect(),
            modes: (!app.chart().message().modes.is_empty())
                .then(|| app.modes().into_iter().map(str::to_string).collect()),
        }
    }
}

/// The resize observer of a view's canvas, with the callback that it calls.
type Observer = (ResizeObserver, Closure<dyn FnMut(Array)>);

/// A listener of a view's canvas: the kind of its events, and its callback.
type Listener = (&'static str, Closure<dyn FnMut(Event)>);

/// A chart in a canvas, with the tools that the chart declares. `free()` removes the view's
/// listeners and its resize observer, and frees its GPU resources.
#[wasm_bindgen]
pub struct ChartView {
    view: Rc<View>,
    diagnostics: JsValue,
}

#[wasm_bindgen]
impl ChartView {
    /// Decodes the chart message `bytes`, builds its app at the canvas's CSS size and the
    /// display's pixel ratio, and draws it into `canvas`, with the fonts that the page added.
    /// `options` gives the project's `styles`, the `colorScheme`, light by default, whether the
    /// view is `transparent`, and `wheel`, `"focus"` by default or `"always"`. Rejects with the
    /// decoder's error for an invalid message, and when the browser does not support WebGPU.
    pub async fn create(
        canvas: HtmlCanvasElement,
        bytes: Vec<u8>,
        #[wasm_bindgen(unchecked_optional_param_type = "ViewOptions")] options: Option<Object>,
    ) -> Result<ChartView, JsValue> {
        let chart = Chart::from_bytes(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        let options: ViewOptions = match options {
            Some(options) => from_value(options.into())?,
            None => ViewOptions::default(),
        };
        let scheme = ColorScheme::from(options.color_scheme.unwrap_or_default());
        let transparent = options.transparent.unwrap_or_default();
        let (sheet, diagnostics) = match &options.styles {
            Some(project) => {
                let (sheet, diagnostics) = load(project);
                (Some(sheet), diagnostics)
            }
            None => (None, Vec::new()),
        };
        let engine = engine();
        let gpu = gpu().await?;
        let styles = cascade(sheet.as_ref(), scheme, transparent);

        let size = css_size(&canvas);
        let dimensions = fit(size, pixel_ratio(), gpu.max_texture_dimension());
        let updates = ChartUpdates::new();
        updates.set_preferences(scheme.preferences());
        let changes = Arc::new(Mutex::new(Vec::new()));
        let heard = changes.clone();
        let reports = match options.reports {
            Some(ReportsOption::Settled) => Reports::Settled,
            Some(ReportsOption::Every) | None => Reports::Every,
        };
        let app = ChartApp::new(chart, styles, engine)
            .size(size)
            .updates(&updates)
            .reports(reports)
            .on_change(move |change| {
                if let Ok(mut changes) = heard.lock() {
                    changes.push(change.clone());
                }
            })
            .build()
            .await
            .map_err(|error| JsError::new(&error.to_string()))?;
        let html_canvas =
            HtmlCanvasCanvas::new(&gpu, canvas.clone(), dimensions, CanvasConfig::default())
                .map_err(|error| JsError::new(&error.to_string()))?;

        let id = PAGE.with(|page| {
            let id = page.next_id.get();
            page.next_id.set(id + 1);
            id
        });
        let view = Rc::new(View {
            id,
            canvas: canvas.clone(),
            wheel: options.wheel.unwrap_or_default(),
            updates,
            changes,
            on_change: options.on_change.dyn_into::<Function>().ok(),
            state: RefCell::new(State::of(app.state())),
            sheet: RefCell::new(sheet),
            scheme: Cell::new(scheme),
            transparent,
            core: RefCell::new(Some(Core {
                app,
                canvas: html_canvas,
                max_texture: gpu.max_texture_dimension(),
                resized: None,
            })),
            queue: RefCell::default(),
            driving: Cell::new(false),
            frame_requested: Cell::new(false),
            destroyed: Cell::new(false),
            touches: RefCell::default(),
            listeners: RefCell::default(),
            observer: RefCell::default(),
        });
        PAGE.with(|page| page.views.borrow_mut().insert(id, Rc::downgrade(&view)));
        if let Some(core) = &mut *view.core.borrow_mut() {
            core.app.waker().set_notify(move || wake(id));
            let changes = Changes {
                scenes: (0..core.app.scenes().len()).collect(),
                ..Default::default()
            };
            apply(&view, core, changes);
        }
        prepare_canvas(&canvas)?;
        listen(&view)?;
        observe(&view)?;
        // A refresh that the app asked for while it built waits without a notice.
        push(&view, Input::Refresh);
        Ok(ChartView {
            view,
            diagnostics: to_value(&diagnostics)?,
        })
    }

    /// The diagnostics of the project's stylesheets: objects with `severity`, `message`, `file`,
    /// `line`, and `column`.
    #[wasm_bindgen(getter, unchecked_return_type = "StyleDiagnostic[]")]
    pub fn diagnostics(&self) -> JsValue {
        self.diagnostics.clone()
    }

    /// Replaces the chart with the chart message `bytes`. By default it fits each scale to the
    /// new rows, and keeps the views of the scales and the selections where the new chart has the
    /// same structure; `options` says otherwise, with `fit`, `views`, and `selections`. Throws the
    /// decoder's error for an invalid message.
    #[wasm_bindgen(js_name = setChart)]
    pub fn set_chart(
        &self,
        bytes: Vec<u8>,
        #[wasm_bindgen(unchecked_optional_param_type = "UpdateOptions")] options: Option<Object>,
    ) -> Result<(), JsValue> {
        let chart = Chart::from_bytes(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        let options: UpdateOptionsJs = match options {
            Some(options) => from_value(options.into())?,
            None => UpdateOptionsJs::default(),
        };
        let mut update = ChartUpdate::new(chart);
        if let Some(fit) = options.fit {
            update = update.fit(match fit {
                FitOption::Refit => Fit::Refit,
                FitOption::Keep => Fit::Keep,
                FitOption::Extend => Fit::Extend,
            });
        }
        if let Some(views) = options.views {
            update = update.views(match views {
                ViewsOption::Keep => Views::Keep,
                ViewsOption::Drop => Views::Drop,
            });
        }
        if let Some(carry) = options.selections {
            update = update.selections(match carry {
                CarryOption::Keep => Carry::Keep,
                CarryOption::Reset => Carry::Reset,
            });
        }
        self.view.updates.send(update);
        Ok(())
    }

    /// The bytes of a change message of the chart's state: the clauses of each of its selections
    /// and the modes that are on, which `setState` applies, such as to the view of the same chart
    /// after a reload. A chart that declares no modes gives no modes. It reads the state after the
    /// view's last step, so a `setState` or a `setMode` shows in it once the view has applied it.
    pub fn state(&self) -> Result<Uint8Array, JsValue> {
        let state = self.view.state.borrow();
        let change = ChartChange {
            selections: (state.selections.iter())
                .map(|(name, clauses)| (name.clone(), clauses.clone()))
                .collect(),
            modes: state.modes.clone(),
            settled: true,
            ..ChartChange::default()
        };
        let bytes = (change.to_message())
            .and_then(|message| Ok(message.to_bytes()?))
            .map_err(|error| JsError::new(&error.to_string()))?;
        Ok(Uint8Array::from(&bytes[..]))
    }

    /// Applies a change message: the clauses of each of its selections, with the resolution that
    /// the chart gives each, and the modes that are on, when it holds the modes, which turns on
    /// each of them and turns off the chart's other toggles. A selection or a mode that the chart
    /// does not declare is left out, so a page can send one chart's changes to another. Throws for
    /// bytes that are not a change message of this format version.
    #[wasm_bindgen(js_name = setState)]
    pub fn set_state(&self, bytes: Vec<u8>) -> Result<(), JsValue> {
        let error = |error: String| JsValue::from(JsError::new(&error));
        let message = ChangeMessage::from_bytes(bytes).map_err(|e| error(e.to_string()))?;
        let change = ChartChange::from_message(&message).map_err(|e| error(e.to_string()))?;
        for (name, clauses) in change.selections {
            self.view.updates.set_clauses(&name, clauses);
        }
        if let Some(on) = change.modes {
            self.view.updates.set_modes_on(on);
        }
        Ok(())
    }

    /// Turns the chart's mode `name` on or off, as a mode widget does.
    #[wasm_bindgen(js_name = setMode)]
    pub fn set_mode(&self, name: String, on: bool) {
        self.view.updates.set_mode(name, on);
    }

    /// Styles the chart with the project's stylesheets `styles`, or with the base stylesheet
    /// alone for `null`, and keeps its views and selections. Returns the diagnostics of the
    /// stylesheets.
    #[wasm_bindgen(js_name = setStyles, unchecked_return_type = "StyleDiagnostic[]")]
    pub fn set_styles(
        &self,
        #[wasm_bindgen(unchecked_param_type = "ProjectStyles | null")] styles: JsValue,
    ) -> Result<JsValue, JsValue> {
        let (sheet, diagnostics) = match from_value::<Option<ProjectStyles>>(styles)? {
            Some(project) => {
                let (sheet, diagnostics) = load(&project);
                (Some(sheet), diagnostics)
            }
            None => (None, Vec::new()),
        };
        *self.view.sheet.borrow_mut() = sheet;
        self.view.restyle();
        Ok(to_value(&diagnostics)?)
    }

    /// Draws the chart for a reader who prefers the color scheme `scheme`, `"light"` or `"dark"`:
    /// its background and the styles of the scheme. Keeps its views and selections.
    #[wasm_bindgen(js_name = setColorScheme)]
    pub fn set_color_scheme(
        &self,
        #[wasm_bindgen(unchecked_param_type = "\"light\" | \"dark\"")] scheme: JsValue,
    ) -> Result<(), JsValue> {
        let scheme = ColorScheme::from(from_value::<Scheme>(scheme)?);
        if scheme != self.view.scheme.replace(scheme) {
            self.view.updates.set_preferences(scheme.preferences());
            self.view.restyle();
        }
        Ok(())
    }
}

/// A view that JavaScript frees releases its canvas, so the canvas calls no freed listener.
impl Drop for ChartView {
    fn drop(&mut self) {
        self.view.release();
    }
}

impl View {
    /// Removes the listeners and the resize observer, and drops the app and its GPU resources.
    fn release(&self) {
        if self.destroyed.replace(true) {
            return;
        }
        for (kind, callback) in self.listeners.take() {
            let _ = self
                .canvas
                .remove_event_listener_with_callback(kind, callback.as_ref().unchecked_ref());
        }
        if let Some((observer, _)) = self.observer.take() {
            observer.disconnect();
        }
        self.queue.borrow_mut().clear();
        // A driver that awaits the app holds the core, and drops it when it returns.
        drop(self.core.take());
        PAGE.with(|page| page.views.borrow_mut().remove(&self.id));
    }

    /// Styles the chart again with its stylesheet, its scheme, and its transparency.
    fn restyle(&self) {
        let sheet = self.sheet.borrow();
        let cascade = cascade(sheet.as_ref(), self.scheme.get(), self.transparent);
        self.updates.send(ChartUpdate::styles(cascade));
    }
}

/// The color that a view's canvas clears to: none for a transparent view, so the page shows
/// through, and the background of `scheme` otherwise, as the CLI's images of one scheme have.
fn canvas_background(scheme: ColorScheme, transparent: bool) -> [f32; 4] {
    match transparent {
        true => [0.0; 4],
        false => scheme.background_rgba(),
    }
}

/// The canvas's size in its own CSS pixels, which a CSS transform of the page does not change, or
/// the default size of a canvas before the page lays it out.
fn css_size(canvas: &HtmlCanvasElement) -> [f32; 2] {
    let (width, height) = (canvas.client_width() as f32, canvas.client_height() as f32);
    if width >= 1.0 && height >= 1.0 {
        [width, height]
    } else {
        [300.0, 150.0]
    }
}

fn pixel_ratio() -> f32 {
    web_sys::window().map_or(1.0, |window| window.device_pixel_ratio() as f32)
}

/// The dimensions of `size` CSS pixels at `scale`, at a lower scale when the device's largest
/// texture would not hold them.
fn fit(size: [f32; 2], scale: f32, max_texture: u32) -> CanvasDimensions {
    let longest = size[0].max(size[1]);
    let scale = if longest * scale > max_texture as f32 {
        (max_texture - 1) as f32 / longest
    } else {
        scale
    };
    CanvasDimensions { size, scale }
}

/// Lets the canvas take the focus and keeps one finger's drag for the page's scroll.
fn prepare_canvas(canvas: &HtmlCanvasElement) -> Result<(), JsValue> {
    if !canvas.has_attribute("tabindex") {
        canvas.set_attribute("tabindex", "0")?;
    }
    canvas.style().set_property("touch-action", "pan-x pan-y")?;
    Ok(())
}

/// Asks the view `id` to refresh its app, from the app's waker.
fn wake(id: u32) {
    let view = PAGE.with(|page| page.views.borrow().get(&id).and_then(Weak::upgrade));
    if let Some(view) = view {
        push(&view, Input::Refresh);
    }
}

/// Queues `input` and starts the driver when it is not running. A move replaces a move that
/// still waits, so the app does not fall behind the pointer.
fn push(view: &Rc<View>, input: Input) {
    if view.destroyed.get() {
        return;
    }
    {
        let mut queue = view.queue.borrow_mut();
        let replaces = matches!(
            (&input, queue.back()),
            (
                Input::Event(WindowEvent::CursorMoved(_)),
                Some(Input::Event(WindowEvent::CursorMoved(_)))
            ) | (Input::Refresh, Some(Input::Refresh))
                | (Input::Resize(..), Some(Input::Resize(..)))
        );
        if replaces {
            queue.pop_back();
        }
        queue.push_back(input);
    }
    if !view.driving.replace(true) {
        spawn_local(drive(view.clone()));
    }
}

/// Handles the view's inputs in order until none waits. The app's updates are asynchronous, so
/// the driver takes the core while it awaits one, and the inputs that arrive meanwhile wait.
async fn drive(view: Rc<View>) {
    loop {
        let input = view.queue.borrow_mut().pop_front();
        let Some(input) = input else {
            break;
        };
        let Some(mut core) = view.core.take() else {
            break;
        };
        let changes = match input {
            Input::Event(event) => Some(core.app.update(&event, Instant::now()).await),
            Input::Refresh => Some(core.app.refresh().await),
            Input::Resize(size, scale) => {
                core.resized = Some(fit(size, scale, core.max_texture));
                let resize = WindowEvent::WindowResize(WindowResizeEvent { size });
                let mut changes = core.app.update(&resize, Instant::now()).await;
                changes.redraw = true;
                Some(changes)
            }
            Input::Draw => {
                if let Some(dimensions) = core.resized.take() {
                    core.canvas.resize(dimensions);
                }
                (core.canvas)
                    .set_background(canvas_background(view.scheme.get(), view.transparent));
                if let Err(error) = core.canvas.render() {
                    log(&format!(
                        "avenger-chart-web: a frame failed to draw: {error}"
                    ));
                }
                None
            }
        };
        if let Some(changes) = changes {
            apply(&view, &mut core, changes);
            *view.state.borrow_mut() = State::of(core.app.state());
        }
        if view.destroyed.get() {
            break;
        }
        *view.core.borrow_mut() = Some(core);
        report(&view);
    }
    view.driving.set(false);
}

/// Passes the changes that the app reported to the page's `onChange`, after the step that made
/// them, when the view holds its app again, so the callback can call the view.
fn report(view: &Rc<View>) {
    let changes = match view.changes.lock() {
        Ok(mut changes) => std::mem::take(&mut *changes),
        Err(_) => return,
    };
    let Some(on_change) = &view.on_change else {
        return;
    };
    for change in changes {
        let bytes = match change
            .to_message()
            .and_then(|message| Ok(message.to_bytes()?))
        {
            Ok(bytes) => bytes,
            Err(error) => {
                log(&format!(
                    "avenger-chart-web: a change has no message: {error}"
                ));
                continue;
            }
        };
        let object = Object::new();
        let set = |key: &str, value: JsValue| {
            let _ = Reflect::set(&object, &JsValue::from_str(key), &value);
        };
        set("bytes", Uint8Array::from(&bytes[..]).into());
        let names: Array = (change.selections.iter())
            .map(|(name, _)| JsValue::from_str(name))
            .collect();
        set("selections", names.into());
        let modes = (change.modes.as_ref()).map_or(JsValue::NULL, |modes| {
            modes
                .iter()
                .map(|name| JsValue::from_str(name))
                .collect::<Array>()
                .into()
        });
        set("modes", modes);
        set("settled", JsValue::from_bool(change.settled));
        if let Err(error) = on_change.call1(&JsValue::NULL, &object) {
            web_sys::console::error_2(
                &JsValue::from_str("avenger-chart-web: onChange threw"),
                &error,
            );
        }
        if view.destroyed.get() {
            return;
        }
    }
}

/// Installs the changed scenes, shows the cursor, and requests a frame when the canvas changed.
fn apply(view: &Rc<View>, core: &mut Core, changes: Changes) {
    for &index in &changes.scenes {
        let scene = &core.app.scenes()[index];
        if let Err(error) = core.canvas.set_layer(index, scene, core.app.text_engine()) {
            log(&format!(
                "avenger-chart-web: a scene failed to install: {error}"
            ));
        }
    }
    if let Some(cursor) = changes.cursor {
        let _ = view.canvas.style().set_property("cursor", cursor.name());
    }
    if !changes.scenes.is_empty() || changes.redraw {
        request_frame(view);
    }
}

/// Draws in the browser's next frame, once however many changes ask for it.
fn request_frame(view: &Rc<View>) {
    if view.frame_requested.replace(true) {
        return;
    }
    let weak = Rc::downgrade(view);
    let callback = Closure::once_into_js(move || {
        if let Some(view) = weak.upgrade() {
            view.frame_requested.set(false);
            push(&view, Input::Draw);
        }
    });
    if let Some(window) = web_sys::window() {
        let _ = window.request_animation_frame(callback.unchecked_ref());
    }
}

fn log(message: &str) {
    web_sys::console::error_1(&JsValue::from_str(message));
}

/// The position of a pointer, wheel, or mouse event in the CSS pixels of `canvas`'s content, from
/// the event's client coordinates, so that dispatched events place as the browser's do.
fn position(canvas: &HtmlCanvasElement, event: &MouseEvent) -> [f32; 2] {
    let rect = canvas.get_bounding_client_rect();
    input::canvas_position(
        [f64::from(event.client_x()), f64::from(event.client_y())],
        [rect.left(), rect.top()],
        [rect.width(), rect.height()],
        [
            f64::from(canvas.offset_width()),
            f64::from(canvas.offset_height()),
        ],
        [
            f64::from(canvas.client_left()),
            f64::from(canvas.client_top()),
        ],
    )
}

fn focused(canvas: &HtmlCanvasElement) -> bool {
    let active = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element());
    active.is_some_and(|active| active == ***canvas)
}

/// Adds the view's DOM listeners to its canvas.
fn listen(view: &Rc<View>) -> Result<(), JsValue> {
    let on = |kind: &'static str, passive: bool, handle: fn(&Rc<View>, Event)| {
        let weak = Rc::downgrade(view);
        let callback = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            if let Some(view) = weak.upgrade() {
                handle(&view, event);
            }
        });
        let options = AddEventListenerOptions::new();
        options.set_passive(passive);
        view.canvas
            .add_event_listener_with_callback_and_add_event_listener_options(
                kind,
                callback.as_ref().unchecked_ref(),
                &options,
            )?;
        view.listeners.borrow_mut().push((kind, callback));
        Ok::<(), JsValue>(())
    };
    on("pointerdown", false, pointer_down)?;
    on("pointermove", true, pointer_move)?;
    on("pointerup", true, pointer_up)?;
    on("pointercancel", true, pointer_cancel)?;
    on("pointerenter", true, |view, event| {
        if !is_touch(&event) {
            push(view, Input::Event(WindowEvent::CursorEntered));
        }
    })?;
    on("pointerleave", true, |view, event| {
        if !is_touch(&event) {
            push(view, Input::Event(WindowEvent::CursorLeft));
        }
    })?;
    on("touchmove", false, |view, event| {
        // Two fingers drag the chart, which the page must not scroll for.
        let touches = event
            .dyn_ref::<TouchEvent>()
            .map_or(0, |event| event.touches().length());
        if touches >= 2 || view.touches.borrow().pinching() {
            event.prevent_default();
        }
    })?;
    on("wheel", false, |view, event| {
        let Some(wheel) = event.dyn_ref::<WheelEvent>() else {
            return;
        };
        if !view.wheel.reaches(focused(&view.canvas)) {
            return;
        }
        event.prevent_default();
        push(
            view,
            Input::Event(input::moved(position(&view.canvas, wheel))),
        );
        let delta = input::wheel(wheel.delta_x(), wheel.delta_y(), wheel.delta_mode());
        push(view, Input::Event(delta));
    })?;
    on("keydown", true, |view, event| {
        key(view, event, ElementState::Pressed)
    })?;
    on("keyup", true, |view, event| {
        key(view, event, ElementState::Released)
    })?;
    on("focus", true, |view, _| {
        push(view, Input::Event(WindowEvent::WindowFocused(true)))
    })?;
    on("blur", true, |view, _| {
        push(view, Input::Event(WindowEvent::WindowFocused(false)))
    })?;
    on("paste", true, |view, event| {
        let text = event
            .dyn_ref::<ClipboardEvent>()
            .and_then(ClipboardEvent::clipboard_data)
            .and_then(|data| data.get_data("text").ok());
        if let Some(text) = text {
            push(view, Input::Event(WindowEvent::Paste(text)));
        }
    })?;
    // The browser shows no menu over a chart, whose right button the chart's tools may use.
    on("contextmenu", false, |_, event| event.prevent_default())?;
    Ok(())
}

fn is_touch(event: &Event) -> bool {
    event
        .dyn_ref::<PointerEvent>()
        .is_some_and(|event| event.pointer_type() == "touch")
}

/// Handles a pointer event at its position in the canvas: a finger's through the view's touches,
/// with `touch`, and a mouse's or a pen's as the window events of `mouse`.
fn pointer(
    view: &Rc<View>,
    event: &Event,
    touch: impl FnOnce(&mut Touches, i32, [f32; 2]) -> Vec<WindowEvent>,
    mouse: impl FnOnce(&PointerEvent, [f32; 2]) -> Vec<WindowEvent>,
) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let at = position(&view.canvas, pointer);
    let events = match pointer.pointer_type() == "touch" {
        true => touch(&mut view.touches.borrow_mut(), pointer.pointer_id(), at),
        false => mouse(pointer, at),
    };
    for event in events {
        push(view, Input::Event(event));
    }
}

fn pointer_down(view: &Rc<View>, event: Event) {
    // A press focuses the canvas, so the wheel and the keys reach the chart after it.
    let options = FocusOptions::new();
    options.set_prevent_scroll(true);
    let _ = view.canvas.focus_with_options(&options);
    pointer(view, &event, Touches::down, |pointer, at| {
        // The canvas keeps the pointer during a drag that leaves it, and the page selects no text.
        event.prevent_default();
        let _ = view.canvas.set_pointer_capture(pointer.pointer_id());
        let button = input::button(pointer.button());
        vec![
            input::moved(at),
            input::press(button, ElementState::Pressed),
        ]
    });
}

fn pointer_move(view: &Rc<View>, event: Event) {
    pointer(view, &event, Touches::moved, |_, at| vec![input::moved(at)]);
}

fn pointer_up(view: &Rc<View>, event: Event) {
    pointer(view, &event, Touches::up, |pointer, at| {
        let button = input::button(pointer.button());
        vec![
            input::moved(at),
            input::press(button, ElementState::Released),
        ]
    });
}

fn pointer_cancel(view: &Rc<View>, event: Event) {
    pointer(
        view,
        &event,
        |touches, id, _| touches.cancel(id),
        // A lost pointer cancels its press, as a lost focus does.
        |_, _| vec![WindowEvent::WindowFocused(false)],
    );
}

fn key(view: &Rc<View>, event: Event, state: ElementState) {
    let Some(keyboard) = event.dyn_ref::<KeyboardEvent>() else {
        return;
    };
    let name = keyboard.key();
    if let Some(event) = input::key(&name, state) {
        push(view, Input::Event(event));
    }
    // Escape gives the focus back to the page after the chart has the key.
    if name == "Escape" && state == ElementState::Pressed {
        let _ = view.canvas.blur();
    }
}

/// Follows the size of the canvas.
fn observe(view: &Rc<View>) -> Result<(), JsValue> {
    let weak = Rc::downgrade(view);
    let callback = Closure::<dyn FnMut(Array)>::new(move |entries: Array| {
        let Some(view) = weak.upgrade() else {
            return;
        };
        let Some(entry) = entries.get(0).dyn_into::<ResizeObserverEntry>().ok() else {
            return;
        };
        let rect = entry.content_rect();
        let size = [rect.width() as f32, rect.height() as f32];
        if size.iter().all(|&length| length >= 1.0) {
            push(&view, Input::Resize(size, pixel_ratio()));
        }
    });
    let observer = ResizeObserver::new(callback.as_ref().unchecked_ref())?;
    observer.observe(&view.canvas);
    *view.observer.borrow_mut() = Some((observer, callback));
    Ok(())
}
