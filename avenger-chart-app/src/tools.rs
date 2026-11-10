//! The app tools that serve the navigations and the tooltips that plots and layers declare.

#[cfg(feature = "geo")]
use avenger_app_tools::navigation::KeyedGeoPanZoom;
use avenger_app_tools::{
    navigation::KeyedPanZoom,
    tooltips::{FieldValues, InstanceTarget, TooltipAnchor, TooltipField, Tooltips},
};
use avenger_chart::{
    proto::{self, tool, tooltip, Channel, NearestMetric},
    resolve::Identity,
};
use avenger_chart_eval::ScaleNavigation;
#[cfg(feature = "geo")]
use avenger_chart_eval::{ProjectionNavigation, ProjectionView};
use avenger_eventstream::stream::Rebuild;
#[cfg(feature = "geo")]
use avenger_geo::PanMode;
use avenger_geometry::query::Metric;
use avenger_layout::Rect;
use avenger_scales::scales::ConfiguredScale;
use avenger_selection::Condition;

use crate::{
    state::{ChartAppState, ChartChange},
    CHART_SCENE, TOOLTIP_SCENE,
};

/// The label of a channel in the conditions that the app writes, such as `x`.
pub(crate) fn label(channel: Channel) -> Option<String> {
    Some(channel.field_name().to_string())
}

/// The tool of navigation, which pans and zooms the scales of the navigated channels of each plot
/// that declares a `Navigate`, through the plot's projection, keyed by the plot's identity. The x
/// and y of the tool are the plot's first and second positional dimensions. A scale whose domain
/// reads a selection has no view, so its navigation moves the selection: the plot's range of the
/// column that the channel records. A double click drops the views of the plot's scales, so they
/// fit again at each size, and the plot's ranges of the selections.
pub(crate) fn navigation() -> KeyedPanZoom<ChartAppState, Identity> {
    KeyedPanZoom::xy(
        CHART_SCENE,
        |state| {
            navigated(state, |state, index| {
                !state.plots[index].kind.navigated().is_empty()
            })
        },
        |state, plot| view_mut(state, plot, 0),
        |state, plot| view_mut(state, plot, 1),
    )
    .projection(|state, plot| {
        let (index, _) = navigation_of(state, plot)?;
        Some(state.plots[index].coords.projection().clone())
    })
    .with_home(|state, plot| {
        if let Some((index, navigate)) = navigation_of(state, plot) {
            let moved: Vec<Channel> = moves(state, index, navigate).collect();
            for channel in moved {
                match state.evaluator.domain_selection(index, channel) {
                    Some(name) => {
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

/// The index of the plot with `identity` in the last evaluation, and its navigation, when the
/// navigation is active in the chart's modes.
pub(crate) fn navigation_of<'a>(
    state: &'a ChartAppState,
    identity: &Identity,
) -> Option<(usize, &'a proto::Navigate)> {
    let declared =
        (state.declaring()).find(|declared| declared.plot_tools && declared.owner == identity)?;
    let navigate = declared
        .tools
        .iter()
        .find_map(|declared| match &declared.kind {
            Some(tool::Kind::Navigate(navigate)) if state.evaluator.is_active(&declared.modes) => {
                Some(navigate)
            }
            _ => None,
        })?;
    Some((declared.index, navigate))
}

/// The channels that `navigate` moves in the plot at `index`: the navigated channels of its kind
/// that `navigate` names, or all of them when it names none.
fn moves<'a>(
    state: &ChartAppState,
    index: usize,
    navigate: &'a proto::Navigate,
) -> impl Iterator<Item = Channel> + 'a {
    let navigated = state.plots[index].kind.navigated();
    navigated.iter().copied().filter(|channel| {
        navigate.channels.is_empty() || navigate.channels.contains(&(*channel as i32))
    })
}

/// The channel of the positional dimension `dimension` of the plot at `index`.
fn dimension_channel(state: &ChartAppState, index: usize, dimension: usize) -> Channel {
    state.plots[index].kind.positions()[dimension]
}

/// The positional dimension of `channel` in the plot at `index`.
pub(crate) fn channel_dimension(
    state: &ChartAppState,
    index: usize,
    channel: Channel,
) -> Option<usize> {
    let positions = state.plots[index].kind.positions();
    positions.iter().position(|position| *position == channel)
}

/// The navigation of the scales of the positional dimensions of the plot with `identity`, from
/// their navigation limits.
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

/// The rectangles of the plots that declare a navigation and that `accepts`, by their identities.
fn navigated(
    state: &ChartAppState,
    accepts: impl Fn(&ChartAppState, usize) -> bool,
) -> Vec<(Identity, Rect)> {
    (state.declaring())
        .filter(|declared| declared.plot_tools && accepts(state, declared.index))
        .filter_map(|declared| {
            navigation_of(state, declared.owner)?;
            let [x, y, width, height] = declared.plot.rect;
            Some((declared.owner.clone(), Rect::new(x, y, width, height)))
        })
        .collect()
}

/// The view of the scale of the positional dimension `dimension` of the plot with `identity`, when
/// its navigation moves the dimension's channel, or the scale that the navigation moves in place
/// of a scale whose domain reads a selection.
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
            let scale = state.plots[index].coords.scale(dimension)?.clone();
            state.moved.push((identity.clone(), channel, scale));
            state.moved.len() - 1
        }
    };
    Some(&mut state.moved[position].2)
}

/// Writes the scales that navigations moved in place of scales whose domains read a selection to
/// their selections, for the plot with `only`, or for every plot. A plot writes one clause to each
/// selection: the clause that it shows, with a range of the column that each moved channel records
/// in place of the conditions of that column. A navigation of `x` and `y` that read one selection
/// therefore keeps both ranges, and a navigation of `x` under a box of `x` and `y` keeps the range
/// of `y`.
pub(crate) fn write_moved(state: &mut ChartAppState, only: Option<&Identity>) {
    let (writing, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut state.moved)
        .into_iter()
        .partition(|(plot, _, _)| only.is_none_or(|only| only == plot));
    state.moved = kept;
    // The ranges of each plot and selection, in the order of the moves.
    let mut ranges: Vec<(Identity, String, Vec<Condition>)> = Vec::new();
    for (plot, channel, scale) in writing {
        let Some((index, shown)) = state.plot(&plot) else {
            continue;
        };
        let Some(dimension) = channel_dimension(state, index, channel) else {
            continue;
        };
        let unchanged = (shown.coords.scale(dimension))
            .is_some_and(|shown| shown.domain().as_ref() == scale.domain().as_ref());
        let evaluator = &state.evaluator;
        let (Some(name), Some(column), false) = (
            evaluator.domain_selection(index, channel),
            evaluator.selection_column(index, channel),
            unchanged,
        ) else {
            continue;
        };
        let range = Condition::Range {
            column,
            channel: label(channel),
            ends: scale.domain().clone(),
            exclude_end: false,
            cells: None,
        };
        match (ranges.iter_mut()).find(|(own, written, _)| *own == plot && *written == name) {
            Some((_, _, moved)) => moved.push(range),
            None => ranges.push((plot, name, vec![range])),
        }
    }
    for (plot, name, moved) in ranges {
        let shown = (state.clause(&name, &plot))
            .map(|clause| clause.conditions.clone())
            .unwrap_or_default();
        let mut conditions: Vec<Condition> = (shown.into_iter())
            .filter(|condition| !moved.iter().any(|range| shares_column(condition, range)))
            .collect();
        conditions.extend(moved);
        state.write(&name, &plot, conditions);
    }
}

/// Whether the conditions `a` and `b` check a column in common.
fn shares_column(a: &Condition, b: &Condition) -> bool {
    let b = b.columns();
    a.columns().iter().any(|column| b.contains(column))
}

/// Reports the settle of the navigation of the plot with `identity` in one change: the domain of
/// each scale that it moves, from the view or the fitted domain without one, and the selections of
/// its scales whose domains read a selection.
fn settled(state: &mut ChartAppState, identity: &Identity) {
    write_moved(state, Some(identity));
    let mut domains = Vec::new();
    let mut selections = Vec::new();
    if let Some((index, navigate)) = navigation_of(state, identity) {
        for channel in moves(state, index, navigate) {
            if let Some(name) = state.evaluator.domain_selection(index, channel) {
                selections.push(name);
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

/// The tool of the navigation of geo plots, which pans and zooms the projection of each geo plot that
/// declares a `Navigate`, keyed by the plot's identity, by the navigation of its geo spec: in its
/// pan mode, or the projection's own, and within its zoom limits. A double click drops the view of
/// the projection, so it fits again at each size.
#[cfg(feature = "geo")]
pub(crate) fn geo_navigation() -> KeyedGeoPanZoom<ChartAppState, Identity> {
    KeyedGeoPanZoom::new(
        CHART_SCENE,
        |state| {
            navigated(state, |state, index| {
                let navigation = state.evaluator.projection_navigation(index);
                matches!(navigation, Some(ProjectionNavigation::Geo { .. }))
            })
        },
        |state, plot| {
            let index = navigation_of(state, plot)?.0;
            match state.evaluator.projection_view_mut(index)? {
                ProjectionView::Geo(projection) => Some(projection),
                _ => None,
            }
        },
        |state, plot| geo_navigation_of(state, plot).0,
    )
    .with_home(|state, plot| {
        if let Some((index, _)) = navigation_of(state, plot) {
            state.evaluator.clear_projection_view(index);
        }
        None
    })
    .with_scale_limits(|state, plot, _| geo_navigation_of(state, plot).1)
    .on_settle(|state, plot| {
        settled_projection(state, plot);
        Rebuild::none()
    })
}

/// The pan mode, and the smallest and the largest scale of the projection, of the geo plot with
/// `identity`, from the evaluator's navigation of its projection.
#[cfg(feature = "geo")]
fn geo_navigation_of(state: &ChartAppState, identity: &Identity) -> (PanMode, (f64, f64)) {
    let navigation = navigation_of(state, identity)
        .and_then(|(index, _)| state.evaluator.projection_navigation(index));
    match navigation {
        Some(ProjectionNavigation::Geo {
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

/// Reports the settle of the navigation of the geo plot with `identity` in one change: the
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

/// The tool of tooltips, which describes the instances of the marks of each plot and layer that
/// declares a `Tooltip` that is active in the chart's modes, on a scene above the chart. A layer's
/// tooltip describes its own marks. A tooltip without fields titles each column with the title of
/// the first encoding of its layer that reads it and sets one, as its axis or legend shows it.
pub(crate) fn tooltips() -> Tooltips<ChartAppState> {
    Tooltips::new(TOOLTIP_SCENE, 2).targets(|state: &ChartAppState| {
        (state.declaring())
            .flat_map(|declared| {
                let marks = declared.marks.clone();
                let titles = declared.titles.clone();
                (declared.tools.iter())
                    .filter_map(move |tool| match &tool.kind {
                        Some(tool::Kind::Tooltip(tooltip))
                            if state.evaluator.is_active(&tool.modes) =>
                        {
                            let target = target(tooltip).with_marks(marks.clone());
                            Some(target.with_titles(titles.clone()))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    })
}

/// The target of `tooltip`, among the instances of every mark. The tooltip of an instance anchor
/// stands beside the instance, as one of the data anchor does.
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
    if let Some(title) = &tooltip.title {
        target = target.with_title(field(title));
    }
    if !tooltip.fields.is_empty() {
        target = target.with_fields(tooltip.fields.iter().map(field));
    }
    if tooltip.hide_swatch {
        target = target.without_swatch();
    }
    // A lump's fields bind its values as its guides' labels do: the kept category or the Other
    // group's label as `value`, and `other`, `count`, and `weight`.
    target.with_values(FieldValues::new(avenger_chart_eval::lump_label_values))
}

fn field(declared: &proto::TooltipField) -> TooltipField {
    let mut field = TooltipField::new(declared.column.as_str());
    if let Some(title) = &declared.title {
        field = field.with_title(title.as_str());
    }
    if let Some(labels) = &declared.labels {
        if !labels.format.is_empty() {
            field = field.with_label_format(labels.format.as_str());
        }
        if !labels.markup.is_empty() {
            field = field.with_label(labels.markup.as_str());
        }
    }
    field
}
