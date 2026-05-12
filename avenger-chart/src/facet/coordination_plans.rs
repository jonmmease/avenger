use std::collections::{HashMap, HashSet};

use datafusion::common::ScalarValue;

use crate::{
    coords::{CellDomainInfo, CoordinatedLayout, CoordinatedOverflow, FacetAxis},
    facet::{
        coord::{FacetBandCoordinationApplyPlan, union_domain_extents},
        coordination::CoordinationGroupKey,
        sharing_level::SharingLevel,
        sharing_policy,
    },
    scales::domain_extent::DomainExtent,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoordinationNodeKey {
    pub(crate) path: Vec<usize>,
}

impl CoordinationNodeKey {
    pub(crate) fn new(path: Vec<usize>) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InitialRequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) local_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) domain_infos: Vec<CellDomainInfo>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementSnapshot {
    pub(crate) nodes: Vec<InitialRequirementNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
    pub(crate) unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    pub(crate) domain_target_nodes: HashSet<CoordinationNodeKey>,
    pub(crate) unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct InitialRequirementPass {
    pub(crate) snapshot: InitialRequirementSnapshot,
    pub(crate) aggregates: InitialRequirementAggregates,
    pub(crate) distribution: InitialRequirementDistributionPlan,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetedRequirementNodeSnapshot {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) local_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementSnapshot {
    pub(crate) nodes: Vec<RetargetedRequirementNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetedRequirementPass {
    pub(crate) snapshot: RetargetedRequirementSnapshot,
    pub(crate) aggregates: RetargetedRequirementAggregates,
    pub(crate) distribution: RetargetedRequirementDistributionPlan,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodePlan {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) apply_plan: FacetBandCoordinationApplyPlan,
    pub(crate) has_legend_overflow: bool,
    pub(crate) has_coordinated_extents: bool,
    pub(crate) remeasure_triggered: bool,
    pub(crate) child_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetPlan {
    pub(crate) node_plans: Vec<RetargetNodePlan>,
}

#[derive(Debug, Clone)]
pub(crate) struct RetargetNodeTrace {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) planned_has_legend_overflow: bool,
    pub(crate) planned_has_coordinated_extents: bool,
    pub(crate) planned_remeasure_required: bool,
    pub(crate) planned_axis_owner_ignore_empty_cells: bool,
    pub(crate) planned_adjusted_main_size: f32,
    pub(crate) planned_child_count: usize,
    pub(crate) parent_cross_size_propagated: bool,
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) remeasure_triggered: bool,
    pub(crate) remeasured_cell_count: usize,
    pub(crate) remeasure_skipped_cell_count: usize,
    pub(crate) remeasured_non_empty_cell_count: usize,
    pub(crate) remeasured_with_coordinated_extents_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct RetargetTrace {
    pub(crate) node_results: Vec<RetargetNodeTrace>,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationChildPlan {
    pub(crate) child_index: usize,
    pub(crate) old_plot_area_width: f32,
    pub(crate) old_plot_area_height: f32,
    pub(crate) target_plot_area_width: Option<f32>,
    pub(crate) target_plot_area_height: Option<f32>,
    pub(crate) target_band_range_end: Option<f32>,
    pub(crate) adjust_plot_area: bool,
    pub(crate) update_band_range: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationNodePlan {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) parent_cross_size_target: Option<f32>,
    pub(crate) child_count: usize,
    pub(crate) child_plans: Vec<FinalPropagationChildPlan>,
    pub(crate) expected_plot_area_adjustments_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPropagationPlan {
    pub(crate) node_plans: Vec<FinalPropagationNodePlan>,
}

#[derive(Debug, Clone)]
pub(crate) struct FinalPropagationNodeTrace {
    pub(crate) node_id: CoordinationNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) planned_parent_cross_size_target: Option<f32>,
    pub(crate) planned_child_count: usize,
    pub(crate) planned_child_plan_count: usize,
    pub(crate) planned_plot_area_adjustments_count: usize,
    pub(crate) child_plot_area_adjustments_count: usize,
    pub(crate) scale_range_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct FinalPropagationTrace {
    pub(crate) node_results: Vec<FinalPropagationNodeTrace>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordinationRunArtifacts {
    pub(crate) initial_requirement_pass: InitialRequirementPass,
    pub(crate) retarget_trace: RetargetTrace,
    pub(crate) retargeted_requirement_pass: RetargetedRequirementPass,
    pub(crate) final_propagation_trace: FinalPropagationTrace,
}

fn merge_overflow_groups(
    overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>>,
) -> HashMap<CoordinationGroupKey, CoordinatedOverflow> {
    overflow_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedOverflow::default();
            for v in values {
                merged.merge(&v);
            }
            (key, merged)
        })
        .collect()
}

fn merge_layout_groups(
    layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>>,
) -> HashMap<CoordinationGroupKey, CoordinatedLayout> {
    layout_by_key
        .into_iter()
        .map(|(key, values)| {
            let mut merged = CoordinatedLayout::default();
            for v in values {
                merged.merge(&v);
            }
            (key, merged)
        })
        .collect()
}

fn aggregate_domain_extents(
    infos: &[CellDomainInfo],
) -> HashMap<(String, Vec<ScalarValue>), DomainExtent> {
    let mut groups: HashMap<(String, Vec<ScalarValue>), Vec<&DomainExtent>> = HashMap::new();

    for info in infos {
        let ancestor_key = sharing_policy::domain_group_key(
            &info.full_cell_path,
            SharingLevel::from_raw(info.domain_sharing_level),
            info.facet_depth,
        );
        groups
            .entry((info.channel.clone(), ancestor_key))
            .or_default()
            .push(&info.extent);
    }

    groups
        .into_iter()
        .map(|(key, extents)| {
            let unified = extents
                .into_iter()
                .fold(None, |acc: Option<DomainExtent>, extent| match acc {
                    None => Some(extent.clone()),
                    Some(acc) => Some(union_domain_extents(&acc, extent)),
                })
                .unwrap_or_else(|| DomainExtent::numeric(0.0, 0.0));
            (key, unified)
        })
        .collect()
}

#[derive(Debug, Clone)]
struct RoundCollectionInput {
    node_id: CoordinationNodeKey,
    key: CoordinationGroupKey,
    local_overflow: Option<CoordinatedOverflow>,
    local_layout: CoordinatedLayout,
}

#[derive(Debug, Clone, Default)]
struct RoundCollectionOutput {
    merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
    overflow_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedOverflow>,
    layout_patches_by_node: HashMap<CoordinationNodeKey, CoordinatedLayout>,
    unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
    domain_target_nodes: HashSet<CoordinationNodeKey>,
}

fn build_round_collection(
    nodes: &[RoundCollectionInput],
    include_domains: bool,
    domain_infos: &[CellDomainInfo],
) -> RoundCollectionOutput {
    let mut overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();

    for node in nodes {
        if let Some(local_overflow) = node.local_overflow.clone() {
            overflow_by_key
                .entry(node.key.clone())
                .or_default()
                .push(local_overflow);
        }
        layout_by_key
            .entry(node.key.clone())
            .or_default()
            .push(node.local_layout.clone());
    }

    let merged_overflow_by_key = merge_overflow_groups(overflow_by_key);
    let merged_layout_by_key = merge_layout_groups(layout_by_key);
    let unified_domain_extents = if include_domains && !domain_infos.is_empty() {
        aggregate_domain_extents(domain_infos)
    } else {
        HashMap::new()
    };

    let mut overflow_patches_by_node = HashMap::new();
    let mut layout_patches_by_node = HashMap::new();
    let mut domain_target_nodes = HashSet::new();

    for node in nodes {
        if let Some(merged) = merged_overflow_by_key.get(&node.key).cloned() {
            overflow_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if let Some(merged) = merged_layout_by_key.get(&node.key).cloned() {
            layout_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if include_domains && !unified_domain_extents.is_empty() {
            domain_target_nodes.insert(node.node_id.clone());
        }
    }

    RoundCollectionOutput {
        merged_overflow_by_key,
        merged_layout_by_key,
        overflow_patches_by_node,
        layout_patches_by_node,
        unified_domain_extents,
        domain_target_nodes,
    }
}

pub(crate) fn build_initial_requirement_pass(
    snapshot: InitialRequirementSnapshot,
) -> InitialRequirementPass {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| RoundCollectionInput {
            node_id: node.node_id.clone(),
            key: node.key.clone(),
            local_overflow: node.local_overflow.clone(),
            local_layout: node.local_layout.clone(),
        })
        .collect::<Vec<_>>();
    let domain_infos = snapshot
        .nodes
        .iter()
        .flat_map(|node| node.domain_infos.iter().cloned())
        .collect::<Vec<_>>();
    let round = build_round_collection(&nodes, true, &domain_infos);

    InitialRequirementPass {
        snapshot,
        aggregates: InitialRequirementAggregates {
            merged_overflow_by_key: round.merged_overflow_by_key,
            merged_layout_by_key: round.merged_layout_by_key,
            unified_domain_extents: round.unified_domain_extents.clone(),
        },
        distribution: InitialRequirementDistributionPlan {
            overflow_patches_by_node: round.overflow_patches_by_node,
            layout_patches_by_node: round.layout_patches_by_node,
            domain_target_nodes: round.domain_target_nodes,
            unified_domain_extents: round.unified_domain_extents,
        },
    }
}

pub(crate) fn build_retargeted_requirement_pass(
    snapshot: RetargetedRequirementSnapshot,
) -> RetargetedRequirementPass {
    let nodes = snapshot
        .nodes
        .iter()
        .map(|node| RoundCollectionInput {
            node_id: node.node_id.clone(),
            key: node.key.clone(),
            local_overflow: node.local_overflow.clone(),
            local_layout: node.local_layout.clone(),
        })
        .collect::<Vec<_>>();
    let round = build_round_collection(&nodes, false, &[]);

    RetargetedRequirementPass {
        snapshot,
        aggregates: RetargetedRequirementAggregates {
            merged_overflow_by_key: round.merged_overflow_by_key,
            merged_layout_by_key: round.merged_layout_by_key,
        },
        distribution: RetargetedRequirementDistributionPlan {
            overflow_patches_by_node: round.overflow_patches_by_node,
            layout_patches_by_node: round.layout_patches_by_node,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        coords::OverflowSpaceRequirement, facet::sharing_policy,
        scales::domain_extent::DomainExtent,
    };

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    fn overflow(top: f32, right: f32, bottom: f32, left: f32) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top,
                right,
                bottom,
                left,
            },
            total: OverflowSpaceRequirement {
                top,
                right,
                bottom,
                left,
            },
        }
    }

    #[test]
    fn initial_requirement_pass_groups_and_distributes_overflow_layout_domains() {
        let key = CoordinationGroupKey::new(1, "col:group");
        let snapshot = InitialRequirementSnapshot {
            nodes: vec![
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    local_overflow: Some(overflow(1.0, 2.0, 3.0, 4.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 2.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                    domain_infos: vec![CellDomainInfo {
                        full_cell_path: vec![s("A")],
                        channel: "x".to_string(),
                        domain_sharing_level: 255,
                        facet_depth: 1,
                        extent: DomainExtent::numeric(0.0, 10.0),
                    }],
                },
                InitialRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key: key.clone(),
                    local_overflow: Some(overflow(3.0, 1.0, 5.0, 2.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 4.0,
                        outer_start: 2.0,
                        outer_end: 1.0,
                        n: 4,
                    },
                    domain_infos: vec![CellDomainInfo {
                        full_cell_path: vec![s("B")],
                        channel: "x".to_string(),
                        domain_sharing_level: 255,
                        facet_depth: 1,
                        extent: DomainExtent::numeric(2.0, 14.0),
                    }],
                },
            ],
        };

        let pass = build_initial_requirement_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.aggregates.unified_domain_extents.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.domain_target_nodes.len(), 2);
    }

    #[test]
    fn initial_requirement_pass_domain_grouping_respects_sharing_keys() {
        let key = CoordinationGroupKey::new(2, "col:shared");
        let info_a = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("X")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 1.0),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("Y")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 2.0),
        };
        let info_c = CellDomainInfo {
            full_cell_path: vec![s("A"), s("Z"), s("Q")],
            channel: "x".to_string(),
            domain_sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(5.0, 7.0),
        };

        let snapshot = InitialRequirementSnapshot {
            nodes: vec![InitialRequirementNodeSnapshot {
                node_id: CoordinationNodeKey::new(vec![0]),
                key,
                local_overflow: None,
                local_layout: CoordinatedLayout::default(),
                domain_infos: vec![info_a.clone(), info_b.clone(), info_c.clone()],
            }],
        };
        let pass = build_initial_requirement_pass(snapshot);

        let key_ab = sharing_policy::domain_group_key(
            &info_a.full_cell_path,
            SharingLevel::from_raw(info_a.domain_sharing_level),
            info_a.facet_depth,
        );
        let key_c = sharing_policy::domain_group_key(
            &info_c.full_cell_path,
            SharingLevel::from_raw(info_c.domain_sharing_level),
            info_c.facet_depth,
        );
        assert_eq!(pass.aggregates.unified_domain_extents.len(), 2);
        assert!(
            pass.aggregates
                .unified_domain_extents
                .contains_key(&("x".to_string(), key_ab))
        );
        assert!(
            pass.aggregates
                .unified_domain_extents
                .contains_key(&("x".to_string(), key_c))
        );
    }

    #[test]
    fn retargeted_requirement_pass_reconciles_overflow_layout_without_domains() {
        let key = CoordinationGroupKey::new(1, "row:group");
        let snapshot = RetargetedRequirementSnapshot {
            nodes: vec![
                RetargetedRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![0]),
                    key: key.clone(),
                    local_overflow: Some(overflow(1.0, 1.0, 2.0, 3.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 3.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                },
                RetargetedRequirementNodeSnapshot {
                    node_id: CoordinationNodeKey::new(vec![1]),
                    key,
                    local_overflow: Some(overflow(2.0, 4.0, 1.0, 1.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 5.0,
                        outer_start: 2.0,
                        outer_end: 1.0,
                        n: 5,
                    },
                },
            ],
        };

        let pass = build_retargeted_requirement_pass(snapshot);
        assert_eq!(pass.snapshot.nodes.len(), 2);
        assert_eq!(pass.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(pass.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(pass.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(pass.distribution.layout_patches_by_node.len(), 2);
    }
}
