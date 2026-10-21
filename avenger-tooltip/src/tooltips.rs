//! The packaged overlay, which finds the tooltips' targets and draws them.

use std::sync::{Arc, Mutex};

use avenger_app::app::OverlayBuilder;
use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::stream::EventStreamConfig;
use avenger_geometry::data::SceneData;
use avenger_geometry::focus::Focus;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::draw::{crosshair, crosshair_x, highlight, instance_color, Tooltip, TooltipStyle};
use crate::error::AvengerTooltipError;
use crate::hover::{lock, Hover};
use crate::target::{InstanceTarget, TooltipAnchor, TooltipTarget};

/// Tooltips: an overlay of the tooltip of the data under the pointer, and the event streams
/// that find it. Clones share the focus that the tooltips show.
///
/// A move shows the focus that the first target finds at the pointer, or hides the tooltip when
/// none finds one. A drag, the wheel, Escape, the window's losing focus, and the pointer's
/// leaving the window hide it. With pinning, a click on a focus pins it: a pinned tooltip
/// ignores moves, drags, the wheel, focus loss, and the pointer's leaving, and hides on a click
/// off every focus or on Escape. When the app rebuilds the hit-testing tree, a hovered focus is
/// found again at the pointer, and a pinned focus finds its data again, by its target's key when
/// its rows change, and hides when its data is gone.
///
/// - An [`InstanceTarget`]'s tooltip is tinted by its instance's color unless the style has no
///   tint. Beside the pointer, the instance has the style's highlight.
/// - A [`GroupTarget`](crate::GroupTarget)'s crosshair spans its series' clip, such as their
///   plot's. Beside the data, the tooltip's callout points at the crosshair at the pointer's
///   height.
#[derive(Clone)]
pub struct Tooltips {
    hover: Arc<Mutex<Hover>>,
    style: TooltipStyle,
}

impl Default for Tooltips {
    fn default() -> Self {
        Self::new()
    }
}

impl Tooltips {
    /// Tooltips of the nearest instance within 40 pixels of the pointer, among every mark with
    /// rows, beside its data, showing its row's columns.
    pub fn new() -> Self {
        Self::over([InstanceTarget::default()])
    }

    /// Tooltips that try `targets` in order: the first that finds a focus at the pointer shows
    /// it, so a target of small marks, such as points, comes before a target of the regions
    /// beneath them.
    pub fn over(targets: impl IntoIterator<Item = impl Into<TooltipTarget>>) -> Self {
        let targets = targets.into_iter().map(Into::into).collect();
        Self {
            hover: Arc::new(Mutex::new(Hover::over(targets))),
            style: TooltipStyle::default(),
        }
    }

    /// The tooltips, where a click on a focus pins it.
    pub fn with_pinning(self) -> Self {
        lock(&self.hover).pin = true;
        self
    }

    /// The tooltips, showing a new focus once the pointer rests for `delay` milliseconds.
    pub fn with_delay(self, delay: u64) -> Self {
        lock(&self.hover).delay = Some(delay);
        self
    }

    /// The tooltips, drawn in `style`: the box and its text, the tint, the crosshair, and the
    /// highlight.
    pub fn with_style(mut self, style: TooltipStyle) -> Self {
        self.style = style;
        self
    }

    /// The focus that the tooltip shows, or `None` when it hides.
    pub fn focus(&self) -> Option<Focus> {
        lock(&self.hover).focus.clone()
    }

    /// The index of the target that found the shown focus, or `None` when the tooltip hides.
    pub fn target(&self) -> Option<usize> {
        let hover = lock(&self.hover);
        hover.focus.as_ref().map(|_| hover.target)
    }

    /// Whether a click pinned the focus that the tooltip shows.
    pub fn pinned(&self) -> bool {
        lock(&self.hover).pinned
    }

    /// Where the shown tooltip stands: its target's anchor, or `None` when the tooltip hides.
    pub fn anchor(&self) -> Option<TooltipAnchor> {
        let hover = lock(&self.hover);
        hover.focus.as_ref()?;
        hover.targets.get(hover.target).map(TooltipTarget::anchor)
    }
}

impl<State: Clone + Send + Sync + 'static> OverlayBuilder<State> for Tooltips {
    fn build(
        &self,
        _state: &State,
        scene: &Arc<SceneGraph>,
        data: &SceneData,
        engine: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let error = |err: AvengerTooltipError| AvengerAppError::InternalError(err.to_string());
        let hover = lock(&self.hover);
        let Some(shown) = &hover.focus else {
            return Ok(Vec::new());
        };
        // The scene can be newer than the tree that found the focus, as during a drag, which
        // doesn't rebuild the tree, so the tooltip draws where the scene draws its data. A
        // pinned focus keeps its data when the rows change, and a hovered one shows the newer
        // build's instance and row, until the pointer finds it again.
        let target = &hover.targets[hover.target];
        let focus = if hover.pinned {
            shown.in_data(data, target.key())
        } else {
            Some(shown.clone())
        };
        let Some(mut focus) = focus.and_then(|focus| focus.in_scene(scene, data, engine)) else {
            return Ok(Vec::new());
        };
        if hover.pinned {
            follow(&mut focus, shown);
        }
        let focus = &focus;
        let Some(member) = focus.members.first() else {
            return Ok(Vec::new());
        };
        let bounds = [0.0, 0.0, scene.width, scene.height];
        let mut marks = Vec::new();
        let group = match target {
            TooltipTarget::Group(target) => {
                let tooltip = Tooltip {
                    style: self.style.clone(),
                    ..Tooltip::of_group(focus, target, scene, engine).map_err(error)?
                };
                let x = crosshair_x(focus);
                // The crosshair spans the members' clip, such as their plot's, or else the scene.
                let span = match &member.clip {
                    Some(Clip::Rect {
                        x,
                        y,
                        width,
                        height,
                    }) => [*x, *y, *width, *height],
                    _ => bounds,
                };
                marks.extend(crosshair(focus, x, span, scene, &self.style));
                match target.anchor {
                    TooltipAnchor::Data => tooltip.marks(|_| [x, focus.pointer[1]], bounds, engine),
                    TooltipAnchor::Pointer => tooltip.marks_beside(focus.pointer, bounds, engine),
                }
            }
            TooltipTarget::Instance(target) => {
                let tooltip = Tooltip {
                    style: self.style.clone(),
                    tint: self.style.tint.and_then(|_| instance_color(member, scene)),
                    ..Tooltip::of_member(member, target, engine).map_err(error)?
                };
                match target.anchor {
                    TooltipAnchor::Data => {
                        tooltip.marks(|side| member.anchor(side), bounds, engine)
                    }
                    TooltipAnchor::Pointer => {
                        if let Some(color) = self.style.highlight {
                            marks.push(highlight(member, color, self.style.highlight_width).into());
                        }
                        tooltip.marks_beside(focus.pointer, bounds, engine)
                    }
                }
            }
        }
        .map_err(error)?;
        marks.push(group.into());
        Ok(marks)
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        Hover::streams(&self.hover)
    }
}

/// Move `focus`'s pointer by as far as its first member moved since `shown`, so a pinned tooltip
/// beside the pointer stays where it stood beside its data. Members match by their marks' names,
/// since a member's index changes with its rows.
pub(crate) fn follow(focus: &mut Focus, shown: &Focus) {
    let moved = shown.members.iter().find_map(|old| {
        let new = (focus.members.iter()).find(|new| new.mark().name == old.mark().name)?;
        Some([
            new.position[0] - old.position[0],
            new.position[1] - old.position[1],
        ])
    });
    if let Some([dx, dy]) = moved {
        focus.pointer = [shown.pointer[0] + dx, shown.pointer[1] + dy];
    }
}
