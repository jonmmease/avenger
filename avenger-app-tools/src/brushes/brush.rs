//! The brushes that a `Brushes` tool holds, with the types of their states erased.

use std::any::Any;
use std::sync::Arc;

use avenger_common::cursor::CursorIcon;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::stream::Rebuild;
use avenger_scenegraph::marks::mark::SceneMark;

use super::style::BrushStyle;

/// How far a press can lie from a box's edge or corner, in pixels, and still resize it.
pub(crate) const HANDLE_TOLERANCE: f32 = 5.0;

/// One brush of a [`Brushes`](super::Brushes) tool. Each brush kind, such as
/// [`LassoBrush`](super::LassoBrush), converts into it.
pub struct Brush<State>(pub(crate) Arc<dyn ErasedBrush<State>>);

/// A brush's gesture and drawings, with their type erased.
pub(crate) type Own = Box<dyn Any + Send + Sync>;

/// What a left-button press would do to a brush.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Claim {
    /// Move or resize an existing selection, which `cursor` shows.
    Edit(CursorIcon),
    /// Start a new selection.
    New,
}

/// What a brush did with a click.
pub(crate) struct Clicked {
    /// The app's rebuild requests.
    pub(crate) rebuild: Rebuild,
    /// Whether the click selected or toggled an instance, which keeps a box under it.
    pub(crate) hit: bool,
}

/// A brush's marks for an update, whether they differ from its last marks, and the first error,
/// such as a scale that does not invert.
#[derive(Default)]
pub(crate) struct Drawing {
    pub(crate) marks: Vec<SceneMark>,
    pub(crate) changed: bool,
    pub(crate) error: Option<String>,
}

/// A brush kind's behavior. Methods that change the brush's drawing return `Some` with the app's
/// rebuild requests, so that the tool can request its own scene. A brush that never claims
/// presses, such as a point brush, keeps the defaults of the gesture methods.
pub(crate) trait ErasedBrush<State>: Send + Sync {
    fn new_state(&self) -> Own;

    /// What a left-button press at `at` would do, or `None` when the brush is inactive or the
    /// press lies outside its areas.
    fn claim(&self, _app: &State, _at: [f32; 2]) -> Option<Claim> {
        None
    }

    /// Start the gesture that `claim` describes for a press at `at`, with Shift held if `shift`.
    /// `trees` are the trees of the brushes' scene and of the scenes below.
    fn press(
        &self,
        _app: &State,
        _own: &mut Own,
        _claim: Claim,
        _at: [f32; 2],
        _shift: bool,
        _trees: &SceneTrees<'_>,
    ) {
    }

    /// Follow a move to `at` after the pointer has moved farther from the press than a click
    /// allows, and report the gesture's value through `on_change` if `report`, or else keep it
    /// for [`flush`](Self::flush).
    fn drag(&self, _: &mut State, _: &mut Own, _at: [f32; 2], _report: bool) -> Option<Rebuild> {
        None
    }

    /// Report the gesture's value through `on_change` if a drag kept it.
    fn flush(&self, _app: &mut State, _own: &mut Own) -> Option<Rebuild> {
        None
    }

    /// End the gesture at `at`, as a click if the pointer stayed within the click distance.
    fn release(&self, _: &mut State, _: &mut Own, _at: [f32; 2], _click: bool) -> Option<Rebuild> {
        None
    }

    /// Restore the value at the press and end the gesture.
    fn cancel(&self, _app: &mut State, _own: &mut Own) -> Option<Rebuild> {
        None
    }

    /// The cursor during the brush's gesture.
    fn cursor(&self, _own: &Own) -> Option<CursorIcon> {
        None
    }

    /// Follow the pointer, or its absence, outside a gesture. Return whether the drawing changed.
    fn hover(&self, _app: &State, _own: &mut Own, _at: Option<[f32; 2]>) -> bool {
        false
    }

    /// Act on a left click at `at`, from a press that stayed within the click distance, with
    /// Shift held if `shift`. `trees` are the trees of the brushes' scene and of the scenes below.
    /// Return `None` when the brush ignores clicks.
    fn click(
        &self,
        _app: &mut State,
        _own: &mut Own,
        _at: [f32; 2],
        _shift: bool,
        _trees: &SceneTrees<'_>,
    ) -> Option<Clicked> {
        None
    }

    fn draw(&self, _app: &State, _own: &mut Own, _style: &BrushStyle) -> Drawing {
        Drawing::default()
    }
}

/// The state of a brush whose state type is `T`.
pub(crate) fn state<T: 'static>(own: &Own) -> &T {
    own.downcast_ref().expect("a brush's state has its type")
}

/// The mutable state of a brush whose state type is `T`.
pub(crate) fn state_mut<T: 'static>(own: &mut Own) -> &mut T {
    own.downcast_mut().expect("a brush's state has its type")
}

/// The cursor over `part` of a box on the screen: for each axis, -1 near its low edge, 1 near its
/// high edge, and 0 between them, with y increasing downward.
pub(crate) fn part_cursor(part: [i8; 2]) -> CursorIcon {
    match part {
        [0, 0] => CursorIcon::Move,
        [_, 0] => CursorIcon::EwResize,
        [0, _] => CursorIcon::NsResize,
        [x, y] if x == y => CursorIcon::NwseResize,
        _ => CursorIcon::NeswResize,
    }
}
