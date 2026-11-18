//! The live chart: a chart app that egui hosts in a rectangle, in the result pane or in a table
//! cell. egui's input over the rectangle reaches the app as avenger window events, and the app's
//! scenes draw into a texture that egui shows in the rectangle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::exec::Exec;
use avenger_app::app::{AvengerApp, Changes};
use avenger_chart::Chart;
use avenger_chart_app::{ChartApp, ChartAppState, ChartUpdate, ChartUpdates};
use avenger_common::cursor::CursorIcon;
use avenger_common::time::Instant;
use avenger_eventstream::window::{Key, MouseButton, WindowEvent, WindowResizeEvent};
use avenger_style::{Cascade, Preferences};
use avenger_typst_label::LabelEngine;
use eframe::{egui, egui_wgpu::RenderState};

use crate::gpu::{self, Painter, Texture};
use crate::input;

/// A chart app in a rectangle of egui.
pub struct LiveChart {
    app: AvengerApp<ChartAppState>,
    updates: ChartUpdates,
    texture: Texture,
    painter: Painter,
    /// Whether the app's scenes changed since the texture drew them.
    dirty: bool,
    /// Whether the app's waker asked for a refresh.
    refresh: Arc<AtomicBool>,
    id: egui::Id,
    /// Whether the pointer is over the chart, as the app last heard.
    hovered: bool,
    /// The buttons pressed over the chart and not yet released, which hold the pointer's events
    /// for the chart until their release.
    pressed: Vec<MouseButton>,
    modifiers: egui::Modifiers,
    cursor: CursorIcon,
}

impl LiveChart {
    /// The app of `chart` at `size`, in logical pixels, whose scenes `state` draws. The app
    /// builds at `size` rounded to whole physical pixels, the size that [`LiveChart::show`] gives
    /// it, so that its first scenes draw at their final size.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        preferences: Preferences,
        engine: LabelEngine,
        size: [f32; 2],
        id: egui::Id,
        runtime: &Exec,
        ctx: &egui::Context,
        state: &RenderState,
    ) -> Result<Self, String> {
        let dimensions = gpu::dimensions(egui::vec2(size[0], size[1]), ctx.pixels_per_point());
        let updates = ChartUpdates::new();
        // The first build draws at the reader's preferences.
        updates.set_preferences(preferences);
        let app = ChartApp::new(chart, cascade, engine)
            .size(dimensions.size)
            .updates(&updates)
            .build();
        let app = runtime.block_on(app).map_err(|error| error.to_string())?;
        #[cfg(not(target_arch = "wasm32"))]
        let app = app
            .with_runtime(runtime.handle())
            .map_err(|error| error.to_string())?;
        let refresh = Arc::new(AtomicBool::new(false));
        {
            let (refresh, ctx) = (refresh.clone(), ctx.clone());
            app.waker().set_notify(move || {
                refresh.store(true, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }
        Ok(Self {
            texture: Texture::new(state, dimensions),
            painter: Painter::new(state, app.images()),
            cursor: app.cursor(),
            app,
            updates,
            dirty: true,
            refresh,
            id,
            hovered: false,
            pressed: Vec::new(),
            modifiers: egui::Modifiers::NONE,
        })
    }

    /// Replaces the chart with `chart`, keeping the views of pans and zooms and the clauses of
    /// the selections, as a chart update does by default.
    pub fn set_chart(&self, chart: Chart) {
        self.updates.send(ChartUpdate::new(chart));
    }

    /// Restyles the chart for a new theme: the cascade of its background and the reader's
    /// preferences, keeping the chart, its views, and its selections.
    pub fn restyle(&self, cascade: Arc<Cascade>, preferences: Preferences) {
        self.updates.send(ChartUpdate::styles(cascade));
        self.updates.set_preferences(preferences);
    }

    /// Whether the chart waits for work: a refresh, a redraw, or an update of its chart.
    pub fn busy(&self) -> bool {
        self.dirty || self.refresh.load(Ordering::SeqCst) || self.updates.pending()
    }

    /// Releases the chart's texture.
    pub fn free(self, state: &RenderState) {
        self.texture.free(state);
    }

    /// Shows the chart in `rect`, sends it the input of this frame, and draws its scenes when
    /// they change. Wheel input over the chart goes to the chart only.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        state: &RenderState,
        runtime: &Exec,
    ) {
        let response = ui.interact(rect, self.id, egui::Sense::click_and_drag());
        if response.clicked() || response.drag_started() {
            response.request_focus();
        }
        let focused = response.has_focus();
        // Escape is the chart's while it has the focus: without the filter, egui gives up the
        // focus on Escape before the chart hears it, and the pane around the chart closes.
        if focused {
            let filter = egui::EventFilter {
                escape: true,
                ..Default::default()
            };
            ui.memory_mut(|memory| memory.set_focus_lock_filter(self.id, filter));
        }
        // The chart hears keys while it has the focus, or while the pointer is over it and no
        // other widget has the focus.
        let keyboard = focused || (self.hovered && ui.memory(|memory| memory.focused().is_none()));
        self.resize(ui, rect, state, runtime);
        // A press or a wheel over a widget on top of the chart, such as a cell's expand button
        // or row edge, is the widget's, not the chart's.
        let on_top = response.hovered();
        let events = self.events(ui, rect, keyboard, on_top);
        for event in events {
            self.update(&event, runtime);
        }
        if self.refresh.swap(false, Ordering::SeqCst) {
            let changes = runtime.block_on(self.app.refresh());
            self.apply(&changes);
        }
        if self.dirty {
            let app = &self.app;
            let scenes = app.scenes().iter().map(|scene| scene.as_ref());
            self.painter
                .draw(state, &self.texture, scenes, app.text_engine());
            self.dirty = false;
        }
        ui.painter().image(
            self.texture.id,
            rect,
            self.texture.uv(),
            egui::Color32::WHITE,
        );
        if self.hovered || !self.pressed.is_empty() {
            ui.ctx().set_cursor_icon(input::cursor(self.cursor));
        }
        if focused {
            let stroke = ui.visuals().selection.stroke;
            ui.painter()
                .rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
        }
    }

    /// Resizes the app to `rect` when its size changes, so that the frame draws the scenes that
    /// the app builds at the new size.
    fn resize(&mut self, ui: &egui::Ui, rect: egui::Rect, state: &RenderState, runtime: &Exec) {
        let scale = ui.ctx().pixels_per_point();
        let dimensions = gpu::dimensions(rect.size(), scale);
        let size = dimensions.size;
        if size == self.app.size() && scale == self.texture.requested.scale {
            return;
        }
        self.texture.resize(state, dimensions);
        // The app builds its scenes at the new size in the update, so this frame draws them.
        self.update(
            &WindowEvent::WindowResize(WindowResizeEvent { size }),
            runtime,
        );
        // The texture's size changed, so it draws again even when no scene changed.
        self.dirty = true;
    }

    /// The avenger events of this frame's egui input.
    fn events(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        keyboard: bool,
        on_top: bool,
    ) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        let (raw, modifiers, window_focused) =
            ui.input(|input| (input.events.clone(), input.modifiers, input.focused));
        if !window_focused && !self.pressed.is_empty() {
            self.pressed.clear();
            events.push(WindowEvent::WindowFocused(false));
        }
        if keyboard || self.hovered {
            for event in input::modifier_events(self.modifiers, modifiers) {
                events.push(event);
            }
            self.modifiers = modifiers;
        }
        let local = |position: egui::Pos2| position - rect.min;
        let mut wheel = false;
        for event in raw {
            match event {
                egui::Event::PointerMoved(position) => {
                    let inside = rect.contains(position);
                    if inside && !self.hovered {
                        self.hovered = true;
                        events.push(WindowEvent::CursorEntered);
                    }
                    if inside || !self.pressed.is_empty() {
                        events.push(input::cursor_moved(local(position)));
                    }
                    if !inside && self.hovered && self.pressed.is_empty() {
                        self.hovered = false;
                        events.push(WindowEvent::CursorLeft);
                    }
                }
                egui::Event::PointerGone => {
                    if self.hovered && self.pressed.is_empty() {
                        self.hovered = false;
                        events.push(WindowEvent::CursorLeft);
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let button = input::button(button);
                    if pressed && on_top && rect.contains(pos) {
                        self.pressed.push(button);
                        events.push(input::cursor_moved(local(pos)));
                        events.push(input::mouse_input(button, true));
                    } else if !pressed && self.pressed.contains(&button) {
                        self.pressed.retain(|held| *held != button);
                        events.push(input::cursor_moved(local(pos)));
                        events.push(input::mouse_input(button, false));
                        if !rect.contains(pos) && self.hovered && self.pressed.is_empty() {
                            self.hovered = false;
                            events.push(WindowEvent::CursorLeft);
                        }
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } if self.hovered && on_top => {
                    wheel = true;
                    events.push(input::wheel(unit, delta));
                }
                egui::Event::Key { key, pressed, .. } if keyboard => {
                    if let Some(named) = input::named_key(key) {
                        events.push(input::key_event(Key::Named(named), pressed, None));
                    }
                }
                egui::Event::Text(text) if keyboard => {
                    for c in text.chars() {
                        let text = Some(c.to_string());
                        events.push(input::key_event(Key::Character(c), true, text));
                        events.push(input::key_event(Key::Character(c), false, None));
                    }
                }
                egui::Event::Paste(text) if keyboard => {
                    events.push(WindowEvent::Paste(text));
                }
                egui::Event::Copy | egui::Event::Cut if keyboard => {
                    let text = self.app.copy_text();
                    if !text.is_empty() {
                        ui.ctx().copy_text(text.to_string());
                    }
                }
                _ => {}
            }
        }
        if wheel {
            // The table under a live cell does not scroll with the chart's wheel.
            ui.input_mut(|input| {
                input.smooth_scroll_delta = egui::Vec2::ZERO;
                input.raw_scroll_delta = egui::Vec2::ZERO;
            });
        }
        events
    }

    fn update(&mut self, event: &WindowEvent, runtime: &Exec) -> Changes {
        let changes = runtime.block_on(self.app.update(event, Instant::now()));
        self.apply(&changes);
        changes
    }

    fn apply(&mut self, changes: &Changes) {
        if !changes.scenes.is_empty() || changes.redraw {
            self.dirty = true;
        }
        if let Some(cursor) = changes.cursor {
            self.cursor = cursor;
        }
    }
}
