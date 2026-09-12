use avenger_eventstream::{
    runtime::{InputSession, LogicalRect},
    window::{ElementState, ImeEvent, Key, NamedKey, WindowEvent, WindowKeyboardInput},
};

use crate::ClipboardPayloadProvider;
#[derive(Debug, Default)]
struct TextAgentInputState {
    suppress_input_once: Option<String>,
    session: Option<InputSession>,
    composition_session: Option<Option<InputSession>>,
}

impl TextAgentInputState {
    fn composition_update(&mut self, text: impl Into<String>) -> WindowEvent {
        self.suppress_input_once = None;
        let session = self
            .composition_session
            .get_or_insert_with(|| self.session.clone())
            .clone();
        WindowEvent::Ime(ImeEvent::Preedit {
            text: text.into().into(),
            cursor: None,
        })
        .with_input_session(session)
    }

    fn composition_end(&mut self, text: impl Into<String>) -> WindowEvent {
        let text = text.into();
        self.suppress_input_once = (!text.is_empty()).then(|| text.clone());
        WindowEvent::Ime(ImeEvent::Commit(text.into())).with_input_session(
            self.composition_session
                .take()
                .unwrap_or_else(|| self.session.clone()),
        )
    }

    fn input(&mut self, text: impl Into<String>) -> Option<WindowEvent> {
        let text = text.into();
        if self.suppress_input_once.take().as_ref() == Some(&text) {
            return None;
        }
        let key = text.chars().next().map(Key::Character)?;
        Some(
            WindowEvent::KeyboardInput(WindowKeyboardInput {
                repeat: false,
                key,
                text: Some(text.into()),
                state: ElementState::Pressed,
            })
            .with_input_session(self.session.clone()),
        )
    }

    fn begin_key_input(&mut self) {
        self.suppress_input_once = None;
    }
}

fn browser_key(key: &str) -> Option<Key> {
    let named = match key {
        "ArrowDown" => Some(NamedKey::ArrowDown),
        "ArrowLeft" => Some(NamedKey::ArrowLeft),
        "ArrowRight" => Some(NamedKey::ArrowRight),
        "ArrowUp" => Some(NamedKey::ArrowUp),
        "End" => Some(NamedKey::End),
        "Home" => Some(NamedKey::Home),
        "PageDown" => Some(NamedKey::PageDown),
        "PageUp" => Some(NamedKey::PageUp),
        "Backspace" => Some(NamedKey::Backspace),
        "Delete" => Some(NamedKey::Delete),
        "Enter" => Some(NamedKey::Enter),
        "Escape" => Some(NamedKey::Escape),
        "Tab" => Some(NamedKey::Tab),
        " " | "Spacebar" => Some(NamedKey::Space),
        "Alt" => Some(NamedKey::Alt),
        "CapsLock" => Some(NamedKey::CapsLock),
        "Control" => Some(NamedKey::Control),
        "Shift" => Some(NamedKey::Shift),
        "Meta" | "OS" => Some(NamedKey::Super),
        "F1" => Some(NamedKey::F1),
        "F2" => Some(NamedKey::F2),
        "F3" => Some(NamedKey::F3),
        "F4" => Some(NamedKey::F4),
        "F5" => Some(NamedKey::F5),
        "F6" => Some(NamedKey::F6),
        "F7" => Some(NamedKey::F7),
        "F8" => Some(NamedKey::F8),
        "F9" => Some(NamedKey::F9),
        "F10" => Some(NamedKey::F10),
        "F11" => Some(NamedKey::F11),
        "F12" => Some(NamedKey::F12),
        _ => None,
    };
    named.map(Key::Named).or_else(|| {
        let mut characters = key.chars();
        let character = characters.next()?;
        characters
            .next()
            .is_none()
            .then_some(Key::Character(character))
    })
}

fn browser_key_event(key: &str, state: ElementState) -> Option<WindowEvent> {
    Some(WindowEvent::KeyboardInput(WindowKeyboardInput {
        repeat: false,
        key: browser_key(key)?,
        text: None,
        state,
    }))
}

fn is_browser_clipboard_shortcut(key: &str, control: bool, meta: bool) -> bool {
    (control || meta) && matches!(key.to_ascii_lowercase().as_str(), "c" | "x" | "v")
}

fn forwarded_browser_key_event(
    key: &str,
    state: ElementState,
    control: bool,
    meta: bool,
) -> Option<WindowEvent> {
    if is_browser_clipboard_shortcut(key, control, meta) {
        None
    } else {
        browser_key_event(key, state)
    }
}

fn prevent_browser_key_default(key: &str, control: bool, meta: bool) -> bool {
    // Printable keys must reach the hidden input so the browser emits text input.
    matches!(
        key,
        "ArrowLeft"
            | "ArrowRight"
            | "ArrowUp"
            | "ArrowDown"
            | "Home"
            | "End"
            | "Backspace"
            | "Delete"
            | "Enter"
            | "Escape"
            | "Tab"
    ) || ((control || meta) && key.chars().count() == 1)
}

fn browser_clipboard_event(name: &str, paste_text: Option<String>) -> Option<WindowEvent> {
    use avenger_eventstream::window::ClipboardEvent;

    let event = match name {
        "copy" => ClipboardEvent::Copy,
        "cut" => ClipboardEvent::Cut,
        "paste" => ClipboardEvent::Paste(paste_text?.into()),
        _ => return None,
    };
    Some(WindowEvent::Clipboard(event))
}

fn resolved_clipboard_payload(
    provider: Option<&ClipboardPayloadProvider>,
    fallback: &str,
) -> String {
    provider
        .and_then(|provider| provider())
        .unwrap_or_else(|| fallback.to_string())
}

fn logical_to_client_rect(
    rect: LogicalRect,
    logical_size: [f32; 2],
    canvas_rect: [f64; 4],
) -> Option<[f64; 4]> {
    if logical_size[0] <= 0.0 || logical_size[1] <= 0.0 {
        return None;
    }
    let [left, top, width, height] = canvas_rect;
    let scale_x = width / f64::from(logical_size[0]);
    let scale_y = height / f64::from(logical_size[1]);
    Some([
        left + f64::from(rect.x()) * scale_x,
        top + f64::from(rect.y()) * scale_y,
        f64::from(rect.width()) * scale_x,
        f64::from(rect.height()) * scale_y,
    ])
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        rc::Rc,
    };

    use avenger_common::time::Instant;
    use avenger_eventstream::runtime::{
        KeyboardPolicy, RuntimeHostCommand, RuntimeWakeEvent, RuntimeWakeKey,
    };
    use wasm_bindgen::{closure::Closure, JsCast, JsValue};
    use web_sys::{
        ClipboardEvent as DomClipboardEvent, CompositionEvent, Event, EventTarget, FocusEvent,
        HtmlCanvasElement, HtmlInputElement, InputEvent, KeyboardEvent, PointerEvent,
    };
    use winit::event_loop::EventLoopProxy;

    use crate::{WinitWgpuEvent, WinitWgpuEvent::App};

    use super::*;

    struct Listener {
        target: EventTarget,
        name: &'static str,
        callback: Closure<dyn FnMut(Event)>,
        capture: bool,
    }

    pub struct TextAgentHost {
        canvas: HtmlCanvasElement,
        input: HtmlInputElement,
        listeners: Vec<Listener>,
        input_state: Rc<RefCell<TextAgentInputState>>,
        clipboard_payload: Rc<RefCell<String>>,
        active: Rc<Cell<bool>>,
        active_wakes: Rc<RefCell<HashMap<RuntimeWakeKey, u64>>>,
        event_proxy: EventLoopProxy<WinitWgpuEvent>,
        logical_canvas_size: [f32; 2],
        keyboard_policy: Rc<RefCell<Option<KeyboardPolicy>>>,
        tab_entry: Rc<Cell<Option<bool>>>,
        pointer: Rc<Cell<Option<i32>>>,
        captured: Rc<Cell<bool>>,
    }

    impl TextAgentHost {
        pub fn new(
            canvas: HtmlCanvasElement,
            event_proxy: EventLoopProxy<WinitWgpuEvent>,
        ) -> Result<Self, JsValue> {
            Self::new_with_clipboard_payload_provider(canvas, event_proxy, None)
        }

        pub fn new_with_clipboard_payload_provider(
            canvas: HtmlCanvasElement,
            event_proxy: EventLoopProxy<WinitWgpuEvent>,
            clipboard_payload_provider: Option<ClipboardPayloadProvider>,
        ) -> Result<Self, JsValue> {
            let window = web_sys::window().ok_or_else(|| JsValue::from_str("missing window"))?;
            let document = window
                .document()
                .ok_or_else(|| JsValue::from_str("missing document"))?;
            let input = document
                .create_element("input")?
                .dyn_into::<HtmlInputElement>()?;
            input.set_type("text");
            input.set_attribute("aria-hidden", "true")?;
            input.set_attribute("autocomplete", "off")?;
            input.set_attribute("autocapitalize", "off")?;
            input.set_attribute("spellcheck", "false")?;
            input.set_tab_index(-1);
            let style = input.style();
            style.set_property("position", "fixed")?;
            style.set_property("width", "1px")?;
            style.set_property("height", "1px")?;
            style.set_property("opacity", "0")?;
            style.set_property("pointer-events", "none")?;
            style.set_property("z-index", "-1")?;
            style.set_property("left", "-10000px")?;
            style.set_property("top", "-10000px")?;
            canvas
                .parent_node()
                .ok_or_else(|| JsValue::from_str("canvas has no parent"))?
                .insert_before(&input, canvas.next_sibling().as_ref())?;

            let mut host = Self {
                canvas,
                input,
                listeners: Vec::new(),
                input_state: Rc::new(RefCell::new(TextAgentInputState::default())),
                clipboard_payload: Rc::new(RefCell::new(String::new())),
                active: Rc::new(Cell::new(false)),
                active_wakes: Rc::new(RefCell::new(HashMap::new())),
                event_proxy,
                logical_canvas_size: [1.0, 1.0],
                keyboard_policy: Default::default(),
                tab_entry: Rc::new(Cell::new(None)),
                pointer: Rc::new(Cell::new(None)),
                captured: Rc::new(Cell::new(false)),
            };
            host.install_canvas_listeners(&document)?;
            host.install_input_listeners()?;
            host.install_focus_listeners(&window)?;
            host.install_clipboard_listeners(document.as_ref(), clipboard_payload_provider)?;
            Ok(host)
        }

        pub fn set_logical_canvas_size(&mut self, size: [f32; 2]) {
            if size[0].is_finite() && size[1].is_finite() && size[0] > 0.0 && size[1] > 0.0 {
                self.logical_canvas_size = size;
            }
        }

        pub fn set_clipboard_payload(&self, payload: impl Into<String>) {
            *self.clipboard_payload.borrow_mut() = payload.into();
        }

        pub fn reset_for_replacement(&mut self, size: [f32; 2]) {
            self.active_wakes.borrow_mut().clear();
            self.apply_commands(vec![RuntimeHostCommand::SetImeAllowed { allowed: false }]);
            // A queued composition end still belongs to the retired session.
            self.input_state.borrow_mut().session = None;
            self.input.set_value("");
            *self.keyboard_policy.borrow_mut() = None;
            self.release_pointer();
            self.tab_entry.set(None);
            self.set_clipboard_payload("");
            self.set_logical_canvas_size(size);
        }

        pub fn apply_commands(&mut self, commands: Vec<RuntimeHostCommand>) {
            for command in commands {
                match command {
                    RuntimeHostCommand::RequestWakeup {
                        key,
                        deadline,
                        generation,
                    } => self.request_wakeup(key, deadline, generation),
                    RuntimeHostCommand::CancelWakeup { key } => {
                        self.active_wakes.borrow_mut().remove(&key);
                    }
                    RuntimeHostCommand::SetInputSession { session } => {
                        self.input_state.borrow_mut().session = session;
                    }
                    RuntimeHostCommand::SetKeyboardPolicy { policy } => {
                        *self.keyboard_policy.borrow_mut() = policy
                    }
                    RuntimeHostCommand::SetPointerCapture { captured } => {
                        if captured {
                            if let Some(id) = self.pointer.get() {
                                if self.canvas.set_pointer_capture(id).is_ok() {
                                    self.captured.set(true);
                                } else {
                                    let _ = self
                                        .event_proxy
                                        .send_event(App(WindowEvent::PointerCaptureLost));
                                }
                            } else {
                                let _ = self
                                    .event_proxy
                                    .send_event(App(WindowEvent::PointerCaptureLost));
                            }
                        } else {
                            self.release_pointer();
                        }
                    }
                    RuntimeHostCommand::SetClipboardPayload { text } => {
                        self.set_clipboard_payload(text)
                    }
                    RuntimeHostCommand::SetImeAllowed { allowed } => {
                        let changed = self.active.replace(allowed) != allowed;
                        if allowed {
                            let canvas_focused = self
                                .input
                                .owner_document()
                                .and_then(|doc| doc.active_element())
                                .is_some_and(|element| element == self.canvas.clone().into());
                            if changed || canvas_focused {
                                let _ = self.input.focus();
                            }
                        } else {
                            self.input_state.borrow_mut().begin_key_input();
                            // Do not steal focus back from another page control.
                            if self
                                .input
                                .owner_document()
                                .and_then(|doc| doc.active_element())
                                .is_some_and(|element| element == self.input.clone().into())
                            {
                                let _ = self.canvas.focus();
                            }
                        }
                        if changed {
                            let event = if allowed {
                                ImeEvent::Enabled
                            } else {
                                ImeEvent::Disabled
                            };
                            let _ = self.event_proxy.send_event(App(WindowEvent::Ime(event)
                                .with_input_session(self.input_state.borrow().session.clone())));
                        }
                    }
                    RuntimeHostCommand::SetImeCursorArea { rect } => {
                        self.position_at_caret(rect);
                    }
                    RuntimeHostCommand::WriteClipboard { text } => {
                        self.set_clipboard_payload(text.clone());
                        if let Some(window) = web_sys::window() {
                            let _ = window.navigator().clipboard().write_text(&text);
                        }
                    }
                    // The canvas presenter owns transient tooltip overlays.
                    RuntimeHostCommand::UpdateTooltip(_) => {}
                }
            }
        }

        fn release_pointer(&self) {
            self.captured.set(false);
            if let Some(id) = self.pointer.get() {
                let _ = self.canvas.release_pointer_capture(id);
            }
        }

        fn install_canvas_listeners(
            &mut self,
            document: &web_sys::Document,
        ) -> Result<(), JsValue> {
            let entry = self.tab_entry.clone();
            self.add_listener_mode(document.as_ref(), "keydown", true, move |event| {
                let event = event.unchecked_into::<KeyboardEvent>();
                entry.set((event.key() == "Tab").then_some(event.shift_key()));
            })?;
            let entry = self.tab_entry.clone();
            self.add_listener_mode(document.as_ref(), "pointerdown", true, move |_| {
                entry.set(None)
            })?;
            let target: EventTarget = self.canvas.clone().into();
            // Winit dispatches the press from pointerdown. If that press opens
            // the text agent, the following mousedown must not refocus canvas.
            let active = self.active.clone();
            self.add_listener(&target, "mousedown", move |event| {
                if active.get() {
                    event.prevent_default();
                }
            })?;
            let policy = self.keyboard_policy.clone();
            // Capture the installed policy before winit can dispatch a focus-changing key.
            self.add_listener_mode(&target, "keydown", true, move |event| {
                let event = event.unchecked_into::<KeyboardEvent>();
                let modifiers = avenger_eventstream::scene::ModifiersState {
                    shift: event.shift_key(),
                    control: event.ctrl_key(),
                    alt: event.alt_key(),
                    meta: event.meta_key(),
                };
                if let (Some(policy), Some(key)) =
                    (policy.borrow().as_ref(), browser_key(&event.key()))
                {
                    if policy.captures(key, modifiers) {
                        event.prevent_default();
                    }
                }
            })?;
            let pointer = self.pointer.clone();
            // Gesture commands can run synchronously inside winit's pointerdown listener.
            self.add_listener_mode(&target, "pointerdown", true, move |event| {
                let event = event.unchecked_into::<PointerEvent>();
                if event.button() == 0 {
                    pointer.set(Some(event.pointer_id()));
                }
            })?;
            let pointer = self.pointer.clone();
            self.add_listener(document.as_ref(), "pointerup", move |event| {
                let event = event.unchecked_into::<PointerEvent>();
                if event.button() == 0 && pointer.get() == Some(event.pointer_id()) {
                    pointer.set(None);
                }
            })?;
            for name in ["lostpointercapture", "pointercancel"] {
                let captured = self.captured.clone();
                let pointer = self.pointer.clone();
                let proxy = self.event_proxy.clone();
                self.add_listener(&target, name, move |_| {
                    // Normal pointerup already ends the gesture before automatic capture release.
                    if captured.replace(false) && pointer.get().is_some() {
                        let _ = proxy.send_event(App(WindowEvent::PointerCaptureLost));
                    }
                })?;
            }
            Ok(())
        }

        fn add_listener(
            &mut self,
            target: &EventTarget,
            name: &'static str,
            callback: impl FnMut(Event) + 'static,
        ) -> Result<(), JsValue> {
            self.add_listener_mode(target, name, false, callback)
        }

        fn add_listener_mode(
            &mut self,
            target: &EventTarget,
            name: &'static str,
            capture: bool,
            callback: impl FnMut(Event) + 'static,
        ) -> Result<(), JsValue> {
            let callback = Closure::wrap(Box::new(callback) as Box<dyn FnMut(Event)>);
            target.add_event_listener_with_callback_and_bool(
                name,
                callback.as_ref().unchecked_ref(),
                capture,
            )?;
            self.listeners.push(Listener {
                target: target.clone(),
                name,
                callback,
                capture,
            });
            Ok(())
        }

        fn install_input_listeners(&mut self) -> Result<(), JsValue> {
            let target: EventTarget = self.input.clone().into();

            let state = self.input_state.clone();
            self.add_listener(&target, "compositionstart", move |_| {
                let mut state = state.borrow_mut();
                state.composition_session = Some(state.session.clone());
            })?;

            let proxy = self.event_proxy.clone();
            let state = self.input_state.clone();
            self.add_listener(&target, "compositionupdate", move |event| {
                let event = event.unchecked_into::<CompositionEvent>();
                let _ = proxy.send_event(App(state
                    .borrow_mut()
                    .composition_update(event.data().unwrap_or_default())));
            })?;

            let proxy = self.event_proxy.clone();
            let state = self.input_state.clone();
            self.add_listener(&target, "compositionend", move |event| {
                let event = event.unchecked_into::<CompositionEvent>();
                let output = state
                    .borrow_mut()
                    .composition_end(event.data().unwrap_or_default());
                let _ = proxy.send_event(App(output));
            })?;

            let proxy = self.event_proxy.clone();
            let state = self.input_state.clone();
            self.add_listener(&target, "input", move |event| {
                let event = event.unchecked_into::<InputEvent>();
                if let Some(data) = event.data() {
                    if let Some(output) = state.borrow_mut().input(data) {
                        let _ = proxy.send_event(App(output));
                    }
                }
                if let Some(input) = event
                    .target()
                    .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
                {
                    input.set_value("");
                }
            })?;

            for (name, element_state) in [
                ("keydown", ElementState::Pressed),
                ("keyup", ElementState::Released),
            ] {
                let proxy = self.event_proxy.clone();
                let state = self.input_state.clone();
                let policy = self.keyboard_policy.clone();
                self.add_listener(&target, name, move |event| {
                    let event = event.unchecked_into::<KeyboardEvent>();
                    if event.is_composing() {
                        return;
                    }
                    if element_state == ElementState::Pressed {
                        state.borrow_mut().begin_key_input();
                    }
                    if let Some(mut output) = forwarded_browser_key_event(
                        &event.key(),
                        element_state,
                        event.ctrl_key(),
                        event.meta_key(),
                    ) {
                        let modifiers = avenger_eventstream::scene::ModifiersState {
                            shift: event.shift_key(),
                            control: event.ctrl_key(),
                            alt: event.alt_key(),
                            meta: event.meta_key(),
                        };
                        let prevent = policy.borrow().as_ref().map_or_else(
                            || {
                                prevent_browser_key_default(
                                    &event.key(),
                                    event.ctrl_key(),
                                    event.meta_key(),
                                )
                            },
                            |p| {
                                browser_key(&event.key())
                                    .is_some_and(|key| p.captures(key, modifiers))
                            },
                        );
                        if prevent && !event.get_modifier_state("AltGraph") {
                            event.prevent_default();
                        }
                        if let WindowEvent::KeyboardInput(input) = &mut output {
                            input.repeat = event.repeat();
                        }
                        let _ = proxy.send_event(App(WindowEvent::ModifiersChanged(modifiers)));
                        let output = output.with_input_session(state.borrow().session.clone());
                        let _ = proxy.send_event(App(output));
                    }
                })?;
            }
            Ok(())
        }

        fn install_focus_listeners(&mut self, window: &web_sys::Window) -> Result<(), JsValue> {
            // The canvas and hidden input form one control. Winit reports a canvas
            // blur when IME takes focus, which must not cancel the editing session.
            for target in [
                EventTarget::from(self.canvas.clone()),
                EventTarget::from(self.input.clone()),
            ] {
                let proxy = self.event_proxy.clone();
                let entry = self.tab_entry.clone();
                let canvas: EventTarget = self.canvas.clone().into();
                let input: EventTarget = self.input.clone().into();
                self.add_listener(&target, "focus", move |event| {
                    let event = event.unchecked_into::<FocusEvent>();
                    let internal = event
                        .related_target()
                        .is_some_and(|t| t == canvas || t == input);
                    let _ = proxy.send_event(App(WindowEvent::WindowFocused(true)));
                    if !internal {
                        if let Some(reverse) = entry.take() {
                            let _ = proxy.send_event(App(WindowEvent::FocusEntered { reverse }));
                        }
                    }
                })?;
                let proxy = self.event_proxy.clone();
                let canvas = EventTarget::from(self.canvas.clone());
                let input = EventTarget::from(self.input.clone());
                self.add_listener(&target, "blur", move |event| {
                    let event = event.unchecked_into::<FocusEvent>();
                    if event
                        .related_target()
                        .is_some_and(|target| target == canvas || target == input)
                    {
                        return;
                    }
                    let _ = proxy.send_event(App(WindowEvent::WindowFocused(false)));
                })?;
            }
            let proxy = self.event_proxy.clone();
            self.add_listener(window.as_ref(), "blur", move |_| {
                let _ = proxy.send_event(App(WindowEvent::WindowFocused(false)));
            })?;
            let proxy = self.event_proxy.clone();
            let canvas = self.canvas.clone();
            let input = self.input.clone();
            self.add_listener(window.as_ref(), "focus", move |_| {
                let focused = input.owner_document().and_then(|doc| doc.active_element());
                if focused.is_some_and(|element| {
                    element == input.clone().into() || element == canvas.clone().into()
                }) {
                    let _ = proxy.send_event(App(WindowEvent::WindowFocused(true)));
                }
            })?;
            Ok(())
        }

        fn install_clipboard_listeners(
            &mut self,
            document: &EventTarget,
            clipboard_payload_provider: Option<ClipboardPayloadProvider>,
        ) -> Result<(), JsValue> {
            for name in ["copy", "cut"] {
                let proxy = self.event_proxy.clone();
                let payload = self.clipboard_payload.clone();
                let provider = clipboard_payload_provider.clone();
                let active = self.active.clone();
                let state = self.input_state.clone();
                let canvas = self.canvas.clone();
                let input = self.input.clone();
                self.add_listener(document, name, move |event| {
                    if (!active.get() && state.borrow().session.is_none())
                        || !canvas
                            .owner_document()
                            .and_then(|d| d.active_element())
                            .is_some_and(|e| {
                                e == canvas.clone().into() || e == input.clone().into()
                            })
                    {
                        return;
                    }
                    let event = event.unchecked_into::<DomClipboardEvent>();
                    if let Some(data) = event.clipboard_data() {
                        let fallback = payload.borrow().clone();
                        let payload = resolved_clipboard_payload(provider.as_ref(), &fallback);
                        let _ = data.set_data("text/plain", &payload);
                        event.prevent_default();
                    }
                    if let Some(output) = browser_clipboard_event(name, None) {
                        let _ = proxy.send_event(App(
                            output.with_input_session(state.borrow().session.clone())
                        ));
                    }
                })?;
            }

            let proxy = self.event_proxy.clone();
            let active = self.active.clone();
            let state = self.input_state.clone();
            let canvas = self.canvas.clone();
            let input = self.input.clone();
            self.add_listener(document, "paste", move |event| {
                if (!active.get() && state.borrow().session.is_none())
                    || !canvas
                        .owner_document()
                        .and_then(|d| d.active_element())
                        .is_some_and(|e| e == canvas.clone().into() || e == input.clone().into())
                {
                    return;
                }
                let event = event.unchecked_into::<DomClipboardEvent>();
                let Some(data) = event.clipboard_data() else {
                    return;
                };
                if let Ok(text) = data.get_data("text/plain") {
                    event.prevent_default();
                    if let Some(output) = browser_clipboard_event("paste", Some(text)) {
                        let _ = proxy.send_event(App(
                            output.with_input_session(state.borrow().session.clone())
                        ));
                    }
                }
            })?;
            Ok(())
        }

        fn request_wakeup(&self, key: RuntimeWakeKey, deadline: Instant, generation: u64) {
            self.active_wakes
                .borrow_mut()
                .insert(key.clone(), generation);
            let active = self.active_wakes.clone();
            let proxy = self.event_proxy.clone();
            let delay = deadline.saturating_duration_since(Instant::now());
            let callback = Closure::once(move || {
                let current = active.borrow().get(&key).copied();
                if current == Some(generation) {
                    active.borrow_mut().remove(&key);
                    let _ = proxy.send_event(App(WindowEvent::RuntimeWake(RuntimeWakeEvent {
                        key,
                        generation,
                    })));
                }
            });
            let delay_ms = delay.as_millis().min(i32::MAX as u128) as i32;
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.as_ref().unchecked_ref(),
                    delay_ms,
                );
                callback.forget();
            }
        }

        fn position_at_caret(&self, rect: Option<LogicalRect>) {
            let rect = rect.and_then(|rect| {
                let canvas_rect = self.canvas.get_bounding_client_rect();
                logical_to_client_rect(
                    rect,
                    self.logical_canvas_size,
                    [
                        canvas_rect.left(),
                        canvas_rect.top(),
                        canvas_rect.width(),
                        canvas_rect.height(),
                    ],
                )
            });
            let (x, y) = rect.map_or((-10_000.0, -10_000.0), |rect| (rect[0], rect[1]));
            let style = self.input.style();
            let _ = style.set_property("left", &format!("{x}px"));
            let _ = style.set_property("top", &format!("{y}px"));
        }
    }

    impl Drop for TextAgentHost {
        fn drop(&mut self) {
            self.active_wakes.borrow_mut().clear();
            for listener in self.listeners.drain(..) {
                let _ = listener
                    .target
                    .remove_event_listener_with_callback_and_bool(
                        listener.name,
                        listener.callback.as_ref().unchecked_ref(),
                        listener.capture,
                    );
            }
            self.input.remove();
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::TextAgentHost;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_commit_suppresses_only_its_matching_input_once() {
        let mut state = TextAgentInputState::default();
        assert_eq!(
            state.composition_update("かな"),
            WindowEvent::Ime(ImeEvent::Preedit {
                text: "かな".into(),
                cursor: None,
            })
        );
        assert_eq!(
            state.composition_end("仮名"),
            WindowEvent::Ime(ImeEvent::Commit("仮名".into()))
        );
        assert_eq!(state.input("仮名"), None);
        assert!(state.input("仮名").is_some());

        state.composition_end("expected");
        let ordinary = state.input("different");
        assert!(ordinary.is_some());
        assert!(state.input("expected").is_some());

        state.composition_end("same");
        state.begin_key_input();
        assert!(state.input("same").is_some());
    }

    #[test]
    fn ordinary_and_dead_key_input_preserve_full_text_once() {
        let mut state = TextAgentInputState::default();
        assert_eq!(browser_key_event("Dead", ElementState::Pressed), None);
        let event = state.input("e\u{301}🙂").unwrap();
        let WindowEvent::KeyboardInput(input) = event else {
            panic!("expected keyboard input")
        };
        assert_eq!(input.key, Key::Character('e'));
        assert_eq!(input.text.as_deref(), Some("e\u{301}🙂"));
    }

    #[test]
    fn hidden_input_keys_are_action_only_and_clipboard_chords_are_suppressed() {
        for key in ["ArrowLeft", "Backspace", "Delete", "Enter", "Escape"] {
            for state in [ElementState::Pressed, ElementState::Released] {
                let WindowEvent::KeyboardInput(input) = browser_key_event(key, state).unwrap()
                else {
                    panic!("expected keyboard input")
                };
                assert_eq!(input.text, None);
                assert_eq!(input.state, state);
            }
        }
        assert!(is_browser_clipboard_shortcut("c", true, false));
        assert!(is_browser_clipboard_shortcut("X", false, true));
        assert!(is_browser_clipboard_shortcut("v", true, false));
        assert!(!is_browser_clipboard_shortcut("v", false, false));
        for key in ["c", "x", "v"] {
            assert_eq!(
                forwarded_browser_key_event(key, ElementState::Pressed, true, false),
                None
            );
            assert!(browser_key_event(key, ElementState::Pressed).is_some());
        }
    }

    #[test]
    fn printable_browser_keys_keep_their_default_text_input() {
        for key in [
            "a", "é", "$", " ", "Shift", "Alt", "Meta", "Control", "Dead",
        ] {
            assert!(!prevent_browser_key_default(key, false, false), "{key}");
        }
        for key in ["ArrowLeft", "Backspace", "Enter", "Escape", "Tab"] {
            assert!(prevent_browser_key_default(key, false, false), "{key}");
        }
        assert!(prevent_browser_key_default("a", true, false));
        assert!(prevent_browser_key_default("a", false, true));
    }

    #[test]
    fn css_scaled_canvas_maps_logical_caret_with_scale_and_offset() {
        let rect = LogicalRect::new(10.0, 20.0, 30.0, 12.0).unwrap();
        assert_eq!(
            logical_to_client_rect(rect, [200.0, 100.0], [100.0, 50.0, 400.0, 300.0]),
            Some([120.0, 110.0, 60.0, 36.0])
        );
    }

    #[test]
    fn dom_clipboard_sequences_emit_one_semantic_action_each() {
        use avenger_eventstream::window::ClipboardEvent;

        assert_eq!(
            browser_clipboard_event("copy", None),
            Some(WindowEvent::Clipboard(ClipboardEvent::Copy))
        );
        assert_eq!(
            browser_clipboard_event("cut", None),
            Some(WindowEvent::Clipboard(ClipboardEvent::Cut))
        );
        assert_eq!(
            browser_clipboard_event("paste", Some("value".to_string())),
            Some(WindowEvent::Clipboard(ClipboardEvent::Paste(
                "value".into()
            )))
        );
    }

    #[test]
    fn synchronous_clipboard_provider_overrides_cached_fallback() {
        let provider: ClipboardPayloadProvider =
            std::sync::Arc::new(|| Some("focused selection".to_string()));
        assert_eq!(
            resolved_clipboard_payload(Some(&provider), "stale cache"),
            "focused selection"
        );
        let empty: ClipboardPayloadProvider = std::sync::Arc::new(|| None);
        assert_eq!(
            resolved_clipboard_payload(Some(&empty), "cached selection"),
            "cached selection"
        );
    }
    #[test]
    fn delayed_composition_keeps_its_original_session() {
        use avenger_eventstream::window::{SessionInputEvent, TextInputEvent};
        let old = InputSession {
            owner: "field-a".into(),
            generation: 1,
        };
        let new = InputSession {
            owner: "field-b".into(),
            generation: 2,
        };
        let mut state = TextAgentInputState {
            session: Some(old.clone()),
            ..Default::default()
        };
        state.composition_update("中");
        state.session = Some(new.clone());
        assert!(
            matches!(state.composition_end("中"), WindowEvent::TextInput(SessionInputEvent { session, event: TextInputEvent::Ime(ImeEvent::Commit(_)) }) if session == old)
        );
        assert!(state.input("中").is_none());
        assert!(
            matches!(state.input("next"), Some(WindowEvent::TextInput(SessionInputEvent { session, .. })) if session == new)
        );
    }
}
