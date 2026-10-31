//! The tools of the selections that plots declare: boxes and lassos in the plots, which each plot
//! draws for the tuples that it shows, and clicks that select instances by a key column.

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
use avenger_eventstream::stream::Rebuild;
use avenger_layout::Rect;
use avenger_scales::{scalar::Scalar, scales::ConfiguredScale};

use crate::{
    state::{plot_scale, ChartAppState, Mode},
    BRUSH_SCENE, CHART_SCENE,
};

const AXES: [Channel; 2] = [Channel::X, Channel::Y];

/// The area of a box or a lasso: the identity of its plot and the name of its selection.
type Area = (Identity, String);

/// What a declaration selects with.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    /// A box on `x`, on `y`, or on both.
    Interval([bool; 2]),
    Points,
    Lasso,
}

fn form(select: &proto::Select) -> Form {
    match &select.form {
        Some(select::Form::Points(_)) => Form::Points,
        Some(select::Form::Lasso(_)) => Form::Lasso,
        Some(select::Form::Interval(interval)) if !interval.channels.is_empty() => {
            Form::Interval(AXES.map(|channel| interval.channels.contains(&(channel as i32))))
        }
        Some(select::Form::Interval(_)) | None => Form::Interval([true, true]),
    }
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
        declared.identity == identity
            && declared.select.selection == *name
            && kind(form(declared.select))
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
/// each set of axes and a lasso brush, each for the plots that also declare a navigation and for
/// the others, and a point brush.
pub(crate) fn brushes() -> Brushes<ChartAppState> {
    let mut brushes = Brushes::new(BRUSH_SCENE, 1);
    for navigates in [false, true] {
        for covers in [[true, false], [false, true], [true, true]] {
            brushes = brushes.with(interval(covers, navigates));
        }
        brushes = brushes.with(lasso(navigates));
    }
    brushes.with(points())
}

/// Whether a brush for plots that do or do not declare navigation takes presses.
fn active(navigates: bool) -> impl Fn(&ChartAppState) -> bool + Send + Sync + 'static {
    move |state| !navigates || state.mode == Mode::Select
}

/// The rectangles of the plots that declare a selection of `kind` and do or do not declare a
/// navigation, with a continuous scale of each axis that a box or a lasso covers.
fn areas(state: &ChartAppState, kind: Form, navigates: bool) -> Vec<(Area, Rect)> {
    let covers = match kind {
        Form::Interval(covers) => covers,
        Form::Lasso => [true, true],
        Form::Points => [false, false],
    };
    let continuous = |plot: &EvaluatedPlot, channel: Channel| {
        plot_scale(plot, channel).is_some_and(|scale| scale.numeric_interval_domain().is_ok())
    };
    declared(state)
        .filter(|declared| form(declared.select) == kind && declared.navigates == navigates)
        .filter(|declared| {
            (AXES.iter().zip(covers))
                .all(|(channel, covered)| !covered || continuous(declared.plot, *channel))
        })
        .map(|declared| {
            let [x, y, width, height] = declared.plot.rect;
            let area = (declared.identity.clone(), declared.select.selection.clone());
            (area, Rect::new(x, y, width, height))
        })
        .collect()
}

/// The scale of `channel` of the plot of `area`.
fn scale(state: &ChartAppState, (identity, _): &Area, channel: Channel) -> ConfiguredScale {
    (state.plot(identity))
        .and_then(|(_, plot)| plot_scale(plot, channel))
        .cloned()
        .expect("the areas of a brush are plots with continuous scales of its axes")
}

/// Whether every row of the plot at `index` matches `test`.
fn satisfied(state: &ChartAppState, index: usize, test: &TupleTest) -> bool {
    state.evaluator.satisfies(index, test).unwrap_or(false)
}

/// The brush of boxes on the axes that `covers`, for the plots that do or do not declare a
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
        .x_scale(|state, area| scale(state, area, Channel::X))
        .y_scale(|state, area| scale(state, area, Channel::Y))
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

/// The box of the plot of `area` on the axes that `covers`: the ranges of the tuple of the
/// selection that the plot shows, when each covered axis records the column of one range and
/// every other test of the tuple holds in the plot.
fn shown_interval(state: &ChartAppState, area: &Area, covers: [bool; 2]) -> Option<Interval> {
    let (identity, name) = area;
    let (index, plot) = state.plot(identity)?;
    let tuple = state.contribution(name, identity)?.tuples.first()?;
    let mut ranges = [None; 2];
    for test in tuple {
        let drawn = match test {
            TupleTest::Range { column, ends, .. } => (0..2)
                .find(|&axis| {
                    covers[axis]
                        && state.evaluator.selection_column(index, AXES[axis]) == Some(column)
                })
                .map(|axis| (axis, ends)),
            _ => None,
        };
        match drawn {
            Some((axis, ends)) => {
                let scale = plot_scale(plot, AXES[axis])?
                    .clone()
                    .with_domain(ends.clone());
                let (start, end) = scale.numeric_interval_domain().ok()?;
                ranges[axis] = Some([start.min(end), start.max(end)]);
            }
            None if satisfied(state, index, test) => {}
            None => return None,
        }
    }
    (covers.iter().zip(&ranges))
        .all(|(covered, range)| !covered || range.is_some())
        .then_some(Interval {
            x: ranges[0],
            y: ranges[1],
        })
}

/// Writes the box of the plot of `area` to its selection: a range of the column that each
/// covered axis records, with the tests of the scope, or no contribution without a box.
fn write_interval(
    state: &mut ChartAppState,
    area: &Area,
    covers: [bool; 2],
    interval: Option<Interval>,
    settled: bool,
) {
    let tuple = interval.and_then(|interval| {
        let declared = declaration(state, area, |kind| matches!(kind, Form::Interval(_)))?;
        let mut tuple = Vec::new();
        for (axis, range) in [interval.x, interval.y].into_iter().enumerate() {
            let (true, Some([low, high])) = (covers[axis], range) else {
                continue;
            };
            let channel = AXES[axis];
            let column = state.evaluator.selection_column(declared.index, channel)?;
            let scale = plot_scale(declared.plot, channel)?.clone();
            tuple.push(TupleTest::Range {
                column: column.to_string(),
                channel,
                ends: scale.with_domain_interval((low, high)).domain().clone(),
                exclude_end: false,
            });
        }
        tuple.extend(scope(&declared));
        Some(tuple)
    });
    let (identity, name) = area;
    state.write(name, identity, tuple.into_iter().collect(), settled);
}

/// The brush of lassos, for the plots that do or do not declare a navigation.
fn lasso(navigates: bool) -> KeyedLassoBrush<ChartAppState, Area, ()> {
    KeyedLassoBrush::new(&format!("lasso {navigates}"))
        .areas(move |state| areas(state, Form::Lasso, navigates))
        .x_scale(|state, area| scale(state, area, Channel::X))
        .y_scale(|state, area| scale(state, area, Channel::Y))
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
/// when each tuple's polygon tests the columns that the plot's `x` and `y` record and every other
/// test of the tuple holds in the plot.
fn shown_polygons(state: &ChartAppState, area: &Area) -> Option<Vec<Vec<[f64; 2]>>> {
    let (identity, name) = area;
    let (index, _) = state.plot(identity)?;
    let columns = AXES.map(|channel| state.evaluator.selection_column(index, channel));
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

/// The vertices of the coordinates `x` and `y`.
fn vertices(x: &ArrayRef, y: &ArrayRef) -> Option<Vec<[f64; 2]>> {
    let numbers = |values: &ArrayRef| -> Option<Vec<f64>> {
        let numbers = cast(values, &DataType::Float64).ok()?;
        let numbers = numbers.as_any().downcast_ref::<Float64Array>()?;
        Some(
            numbers
                .iter()
                .map(|value| value.unwrap_or(f64::NAN))
                .collect(),
        )
    };
    let (x, y) = (numbers(x)?, numbers(y)?);
    Some(x.into_iter().zip(y).map(|(x, y)| [x, y]).collect())
}

/// Writes the lasso of the plot of `area` to its selection: a tuple for each polygon over the
/// columns that the plot's `x` and `y` record, with the tests of the scope.
fn write_lasso(state: &mut ChartAppState, area: &Area, lasso: &Lasso<()>, settled: bool) {
    let tuples = (|| {
        let declared = declaration(state, area, |kind| kind == Form::Lasso)?;
        let [x_column, y_column] =
            AXES.map(|channel| state.evaluator.selection_column(declared.index, channel));
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
    state.write(name, identity, tuples.unwrap_or_default(), settled);
}

/// The key column of a point selection.
fn key(select: &proto::Select) -> Option<&str> {
    match &select.form {
        Some(select::Form::Points(points)) => Some(&points.key),
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
        .instances(point_instances)
        .value(points_of)
        .on_change(|state, area, values| {
            write_points(state, area, values);
            Rebuild::tree(CHART_SCENE)
        })
}

/// The instances of the marks of the plot of `area`, by their values of the key column of its
/// point selection, or by the plot's value when the key column is a partition column.
fn point_instances(state: &ChartAppState, area: &Area) -> Instances<Scalar> {
    let Some(declared) = declaration(state, area, |form| form == Form::Points) else {
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
    let (identity, name) = area;
    let Some(key) = declaration(state, area, |form| form == Form::Points)
        .and_then(|declared| key(declared.select))
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
    let tuples = (|| {
        let declared = declaration(state, area, |form| form == Form::Points)?;
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
    state.write(name, identity, tuples.unwrap_or_default(), true);
}
