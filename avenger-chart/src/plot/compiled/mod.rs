//! CompiledPlot - Immutable, serializable plot ready for rendering

mod child_frame_container;
mod child_frame_runtime;
mod child_frame_scope;
mod container_band_guide;
mod container_domain_sharing;
mod container_guide;
mod container_labels;
mod container_sharing;
mod coordination_scope;
mod domain_coordination;
mod legends;
mod mark_data_runtime;
pub(crate) mod rendering;
pub mod scale_provider;
pub(crate) mod scales; // Made public so plot.rs can call build_scale_builder_from_marks
mod titles;
mod validation;

use std::{any::Any, collections::HashMap, sync::Arc};

use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use datafusion_proto::protobuf::LogicalPlanNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use avenger_chart_core::{
    AvengerChartError, AxisSpec, CompiledGuide, CompiledMark, CompiledSubplotChildPlot,
    CompiledSubplotPayload, CoordMeasurement, CoordinateSystemTransform,
    EvaluationContext as CoreEvaluationContext, Legend, ScaleRangeBinding, SerializableDataFrame,
    SerializableScalarMap, Theme, channel::strip_trailing_numbers,
};
use avenger_chart_scales::{ConfiguredScaleWithSpec, PlotScaleSpec as ScaleSpec, ScaleBuilder};

use crate::layout::{
    ChildFrameContentMeasurement, ChildFrameContentSolver, ContentAllocation, ContentLayout,
    ContentLayoutSolver, FrameAllocation, FrameDemand, LayoutSpec, SinglePlotContentMeasurement,
    SinglePlotContentSolver,
};

pub use self::child_frame_container::ChildFrameContainerView;
pub(crate) use self::child_frame_container::{
    child_frame_container_overflow, child_frame_container_view_from_cartesian_positioned,
    child_frame_container_view_from_concat, child_frame_container_view_from_polar_positioned,
};
pub(crate) use self::child_frame_runtime::{
    ChildFrameDataSelection, ChildFrameRuntime, PreparedChildFramePlot,
    fixed_child_plot_area_layout_spec, measure_child_frame_plot_with_builder,
};
pub(crate) use self::child_frame_scope::{
    ChildFrameKey, ChildFrameScopeKey, ChildFrameSharingLevel, ChildFrameSharingPath,
    ContainerPathSegment, container_path_without_facet_segments,
};
pub(crate) use self::container_band_guide::{
    ContainerBandGuideMeasurementConfig, ContainerBandGuideRenderConfig,
    measure_container_band_guide_slab, render_container_band_guide_slab,
};
pub(crate) use self::container_domain_sharing::{
    ChildFrameChannelDomainExtent, ChildFrameDomainSharingInput,
    child_frame_domain_sharing_levels_for_plot, coordinated_child_frame_domain_extents,
    extract_child_frame_shared_domain_extents,
};
pub(crate) use self::container_guide::{
    measure_child_frame_container_guide_overflow, render_child_frame_container_guide_labels,
};
pub(crate) use self::container_labels::ContainerLabelPlacement;
#[cfg(test)]
pub(crate) use self::container_labels::container_label_items_from_child_frame_container;
pub(crate) use self::container_sharing::{
    ContainerEdgeLevelProjection, EdgeOwnershipRequest, EdgeOwnershipScope, SharingGroupEdge,
    edge_ownership_scope_for_request, enumeration_ancestor_path, owner_for_scope,
    project_container_edge_levels, shared_path_key,
};
pub(crate) use self::coordination_scope::{CoordinationKind, CoordinationScopeKey};
pub(crate) use self::domain_coordination::{ChildFrameDomainRequest, aggregate_domain_requests};
use self::legends::PreparedLegendPlan;
pub(crate) use self::mark_data_runtime::{
    MarkDataRequest, PreparedMarkData, prepare_mark_data as prepare_mark_data_runtime,
};

use super::title::{PlotSubtitle, PlotTitle};

#[serde_as]
#[derive(Serialize, Deserialize)]
pub struct CompiledPlot {
    /// Coordinate system transform for position mapping
    pub(crate) coord_transform: Box<dyn CoordinateSystemTransform>,

    /// Guide renderer for axes/grids
    pub(crate) compiled_guide: Option<Arc<dyn CompiledGuide>>,

    /// Mark renderers
    pub(crate) marks: Vec<Arc<dyn CompiledMark>>,

    /// Axis specifications
    pub(crate) axis_specs: HashMap<String, AxisSpec>,

    /// Legends
    pub(crate) legends: IndexMap<String, Legend>,

    /// Layout specification
    pub(crate) layout_spec: LayoutSpec,

    /// Plot title
    pub(crate) title: Option<PlotTitle>,

    /// Plot subtitle
    pub(crate) subtitle: Option<PlotSubtitle>,

    /// Theme
    pub(crate) theme: Option<Arc<Theme>>,

    /// Mapping from scale names to coordinate channel
    pub(crate) scale_to_coord_channel: HashMap<String, String>,

    /// Scale specifications for building scales
    pub(crate) scale_specs: HashMap<String, ScaleSpec>,

    // Note: We intentionally do not persist a ScaleBuilder here. Scales are
    // rebuilt per evaluation using current params to ensure correctness for
    // paramized data queries and to keep direct vs serialized paths identical.
    /// Plot-level data for mark data inheritance
    #[serde_as(as = "Option<FromInto<SerializableDataFrame>>")]
    pub(crate) data: Option<LogicalPlanNode>,

    /// Default parameter values for prepared statements
    #[serde_as(as = "FromInto<SerializableScalarMap>")]
    pub(crate) default_params: IndexMap<String, ScalarValue>,
}

impl CompiledPlot {
    /// Get the theme or create default if not set
    pub fn get_theme(&self) -> Arc<Theme> {
        self.theme
            .clone()
            .unwrap_or_else(|| Arc::new(Theme::light()))
    }

    /// Get title if configured
    pub fn get_title(&self) -> Option<&PlotTitle> {
        self.title.as_ref()
    }

    /// Get subtitle if configured
    pub fn get_subtitle(&self) -> Option<&PlotSubtitle> {
        self.subtitle.as_ref()
    }

    /// Get layout spec
    pub fn get_layout_spec(&self) -> &LayoutSpec {
        &self.layout_spec
    }

    /// Get default parameter values
    pub fn get_default_params(&self) -> &IndexMap<String, ScalarValue> {
        &self.default_params
    }

    /// Get compiled mark renderers
    pub fn marks(&self) -> &[Arc<dyn CompiledMark>] {
        &self.marks
    }

    /// Build scales from an existing ScaleBuilder with specific dimensions.
    ///
    /// Reuses cached data queries from the provided builder, making it efficient
    /// for building scales multiple times (e.g., initial layout pass and final
    /// render pass).
    ///
    /// Typical usage:
    /// ```ignore
    /// // Build once (queries data)
    /// let builder = build_scale_builder_from_marks(...).await?;
    ///
    /// // Reuse multiple times (no queries)
    /// let scales1 = plot.build_scales_from_builder(&builder, 400.0, 300.0, ctx, params).await?;
    /// let scales2 = plot.build_scales_from_builder(&builder, 800.0, 600.0, ctx, params).await?;
    /// ```
    pub async fn build_scales_from_builder(
        &self,
        builder: &ScaleBuilder,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        // Build coordinate system range bindings map
        let mut coord_system_range_bindings = HashMap::<String, ScaleRangeBinding>::new();
        for channel in builder.channel_builders().keys() {
            let base = strip_trailing_numbers(channel);
            if let Some(binding) = self.coord_transform.default_range_binding(base) {
                coord_system_range_bindings.insert(channel.clone(), binding);
            }
        }

        let theme = self.get_theme();
        let default_range_resolver = scales::default_range_for_compiled_marks(&self.marks);
        let built = builder
            .build_scales(
                plot_area_width,
                plot_area_height,
                &coord_system_range_bindings,
                &self.scale_specs,
                &default_range_resolver,
                theme.as_ref(),
                ctx,
                params,
            )
            .await?;

        Ok(built)
    }

    /// Get scale specifications
    pub fn scale_specs(&self) -> &HashMap<String, ScaleSpec> {
        &self.scale_specs
    }

    /// Get legends
    pub fn legends(&self) -> &IndexMap<String, Legend> {
        &self.legends
    }

    /// Build configured scales for a provided DataFrame and plot-area dimensions.
    pub async fn build_scales_for_dataframe(
        &self,
        df: &DataFrame,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<HashMap<String, ConfiguredScaleWithSpec>, AvengerChartError> {
        let eval_ctx =
            CoreEvaluationContext::new(self.get_theme(), Arc::new(ctx.clone()), params.clone());
        let scale_builder = scales::build_scale_builder_from_marks(
            &self.marks,
            &self.scale_specs,
            &self.coord_transform,
            &self.data,
            Some(df.clone()),
            &eval_ctx,
            self.get_theme().as_ref(),
        )
        .await?;

        self.build_scales_from_builder(
            &scale_builder,
            plot_area_width,
            plot_area_height,
            ctx,
            params,
        )
        .await
    }
}

pub(crate) fn compiled_subplot_payload_child_plot(
    payload: &CompiledSubplotPayload,
) -> &CompiledPlot {
    payload
        .compiled_child_plot()
        .as_any()
        .downcast_ref::<CompiledPlot>()
        .expect("subplot payload child plot is not an avenger-chart CompiledPlot")
}

pub(crate) fn compiled_subplot_payload_child_plot_arc(
    payload: &CompiledSubplotPayload,
) -> Arc<CompiledPlot> {
    match Arc::clone(payload.compiled_child_plot())
        .into_any_arc()
        .downcast::<CompiledPlot>()
    {
        Ok(compiled) => compiled,
        Err(_) => panic!("subplot payload child plot is not an avenger-chart CompiledPlot"),
    }
}

#[typetag::serde]
impl CompiledSubplotChildPlot for CompiledPlot {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

/// Measurement results from `measure_plot_components`
///
/// This captures all the computation needed for layout coordination without
/// actually rendering any marks. The render pass uses this to avoid re-measuring.
///
/// Overflow info is available via `layout.overflow` (guide only) and
/// `layout.total_overflow` (guide + legends).
pub struct ComponentsMeasurement {
    /// Coordinate-system-specific measurement data (e.g., facet cell layout)
    ///
    /// This is computed by top-level coordinate measurement dispatch and
    /// contains layout data that's available to both guides and marks during
    /// rendering. For facet coordinate systems, this includes cell positions,
    /// subplot measurements, and computed padding. For non-layout coordinate
    /// systems, this is an `EmptyCoordMeasurement`.
    pub coord_measurement: Box<dyn CoordMeasurement>,

    /// Scales for rendering
    pub scales: std::collections::HashMap<String, ConfiguredScaleWithSpec>,

    /// Plot area dimensions
    pub plot_area_width: f32,
    pub plot_area_height: f32,

    /// Canvas size
    pub canvas_size: (f32, f32),

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Layout solution (contains overflow, total_overflow, legends/titles positioning)
    pub layout: crate::render::LayoutSolution,

    /// Allocation granted to this measured chart frame by its parent.
    pub frame_allocation: FrameAllocation,

    /// Merged params (defaults + provided + canvas dimensions)
    pub params: indexmap::IndexMap<String, datafusion::common::ScalarValue>,

    /// Prepared legend plan used by both layout and render phases
    pub(crate) legend_plan: PreparedLegendPlan,
}

impl std::fmt::Debug for ComponentsMeasurement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentsMeasurement")
            .field("coord_measurement", &"<coord_measurement>")
            .field("scales", &format!("{} scales", self.scales.len()))
            .field("plot_area_width", &self.plot_area_width)
            .field("plot_area_height", &self.plot_area_height)
            .field("canvas_size", &self.canvas_size)
            .field("layout", &"LayoutSolution")
            .field("frame_allocation", &self.frame_allocation)
            .field("legend_plan", &"<legend_plan>")
            .finish()
    }
}

impl ComponentsMeasurement {
    pub(crate) fn refresh_frame_allocation_rect(&mut self) {
        self.frame_allocation.rect.width = self.canvas_size.0;
        self.frame_allocation.rect.height = self.canvas_size.1;
    }

    pub(crate) fn sync_canvas_size_from_layout(&mut self) {
        self.canvas_size = self.layout.canvas_size;
        self.refresh_frame_allocation_rect();
    }

    pub(crate) fn frame_demand(&self) -> FrameDemand {
        FrameDemand::from_guide_and_rendered_envelope(
            self.layout.overflow.clone(),
            self.layout.total_overflow.clone(),
        )
    }

    pub(crate) fn content_allocation(&self) -> ContentAllocation {
        ContentAllocation::new(self.frame_allocation, *self.layout.plot_area_bounds())
    }

    pub(crate) fn content_layout(&self) -> Result<ContentLayout, AvengerChartError> {
        let allocation = self.content_allocation();
        let frame_demand = self.frame_demand();

        if let Some(container) = self.child_frame_container_view()? {
            let solver = ChildFrameContentSolver;
            let child_frame_allocations = container.child_frame_allocations();
            let demand = solver.measure_content_demand(
                &allocation,
                &ChildFrameContentMeasurement {
                    frame_demand,
                    child_frame_allocations,
                },
            )?;
            let plan = solver.coordinate_content(&allocation, &demand)?;
            solver.realize_content(allocation, demand, plan)
        } else {
            let solver = SinglePlotContentSolver;
            let demand = solver.measure_content_demand(
                &allocation,
                &SinglePlotContentMeasurement { frame_demand },
            )?;
            let plan = solver.coordinate_content(&allocation, &demand)?;
            solver.realize_content(allocation, demand, plan)
        }
    }
}

/// Extended components returned from plot evaluation
///
/// This structure supports both measurement and rendering modes,
/// and includes all plot components (data, guides, legends, titles).
pub struct PlotComponents {
    /// Data mark scene graph elements
    pub data_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Guide mark scene graph elements (axes, grids)
    pub guide_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Legend scene graph elements
    pub legend_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Title scene graph elements
    pub title_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Subtitle scene graph elements
    pub subtitle_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,

    /// Plot area bounds (data rectangle)
    pub plot_bounds: crate::layout::LayoutBounds,

    /// Clip region for data marks
    pub clip: avenger_scenegraph::marks::group::Clip,

    /// Size dimensions used for this evaluation
    pub size: (f32, f32),

    /// Whether `size` represents canvas dimensions (true) or plot area dimensions (false)
    pub size_is_canvas: bool,

    /// Debug marks (layout visualization) - these are in absolute canvas coordinates
    pub debug_marks: Vec<avenger_scenegraph::marks::mark::SceneMark>,
}
