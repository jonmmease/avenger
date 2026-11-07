//! How far the mark of each row reaches past its position, from the same encoded rows that the
//! marks draw, so that the fit of the position scales keeps the marks at the ends of the data
//! inside the plot.

use std::collections::HashMap;

use avenger_chart::{
    proto::{cartesian_layer, Channel},
    FitKind,
};
use avenger_common::{
    types::{ImageAlign, ImageBaseline, SymbolShape},
    value::ScalarOrArray,
};
use avenger_scenegraph::marks::text::text_origin;
use avenger_style::Property;
use lyon_path::{geom::euclid::default::Transform2D, geom::Angle, Path};

use crate::{
    encode::{at, numbers, strings, LayerRows, Result},
    marks::{image_anchor, image_sizes, path_array, path_values, shape_names, text_mark},
    style::number as style_number,
};

/// How far the mark of a row reaches past its position, in pixels: toward the left, the right, the
/// top, and the bottom.
pub(crate) type Reach = [f32; 4];

/// The bounds of `points` turned by `angle` degrees and scaled by `scale`: the reach to the left,
/// the right, the top, and the bottom of the origin.
fn turned_bounds(points: &[(f32, f32)], angle: f32, scale: f32) -> Reach {
    let turn = Transform2D::<f32>::rotation(Angle::degrees(angle));
    let mut reach = [0.0f32; 4];
    for &(x, y) in points {
        let point = turn.transform_point(lyon_path::geom::point(x, y));
        reach[0] = reach[0].max(-point.x * scale);
        reach[1] = reach[1].max(point.x * scale);
        reach[2] = reach[2].max(-point.y * scale);
        reach[3] = reach[3].max(point.y * scale);
    }
    reach
}

/// The points of `path` that bound it: its endpoints and control points.
fn path_points(path: &Path) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    for event in path.iter() {
        match event {
            lyon_path::Event::Begin { at } => points.push((at.x, at.y)),
            lyon_path::Event::Line { from, to } => {
                points.push((from.x, from.y));
                points.push((to.x, to.y));
            }
            lyon_path::Event::Quadratic { from, ctrl, to } => {
                points.extend([(from.x, from.y), (ctrl.x, ctrl.y), (to.x, to.y)]);
            }
            lyon_path::Event::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                points.extend([
                    (from.x, from.y),
                    (ctrl1.x, ctrl1.y),
                    (ctrl2.x, ctrl2.y),
                    (to.x, to.y),
                ]);
            }
            lyon_path::Event::End { .. } => {}
        }
    }
    points
}

/// The points of a unit symbol shape: a circle as the corners of the polygon of 16 sides around
/// it, which bound it at every angle.
fn shape_points(name: &str) -> Vec<(f32, f32)> {
    match SymbolShape::from_vega_str(name) {
        Ok(SymbolShape::Path(path)) => path_points(&path),
        _ => (0..16)
            .map(|step| {
                let theta = step as f32 * std::f32::consts::TAU / 16.0;
                let radius = 0.5 / (std::f32::consts::PI / 16.0).cos();
                (radius * theta.cos(), radius * theta.sin())
            })
            .collect(),
    }
}

fn max_reach(a: Reach, b: Reach) -> Reach {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// The largest value of each row among `alternatives`, or `default` without any.
fn largest(alternatives: &[ScalarOrArray<f32>], len: usize, default: f32) -> Vec<f32> {
    if alternatives.is_empty() {
        return vec![default; len];
    }
    (0..len)
        .map(|row| {
            (alternatives.iter())
                .map(|values| at(values, row))
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect()
}

/// The computed number of `property` of a layer's mark node, or 0 for a percentage `size`, a
/// fraction of a band that has no width before the fit.
fn number_or_zero(rows: &LayerRows, property: Property) -> f32 {
    rows.style.number(property).unwrap_or(0.0)
}

/// Every choice of a value for each of `channels`: the encoding's value, 0, or one of its
/// conditional values, from 1.
fn choices(rows: &LayerRows, channels: &[Channel]) -> Vec<Vec<(Channel, usize)>> {
    let mut all = vec![Vec::new()];
    for channel in channels {
        let count = (rows.encoding(*channel)).map_or(0, |encoding| encoding.conditional.len());
        all = (all.into_iter())
            .flat_map(|choice| {
                (0..=count).map(move |index| {
                    let mut choice = choice.clone();
                    choice.push((*channel, index));
                    choice
                })
            })
            .collect();
    }
    all
}

/// Each angle that each row can take: of the encoding and its conditional values, or the computed
/// `angle` without an encoding.
fn angles(rows: &LayerRows) -> Result<Vec<Vec<f32>>> {
    let alternatives = rows.alternatives(Channel::Angle, &numbers)?;
    let default = style_number(rows.style, Property::Angle);
    Ok((0..rows.len())
        .map(|row| match alternatives.is_empty() {
            true => vec![default],
            false => alternatives.iter().map(|values| at(values, row)).collect(),
        })
        .collect())
}

/// The reach of the mark of each drawn row of `rows`, over every value that its encodings and
/// their conditional values can give it, or `None` for a mark that reaches no farther than its
/// position.
pub(crate) fn reach(rows: &LayerRows) -> Result<Option<Vec<Reach>>> {
    use cartesian_layer::Mark;
    let len = rows.len();
    // Half the widest stroke of each row, of the `stroke_width` channel or the computed
    // `stroke-width`, when a choice of the conditional values of the paints and the opacities
    // shows a stroke, as a selection can.
    let stroke = |rows: &LayerRows| -> Result<Vec<f32>> {
        let paints = [
            Channel::Stroke,
            Channel::Color,
            Channel::Opacity,
            Channel::StrokeOpacity,
        ];
        let mut strokes = false;
        for choice in choices(rows, &paints) {
            if rows.choosing(&choice).strokes()? {
                strokes = true;
                break;
            }
        }
        Ok(match strokes {
            true => largest(
                &rows.alternatives(Channel::StrokeWidth, &numbers)?,
                len,
                style_number(rows.style, Property::StrokeWidth),
            )
            .into_iter()
            .map(|width| width / 2.0)
            .collect(),
            false => vec![0.0; len],
        })
    };
    match rows.mark {
        Mark::Symbol(_) => {
            let sizes = largest(
                &rows.alternatives(Channel::Size, &numbers)?,
                len,
                number_or_zero(rows, Property::Size),
            );
            let mut shape_alternatives = rows.alternatives(Channel::Shape, &strings)?;
            if shape_alternatives.is_empty() {
                shape_alternatives = vec![shape_names(rows)?];
            }
            let angles = angles(rows)?;
            let stroke = stroke(rows)?;
            let mut shapes: HashMap<String, Vec<(f32, f32)>> = HashMap::new();
            let mut reach = Vec::with_capacity(len);
            for row in 0..len {
                let mut row_reach = [0.0; 4];
                for names in &shape_alternatives {
                    let name = at(names, row);
                    let points = shapes
                        .entry(name.clone())
                        .or_insert_with(|| shape_points(&name));
                    for angle in &angles[row] {
                        let bounds = turned_bounds(points, *angle, sizes[row].sqrt());
                        row_reach = max_reach(row_reach, bounds);
                    }
                }
                reach.push(row_reach.map(|side| side + stroke[row]));
            }
            Ok(Some(reach))
        }
        Mark::Text(text) => {
            let font_sizes = largest(
                &rows.alternatives(Channel::Size, &numbers)?,
                len,
                rows.style.font_size(),
            );
            let angles = angles(rows)?;
            let mut reach = vec![[0.0f32; 4]; len];
            // The text and the width of a row can each take the encoding's value or a
            // conditional value, and the text reaches the farthest at one of their pairs.
            for choices in choices(rows, &[Channel::Text, Channel::Width]) {
                let chosen = rows.choosing(&choices);
                let zero = ScalarOrArray::new_scalar(0.0);
                let mut mark = text_mark(&chosen, text, zero.clone(), zero)?;
                // The largest font size of each row bounds its text at every selection.
                mark.font_size = ScalarOrArray::new_array(font_sizes.clone());
                for (row, label) in mark.labels().enumerate() {
                    let bounds = rows.engine.bounds(&label.label)?;
                    let [x, y] = text_origin(&bounds, [0.0, 0.0], label.align, label.baseline);
                    let corners = [
                        (x, y),
                        (x + bounds.width, y),
                        (x, y + bounds.height),
                        (x + bounds.width, y + bounds.height),
                    ];
                    reach[row] = angles[row].iter().fold(reach[row], |reach, angle| {
                        max_reach(reach, turned_bounds(&corners, *angle, 1.0))
                    });
                }
            }
            Ok(Some(reach))
        }
        Mark::Path(_) => {
            let mut alternatives = rows.alternatives(Channel::Path, &path_array)?;
            if alternatives.is_empty() {
                alternatives = vec![path_values(rows)?];
            }
            let angles = angles(rows)?;
            let stroke = stroke(rows)?;
            Ok(Some(
                (0..len)
                    .map(|row| {
                        let points: Vec<(f32, f32)> = (alternatives.iter())
                            .flat_map(|paths| path_points(&at(paths, row)))
                            .collect();
                        (angles[row].iter())
                            .fold([0.0; 4], |reach, angle| {
                                max_reach(reach, turned_bounds(&points, *angle, 1.0))
                            })
                            .map(|side| side + stroke[row])
                    })
                    .collect(),
            ))
        }
        Mark::Image(_) if !(rows.has(Channel::X2) && rows.has(Channel::Y2)) => {
            // The largest size of each row bounds its image at every selection, and an image
            // that does not draw reaches nowhere.
            let [own_widths, own_heights] = image_sizes(rows)?;
            let sized = |channel: Channel, own: &ScalarOrArray<f32>| -> Result<Vec<f32>> {
                let alternatives = rows.alternatives(channel, &numbers)?;
                Ok((0..len)
                    .map(|row| match alternatives.is_empty() {
                        true => at(own, row),
                        false => (alternatives.iter())
                            .map(|values| at(values, row))
                            .fold(f32::NEG_INFINITY, f32::max),
                    })
                    .map(|size| if size.is_finite() { size } else { 0.0 })
                    .collect())
            };
            let widths = sized(Channel::Width, &own_widths)?;
            let heights = sized(Channel::Height, &own_heights)?;
            let (align, baseline) = image_anchor(rows.style);
            Ok(Some(
                (0..len)
                    .map(|row| {
                        let (width, height) = (widths[row], heights[row]);
                        let (left, right) = match align {
                            ImageAlign::Left => (0.0, width),
                            ImageAlign::Center => (width / 2.0, width / 2.0),
                            ImageAlign::Right => (width, 0.0),
                        };
                        let (top, bottom) = match baseline {
                            ImageBaseline::Top => (0.0, height),
                            ImageBaseline::Middle => (height / 2.0, height / 2.0),
                            ImageBaseline::Bottom => (height, 0.0),
                        };
                        [left, right, top, bottom]
                    })
                    .collect(),
            ))
        }
        Mark::Bar(bar) => {
            // A bar at a continuous key without a second position is as thick as the computed
            // `continuous-size`, about its key.
            let horizontal = rows.horizontal(bar.orient);
            let (key, key2) = match horizontal {
                true => (Channel::Y, Channel::Y2),
                false => (Channel::X, Channel::X2),
            };
            let continuous_key = rows.kind(key) == Some(FitKind::Interval);
            let banded = rows.bands(key)?.is_some();
            if !rows.has(key) || rows.has(key2) || banded || !continuous_key {
                return Ok(None);
            }
            let half = style_number(rows.style, Property::ContinuousSize) / 2.0;
            let reach = match horizontal {
                true => [0.0, 0.0, half, half],
                false => [half, half, 0.0, 0.0],
            };
            Ok(Some(vec![reach; len]))
        }
        _ => Ok(None),
    }
}

/// The reach of each row along a position channel: toward the lower values of its domain and
/// toward the higher values, which a `y` that runs up the plot reverses.
pub(crate) fn along(reach: &[Reach], channel: Channel, reversed: bool) -> (Vec<f64>, Vec<f64>) {
    let (low, high) = match channel {
        Channel::Y => (3, 2),
        _ => (0, 1),
    };
    let (low, high) = if reversed { (high, low) } else { (low, high) };
    reach
        .iter()
        .map(|reach| (reach[low] as f64, reach[high] as f64))
        .unzip()
}
