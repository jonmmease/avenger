//! The tools of the selections that plots declare: boxes and lassos in the plots, which each plot
//! draws for the tuples that it shows, and clicks and lassos with a key that select instances by a
//! key column. Boxes and lassos work through each plot's projection, so one brush serves the plots
//! of every coordinate system: a box is a rectangle in a Cartesian plot, an annular sector in a
//! polar plot, and with the `geo` feature, a box between two meridians and two parallels on a map,
//! which selects a cyclic range of longitude, and a lasso on a map selects the places inside it.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, Float64Array},
    compute::cast,
    datatypes::DataType,
};
use avenger_app_tools::brushes::{
    Brushes, Instances, Interval, KeyedIntervalBrush, KeyedLassoBrush, KeyedPointBrush, Lasso,
};
use avenger_chart::{
    proto::{self, select, tool, Channel},
    resolve::Identity,
    TupleTest,
};
use avenger_chart_eval::EvaluatedPlot;
use avenger_coords::PlotProjection;
use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;
use avenger_scales::{scalar::Scalar, scales::ConfiguredScale};

use crate::{
    state::{ChartAppState, ChartChange, Mode},
    BRUSH_SCENE, CHART_SCENE,
};

/// The area of a box or a lasso: the identity of its plot and the name of its selection.
type Area = (Identity, String);

/// What a declaration selects with.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    /// A box on the plot's first positional dimension, on its second, or on both: `x` and `y`,
    /// or `theta` and `radius`.
    Interval([bool; 2]),
    Points,
    /// A lasso that selects the rows inside its path, as a polygon of the plot's positional
    /// dimensions.
    Lasso,
    /// A lasso that selects the instances inside its path by a key column.
    KeyedLasso,
}

/// The form of the selection that `declared` declares. A box covers the positional dimensions of
/// the plot whose channels it names, or both when it names none.
fn form(declared: &Declared) -> Form {
    match &declared.select.form {
        Some(select::Form::Points(_)) => Form::Points,
        Some(select::Form::Lasso(lasso)) if !lasso.key.is_empty() => Form::KeyedLasso,
        Some(select::Form::Lasso(_)) => Form::Lasso,
        Some(select::Form::Interval(interval)) if !interval.channels.is_empty() => {
            let channels = dimensions(declared.plot);
            Form::Interval(channels.map(|channel| interval.channels.contains(&(channel as i32))))
        }
        Some(select::Form::Interval(_)) | None => Form::Interval([true, true]),
    }
}

/// The channels of the positional dimensions of `plot`, in the order of its projection.
fn dimensions(plot: &EvaluatedPlot) -> [Channel; 2] {
    let positions = plot.coord.kind().positions();
    [positions[0], positions[1]]
}

/// A selection that a plot of the last evaluation declares.
struct Declared<'a> {
    index: usize,
    plot: &'a EvaluatedPlot,
    identity: &'a Identity,
    select: &'a proto::Select,
    /// Whether the plot also declares a navigation, so that its boxes and lassos take presses in
    /// `Mode::Select` alone.
    navigates: bool,
}

fn declared(state: &ChartAppState) -> impl Iterator<Item = Declared<'_>> {
    state.declaring().flat_map(|(index, plot, tools)| {
        let navigates =
            (tools.iter()).any(|declared| matches!(declared.kind, Some(tool::Kind::Navigate(_))));
        (plot.identity.iter()).flat_map(move |identity| {
            tools
                .iter()
                .filter_map(move |declared| match &declared.kind {
                    Some(tool::Kind::Select(select)) => Some(Declared {
                        index,
                        plot,
                        identity,
                        select,
                        navigates,
                    }),
                    _ => None,
                })
        })
    })
}

/// The declaration of the selection of `area` whose form is `kind`.
fn declaration<'a>(
    state: &'a ChartAppState,
    (identity, name): &Area,
    kind: impl Fn(Form) -> bool,
) -> Option<Declared<'a>> {
    declared(state).find(|declared| {
        declared.identity == identity && declared.select.selection == *name && kind(form(declared))
    })
}

/// The tests of the scope of a declaration: a set of the plot's value of each scope column.
fn scope(declared: &Declared) -> Vec<TupleTest> {
    (declared.select.scope.iter())
        .filter_map(|column| {
            Some(TupleTest::Set {
                column: column.clone(),
                channel: Channel::Unspecified,
                values: declared.identity.partition.get(column)?.clone(),
            })
        })
        .collect()
}

/// The tool of the selections, on a scene between the chart and its tooltips: a box brush for
/// each set of dimensions, a lasso brush, and a brush of lassos with a key, each for the plots
/// that also declare a navigation and for the others, and a point brush.
pub(crate) fn brushes() -> Brushes<ChartAppState> {
    let mut brushes = Brushes::new(BRUSH_SCENE, 1);
    for navigates in [false, true] {
        for covers in [[true, false], [false, true], [true, true]] {
            brushes = brushes.with(interval(covers, navigates));
        }
        brushes = brushes.with(lasso(navigates));
        brushes = brushes.with(keyed_lasso(navigates));
    }
    brushes.with(points())
}

/// Whether a brush for plots that do or do not declare navigation takes presses.
fn active(navigates: bool) -> impl Fn(&ChartAppState) -> bool + Send + Sync + 'static {
    move |state| !navigates || state.mode == Mode::Select
}

/// The rectangles of the plots that declare a selection of `kind` and do or do not declare a
/// navigation. A box or a lasso needs the plot's projection, and a dimension that it covers needs a
/// column that the plot records and a continuous scale, or no scale, as a map's longitude has.
fn areas(state: &ChartAppState, kind: Form, navigates: bool) -> Vec<(Area, Rect)> {
    // A lasso, with a key or without, draws its path through both dimensions.
    let covers = match kind {
        Form::Interval(covers) => covers,
        Form::Lasso | Form::KeyedLasso => [true, true],
        Form::Points => [false, false],
    };
    let usable = |declared: &Declared, dimension: usize| {
        let channel = dimensions(declared.plot)[dimension];
        let recorded = state.evaluator.selection_column(declared.index, channel);
        let scale = declared.plot.coord.scale(channel);
        (recorded.is_some() || kind == Form::KeyedLasso)
            && scale.is_none_or(|scale| scale.numeric_interval_domain().is_ok())
    };
    declared(state)
        .filter(|declared| form(declared) == kind && declared.navigates == navigates)
        .filter(|declared| kind == Form::Points || declared.plot.projection.is_some())
        .filter(|declared| {
            (0..2).all(|dimension| !covers[dimension] || usable(declared, dimension))
        })
        .map(|declared| {
            let [x, y, width, height] = declared.plot.rect;
            let area = (declared.identity.clone(), declared.select.selection.clone());
            (area, Rect::new(x, y, width, height))
        })
        .collect()
}

/// The projection of the plot of `area`.
fn projection(state: &ChartAppState, (identity, _): &Area) -> Option<Arc<dyn PlotProjection>> {
    state.plot(identity)?.1.projection.clone()
}

/// The scale of the positional dimension `dimension` of the plot of `area`, or `None` for a
/// dimension without one, such as the longitude of a map.
fn scale(state: &ChartAppState, (identity, _): &Area, dimension: usize) -> Option<ConfiguredScale> {
    let (_, plot) = state.plot(identity)?;
    plot.coord.scale(dimensions(plot)[dimension]).cloned()
}

/// Whether every row of the plot at `index` matches `test`.
fn satisfied(state: &ChartAppState, index: usize, test: &TupleTest) -> bool {
    state.evaluator.satisfies(index, test).unwrap_or(false)
}

/// The brush of boxes on the dimensions that `covers`, for the plots that do or do not declare a
/// navigation.
fn interval(covers: [bool; 2], navigates: bool) -> KeyedIntervalBrush<ChartAppState, Area> {
    let id = format!("box {covers:?} {navigates}");
    let brush = match covers {
        [true, false] => KeyedIntervalBrush::x(&id),
        [false, true] => KeyedIntervalBrush::y(&id),
        _ => KeyedIntervalBrush::xy(&id),
    };
    brush
        .areas(move |state| areas(state, Form::Interval(covers), navigates))
        .projection(projection)
        .x_scale(|state, area| scale(state, area, 0))
        .y_scale(|state, area| scale(state, area, 1))
        .value(move |state, area| shown_interval(state, area, covers))
        .on_change(move |state, area, interval| {
            write_interval(state, area, covers, interval, false);
            Rebuild::scene(CHART_SCENE)
        })
        .on_settle(move |state, area, interval| {
            write_interval(state, area, covers, interval, true);
            Rebuild::tree(CHART_SCENE)
        })
        .active(active(navigates))
}

/// The box of the plot of `area` on the dimensions that `covers`: the ranges of the tuple of the
/// selection that the plot shows, when each covered dimension records the column of one range, a
/// cyclic range along a dimension that wraps, and every other test of the tuple holds in the plot.
fn shown_interval(state: &ChartAppState, area: &Area, covers: [bool; 2]) -> Option<Interval> {
    let (identity, name) = area;
    let (index, plot) = state.plot(identity)?;
    let tuple = state.contribution(name, identity)?.tuples.first()?;
    let channels = dimensions(plot);
    let recorded = |column: &str| {
        (0..2).find(|&dimension| {
            covers[dimension]
                && state.evaluator.selection_column(index, channels[dimension]) == Some(column)
        })
    };
    let mut ranges = [None; 2];
    for test in tuple {
        match test {
            TupleTest::Range { column, ends, .. } if recorded(column).is_some() => {
                let dimension = recorded(column)?;
                let [start, end] = match plot.coord.scale(channels[dimension]) {
                    Some(scale) => {
                        let scale = scale.clone().with_domain(ends.clone());
                        let (start, end) = scale.numeric_interval_domain().ok()?;
                        [start, end]
                    }
                    None => <[f64; 2]>::try_from(numbers(ends)?).ok()?,
                };
                ranges[dimension] = Some([start.min(end), start.max(end)]);
            }
            TupleTest::CyclicRange {
                column, start, end, ..
            } if recorded(column).is_some() => {
                let dimension = recorded(column)?;
                // A cyclic range draws along a dimension that wraps.
                plot.projection.as_ref()?.period(dimension)?;
                ranges[dimension] = Some([*start, *end]);
            }
            _ if satisfied(state, index, test) => {}
            _ => return None,
        }
    }
    (covers.iter().zip(&ranges))
        .all(|(covered, range)| !covered || range.is_some())
        .then_some(Interval {
            x: ranges[0],
            y: ranges[1],
        })
}

/// Writes the box of the plot of `area` to its selection, with the tests of the scope, or no
/// contribution without a box. Each covered dimension writes a range of the column that it
/// records, or a cyclic range along a dimension that wraps, whose period is the width of the
/// domain of the dimension's scale, which spans the whole circle, or the period of its range
/// positions without a scale, such as 360 degrees of longitude.
fn write_interval(
    state: &mut ChartAppState,
    area: &Area,
    covers: [bool; 2],
    interval: Option<Interval>,
    settled: bool,
) {
    let tuple = interval.and_then(|interval| {
        let declared = declaration(state, area, |kind| matches!(kind, Form::Interval(_)))?;
        let plot = declared.plot;
        let channels = dimensions(plot);
        let mut tuple = Vec::new();
        for (dimension, range) in [interval.x, interval.y].into_iter().enumerate() {
            let (true, Some([first, second])) = (covers[dimension], range) else {
                continue;
            };
            let channel = channels[dimension];
            let column = state.evaluator.selection_column(declared.index, channel)?;
            let scale = plot.coord.scale(channel);
            let period = plot.projection.as_ref()?.period(dimension);
            let test = match (period, scale) {
                (Some(period), scale) => {
                    let period = match scale {
                        Some(scale) => {
                            let (start, end) = scale.numeric_interval_domain().ok()?;
                            (end - start).abs()
                        }
                        None => period,
                    };
                    TupleTest::CyclicRange {
                        column: column.to_string(),
                        channel,
                        start: first,
                        end: second,
                        period,
                    }
                }
                (None, Some(scale)) => TupleTest::Range {
                    column: column.to_string(),
                    channel,
                    ends: scale
                        .clone()
                        .with_domain_interval((first, second))
                        .domain()
                        .clone(),
                    exclude_end: false,
                },
                (None, None) => TupleTest::Range {
                    column: column.to_string(),
                    channel,
                    ends: Arc::new(Float64Array::from(vec![first, second])),
                    exclude_end: false,
                },
            };
            tuple.push(test);
        }
        tuple.extend(scope(&declared));
        Some(tuple)
    });
    let (identity, name) = area;
    state.write(name, identity, tuple.into_iter().collect());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The brush of lassos, for the plots that do or do not declare a navigation.
fn lasso(navigates: bool) -> KeyedLassoBrush<ChartAppState, Area, ()> {
    KeyedLassoBrush::new(&format!("lasso {navigates}"))
        .areas(move |state| areas(state, Form::Lasso, navigates))
        .projection(projection)
        .x_scale(|state, area| scale(state, area, 0))
        .y_scale(|state, area| scale(state, area, 1))
        .value(|state, area| Lasso {
            polygons: shown_polygons(state, area).unwrap_or_default(),
            keys: Vec::new(),
        })
        .on_change(|state, area, lasso| {
            write_lasso(state, area, &lasso, false);
            Rebuild::scene(CHART_SCENE)
        })
        .on_settle(|state, area, lasso| {
            write_lasso(state, area, &lasso, true);
            Rebuild::tree(CHART_SCENE)
        })
        .active(active(navigates))
}

/// The polygons of the plot of `area`: those of the tuples of the selection that the plot shows,
/// when each tuple's polygon tests the columns that the plot's positional dimensions record and
/// every other test of the tuple holds in the plot.
fn shown_polygons(state: &ChartAppState, area: &Area) -> Option<Vec<Vec<[f64; 2]>>> {
    let (identity, name) = area;
    let (index, plot) = state.plot(identity)?;
    let columns = dimensions(plot).map(|channel| state.evaluator.selection_column(index, channel));
    let mut polygons = Vec::new();
    for tuple in &state.contribution(name, identity)?.tuples {
        let mut polygon = None;
        for test in tuple {
            match test {
                TupleTest::Polygon {
                    x_column,
                    y_column,
                    x,
                    y,
                } if columns == [Some(x_column.as_str()), Some(y_column.as_str())] => {
                    polygon = Some(vertices(x, y)?);
                }
                _ if satisfied(state, index, test) => {}
                _ => return None,
            }
        }
        polygons.extend(polygon);
    }
    Some(polygons)
}

/// The numbers of `values`, with `NaN` for a null.
fn numbers(values: &ArrayRef) -> Option<Vec<f64>> {
    let numbers = cast(values, &DataType::Float64).ok()?;
    let numbers = numbers.as_any().downcast_ref::<Float64Array>()?;
    Some(
        numbers
            .iter()
            .map(|value| value.unwrap_or(f64::NAN))
            .collect(),
    )
}

/// The vertices of the coordinates `x` and `y`.
fn vertices(x: &ArrayRef, y: &ArrayRef) -> Option<Vec<[f64; 2]>> {
    let (x, y) = (numbers(x)?, numbers(y)?);
    Some(x.into_iter().zip(y).map(|(x, y)| [x, y]).collect())
}

/// Writes the lasso of the plot of `area` to its selection: a tuple for each polygon over the
/// columns that the plot's positional dimensions record, with the tests of the scope.
fn write_lasso(state: &mut ChartAppState, area: &Area, lasso: &Lasso<()>, settled: bool) {
    let tuples = (|| {
        let declared = declaration(state, area, |kind| kind == Form::Lasso)?;
        let [x_column, y_column] = dimensions(declared.plot)
            .map(|channel| state.evaluator.selection_column(declared.index, channel));
        let (x_column, y_column) = (x_column?, y_column?);
        let scope = scope(&declared);
        let tuples = lasso.polygons.iter().map(|polygon| {
            let coordinate = |axis: usize| -> ArrayRef {
                Arc::new(Float64Array::from_iter_values(
                    polygon.iter().map(|vertex| vertex[axis]),
                ))
            };
            let mut tuple = vec![TupleTest::Polygon {
                x_column: x_column.to_string(),
                y_column: y_column.to_string(),
                x: coordinate(0),
                y: coordinate(1),
            }];
            tuple.extend(scope.iter().cloned());
            tuple
        });
        Some(tuples.collect())
    })();
    let (identity, name) = area;
    state.write(name, identity, tuples.unwrap_or_default());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The key column of a point selection or of a lasso with a key.
fn key(select: &proto::Select) -> Option<&str> {
    match &select.form {
        Some(select::Form::Points(points)) => Some(&points.key),
        Some(select::Form::Lasso(lasso)) if !lasso.key.is_empty() => Some(&lasso.key),
        _ => None,
    }
}

/// The brush of the point selections, with an area for each plot and point selection that it
/// declares, in which a click selects the instances of the plot's marks by their values of the
/// key column.
fn points() -> KeyedPointBrush<ChartAppState, Area, Scalar> {
    KeyedPointBrush::new()
        .areas(|state| {
            ([false, true].into_iter())
                .flat_map(|navigates| areas(state, Form::Points, navigates))
                .collect()
        })
        .instances(|state, area| keyed_instances(state, area, Form::Points))
        .value(points_of)
        .on_change(|state, area, values| {
            write_points(state, area, values);
            Rebuild::tree(CHART_SCENE)
        })
}

/// The instances of the marks of the plot of `area`, by their values of the key column of its
/// declaration of `kind`, a point selection or a lasso with a key, or by the plot's value when the
/// key column is a partition column.
fn keyed_instances(state: &ChartAppState, area: &Area, kind: Form) -> Instances<Scalar> {
    let Some(declared) = declaration(state, area, |form| form == kind) else {
        return Instances::marks(Vec::<String>::new());
    };
    let key = key(declared.select).unwrap_or_default().to_string();
    let partition = declared.identity.partition.get(&key).cloned();
    Instances::marks(declared.plot.marks.clone()).key(move |datum| {
        match datum.rows.column_by_name(&key) {
            Some(column) => Scalar::try_from_array(column.as_ref(), datum.row).ok(),
            None => Some(Scalar(partition.clone()?)),
        }
    })
}

/// The values of the key column that the selection of `area` holds for its plot.
fn points_of(state: &ChartAppState, area: &Area) -> Vec<Scalar> {
    keys_of(state, area, Form::Points)
}

/// The values of the key column of the declaration of `area` of `kind` that the selection of
/// `area` holds for its plot.
fn keys_of(state: &ChartAppState, area: &Area, kind: Form) -> Vec<Scalar> {
    let (identity, name) = area;
    let Some(key) =
        declaration(state, area, |form| form == kind).and_then(|declared| key(declared.select))
    else {
        return Vec::new();
    };
    let Some(contribution) = state.contribution(name, identity) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    for test in contribution.tuples.iter().flatten() {
        match test {
            TupleTest::Set {
                column,
                values: set,
                ..
            } if column == key => values.extend(
                (0..set.len()).filter_map(|row| Scalar::try_from_array(set.as_ref(), row).ok()),
            ),
            _ => {}
        }
    }
    values
}

/// Writes `values` to the selection of `area`, owned by its plot: a set of the values of its key
/// column with the tests of the scope, or no contribution without values.
fn write_points(state: &mut ChartAppState, area: &Area, values: Vec<Scalar>) {
    write_keys(state, area, Form::Points, values, true);
}

/// Writes `values` to the selection of the declaration of `area` of `kind`, owned by its plot: a
/// set of the values of its key column with the tests of the scope, or no contribution without
/// values.
fn write_keys(
    state: &mut ChartAppState,
    area: &Area,
    kind: Form,
    values: Vec<Scalar>,
    settled: bool,
) {
    let tuples = (|| {
        let declared = declaration(state, area, |form| form == kind)?;
        let key = key(declared.select)?;
        if values.is_empty() {
            return None;
        }
        let mut tuple = vec![TupleTest::Set {
            column: key.to_string(),
            channel: Channel::Unspecified,
            values: Scalar::iter_to_array(values).ok()?,
        }];
        tuple.extend(scope(&declared));
        Some(vec![tuple])
    })();
    let (identity, name) = area;
    state.write(name, identity, tuples.unwrap_or_default());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The brush of lassos with a key, for the plots that do or do not declare a navigation, which
/// select the instances inside their paths by their values of the key column.
fn keyed_lasso(navigates: bool) -> KeyedLassoBrush<ChartAppState, Area, Scalar> {
    KeyedLassoBrush::new(&format!("keyed lasso {navigates}"))
        .areas(move |state| areas(state, Form::KeyedLasso, navigates))
        .projection(projection)
        .x_scale(|state, area| scale(state, area, 0))
        .y_scale(|state, area| scale(state, area, 1))
        .instances_in(|state, area| keyed_instances(state, area, Form::KeyedLasso))
        .value(shown_keyed_lasso)
        .on_change(|state, area, lasso| {
            write_keyed_lasso(state, area, lasso, false);
            Rebuild::scene(CHART_SCENE)
        })
        .on_settle(|state, area, lasso| {
            write_keyed_lasso(state, area, lasso, true);
            Rebuild::tree(CHART_SCENE)
        })
        .active(active(navigates))
}

/// The lasso of `area`: the keys that the selection holds for its plot, and the paths that wrote
/// them, while the plot's contribution still holds those keys.
fn shown_keyed_lasso(state: &ChartAppState, area: &Area) -> Lasso<Scalar> {
    let keys = keys_of(state, area, Form::KeyedLasso);
    let polygons = (state.lassos.iter())
        .find(|(own, lasso)| own == area && lasso.keys == keys)
        .map(|(_, lasso)| lasso.polygons.clone())
        .unwrap_or_default();
    Lasso { polygons, keys }
}

/// Writes the keys of the instances inside the lasso of `area` to its selection, and keeps the
/// lasso's paths with the keys that they wrote, so the paths stay drawn.
fn write_keyed_lasso(state: &mut ChartAppState, area: &Area, lasso: Lasso<Scalar>, settled: bool) {
    let mut keys: Vec<Scalar> = Vec::new();
    for key in lasso.keys {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    state.lassos.retain(|(own, _)| own != area);
    if !lasso.polygons.is_empty() {
        let kept = Lasso {
            polygons: lasso.polygons,
            keys: keys.clone(),
        };
        state.lassos.push((area.clone(), kept));
    }
    write_keys(state, area, Form::KeyedLasso, keys, settled);
}
