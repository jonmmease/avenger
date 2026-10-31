//! The fit of position scales at each size: the reach of each mark past its position, the
//! summaries of data domains, and the fitting scale of each position group.

use std::{collections::HashMap, sync::Arc};

use arrow::{array::Float64Array, datatypes::DataType};
use avenger_common::types::{ImageAlign, ImageBaseline, SymbolShape};
use avenger_scales::scalar::Scalar;
use avenger_scales::scales::{
    domain_summary::{CategoryOptions, CategoryOrder, DomainOptions},
    fitting::FittingScale,
};
use avenger_scenegraph::marks::text::text_origin;
use lyon_path::{geom::euclid::default::Transform2D, geom::Angle, Path};

use avenger_chart::{
    proto::{self, mark, Channel},
    resolve::ScaleGroup,
    Chart, FitKind,
};

use crate::{
    encode::{
        self, at, image_anchor, image_size, shape_names, symbol_filled, symbol_stroke_width,
        UnitRows,
    },
    scale::{self, eval_error, DomainSource},
    select::Selections,
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

/// How far the mark of a row reaches past its position, in pixels: toward the left, the right,
/// the top, and the bottom.
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

/// The points of a unit symbol shape: a circle as its bounding square's corners at each angle,
/// which bound it the same way.
fn shape_points(name: &str) -> Vec<(f32, f32)> {
    match SymbolShape::from_vega_str(name) {
        Ok(SymbolShape::Path(path)) => path_points(&path),
        _ => (0..16)
            .map(|step| {
                let theta = step as f32 * std::f32::consts::TAU / 16.0;
                (
                    0.5 * theta.cos() / (std::f32::consts::PI / 16.0).cos(),
                    0.5 * theta.sin() / (std::f32::consts::PI / 16.0).cos(),
                )
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
fn largest(
    alternatives: &[avenger_common::value::ScalarOrArray<f32>],
    len: usize,
    default: f32,
) -> Vec<f32> {
    if alternatives.is_empty() {
        return vec![default; len];
    }
    (0..len)
        .map(|row| {
            alternatives
                .iter()
                .map(|values| at(values, row))
                .fold(f32::NEG_INFINITY, f32::max)
        })
        .collect()
}

/// Each angle of each row among `alternatives`, or zero without any.
fn angles(rows: &UnitRows) -> Result<Vec<Vec<f32>>> {
    let alternatives = rows.alternatives(Channel::Angle, &|values, scale| match scale {
        Some(scale) => scale
            .scale_to_numeric(values)
            .map_err(|error| eval_error("an angle", error)),
        None => avenger_scales::scales::coerce::Coercer::default()
            .to_numeric(values, None)
            .map_err(|error| eval_error("an angle", error)),
    })?;
    let len = rows.len();
    Ok((0..len)
        .map(|row| {
            if alternatives.is_empty() {
                vec![0.0]
            } else {
                alternatives.iter().map(|values| at(values, row)).collect()
            }
        })
        .collect())
}

fn sizes(rows: &UnitRows) -> Result<Vec<avenger_common::value::ScalarOrArray<f32>>> {
    rows.alternatives(Channel::Size, &|values, scale| match scale {
        Some(scale) => scale
            .scale_to_numeric(values)
            .map_err(|error| eval_error("a size", error)),
        None => avenger_scales::scales::coerce::Coercer::default()
            .to_numeric(values, None)
            .map_err(|error| eval_error("a size", error)),
    })
}

/// The reach of the mark of each drawn row of `rows`, every value that its encodings and their
/// conditions can give it, or `None` for a mark that reaches no farther than its position.
pub(crate) fn reach(rows: &UnitRows) -> Result<Option<Vec<Reach>>> {
    let len = rows.len();
    match rows.mark {
        mark::Mark::Symbol(symbol) => {
            let sizes = largest(&sizes(rows)?, len, rows.theme.symbol_size);
            let shape_alternatives =
                rows.alternatives(Channel::Shape, &|values, scale| match scale {
                    Some(scale) => scale
                        .scale_to_string(values)
                        .map_err(|error| eval_error("a shape", error)),
                    None => Ok(avenger_common::value::ScalarOrArray::new_array(
                        arrow::compute::cast(values, &DataType::Utf8)?
                            .as_any()
                            .downcast_ref::<arrow::array::StringArray>()
                            .expect("a Utf8 array")
                            .iter()
                            .map(|value| value.unwrap_or_default().to_string())
                            .collect(),
                    )),
                })?;
            let shape_alternatives = if shape_alternatives.is_empty() {
                vec![shape_names(rows)?]
            } else {
                shape_alternatives
            };
            let angles = angles(rows)?;
            let stroke = match symbol_filled(symbol) && !rows.has(Channel::Stroke) {
                true => 0.0,
                false => symbol_stroke_width(symbol).unwrap_or(1.0) / 2.0,
            };
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
                        row_reach =
                            max_reach(row_reach, turned_bounds(points, *angle, sizes[row].sqrt()));
                    }
                }
                reach.push(row_reach.map(|side| side + stroke));
            }
            Ok(Some(reach))
        }
        mark::Mark::Text(text) => {
            let zero = avenger_common::value::ScalarOrArray::new_scalar(0.0);
            let mut mark = encode::text_mark(rows, text, zero.clone(), zero)?;
            mark.font_size = avenger_common::value::ScalarOrArray::new_array(largest(
                &sizes(rows)?,
                len,
                rows.theme.font_size,
            ));
            let angles = angles(rows)?;
            let mut reach = Vec::with_capacity(len);
            for (row, label) in mark.labels().enumerate() {
                let bounds = rows
                    .engine
                    .bounds(&label.label)
                    .map_err(|error| eval_error("the text of a text mark", error))?;
                let [x, y] = text_origin(&bounds, [0.0, 0.0], label.align, label.baseline);
                let corners = [
                    (x, y),
                    (x + bounds.width, y),
                    (x, y + bounds.height),
                    (x + bounds.width, y + bounds.height),
                ];
                let row_reach = angles[row].iter().fold([0.0; 4], |reach, angle| {
                    max_reach(reach, turned_bounds(&corners, *angle, 1.0))
                });
                reach.push(row_reach);
            }
            Ok(Some(reach))
        }
        mark::Mark::Path(path) => {
            let paths = encode::path_values(rows)?;
            let angles = angles(rows)?;
            let stroke = path.stroke_width.unwrap_or(1.0) / 2.0;
            Ok(Some(
                (0..len)
                    .map(|row| {
                        let points = path_points(&at(&paths, row));
                        angles[row]
                            .iter()
                            .fold([0.0; 4], |reach, angle| {
                                max_reach(reach, turned_bounds(&points, *angle, 1.0))
                            })
                            .map(|side| side + stroke)
                    })
                    .collect(),
            ))
        }
        mark::Mark::Image(image) if !(rows.has(Channel::X2) && rows.has(Channel::Y2)) => {
            let [width, height] = image_size(image, rows.theme);
            let (align, baseline) = image_anchor(image);
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
            Ok(Some(vec![[left, right, top, bottom]; len]))
        }
        mark::Mark::Bar(bar) => {
            // A bar at a continuous key without a second position is as thick as the theme
            // says, about its key.
            let horizontal = match proto::Orient::try_from(bar.orient) {
                Ok(proto::Orient::Horizontal) => true,
                Ok(proto::Orient::Vertical) => false,
                _ => continuous(rows, Channel::X) && !continuous(rows, Channel::Y),
            };
            let (key, key2) = if horizontal {
                (Channel::Y, Channel::Y2)
            } else {
                (Channel::X, Channel::X2)
            };
            let banded = rows
                .scale_for(key)
                .is_some_and(|scale| encode::bandwidth(scale).is_some());
            if !rows.has(key) || rows.has(key2) || banded {
                return Ok(None);
            }
            let half = rows.theme.bar_thickness / 2.0;
            let reach = if horizontal {
                [0.0, 0.0, half, half]
            } else {
                [half, half, 0.0, 0.0]
            };
            Ok(Some(vec![reach; len]))
        }
        _ => Ok(None),
    }
}

fn continuous(rows: &UnitRows, channel: Channel) -> bool {
    rows.unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)
        .and_then(|encoding| encoding.scale.as_ref())
        .is_some_and(|spec| FitKind::of(&spec.r#type) == FitKind::Interval)
}

/// The reach of each row along a position channel: toward the lower values of its domain and
/// toward the higher values.
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

/// A position scale of a plot, which fits its domain at each length.
#[derive(Clone)]
pub(crate) struct Position {
    pub(crate) fitting: FittingScale,
    /// Whether the plot clips its marks, because the domain is explicit and can exclude data.
    pub(crate) clip: bool,
    /// Whether the producer's summary covers each member, whose reach the evaluation checks.
    pub(crate) summarized: Vec<bool>,
}

/// The values of a position group for its summary: the values of each member, with the reach of
/// each value's mark when the evaluator fits the domain to them, or `None` for a member without
/// values.
pub(crate) type FitValues = Vec<Option<scale::ExtendedValues>>;

/// Whether the range of a position group runs in reverse: down a `y` axis, unless the spec
/// reverses it.
pub(crate) fn reversed(group: &ScaleGroup) -> bool {
    group.spec.range.as_ref().is_some_and(|range| range.reverse)
}

/// The fitting scale of a position group, from its spec, the domains of selections, and the
/// values of its encodings with the reach of their marks.
pub(crate) fn position(
    chart: &Chart,
    group: &ScaleGroup,
    values: &FitValues,
    selections: &Selections,
    theme: &Theme,
) -> Result<Position> {
    let spec = &group.spec;
    let value_type = values
        .iter()
        .flatten()
        .next()
        .map(|(values, _)| values.data_type().clone())
        .unwrap_or(DataType::Float64);
    let mut unfitted = scale::unfitted(spec, &value_type)?;
    // The padding of the theme for the bands and points of a spec that sets none.
    let options = &mut unfitted.config.options;
    let defaults: &[(&str, f32)] = match spec.r#type.as_str() {
        "band" => &[
            ("padding_inner", theme.band_padding_inner),
            ("padding_outer", theme.band_padding_outer),
        ],
        "point" => &[("padding", theme.point_padding)],
        _ => &[],
    };
    for (name, value) in defaults {
        options
            .entry(name.to_string())
            .or_insert_with(|| Scalar::from_f64(*value as f64));
    }
    // A continuous range runs up the plot for `y`, and a discrete one down it, so the first
    // category is at the top. The fit sets its length at each size.
    let up = group.channel == Channel::Y && FitKind::of(&spec.r#type) != FitKind::Categories;
    let (start, end) = if up != reversed(group) {
        (1.0, 0.0)
    } else {
        (0.0, 1.0)
    };
    unfitted.config.range = Arc::new(Float64Array::from(vec![start, end]));
    Ok(match scale::domain_source(chart, group, selections)? {
        DomainSource::Explicit(domain) => {
            let scale = unfitted.clone().with_domain(domain.clone());
            let summary = scale::summarize(&unfitted, values.iter().flatten())?;
            let fitting = match (FitKind::of(&spec.r#type), summary) {
                // A band or point scale keeps its categories, and fits its clip padding to the
                // reach of the marks that its rows draw.
                (FitKind::Categories, Some(summary)) if spec.r#type != "ordinal" => {
                    FittingScale::new(
                        scale,
                        summary,
                        DomainOptions::Categories(CategoryOptions {
                            order: CategoryOrder::Explicit(domain),
                        }),
                    )
                }
                _ => FittingScale::from(scale),
            };
            Position {
                fitting,
                clip: true,
                summarized: vec![false; values.len()],
            }
        }
        DomainSource::Inferred { summaries, data } => {
            let fitting = match scale::inferred_summary(&unfitted, summaries, &data, values)? {
                Some(summary) => FittingScale::new(
                    unfitted.clone(),
                    summary,
                    scale::domain_options(chart, spec)?,
                ),
                None => FittingScale::from(unfitted.clone()),
            };
            Position {
                fitting,
                clip: false,
                summarized: data.iter().map(|data| !data).collect(),
            }
        }
    })
}
