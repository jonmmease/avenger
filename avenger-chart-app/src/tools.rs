//! The app tools that serve the navigations and the tooltips that plots declare.

use avenger_app_tools::{
    navigation::KeyedPanZoom,
    tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips},
};
use avenger_chart::{
    proto::{self, navigate_limits, tool, tooltip, Channel, NearestMetric},
    resolve::Identity,
};
use avenger_chart_eval::PlotCoord;
use avenger_eventstream::stream::Rebuild;
use avenger_geometry::query::Metric;
use avenger_layout::Rect;
use avenger_scales::scales::ConfiguredScale;

use crate::{
    state::{ChartAppState, ChartChange},
    CHART_SCENE, TOOLTIP_SCENE,
};

const AXES: [Channel; 2] = [Channel::X, Channel::Y];

/// The tool of navigation, which pans and zooms the scales of each Cartesian plot that declares
/// a `Navigate`, keyed by the plot's identity. A double click drops the views of the plot's
/// scales, so they fit again at each size.
pub(crate) fn navigation() -> KeyedPanZoom<ChartAppState, Identity> {
    KeyedPanZoom::xy(
        CHART_SCENE,
        navigated,
        |state, plot| view_mut(state, plot, Channel::X),
        |state, plot| view_mut(state, plot, Channel::Y),
    )
    .with_home(|state, plot| {
        if let Some((index, navigate)) = navigation_of(state, plot) {
            let moved: Vec<Channel> = moves(navigate).collect();
            for channel in moved {
                state.evaluator.clear_view(index, channel);
            }
        }
        [None, None]
    })
    .bounds(bounds)
    .min_span(|state, plot| {
        navigation_of(state, plot).map_or([None; 2], |(_, navigate)| {
            limits(navigate).map(|limits| limits.and_then(|limits| limits.min_span))
        })
    })
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

/// The channels that `navigate` moves.
fn moves(navigate: &proto::Navigate) -> impl Iterator<Item = Channel> + '_ {
    AXES.into_iter().filter(|channel| {
        navigate.channels.is_empty() || navigate.channels.contains(&(*channel as i32))
    })
}

/// The limits of `x` and of `y` of `navigate`.
fn limits(navigate: &proto::Navigate) -> [Option<&proto::NavigateLimits>; 2] {
    [navigate.x.as_ref(), navigate.y.as_ref()]
}

/// The rectangles of the Cartesian plots that declare navigation, by their identities.
fn navigated(state: &ChartAppState) -> Vec<(Identity, Rect)> {
    (state.declaring())
        .filter(|(_, plot, _)| matches!(plot.coord, PlotCoord::Cartesian { .. }))
        .filter_map(|(_, plot, _)| {
            let identity = plot.identity.as_ref()?;
            navigation_of(state, identity)?;
            let [x, y, width, height] = plot.rect;
            Some((identity.clone(), Rect::new(x, y, width, height)))
        })
        .collect()
}

/// The view of the scale of `channel` of the plot with `identity`, when its navigation moves the
/// channel.
fn view_mut<'a>(
    state: &'a mut ChartAppState,
    identity: &Identity,
    channel: Channel,
) -> Option<&'a mut ConfiguredScale> {
    let index = {
        let (index, navigate) = navigation_of(state, identity)?;
        moves(navigate)
            .any(|moved| moved == channel)
            .then_some(index)?
    };
    state.evaluator.view_mut(index, channel)
}

/// The bounds of `x` and of `y` of the navigation of the plot with `identity`, in the numeric
/// values of their scales.
fn bounds(state: &ChartAppState, identity: &Identity) -> [Option<[f64; 2]>; 2] {
    let Some((index, navigate)) = navigation_of(state, identity) else {
        return [None; 2];
    };
    let home = state.evaluator.home_scales(index);
    let mut bounds = [None; 2];
    for (axis, limits) in limits(navigate).into_iter().enumerate() {
        bounds[axis] = match limits.and_then(|limits| limits.bounds.as_ref()) {
            Some(navigate_limits::Bounds::Data(_)) => (home[axis].as_ref())
                .and_then(|home| home.numeric_interval_domain().ok())
                .map(|(start, end)| [start.min(end), start.max(end)]),
            Some(navigate_limits::Bounds::Interval(interval)) => {
                Some([interval.low, interval.high])
            }
            None => None,
        };
    }
    bounds
}

/// Reports the domain of each scale that the navigation of the plot with `identity` moves: the
/// domain of its view, or its fitted domain without one.
fn settled(state: &ChartAppState, identity: &Identity) {
    let Some((index, navigate)) = navigation_of(state, identity) else {
        return;
    };
    let home = state.evaluator.home_scales(index);
    for channel in moves(navigate) {
        let Some(scale) = state.evaluator.scale_identity(index, channel) else {
            continue;
        };
        let axis = usize::from(channel == Channel::Y);
        let shown = (state.evaluator.view(index, channel)).or(home[axis].as_ref());
        if let Some(shown) = shown {
            state.report(ChartChange::View {
                scale: scale.clone(),
                domain: shown.domain().clone(),
            });
        }
    }
}

/// The tool of tooltips, which describes the instances of the marks of each plot that declares a
/// `Tooltip`, on a scene above the chart.
pub(crate) fn tooltips() -> Tooltips<ChartAppState> {
    Tooltips::new(TOOLTIP_SCENE, 1).targets(|state: &ChartAppState| {
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
