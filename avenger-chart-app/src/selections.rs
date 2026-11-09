//! The tools of the selections that plots and layers declare: boxes and lassos in the plots, which
//! each owner draws for the clauses that it shows, and clicks and lassos with keys that select
//! instances by their key columns. A layer's selection writes its own clauses and selects the
//! instances of its own marks. Boxes and lassos work through each plot's coordinates, so one brush serves
//! the plots of every coordinate system: a box is a rectangle in a Cartesian plot, an annular
//! sector in a polar plot, and with the `geo` feature, a box between two meridians and two
//! parallels on a geo plot, which selects a cyclic range of longitude, and a lasso on a geo plot selects the
//! places inside it.
//!
//! A box whose declaration has a `cell` selects whole cells of each dimension that it selects
//! from a scale: it writes the plot's grid with each range, snaps to the cells, and reports its
//! focus to the host while the pointer is over its plot.

use std::sync::Arc;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array},
    compute::cast,
    datatypes::DataType,
};
use avenger_app::app::{Tool, ToolHandler, ToolInstall};
use avenger_app_tools::brushes::{
    Brushes, Instances, Interval, KeyedIntervalBrush, KeyedLassoBrush, KeyedPointBrush, Lasso,
};
use avenger_chart::{
    proto::{self, select, tool, Channel},
    resolve::Identity,
};
use avenger_chart_eval::EvaluatedPlot;
use avenger_coords::PlotCoords;
use avenger_eventstream::{
    manager::SceneTrees,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, Rebuild},
};
use avenger_layout::Rect;
use avenger_scales::{categories::CategoryIndex, scalar::Scalar};
use avenger_selection::{Condition, PixelGrid};

use crate::{
    state::{ChartAppState, ChartChange, FocusDimension, SelectionFocus},
    tools::label,
    BRUSH_SCENE, CHART_SCENE, TOOLTIP_SCENE,
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
    /// A lasso that selects the instances inside its path by its key columns.
    KeyedLasso,
}

/// The form of the selection that `declared` declares. A box covers the positional dimensions of
/// the plot whose channels it names, or both when it names none.
fn form(declared: &Declared) -> Form {
    match &declared.select.form {
        Some(select::Form::Points(_)) => Form::Points,
        Some(select::Form::Lasso(lasso)) if !lasso.keys.is_empty() => Form::KeyedLasso,
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
    let positions = plot.kind.positions();
    [positions[0], positions[1]]
}

/// A selection that a plot or a layer of the last evaluation declares.
struct Declared<'a> {
    index: usize,
    plot: &'a EvaluatedPlot,
    /// The owner of its clauses: the plot, or the layer.
    identity: &'a Identity,
    /// The marks that it selects the instances of, and the coordinates of its positions.
    marks: Vec<String>,
    coords: &'a PlotCoords,
    select: &'a proto::Select,
    /// Whether it is active in the chart's modes, so that its brush takes presses. An inactive box
    /// or lasso still draws.
    active: bool,
}

fn declared(state: &ChartAppState) -> impl Iterator<Item = Declared<'_>> {
    state.declaring().flat_map(move |declaration| {
        let crate::state::Declared {
            index,
            plot,
            owner,
            marks,
            coords,
            tools,
            ..
        } = declaration;
        tools
            .iter()
            .filter_map(move |declared| match &declared.kind {
                Some(tool::Kind::Select(select)) => Some(Declared {
                    index,
                    plot,
                    identity: owner,
                    marks: marks.clone(),
                    coords,
                    select,
                    active: state.evaluator.is_active(&declared.modes),
                }),
                _ => None,
            })
            .collect::<Vec<_>>()
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

/// The conditions of the scope of a declaration: a set of the plot's value of each scope column.
fn scope(declared: &Declared) -> Vec<Condition> {
    (declared.select.scope.iter())
        .filter_map(|column| {
            let value = declared.identity.partition.get(column)?.clone();
            Some(Condition::values(column.clone(), None, value))
        })
        .collect()
}

/// The width of the cells that the box of `declared` selects whole, when it declares one.
fn cell(declared: &Declared) -> Option<f64> {
    match &declared.select.form {
        Some(select::Form::Interval(interval)) => interval.cell,
        _ => None,
    }
}

/// The grid of each dimension of the box of `declared` that selects cells: each dimension that
/// the box covers, whose scale has a numeric range, and that does not wrap, with the plot's scale
/// of the last evaluation.
fn grids(declared: &Declared, covers: [bool; 2]) -> [Option<PixelGrid>; 2] {
    let Some(cell) = cell(declared) else {
        return [None, None];
    };
    std::array::from_fn(|dimension| {
        let coords = declared.coords;
        if !covers[dimension] || coords.dimension(dimension).period().is_some() {
            return None;
        }
        let scale = coords.scale(dimension)?.clone();
        PixelGrid::new(scale, cell).ok()
    })
}

/// The focus that a press at `at` would start, when a brush takes presses there: the last area
/// there of a point selection or of a box whose declaration has a `cell`, as a brush takes the
/// last area under a press.
fn focus_at(state: &ChartAppState, at: [f32; 2]) -> Option<SelectionFocus> {
    let mut found = None;
    for (area, rect) in areas(state, Form::Points, true) {
        if rect.contains(at) {
            found = focus_of(state, &area, Form::Points).or(found);
        }
    }
    for covers in [[true, false], [false, true], [true, true]] {
        for (area, rect) in areas(state, Form::Interval(covers), true) {
            if rect.contains(at) {
                found = focus_of(state, &area, Form::Interval(covers)).or(found);
            }
        }
    }
    found
}

/// The focus of the selection of `area` of the form `kind`: a point selection's keys, or each
/// dimension that a box of cells covers, when the box selects cells of all of them, with the
/// columns of the selection's scope.
fn focus_of(state: &ChartAppState, area: &Area, kind: Form) -> Option<SelectionFocus> {
    let declared = declaration(state, area, |form| form == kind)?;
    let mut dimensions: Vec<FocusDimension> = match kind {
        Form::Points => (keys(declared.select)?.iter())
            .map(|key| FocusDimension {
                column: key.clone(),
                channel: Channel::Unspecified,
                grid: None,
            })
            .collect(),
        Form::Interval(covers) => {
            let channels = dimensions(declared.plot);
            let grids = grids(&declared, covers);
            let mut focused = Vec::new();
            for dimension in (0..2).filter(|&dimension| covers[dimension]) {
                let channel = channels[dimension];
                let column = state.evaluator.selection_column(declared.index, channel)?;
                focused.push(FocusDimension {
                    column,
                    channel,
                    grid: Some(grids[dimension].clone()?),
                });
            }
            focused
        }
        Form::Lasso | Form::KeyedLasso => return None,
    };
    dimensions.extend(declared.select.scope.iter().map(|column| FocusDimension {
        column: column.clone(),
        channel: Channel::Unspecified,
        grid: None,
    }));
    Some(SelectionFocus {
        selection: declared.select.selection.clone(),
        owner: declared.identity.clone(),
        dimensions,
    })
}

/// Reports the focus of the focused selection again after an evaluation, when its grids
/// changed, or no focus when its plot no longer declares it.
pub(crate) fn refocus(state: &mut ChartAppState) {
    let Some(focused) = &state.focused else {
        return;
    };
    let area = (focused.owner.clone(), focused.selection.clone());
    let focus = [[true, false], [false, true], [true, true]]
        .map(Form::Interval)
        .into_iter()
        .chain([Form::Points])
        .find_map(|kind| focus_of(state, &area, kind));
    state.focus(focus);
}

/// The tool that reports the focus of selections that can pre-aggregate: the selection under the
/// pointer, from a stream on the tooltips' scene, which is above the brushes, so a press reports
/// its focus before a brush takes it. The stream handles each event and passes it on.
pub(crate) struct Focus;

#[async_trait::async_trait]
impl Tool<ChartAppState> for Focus {
    type State = ();

    fn install(&self, tool: &mut ToolInstall<'_, ChartAppState, ()>) {
        let events = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        tool.stream(TOOLTIP_SCENE, events, FollowFocus);
    }
}

struct FollowFocus;

#[async_trait::async_trait]
impl ToolHandler<ChartAppState, ()> for FollowFocus {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartAppState,
        _own: &mut (),
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let focus = match event {
            SceneGraphEvent::CursorLeft => None,
            event => event.position().and_then(|at| focus_at(state, at)),
        };
        state.focus(focus);
        Rebuild::none()
    }
}

/// The tool of the selections, on a scene between the chart and its tooltips: a box brush for
/// each set of dimensions, a lasso brush, and a brush of lassos with a key, each for the
/// selections that are active in the chart's modes, which take presses, and for the others, which
/// only draw, and a point brush for the active point selections. An active box or lasso takes the
/// presses on its plot over the plot's navigation, which pans and zooms below it.
pub(crate) fn brushes() -> Brushes<ChartAppState> {
    let mut brushes = Brushes::new(BRUSH_SCENE, 1);
    for active in [false, true] {
        for covers in [[true, false], [false, true], [true, true]] {
            brushes = brushes.with(interval(covers, active));
        }
        brushes = brushes.with(lasso(active));
        brushes = brushes.with(keyed_lasso(active));
    }
    brushes.with(points())
}

/// The rectangles of the plots and layers that declare a selection of `kind` that is or is not
/// active in the chart's modes. A box or a lasso needs the plot's projection, and a dimension that it covers needs a
/// column that the plot records and a continuous scale, or no scale, as a geo plot's longitude has.
fn areas(state: &ChartAppState, kind: Form, active: bool) -> Vec<(Area, Rect)> {
    // A lasso, with a key or without, draws its path through both dimensions.
    let covers = match kind {
        Form::Interval(covers) => covers,
        Form::Lasso | Form::KeyedLasso => [true, true],
        Form::Points => [false, false],
    };
    let usable = |declared: &Declared, dimension: usize| {
        let channel = dimensions(declared.plot)[dimension];
        let recorded = state.evaluator.selection_column(declared.index, channel);
        let scale = declared.coords.scale(dimension);
        (recorded.is_some() || kind == Form::KeyedLasso)
            && scale.is_none_or(|scale| scale.numeric_interval_domain().is_ok())
    };
    declared(state)
        .filter(|declared| form(declared) == kind && declared.active == active)
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

/// The coordinates of the owner of `area`, which convert the data of its positional dimensions to
/// positions: its plot's, or its layer's.
fn coords(state: &ChartAppState, (identity, _): &Area) -> Option<PlotCoords> {
    Some(state.declared(identity)?.coords.clone())
}

/// Whether every row of the plot at `index` matches `condition`.
fn satisfied(state: &ChartAppState, index: usize, condition: &Condition) -> bool {
    state.evaluator.satisfies(index, condition).unwrap_or(false)
}

/// The brush of boxes on the dimensions that `covers`, for the boxes that are or are not active
/// in the chart's mode.
fn interval(covers: [bool; 2], active: bool) -> KeyedIntervalBrush<ChartAppState, Area> {
    let id = format!("box {covers:?} {active}");
    let brush = match covers {
        [true, false] => KeyedIntervalBrush::x(&id),
        [false, true] => KeyedIntervalBrush::y(&id),
        _ => KeyedIntervalBrush::xy(&id),
    };
    brush
        .areas(move |state| areas(state, Form::Interval(covers), active))
        .coords(coords)
        .cells(move |state, area| {
            let declared = declaration(state, area, |kind| kind == Form::Interval(covers));
            declared.map_or([None, None], |declared| {
                grids(&declared, covers).map(|grid| grid.map(|grid| grid.cell))
            })
        })
        .value(move |state, area| shown_interval(state, area, covers))
        .on_change(move |state, area, interval| {
            write_interval(state, area, covers, interval, false);
            Rebuild::scene(CHART_SCENE)
        })
        .on_settle(move |state, area, interval| {
            write_interval(state, area, covers, interval, true);
            Rebuild::tree(CHART_SCENE)
        })
        .active(move |_| active)
}

/// The box of the plot of `area` on the dimensions that `covers`: the ranges of the clause of the
/// selection that the plot shows, when each covered dimension records the column of one range, a
/// cyclic range along a dimension that wraps, and every other condition of the clause holds in
/// the plot.
fn shown_interval(state: &ChartAppState, area: &Area, covers: [bool; 2]) -> Option<Interval> {
    let (identity, name) = area;
    let owner = state.declared(identity)?;
    let (index, plot, coords) = (owner.index, owner.plot, owner.coords);
    let conditions = &state.clause(name, identity)?.conditions;
    let channels = dimensions(plot);
    let recorded = |column: &str| {
        (0..2).find(|&dimension| {
            covers[dimension]
                && state
                    .evaluator
                    .selection_column(index, channels[dimension])
                    .as_deref()
                    == Some(column)
        })
    };
    let mut ranges = [None; 2];
    for condition in conditions {
        match condition {
            Condition::Range { column, ends, .. } if recorded(column).is_some() => {
                let dimension = recorded(column)?;
                let [start, end] = match coords.scale(dimension) {
                    Some(scale) => {
                        let scale = scale.clone().with_domain(ends.clone());
                        let (start, end) = scale.numeric_interval_domain().ok()?;
                        [start, end]
                    }
                    None => <[f64; 2]>::try_from(numbers(ends)?).ok()?,
                };
                ranges[dimension] = Some([start.min(end), start.max(end)]);
            }
            Condition::CyclicRange {
                column, start, end, ..
            } if recorded(column).is_some() => {
                let dimension = recorded(column)?;
                // A cyclic range draws along a dimension that wraps.
                coords.dimension(dimension).period()?;
                ranges[dimension] = Some([*start, *end]);
            }
            _ if satisfied(state, index, condition) => {}
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

/// Writes the box of the plot of `area` to its selection, with the conditions of the scope, or no
/// clause without a box. Each covered dimension writes a range of the column that it
/// records, or a cyclic range along a dimension that wraps, whose period is the dimension's period
/// in data, such as the width of the domain of a scale of a whole circle or 360 degrees of
/// longitude. The types of a range's ends and the pixel grids come from the declared scale of the
/// dimension's channel, not from the coordinates.
fn write_interval(
    state: &mut ChartAppState,
    area: &Area,
    covers: [bool; 2],
    interval: Option<Interval>,
    settled: bool,
) {
    let conditions = interval.and_then(|interval| {
        let declared = declaration(state, area, |kind| matches!(kind, Form::Interval(_)))?;
        let plot = declared.plot;
        let channels = dimensions(plot);
        let grids = grids(&declared, covers);
        let coords = declared.coords.clone();
        let mut conditions = Vec::new();
        for (dimension, range) in [interval.x, interval.y].into_iter().enumerate() {
            let (true, Some([first, second])) = (covers[dimension], range) else {
                continue;
            };
            let channel = channels[dimension];
            let column = state.evaluator.selection_column(declared.index, channel)?;
            let scale = coords.scale(dimension);
            let wraps = coords.dimension(dimension).period().is_some();
            let condition = match (wraps, scale) {
                (true, _) => {
                    let period = coords.dimension(dimension).data_period()?;
                    Condition::CyclicRange {
                        column: column.clone(),
                        channel: label(channel),
                        start: first,
                        end: second,
                        period,
                    }
                }
                (false, Some(scale)) => Condition::Range {
                    column: column.clone(),
                    channel: label(channel),
                    ends: scale
                        .clone()
                        .with_domain_interval((first, second))
                        .domain()
                        .clone(),
                    exclude_end: false,
                    cells: (grids[dimension].clone()).and_then(|grid| {
                        let [low, high] = coords
                            .dimension(dimension)
                            .positions_of([first, second])
                            .ok()?;
                        // The box covers whole cells, so each edge lies on a cell's edge, which the
                        // conversion through data can miss by a rounding.
                        let (origin, cell) = (grid.origin, grid.cell);
                        let edge =
                            |position: f64| origin + ((position - origin) / cell).round() * cell;
                        grid.span(edge(low), edge(high)).ok()
                    }),
                },
                (false, None) => Condition::Range {
                    column,
                    channel: label(channel),
                    ends: Arc::new(Float64Array::from(vec![first, second])),
                    exclude_end: false,
                    cells: None,
                },
            };
            conditions.push(condition);
        }
        conditions.extend(scope(&declared));
        Some(conditions)
    });
    let (identity, name) = area;
    state.write(name, identity, conditions.unwrap_or_default());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The brush of lassos, for the lassos that are or are not active in the chart's modes.
fn lasso(active: bool) -> KeyedLassoBrush<ChartAppState, Area, ()> {
    KeyedLassoBrush::new(&format!("lasso {active}"))
        .areas(move |state| areas(state, Form::Lasso, active))
        .coords(coords)
        .value(|state, area| Lasso {
            polygons: shown_polygons(state, area).unwrap_or_default(),
            ids: Vec::new(),
        })
        .on_change(|state, area, lasso| {
            write_lasso(state, area, &lasso, false);
            Rebuild::scene(CHART_SCENE)
        })
        .on_settle(|state, area, lasso| {
            write_lasso(state, area, &lasso, true);
            Rebuild::tree(CHART_SCENE)
        })
        .active(move |_| active)
}

/// The polygons of the plot of `area`: those of the clause of the selection that the plot shows,
/// when its polygon condition checks the columns that the plot's positional dimensions record and
/// every other condition of the clause holds in the plot.
fn shown_polygons(state: &ChartAppState, area: &Area) -> Option<Vec<Vec<[f64; 2]>>> {
    let (identity, name) = area;
    let owner = state.declared(identity)?;
    let (index, plot) = (owner.index, owner.plot);
    let columns = dimensions(plot).map(|channel| state.evaluator.selection_column(index, channel));
    let mut polygons = Vec::new();
    for condition in &state.clause(name, identity)?.conditions {
        match condition {
            Condition::Polygon {
                x_column,
                y_column,
                polygons: shown,
                ..
            } if columns == [Some(x_column.clone()), Some(y_column.clone())] => {
                for (x, y) in shown {
                    polygons.push(vertices(x, y)?);
                }
            }
            _ if satisfied(state, index, condition) => {}
            _ => return None,
        }
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

/// Writes the lasso of the plot of `area` to its selection: a polygon condition of its paths over
/// the columns that the plot's positional dimensions record, with the conditions of the scope.
fn write_lasso(state: &mut ChartAppState, area: &Area, lasso: &Lasso<()>, settled: bool) {
    let conditions = (|| {
        let declared = declaration(state, area, |kind| kind == Form::Lasso)?;
        let [x_column, y_column] = dimensions(declared.plot)
            .map(|channel| state.evaluator.selection_column(declared.index, channel));
        let (x_column, y_column) = (x_column?, y_column?);
        if lasso.polygons.is_empty() {
            return None;
        }
        let polygons = (lasso.polygons.iter())
            .map(|polygon| {
                let coordinate = |axis: usize| -> ArrayRef {
                    Arc::new(Float64Array::from_iter_values(
                        polygon.iter().map(|vertex| vertex[axis]),
                    ))
                };
                (coordinate(0), coordinate(1))
            })
            .collect();
        let mut conditions = vec![Condition::Polygon {
            x_column,
            y_column,
            polygons,
            cells: None,
        }];
        conditions.extend(scope(&declared));
        Some(conditions)
    })();
    let (identity, name) = area;
    state.write(name, identity, conditions.unwrap_or_default());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The key columns of a point selection or of a lasso with keys.
fn keys(select: &proto::Select) -> Option<Vec<String>> {
    match &select.form {
        Some(select::Form::Points(points)) => {
            Some(points.keys.iter().map(|key| key.column.clone()).collect())
        }
        Some(select::Form::Lasso(lasso)) if !lasso.keys.is_empty() => Some(lasso.keys.clone()),
        _ => None,
    }
}

/// The brush of the point selections, with an area for each plot and point selection that it
/// declares, in which a click selects the instances of the plot's marks by their values of the
/// key columns.
fn points() -> KeyedPointBrush<ChartAppState, Area, Vec<Scalar>> {
    KeyedPointBrush::new()
        .areas(|state| areas(state, Form::Points, true))
        .instances(|state, area| keyed_instances(state, area, Form::Points))
        .value(points_of)
        .on_change(|state, area, values| {
            write_points(state, area, values);
            Rebuild::tree(CHART_SCENE)
        })
}

/// The instances of the marks of the plot of `area`, by the tuple of their values of the key
/// columns of its declaration of `kind`, a point selection or a lasso with keys, with the plot's
/// value of a key column that is a partition column.
fn keyed_instances(state: &ChartAppState, area: &Area, kind: Form) -> Instances<Vec<Scalar>> {
    let Some(declared) = declaration(state, area, |form| form == kind) else {
        return Instances::marks(Vec::<String>::new());
    };
    let keys: Vec<(String, Option<ArrayRef>)> = (keys(declared.select).unwrap_or_default().iter())
        .map(|key| (key.clone(), declared.identity.partition.get(key).cloned()))
        .collect();
    Instances::marks(declared.marks.clone()).id(move |datum| {
        (keys.iter())
            .map(|(key, partition)| match datum.rows.column_by_name(key) {
                // A lump is its key, its category or its Other group, whatever its weight.
                Some(column) => {
                    let column =
                        avenger_chart::lump::keys(column).unwrap_or_else(|| column.clone());
                    Scalar::at(column.as_ref(), datum.row).ok()
                }
                None => Some(Scalar(partition.clone()?)),
            })
            .collect()
    })
}

/// The key of a point selection that is a column of lumps, and the source column that its sets
/// name in its place, when the selection has one.
fn lump_source<'a>(declared: &Declared<'a>) -> Option<(&'a str, &'a str)> {
    match &declared.select.form {
        Some(select::Form::Points(points)) => {
            (points.keys.iter()).find_map(|key| Some((key.column.as_str(), key.source.as_deref()?)))
        }
        _ => None,
    }
}

/// The keys of the lumps of `key` that the plot of `declared` draws: the plot's kept categories
/// and its Other group.
fn drawn_keys(state: &ChartAppState, declared: &Declared, key: &str) -> Option<ArrayRef> {
    state
        .evaluator
        .drawn_lump_keys(declared.index, key)
        .ok()
        .flatten()
}

/// The keys of the lumps of `key` that a set of `values` of their source selects in the plot of
/// `declared`: the key of each kept value, and for a complement, the key of each kept category of
/// the plot outside `values` and the plot's Other group.
fn lump_tuples(
    state: &ChartAppState,
    declared: &Declared,
    key: &str,
    values: &ArrayRef,
    complement: bool,
) -> Vec<Vec<Scalar>> {
    let Some(drawn) = drawn_keys(state, declared, key) else {
        return Vec::new();
    };
    let tuple = |keys: &ArrayRef, row: usize| Some(vec![Scalar::try_from_array(keys, row).ok()?]);
    if complement {
        let Some(excluded) = avenger_chart::lump::kept_keys(drawn.data_type(), values) else {
            return Vec::new();
        };
        let Ok(positions) = CategoryIndex::new(&excluded).and_then(|index| index.positions(&drawn))
        else {
            return Vec::new();
        };
        return (positions.into_iter().enumerate())
            .filter(|(_, position)| position.is_none())
            .filter_map(|(row, _)| tuple(&drawn, row))
            .collect();
    }
    let Some(kept) = avenger_chart::lump::kept_keys(drawn.data_type(), values) else {
        return Vec::new();
    };
    (0..kept.len())
        .filter_map(|row| tuple(&kept, row))
        .collect()
}

/// The set over the source `source` of the lumps of `key` of the tuples of keys `tuples` in the
/// plot of `declared`: the value of each kept category, or when the tuples hold the Other group,
/// the complement of the plot's kept categories outside the tuples, so the selection holds every
/// value that the plot draws as Other and the kept categories of the tuples.
fn lump_set(
    state: &ChartAppState,
    declared: &Declared,
    (key, source): (&str, &str),
    tuples: &[Vec<Scalar>],
) -> Option<Condition> {
    let drawn = drawn_keys(state, declared, key)?;
    let value_type = drawn.as_struct().column(0).data_type().clone();
    let is_other = |scalar: &Scalar| {
        avenger_chart::lump::others(&scalar.0).is_some_and(|others| others.value(0))
    };
    let values = |keys: Vec<&dyn Array>| match keys.is_empty() {
        true => Some(arrow::array::new_empty_array(&value_type)),
        false => arrow::compute::concat(&keys).ok(),
    };
    if !tuples.iter().any(|tuple| is_other(&tuple[0])) {
        let kept = (tuples.iter())
            .map(|tuple| tuple[0].0.as_struct().column(0).as_ref())
            .collect();
        return Some(Condition::values(source.to_string(), None, values(kept)?));
    }
    // The kept categories of the plot that the tuples do not hold.
    let selected = (tuples.iter())
        .map(|tuple| tuple[0].0.as_ref())
        .collect::<Vec<_>>();
    let selected = arrow::compute::concat(&selected).ok()?;
    let positions = (CategoryIndex::new(&selected).ok()?.positions(&drawn)).ok()?;
    let other = avenger_chart::lump::others(&drawn)?;
    let excluded = (0..drawn.len())
        .filter(|&row| !other.value(row) && positions[row].is_none())
        .map(|row| drawn.as_struct().column(0).slice(row, 1))
        .collect::<Vec<_>>();
    let excluded = values(excluded.iter().map(|value| value.as_ref()).collect())?;
    Condition::values(source.to_string(), None, excluded)
        .complement()
        .ok()
}

/// The tuples of key values that the selection of `area` holds for its plot.
fn points_of(state: &ChartAppState, area: &Area) -> Vec<Vec<Scalar>> {
    keys_of(state, area, Form::Points)
}

/// The tuples of values of the key columns of the declaration of `area` of `kind` that the
/// selection of `area` holds for its plot.
fn keys_of(state: &ChartAppState, area: &Area, kind: Form) -> Vec<Vec<Scalar>> {
    let (identity, name) = area;
    let Some(declared) = declaration(state, area, |form| form == kind) else {
        return Vec::new();
    };
    let Some(keys) = keys(declared.select) else {
        return Vec::new();
    };
    let Some(clause) = state.clause(name, identity) else {
        return Vec::new();
    };
    let mut tuples = Vec::new();
    for condition in &clause.conditions {
        match condition {
            Condition::Set {
                columns,
                values,
                complement,
                ..
            } if lump_source(&declared).is_some_and(|(_, source)| columns == &[source]) => {
                let (key, _) = lump_source(&declared).expect("a lump source");
                tuples.extend(lump_tuples(state, &declared, key, &values[0], *complement));
            }
            Condition::Set {
                columns,
                values,
                complement: false,
                ..
            } if columns.as_slice() == keys => {
                let count = values.first().map_or(0, |list| list.len());
                tuples.extend((0..count).filter_map(|row| {
                    (values.iter())
                        .map(|list| Scalar::at(list.as_ref(), row).ok())
                        .collect::<Option<Vec<_>>>()
                }));
            }
            _ => {}
        }
    }
    tuples
}

/// Writes `tuples` to the selection of `area`, owned by its plot: a set of the tuples of values of
/// its key columns with the conditions of the scope, or no clause without tuples.
fn write_points(state: &mut ChartAppState, area: &Area, tuples: Vec<Vec<Scalar>>) {
    write_keys(state, area, Form::Points, tuples, true);
}

/// Writes `tuples` to the selection of the declaration of `area` of `kind`, owned by its plot: a
/// set of the tuples of values of its key columns with the conditions of the scope, or no clause
/// without tuples.
fn write_keys(
    state: &mut ChartAppState,
    area: &Area,
    kind: Form,
    tuples: Vec<Vec<Scalar>>,
    settled: bool,
) {
    let conditions = (|| {
        let declared = declaration(state, area, |form| form == kind)?;
        let keys = keys(declared.select)?;
        if tuples.is_empty() {
            return None;
        }
        if let Some(source) = lump_source(&declared) {
            let mut conditions = vec![lump_set(state, &declared, source, &tuples)?];
            conditions.extend(scope(&declared));
            return Some(conditions);
        }
        let lists = (0..keys.len())
            .map(|column| {
                Scalar::iter_to_array(tuples.iter().map(|tuple| tuple[column].clone()).collect())
            })
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let set = Condition::set(keys.to_vec(), vec![None; keys.len()], lists).ok()?;
        let mut conditions = vec![set];
        conditions.extend(scope(&declared));
        Some(conditions)
    })();
    let (identity, name) = area;
    state.write(name, identity, conditions.unwrap_or_default());
    state.report(ChartChange {
        settled,
        ..ChartChange::default()
    });
}

/// The brush of lassos with a key, for the lassos that are or are not active in the chart's modes,
/// which select the instances inside their paths by their values of the key columns.
fn keyed_lasso(active: bool) -> KeyedLassoBrush<ChartAppState, Area, Vec<Scalar>> {
    KeyedLassoBrush::new(&format!("keyed lasso {active}"))
        .areas(move |state| areas(state, Form::KeyedLasso, active))
        .coords(coords)
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
        .active(move |_| active)
}

/// The lasso of `area`: the keys that the selection holds for its plot, and the paths that wrote
/// them, while the plot's clause still holds those keys.
fn shown_keyed_lasso(state: &ChartAppState, area: &Area) -> Lasso<Vec<Scalar>> {
    let keys = keys_of(state, area, Form::KeyedLasso);
    let polygons = (state.lassos.iter())
        .find(|(own, lasso)| own == area && lasso.ids == keys)
        .map(|(_, lasso)| lasso.polygons.clone())
        .unwrap_or_default();
    Lasso {
        polygons,
        ids: keys,
    }
}

/// Writes the keys of the instances inside the lasso of `area` to its selection, and keeps the
/// lasso's paths with the keys that they wrote, so the paths stay drawn.
fn write_keyed_lasso(
    state: &mut ChartAppState,
    area: &Area,
    lasso: Lasso<Vec<Scalar>>,
    settled: bool,
) {
    let mut keys: Vec<Vec<Scalar>> = Vec::new();
    for key in lasso.ids {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    state.lassos.retain(|(own, _)| own != area);
    if !lasso.polygons.is_empty() {
        let kept = Lasso {
            polygons: lasso.polygons,
            ids: keys.clone(),
        };
        state.lassos.push((area.clone(), kept));
    }
    write_keys(state, area, Form::KeyedLasso, keys, settled);
}
