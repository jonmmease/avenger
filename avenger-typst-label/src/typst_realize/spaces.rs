//! Ported from crates/typst-realize/src/spaces.rs @ v0.15.1, modified for Avenger.
//!
//! The space collapsing infrastructure for realization.

use crate::typst_library::foundations::Content;
use crate::typst_library::routines::Pair;
use crate::typst_library::text::{LinebreakElem, SpaceElem};

/// State kept for space collapsing.
// avenger: no `Invisible` state, which only tags and non-weak spacing have.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum SpaceState {
    /// Destructive elements discard spaces that come before or after.
    Destructive,
    /// Normal elements. Spaces are only kept if supported on both sides.
    Supportive,
    /// Adjacent spaces collapse as one with the styles of the first space.
    Space,
}

/// Run the space collapsing algorithm on `buf[start..]`. This discards space
/// elements that are at the edges of the range or in the vicinity of
/// destructive elements and collapses adjacent spaces into one with the styles
/// of the first space.
///
/// This is implemented efficiently in-place by shifting elements in the buffer
/// to the left whenever we discard or collapse a space.
pub(crate) fn collapse_spaces(buf: &mut Vec<Pair>, start: usize) {
    let mut cursor = start;
    let mut prev_space = cursor;
    let mut state = SpaceState::Destructive;

    // We do one pass over the elements, backshifting everything as necessary
    // when a space collapses. The variable `cursor` is our cursor in the
    // result. The variable `i` is our cursor in the original elements. At all
    // times, we have `cursor <= i`, so we can do it in-place.
    for i in start..buf.len() {
        let (content, _) = buf[i];

        state = match collapse_state(content) {
            SpaceState::Destructive => {
                if state == SpaceState::Space {
                    buf.copy_within(prev_space + 1..cursor, prev_space);
                    cursor -= 1;
                }
                SpaceState::Destructive
            }
            SpaceState::Supportive => SpaceState::Supportive,
            SpaceState::Space => {
                if state != SpaceState::Supportive {
                    continue;
                }
                prev_space = cursor;
                SpaceState::Space
            }
        };

        // Copy over normal elements (in place).
        if cursor < i {
            buf[cursor] = buf[i];
        }
        cursor += 1;
    }

    if state == SpaceState::Space {
        buf.copy_within(prev_space + 1..cursor, prev_space);
        cursor -= 1;
    }

    // Delete all the excess that's left due to the gaps produced by spaces.
    buf.truncate(cursor);
}

/// Space collapsing state for general elements.
// avenger: no tags, `HElem` or HTML elements, so no styles are needed.
pub(crate) fn collapse_state(content: &Content) -> SpaceState {
    if content.is::<LinebreakElem>() {
        SpaceState::Destructive
    } else if content.is::<SpaceElem>() {
        SpaceState::Space
    } else {
        SpaceState::Supportive
    }
}
