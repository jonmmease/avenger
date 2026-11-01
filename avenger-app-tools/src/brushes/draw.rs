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

/// A path through the points of each line of `lines`, closed where its flag is set.
pub(crate) fn outline<'a>(lines: impl IntoIterator<Item = (&'a [[f32; 2]], bool)>) -> Path {
    let mut builder = Path::builder();
    for (line, close) in lines {
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

/// The distance from `at` to the nearest segment of `lines`.
pub(crate) fn distance(at: [f32; 2], lines: &[Vec<[f32; 2]>]) -> f32 {
    let segment = |a: [f32; 2], b: [f32; 2]| {
        let [dx, dy] = [b[0] - a[0], b[1] - a[1]];
        let length = dx * dx + dy * dy;
        let along = if length > 0.0 {
            (((at[0] - a[0]) * dx + (at[1] - a[1]) * dy) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (at[0] - a[0] - along * dx).hypot(at[1] - a[1] - along * dy)
    };
    (lines.iter().flat_map(|line| line.windows(2)))
        .map(|pair| segment(pair[0], pair[1]))
        .fold(f32::INFINITY, f32::min)
}

/// The point halfway along `line`, by length.
pub(crate) fn middle(line: &[[f32; 2]]) -> Option<[f32; 2]> {
    let length = |a: [f32; 2], b: [f32; 2]| (b[0] - a[0]).hypot(b[1] - a[1]);
    let total: f32 = line.windows(2).map(|pair| length(pair[0], pair[1])).sum();
    let mut left = total / 2.0;
    for pair in line.windows(2) {
        let step = length(pair[0], pair[1]);
        if step >= left && step > 0.0 {
            let along = left / step;
            return Some(
                [0, 1].map(|axis| pair[0][axis] + along * (pair[1][axis] - pair[0][axis])),
            );
        }
        left -= step;
    }
    line.first().copied()
}

/// The lines of `pieces`, each joined to the next where it ends at the next's start, and whether
/// each closes, ending where it starts. Ends within a hundredth of a pixel meet.
pub(crate) fn joined(
    pieces: impl IntoIterator<Item = Vec<[f32; 2]>>,
) -> Vec<(Vec<[f32; 2]>, bool)> {
    let meet = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01;
    let mut lines: Vec<Vec<[f32; 2]>> = Vec::new();
    for piece in pieces {
        match lines.last_mut() {
            Some(line)
                if line
                    .last()
                    .zip(piece.first())
                    .is_some_and(|(a, b)| meet(*a, *b)) =>
            {
                line.extend(piece.into_iter().skip(1));
            }
            _ => lines.push(piece),
        }
    }
    // The last line continues into the first where the outline runs around past its start.
    let start = lines.first().and_then(|line| line.first()).copied();
    let end = lines.last().and_then(|line| line.last()).copied();
    let around = matches!((start, end), (Some(start), Some(end)) if meet(end, start));
    if lines.len() > 1 && around {
        let last = lines.pop().unwrap_or_default();
        let first = std::mem::take(&mut lines[0]);
        lines[0] = last.into_iter().chain(first.into_iter().skip(1)).collect();
    }
    (lines.into_iter())
        .map(|mut line| {
            let closes = line.len() > 2 && meet(line[0], line[line.len() - 1]);
            if closes {
                line.pop();
            }
            (line, closes)
        })
        .collect()
}
