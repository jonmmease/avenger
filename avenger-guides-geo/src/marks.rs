//! Scene marks for screen-space geometry: path marks for polygons and lines,
//! and a symbol mark for points.

use std::sync::Arc;

use crate::error::AvengerGuidesGeoError;
use avenger_color::ColorOrGradient;
use avenger_common::types::{StrokeCap, StrokeJoin, SymbolShape};
use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_geo::paths::row_parts;
use avenger_scenegraph::marks::{
    group::SceneGroup, mark::SceneMark, path::ScenePathMark, symbol::SceneSymbolMark,
};
use geoarrow_array::GeoArrowArray;

/// How [`geometry_marks`] draws a screen-space array. `fill`, `stroke`, and
/// `point_size` are a value for every row, or one value for all of them.
#[derive(Debug, Clone)]
pub struct GeometryStyle {
    /// The fill of polygons and points. Lines aren't filled.
    pub fill: ScalarOrArray<ColorOrGradient>,
    pub stroke: ScalarOrArray<ColorOrGradient>,
    /// The stroke width of every row: the marks take one width, so a layer
    /// that needs two is drawn with two calls.
    pub stroke_width: f32,
    /// The area of each point's symbol, in square pixels.
    pub point_size: ScalarOrArray<f32>,
    pub point_shape: SymbolShape,
}

impl Default for GeometryStyle {
    fn default() -> Self {
        Self {
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.27, 0.51, 0.71, 1.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.2, 0.2, 0.2, 1.0])),
            stroke_width: 1.0,
            point_size: ScalarOrArray::new_scalar(16.0),
            point_shape: SymbolShape::Circle,
        }
    }
}

/// A group with a path mark of the rows' polygons, a path mark of their
/// lines, and a symbol mark of their points, drawn in that order, from a
/// screen-space GeoArrow array such as
/// [`avenger_geo::Projector::project_geometry`] returns. The geometry's
/// coordinates are drawn as they are, so the array can also hold data that is
/// already in screen coordinates.
///
/// A path mark's instances are the array's rows, and its `indices` select
/// the rows with polygons, or with lines, so a pick reports the original row.
/// The line mark isn't filled, since a fill would close its lines. When
/// every row has at most one point, the symbol mark's instances are rows too.
/// A row with several points gives a symbol for each, and the symbol mark's
/// instances are then the points, in row order. Each mark draws its rows in
/// the array's order, and a mark without rows is left out.
///
/// # Errors
/// Returns an error for a row that can't be read, and for a style array that doesn't have a
/// value for each row.
pub fn geometry_marks(
    geometry: &dyn GeoArrowArray,
    style: &GeometryStyle,
) -> Result<SceneGroup, AvengerGuidesGeoError> {
    let parts = row_parts(geometry)?;
    let len = parts.polygons.len();
    for (name, values) in [
        ("fill", rows_of(&style.fill)),
        ("stroke", rows_of(&style.stroke)),
        ("point_size", rows_of(&style.point_size)),
    ] {
        if values.is_some_and(|values| values != len) {
            return Err(AvengerGuidesGeoError::InvalidConfig(format!(
                "a style's {name} has {} values for {len} rows",
                values.unwrap_or_default()
            )));
        }
    }
    let mut marks: Vec<SceneMark> = Vec::new();

    let transparent = ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0; 4]));
    for (paths, fill) in [
        (parts.polygons, style.fill.clone()),
        (parts.lines, transparent),
    ] {
        let rows: Vec<usize> = (0..len)
            .filter(|&row| paths[row].iter().next().is_some())
            .collect();
        if rows.is_empty() {
            continue;
        }
        marks.push(
            ScenePathMark {
                len: len as u32,
                path: ScalarOrArray::new_array(paths),
                fill,
                stroke: style.stroke.clone(),
                stroke_width: Some(style.stroke_width),
                stroke_cap: StrokeCap::Round,
                stroke_join: StrokeJoin::Round,
                indices: Some(Arc::new(rows)),
                ..Default::default()
            }
            .into(),
        );
    }

    if !parts.points.is_empty() {
        let one_per_row = parts.point_rows.windows(2).all(|pair| pair[0] < pair[1]);
        let symbols = if one_per_row {
            // Rows without a point have none to draw; `indices` leaves them out.
            let mut x = vec![f32::NAN; len];
            let mut y = vec![f32::NAN; len];
            for (&[px, py], &row) in parts.points.iter().zip(&parts.point_rows) {
                x[row] = px;
                y[row] = py;
            }
            SceneSymbolMark {
                len: len as u32,
                x: ScalarOrArray::new_array(x),
                y: ScalarOrArray::new_array(y),
                fill: style.fill.clone(),
                stroke: style.stroke.clone(),
                size: style.point_size.clone(),
                indices: Some(Arc::new(parts.point_rows.clone())),
                ..symbol_style(style)
            }
        } else {
            let rows = &parts.point_rows;
            SceneSymbolMark {
                len: parts.points.len() as u32,
                x: ScalarOrArray::new_array(parts.points.iter().map(|point| point[0]).collect()),
                y: ScalarOrArray::new_array(parts.points.iter().map(|point| point[1]).collect()),
                fill: by_point(&style.fill, len, rows),
                stroke: by_point(&style.stroke, len, rows),
                size: by_point(&style.point_size, len, rows),
                ..symbol_style(style)
            }
        };
        marks.push(symbols.into());
    }

    Ok(SceneGroup {
        marks,
        ..Default::default()
    })
}

/// The number of values in a style array, or `None` for one value for every row.
fn rows_of<T: Clone + Sync>(values: &ScalarOrArray<T>) -> Option<usize> {
    match values.value() {
        ScalarOrArrayValue::Scalar(_) => None,
        ScalarOrArrayValue::Array(values) => Some(values.len()),
    }
}

/// Row-aligned `values` as a value for each point, from the row of each point.
fn by_point<T: Clone + Sync>(
    values: &ScalarOrArray<T>,
    len: usize,
    rows: &[usize],
) -> ScalarOrArray<T> {
    let values = values.as_vec(len, None);
    ScalarOrArray::new_array(rows.iter().map(|&row| values[row].clone()).collect())
}

fn symbol_style(style: &GeometryStyle) -> SceneSymbolMark {
    SceneSymbolMark {
        shapes: vec![style.point_shape.clone()],
        stroke_width: Some(style.stroke_width),
        ..Default::default()
    }
}
