//! Marks as the filled and stroked paths that vector renderers draw.

use avenger_color::{ColorOrGradient, Gradient};
use avenger_common::pattern::PatternFill;
use avenger_common::types::{StrokeCap, StrokeJoin};
use itertools::izip;
use lyon_algorithms::aabb::bounding_box;
use lyon_path::{Path, PathEvent};
use lyon_tessellation::TessellationError;

use crate::{
    marks::{
        arc::SceneArcMark, area::SceneAreaMark, line::SceneLineMark, mark::SceneMark,
        path::ScenePathMark, rect::SceneRectMark, rule::SceneRuleMark, symbol::SceneSymbolMark,
        trail::SceneTrailMark,
    },
    render::{
        paint::GradientBounds,
        pattern::{PatternRect, PatternRenderContext},
        stroke::{needs_explicit_caps, stroke_outline},
    },
};

/// A mark item's path, with the fill and stroke that draw it.
#[derive(Debug, Clone)]
pub struct PathItem<'a> {
    /// The path, in scene coordinates.
    pub path: Path,
    pub fill: Option<FillItem<'a>>,
    pub stroke: Option<StrokeItem<'a>>,
    /// The box that gradients and mark-anchored patterns are placed in.
    pub bounds: GradientBounds,
    pub gradients: &'a [Gradient],
}

#[derive(Debug, Clone, Copy)]
pub struct FillItem<'a> {
    pub paint: &'a ColorOrGradient,
    /// The pattern drawn over the fill.
    pub pattern: Option<&'a PatternFill>,
}

#[derive(Debug, Clone, Copy)]
pub struct StrokeItem<'a> {
    pub paint: &'a ColorOrGradient,
    /// The stroke width, which is positive and finite.
    pub width: f32,
    pub cap: StrokeCap,
    pub join: StrokeJoin,
    pub dash: Option<&'a [f32]>,
}

impl<'a> PathItem<'a> {
    /// The stroke as a filled outline when native strokes would draw some of its caps differently
    /// (see `needs_explicit_caps`), or `None` when a native stroke draws it.
    pub fn stroke_outline(&self) -> Result<Option<Path>, TessellationError> {
        let Some(stroke) = &self.stroke else {
            return Ok(None);
        };
        if !needs_explicit_caps(&self.path, stroke.dash, stroke.cap) {
            return Ok(None);
        }
        stroke_outline(
            &self.path,
            stroke.dash,
            stroke.width,
            stroke.cap,
            stroke.join,
        )
        .map(Some)
    }

    /// The fill's pattern and the context to build its geometry in, given where group-anchored
    /// patterns start.
    pub fn pattern(
        &self,
        group_origin: [f32; 2],
    ) -> Option<(&'a PatternFill, PatternRenderContext<'a>)> {
        let fill = self.fill?;
        let bbox = bounding_box(&self.path);
        Some((
            fill.pattern?,
            PatternRenderContext {
                group_origin,
                mark_origin: self.bounds.min,
                host_bounds: PatternRect::new(bbox.min.x, bbox.min.y, bbox.width(), bbox.height()),
                host_fill: fill.paint,
                gradients: self.gradients,
            },
        ))
    }
}

/// The paths that draw a mark. Group, text, and image marks draw none, and neither does an item
/// with a non-finite coordinate.
pub fn path_items(
    mark: &SceneMark,
    origin: [f32; 2],
) -> Result<Vec<PathItem<'_>>, TessellationError> {
    let mut items = match mark {
        SceneMark::Rect(mark) => rect_items(mark, origin),
        SceneMark::Path(mark) => path_mark_items(mark, origin),
        SceneMark::Symbol(mark) => symbol_items(mark, origin),
        SceneMark::Arc(mark) => arc_items(mark, origin),
        SceneMark::Area(mark) => vec![area_item(mark, origin)],
        SceneMark::Line(mark) => vec![line_item(mark, origin)],
        SceneMark::Rule(mark) => rule_items(mark, origin),
        SceneMark::Trail(mark) => trail_items(mark, origin)?,
        SceneMark::Group(_)
        | SceneMark::Text(_)
        | SceneMark::Image(_)
        | SceneMark::MeshImage(_) => Vec::new(),
    };
    items.retain(|item| path_is_finite(&item.path));
    Ok(items)
}

/// The paths that draw a path mark, such as a group's background, without items that have a
/// non-finite coordinate.
pub fn path_mark_items(mark: &ScenePathMark, origin: [f32; 2]) -> Vec<PathItem<'_>> {
    izip!(
        mark.transformed_path_iter(origin),
        mark.fill_iter(),
        mark.fill_pattern_iter(),
        mark.stroke_iter()
    )
    .map(|(path, fill, pattern, stroke)| PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: Some(FillItem {
            paint: fill,
            pattern: pattern.as_ref(),
        }),
        stroke: stroke_item(
            stroke,
            mark.stroke_width.unwrap_or(0.0),
            mark.stroke_cap,
            mark.stroke_join,
            None,
        ),
        gradients: &mark.gradients,
    })
    .filter(|item| path_is_finite(&item.path))
    .collect()
}

fn rect_items(mark: &SceneRectMark, origin: [f32; 2]) -> Vec<PathItem<'_>> {
    izip!(
        mark.transformed_path_iter(origin),
        mark.fill_iter(),
        mark.fill_pattern_iter(),
        mark.stroke_iter(),
        mark.stroke_width_iter()
    )
    .map(|(path, fill, pattern, stroke, width)| PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: Some(FillItem {
            paint: fill,
            pattern: pattern.as_ref(),
        }),
        stroke: stroke_item(stroke, *width, StrokeCap::Butt, StrokeJoin::Miter, None),
        gradients: &mark.gradients,
    })
    .collect()
}

fn symbol_items(mark: &SceneSymbolMark, origin: [f32; 2]) -> Vec<PathItem<'_>> {
    izip!(
        mark.transformed_path_iter(origin),
        mark.fill_iter(),
        mark.fill_pattern_iter(),
        mark.stroke_iter(),
        mark.x_iter(),
        mark.y_iter(),
        mark.size_iter()
    )
    .map(|(path, fill, pattern, stroke, x, y, size)| PathItem {
        path,
        fill: Some(FillItem {
            paint: fill,
            pattern: pattern.as_ref(),
        }),
        stroke: stroke_item(
            stroke,
            mark.stroke_width.unwrap_or(0.0),
            StrokeCap::Butt,
            StrokeJoin::Miter,
            None,
        ),
        // Symbols place their paint on their nominal square, whatever their shape and rotation.
        bounds: GradientBounds::symbol([x + origin[0], y + origin[1]], *size),
        gradients: &mark.gradients,
    })
    .collect()
}

fn arc_items(mark: &SceneArcMark, origin: [f32; 2]) -> Vec<PathItem<'_>> {
    izip!(
        mark.transformed_path_iter(origin),
        mark.fill_iter(),
        mark.fill_pattern_iter(),
        mark.stroke_iter(),
        mark.stroke_width_iter()
    )
    .map(|(path, fill, pattern, stroke, width)| PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: Some(FillItem {
            paint: fill,
            pattern: pattern.as_ref(),
        }),
        stroke: stroke_item(stroke, *width, StrokeCap::Butt, StrokeJoin::Miter, None),
        gradients: &mark.gradients,
    })
    .collect()
}

fn area_item(mark: &SceneAreaMark, origin: [f32; 2]) -> PathItem<'_> {
    let path = mark.transformed_path(origin);
    PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: Some(FillItem {
            paint: &mark.fill,
            pattern: mark.fill_pattern.as_ref(),
        }),
        stroke: stroke_item(
            &mark.stroke,
            mark.stroke_width,
            mark.stroke_cap,
            mark.stroke_join,
            mark.stroke_dash.as_deref(),
        ),
        gradients: &mark.gradients,
    }
}

fn line_item(mark: &SceneLineMark, origin: [f32; 2]) -> PathItem<'_> {
    let path = mark.undashed_path(origin);
    PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: None,
        stroke: stroke_item(
            &mark.stroke,
            mark.stroke_width,
            mark.stroke_cap,
            mark.stroke_join,
            mark.stroke_dash.as_deref(),
        ),
        gradients: &mark.gradients,
    }
}

fn rule_items(mark: &SceneRuleMark, origin: [f32; 2]) -> Vec<PathItem<'_>> {
    let dashes: Box<dyn Iterator<Item = Option<&[f32]>>> = match mark.stroke_dash_iter() {
        Some(dashes) => Box::new(dashes.map(|dash| Some(dash.as_slice()))),
        None => Box::new(std::iter::repeat(None)),
    };
    izip!(
        mark.undashed_path_iter(origin),
        mark.stroke_iter(),
        mark.stroke_width_iter(),
        mark.stroke_cap_iter(),
        dashes
    )
    .map(|(path, stroke, width, cap, dash)| PathItem {
        bounds: GradientBounds::from_path(&path),
        path,
        fill: None,
        stroke: stroke_item(stroke, *width, *cap, StrokeJoin::Miter, dash),
        gradients: &mark.gradients,
    })
    .collect()
}

fn trail_items(
    mark: &SceneTrailMark,
    origin: [f32; 2],
) -> Result<Vec<PathItem<'_>>, TessellationError> {
    let centerline = mark.transformed_path(origin);
    if !path_is_finite(&centerline) {
        return Ok(Vec::new());
    }
    // A trail is a variable-width stroke, drawn as its filled outline in the stroke's paint.
    Ok(vec![PathItem {
        bounds: GradientBounds::from_path(&centerline),
        path: mark.outline(origin)?,
        fill: Some(FillItem {
            paint: &mark.stroke,
            pattern: None,
        }),
        stroke: None,
        gradients: &mark.gradients,
    }])
}

fn stroke_item<'a>(
    paint: &'a ColorOrGradient,
    width: f32,
    cap: StrokeCap,
    join: StrokeJoin,
    dash: Option<&'a [f32]>,
) -> Option<StrokeItem<'a>> {
    (width > 0.0 && width.is_finite()).then_some(StrokeItem {
        paint,
        width,
        cap,
        join,
        dash,
    })
}

/// Whether every point of `path` is finite. Renderers draw nothing for an item whose path has a
/// non-finite point.
pub fn path_is_finite(path: &Path) -> bool {
    let finite = |p: lyon_path::math::Point| p.x.is_finite() && p.y.is_finite();
    path.iter().all(|event| match event {
        PathEvent::Begin { at } => finite(at),
        PathEvent::Line { to, .. } => finite(to),
        PathEvent::Quadratic { ctrl, to, .. } => finite(ctrl) && finite(to),
        PathEvent::Cubic {
            ctrl1, ctrl2, to, ..
        } => finite(ctrl1) && finite(ctrl2) && finite(to),
        PathEvent::End { .. } => true,
    })
}
