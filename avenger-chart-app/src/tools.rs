//! The app tools that serve the navigations and the tooltips that plots declare.

#[cfg(feature = "geo")]
use avenger_app_tools::navigation::KeyedMapPanZoom;
use avenger_app_tools::{
    navigation::KeyedPanZoom,
    tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips},
};
use avenger_chart::{
    proto::{self, tool, tooltip, Channel, NearestMetric},
    resolve::Identity,
    TupleTest,
};
use avenger_chart_eval::{EvaluatedPlot, ScaleNavigation};
#[cfg(feature = "geo")]
use avenger_chart_eval::{ProjectionNavigation, ProjectionView};
use avenger_eventstream::stream::Rebuild;
#[cfg(feature = "geo")]
use avenger_geo::PanMode;
use avenger_geometry::query::Metric;
use avenger_layout::Rect;
use avenger_scales::scales::ConfiguredScale;

use crate::{
    state::{ChartAppState, ChartChange},
    CHART_SCENE, TOOLTIP_SCENE,
};

/// The tool of navigation, which pans and zooms the scales of the navigated channels of each plot
/// that declares a `Navigate`, through the plot's projection, keyed by the plot's identity. The x
/// and y of the tool are the plot's first and second positional dimensions. A scale whose domain is
/// a selection domain has no view, so its navigation moves the selection: the plot's range of the
/// column that the channel records. A double click drops the views of the plot's scales, so they
/// fit again at each size, and the plot's ranges of the selections.
pub(crate) fn navigation() -> KeyedPanZoom<ChartAppState, Identity> {
    KeyedPanZoom::xy(
        CHART_SCENE,
        |state| navigated(state, |_, plot| !plot.coord.kind().navigated().is_empty()),
        |state, plot| view_mut(state, plot, 0),
        |state, plot| view_mut(state, plot, 1),
    )
    .projection(|state, plot| {
        let (index, _) = navigation_of(state, plot)?;
        state.plots[index].projection.clone()
    })
    .with_home(|state, plot| {
        if let Some((index, navigate)) = navigation_of(state, plot) {
            let moved: Vec<Channel> = moves(state, index, navigate).collect();
            for channel in moved {
                match state.evaluator.domain_selection(index, channel) {
                    Some(name) => {
                        let name = name.to_string();
                        (state.moved).retain(|(own, moved, _)| (own, *moved) != (plot, channel));
                        state.write(&name, plot, Vec::new());
                    }
                    None => state.evaluator.clear_scale_view(index, channel),
                }
            }
        }
        [None, None]
    })
    .bounds(|state, plot| scale_navigation(state, plot).map(|navigation| navigation.bounds))
    .min_span(|state, plot| scale_navigation(state, plot).map(|navigation| navigation.min_span))
    .on_settle(|state, plot| {
        settled(state, plot);
        Rebuild::none()
    })
}

/// The index of the plot with `identity` in the last evaluation, and the first navigation that
/// it declares.
fn navigation_of<'a>(
    state: &'a ChartAppState,
    identity: &Identity,
) -> Option<(usize, &'a proto::Navigate)> {
    let (index, _, tools) = state
        .declaring()
        .find(|(_, plot, _)| plot.identity.as_ref() == Some(identity))?;
    let navigate = tools.iter().find_map(|declared| match &declared.kind {
        Some(tool::Kind::Navigate(navigate)) => Some(navigate),
        _ => None,
    })?;
    Some((index, navigate))
}

/// The channels that `navigate` moves in the plot at `index`: the navigated channels of its kind
/// that `navigate` names, or all of them when it names none.
fn moves<'a>(
    state: &ChartAppState,
    index: usize,
    navigate: &'a proto::Navigate,
) -> impl Iterator<Item = Channel> + 'a {
    let navigated = state.plots[index].coord.kind().navigated();
    navigated.iter().copied().filter(|channel| {
        navigate.channels.is_empty() || navigate.channels.contains(&(*channel as i32))
    })
}

/// The channel of the positional dimension `dimension` of the plot at `index`.
fn dimension_channel(state: &ChartAppState, index: usize, dimension: usize) -> Channel {
    state.plots[index].coord.kind().positions()[dimension]
}

/// The navigation of the scales of the positional dimensions of the plot with `identity`, from
/// their specs.
fn scale_navigation(state: &ChartAppState, identity: &Identity) -> [ScaleNavigation; 2] {
    let index = navigation_of(state, identity).map(|(index, _)| index);
    [0, 1].map(|dimension| {
        (index.and_then(|index| {
            let channel = dimension_channel(state, index, dimension);
            state.evaluator.scale_navigation(index, channel)
        }))
        .unwrap_or_default()
    })
}

/// The rectangles of the plots that declare navigation and that `accepts`, by their index and
/// their plot, by their identities.
fn navigated(
    state: &ChartAppState,
    accepts: impl Fn(usize, &EvaluatedPlot) -> bool,
) -> Vec<(Identity, Rect)> {
    (state.declaring())
        .filter(|(index, plot, _)| accepts(*index, plot))
        .filter_map(|(_, plot, _)| {
            let identity = plot.identity.as_ref()?;
            navigation_of(state, identity)?;
            let [x, y, width, height] = plot.rect;
            Some((identity.clone(), Rect::new(x, y, width, height)))
        })
        .collect()
}

/// The view of the scale of the positional dimension `dimension` of the plot with `identity`, when
/// its navigation moves the dimension's channel, or the scale that the navigation moves in place
/// of a scale whose domain is a selection domain.
fn view_mut<'a>(
    state: &'a mut ChartAppState,
    identity: &Identity,
    dimension: usize,
) -> Option<&'a mut ConfiguredScale> {
    let (index, channel) = {
        let (index, navigate) = navigation_of(state, identity)?;
        let channel = dimension_channel(state, index, dimension);
        (moves(state, index, navigate).any(|moved| moved == channel)).then_some((index, channel))?
    };
    if state.evaluator.domain_selection(index, channel).is_none() {
        return state.evaluator.scale_view_mut(index, channel);
    }
    let position =
        (state.moved.iter()).position(|(own, moved, _)| (own, *moved) == (identity, channel));
    let position = match position {
        Some(position) => position,
        None => {
            let scale = state.plots[index].coord.scale(channel)?.clone();
            state.moved.push((identity.clone(), channel, scale));
            state.moved.len() - 1
        }
    };
    Some(&mut state.moved[position].2)
}

/// Writes the scales that navigations moved in place of scales whose domains are selection
/// domains to their selections, for the plot with `only`, or for every plot. A plot writes one
/// contribution to each selection: the tuple that it shows, with a range of the column that each
/// moved channel records in place of the tests of that column. A navigation of `x` and `y` that
/// read one selection therefore keeps both ranges, and a navigation of `x` under a box of `x` and
/// `y` keeps the range of `y`.
pub(crate) fn write_moved(state: &mut ChartAppState, only: Option<&Identity>) {
    let (writing, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut state.moved)
        .into_iter()
        .partition(|(plot, _, _)| only.is_none_or(|only| only == plot));
    state.moved = kept;
    // The ranges of each plot and selection, in the order of the moves.
    let mut ranges: Vec<(Identity, String, Vec<TupleTest>)> = Vec::new();
    for (plot, channel, scale) in writing {
        let Some((index, shown)) = state.plot(&plot) else {
            continue;
        };
        let unchanged = (shown.coord.scale(channel))
            .is_some_and(|shown| shown.domain().as_ref() == scale.domain().as_ref());
        let evaluator = &state.evaluator;
        let (Some(name), Some(column), false) = (
            evaluator.domain_selection(index, channel),
            evaluator.selection_column(index, channel),
            unchanged,
        ) else {
            continue;
        };
        let range = TupleTest::Range {
            column: column.to_string(),
            channel,
            ends: scale.domain().clone(),
            exclude_end: false,
        };
        let name = name.to_string();
        match (ranges.iter_mut()).find(|(own, written, _)| *own == plot && *written == name) {
            Some((_, _, moved)) => moved.push(range),
            None => ranges.push((plot, name, vec![range])),
        }
    }
    for (plot, name, moved) in ranges {
        let shown = (state.contribution(&name, &plot))
            .and_then(|contribution| contribution.tuples.first())
            .cloned()
            .unwrap_or_default();
        let mut tuple: Vec<TupleTest> = (shown.into_iter())
            .filter(|test| !moved.iter().any(|range| shares_column(test, range)))
            .collect();
        tuple.extend(moved);
        state.write(&name, &plot, vec![tuple]);
    }
}

/// Whether the tests `a` and `b` test a column in common.
fn shares_column(a: &TupleTest, b: &TupleTest) -> bool {
    let columns = |test: &TupleTest| -> Vec<String> {
        match test {
            TupleTest::Set { column, .. }
            | TupleTest::Range { column, .. }
            | TupleTest::CyclicRange { column, .. } => vec![column.clone()],
            TupleTest::Polygon {
                x_column, y_column, ..
            } => vec![x_column.clone(), y_column.clone()],
        }
    };
    let b = columns(b);
    columns(a).iter().any(|column| b.contains(column))
}

/// Reports the settle of the navigation of the plot with `identity` in one change: the domain of
/// each scale that it moves, from the view or the fitted domain without one, and the selections
/// of its scales whose domains are selection domains.
fn settled(state: &mut ChartAppState, identity: &Identity) {
    write_moved(state, Some(identity));
    let mut domains = Vec::new();
    let mut selections = Vec::new();
    if let Some((index, navigate)) = navigation_of(state, identity) {
        for channel in moves(state, index, navigate) {
            if let Some(name) = state.evaluator.domain_selection(index, channel) {
                selections.push(name.to_string());
                continue;
            }
            let Some(scale) = state.evaluator.scale_identity(index, channel) else {
                continue;
            };
            let shown = (state.evaluator.scale_view(index, channel).cloned())
                .or_else(|| state.evaluator.home_scale(index, channel));
            if let Some(shown) = shown {
                domains.push((scale.clone(), shown.domain().clone()));
            }
        }
    }
    for name in selections {
        state.touch(&name);
    }
    state.report(ChartChange {
        domains,
        settled: true,
        ..ChartChange::default()
    });
}

/// The tool of the navigation of maps, which pans and zooms the projection of each map plot that
/// declares a `Navigate`, keyed by the plot's identity, by the navigation of its geo spec: in its
/// pan mode, or the projection's own, and within its zoom limits. A double click drops the view
/// of the projection, so it fits again at each size.
#[cfg(feature = "geo")]
pub(crate) fn map_navigation() -> KeyedMapPanZoom<ChartAppState, Identity> {
    KeyedMapPanZoom::new(
        CHART_SCENE,
        |state| {
            navigated(state, |index, _| {
                let navigation = state.evaluator.projection_navigation(index);
                matches!(navigation, Some(ProjectionNavigation::Map { .. }))
            })
        },
        |state, plot| {
            let index = navigation_of(state, plot)?.0;
            match state.evaluator.projection_view_mut(index)? {
                ProjectionView::Map(projection) => Some(projection),
                _ => None,
            }
        },
        |state, plot| map_navigation_of(state, plot).0,
    )
    .with_home(|state, plot| {
        if let Some((index, _)) = navigation_of(state, plot) {
            state.evaluator.clear_projection_view(index);
        }
        None
    })
    .with_scale_limits(|state, plot, _| map_navigation_of(state, plot).1)
    .on_settle(|state, plot| {
        settled_projection(state, plot);
        Rebuild::none()
    })
}

/// The pan mode, and the smallest and the largest scale of the projection, of the map plot with
/// `identity`, from the evaluator's navigation of its projection.
#[cfg(feature = "geo")]
fn map_navigation_of(state: &ChartAppState, identity: &Identity) -> (PanMode, (f64, f64)) {
    let navigation = navigation_of(state, identity)
        .and_then(|(index, _)| state.evaluator.projection_navigation(index));
    match navigation {
        Some(ProjectionNavigation::Map {
            pan_mode,
            min_scale,
            max_scale,
        }) => (
            pan_mode,
            (min_scale.unwrap_or(0.0), max_scale.unwrap_or(f64::INFINITY)),
        ),
        _ => (PanMode::Translate, (0.0, f64::INFINITY)),
    }
}

/// Reports the settle of the navigation of the map plot with `identity` in one change: the
/// projection that it shows, from its view or its fit without one.
#[cfg(feature = "geo")]
fn settled_projection(state: &mut ChartAppState, identity: &Identity) {
    let mut projections = Vec::new();
    if let Some((index, _)) = navigation_of(state, identity) {
        let evaluator = &state.evaluator;
        let shown = (evaluator.projection_view(index).cloned())
            .or_else(|| evaluator.home_projection(index));
        if let (Some(projection), Some(shown)) = (evaluator.projection_identity(index), shown) {
            projections.push((projection.clone(), shown));
        }
    }
    state.report(ChartChange {
        projections,
        settled: true,
        ..ChartChange::default()
    });
}

/// The tool of tooltips, which describes the instances of the marks of each plot that declares a
/// `Tooltip`, on a scene above the chart.
pub(crate) fn tooltips() -> Tooltips<ChartAppState> {
    Tooltips::new(TOOLTIP_SCENE, 2).targets(|state: &ChartAppState| {
        (state.declaring())
            .flat_map(|(_, plot, tools)| {
                tools.iter().filter_map(|declared| match &declared.kind {
                    Some(tool::Kind::Tooltip(tooltip)) => {
                        Some(target(tooltip).with_marks(plot.marks.clone()))
                    }
                    _ => None,
                })
            })
            .collect()
    })
}

/// The target of `tooltip`, among the instances of every mark.
fn target(tooltip: &proto::Tooltip) -> InstanceTarget {
    let mut target = match &tooltip.find {
        Some(tooltip::Find::Nearest(nearest)) => {
            let metric = match NearestMetric::try_from(nearest.metric) {
                Ok(NearestMetric::AlongX) => Metric::AlongX,
                Ok(NearestMetric::AlongY) => Metric::AlongY,
                _ => Metric::Euclidean,
            };
            InstanceTarget::nearest(nearest.max_radius).with_metric(metric)
        }
        Some(tooltip::Find::Exact(_)) | None => InstanceTarget::exact(),
    };
    if tooltip.anchor == proto::TooltipAnchor::Pointer as i32 {
        target = target.with_anchor(TooltipAnchor::Pointer);
    }
    if let Some(header) = &tooltip.header {
        target = target.with_header(field(header));
    }
    if !tooltip.fields.is_empty() {
        target = target.with_fields(tooltip.fields.iter().map(field));
    }
    if tooltip.hide_swatch {
        target = target.without_swatch();
    }
    target
}

fn field(declared: &proto::TooltipField) -> TooltipField {
    let mut field = TooltipField::new(declared.column.as_str());
    if let Some(title) = &declared.title {
        field = field.with_title(title.as_str());
    }
    if !declared.format.is_empty() {
        field = field.with_value_format(declared.format.as_str());
    }
    if !declared.label.is_empty() {
        field = field.with_value_label(declared.label.as_str());
    }
    field
}
