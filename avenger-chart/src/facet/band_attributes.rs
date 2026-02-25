use std::{collections::HashMap, sync::Arc};

use avenger_scales::scales::ConfiguredScale;
use datafusion::{
    common::ScalarValue, dataframe::DataFrame, logical_expr::Expr, logical_expr::lit,
};

use crate::{
    coords::{FacetAxis, OverflowSpaceRequirement},
    error::AvengerChartError,
    facet::{
        coord::{ChannelDomainExtent, FacetBandNestedMeasureContext},
        empty_cell_policy::FacetEmptyCellPolicy,
        evaluated_facet_tree::EvaluatedFacetTree,
        layout_plan::{FacetBandPlan, FacetCellEmptyKind},
        scale_precompute::{FacetScaleNodeArtifacts, FacetScaleNodeKey},
        sharing_level::SharingLevel,
    },
    plot::compiled::{CompiledPlot, ComponentsMeasurement},
    render::EvaluationContext,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FacetBandNodeKey {
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
pub(crate) struct FacetBandSemantics {
    pub(crate) node_id: FacetBandNodeKey,
    pub(crate) facet_depth: u8,
    pub(crate) coordination_field_identity: String,
    pub(crate) empty_cell_policy: FacetEmptyCellPolicy,
    pub(crate) cell_values: Vec<ScalarValue>,
    pub(crate) cells: Vec<FacetBandCellSemantic>,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandPreparedSynthesis {
    pub(crate) cell_synthesis: FacetBandSemantics,
    pub(crate) renderable_mask: Vec<bool>,
    pub(crate) scale_artifacts_key: FacetScaleNodeKey,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct OverflowProbeSummary {
    pub(crate) cell_overflows: Vec<(OverflowSpaceRequirement, OverflowSpaceRequirement)>,
    pub(crate) max_child_padding: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandOverflowSynthesis {
    pub(crate) prepared_synthesis: FacetBandPreparedSynthesis,
    pub(crate) overflow_probe_summary: OverflowProbeSummary,
}

#[derive(Clone, Debug)]
pub(crate) struct FacetBandLocalSynthesis {
    pub(crate) overflow_synthesis: FacetBandOverflowSynthesis,
    pub(crate) band_layout_plan: FacetBandPlan,
    pub(crate) final_subplot_cross_size: f32,
    pub(crate) channel_sharing_levels: HashMap<String, SharingLevel>,
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
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::guide::FacetDirection;
    use indexmap::IndexMap;

    fn s(value: &str) -> ScalarValue {
        ScalarValue::Utf8(Some(value.to_string()))
    }

    #[test]
    fn synthesize_cell_attributes_preserves_semantics_and_order() -> Result<(), AvengerChartError> {
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
        let cell_synthesis = FacetBandSemantics::from_tree_and_values(
            FacetAxis::Column,
            &[],
            1,
            "k".to_string(),
            FacetEmptyCellPolicy::Hole,
            &tree,
            &cell_values,
        )?;

        assert_eq!(cell_synthesis.cells.len(), 2);
        assert_eq!(cell_synthesis.cells[0].value, s("A"));
        assert_eq!(cell_synthesis.cells[1].value, s("B"));
        assert!(cell_synthesis.cells.iter().all(|cell| cell.in_domain_slot));
        Ok(())
    }
}
