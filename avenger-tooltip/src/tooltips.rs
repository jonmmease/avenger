//! The packaged overlay: the tooltips' targets and the overlay that finds and draws them.

use std::sync::{Arc, Mutex};

use avenger_app::app::OverlayBuilder;
use avenger_app::error::AvengerAppError;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::stream::EventStreamConfig;
use avenger_geometry::focus::{Focus, FocusStrategy};
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;

use crate::draw::{
    crosshair, crosshair_x, highlight, instance_color, Tooltip, TooltipContent, TooltipStyle,
};
use crate::error::AvengerTooltipError;
use crate::hover::{lock, Hover};

/// Where a tooltip stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TooltipAnchor {
    /// Beside the focus's data, with a callout to it, so it stays still while the pointer moves
    /// within the focus.
    #[default]
    Data,
    /// Below and right of the pointer, as Vega's tooltips stand, so the pointer stays in view.
    /// It follows the pointer within the focus, over an outline of the data that no callout
    /// points at, for marks without one point to point at, such as areas and map regions.
    Pointer,
}

/// What tooltips look for: the marks that they search, how they find a focus among them, where
/// the focus's tooltip stands, and what it shows of the focus's rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TooltipTarget {
    pub strategy: FocusStrategy,
    /// The names of the marks to search, or `None` for every mark with rows.
    pub marks: Option<Vec<String>>,
    pub anchor: TooltipAnchor,
    pub content: TooltipContent,
}

impl TooltipTarget {
    /// A target that finds its focus with `strategy` among every mark with rows.
    pub fn new(strategy: FocusStrategy) -> Self {
        Self {
            strategy,
            ..Default::default()
        }
    }

    /// The target, searching only the marks named `marks`.
    pub fn with_marks(mut self, marks: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.marks = Some(marks.into_iter().map(Into::into).collect());
        self
    }

    pub fn with_anchor(mut self, anchor: TooltipAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    pub fn with_content(mut self, content: TooltipContent) -> Self {
        self.content = content;
        self
    }
}

/// Tooltips: an overlay of the tooltip of the data under the pointer, and the event streams
/// that find it. Clones share the focus that the tooltips show.
///
/// A move shows the focus that the first target finds at the pointer, or hides the tooltip when
/// none finds one. A drag, the wheel, Escape, the window's losing focus, and the pointer's
/// leaving the window hide it. With pinning, a click on a focus pins it: a pinned tooltip
/// ignores moves, drags, the wheel, focus loss, and the pointer's leaving, and hides on a click
/// off every focus or on Escape. When the app rebuilds the hit-testing tree, a pinned focus finds
/// its instances again, and hides when one of them is gone, and a hovered focus is found again at
/// the pointer.
///
/// - One member's tooltip has its target's header and fields, tinted by the member's instance
///   color.
/// - A [`FocusStrategy::GroupX`] focus's tooltip has its target's header from the first
///   member's row, and a row for each member, labeled by its mark's name, with its swatch. A
///   crosshair stands at the focus's x across the members' clip, with a marker on each member.
///   Beside the data, the tooltip's callout points at the crosshair at the pointer's height.
#[derive(Clone)]
pub struct Tooltips {
    hover: Arc<Mutex<Hover>>,
    style: TooltipStyle,
    tint: bool,
    outline: Option<[f32; 4]>,
    outline_width: f32,
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
        Self::over(vec![TooltipTarget::default()])
    }

    /// Tooltips that try `targets` in order: the first that finds a focus at the pointer shows
    /// it, so a target of small marks, such as points, comes before a target of the regions
    /// beneath them.
    pub fn over(targets: Vec<TooltipTarget>) -> Self {
        Self {
            hover: Arc::new(Mutex::new(Hover::over(targets))),
            style: TooltipStyle::default(),
            tint: true,
            outline: Some([0.1, 0.1, 0.12, 0.9]),
            outline_width: 1.5,
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

    pub fn with_style(mut self, style: TooltipStyle) -> Self {
        self.style = style;
        self
    }

    /// The tooltips, where one member's tooltip takes the member's instance color as its tint
    /// when `tint` is set, as it does by default.
    pub fn with_tint(mut self, tint: bool) -> Self {
        self.tint = tint;
        self
    }

    /// The tooltips, outlining the member of a tooltip beside the pointer in `color`, or with
    /// no outline for `None`.
    pub fn with_outline(mut self, color: Option<[f32; 4]>) -> Self {
        self.outline = color;
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
}

impl<State: Clone + Send + Sync + 'static> OverlayBuilder<State> for Tooltips {
    fn build(
        &self,
        _state: &State,
        scene: &SceneGraph,
        engine: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let error = |err: AvengerTooltipError| AvengerAppError::InternalError(err.to_string());
        let hover = lock(&self.hover);
        let Some(shown) = &hover.focus else {
            return Ok(Vec::new());
        };
        // The scene can be newer than the tree that found the focus, as during a drag, which
        // doesn't rebuild the tree, so the tooltip draws where the scene draws its data.
        let Some(mut focus) = shown.in_scene(scene, engine) else {
            return Ok(Vec::new());
        };
        if hover.pinned {
            follow(&mut focus, shown);
        }
        let focus = &focus;
        let target = &hover.targets[hover.target];
        let Some(member) = focus.members.first() else {
            return Ok(Vec::new());
        };
        let bounds = [0.0, 0.0, scene.width, scene.height];
        let mut marks = Vec::new();
        let group = if let FocusStrategy::GroupX { .. } = target.strategy {
            let tooltip = Tooltip {
                style: self.style.clone(),
                ..Tooltip::of_group(focus, &target.content, scene, engine).map_err(error)?
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
        } else {
            let tooltip = Tooltip {
                style: self.style.clone(),
                tint: self.tint.then(|| instance_color(member, scene)).flatten(),
                ..Tooltip::of_member(member, &target.content, engine).map_err(error)?
            };
            match target.anchor {
                TooltipAnchor::Data => tooltip.marks(|side| member.anchor(side), bounds, engine),
                TooltipAnchor::Pointer => {
                    if let Some(color) = self.outline {
                        marks.push(highlight(member, color, self.outline_width).into());
                    }
                    tooltip.marks_beside(focus.pointer, bounds, engine)
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
/// beside the pointer stays where it stood beside its data.
pub(crate) fn follow(focus: &mut Focus, shown: &Focus) {
    let moved = shown.members.iter().find_map(|old| {
        let new = focus.members.iter().find(|new| new.mark() == old.mark())?;
        Some([
            new.position[0] - old.position[0],
            new.position[1] - old.position[1],
        ])
    });
    if let Some([dx, dy]) = moved {
        focus.pointer = [shown.pointer[0] + dx, shown.pointer[1] + dy];
    }
}
