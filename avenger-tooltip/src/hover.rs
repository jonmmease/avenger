//! The tooltips' hover: the focus at the pointer, which the tooltips' overlay shows. Each change
//! asks the app for the overlay only, so a hover never builds the scene or its hit-testing tree.

use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_geometry::focus::Focus;
use avenger_geometry::rtree::SceneGraphRTree;

use crate::tooltips::{follow, TooltipAnchor, TooltipTarget};

/// The farthest that a press can move before its release, and still be a click that pins.
const CLICK_SLOP: f32 = 4.0;

/// The tooltips' hover: how it finds the focus at the pointer, and the focus that it shows.
#[derive(Debug, Clone)]
pub(crate) struct Hover {
    pub(crate) targets: Vec<TooltipTarget>,
    pub(crate) pin: bool,
    pub(crate) delay: Option<u64>,
    pub(crate) focus: Option<Focus>,
    /// The index of the target that found the focus.
    pub(crate) target: usize,
    pub(crate) pinned: bool,
    /// The pointer's last position, or `None` once it leaves the window.
    pointer: Option<[f32; 2]>,
    /// Where a mouse button went down, while it's down, so the pointer drags instead of
    /// hovering.
    pressed: Option<[f32; 2]>,
}

const OVERLAY: UpdateStatus = UpdateStatus {
    rerender: false,
    rebuild_geometry: false,
    overlay: true,
};

impl Hover {
    pub(crate) fn over(targets: Vec<TooltipTarget>) -> Self {
        Self {
            targets,
            pin: false,
            delay: None,
            focus: None,
            target: 0,
            pinned: false,
            pointer: None,
            pressed: None,
        }
    }

    /// The event streams that drive `hover`.
    ///
    /// A move shows the focus at the pointer, or hides the tooltip when there's none. A drag,
    /// the wheel, Escape, the window's losing focus, and the pointer's leaving the window hide
    /// it. With pinning, a click on a focus pins it: a pinned tooltip ignores moves, drags, the
    /// wheel, focus loss, and the pointer's leaving, and hides on a click off every focus or
    /// on Escape. When the app rebuilds the hit-testing tree, a pinned focus finds its instances
    /// again, and hides when one of them is gone, and a hovered focus is found again at the
    /// pointer.
    pub(crate) fn streams<State: Clone + Send + Sync + 'static>(
        hover: &Arc<Mutex<Hover>>,
    ) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        let types = vec![
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseDown,
            SceneGraphEventType::MouseUp,
            SceneGraphEventType::Click,
            SceneGraphEventType::MouseWheel,
            SceneGraphEventType::KeyPress,
            SceneGraphEventType::WindowFocused,
            SceneGraphEventType::CursorLeft,
            SceneGraphEventType::TreeRebuilt,
        ];
        let immediate = |types| EventStreamConfig {
            types,
            ..Default::default()
        };
        let driver = |show, rests| {
            Arc::new(Driver {
                hover: hover.clone(),
                show,
                rests,
            })
        };
        let Some(delay) = lock(hover).delay else {
            return vec![(immediate(types), driver(true, false))];
        };
        // A move hides a focus that the pointer left at once, and shows the new focus once the
        // pointer rests.
        let rests = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            debounce: Some(DebounceConfig::new(delay)),
            ..Default::default()
        };
        vec![
            (immediate(types), driver(false, false)),
            (rests, driver(true, true)),
        ]
    }

    /// Whether the tooltip shows `focus`.
    fn shows(&self, focus: Option<&Focus>) -> bool {
        match (&self.focus, focus) {
            (Some(shown), Some(focus)) => shown.same_members(focus),
            (shown, focus) => shown.is_none() && focus.is_none(),
        }
    }

    /// The focus at `pointer` of the first target that finds one, with the target's index. The
    /// target that found `shown` keeps its members' marks in place, as
    /// [`SceneGraphRTree::refocus`] does.
    fn find(
        &self,
        pointer: [f32; 2],
        rtree: &SceneGraphRTree,
        shown: Option<&Focus>,
    ) -> Option<(usize, Focus)> {
        self.targets.iter().enumerate().find_map(|(index, target)| {
            let shown = shown.filter(|_| index == self.target);
            rtree
                .refocus(pointer, &target.strategy, target.marks.as_deref(), shown)
                .map(|focus| (index, focus))
        })
    }

    /// Show the focus that a target `found`, and ask for the overlay when it differs from the
    /// shown one, or when a tooltip at the pointer follows the pointer within it.
    fn show(&mut self, found: Option<(usize, Focus)>) -> UpdateStatus {
        let (target, focus) = match found {
            Some((target, focus)) => (target, Some(focus)),
            None => (self.target, None),
        };
        if !self.shows(focus.as_ref()) || target != self.target {
            self.focus = focus;
            self.target = target;
            return OVERLAY;
        }
        let anchor = self.targets.get(target).map(|target| target.anchor);
        match (anchor, &mut self.focus, focus) {
            (Some(TooltipAnchor::Pointer), Some(shown), Some(focus))
                if shown.pointer != focus.pointer =>
            {
                shown.pointer = focus.pointer;
                OVERLAY
            }
            _ => UpdateStatus::default(),
        }
    }

    /// Unpin and hide the tooltip.
    fn hide(&mut self) -> UpdateStatus {
        self.pinned = false;
        self.show(None)
    }

    /// Hide the tooltip, unless it's pinned.
    fn hide_hover(&mut self) -> UpdateStatus {
        if self.pinned {
            UpdateStatus::default()
        } else {
            self.show(None)
        }
    }

    /// Pin the focus at a click's `position`, or unpin and hide the tooltip off every focus.
    /// A press that moved farther than a click before its release drags, and pins nothing.
    fn click(&mut self, position: [f32; 2], rtree: &SceneGraphRTree) -> UpdateStatus {
        let dragged = self
            .pressed
            .is_some_and(|at| (at[0] - position[0]).hypot(at[1] - position[1]) > CLICK_SLOP);
        if dragged {
            return UpdateStatus::default();
        }
        match self.find(position, rtree, None) {
            Some(found) => {
                self.pinned = true;
                self.show(Some(found))
            }
            None => self.hide(),
        }
    }

    /// Find the shown focus again in a rebuilt tree.
    fn find_again(&mut self, rtree: &SceneGraphRTree) -> UpdateStatus {
        let Some(shown) = &self.focus else {
            return UpdateStatus::default();
        };
        if self.pinned {
            return match rtree.find_again(shown) {
                Some(mut focus) => {
                    follow(&mut focus, shown);
                    self.show(Some((self.target, focus)))
                }
                None => self.hide(),
            };
        }
        let found = self
            .pointer
            .and_then(|pointer| self.find(pointer, rtree, Some(shown)));
        self.show(found)
    }
}

/// The hover, through a lock that a handler that panicked leaves usable.
pub(crate) fn lock(hover: &Mutex<Hover>) -> MutexGuard<'_, Hover> {
    hover.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Drives a hover from its events. A driver that doesn't `show` only hides a focus that the
/// pointer left, for a delayed hover's other stream to show the new one, which handles a move
/// once the pointer `rests`.
struct Driver {
    hover: Arc<Mutex<Hover>>,
    show: bool,
    rests: bool,
}

#[async_trait]
impl<State: Clone + Send + Sync + 'static> EventStreamHandler<State> for Driver {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut State,
        rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut hover = lock(&self.hover);
        match event {
            SceneGraphEvent::CursorMoved(moved) => {
                if !self.rests {
                    // A move to where the pointer is, as macOS reports before each wheel step,
                    // finds nothing new: a still pointer's focus changes only when the tree
                    // rebuilds, which `TreeRebuilt` handles.
                    if hover.pointer == Some(moved.position) {
                        return UpdateStatus::default();
                    }
                    hover.pointer = Some(moved.position);
                }
                if hover.pinned || hover.pressed.is_some() {
                    return UpdateStatus::default();
                }
                let found = hover.find(moved.position, rtree, hover.focus.as_ref());
                if self.show {
                    hover.show(found)
                } else if !hover.shows(found.as_ref().map(|(_, focus)| focus)) {
                    hover.show(None)
                } else {
                    UpdateStatus::default()
                }
            }
            SceneGraphEvent::MouseDown(down) => {
                hover.pressed = Some(down.position);
                hover.hide_hover()
            }
            SceneGraphEvent::MouseUp(_) => {
                hover.pressed = None;
                UpdateStatus::default()
            }
            SceneGraphEvent::Click(click) if hover.pin && click.button == MouseButton::Left => {
                hover.click(click.position, rtree)
            }
            SceneGraphEvent::CursorLeft => {
                hover.pointer = None;
                hover.hide_hover()
            }
            SceneGraphEvent::MouseWheel(_) | SceneGraphEvent::WindowFocused(false) => {
                hover.hide_hover()
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                hover.hide()
            }
            SceneGraphEvent::TreeRebuilt => hover.find_again(rtree),
            _ => UpdateStatus::default(),
        }
    }
}
