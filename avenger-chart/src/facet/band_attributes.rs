use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{FacetEmptyCellPolicy, SharingLevel};
use avenger_scales::scales::ConfiguredScale;
use datafusion::{common::ScalarValue, dataframe::DataFrame, logical_expr::lit};

use crate::{
    coords::{FacetAxis, OverflowSpaceRequirement},
    error::AvengerChartError,
    facet::{
        coord::{ChannelDomainExtent, FacetBandNestedMeasureContext},
        evaluated_facet_tree::EvaluatedFacetTree,
        layout_plan::{FacetBandPlan, FacetCellEmptyKind},
        probe_summary::FacetCellProbeSummary,
        scale_precompute::{FacetScaleNodeArtifacts, FacetScaleNodeKey},
    },
    partition::PartitionCellPlan,
    plot::compiled::{CompiledPlot, ComponentsMeasurement},
    render::EvaluationContext,
    scales::domain_extent::DomainExtent,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FacetBandNodeKey {
    pub(crate) axis: FacetAxis,
    pub(crate) facet_path: Vec<ScalarValue>,
}

pub(crate) type FacetBandCellSemantic = PartitionCellPlan;

#[derive(Clone, Debug)]
pub(crate) struct FacetBandSemantics {
    pub(crate) node_id: FacetBandNodeKey,
    pub(crate) facet_depth: u8,
    pub(crate) coordination_field_identity: String,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) cell_values: Vec<ScalarValue>,
    pub(crate) cells: Vec<FacetBandCellSemantic>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPreparedInputs {
    pub(crate) cell_semantics: FacetBandSemantics,
    pub(crate) renderable_mask: Vec<bool>,
    pub(crate) scale_artifacts_key: FacetScaleNodeKey,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OverflowProbeSummary {
    pub(crate) cell_overflows: Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>,
    pub(crate) cell_probe_summaries: Vec<FacetCellProbeSummary>,
    pub(crate) max_child_padding: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandOverflowProbe {
    pub(crate) prepared_inputs: FacetBandPreparedInputs,
    pub(crate) overflow_probe_summary: OverflowProbeSummary,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandLocalLayout {
    pub(crate) overflow_probe: FacetBandOverflowProbe,
    pub(crate) band_layout_plan: FacetBandPlan,
    pub(crate) final_subplot_cross_size: f32,
}

pub(crate) struct FacetBandPreparedRuntime {
    pub(crate) data_overrides: Vec<DataFrame>,
    pub(crate) subplot_eval_ctx: EvaluationContext,
    pub(crate) nested_measure_ctx: FacetBandNestedMeasureContext,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) original_band_scale: ConfiguredScale,
    pub(crate) initial_subplot_band_size: f32,
    pub(crate) scale_artifacts: Arc<FacetScaleNodeArtifacts>,
}

pub(crate) struct FacetBandMeasuredRuntime {
    pub(crate) measurements: Vec<ComponentsMeasurement>,
    pub(crate) local_domain_extents: Vec<HashMap<String, ChannelDomainExtent>>,
    pub(crate) coordinated_domain_extents: Vec<HashMap<String, DomainExtent>>,
}

impl FacetBandSemantics {
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
                    facet_tree.cell_predicate(&full_path, SharingLevel::FREE.raw())
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
            node_id: FacetBandNodeKey {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use indexmap::IndexMap;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    #[test]
    fn build_cell_semantics_preserves_semantics_and_order() -> Result<(), AvengerChartError> {
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
        let cell_semantics = FacetBandSemantics::from_tree_and_values(
            FacetAxis::Column,
            &[],
            1,
            "k".to_string(),
            FacetEmptyCellPolicy::Hole,
            &tree,
            &cell_values,
        )?;

        assert_eq!(cell_semantics.cells.len(), 2);
        assert_eq!(cell_semantics.cells[0].value, s("A"));
        assert_eq!(cell_semantics.cells[1].value, s("B"));
        assert!(cell_semantics.cells.iter().all(|cell| cell.in_domain_slot));
        Ok(())
    }
}
