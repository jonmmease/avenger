//! Scale-domain sharing policy for child-frame containers.
//!
//! This module converts child plot scale-sharing declarations into generic
//! child-frame domain requests. Container implementations still decide which
//! child frames exist and what their scope keys are.

use std::collections::{HashMap, HashSet};

use avenger_chart_core::{
    AvengerChartError, DomainCoordination, DomainCoordinationGroup, ResolvedUnitAspectConstraint,
    SharingLevel, sharing_group_boundary,
};

use crate::{
    channel::value::strip_trailing_numbers,
    scales::{DomainExtent, ScaleBuilder},
};

use super::{
    ChildFrameDomainRequest, ChildFrameScopeKey, CompiledPlot, CoordinationKind,
    CoordinationScopeKey, UnitAspectDomainNode, UnitAspectSpanGraphInput,
    aggregate_domain_requests, coordination_scope::CoordinationGroup, solve_unit_aspect_span_graph,
};

/// One local channel domain plus the child-frame domain coordination that applies to it.
#[derive(Clone, Debug)]
pub(crate) struct ChildFrameChannelDomainExtent {
    pub(crate) extent: DomainExtent,
    pub(crate) domain_coordination: DomainCoordination,
}

/// Domain-sharing inputs for one measured child frame.
pub(crate) struct ChildFrameDomainSharingInput<'a> {
    pub(crate) scope_key: &'a ChildFrameScopeKey,
    pub(crate) local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
    pub(crate) channel_domain_sharing_levels: &'a HashMap<String, DomainCoordination>,
}

impl<'a> ChildFrameDomainSharingInput<'a> {
    pub(crate) fn new(
        scope_key: &'a ChildFrameScopeKey,
        local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
        channel_domain_sharing_levels: &'a HashMap<String, DomainCoordination>,
    ) -> Self {
        Self {
            scope_key,
            local_domain_extents,
            channel_domain_sharing_levels,
        }
    }
}

/// Domain-sharing plus plot-area inputs for unit-aspect child-frame solves.
pub(crate) struct ChildFrameUnitAspectDomainSharingInput<'a> {
    pub(crate) domain_input: ChildFrameDomainSharingInput<'a>,
    pub(crate) unit_aspect_constraints: Vec<ResolvedUnitAspectConstraint>,
    pub(crate) plot_area_width: f32,
    pub(crate) plot_area_height: f32,
}

impl<'a> ChildFrameUnitAspectDomainSharingInput<'a> {
    pub(crate) fn new(
        scope_key: &'a ChildFrameScopeKey,
        local_domain_extents: &'a HashMap<String, ChildFrameChannelDomainExtent>,
        channel_domain_sharing_levels: &'a HashMap<String, DomainCoordination>,
        unit_aspect_constraints: Vec<ResolvedUnitAspectConstraint>,
        plot_area_width: f32,
        plot_area_height: f32,
    ) -> Self {
        Self {
            domain_input: ChildFrameDomainSharingInput::new(
                scope_key,
                local_domain_extents,
                channel_domain_sharing_levels,
            ),
            unit_aspect_constraints,
            plot_area_width,
            plot_area_height,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ChildFrameUnitAspectDomainSharingOutput {
    pub(crate) coordinated_domain_extents: Vec<HashMap<String, DomainExtent>>,
    pub(crate) unit_aspect_domain_overrides: Vec<HashMap<String, DomainExtent>>,
}

/// Extract the strongest scale-sharing level requested by each child plot channel.
pub(crate) fn child_frame_domain_sharing_levels_for_plot(
    plot: &CompiledPlot,
) -> HashMap<String, DomainCoordination> {
    let mut coordinations = HashMap::new();
    for mark in &plot.marks {
        for (channel, channel_value) in mark.data_context().channels() {
            collect_channel_domain_coordination(&mut coordinations, channel, channel_value);
        }
    }
    coordinations
}

fn collect_channel_domain_coordination(
    coordinations: &mut HashMap<String, DomainCoordination>,
    channel: &str,
    channel_value: &avenger_chart_core::ChannelValue,
) {
    let Some(coordination) = channel_value.get_domain_coordination() else {
        return;
    };
    let scale_name = channel_value
        .get_scale_name(channel)
        .unwrap_or_else(|| strip_trailing_numbers(channel).to_string());
    let sharing_level = SharingLevel::from(coordination.scope);
    coordinations
        .entry(scale_name)
        .and_modify(|existing: &mut DomainCoordination| {
            if sharing_level > SharingLevel::from(existing.scope) {
                *existing = coordination.clone();
            }
        })
        .or_insert_with(|| coordination.clone());
}

/// Extract local child-frame domain extents only for channels that request sharing.
pub(crate) fn extract_child_frame_domain_extents(
    scale_builder: &ScaleBuilder,
    domain_coordinations: &HashMap<String, DomainCoordination>,
    extra_channels: &HashSet<String>,
) -> HashMap<String, ChildFrameChannelDomainExtent> {
    let mut channels = domain_coordinations
        .iter()
        .filter_map(|(channel, coordination)| {
            (!SharingLevel::from(coordination.scope).is_free()).then_some(channel.as_str())
        })
        .collect::<Vec<_>>();
    channels.extend(extra_channels.iter().map(String::as_str));
    channels.sort_unstable();
    channels.dedup();
    if channels.is_empty() {
        return HashMap::new();
    }

    scale_builder
        .extract_domain_extents(&channels)
        .into_iter()
        .map(|(channel, extent)| {
            let domain_coordination = domain_coordinations
                .get(&channel)
                .cloned()
                .unwrap_or_else(|| DomainCoordination::scale_name(SharingLevel::FREE.into()));
            (
                channel,
                ChildFrameChannelDomainExtent {
                    extent,
                    domain_coordination,
                },
            )
        })
        .collect()
}

pub(crate) fn child_frame_domain_scope_key(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    coordination: &DomainCoordination,
) -> CoordinationScopeKey {
    let sharing_level = SharingLevel::from(coordination.scope);
    debug_assert!(
        !sharing_level.is_free(),
        "Free child-frame scale domains should not need a coordination scope"
    );
    let depth = u8::try_from(child_scope.container_path.len() + 1).unwrap_or(u8::MAX);
    let boundary = sharing_group_boundary(depth, sharing_level);
    let container_path = child_scope
        .container_path
        .get(..boundary.min(child_scope.container_path.len()))
        .unwrap_or(&child_scope.container_path)
        .to_vec();

    let key = CoordinationScopeKey::new(
        CoordinationKind::ScaleDomain,
        container_path,
        CoordinationGroup::Container,
    );
    apply_domain_group_to_key(key, channel, coordination)
}

pub(crate) fn apply_domain_group_to_key(
    key: CoordinationScopeKey,
    channel: &str,
    coordination: &DomainCoordination,
) -> CoordinationScopeKey {
    match &coordination.group {
        DomainCoordinationGroup::ScaleName => key.with_channel(channel),
        DomainCoordinationGroup::Named(group) => key.with_named_group(group.clone()),
    }
}

fn child_frame_domain_request(
    child_scope: &ChildFrameScopeKey,
    channel: &str,
    annotated: &ChildFrameChannelDomainExtent,
) -> Option<ChildFrameDomainRequest> {
    (!SharingLevel::from(annotated.domain_coordination.scope).is_free()).then(|| {
        ChildFrameDomainRequest::new(
            child_frame_domain_scope_key(child_scope, channel, &annotated.domain_coordination),
            annotated.extent.clone(),
        )
    })
}

/// Resolve the coordinated scale domains each child frame should use.
///
/// Free channels are omitted. Non-free channels are grouped by the current
/// child-frame container. Empty children with no local extent still receive a
/// coordinated extent when another child in the group supplied one.
pub(crate) fn coordinated_child_frame_domain_extents(
    children: &[ChildFrameDomainSharingInput<'_>],
) -> Vec<HashMap<String, DomainExtent>> {
    let unified = unified_child_frame_domain_extents(children);
    coordinated_child_frame_domain_extents_from_unified(children, &unified)
}

pub(crate) fn coordinated_child_frame_domain_extents_with_unit_aspect(
    children: &[ChildFrameUnitAspectDomainSharingInput<'_>],
) -> Result<ChildFrameUnitAspectDomainSharingOutput, AvengerChartError> {
    let domain_inputs = children
        .iter()
        .map(|child| ChildFrameDomainSharingInput {
            scope_key: child.domain_input.scope_key,
            local_domain_extents: child.domain_input.local_domain_extents,
            channel_domain_sharing_levels: child.domain_input.channel_domain_sharing_levels,
        })
        .collect::<Vec<_>>();
    let unified = unified_child_frame_domain_extents(&domain_inputs);
    let mut coordinated =
        coordinated_child_frame_domain_extents_from_unified(&domain_inputs, &unified);
    let mut overrides = vec![HashMap::new(); children.len()];

    let mut graph_inputs = Vec::new();
    for child in children {
        for constraint in &child.unit_aspect_constraints {
            let Some((x_node, x_extent)) = unit_aspect_domain_node_and_extent(
                &child.domain_input,
                &unified,
                &constraint.x_scale,
            ) else {
                continue;
            };
            let Some((y_node, y_extent)) = unit_aspect_domain_node_and_extent(
                &child.domain_input,
                &unified,
                &constraint.y_scale,
            ) else {
                continue;
            };
            graph_inputs.push(UnitAspectSpanGraphInput {
                x_node,
                y_node,
                x_extent,
                y_extent,
                x_range_span: f64::from(child.plot_area_width),
                y_range_span: f64::from(child.plot_area_height),
                ratio: constraint.ratio,
            });
        }
    }

    if graph_inputs.is_empty() {
        return Ok(ChildFrameUnitAspectDomainSharingOutput {
            coordinated_domain_extents: coordinated,
            unit_aspect_domain_overrides: overrides,
        });
    }

    let solved = solve_unit_aspect_span_graph(&graph_inputs)?;
    for (index, child) in children.iter().enumerate() {
        for constraint in &child.unit_aspect_constraints {
            for scale_name in [&constraint.x_scale, &constraint.y_scale] {
                let Some((node, _)) =
                    unit_aspect_domain_node_and_extent(&child.domain_input, &unified, scale_name)
                else {
                    continue;
                };
                if let Some(extent) = solved.get(&node) {
                    coordinated[index].insert(scale_name.clone(), extent.clone());
                    overrides[index].insert(scale_name.clone(), extent.clone());
                }
            }
        }
    }

    Ok(ChildFrameUnitAspectDomainSharingOutput {
        coordinated_domain_extents: coordinated,
        unit_aspect_domain_overrides: overrides,
    })
}

fn unified_child_frame_domain_extents(
    children: &[ChildFrameDomainSharingInput<'_>],
) -> HashMap<CoordinationScopeKey, DomainExtent> {
    aggregate_domain_requests(children.iter().flat_map(|child| {
        child
            .local_domain_extents
            .iter()
            .filter_map(move |(channel, annotated)| {
                child_frame_domain_request(child.scope_key, channel, annotated)
            })
    }))
}

fn coordinated_child_frame_domain_extents_from_unified(
    children: &[ChildFrameDomainSharingInput<'_>],
    unified: &HashMap<CoordinationScopeKey, DomainExtent>,
) -> Vec<HashMap<String, DomainExtent>> {
    children
        .iter()
        .map(|child| {
            let mut coordinated = HashMap::new();
            for (channel, coordination) in child.channel_domain_sharing_levels {
                if SharingLevel::from(coordination.scope).is_free() {
                    continue;
                }

                let key = child_frame_domain_scope_key(child.scope_key, channel, coordination);
                if let Some(unified_extent) = unified.get(&key) {
                    coordinated.insert(channel.clone(), unified_extent.clone());
                }
            }
            coordinated
        })
        .collect()
}

fn unit_aspect_domain_node_and_extent(
    child: &ChildFrameDomainSharingInput<'_>,
    unified: &HashMap<CoordinationScopeKey, DomainExtent>,
    scale_name: &str,
) -> Option<(UnitAspectDomainNode, DomainExtent)> {
    if let Some(coordination) = child.channel_domain_sharing_levels.get(scale_name)
        && !SharingLevel::from(coordination.scope).is_free()
    {
        let key = child_frame_domain_scope_key(child.scope_key, scale_name, coordination);
        let extent = unified.get(&key).cloned().or_else(|| {
            child
                .local_domain_extents
                .get(scale_name)
                .map(|local| local.extent.clone())
        })?;
        return Some((UnitAspectDomainNode::Shared(key), extent));
    }

    let extent = child.local_domain_extents.get(scale_name)?.extent.clone();
    Some((
        UnitAspectDomainNode::Free {
            cell: format!("{:?}", child.scope_key),
            scale: scale_name.to_string(),
        },
        extent,
    ))
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::{ResolvedUnitAspectConstraint, UnitAspectPolicy};

    use crate::{
        container::{ChildFrameKey, ContainerPathSegment},
        prelude::*,
        scales::domain_extent::DomainExtent,
    };

    use super::*;
    use datafusion::prelude::{SessionContext, col};

    fn child_scope(
        ancestor: Option<&str>,
        child_index: usize,
        child_key: Option<&str>,
    ) -> ChildFrameScopeKey {
        let container_path = ancestor
            .map(|key| vec![ContainerPathSegment::concat_child(0, Some(key))])
            .unwrap_or_default();
        ChildFrameScopeKey::new(
            container_path,
            ChildFrameKey::ConcatChild {
                index: child_index,
                key: child_key.map(ToOwned::to_owned),
            },
        )
    }

    fn extent(max: f64, sharing_level: SharingLevel) -> ChildFrameChannelDomainExtent {
        ChildFrameChannelDomainExtent {
            extent: DomainExtent::numeric(0.0, max),
            domain_coordination: DomainCoordination::scale_name(sharing_level.into()),
        }
    }

    fn named_extent(
        max: f64,
        sharing_level: SharingLevel,
        group: &str,
    ) -> ChildFrameChannelDomainExtent {
        ChildFrameChannelDomainExtent {
            extent: DomainExtent::numeric(0.0, max),
            domain_coordination: DomainCoordination::named(sharing_level.into(), group).unwrap(),
        }
    }

    fn unit_aspect_constraint() -> ResolvedUnitAspectConstraint {
        ResolvedUnitAspectConstraint {
            x_channel: "x".to_string(),
            y_channel: "y".to_string(),
            x_scale: "x".to_string(),
            y_scale: "y".to_string(),
            ratio: 1.0,
            policy: UnitAspectPolicy::ExpandDomain,
        }
    }

    fn numeric_span(extent: &DomainExtent) -> f64 {
        let avenger_chart_scales::domain_extent::DomainBounds::Numeric { min, max } =
            &extent.bounds
        else {
            panic!("expected numeric extent");
        };
        *max - *min
    }

    fn assert_span(extent: &DomainExtent, expected: f64) {
        let actual = numeric_span(extent);
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected span {expected}, got {actual}"
        );
    }

    #[test]
    fn coordinated_child_frame_domain_extents_group_siblings() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let right_extents = HashMap::from([("x".to_string(), extent(101.0, SharingLevel::GLOBAL))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(coordinated.len(), 2);
        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(coordinated[0].get("x"), coordinated[1].get("x"));
    }

    #[test]
    fn free_child_frame_domain_extents_are_not_coordinated() {
        let scope = child_scope(None, 0, Some("left"));
        let local_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::FREE))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::FREE.into()),
        )]);
        let inputs = [ChildFrameDomainSharingInput::new(
            &scope,
            &local_extents,
            &sharing_levels,
        )];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(coordinated, vec![HashMap::new()]);
    }

    #[tokio::test]
    async fn child_frame_domain_sharing_levels_include_mark_owned_parallel_dimensions()
    -> Result<(), avenger_chart_core::AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<Parallel>::new()
            .mark(
                ParallelLine::new()
                    .dimension_with("mpg", col("mpg"), |dimension| dimension.free_domain())
                    .dimension("origin", col("origin")),
            )
            .compile(&ctx)
            .await?;

        let sharing = child_frame_domain_sharing_levels_for_plot(&compiled);

        assert!(
            sharing
                .get("mpg")
                .map(|coordination| SharingLevel::from(coordination.scope).is_free())
                .unwrap_or(false)
        );
        assert_eq!(
            sharing.get("origin").map(|coordination| coordination.scope),
            None,
            "implicit shared coordinate scales do not need explicit child-frame coordination"
        );
        assert!(!sharing.contains_key("__avenger_parallel_dim_mpg"));
        Ok(())
    }

    #[test]
    fn empty_child_frame_uses_saved_sharing_level() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([("x".to_string(), extent(2.0, SharingLevel::GLOBAL))]);
        let empty_extents = HashMap::new();
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &empty_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[1].get("x"),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
    }

    #[test]
    fn child_frame_domain_sharing_respects_container_path() {
        let left_scope = child_scope(Some("outer-left"), 0, Some("inner"));
        let right_scope = child_scope(Some("outer-right"), 0, Some("inner"));
        let left_extents =
            HashMap::from([("x".to_string(), extent(2.0, SharingLevel::from_raw(1)))]);
        let right_extents =
            HashMap::from([("x".to_string(), extent(101.0, SharingLevel::from_raw(1)))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::from_raw(1).into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
        assert_eq!(
            coordinated[1].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn named_child_frame_domain_extents_can_group_different_channels() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([(
            "x".to_string(),
            named_extent(2.0, SharingLevel::GLOBAL, "height"),
        )]);
        let right_extents = HashMap::from([(
            "y".to_string(),
            named_extent(101.0, SharingLevel::GLOBAL, "height"),
        )]);
        let left_sharing = HashMap::from([(
            "x".to_string(),
            DomainCoordination::named(SharingLevel::GLOBAL.into(), "height").unwrap(),
        )]);
        let right_sharing = HashMap::from([(
            "y".to_string(),
            DomainCoordination::named(SharingLevel::GLOBAL.into(), "height").unwrap(),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &left_sharing),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &right_sharing),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(
            coordinated[1].get("y"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn free_named_child_frame_domain_extents_are_not_coordinated_across_siblings() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([(
            "x".to_string(),
            named_extent(2.0, SharingLevel::FREE, "height"),
        )]);
        let right_extents = HashMap::from([(
            "y".to_string(),
            named_extent(101.0, SharingLevel::FREE, "height"),
        )]);
        let left_sharing = HashMap::from([(
            "x".to_string(),
            DomainCoordination::named(SharingLevel::FREE.into(), "height").unwrap(),
        )]);
        let right_sharing = HashMap::from([(
            "y".to_string(),
            DomainCoordination::named(SharingLevel::FREE.into(), "height").unwrap(),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &left_sharing),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &right_sharing),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(coordinated, vec![HashMap::new(), HashMap::new()]);
    }

    #[test]
    fn named_child_frame_domain_sharing_respects_outer_owner_scope() {
        let left_scope = child_scope(Some("outer-left"), 0, Some("inner"));
        let right_scope = child_scope(Some("outer-right"), 0, Some("inner"));
        let sharing_level = SharingLevel::from_raw(1);
        let left_extents =
            HashMap::from([("x".to_string(), named_extent(2.0, sharing_level, "height"))]);
        let right_extents = HashMap::from([(
            "y".to_string(),
            named_extent(101.0, sharing_level, "height"),
        )]);
        let left_sharing = HashMap::from([(
            "x".to_string(),
            DomainCoordination::named(sharing_level.into(), "height").unwrap(),
        )]);
        let right_sharing = HashMap::from([(
            "y".to_string(),
            DomainCoordination::named(sharing_level.into(), "height").unwrap(),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &left_sharing),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &right_sharing),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 2.0))
        );
        assert_eq!(
            coordinated[1].get("y"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
    }

    #[test]
    fn child_frame_domain_sharing_level_two_projects_one_ancestor() {
        let left_scope = child_scope(Some("outer-left"), 0, Some("inner"));
        let right_scope = child_scope(Some("outer-right"), 0, Some("inner"));
        let left_extents =
            HashMap::from([("x".to_string(), extent(2.0, SharingLevel::from_raw(2)))]);
        let right_extents =
            HashMap::from([("x".to_string(), extent(101.0, SharingLevel::from_raw(2)))]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::from_raw(2).into()),
        )]);
        let inputs = [
            ChildFrameDomainSharingInput::new(&left_scope, &left_extents, &sharing_levels),
            ChildFrameDomainSharingInput::new(&right_scope, &right_extents, &sharing_levels),
        ];

        let coordinated = coordinated_child_frame_domain_extents(&inputs);

        assert_eq!(
            coordinated[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 101.0))
        );
        assert_eq!(coordinated[0].get("x"), coordinated[1].get("x"));
    }

    #[test]
    fn unit_aspect_child_frame_domains_expand_shared_x_from_free_y() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let left_extents = HashMap::from([
            ("x".to_string(), extent(10.0, SharingLevel::GLOBAL)),
            ("y".to_string(), extent(10.0, SharingLevel::FREE)),
        ]);
        let right_extents = HashMap::from([
            ("x".to_string(), extent(8.0, SharingLevel::GLOBAL)),
            ("y".to_string(), extent(5.0, SharingLevel::FREE)),
        ]);
        let sharing_levels = HashMap::from([(
            "x".to_string(),
            DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
        )]);
        let constraints = unit_aspect_constraint();
        let inputs = [
            ChildFrameUnitAspectDomainSharingInput::new(
                &left_scope,
                &left_extents,
                &sharing_levels,
                vec![constraints.clone()],
                200.0,
                100.0,
            ),
            ChildFrameUnitAspectDomainSharingInput::new(
                &right_scope,
                &right_extents,
                &sharing_levels,
                vec![constraints],
                200.0,
                100.0,
            ),
        ];

        let output =
            coordinated_child_frame_domain_extents_with_unit_aspect(&inputs).expect("solve");
        let coordinated = output.coordinated_domain_extents;
        let overrides = output.unit_aspect_domain_overrides;

        assert_eq!(coordinated.len(), 2);
        assert_span(coordinated[0].get("x").expect("left x"), 20.0);
        assert_span(coordinated[1].get("x").expect("right x"), 20.0);
        assert_eq!(coordinated[0].get("x"), coordinated[1].get("x"));
        assert_span(coordinated[0].get("y").expect("left y"), 10.0);
        assert_span(coordinated[1].get("y").expect("right y"), 10.0);
        assert_span(overrides[0].get("x").expect("left x override"), 20.0);
        assert_span(overrides[0].get("y").expect("left y override"), 10.0);
        assert_span(overrides[1].get("x").expect("right x override"), 20.0);
        assert_span(overrides[1].get("y").expect("right y override"), 10.0);
    }

    #[test]
    fn unit_aspect_child_frame_domains_reject_inconsistent_shared_equations() {
        let left_scope = child_scope(None, 0, Some("left"));
        let right_scope = child_scope(None, 1, Some("right"));
        let local_extents = HashMap::from([
            ("x".to_string(), extent(10.0, SharingLevel::GLOBAL)),
            ("y".to_string(), extent(10.0, SharingLevel::GLOBAL)),
        ]);
        let sharing_levels = HashMap::from([
            (
                "x".to_string(),
                DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
            ),
            (
                "y".to_string(),
                DomainCoordination::scale_name(SharingLevel::GLOBAL.into()),
            ),
        ]);
        let constraints = unit_aspect_constraint();
        let inputs = [
            ChildFrameUnitAspectDomainSharingInput::new(
                &left_scope,
                &local_extents,
                &sharing_levels,
                vec![constraints.clone()],
                200.0,
                100.0,
            ),
            ChildFrameUnitAspectDomainSharingInput::new(
                &right_scope,
                &local_extents,
                &sharing_levels,
                vec![constraints],
                400.0,
                100.0,
            ),
        ];

        let err = coordinated_child_frame_domain_extents_with_unit_aspect(&inputs)
            .expect_err("inconsistent repeat equations should fail");

        assert!(err.to_string().contains("inconsistent"));
    }
}
