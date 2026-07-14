use avenger_eventstream::window::{
    CanvasResizeEvent, ElementState, Key, MouseButton, MouseScrollDelta, NamedKey,
    WindowCursorMoved, WindowEvent, WindowKeyboardInput, WindowMouseInput, WindowMouseWheel,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderPendingEventKey {
    WindowResize,
    CanvasResize,
    WindowMoved,
    WindowFocused,
    WindowCloseRequested,
    CursorMoved,
    CursorPresence,
    MouseWheel,
    Touch,
    Immediate,
}

fn render_pending_event_key(event: &WindowEvent) -> RenderPendingEventKey {
    match event {
        WindowEvent::WindowResize(_) => RenderPendingEventKey::WindowResize,
        WindowEvent::CanvasResize(_) => RenderPendingEventKey::CanvasResize,
        WindowEvent::WindowMoved(_) => RenderPendingEventKey::WindowMoved,
        WindowEvent::WindowFocused(_) => RenderPendingEventKey::WindowFocused,
        WindowEvent::WindowCloseRequested => RenderPendingEventKey::WindowCloseRequested,
        WindowEvent::CursorMoved(_) => RenderPendingEventKey::CursorMoved,
        WindowEvent::CursorEntered | WindowEvent::CursorLeft => {
            RenderPendingEventKey::CursorPresence
        }
        WindowEvent::MouseWheel(_) => RenderPendingEventKey::MouseWheel,
        WindowEvent::Touch(_) => RenderPendingEventKey::Touch,
        WindowEvent::MouseInput(_)
        | WindowEvent::KeyboardInput(_)
        | WindowEvent::FileChanged(_)
        | WindowEvent::WindowResizeSettled(_)
        | WindowEvent::CanvasResizeSettled(_)
        | WindowEvent::InteractionSettled { .. } => RenderPendingEventKey::Immediate,
    }
}

pub(crate) fn coalesce_render_pending_event(pending: &mut Vec<WindowEvent>, event: WindowEvent) {
    if let WindowEvent::MouseWheel(new_event) = event {
        if let Some(WindowEvent::MouseWheel(existing_event)) = pending
            .iter_mut()
            .find(|candidate| matches!(candidate, WindowEvent::MouseWheel(_)))
        {
            existing_event.delta =
                coalesce_mouse_scroll_delta(existing_event.delta, new_event.delta);
        } else {
            pending.push(WindowEvent::MouseWheel(new_event));
        }
        return;
    }

    let key = render_pending_event_key(&event);
    if let Some(existing) = pending
        .iter_mut()
        .find(|candidate| render_pending_event_key(candidate) == key)
    {
        *existing = event;
    } else {
        pending.push(event);
    }
}

pub(crate) fn coalesce_mouse_scroll_delta(
    current: MouseScrollDelta,
    next: MouseScrollDelta,
) -> MouseScrollDelta {
    match (current, next) {
        (
            MouseScrollDelta::LineDelta(current_x, current_y),
            MouseScrollDelta::LineDelta(next_x, next_y),
        ) => MouseScrollDelta::LineDelta(current_x + next_x, current_y + next_y),
        (
            MouseScrollDelta::PixelDelta(current_x, current_y),
            MouseScrollDelta::PixelDelta(next_x, next_y),
        ) => MouseScrollDelta::PixelDelta(current_x + next_x, current_y + next_y),
        (_, next) => next,
    }
}

pub struct EguiResponseState {
    pub hovered: bool,
    pub drag_started: bool,
    pub drag_stopped: bool,
    pub has_focus: bool,
}

impl EguiResponseState {
    pub fn from_response(response: &egui::Response) -> Self {
        Self {
            hovered: response.hovered(),
            drag_started: response.drag_started(),
            drag_stopped: response.drag_stopped(),
            has_focus: response.has_focus(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct EguiEventTranslator {
    hovered: bool,
    pointer_captured: bool,
    last_pointer_position: Option<[f32; 2]>,
    last_size: Option<[f32; 2]>,
}

impl EguiEventTranslator {
    pub fn translate_frame(
        &mut self,
        rect: egui::Rect,
        response: EguiResponseState,
        input: &egui::InputState,
    ) -> Vec<WindowEvent> {
        let mut events = Vec::new();
        let hovered = response.hovered;

        if hovered && !self.hovered {
            events.push(WindowEvent::CursorEntered);
        } else if !hovered && self.hovered && !self.pointer_captured {
            events.push(WindowEvent::CursorLeft);
            self.last_pointer_position = None;
        }
        self.hovered = hovered;

        if response.drag_started {
            self.pointer_captured = true;
        }

        for event in &input.raw.events {
            match event {
                egui::Event::PointerMoved(pos) => {
                    if rect.contains(*pos) || self.pointer_captured {
                        self.push_cursor_moved(&mut events, rect, *pos);
                    }
                }
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    ..
                } => {
                    let inside = rect.contains(*pos);
                    if *pressed && inside {
                        self.pointer_captured = true;
                    }
                    if inside || self.pointer_captured {
                        events.push(WindowEvent::MouseInput(WindowMouseInput {
                            state: if *pressed {
                                ElementState::Pressed
                            } else {
                                ElementState::Released
                            },
                            button: egui_button_to_avenger(*button),
                        }));
                    }
                    if !*pressed {
                        self.pointer_captured = false;
                    }
                }
                egui::Event::MouseWheel { unit, delta, .. } => {
                    if hovered {
                        events.push(WindowEvent::MouseWheel(WindowMouseWheel {
                            delta: egui_wheel_to_avenger(*unit, *delta),
                        }));
                    }
                }
                egui::Event::Key { key, pressed, .. } => {
                    if response.has_focus
                        && let Some(key) = egui_key_to_avenger(*key)
                    {
                        events.push(WindowEvent::KeyboardInput(WindowKeyboardInput {
                            key,
                            text: None,
                            state: if *pressed {
                                ElementState::Pressed
                            } else {
                                ElementState::Released
                            },
                        }));
                    }
                }
                egui::Event::PointerGone => {
                    if self.hovered || self.pointer_captured {
                        events.push(WindowEvent::CursorLeft);
                    }
                    self.hovered = false;
                    self.pointer_captured = false;
                    self.last_pointer_position = None;
                }
                _ => {}
            }
        }

        if response.drag_stopped {
            self.pointer_captured = false;
            if !hovered {
                self.last_pointer_position = None;
            }
        }

        let size = [rect.width(), rect.height()];
        if self.last_size != Some(size) {
            self.last_size = Some(size);
            events.push(WindowEvent::CanvasResize(CanvasResizeEvent { size }));
        }

        events
    }

    fn push_cursor_moved(
        &mut self,
        events: &mut Vec<WindowEvent>,
        rect: egui::Rect,
        pos: egui::Pos2,
    ) {
        let position = local_position(rect, pos);
        if self.last_pointer_position != Some(position) {
            self.last_pointer_position = Some(position);
            events.push(WindowEvent::CursorMoved(WindowCursorMoved { position }));
        }
    }
}

pub fn local_position(rect: egui::Rect, pos: egui::Pos2) -> [f32; 2] {
    [pos.x - rect.min.x, pos.y - rect.min.y]
}

fn egui_button_to_avenger(button: egui::PointerButton) -> MouseButton {
    match button {
        egui::PointerButton::Primary => MouseButton::Left,
        egui::PointerButton::Secondary => MouseButton::Right,
        egui::PointerButton::Middle => MouseButton::Middle,
        egui::PointerButton::Extra1 => MouseButton::Back,
        egui::PointerButton::Extra2 => MouseButton::Forward,
    }
}

pub(crate) fn egui_wheel_to_avenger(
    unit: egui::MouseWheelUnit,
    delta: egui::Vec2,
) -> MouseScrollDelta {
    match unit {
        egui::MouseWheelUnit::Point => MouseScrollDelta::PixelDelta(delta.x as f64, delta.y as f64),
        egui::MouseWheelUnit::Line => MouseScrollDelta::LineDelta(delta.x, delta.y),
        egui::MouseWheelUnit::Page => MouseScrollDelta::LineDelta(delta.x * 24.0, delta.y * 24.0),
    }
}

pub(crate) fn egui_key_to_avenger(key: egui::Key) -> Option<Key> {
    let named = match key {
        egui::Key::ArrowDown => NamedKey::ArrowDown,
        egui::Key::ArrowLeft => NamedKey::ArrowLeft,
        egui::Key::ArrowRight => NamedKey::ArrowRight,
        egui::Key::ArrowUp => NamedKey::ArrowUp,
        egui::Key::Escape => NamedKey::Escape,
        egui::Key::Tab => NamedKey::Tab,
        egui::Key::Backspace => NamedKey::Backspace,
        egui::Key::Enter => NamedKey::Enter,
        egui::Key::Space => NamedKey::Space,
        egui::Key::Delete => NamedKey::Delete,
        egui::Key::Home => NamedKey::Home,
        egui::Key::End => NamedKey::End,
        egui::Key::PageUp => NamedKey::PageUp,
        egui::Key::PageDown => NamedKey::PageDown,
        egui::Key::F1 => NamedKey::F1,
        egui::Key::F2 => NamedKey::F2,
        egui::Key::F3 => NamedKey::F3,
        egui::Key::F4 => NamedKey::F4,
        egui::Key::F5 => NamedKey::F5,
        egui::Key::F6 => NamedKey::F6,
        egui::Key::F7 => NamedKey::F7,
        egui::Key::F8 => NamedKey::F8,
        egui::Key::F9 => NamedKey::F9,
        egui::Key::F10 => NamedKey::F10,
        egui::Key::F11 => NamedKey::F11,
        egui::Key::F12 => NamedKey::F12,
        egui::Key::A => return Some(Key::Character('a')),
        egui::Key::B => return Some(Key::Character('b')),
        egui::Key::C => return Some(Key::Character('c')),
        egui::Key::D => return Some(Key::Character('d')),
        egui::Key::E => return Some(Key::Character('e')),
        egui::Key::F => return Some(Key::Character('f')),
        egui::Key::G => return Some(Key::Character('g')),
        egui::Key::H => return Some(Key::Character('h')),
        egui::Key::I => return Some(Key::Character('i')),
        egui::Key::J => return Some(Key::Character('j')),
        egui::Key::K => return Some(Key::Character('k')),
        egui::Key::L => return Some(Key::Character('l')),
        egui::Key::M => return Some(Key::Character('m')),
        egui::Key::N => return Some(Key::Character('n')),
        egui::Key::O => return Some(Key::Character('o')),
        egui::Key::P => return Some(Key::Character('p')),
        egui::Key::Q => return Some(Key::Character('q')),
        egui::Key::R => return Some(Key::Character('r')),
        egui::Key::S => return Some(Key::Character('s')),
        egui::Key::T => return Some(Key::Character('t')),
        egui::Key::U => return Some(Key::Character('u')),
        egui::Key::V => return Some(Key::Character('v')),
        egui::Key::W => return Some(Key::Character('w')),
        egui::Key::X => return Some(Key::Character('x')),
        egui::Key::Y => return Some(Key::Character('y')),
        egui::Key::Z => return Some(Key::Character('z')),
        egui::Key::Num0 => return Some(Key::Character('0')),
        egui::Key::Num1 => return Some(Key::Character('1')),
        egui::Key::Num2 => return Some(Key::Character('2')),
        egui::Key::Num3 => return Some(Key::Character('3')),
        egui::Key::Num4 => return Some(Key::Character('4')),
        egui::Key::Num5 => return Some(Key::Character('5')),
        egui::Key::Num6 => return Some(Key::Character('6')),
        egui::Key::Num7 => return Some(Key::Character('7')),
        egui::Key::Num8 => return Some(Key::Character('8')),
        egui::Key::Num9 => return Some(Key::Character('9')),
        _ => return None,
    };
    Some(Key::Named(named))
}
