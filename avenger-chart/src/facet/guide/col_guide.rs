//! FacetColGuide implementation for column-based faceting.
//!
//! This module is a thin wrapper around the shared facet-band guide engine.

use crate::facet::guide::band_guide_engine::{self, ColGuideAxisOps, FacetGuideState};
use crate::facet::marks::facet::CompiledFacetColumnSubplot;
use crate::facet::overflow_projection::FacetOverflowResolutionPhase;
use crate::plot::compiled::CompiledPlot;
use avenger_chart_cartesian::CartesianAxis;
use avenger_chart_core::{
    AvengerChartError, CompiledGuide, CompiledMarkCore, CoordMeasurement, CoordinateGuide,
    GuideOverflowPhase, GuideSharingContext, LayoutBounds, MeasurementResult,
    OverflowSpaceRequirement, SerializableDataFrame, SharingLevel, Theme,
};
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
use crate::facet::guide::band_guide_engine::GuideAnchorSource;
#[cfg(test)]
use avenger_chart_core::CoordinatedOverflow;

/// Guide configuration for FacetCol coordinate system
#[derive(Clone, Default)]
pub struct FacetColGuideConfig {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot extracted from the facet subplot mark
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet subplot mark's data (used when data_override is None)
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("top" or "bottom")
    position: Option<String>,
    /// Slot-sharing level for the column facet variable (0=Free, N=Level(N), 255=Shared)
    sharing_level: u8,
}

impl FacetColGuideConfig {
    /// Create a new FacetColGuideConfig
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the facet title
    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.facet_title = title;
        self
    }
}

impl CoordinateGuide for FacetColGuideConfig {
    type Axis = CartesianAxis;

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // Facet guides do not compose cartesian axes directly.
    }

    fn set_compiled_marks<M>(
        &mut self,
        compiled_marks: &[Arc<M>],
        _session_context: &SessionContext,
    ) where
        M: CompiledMarkCore + ?Sized,
    {
        for mark in compiled_marks {
            if let Some(facet_col) = mark.as_any().downcast_ref::<CompiledFacetColumnSubplot>() {
                self.compiled_subplot = Some(facet_col.compiled_subplot_arc());
                self.facet_data_plan = mark.data_context().logical_plan_node().cloned();
                self.facet_title = facet_col.facet_title().map(|s| s.to_string());
                self.position = facet_col.facet_position().map(|s| s.to_string());
                self.sharing_level = facet_col
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
        Box::new(FacetColGuide {
            facet_title: self.facet_title,
            compiled_subplot: self.compiled_subplot,
            facet_data_plan: self.facet_data_plan,
            position: self.position,
            sharing_level: self.sharing_level,
        })
    }
}

/// Compiled guide for FacetCol coordinate system
#[serde_as]
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FacetColGuide {
    /// Optional facet title
    pub facet_title: Option<String>,
    /// Compiled subplot for measuring overflow
    compiled_subplot: Option<Arc<CompiledPlot>>,
    /// Logical plan for the facet subplot mark's data (used when data_override is None)
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    facet_data_plan: Option<LogicalPlanNode>,
    /// Position of facet labels ("top" or "bottom")
    position: Option<String>,
    /// Slot-sharing level for the column facet variable (0=Free, N=Level(N), 255=Shared)
    #[serde(default)]
    sharing_level: u8,
}

#[cfg(test)]
fn resolve_guide_anchor_overflow(
    place_at_bottom: bool,
    title_visible: bool,
    coordinated_overflow: Option<&CoordinatedOverflow>,
    local_overflow: Option<&CoordinatedOverflow>,
) -> (f32, GuideAnchorSource) {
    band_guide_engine::resolve_col_guide_anchor_overflow(
        place_at_bottom,
        title_visible,
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
impl CompiledGuide for FacetColGuide {
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
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
        theme: &Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        ctx: &SessionContext,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
        phase: GuideOverflowPhase,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let phase = match phase {
            GuideOverflowPhase::Measurement => FacetOverflowResolutionPhase::Measurement,
            GuideOverflowPhase::Final => FacetOverflowResolutionPhase::Final,
        };
        band_guide_engine::measure_overflow_common::<ColGuideAxisOps>(
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
        _theme: &Theme,
        _params: &IndexMap<String, datafusion::common::ScalarValue>,
        _data_override: Option<&datafusion::dataframe::DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        _coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<MeasurementResult, AvengerChartError> {
        Ok(MeasurementResult::default())
    }

    async fn evaluate(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        plot_bounds: &LayoutBounds,
        guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, datafusion::common::ScalarValue>,
        ctx: &SessionContext,
        data_override: Option<&datafusion::dataframe::DataFrame>,
        sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        band_guide_engine::evaluate_common::<ColGuideAxisOps>(
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

impl FacetColGuide {
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
    use crate::facet::FacetDirection;
    use crate::facet::evaluated_facet_tree::{EvaluatedFacetTree, PartitionNode};
    use crate::facet::guide_utils::facet_guide_labels_visible_for_cell;
    use avenger_chart_core::AxisPosition;
    use indexmap::IndexMap;

    fn s(value: &str) -> datafusion::common::ScalarValue {
        datafusion::common::ScalarValue::Utf8(Some(value.to_string()))
    }

    fn row_then_column_tree() -> EvaluatedFacetTree {
        let make_leaf = || {
            PartitionNode::leaf(
                FacetDirection::Column,
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
            FacetDirection::Row,
            255,
            "length_bin".to_string(),
            None,
            children,
        )))
    }

    fn coordinated_overflow(
        guide_top: f32,
        guide_bottom: f32,
        total_top: f32,
        total_bottom: f32,
    ) -> CoordinatedOverflow {
        CoordinatedOverflow {
            guide: OverflowSpaceRequirement {
                top: guide_top,
                bottom: guide_bottom,
                ..Default::default()
            },
            total: OverflowSpaceRequirement {
                top: total_top,
                bottom: total_bottom,
                ..Default::default()
            },
        }
    }

    #[test]
    fn resolve_guide_anchor_top_visible_title_prefers_coordinated_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(41.0, 34.0, 55.0, 34.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow(false, true, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 55.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_top_hidden_title_prefers_coordinated_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(5.0, 39.0, 5.0, 39.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow(false, false, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 5.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_top_hidden_title_uses_coordinated_when_local_total_is_zero() {
        let local = coordinated_overflow(0.0, 8.0, 0.0, 40.0);
        let coordinated = coordinated_overflow(7.0, 39.0, 7.0, 39.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow(false, false, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 7.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_bottom_prefers_coordinated_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(41.0, 7.0, 41.0, 55.0);
        let (resolved, source) =
            resolve_guide_anchor_overflow(true, false, Some(&coordinated), Some(&local));
        assert_eq!(resolved, 55.0);
        assert_eq!(source, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_falls_back_to_local_total_overflow() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let (resolved_top, source_top) =
            resolve_guide_anchor_overflow(false, false, None, Some(&local));
        let (resolved_bottom, source_bottom) =
            resolve_guide_anchor_overflow(true, false, None, Some(&local));
        assert_eq!(resolved_top, 60.0);
        assert_eq!(source_top, GuideAnchorSource::LocalGuide);
        assert_eq!(resolved_bottom, 40.0);
        assert_eq!(source_bottom, GuideAnchorSource::LocalGuide);
    }

    #[test]
    fn resolve_guide_anchor_prefers_coordinated_over_local_when_both_present() {
        let local = coordinated_overflow(12.0, 8.0, 60.0, 40.0);
        let coordinated = coordinated_overflow(5.0, 39.0, 5.0, 39.0);
        let (resolved_top, source_top) =
            resolve_guide_anchor_overflow(false, true, Some(&coordinated), Some(&local));
        let (resolved_bottom, source_bottom) =
            resolve_guide_anchor_overflow(true, true, Some(&coordinated), Some(&local));
        assert_eq!(resolved_top, 5.0);
        assert_eq!(source_top, GuideAnchorSource::CoordinatedGuide);
        assert_eq!(resolved_bottom, 39.0);
        assert_eq!(source_bottom, GuideAnchorSource::CoordinatedGuide);
    }

    #[test]
    fn resolve_guide_anchor_defaults_to_zero_without_overflow_context() {
        let (resolved, source) = resolve_guide_anchor_overflow(false, false, None, None);
        assert_eq!(resolved, 0.0);
        assert_eq!(source, GuideAnchorSource::DefaultZero);
    }

    #[test]
    fn propagated_subplot_overflow_uses_total_subtree() {
        let local = coordinated_overflow(17.0, 9.0, 53.0, 41.0);
        let propagated = propagated_subplot_overflow(Some(local));
        assert_eq!(propagated.top, 53.0);
        assert_eq!(propagated.bottom, 41.0);
    }

    #[test]
    fn shared_top_owner_visibility_only_far_top_row_shows_labels() {
        let tree = row_then_column_tree();
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Top,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Top,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Top,
            255
        ));
    }

    #[test]
    fn shared_bottom_owner_visibility_only_far_bottom_row_shows_labels() {
        let tree = row_then_column_tree();
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("short")],
            AxisPosition::Bottom,
            255
        ));
        assert!(!facet_guide_labels_visible_for_cell(
            &tree,
            &[s("medium")],
            AxisPosition::Bottom,
            255
        ));
        assert!(facet_guide_labels_visible_for_cell(
            &tree,
            &[s("long")],
            AxisPosition::Bottom,
            255
        ));
    }

    #[test]
    fn free_sharing_visibility_shows_labels_in_all_rows() {
        let tree = row_then_column_tree();
        for path in ["short", "medium", "long"] {
            assert!(facet_guide_labels_visible_for_cell(
                &tree,
                &[s(path)],
                AxisPosition::Bottom,
                0
            ));
        }
    }
}
