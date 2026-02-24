use std::collections::{HashMap, HashSet};

use datafusion::common::ScalarValue;

use crate::{
    coords::{CellDomainInfo, CoordinatedLayout, CoordinatedOverflow, FacetAxis},
    facet::{
        coord::{FacetBandCoordApplyPlan, union_domain_extents},
        coordination::CoordinationGroupKey,
        coordination_remeasure::FacetCoordRemeasurePlan,
        sharing_level::SharingLevel,
        sharing_policy,
    },
    scales::domain_extent::DomainExtent,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct CoordNodeKey {
    pub(crate) path: Vec<usize>,
}

impl CoordNodeKey {
    pub(crate) fn new(path: Vec<usize>) -> Self {
        Self { path }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CoordGroupNodeSnapshot {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) local_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
    pub(crate) domain_infos: Vec<CellDomainInfo>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordGroupSnapshot {
    pub(crate) nodes: Vec<CoordGroupNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordGroupAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
    pub(crate) unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordGroupDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordNodeKey, CoordinatedLayout>,
    pub(crate) domain_target_nodes: HashSet<CoordNodeKey>,
    pub(crate) unified_domain_extents: HashMap<(String, Vec<ScalarValue>), DomainExtent>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordGroupDistribution {
    pub(crate) snapshot: CoordGroupSnapshot,
    pub(crate) aggregates: CoordGroupAggregates,
    pub(crate) distribution: CoordGroupDistributionPlan,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordReconcileNodeSnapshot {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) key: CoordinationGroupKey,
    pub(crate) local_overflow: Option<CoordinatedOverflow>,
    pub(crate) local_layout: CoordinatedLayout,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordReconcileSnapshot {
    pub(crate) nodes: Vec<CoordReconcileNodeSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordReconcileAggregates {
    pub(crate) merged_overflow_by_key: HashMap<CoordinationGroupKey, CoordinatedOverflow>,
    pub(crate) merged_layout_by_key: HashMap<CoordinationGroupKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordReconcileDistributionPlan {
    pub(crate) overflow_patches_by_node: HashMap<CoordNodeKey, CoordinatedOverflow>,
    pub(crate) layout_patches_by_node: HashMap<CoordNodeKey, CoordinatedLayout>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordReconcileDistribution {
    pub(crate) snapshot: CoordReconcileSnapshot,
    pub(crate) aggregates: CoordReconcileAggregates,
    pub(crate) distribution: CoordReconcileDistributionPlan,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordApplyNodeIntent {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) apply_plan: FacetBandCoordApplyPlan,
    pub(crate) remeasure_plan: Option<FacetCoordRemeasurePlan>,
    pub(crate) has_legend_overflow: bool,
    pub(crate) has_coordinated_extents: bool,
    pub(crate) remeasure_triggered: bool,
    pub(crate) child_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordApplyIntent {
    pub(crate) node_derivations: Vec<CoordApplyNodeIntent>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordApplyNodeOutcome {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) derived_has_legend_overflow: bool,
    pub(crate) derived_has_coordinated_extents: bool,
    pub(crate) derived_remeasure_required: bool,
    pub(crate) derived_axis_owner_ignore_empty_cells: bool,
    pub(crate) derived_adjusted_main_size: f32,
    pub(crate) derived_child_count: usize,
    pub(crate) parent_cross_size_propagated: bool,
    pub(crate) subplot_cross_size_before: f32,
    pub(crate) subplot_cross_size_after: f32,
    pub(crate) remeasure_triggered: bool,
    pub(crate) remeasured_cell_count: usize,
    pub(crate) remeasured_non_empty_cell_count: usize,
    pub(crate) remeasured_with_coordinated_extents_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordApplyTrace {
    pub(crate) node_results: Vec<CoordApplyNodeOutcome>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordRetargetChildIntent {
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
pub(crate) struct CoordRetargetNodeIntent {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) parent_cross_size_target: Option<f32>,
    pub(crate) child_count: usize,
    pub(crate) child_intents: Vec<CoordRetargetChildIntent>,
    pub(crate) expected_plot_area_adjustments_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordRetargetIntent {
    pub(crate) node_derivations: Vec<CoordRetargetNodeIntent>,
}

#[derive(Debug, Clone)]
pub(crate) struct CoordRetargetNodeOutcome {
    pub(crate) node_id: CoordNodeKey,
    pub(crate) axis: FacetAxis,
    pub(crate) derived_parent_cross_size_target: Option<f32>,
    pub(crate) derived_child_count: usize,
    pub(crate) derived_child_intent_count: usize,
    pub(crate) derived_expected_plot_area_adjustments_count: usize,
    pub(crate) child_plot_area_adjustments_count: usize,
    pub(crate) scale_range_retarget_count: usize,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordRetargetTrace {
    pub(crate) node_results: Vec<CoordRetargetNodeOutcome>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CoordinationRunArtifacts {
    pub(crate) phase7: CoordGroupDistribution,
    pub(crate) phase8: CoordApplyTrace,
    pub(crate) phase9: CoordReconcileDistribution,
    pub(crate) phase10: CoordRetargetTrace,
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
            SharingLevel::from_raw(info.sharing_level),
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

pub(crate) fn build_phase7_ir(snapshot: CoordGroupSnapshot) -> CoordGroupDistribution {
    let mut overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();
    let mut domain_infos: Vec<CellDomainInfo> = Vec::new();

    for node in &snapshot.nodes {
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
        domain_infos.extend(node.domain_infos.iter().cloned());
    }

    let merged_overflow_by_key = merge_overflow_groups(overflow_by_key);
    let merged_layout_by_key = merge_layout_groups(layout_by_key);
    let unified_domain_extents = if domain_infos.is_empty() {
        HashMap::new()
    } else {
        aggregate_domain_extents(&domain_infos)
    };

    let mut overflow_patches_by_node = HashMap::new();
    let mut layout_patches_by_node = HashMap::new();
    let mut domain_target_nodes = HashSet::new();

    for node in &snapshot.nodes {
        if let Some(merged) = merged_overflow_by_key.get(&node.key).cloned() {
            overflow_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if let Some(merged) = merged_layout_by_key.get(&node.key).cloned() {
            layout_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if !unified_domain_extents.is_empty() {
            domain_target_nodes.insert(node.node_id.clone());
        }
    }

    CoordGroupDistribution {
        snapshot,
        aggregates: CoordGroupAggregates {
            merged_overflow_by_key,
            merged_layout_by_key,
            unified_domain_extents: unified_domain_extents.clone(),
        },
        distribution: CoordGroupDistributionPlan {
            overflow_patches_by_node,
            layout_patches_by_node,
            domain_target_nodes,
            unified_domain_extents,
        },
    }
}

pub(crate) fn build_phase9_ir(snapshot: CoordReconcileSnapshot) -> CoordReconcileDistribution {
    let mut overflow_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedOverflow>> =
        HashMap::new();
    let mut layout_by_key: HashMap<CoordinationGroupKey, Vec<CoordinatedLayout>> = HashMap::new();

    for node in &snapshot.nodes {
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

    let mut overflow_patches_by_node = HashMap::new();
    let mut layout_patches_by_node = HashMap::new();
    for node in &snapshot.nodes {
        if let Some(merged) = merged_overflow_by_key.get(&node.key).cloned() {
            overflow_patches_by_node.insert(node.node_id.clone(), merged);
        }
        if let Some(merged) = merged_layout_by_key.get(&node.key).cloned() {
            layout_patches_by_node.insert(node.node_id.clone(), merged);
        }
    }

    CoordReconcileDistribution {
        snapshot,
        aggregates: CoordReconcileAggregates {
            merged_overflow_by_key,
            merged_layout_by_key,
        },
        distribution: CoordReconcileDistributionPlan {
            overflow_patches_by_node,
            layout_patches_by_node,
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
    fn phase7_ir_groups_and_distributes_overflow_layout_domains() {
        let key = CoordinationGroupKey::new(1, "col:group");
        let snapshot = CoordGroupSnapshot {
            nodes: vec![
                CoordGroupNodeSnapshot {
                    node_id: CoordNodeKey::new(vec![0]),
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
                        sharing_level: 255,
                        facet_depth: 1,
                        extent: DomainExtent::numeric(0.0, 10.0),
                    }],
                },
                CoordGroupNodeSnapshot {
                    node_id: CoordNodeKey::new(vec![1]),
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
                        sharing_level: 255,
                        facet_depth: 1,
                        extent: DomainExtent::numeric(2.0, 14.0),
                    }],
                },
            ],
        };

        let ir = build_phase7_ir(snapshot);
        assert_eq!(ir.snapshot.nodes.len(), 2);
        assert_eq!(ir.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(ir.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(ir.aggregates.unified_domain_extents.len(), 1);
        assert_eq!(ir.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(ir.distribution.layout_patches_by_node.len(), 2);
        assert_eq!(ir.distribution.domain_target_nodes.len(), 2);
    }

    #[test]
    fn phase7_ir_domain_grouping_respects_sharing_keys() {
        let key = CoordinationGroupKey::new(2, "col:shared");
        let info_a = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("X")],
            channel: "x".to_string(),
            sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 1.0),
        };
        let info_b = CellDomainInfo {
            full_cell_path: vec![s("A"), s("B"), s("Y")],
            channel: "x".to_string(),
            sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(0.0, 2.0),
        };
        let info_c = CellDomainInfo {
            full_cell_path: vec![s("A"), s("Z"), s("Q")],
            channel: "x".to_string(),
            sharing_level: 1,
            facet_depth: 3,
            extent: DomainExtent::numeric(5.0, 7.0),
        };

        let snapshot = CoordGroupSnapshot {
            nodes: vec![CoordGroupNodeSnapshot {
                node_id: CoordNodeKey::new(vec![0]),
                key,
                local_overflow: None,
                local_layout: CoordinatedLayout::default(),
                domain_infos: vec![info_a.clone(), info_b.clone(), info_c.clone()],
            }],
        };
        let ir = build_phase7_ir(snapshot);

        let key_ab = sharing_policy::domain_group_key(
            &info_a.full_cell_path,
            SharingLevel::from_raw(info_a.sharing_level),
            info_a.facet_depth,
        );
        let key_c = sharing_policy::domain_group_key(
            &info_c.full_cell_path,
            SharingLevel::from_raw(info_c.sharing_level),
            info_c.facet_depth,
        );
        assert_eq!(ir.aggregates.unified_domain_extents.len(), 2);
        assert!(
            ir.aggregates
                .unified_domain_extents
                .contains_key(&("x".to_string(), key_ab))
        );
        assert!(
            ir.aggregates
                .unified_domain_extents
                .contains_key(&("x".to_string(), key_c))
        );
    }

    #[test]
    fn phase9_ir_reconciles_overflow_layout_without_domains() {
        let key = CoordinationGroupKey::new(1, "row:group");
        let snapshot = CoordReconcileSnapshot {
            nodes: vec![
                CoordReconcileNodeSnapshot {
                    node_id: CoordNodeKey::new(vec![0]),
                    key: key.clone(),
                    local_overflow: Some(overflow(1.0, 1.0, 2.0, 3.0)),
                    local_layout: CoordinatedLayout {
                        padding_inner_px: 3.0,
                        outer_start: 1.0,
                        outer_end: 2.0,
                        n: 2,
                    },
                },
                CoordReconcileNodeSnapshot {
                    node_id: CoordNodeKey::new(vec![1]),
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

        let ir = build_phase9_ir(snapshot);
        assert_eq!(ir.snapshot.nodes.len(), 2);
        assert_eq!(ir.aggregates.merged_overflow_by_key.len(), 1);
        assert_eq!(ir.aggregates.merged_layout_by_key.len(), 1);
        assert_eq!(ir.distribution.overflow_patches_by_node.len(), 2);
        assert_eq!(ir.distribution.layout_patches_by_node.len(), 2);
    }
}
