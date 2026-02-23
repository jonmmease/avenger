use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, logical_expr::lit,
};

use crate::{
    coords::{FacetAxis, OverflowSpaceRequirement},
    error::AvengerChartError,
    facet::{
        coord::{ChannelDomainExtent, FacetBandCoordMeasurement, FacetBandNestedMeasureContext},
        empty_cell_policy::FacetEmptyCellPolicy,
        evaluated_facet_tree::EvaluatedFacetTree,
        layout_plan::{FacetBandPlan, FacetCellEmptyKind},
        scale_precompute::{FacetScaleNodeArtifacts, FacetScaleNodeKey},
    },
    plot::compiled::{CompiledPlot, ComponentsMeasurement},
    render::EvaluationContext,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FacetBandNodeId {
    pub(crate) axis: FacetAxis,
    pub(crate) facet_path: Vec<ScalarValue>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandCellSemantic {
    pub(crate) value: ScalarValue,
    pub(crate) full_path: Vec<ScalarValue>,
    pub(crate) in_domain_slot: bool,
    pub(crate) has_data_rows: bool,
    pub(crate) empty_kind: FacetCellEmptyKind,
    pub(crate) is_empty: bool,
    pub(crate) filter_predicate: Option<Expr>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPhase3Ir {
    pub(crate) node_id: FacetBandNodeId,
    pub(crate) facet_depth: u8,
    pub(crate) coordination_field_identity: String,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) cell_values: Vec<ScalarValue>,
    pub(crate) cells: Vec<FacetBandCellSemantic>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPhase4Ir {
    pub(crate) phase3: FacetBandPhase3Ir,
    pub(crate) renderable_mask: Vec<bool>,
    pub(crate) scale_artifacts_key: FacetScaleNodeKey,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OverflowProbeSummary {
    pub(crate) cell_overflows: Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>,
    pub(crate) max_child_padding: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPhase5Ir {
    pub(crate) phase4: FacetBandPhase4Ir,
    pub(crate) overflow_probe_summary: OverflowProbeSummary,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPhase6Ir {
    pub(crate) phase5: FacetBandPhase5Ir,
    pub(crate) band_layout_plan: FacetBandPlan,
    pub(crate) final_subplot_cross_size: f32,
    pub(crate) channel_sharing_levels: HashMap<String, u8>,
}

pub(crate) struct FacetBandPhase4Sidecars {
    pub(crate) data_overrides: Vec<DataFrame>,
    pub(crate) subplot_eval_ctx: EvaluationContext,
    pub(crate) nested_measure_ctx: FacetBandNestedMeasureContext,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) original_band_scale: ConfiguredScale,
    pub(crate) initial_subplot_band_size: f32,
    pub(crate) scale_artifacts: Arc<FacetScaleNodeArtifacts>,
}

pub(crate) struct FacetBandPhase6Sidecars {
    pub(crate) measurements: Vec<ComponentsMeasurement>,
    pub(crate) local_domain_extents: Vec<HashMap<String, ChannelDomainExtent>>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FacetBandIrParityReport {
    pub(crate) compared_cells: usize,
}

impl FacetBandPhase3Ir {
    pub(crate) fn from_tree_and_values(
        axis: FacetAxis,
        facet_path: &[ScalarValue],
        facet_depth: u8,
        coordination_field_identity: String,
        empty_cell_policy: FacetEmptyCellPolicy,
        facet_tree: &EvaluatedFacetTree,
        cell_values: &[ScalarValue],
    ) -> Result<Self, AvengerChartError> {
        let cells = cell_values
            .iter()
            .map(|value| {
                let mut full_path = facet_path.to_vec();
                full_path.push(value.clone());

                let in_domain_slot = facet_tree.cell_exists(&full_path);
                let has_data_rows = if in_domain_slot {
                    facet_tree.cell_has_data(&full_path)
                } else {
                    false
                };
                let is_empty = !has_data_rows;
                let empty_kind = if !in_domain_slot {
                    FacetCellEmptyKind::DomainPlaceholder
                } else {
                    FacetCellEmptyKind::DataEmpty
                };
                let filter_predicate = if in_domain_slot {
                    facet_tree.cell_predicate(&full_path, 0)
                } else {
                    Some(lit(false))
                };

                Ok(FacetBandCellSemantic {
                    value: value.clone(),
                    full_path,
                    in_domain_slot,
                    has_data_rows,
                    empty_kind,
                    is_empty,
                    filter_predicate,
                })
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;

        Ok(Self {
            node_id: FacetBandNodeId {
                axis,
                facet_path: facet_path.to_vec(),
            },
            facet_depth,
            coordination_field_identity,
            empty_cell_policy,
            cell_values: cell_values.to_vec(),
            cells,
        })
    }
}

fn approx_eq(lhs: f32, rhs: f32, eps: f32) -> bool {
    (lhs - rhs).abs() <= eps
}

pub(crate) fn assert_ir_legacy_parity(
    ir_measurement: &FacetBandCoordMeasurement,
    legacy_measurement: &FacetBandCoordMeasurement,
) -> FacetBandIrParityReport {
    const EPS: f32 = 0.01;
    assert_eq!(ir_measurement.axis, legacy_measurement.axis);
    assert_eq!(
        ir_measurement.coordination_field_identity,
        legacy_measurement.coordination_field_identity
    );
    assert_eq!(ir_measurement.facet_depth, legacy_measurement.facet_depth);
    assert_eq!(
        ir_measurement.empty_cell_policy,
        legacy_measurement.empty_cell_policy
    );
    assert_eq!(ir_measurement.cells.len(), legacy_measurement.cells.len());
    assert_eq!(
        ir_measurement.channel_sharing_levels,
        legacy_measurement.channel_sharing_levels
    );

    assert!(approx_eq(
        ir_measurement.local_layout.padding_inner_px,
        legacy_measurement.local_layout.padding_inner_px,
        EPS
    ));
    assert!(approx_eq(
        ir_measurement.local_layout.outer_start,
        legacy_measurement.local_layout.outer_start,
        EPS
    ));
    assert!(approx_eq(
        ir_measurement.local_layout.outer_end,
        legacy_measurement.local_layout.outer_end,
        EPS
    ));
    assert_eq!(
        ir_measurement.local_layout.n,
        legacy_measurement.local_layout.n
    );
    assert!(approx_eq(
        ir_measurement.subplot_cross_size,
        legacy_measurement.subplot_cross_size,
        EPS
    ));

    for (ir_cell, legacy_cell) in ir_measurement
        .cells
        .iter()
        .zip(legacy_measurement.cells.iter())
    {
        assert_eq!(ir_cell.plan.value, legacy_cell.plan.value);
        assert_eq!(ir_cell.plan.full_path, legacy_cell.plan.full_path);
        assert_eq!(ir_cell.plan.in_domain_slot, legacy_cell.plan.in_domain_slot);
        assert_eq!(ir_cell.plan.has_data_rows, legacy_cell.plan.has_data_rows);
        assert_eq!(ir_cell.plan.empty_kind, legacy_cell.plan.empty_kind);
        assert_eq!(ir_cell.plan.is_empty, legacy_cell.plan.is_empty);
        assert_eq!(
            ir_cell.local_domain_extents.len(),
            legacy_cell.local_domain_extents.len()
        );

        assert!(approx_eq(
            ir_cell.measurement.plot_area_width,
            legacy_cell.measurement.plot_area_width,
            EPS
        ));
        assert!(approx_eq(
            ir_cell.measurement.plot_area_height,
            legacy_cell.measurement.plot_area_height,
            EPS
        ));
        assert!(approx_eq(
            ir_cell.measurement.layout.overflow.top,
            legacy_cell.measurement.layout.overflow.top,
            EPS
        ));
        assert!(approx_eq(
            ir_cell.measurement.layout.overflow.right,
            legacy_cell.measurement.layout.overflow.right,
            EPS
        ));
        assert!(approx_eq(
            ir_cell.measurement.layout.overflow.bottom,
            legacy_cell.measurement.layout.overflow.bottom,
            EPS
        ));
        assert!(approx_eq(
            ir_cell.measurement.layout.overflow.left,
            legacy_cell.measurement.layout.overflow.left,
            EPS
        ));
    }

    FacetBandIrParityReport {
        compared_cells: ir_measurement.cells.len(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::guide::FacetDirection;
    use indexmap::IndexMap;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    #[test]
    fn phase3_ir_preserves_cell_semantics_and_order() -> Result<(), AvengerChartError> {
        let mut children = IndexMap::new();
        children.insert(
            s("A"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Column,
                255,
                "k".to_string(),
                None,
                vec![s("A")],
            )),
        );
        children.insert(
            s("B"),
            Box::new(PartitionNode::leaf(
                FacetDirection::Column,
                255,
                "k".to_string(),
                None,
                vec![s("B")],
            )),
        );
        let tree = EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            255,
            "k".to_string(),
            None,
            children,
        )));

        let cell_values = vec![s("A"), s("B")];
        let phase3 = FacetBandPhase3Ir::from_tree_and_values(
            FacetAxis::Column,
            &[],
            1,
            "k".to_string(),
            FacetEmptyCellPolicy::Hole,
            &tree,
            &cell_values,
        )?;

        assert_eq!(phase3.cells.len(), 2);
        assert_eq!(phase3.cells[0].value, s("A"));
        assert_eq!(phase3.cells[1].value, s("B"));
        assert!(phase3.cells.iter().all(|cell| cell.in_domain_slot));
        Ok(())
    }
}
