//! Brush chrome: boxes, their handles, and lassos, clipped to an area and noninteractive, and
//! the cache of each area's chrome.

use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_layout::Rect;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::render::stroke::dash_path;
use lyon_path::geom::point;
use lyon_path::Path;

use super::brush::Drawing;
use super::style::BrushStyle;

/// Each area's view and marks from a brush's last drawing. A brush draws an area again only when
/// the area's view changes.
pub(crate) struct Drawn<A, V>(Vec<(A, V, Vec<SceneMark>)>);

impl<A, V> Default for Drawn<A, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<A: PartialEq, V: PartialEq> Drawn<A, V> {
    /// Add the marks of each area of `views` to `drawing`: the last marks of an area whose view
    /// is unchanged, and otherwise the marks that `draw` makes for its view.
    pub(crate) fn update(
        &mut self,
        views: Vec<(A, V)>,
        drawing: &mut Drawing,
        mut draw: impl FnMut(&V) -> Vec<SceneMark>,
    ) {
        drawing.changed |= views.len() != self.0.len()
            || (views.iter().zip(&self.0)).any(|((key, _), (drawn, _, _))| key != drawn);
        let mut drawn = Vec::with_capacity(views.len());
        for (key, view) in views {
            let kept = (self.0.iter_mut())
                .find(|(old, old_view, _)| *old == key && *old_view == view)
                .map(|(_, _, marks)| std::mem::take(marks));
            let marks = kept.unwrap_or_else(|| {
                drawing.changed = true;
                draw(&view)
            });
            drawing.marks.extend(marks.iter().cloned());
            drawn.push((key, view, marks));
        }
        self.0 = drawn;
    }
}

/// `at`, in scene coordinates, from the top-left corner of `area`.
pub(crate) fn local(at: [f32; 2], area: Rect) -> [f32; 2] {
    [at[0] - area.x, at[1] - area.y]
}

/// `at`, in scene coordinates, moved inside `area` and counted from its top-left corner.
pub(crate) fn local_clamped(at: [f32; 2], area: Rect) -> [f32; 2] {
    [
        (at[0] - area.x).clamp(0.0, area.width),
        (at[1] - area.y).clamp(0.0, area.height),
    ]
}

/// A box's pixels along each axis, from low to high: `[[left, right], [top, bottom]]`.
#[cfg(feature = "scales")]
pub(crate) type Pixels = [[f32; 2]; 2];

/// A noninteractive group of `marks` named `name`, in the coordinates of `area` from its top-left
/// corner, clipped to `area`.
pub(crate) fn clipped(name: &str, area: Rect, marks: Vec<SceneMark>) -> SceneMark {
    SceneGroup {
        name: name.to_string(),
        interactive: Some(false),
        origin: [area.x, area.y],
        clip: Clip::Rect {
            x: 0.0,
            y: 0.0,
            width: area.width,
            height: area.height,
        },
        marks,
        ..Default::default()
    }
    .into()
}

/// The chrome of the box at `pixels`: its fill and outline, and a handle centered on each point
/// of `handles`.
#[cfg(feature = "scales")]
pub(crate) fn box_marks(
    pixels: Pixels,
    handles: &[[f32; 2]],
    style: &BrushStyle,
) -> Vec<SceneMark> {
    let [[left, right], [top, bottom]] = pixels;
    let corners = [[left, top], [right, top], [right, bottom], [left, bottom]];
    let path = outline([corners.as_slice()], true);
    let mut marks = vec![
        rects("brush_box", &[[left, top, right, bottom]], style.fill, None),
        stroked("brush_outline", &path, style),
    ];
    marks.extend(handle_marks(handles, style));
    marks
}

/// A square handle centered on each point of `handles`.
pub(crate) fn handle_marks(handles: &[[f32; 2]], style: &BrushStyle) -> Option<SceneMark> {
    if handles.is_empty() {
        return None;
    }
    let half = style.handle_size / 2.0;
    let squares: Vec<[f32; 4]> = (handles.iter())
        .map(|[x, y]| [x - half, y - half, x + half, y + half])
        .collect();
    let stroke = Some((style.stroke, style.stroke_width));
    Some(rects("brush_handles", &squares, style.handle_fill, stroke))
}

/// One rect mark with an instance for each `[left, top, right, bottom]` of `boxes`.
fn rects(
    name: &str,
    boxes: &[[f32; 4]],
    fill: [f32; 4],
    stroke: Option<([f32; 4], f32)>,
) -> SceneMark {
    let column = |f: fn(&[f32; 4]) -> f32| ScalarOrArray::new_array(boxes.iter().map(f).collect());
    let (stroke, stroke_width) = stroke.unwrap_or(([0.0; 4], 0.0));
    SceneRectMark {
        name: name.to_string(),
        len: boxes.len() as u32,
        x: column(|b| b[0]),
        y: column(|b| b[1]),
        width: Some(column(|b| b[2] - b[0])),
        height: Some(column(|b| b[3] - b[1])),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(stroke).into(),
        stroke_width: stroke_width.into(),
        ..Default::default()
    }
    .into()
}

/// A path through the points of each of `lines`, closed if `close`.
pub(crate) fn outline<'a>(lines: impl IntoIterator<Item = &'a [[f32; 2]]>, close: bool) -> Path {
    let mut builder = Path::builder();
    for line in lines {
        let Some(([x, y], rest)) = line.split_first() else {
            continue;
        };
        builder.begin(point(*x, *y));
        for [x, y] in rest {
            builder.line_to(point(*x, *y));
        }
        builder.end(close);
    }
    builder.build()
}

/// A path mark that outlines `path` with the style's stroke and its dash pattern, and fills nothing.
pub(crate) fn stroked(name: &str, path: &Path, style: &BrushStyle) -> SceneMark {
    let path = match &style.stroke_dash {
        Some(dash) => dash_path(path, dash),
        None => path.clone(),
    };
    path_mark(
        name,
        path,
        [0.0; 4],
        Some((style.stroke, style.stroke_width)),
    )
}

/// A path mark that fills `path` with `fill`, and outlines it with `stroke` and its width.
pub(crate) fn path_mark(
    name: &str,
    path: Path,
    fill: [f32; 4],
    stroke: Option<([f32; 4], f32)>,
) -> SceneMark {
    let (stroke, stroke_width) = stroke.unwrap_or(([0.0; 4], 0.0));
    ScenePathMark {
        name: name.to_string(),
        path: path.into(),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(stroke).into(),
        stroke_width: Some(stroke_width),
        ..Default::default()
    }
    .into()
}
