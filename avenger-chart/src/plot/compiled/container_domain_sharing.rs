//! Scale-domain sharing policy for child-frame containers.
//!
//! This module converts child plot scale-sharing declarations into generic
//! child-frame domain requests. Container implementations still decide which
//! child frames exist and what their scope keys are.

use std::collections::{HashMap, HashSet};

use avenger_chart_core::{
    AvengerChartError, CoordinateDomainCellKey, CoordinateDomainCellRequest,
    CoordinateDomainDescriptor, CoordinateDomainGroupRequest, CoordinateDomainNode,
    CoordinateDomainScaleState, CoordinateDomainSharedNodeKey, CoordinateDomainSharingPolicy,
    DomainCoordination, DomainCoordinationGroup, SharingLevel, sharing_group_boundary,
};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    channel::value::strip_trailing_numbers,
    scales::{DomainExtent, ScaleBuilder},
};

use super::{
    ChildFrameDomainRequest, ChildFrameScopeKey, CompiledPlot, CoordinationKind,
    CoordinationScopeKey, aggregate_domain_requests, coordination_scope::CoordinationGroup,
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

#[derive(Debug)]
pub(crate) struct ChildFrameCoordinateDomainSharingOutput {
    pub(crate) coordinated_domain_extents: Vec<HashMap<String, DomainExtent>>,
    pub(crate) coordinate_domain_overrides: Vec<HashMap<String, DomainExtent>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildFrameCoordinateDomainGroupKind {
    FacetRepeat,
    AuthoredConcat,
}

pub(crate) struct ChildFrameCoordinateDomainCell<'a> {
    pub(crate) plot: &'a CompiledPlot,
    pub(crate) cell_key: CoordinateDomainCellKey,
    pub(crate) plot_area_width: f32,
    pub(crate) plot_area_height: f32,
    pub(crate) params: &'a IndexMap<String, ScalarValue>,
    pub(crate) coordinated_domain_extents: HashMap<String, DomainExtent>,
    pub(crate) descriptor_scale_states:
        Vec<(CoordinateDomainDescriptor, Vec<CoordinateDomainScaleState>)>,
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

pub(crate) fn resolve_child_frame_coordinate_domain_extents(
    group_kind: ChildFrameCoordinateDomainGroupKind,
    children: &[ChildFrameCoordinateDomainCell<'_>],
) -> Result<ChildFrameCoordinateDomainSharingOutput, AvengerChartError> {
    let mut coordinated = children
        .iter()
        .map(|child| child.coordinated_domain_extents.clone())
        .collect::<Vec<_>>();
    let mut overrides = vec![HashMap::new(); children.len()];

    let descriptor_ids = children
        .iter()
        .flat_map(|child| {
            child
                .descriptor_scale_states
                .iter()
                .map(|(descriptor, _)| descriptor.id.clone())
        })
        .collect::<HashSet<_>>();
    if descriptor_ids.is_empty() {
        return Ok(ChildFrameCoordinateDomainSharingOutput {
            coordinated_domain_extents: coordinated,
            coordinate_domain_overrides: overrides,
        });
    }

    for descriptor_id in descriptor_ids {
        let mut descriptor = None::<CoordinateDomainDescriptor>;
        let mut child_indices = Vec::new();
        let mut request_cells = Vec::new();
        let mut key_to_index = HashMap::new();
        let mut has_shared_nodes = false;
        for (index, child) in children.iter().enumerate() {
            let Some((child_descriptor, scale_states)) = child
                .descriptor_scale_states
                .iter()
                .find(|(descriptor, _)| descriptor.id == descriptor_id)
            else {
                continue;
            };
            if scale_states.is_empty() {
                continue;
            }
            descriptor.get_or_insert_with(|| child_descriptor.clone());
            has_shared_nodes |= scale_states
                .iter()
                .any(|state| matches!(state.node, CoordinateDomainNode::Shared { .. }));
            key_to_index.insert(child.cell_key.clone(), index);
            child_indices.push(index);
            request_cells.push(CoordinateDomainCellRequest {
                cell_key: &child.cell_key,
                plot_area_width: child.plot_area_width,
                plot_area_height: child.plot_area_height,
                params: child.params,
                scale_states,
            });
        }

        let Some(descriptor) = descriptor else {
            continue;
        };
        validate_coordinate_domain_group_policy(&descriptor, group_kind, has_shared_nodes)?;
        let Some(first_child_index) = child_indices.first().copied() else {
            continue;
        };
        let Some(provider) = children[first_child_index]
            .plot
            .coord_transform
            .domain_provider()
        else {
            continue;
        };
        let resolution = provider.resolve_domain_group(CoordinateDomainGroupRequest {
            descriptor_id: &descriptor.id,
            cells: &request_cells,
        })?;

        for cell_resolution in resolution.cells {
            let Some(index) = key_to_index.get(&cell_resolution.cell_key).copied() else {
                return Err(AvengerChartError::InternalError(format!(
                    "coordinate domain descriptor '{}' returned unknown child-frame cell key '{}'",
                    descriptor.id,
                    cell_resolution.cell_key.as_str()
                )));
            };
            for (scale_name, extent) in cell_resolution.domain_overrides {
                coordinated[index].insert(scale_name.clone(), extent.clone());
                overrides[index].insert(scale_name, extent);
            }
        }
    }

    Ok(ChildFrameCoordinateDomainSharingOutput {
        coordinated_domain_extents: coordinated,
        coordinate_domain_overrides: overrides,
    })
}

fn validate_coordinate_domain_group_policy(
    descriptor: &CoordinateDomainDescriptor,
    group_kind: ChildFrameCoordinateDomainGroupKind,
    has_shared_nodes: bool,
) -> Result<(), AvengerChartError> {
    if !has_shared_nodes {
        return Ok(());
    }
    match descriptor.sharing_policy {
        CoordinateDomainSharingPolicy::AnyCompatibleGroup => Ok(()),
        CoordinateDomainSharingPolicy::FacetRepeatGroups
            if group_kind == ChildFrameCoordinateDomainGroupKind::FacetRepeat =>
        {
            Ok(())
        }
        CoordinateDomainSharingPolicy::FacetRepeatGroups => {
            Err(AvengerChartError::InvalidArgument(format!(
                "coordinate domain descriptor '{}' does not support authored concat/grid shared domains",
                descriptor.id
            )))
        }
        CoordinateDomainSharingPolicy::LocalOnly => {
            Err(AvengerChartError::InvalidArgument(format!(
                "coordinate domain descriptor '{}' does not support shared child-frame domains",
                descriptor.id
            )))
        }
    }
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

pub(crate) fn child_frame_coordinate_domain_node_for_scale(
    cell_key: &CoordinateDomainCellKey,
    child_scope: &ChildFrameScopeKey,
    channel_domain_sharing_levels: &HashMap<String, DomainCoordination>,
    scale_name: &str,
) -> CoordinateDomainNode {
    if let Some(coordination) = channel_domain_sharing_levels.get(scale_name)
        && !SharingLevel::from(coordination.scope).is_free()
    {
        let key = child_frame_domain_scope_key(child_scope, scale_name, coordination);
        return CoordinateDomainNode::Shared {
            key: CoordinateDomainSharedNodeKey::new(format!("{key:?}")),
        };
    }

    CoordinateDomainNode::Local {
        cell_key: cell_key.clone(),
        scale_name: scale_name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        any::Any,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use avenger_common::value::ScalarOrArray;
    use avenger_scales::scales::ScaleImpl;
    use serde::{Deserialize, Serialize};

    use avenger_chart_core::{
        AxisSpec, CompiledGuide, CompiledMark, CompiledParamSpec, CompiledSelectionSpec,
        CompiledStoreSpec, CoordinateDomainBinding, CoordinateDomainCellKey,
        CoordinateDomainCellResolution, CoordinateDomainDescriptor, CoordinateDomainGroupRequest,
        CoordinateDomainGroupResolution, CoordinateDomainNode, CoordinateDomainProvider,
        CoordinateDomainRole, CoordinateDomainScaleState, CoordinateDomainSharedNodeKey,
        CoordinateDomainSharingPolicy, CoordinateSystemTransform, CoordinateSystemTransformCore,
        EventDatumFieldSpec, Legend, PlotGeometry, ScaleRangeBinding, Theme, TimeContext,
        ToolMetadata,
    };
    use avenger_chart_scales::PlotScaleSpec as ScaleSpec;
    use datafusion_proto::protobuf::LogicalPlanNode;
    use indexmap::IndexMap;

    use crate::{
        container::{ChildFrameKey, ContainerPathSegment},
        layout::LayoutSpec,
        prelude::*,
        scales::domain_extent::DomainExtent,
    };

    use super::*;
    use datafusion::prelude::{SessionContext, col};

    static FAKE_GROUP_CALLS: AtomicUsize = AtomicUsize::new(0);
    static FAKE_GROUP_CELL_COUNT: AtomicUsize = AtomicUsize::new(0);
    static FAKE_PROVIDER_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Clone, Copy, Debug, Serialize, Deserialize)]
    struct FakeCoordinateDomainTransform {
        policy_code: u8,
    }

    impl FakeCoordinateDomainTransform {
        fn new(sharing_policy: CoordinateDomainSharingPolicy) -> Self {
            let policy_code = match sharing_policy {
                CoordinateDomainSharingPolicy::LocalOnly => 0,
                CoordinateDomainSharingPolicy::FacetRepeatGroups => 1,
                CoordinateDomainSharingPolicy::AnyCompatibleGroup => 2,
            };
            Self { policy_code }
        }

        fn sharing_policy(&self) -> CoordinateDomainSharingPolicy {
            match self.policy_code {
                0 => CoordinateDomainSharingPolicy::LocalOnly,
                1 => CoordinateDomainSharingPolicy::FacetRepeatGroups,
                _ => CoordinateDomainSharingPolicy::AnyCompatibleGroup,
            }
        }

        fn descriptor(&self) -> CoordinateDomainDescriptor {
            fake_coordinate_domain_descriptor(self.sharing_policy())
        }
    }

    impl CoordinateSystemTransformCore for FakeCoordinateDomainTransform {
        fn required_channels(&self) -> &'static [&'static str] {
            &[]
        }

        fn transform(
            &self,
            _position_channels: &HashMap<&str, ScalarOrArray<f32>>,
            _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
            _plot_width: f32,
            _plot_height: f32,
        ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
            Err(AvengerChartError::InternalError(
                "fake coordinate transform is not renderable".to_string(),
            ))
        }

        fn default_range_binding(&self, _channel: &str) -> Option<ScaleRangeBinding> {
            None
        }

        fn default_scale_options(
            &self,
            _channel: &str,
            _scale_impl: &dyn ScaleImpl,
        ) -> HashMap<String, ScalarValue> {
            HashMap::new()
        }

        fn domain_provider(&self) -> Option<&dyn CoordinateDomainProvider> {
            Some(self)
        }
    }

    impl CoordinateDomainProvider for FakeCoordinateDomainTransform {
        fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
            vec![self.descriptor()]
        }

        fn resolve_domain_group(
            &self,
            request: CoordinateDomainGroupRequest<'_>,
        ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
            FAKE_GROUP_CALLS.fetch_add(1, Ordering::SeqCst);
            FAKE_GROUP_CELL_COUNT.store(request.cells.len(), Ordering::SeqCst);
            Ok(CoordinateDomainGroupResolution {
                cells: request
                    .cells
                    .iter()
                    .map(|cell| CoordinateDomainCellResolution {
                        cell_key: cell.cell_key.clone(),
                        domain_overrides: cell
                            .scale_states
                            .iter()
                            .map(|state| {
                                (
                                    state.scale_name.clone(),
                                    DomainExtent::numeric(0.0, f64::from(cell.plot_area_width)),
                                )
                            })
                            .collect(),
                        metadata: Vec::new(),
                    })
                    .collect(),
            })
        }
    }

    #[typetag::serde]
    impl CoordinateSystemTransform for FakeCoordinateDomainTransform {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
            Box::new(*self)
        }
    }

    fn fake_coordinate_domain_descriptor(
        sharing_policy: CoordinateDomainSharingPolicy,
    ) -> CoordinateDomainDescriptor {
        let mut descriptor = CoordinateDomainDescriptor::new("fake_coordinate_domain");
        descriptor.bindings = vec![
            CoordinateDomainBinding::observed("x", CoordinateDomainRole::X),
            CoordinateDomainBinding::observed("y", CoordinateDomainRole::Y),
        ];
        descriptor.sharing_policy = sharing_policy;
        descriptor.depends_on_plot_area = true;
        descriptor
    }

    fn fake_compiled_plot(sharing_policy: CoordinateDomainSharingPolicy) -> CompiledPlot {
        CompiledPlot {
            coord_transform: Box::new(FakeCoordinateDomainTransform::new(sharing_policy)),
            compiled_guide: None::<Arc<dyn CompiledGuide>>,
            marks: Vec::<Arc<dyn CompiledMark>>::new(),
            mark_groups: Vec::new(),
            mark_group_index_by_mark: Vec::new(),
            axis_specs: HashMap::<String, AxisSpec>::new(),
            legends: IndexMap::<String, Legend>::new(),
            legend_colorbar_overlays: Vec::new(),
            layout_spec: LayoutSpec::default(),
            title: None,
            subtitle: None,
            theme: None::<Arc<Theme>>,
            time_context: TimeContext::default(),
            scale_to_coord_channel: HashMap::new(),
            scale_specs: HashMap::<String, ScaleSpec>::new(),
            data: None::<LogicalPlanNode>,
            default_params: IndexMap::new(),
            param_specs: IndexMap::<String, CompiledParamSpec>::new(),
            store_specs: IndexMap::<String, CompiledStoreSpec>::new(),
            event_bindings: Vec::new(),
            event_datum_fields: Vec::<EventDatumFieldSpec>::new(),
            event_coord_fields: Vec::<EventDatumFieldSpec>::new(),
            selection_specs: IndexMap::<String, CompiledSelectionSpec>::new(),
            cursor_params: Vec::new(),
            tool_metadata: Vec::<ToolMetadata>::new(),
        }
    }

    fn fake_coordinate_domain_cell<'a>(
        plot: &'a CompiledPlot,
        params: &'a IndexMap<String, ScalarValue>,
        cell_id: &str,
        plot_area_width: f32,
        x_node: CoordinateDomainNode,
        y_node: CoordinateDomainNode,
    ) -> ChildFrameCoordinateDomainCell<'a> {
        let descriptor = plot
            .coord_transform
            .domain_provider()
            .expect("fake provider")
            .domain_descriptors()
            .pop()
            .expect("fake descriptor");
        ChildFrameCoordinateDomainCell {
            plot,
            cell_key: CoordinateDomainCellKey::new(cell_id),
            plot_area_width,
            plot_area_height: 100.0,
            params,
            coordinated_domain_extents: HashMap::new(),
            descriptor_scale_states: vec![(
                descriptor,
                vec![
                    fake_scale_state("x", CoordinateDomainRole::X, x_node),
                    fake_scale_state("y", CoordinateDomainRole::Y, y_node),
                ],
            )],
        }
    }

    fn fake_scale_state(
        scale_name: &str,
        role: CoordinateDomainRole,
        node: CoordinateDomainNode,
    ) -> CoordinateDomainScaleState {
        CoordinateDomainScaleState {
            scale_name: scale_name.to_string(),
            coord_channel: scale_name.to_string(),
            role,
            base_domain: Some(DomainExtent::numeric(0.0, 10.0)),
            range: Some((0.0, 100.0)),
            node,
            has_explicit_domain: false,
            raw_domain_param: None,
        }
    }

    fn local_node(cell_id: &str, scale_name: &str) -> CoordinateDomainNode {
        CoordinateDomainNode::Local {
            cell_key: CoordinateDomainCellKey::new(cell_id),
            scale_name: scale_name.to_string(),
        }
    }

    fn shared_node(key: &str) -> CoordinateDomainNode {
        CoordinateDomainNode::Shared {
            key: CoordinateDomainSharedNodeKey::new(key),
        }
    }

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
    fn generic_child_frame_coordinate_domain_solver_calls_provider_for_facet_repeat_group() {
        let _guard = FAKE_PROVIDER_TEST_LOCK.lock().expect("test lock");
        FAKE_GROUP_CALLS.store(0, Ordering::SeqCst);
        FAKE_GROUP_CELL_COUNT.store(0, Ordering::SeqCst);
        let plot = fake_compiled_plot(CoordinateDomainSharingPolicy::FacetRepeatGroups);
        let params = IndexMap::new();
        let children = [
            fake_coordinate_domain_cell(
                &plot,
                &params,
                "left",
                120.0,
                shared_node("shared:x"),
                local_node("left", "y"),
            ),
            fake_coordinate_domain_cell(
                &plot,
                &params,
                "right",
                80.0,
                shared_node("shared:x"),
                local_node("right", "y"),
            ),
        ];

        let output = resolve_child_frame_coordinate_domain_extents(
            ChildFrameCoordinateDomainGroupKind::FacetRepeat,
            &children,
        )
        .expect("facet/repeat coordinate-domain solve");

        assert_eq!(FAKE_GROUP_CALLS.load(Ordering::SeqCst), 1);
        assert_eq!(FAKE_GROUP_CELL_COUNT.load(Ordering::SeqCst), 2);
        assert_eq!(
            output.coordinate_domain_overrides[0].get("x"),
            Some(&DomainExtent::numeric(0.0, 120.0))
        );
        assert_eq!(
            output.coordinate_domain_overrides[1].get("x"),
            Some(&DomainExtent::numeric(0.0, 80.0))
        );
        assert_eq!(
            output.coordinated_domain_extents[0].get("x"),
            output.coordinate_domain_overrides[0].get("x")
        );
    }

    #[test]
    fn provider_policy_rejects_authored_concat_shared_coordinate_domain_groups() {
        let _guard = FAKE_PROVIDER_TEST_LOCK.lock().expect("test lock");
        FAKE_GROUP_CALLS.store(0, Ordering::SeqCst);
        let plot = fake_compiled_plot(CoordinateDomainSharingPolicy::FacetRepeatGroups);
        let params = IndexMap::new();
        let children = [
            fake_coordinate_domain_cell(
                &plot,
                &params,
                "left",
                120.0,
                shared_node("shared:x"),
                local_node("left", "y"),
            ),
            fake_coordinate_domain_cell(
                &plot,
                &params,
                "right",
                80.0,
                shared_node("shared:x"),
                local_node("right", "y"),
            ),
        ];

        let err = resolve_child_frame_coordinate_domain_extents(
            ChildFrameCoordinateDomainGroupKind::AuthoredConcat,
            &children,
        )
        .expect_err("authored concat shared provider group should fail");

        assert_eq!(FAKE_GROUP_CALLS.load(Ordering::SeqCst), 0);
        assert!(
            err.to_string()
                .contains("does not support authored concat/grid shared domains")
        );
    }
}
