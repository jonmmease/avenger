//! The tooltips' hover: the focus at the pointer, which the tooltips' overlay shows. Each change
//! asks the app for the overlay only, so a hover never builds the scene or its hit-testing tree.

use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{
    DebounceConfig, EventStreamConfig, EventStreamFilter, UpdateStatus,
};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_geometry::focus::{Focus, FocusQuery};
use avenger_geometry::rtree::SceneGraphRTree;

use crate::target::{TooltipAnchor, TooltipTarget};
use crate::tooltips::follow;

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
    /// The shown tooltip's box, `[x, y, width, height]` in the scene, as the last build drew it.
    pub(crate) drawn: Option<[f32; 4]>,
    /// The member of the focus that the tooltip shows, when an instance target found a stack of
    /// instances drawn over each other, which a pinned tooltip pages through.
    pub(crate) page: usize,
    /// The boxes of a pinned tooltip's arrows, to the previous and the next member, in the scene,
    /// as the last build drew them.
    pub(crate) arrows: Option<[[f32; 4]; 2]>,
    /// The pointer's last position, or `None` once it leaves the window.
    pub(crate) pointer: Option<[f32; 2]>,
    /// Where a mouse button went down, while it's down, so the pointer drags instead of
    /// hovering.
    pressed: Option<[f32; 2]>,
}

impl Hover {
    pub(crate) fn over(targets: Vec<TooltipTarget>) -> Self {
        Self {
            targets,
            pin: false,
            delay: None,
            focus: None,
            target: 0,
            pinned: false,
            drawn: None,
            page: 0,
            arrows: None,
            pointer: None,
            pressed: None,
        }
    }

    /// The event streams that drive `hover`, as [`Tooltips`](crate::Tooltips) describes them.
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
        // The pinned tooltip is a surface over the scene: it takes the presses and clicks on its
        // box, so the scene's streams beneath, such as a pan, don't act on them. Moves and the
        // wheel pass through.
        let on_box = {
            let hover = hover.clone();
            EventStreamFilter(Arc::new(move |event: &SceneGraphEvent| {
                let hover = lock(&hover);
                event
                    .position()
                    .is_some_and(|point| hover.on_pinned_box(point))
            }))
        };
        let claims = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::Click,
                SceneGraphEventType::DoubleClick,
            ],
            filter: Some(vec![on_box]),
            consume: true,
            ..Default::default()
        };
        let claim = (
            claims,
            Arc::new(Claim(hover.clone())) as Arc<dyn EventStreamHandler<State>>,
        );
        let Some(delay) = lock(hover).delay else {
            return vec![claim, (immediate(types), driver(true, false))];
        };
        // A move hides a focus that the pointer left at once, and shows the new focus once the
        // pointer rests.
        let rests = EventStreamConfig {
            types: vec![SceneGraphEventType::CursorMoved],
            debounce: Some(DebounceConfig::new(delay)),
            ..Default::default()
        };
        vec![
            claim,
            (immediate(types), driver(false, false)),
            (rests, driver(true, true)),
        ]
    }

    /// How many members of the focus the tooltip pages through: an instance target's stack of
    /// instances drawn over each other, and else one.
    pub(crate) fn pages(&self) -> usize {
        match (self.targets.get(self.target), &self.focus) {
            (Some(TooltipTarget::Instance(_)), Some(focus)) => focus.members.len().max(1),
            _ => 1,
        }
    }

    /// Show the member `by` pages on from the shown one, around the stack, while the tooltip is
    /// pinned.
    fn step(&mut self, by: isize) -> UpdateStatus {
        let pages = self.pages();
        if !self.pinned || pages < 2 {
            return UpdateStatus::UNCHANGED;
        }
        self.page = (self.page as isize + by).rem_euclid(pages as isize) as usize;
        UpdateStatus::OVERLAY
    }

    /// The arrow whose side of the pinned tooltip's footer holds `point`, as the pages it steps:
    /// -1 to the previous instance, and 1 to the next.
    pub(crate) fn arrow_at(&self, [x, y]: [f32; 2]) -> Option<isize> {
        let on = |[left, top, width, height]: [f32; 4]| {
            (left..=left + width).contains(&x) && (top..=top + height).contains(&y)
        };
        let [previous, next] = self.arrows?;
        match (on(previous), on(next)) {
            (true, _) => Some(-1),
            (_, true) => Some(1),
            _ => None,
        }
    }

    /// Whether the pinned tooltip's box, as the last build drew it, holds `point`.
    pub(crate) fn on_pinned_box(&self, point: [f32; 2]) -> bool {
        self.pinned
            && self.drawn.is_some_and(|[x, y, width, height]| {
                (x..=x + width).contains(&point[0]) && (y..=y + height).contains(&point[1])
            })
    }

    /// Whether the tooltip shows `focus`.
    fn shows(&self, focus: Option<&Focus>) -> bool {
        match (&self.focus, focus) {
            (Some(shown), Some(focus)) => shown.same_members(focus),
            (shown, focus) => shown.is_none() && focus.is_none(),
        }
    }

    /// The focus at `pointer` of the first target that finds one, with the target's index. A
    /// group target that found `shown` keeps its series in place, as [`FocusQuery::showing`]
    /// does, and puts them in its order.
    fn find(
        &self,
        pointer: [f32; 2],
        rtree: &SceneGraphRTree,
        shown: Option<&Focus>,
    ) -> Option<(usize, Focus)> {
        self.targets.iter().enumerate().find_map(|(index, target)| {
            let focus = match target {
                TooltipTarget::Instance(target) => {
                    let query = FocusQuery::at(pointer, target.strategy);
                    match &target.marks {
                        Some(marks) => rtree.focus(query.among(marks)),
                        None => rtree.focus(query),
                    }
                }
                TooltipTarget::Group(target) => {
                    let mut query = FocusQuery::at(pointer, target.strategy()).among(&target.marks);
                    if let Some(shown) = shown.filter(|_| index == self.target) {
                        query = query.showing(shown);
                    }
                    let mut focus = rtree.focus(query)?;
                    target.order(&mut focus);
                    Some(focus)
                }
            };
            focus.map(|focus| (index, focus))
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
            self.page = 0;
            return UpdateStatus::OVERLAY;
        }
        let anchor = self.targets.get(target).map(TooltipTarget::anchor);
        match (anchor, &mut self.focus, focus) {
            (Some(TooltipAnchor::Pointer), Some(shown), Some(focus))
                if shown.pointer != focus.pointer =>
            {
                shown.pointer = focus.pointer;
                UpdateStatus::OVERLAY
            }
            _ => UpdateStatus::UNCHANGED,
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
            UpdateStatus::UNCHANGED
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
            return UpdateStatus::UNCHANGED;
        }
        match self.find(position, rtree, None) {
            // The tooltip draws again as a pinned one, though it shows the same focus.
            Some(found) => {
                self.pinned = true;
                self.show(Some(found));
                UpdateStatus::OVERLAY
            }
            None => self.hide(),
        }
    }

    /// Find the shown focus again in a rebuilt tree.
    fn find_again(&mut self, rtree: &SceneGraphRTree) -> UpdateStatus {
        let Some(shown) = &self.focus else {
            return UpdateStatus::UNCHANGED;
        };
        if self.pinned {
            let key = self.targets.get(self.target).and_then(TooltipTarget::key);
            let again = shown.in_data(rtree.data(), key);
            return match again.and_then(|focus| rtree.find_again(&focus)) {
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

/// Takes the presses and clicks on the pinned tooltip's box. A press notes where it went down, so
/// a release off the box after a drag from it pins nothing, and a press on an arrow pages through
/// the stack at once, so presses in quick succession, which make a double click, each page.
struct Claim(Arc<Mutex<Hover>>);

#[async_trait]
impl<State: Clone + Send + Sync + 'static> EventStreamHandler<State> for Claim {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut State,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut hover = lock(&self.0);
        match event {
            SceneGraphEvent::MouseDown(down) => {
                hover.pressed = Some(down.position);
                match hover.arrow_at(down.position) {
                    Some(by) if down.button == MouseButton::Left => hover.step(by),
                    _ => UpdateStatus::UNCHANGED,
                }
            }
            _ => UpdateStatus::UNCHANGED,
        }
    }
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
                        return UpdateStatus::UNCHANGED;
                    }
                    hover.pointer = Some(moved.position);
                }
                if hover.pinned {
                    return UpdateStatus::UNCHANGED;
                }
                // A press keeps the tooltip, for a click to pin, until it drags, as a pan does.
                if let Some(at) = hover.pressed {
                    let [x, y] = moved.position;
                    return match (at[0] - x).hypot(at[1] - y) > CLICK_SLOP {
                        true => hover.hide_hover(),
                        false => UpdateStatus::UNCHANGED,
                    };
                }
                let found = hover.find(moved.position, rtree, hover.focus.as_ref());
                if self.show {
                    hover.show(found)
                } else if !hover.shows(found.as_ref().map(|(_, focus)| focus)) {
                    hover.show(None)
                } else {
                    UpdateStatus::UNCHANGED
                }
            }
            SceneGraphEvent::MouseDown(down) => {
                hover.pressed = Some(down.position);
                UpdateStatus::UNCHANGED
            }
            SceneGraphEvent::MouseUp(_) => {
                hover.pressed = None;
                UpdateStatus::UNCHANGED
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
            SceneGraphEvent::KeyPress(press) => match press.key {
                Key::Named(NamedKey::Escape) => hover.hide(),
                Key::Named(NamedKey::ArrowLeft) => hover.step(-1),
                Key::Named(NamedKey::ArrowRight) => hover.step(1),
                _ => UpdateStatus::UNCHANGED,
            },
            SceneGraphEvent::TreeRebuilt => hover.find_again(rtree),
            _ => UpdateStatus::UNCHANGED,
        }
    }
}
