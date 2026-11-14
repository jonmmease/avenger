//! The JavaScript interface: `loadFonts`, and `ChartView`, which runs the app of a chart in a
//! canvas, takes the canvas's DOM events, and draws through the page's one WebGPU device.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    rc::{Rc, Weak},
    sync::Arc,
};

use avenger_app::app::{AvengerApp, Changes};
use avenger_chart::Chart;
use avenger_chart_app::{ChartApp, ChartAppState, ChartUpdate, ChartUpdates};
use avenger_common::{canvas::CanvasDimensions, time::Instant};
use avenger_eventstream::window::{ElementState, WindowEvent, WindowResizeEvent};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::{EngineOptions, LabelEngine, RegisteredFont};
use avenger_wgpu::{
    canvas::CanvasConfig,
    html_canvas::{BrowserGpu, HtmlCanvasCanvas},
};
use js_sys::{Array, Object, Promise, Reflect, Uint8Array};
use wasm_bindgen::{closure::Closure, prelude::*, JsCast};
use wasm_bindgen_futures::{future_to_promise, spawn_local, JsFuture};
use web_sys::{
    AddEventListenerOptions, ClipboardEvent, Event, FocusOptions, HtmlCanvasElement, KeyboardEvent,
    PointerEvent, ResizeObserver, ResizeObserverEntry, Response, TouchEvent, WheelEvent,
};

use crate::{
    fonts::{font_options, read_fonts},
    input::{self, Touches, Wheel},
    styles::{cascade, PageDiagnostic},
};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** The options of `ChartView.create`. */
export interface ViewOptions {
  /** CSS text that styles the chart after the base stylesheet. */
  stylesheet?: string;
  /** When wheel events reach the chart: while its canvas has the focus, or always. */
  wheel?: "focus" | "always";
}

/** A diagnostic of a page stylesheet, at a line and a column of its text, both from 1. */
export interface StyleDiagnostic {
  severity: "error" | "warning";
  message: string;
  line: number;
  column: number;
}
"#;

#[wasm_bindgen(start)]
fn start() {
    console_error_panic_hook::set_once();
}

/// What the page shares among its views: the fonts that it loaded, the loads that it started,
/// the label engine of the loaded fonts, and the WebGPU device.
#[derive(Default)]
struct Page {
    fonts: RefCell<Vec<RegisteredFont>>,
    loads: RefCell<Vec<Promise>>,
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

/// Fetches the font file at `url`, one TrueType, OpenType, or collection font, and registers its
/// fonts after the bundled fonts with the views made after the load ends.
#[wasm_bindgen(js_name = loadFonts, unchecked_return_type = "Promise<void>")]
pub fn load_fonts(url: String) -> Promise {
    let load = future_to_promise(async move {
        let window = web_sys::window().ok_or_else(|| JsError::new("no window"))?;
        let response: Response = JsFuture::from(window.fetch_with_str(&url))
            .await?
            .dyn_into()?;
        if !response.ok() {
            let status = response.status();
            return Err(
                JsError::new(&format!("cannot load fonts from {url}: status {status}")).into(),
            );
        }
        let buffer = JsFuture::from(response.array_buffer()?).await?;
        let bytes = Uint8Array::new(&buffer).to_vec();
        let fonts = read_fonts(&bytes)
            .map_err(|error| JsError::new(&format!("cannot load fonts from {url}: {error}")))?;
        PAGE.with(|page| page.fonts.borrow_mut().extend(fonts));
        Ok(JsValue::UNDEFINED)
    });
    PAGE.with(|page| page.loads.borrow_mut().push(load.clone()));
    load
}

/// The engine of the bundled fonts and the page's fonts, after the loads that the page started
/// end.
async fn engine() -> Result<LabelEngine, JsValue> {
    let loads = PAGE.with(|page| page.loads.borrow().clone());
    for load in loads {
        JsFuture::from(load).await?;
    }
    Ok(PAGE.with(|page| {
        let fonts = page.fonts.borrow();
        let mut engine = page.engine.borrow_mut();
        match &*engine {
            Some((count, engine)) if *count == fonts.len() => engine.clone(),
            _ => {
                let built = LabelEngine::new(EngineOptions {
                    fonts: font_options(fonts.clone()),
                })
                .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
                .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()));
                *engine = Some((fonts.len(), built.clone()));
                built
            }
        }
    }))
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

/// The resize observer of a view's canvas, with the callback that it calls.
type Observer = (ResizeObserver, Closure<dyn FnMut(Array)>);

struct Listener {
    kind: &'static str,
    callback: Closure<dyn FnMut(Event)>,
}

/// A chart in a canvas, with the tools that the chart declares.
#[wasm_bindgen]
pub struct ChartView {
    view: Rc<View>,
    diagnostics: Array,
}

#[wasm_bindgen]
impl ChartView {
    /// Decodes the chart message `bytes`, builds its app at the canvas's CSS size and the
    /// display's pixel ratio, and draws it into `canvas`. `options` is an object with an optional
    /// `stylesheet`, CSS text that styles the chart after the base stylesheet, and an optional
    /// `wheel`, `"focus"` by default or `"always"`. Rejects with the decoder's error for an
    /// invalid message, and when the browser does not support WebGPU. It waits for the fonts that
    /// the page started to load.
    pub async fn create(
        canvas: HtmlCanvasElement,
        bytes: Vec<u8>,
        #[wasm_bindgen(unchecked_optional_param_type = "ViewOptions")] options: Option<Object>,
    ) -> Result<ChartView, JsValue> {
        let chart = Chart::from_bytes(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        let stylesheet = string_option(options.as_ref(), "stylesheet")?;
        let wheel = match string_option(options.as_ref(), "wheel")? {
            Some(name) => Wheel::parse(&name).map_err(|error| JsError::new(&error))?,
            None => Wheel::default(),
        };
        let engine = engine().await?;
        let gpu = gpu().await?;
        let (styles, diagnostics) = cascade(stylesheet.as_deref());

        let size = css_size(&canvas);
        let dimensions = fit(size, pixel_ratio(), gpu.max_texture_dimension());
        let updates = ChartUpdates::new();
        let app = ChartApp::new(chart, styles, engine)
            .size(size)
            .updates(&updates)
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
            wheel,
            updates,
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
            diagnostics: diagnostics_array(&diagnostics),
        })
    }

    /// The diagnostics of the `stylesheet` option: objects with `severity`, `message`, `line`,
    /// and `column`.
    #[wasm_bindgen(getter, unchecked_return_type = "StyleDiagnostic[]")]
    pub fn diagnostics(&self) -> Array {
        self.diagnostics.clone()
    }

    /// Replaces the chart with the chart message `bytes`, and keeps the views of its scales and
    /// its selections where the new chart has the same structure. Throws the decoder's error for
    /// an invalid message.
    #[wasm_bindgen(js_name = setChart)]
    pub fn set_chart(&self, bytes: Vec<u8>) -> Result<(), JsError> {
        let chart = Chart::from_bytes(bytes).map_err(|error| JsError::new(&error.to_string()))?;
        self.view.updates.send(ChartUpdate::new(chart));
        Ok(())
    }

    /// Styles the chart with `stylesheet`, CSS text after the base stylesheet, or with the base
    /// stylesheet alone for `null`, and keeps its views and selections. Returns the diagnostics
    /// of the stylesheet.
    #[wasm_bindgen(js_name = setStylesheet, unchecked_return_type = "StyleDiagnostic[]")]
    pub fn set_stylesheet(&self, stylesheet: Option<String>) -> Array {
        let (styles, diagnostics) = cascade(stylesheet.as_deref());
        self.view.updates.send(ChartUpdate::styles(styles));
        diagnostics_array(&diagnostics)
    }

    /// Removes the view's listeners and its resize observer, and frees its GPU resources.
    pub fn destroy(&self) {
        self.view.release();
    }
}

/// A view that JavaScript frees without `destroy` releases its canvas too, so the canvas calls no
/// freed listener.
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
        for listener in self.listeners.take() {
            let _ = self.canvas.remove_event_listener_with_callback(
                listener.kind,
                listener.callback.as_ref().unchecked_ref(),
            );
        }
        if let Some((observer, _)) = self.observer.take() {
            observer.disconnect();
        }
        self.queue.borrow_mut().clear();
        // A driver that awaits the app holds the core, and drops it when it returns.
        drop(self.core.take());
        PAGE.with(|page| page.views.borrow_mut().remove(&self.id));
    }
}

/// The string at `key` of `options`, when it has one.
fn string_option(options: Option<&Object>, key: &str) -> Result<Option<String>, JsValue> {
    let Some(options) = options else {
        return Ok(None);
    };
    let value = Reflect::get(options, &JsValue::from_str(key))?;
    if value.is_undefined() || value.is_null() {
        return Ok(None);
    }
    value
        .as_string()
        .map(Some)
        .ok_or_else(|| JsError::new(&format!("the {key} option is a string")).into())
}

fn diagnostics_array(diagnostics: &[PageDiagnostic]) -> Array {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let object = Object::new();
            let set = |key: &str, value: JsValue| {
                let _ = Reflect::set(&object, &JsValue::from_str(key), &value);
            };
            set("severity", JsValue::from_str(diagnostic.severity));
            set("message", JsValue::from_str(&diagnostic.message));
            set("line", JsValue::from(diagnostic.line));
            set("column", JsValue::from(diagnostic.column));
            JsValue::from(object)
        })
        .collect()
}

/// The canvas's size in CSS pixels, or the default size of a canvas before the page lays it out.
fn css_size(canvas: &HtmlCanvasElement) -> [f32; 2] {
    let rect = canvas.get_bounding_client_rect();
    let (width, height) = (rect.width() as f32, rect.height() as f32);
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
        }
        if view.destroyed.get() {
            break;
        }
        *view.core.borrow_mut() = Some(core);
    }
    view.driving.set(false);
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
fn position(canvas: &HtmlCanvasElement, event: &Event) -> [f32; 2] {
    let client = |key: &str| {
        Reflect::get(event, &JsValue::from_str(key))
            .ok()
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0)
    };
    let rect = canvas.get_bounding_client_rect();
    [
        (client("clientX") - rect.left() - f64::from(canvas.client_left())) as f32,
        (client("clientY") - rect.top() - f64::from(canvas.client_top())) as f32,
    ]
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
        view.listeners
            .borrow_mut()
            .push(Listener { kind, callback });
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
            Input::Event(input::moved(position(&view.canvas, &event))),
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

fn pointer_down(view: &Rc<View>, event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let at = position(&view.canvas, &event);
    // A press focuses the canvas, so the wheel and the keys reach the chart after it.
    let options = FocusOptions::new();
    options.set_prevent_scroll(true);
    let _ = view.canvas.focus_with_options(&options);
    if pointer.pointer_type() == "touch" {
        for event in view.touches.borrow_mut().down(pointer.pointer_id(), at) {
            push(view, Input::Event(event));
        }
        return;
    }
    // The canvas keeps the pointer during a drag that leaves it, and the page selects no text.
    event.prevent_default();
    let _ = view.canvas.set_pointer_capture(pointer.pointer_id());
    push(view, Input::Event(input::moved(at)));
    let button = input::button(pointer.button());
    push(
        view,
        Input::Event(input::press(button, ElementState::Pressed)),
    );
}

fn pointer_move(view: &Rc<View>, event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let at = position(&view.canvas, &event);
    if pointer.pointer_type() == "touch" {
        for event in view.touches.borrow_mut().moved(pointer.pointer_id(), at) {
            push(view, Input::Event(event));
        }
        return;
    }
    push(view, Input::Event(input::moved(at)));
}

fn pointer_up(view: &Rc<View>, event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    let at = position(&view.canvas, &event);
    if pointer.pointer_type() == "touch" {
        for event in view.touches.borrow_mut().up(pointer.pointer_id(), at) {
            push(view, Input::Event(event));
        }
        return;
    }
    push(view, Input::Event(input::moved(at)));
    let button = input::button(pointer.button());
    push(
        view,
        Input::Event(input::press(button, ElementState::Released)),
    );
}

fn pointer_cancel(view: &Rc<View>, event: Event) {
    let Some(pointer) = event.dyn_ref::<PointerEvent>() else {
        return;
    };
    if pointer.pointer_type() == "touch" {
        for event in view.touches.borrow_mut().cancel(pointer.pointer_id()) {
            push(view, Input::Event(event));
        }
        return;
    }
    // A lost pointer cancels its press, as a lost focus does.
    push(view, Input::Event(WindowEvent::WindowFocused(false)));
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
