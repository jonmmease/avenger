use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowSceneSizing {
    #[default]
    SurfaceFollowsWindow,
    MatchSceneGraph,
    MatchSceneGraphAxes {
        width: bool,
        height: bool,
    },
}

impl WindowSceneSizing {
    pub(super) fn matching_axes(self) -> (bool, bool) {
        match self {
            Self::SurfaceFollowsWindow => (false, false),
            Self::MatchSceneGraph => (true, true),
            Self::MatchSceneGraphAxes { width, height } => (width, height),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CanvasFrameOptions {
    pub resize_width: bool,
    pub resize_height: bool,
    pub min_size: [f32; 2],
    pub extra_window_size: [f32; 2],
    pub handle_thickness: f32,
}

impl Default for CanvasFrameOptions {
    fn default() -> Self {
        Self {
            resize_width: false,
            resize_height: false,
            min_size: [120.0, 120.0],
            extra_window_size: [320.0, 240.0],
            handle_thickness: 8.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CanvasFrameHandle {
    Right,
    Bottom,
    Corner,
}

impl CanvasFrameHandle {
    pub(super) fn resizes_width(self) -> bool {
        matches!(self, Self::Right | Self::Corner)
    }

    pub(super) fn resizes_height(self) -> bool {
        matches!(self, Self::Bottom | Self::Corner)
    }

    pub(super) fn cursor(self) -> CursorIcon {
        match self {
            Self::Right => CursorIcon::EResize,
            Self::Bottom => CursorIcon::SResize,
            Self::Corner => CursorIcon::SeResize,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CanvasFrameDrag {
    pub(super) handle: CanvasFrameHandle,
    pub(super) start_pointer: [f32; 2],
    pub(super) start_canvas_size: [f32; 2],
}

#[derive(Clone, Debug)]
pub(super) struct CanvasFrameState {
    pub(super) options: CanvasFrameOptions,
    pub(super) canvas_size: [f32; 2],
    pub(super) hover_handle: Option<CanvasFrameHandle>,
    pub(super) active_drag: Option<CanvasFrameDrag>,
    pub(super) last_cursor_position: Option<[f32; 2]>,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct CanvasFrameEventOutcome {
    pub(super) consumed: bool,
    pub(super) cursor: Option<CursorIcon>,
    pub(super) redraw_overlay: bool,
    pub(super) resize: Option<[f32; 2]>,
    pub(super) resize_settled: Option<[f32; 2]>,
}

impl CanvasFrameState {
    pub(super) fn new(options: CanvasFrameOptions) -> Self {
        Self {
            options,
            canvas_size: [0.0, 0.0],
            hover_handle: None,
            active_drag: None,
            last_cursor_position: None,
        }
    }

    pub(super) fn update_scene_size(&mut self, size: [f32; 2]) {
        if let Some(drag) = self.active_drag {
            if !drag.handle.resizes_width() {
                self.canvas_size[0] = size[0];
            }
            if !drag.handle.resizes_height() {
                self.canvas_size[1] = size[1];
            }
        } else {
            self.canvas_size = size;
        }
    }

    pub(super) fn initial_window_size(&self) -> [f32; 2] {
        [
            self.canvas_size[0] + self.options.extra_window_size[0],
            self.canvas_size[1] + self.options.extra_window_size[1],
        ]
    }

    pub(super) fn overlay(&self) -> CanvasFrameOverlay {
        CanvasFrameOverlay {
            size: self.canvas_size,
            resize_width: self.options.resize_width,
            resize_height: self.options.resize_height,
            handle_thickness: self.options.handle_thickness,
        }
    }

    pub(super) fn hit_test(&self, position: [f32; 2]) -> Option<CanvasFrameHandle> {
        let [x, y] = position;
        let [width, height] = self.canvas_size;
        let thickness = self.options.handle_thickness.max(1.0);
        let near_right = self.options.resize_width
            && x >= width - thickness
            && x <= width + thickness
            && y >= 0.0
            && y <= height + thickness;
        let near_bottom = self.options.resize_height
            && y >= height - thickness
            && y <= height + thickness
            && x >= 0.0
            && x <= width + thickness;

        match (near_right, near_bottom) {
            (true, true) if self.options.resize_width && self.options.resize_height => {
                Some(CanvasFrameHandle::Corner)
            }
            (true, _) => Some(CanvasFrameHandle::Right),
            (_, true) => Some(CanvasFrameHandle::Bottom),
            _ => None,
        }
    }

    pub(super) fn handle_cursor_moved(&mut self, position: [f32; 2]) -> CanvasFrameEventOutcome {
        self.last_cursor_position = Some(position);

        if let Some(drag) = self.active_drag {
            let size = self.drag_size(drag, position);
            let changed = self.canvas_size != size;
            self.canvas_size = size;
            return CanvasFrameEventOutcome {
                consumed: true,
                cursor: Some(drag.handle.cursor()),
                redraw_overlay: changed,
                resize: changed.then_some(size),
                resize_settled: None,
            };
        }

        let hover_handle = self.hit_test(position);
        let changed = self.hover_handle != hover_handle;
        self.hover_handle = hover_handle;
        CanvasFrameEventOutcome {
            consumed: hover_handle.is_some(),
            cursor: Some(hover_handle.map_or(CursorIcon::Default, CanvasFrameHandle::cursor)),
            redraw_overlay: changed,
            resize: None,
            resize_settled: None,
        }
    }

    pub(super) fn handle_cursor_left(&mut self) -> CanvasFrameEventOutcome {
        if self.active_drag.is_some() {
            return CanvasFrameEventOutcome::default();
        }
        self.last_cursor_position = None;
        let changed = self.hover_handle.take().is_some();
        CanvasFrameEventOutcome {
            consumed: false,
            cursor: Some(CursorIcon::Default),
            redraw_overlay: changed,
            resize: None,
            resize_settled: None,
        }
    }

    pub(super) fn handle_focus_lost(&mut self) -> CanvasFrameEventOutcome {
        let was_dragging = self.active_drag.take().is_some();
        self.hover_handle = None;
        self.last_cursor_position = None;
        CanvasFrameEventOutcome {
            cursor: Some(CursorIcon::Default),
            redraw_overlay: true,
            resize: was_dragging.then_some(self.canvas_size),
            resize_settled: was_dragging.then_some(self.canvas_size),
            ..Default::default()
        }
    }

    pub(super) fn handle_mouse_input(
        &mut self,
        state: ElementState,
        button: MouseButton,
    ) -> CanvasFrameEventOutcome {
        if button != MouseButton::Left {
            return CanvasFrameEventOutcome::default();
        }

        match state {
            ElementState::Pressed => {
                let Some(position) = self.last_cursor_position else {
                    return CanvasFrameEventOutcome::default();
                };
                let handle = self.hover_handle.or_else(|| self.hit_test(position));
                if let Some(handle) = handle {
                    self.active_drag = Some(CanvasFrameDrag {
                        handle,
                        start_pointer: position,
                        start_canvas_size: self.canvas_size,
                    });
                    return CanvasFrameEventOutcome {
                        consumed: true,
                        cursor: Some(handle.cursor()),
                        redraw_overlay: true,
                        resize: None,
                        resize_settled: None,
                    };
                }
                CanvasFrameEventOutcome::default()
            }
            ElementState::Released => {
                let Some(_drag) = self.active_drag.take() else {
                    return CanvasFrameEventOutcome::default();
                };
                self.hover_handle = self
                    .last_cursor_position
                    .and_then(|position| self.hit_test(position));
                CanvasFrameEventOutcome {
                    consumed: true,
                    cursor: Some(
                        self.hover_handle
                            .map_or(CursorIcon::Default, CanvasFrameHandle::cursor),
                    ),
                    redraw_overlay: true,
                    resize: Some(self.canvas_size),
                    resize_settled: Some(self.canvas_size),
                }
            }
        }
    }

    pub(super) fn drag_size(&self, drag: CanvasFrameDrag, position: [f32; 2]) -> [f32; 2] {
        let mut size = self.canvas_size;
        if drag.handle.resizes_width() {
            size[0] = (drag.start_canvas_size[0] + position[0] - drag.start_pointer[0])
                .max(self.options.min_size[0]);
        }
        if drag.handle.resizes_height() {
            size[1] = (drag.start_canvas_size[1] + position[1] - drag.start_pointer[1])
                .max(self.options.min_size[1]);
        }
        size
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame_state(resize_width: bool, resize_height: bool) -> CanvasFrameState {
        let mut state = CanvasFrameState::new(CanvasFrameOptions {
            resize_width,
            resize_height,
            min_size: [120.0, 100.0],
            extra_window_size: [320.0, 240.0],
            handle_thickness: 8.0,
        });
        state.update_scene_size([400.0, 300.0]);
        state
    }
    #[test]
    fn frame_hit_testing_selects_enabled_handles() {
        let width_only = frame_state(true, false);
        assert_eq!(
            width_only.hit_test([398.0, 150.0]),
            Some(CanvasFrameHandle::Right)
        );
        assert_eq!(width_only.hit_test([200.0, 298.0]), None);

        let height_only = frame_state(false, true);
        assert_eq!(
            height_only.hit_test([200.0, 298.0]),
            Some(CanvasFrameHandle::Bottom)
        );
        assert_eq!(height_only.hit_test([398.0, 150.0]), None);

        let both = frame_state(true, true);
        assert_eq!(
            both.hit_test([399.0, 299.0]),
            Some(CanvasFrameHandle::Corner)
        );
        assert_eq!(both.hit_test([200.0, 200.0]), None);
    }
    #[test]
    fn frame_drag_clamps_to_min_size() {
        let mut state = frame_state(true, true);
        state.handle_cursor_moved([400.0, 300.0]);
        let press = state.handle_mouse_input(ElementState::Pressed, MouseButton::Left);
        assert!(press.consumed);

        let drag = state.handle_cursor_moved([20.0, 20.0]);
        assert!(drag.consumed);
        assert_eq!(drag.resize, Some([120.0, 100.0]));
        assert_eq!(state.canvas_size, [120.0, 100.0]);

        let release = state.handle_mouse_input(ElementState::Released, MouseButton::Left);
        assert_eq!(release.resize_settled, Some([120.0, 100.0]));
    }
    #[test]
    fn focus_loss_finishes_the_frame_drag_at_its_latest_size() {
        let mut frame = frame_state(true, true);
        frame.handle_cursor_moved([400.0, 300.0]);
        frame.handle_mouse_input(ElementState::Pressed, MouseButton::Left);
        frame.handle_cursor_moved([450.0, 340.0]);
        let result = frame.handle_focus_lost();
        assert!(!result.consumed);
        assert_eq!(result.resize, Some([450.0, 340.0]));
        assert_eq!(result.resize_settled, result.resize);
        assert!(frame.active_drag.is_none());
        assert!(frame.handle_cursor_moved([500.0, 400.0]).resize.is_none());
    }
}
