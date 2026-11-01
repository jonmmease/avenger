//! The app tools that serve the navigations and the tooltips that plots declare.

use avenger_app_tools::{
    navigation::KeyedPanZoom,
    tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips},
};
use avenger_chart::{
    proto::{self, tool, tooltip, Channel, NearestMetric},
    resolve::Identity,
};
use avenger_chart_eval::ScaleNavigation;
use avenger_eventstream::stream::Rebuild;
use avenger_geometry::query::Metric;
use avenger_layout::Rect;
use avenger_scales::scales::ConfiguredScale;

use crate::{
    state::{ChartAppState, ChartChange},
    CHART_SCENE, TOOLTIP_SCENE,
};

/// The tool of navigation, which pans and zooms the scales of the navigated channels of each plot
/// that declares a `Navigate`, through the plot's projection, keyed by the plot's identity. The x
/// and y of the tool are the plot's first and second positional dimensions. A double click drops
/// the views of the plot's scales, so they fit again at each size.
pub(crate) fn navigation() -> KeyedPanZoom<ChartAppState, Identity> {
    KeyedPanZoom::xy(
        CHART_SCENE,
        navigated,
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
                state.evaluator.clear_scale_view(index, channel);
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

/// The rectangles of the plots that declare navigation and whose kind navigates scales, by their
/// identities.
fn navigated(state: &ChartAppState) -> Vec<(Identity, Rect)> {
    (state.declaring())
        .filter(|(_, plot, _)| !plot.coord.kind().navigated().is_empty())
        .filter_map(|(_, plot, _)| {
            let identity = plot.identity.as_ref()?;
            navigation_of(state, identity)?;
            let [x, y, width, height] = plot.rect;
            Some((identity.clone(), Rect::new(x, y, width, height)))
        })
        .collect()
}

/// The view of the scale of the positional dimension `dimension` of the plot with `identity`, when
/// its navigation moves the dimension's channel.
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
    state.evaluator.scale_view_mut(index, channel)
}

/// Reports the domains of the scales that the navigation of the plot with `identity` moves, in
/// one change: the domain of each view, or the fitted domain without one.
fn settled(state: &ChartAppState, identity: &Identity) {
    let Some((index, navigate)) = navigation_of(state, identity) else {
        return;
    };
    let mut domains = Vec::new();
    for channel in moves(state, index, navigate) {
        let Some(scale) = state.evaluator.scale_identity(index, channel) else {
            continue;
        };
        let shown = (state.evaluator.scale_view(index, channel).cloned())
            .or_else(|| state.evaluator.home_scale(index, channel));
        if let Some(shown) = shown {
            domains.push((scale.clone(), shown.domain().clone()));
        }
    }
    if !domains.is_empty() {
        state.report(ChartChange { domains });
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
