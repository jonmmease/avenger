//! FacetRowGuide implementation for row-based faceting.
//!
//! This module is a thin wrapper around the shared facet-band guide engine.

use crate::cartesian::axis::CartesianAxis;
use crate::error::AvengerChartError;
use crate::facet::guide::band_guide_engine::{self, FacetGuideState, RowGuideAxisOps};
use crate::facet::marks::facet::CompiledFacetRowSubplot;
use crate::facet::overflow_projection::FacetOverflowResolutionPhase;
use crate::guide::{
    CompiledGuide, CoordinateGuide, GuideOverflowPhase, GuideSharingContext, MeasurementResult,
    OverflowSpaceRequirement,
};
use crate::layout::LayoutBounds;
use crate::marks::CompiledMark;
use crate::plot::compiled::CompiledPlot;
use crate::plot::compiled::SharingLevel;
use crate::serialization::SerializableDataFrame;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::Clip;
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::prelude::SessionContext;
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
use crate::coords::CoordinatedOverflow;
#[cfg(test)]
use crate::facet::guide::band_guide_engine::GuideAnchorSource;

/// Guide configuration for FacetRow coordinate system
#[derive(Clone, Default)]
pub struct FacetRowGuideConfig {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot extracted from the facet subplot mark
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet subplot mark's data (used when data_override is None)
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("left" or "right")
    position: Option<String>,
    /// Slot-sharing level for the row facet variable (0=Free, N=Level(N), 255=Shared)
    sharing_level: u8,
}

impl FacetRowGuideConfig {
    /// Create a new FacetRowGuideConfig
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the facet title
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.facet_title = title;
        self
    }
}

impl CoordinateGuide for FacetRowGuideConfig {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // Facet guides do not compose cartesian axes directly.
    }

    fn set_compiled_marks(
        &mut self,
        compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
        for mark in &compiled_marks {
            if let Some(facet_row) = mark.as_any().downcast_ref::<CompiledFacetRowSubplot>() {
                self.compiled_subplot = Some(facet_row.compiled_subplot().clone());
                self.facet_data_plan = mark.data_context().logical_plan_node().cloned();
                self.facet_title = facet_row.facet_title().map(|s| s.to_string());
                self.position = facet_row.facet_position().map(|s| s.to_string());
                self.sharing_level = facet_row
                    .facet_slot_sharing()
                    .map(|sharing| sharing.to_level())
                    .unwrap_or(0);
                break;
            }
        }
    }

    fn update(&mut self, _other: Self) {
        // No-op.
    }

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(FacetRowGuide {
            facet_title: self.facet_title,
            compiled_subplot: self.compiled_subplot,
            facet_data_plan: self.facet_data_plan,
            position: self.position,
            sharing_level: self.sharing_level,
        })
    }
}

/// Compiled guide for FacetRow coordinate system
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetRowGuide {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot for measuring overflow
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet subplot mark's data (used when data_override is None)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("left" or "right")
    position: Option<String>,
    /// Slot-sharing level for the row facet variable (0=Free, N=Level(N), 255=Shared)
    #[serde(default)]
    sharing_level: u8,
}

#[cfg(test)]
fn resolve_guide_anchor_overflow_horizontal(
    place_at_right: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    band_guide_engine::resolve_row_guide_anchor_overflow_horizontal(
        place_at_right,
        coordinated_overflow,
        local_overflow,
    )
}

#[cfg(test)]
fn propagated_subplot_overflow(
    local_overflow: Option<CoordinatedOverflow>,
) -> OverflowSpaceRequirement {
    band_guide_engine::propagated_subplot_overflow(local_overflow)
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for FacetRowGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        self.measure_overflow_for_phase(
            scales,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            sharing_context,
            coord_measurement,
            GuideOverflowPhase::Measurement,
        )
        .await
    }

    async fn measure_overflow_for_phase(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
        phase: GuideOverflowPhase,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let phase = match phase {
            GuideOverflowPhase::Measurement => FacetOverflowResolutionPhase::Measurement,
            GuideOverflowPhase::Final => FacetOverflowResolutionPhase::Final,
        };
        band_guide_engine::measure_overflow_common::<RowGuideAxisOps>(
            &self.as_engine_state(),
            scales,
            plot_width,
            plot_height,
            theme,
            params,
            data_override,
            ctx,
            sharing_context,
            coord_measurement,
            phase,
        )
        .await
    }

    async fn measure_with_coordination(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &crate::theme::Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn crate::coords::CoordMeasurement>,
    ) -> Result<MeasurementResult, AvengerChartError> {
        Ok(MeasurementResult::default())
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        guide_overflow: &crate::guide::OverflowSpaceRequirement,
        theme: &crate::theme::Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn crate::coords::CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        band_guide_engine::evaluate_common::<RowGuideAxisOps>(
            &self.as_engine_state(),
            scales,
            plot_width,
            plot_height,
            plot_bounds,
            guide_overflow,
            theme,
            params,
            data_override,
            ctx,
            sharing_context,
            coord_measurement,
        )
        .await
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl FacetRowGuide {
    fn as_engine_state(&self) -> FacetGuideState {
        FacetGuideState {
            facet_title: self.facet_title.clone(),
            compiled_subplot: self.compiled_subplot.clone(),
            facet_data_plan: self.facet_data_plan.clone(),
            position: self.position.clone(),
            sharing_level: SharingLevel::from_raw(self.sharing_level),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_core::AxisPosition;
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::facet::guide_utils::facet_guide_labels_visible_for_cell;
    use indexmap::IndexMap;

    fn s(value: &str) -> datafusion::common::ScalarValue {
        datafusion::common::ScalarValue::Utf8(Some(value.to_string()))
    }

    fn column_then_row_tree() -> EvaluatedFacetTree {
        let make_leaf = || {
            PartitionNode::leaf(
                FacetDirection::Row,
                255,
                "species".to_string(),
                None,
                vec![s("setosa"), s("versicolor"), s("virginica")],
            )
        };

        let mut children = IndexMap::new();
        children.insert(s("short"), Box::new(make_leaf()));
        children.insert(s("medium"), Box::new(make_leaf()));
        children.insert(s("long"), Box::new(make_leaf()));

        EvaluatedFacetTree::new(Some(PartitionNode::branch(
            FacetDirection::Column,
            255,
            "length_bin".to_string(),
            None,
            children,
        )))
    }

    fn coordinated_overflow(
        guide_left: f32,
        guide_right: f32,
        total_left: f32,
        total_right: f32,
    ) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                left: guide_left,
                right: guide_right,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                left: total_left,
                right: total_right,
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolve_guide_anchor_right_uses_total_subtree() {
        let coordinated = coordinated_overflow(13.0, 41.0, 13.0, 55.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow_horizontal(true, Some(&coordinated), None);
        assert_eq!(resolved, 55.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_left_uses_total_subtree() {
        let coordinated = coordinated_overflow(7.0, 41.0, 55.0, 41.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow_horizontal(false, Some(&coordinated), None);
        assert_eq!(resolved, 55.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_falls_back_to_local_total_overflow() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let (resolved_right, source_right) =
            resolve_guide_anchor_overflow_horizontal(true, None, Some(&local));
        let (resolved_left, source_left) =
            resolve_guide_anchor_overflow_horizontal(false, None, Some(&local));
        assert_eq!(resolved_right, 40.0);
        assert_eq!(source_right, GuideAnchorSource::LocalGuide);
        assert_eq!(resolved_left, 60.0);
        assert_eq!(source_left, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn resolve_guide_anchor_prefers_coordinated_over_local_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(5.0, 39.0, 5.0, 39.0);
        let (resolved_right, source_right) =
            resolve_guide_anchor_overflow_horizontal(true, Some(&coordinated), Some(&local));
        let (resolved_left, source_left) =
            resolve_guide_anchor_overflow_horizontal(false, Some(&coordinated), Some(&local));
        assert_eq!(resolved_right, 39.0);
        assert_eq!(source_right, GuideAnchorSource::CoordinatedGuide);
        assert_eq!(resolved_left, 5.0);
        assert_eq!(source_left, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_defaults_to_zero_without_overflow_context() {
        let (resolved, source) = resolve_guide_anchor_overflow_horizontal(true, None, None);
        assert_eq!(resolved, 0.0);
        assert_eq!(source, GuideAnchorSource::DefaultZero);
    }

    #[test]
    fn propagated_subplot_overflow_uses_total_subtree() {
        let local = coordinated_overflow(11.0, 23.0, 49.0, 65.0);
        let propagated = propagated_subplot_overflow(Some(local));
        assert_eq!(propagated.left, 49.0);
        assert_eq!(propagated.right, 65.0);
    }

    #[test]
    fn shared_right_owner_visibility_only_far_right_column_shows_labels() {
        let tree = column_then_row_tree();
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Right,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Right,
            255
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Right,
            255
        ));
    }

    #[test]
    fn shared_left_owner_visibility_only_far_left_column_shows_labels() {
        let tree = column_then_row_tree();
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Left,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Left,
            255
        ));
    }

    #[test]
    fn free_sharing_visibility_shows_labels_in_all_columns() {
        let tree = column_then_row_tree();
        for path in ["short", "medium", "long"] {
            assert!(facet_guide_labels_visible_for_cell(
                &tree,
                &[s(path)],
                AxisPosition::Right,
                0
            ));
        }
    }
}
