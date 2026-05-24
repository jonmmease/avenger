//! Concatenation coordinate systems.
//!
//! `HConcat` and `VConcat` are container coordinate systems: their marks are
//! child `Subplot` marks, and their coordinate measurement produces child-frame
//! placement metadata for later rendering/debug consumers.

mod subplot;

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::Clip, mark::SceneMark};
use datafusion::{common::ScalarValue, dataframe::DataFrame, prelude::SessionContext};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    container::{
        BandChildFrameInput, BandChildFramePlacement, BandSpacing, BoundaryDemand1D, ChildFrameKey,
        ChildFramePlacementResult, ChildFrameScopeKey, ChildFrameSharingLevel,
        ContainerPathSegment,
    },
    coords::{
        CoordMeasureRequest, CoordMeasurement, CoordinateSystem, CoordinateSystemTransform,
        PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    guide::{
        CompiledGuide, CoordinateGuide, GuideSharingContext, GuideUpdate, OverflowSpaceRequirement,
    },
    layout::{BandDirection, LayoutBounds, Size2D},
    marks::CompiledMark,
    plot::compiled::{
        ChildFrameDataSelection, ChildFrameDomainSharingInput, ChildFrameRuntime,
        ComponentsMeasurement, ContainerLabelPlacement, PreparedChildFramePlot,
        child_frame_container_view_from_concat, container_path_without_facet_segments,
        coordinated_child_frame_domain_extents, measure_child_frame_container_guide_overflow,
        render_child_frame_container_guide_labels,
    },
    render::EvaluationContext,
    scales::{DomainExtent, ScaleRangeBinding},
    theme::Theme,
};

pub use subplot::{CompiledConcatSubplot, compiled_subplot};

/// Horizontal concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HConcat;

impl HConcat {
    pub fn new() -> Self {
        Self
    }
}

/// Vertical concatenation of `Subplot` marks.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct VConcat;

impl VConcat {
    pub fn new() -> Self {
        Self
    }
}

impl CoordinateSystem for HConcat {
    type Guide = ConcatGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystem for VConcat {
    type Guide = ConcatGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

/// Guide for concat containers.
///
/// The guide reserves parent-frame space for measured child frames and renders
/// optional child labels supplied by `Subplot::label`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ConcatGuide;

impl GuideUpdate for ConcatGuide {
    fn update(self, _other: Self) -> Self {
        self
    }
}

impl CoordinateGuide for ConcatGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {}

    fn set_compiled_marks(
        &mut self,
        _compiled_marks: Vec<Arc<dyn CompiledMark>>,
        _session_context: &SessionContext,
    ) {
    }

    fn update(&mut self, _other: Self) {}

    fn build(self) -> Box<dyn CompiledGuide> {
        Box::new(self)
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CompiledGuide for ConcatGuide {
    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        _data_override: Option<&DataFrame>,
        _ctx: &SessionContext,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: Option<&dyn CoordMeasurement>,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        let Some(concat) = coord_measurement.and_then(concat_coord_ref) else {
            return Ok(OverflowSpaceRequirement::default());
        };

        let container = child_frame_container_view_from_concat(concat)?;
        measure_child_frame_container_guide_overflow(
            plot_width,
            plot_height,
            &container,
            Some(concat_label_placement(concat)),
            theme,
            params,
        )
    }

    async fn evaluate(
        &self,
        _scales: &HashMap<String, ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        plot_bounds: &LayoutBounds,
        _guide_overflow: &OverflowSpaceRequirement,
        theme: &Theme,
        params: &IndexMap<String, ScalarValue>,
        _ctx: &SessionContext,
        _data_override: Option<&DataFrame>,
        _sharing_context: GuideSharingContext<'_>,
        coord_measurement: &dyn CoordMeasurement,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let concat = concat_coord_ref(coord_measurement).ok_or_else(|| {
            AvengerChartError::InternalError(
                "ConcatGuide received non-concat coordinate measurement".to_string(),
            )
        })?;
        let container = child_frame_container_view_from_concat(concat)?;
        render_child_frame_container_guide_labels(
            &container,
            Some(concat_label_placement(concat)),
            plot_bounds,
            theme,
            params,
        )
    }

    fn get_clip(
        &self,
        _plot_width: f32,
        _plot_height: f32,
        _scales: &HashMap<String, ConfiguredScale>,
    ) -> Clip {
        Clip::None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for HConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        request: CoordMeasureRequest<'_>,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Horizontal,
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
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
}

#[async_trait::async_trait]
#[typetag::serde]
impl CoordinateSystemTransform for VConcat {
    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }

    async fn measure(
        &self,
        request: CoordMeasureRequest<'_>,
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Vertical,
            request.plot_width(),
            request.plot_height(),
            request.eval_ctx(),
            request.data(),
            request.compiled_marks(),
            request.facet_path(),
        )
        .await
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        plot_width: f32,
        plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        container_point_geometry(position_channels, position_values, plot_width, plot_height)
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
}

#[derive(Debug)]
pub struct ConcatCoordMeasurement {
    pub(crate) children: Vec<ConcatChildMeasurement>,
    pub(crate) child_band_layout: BandChildFramePlacement,
    pub(crate) fallback_content_size: Size2D,
}

impl ConcatCoordMeasurement {
    pub(crate) fn children(&self) -> &[ConcatChildMeasurement] {
        &self.children
    }

    pub(crate) fn child(&self, child_index: usize) -> Option<&ConcatChildMeasurement> {
        self.children
            .iter()
            .find(|child| child.child_index == child_index)
    }

    pub(crate) fn child_scope_key(&self, child_index: usize) -> Option<ChildFrameScopeKey> {
        self.child(child_index)
            .map(ConcatChildMeasurement::scope_key)
    }

    pub(crate) fn child_frame_placement(&self) -> ChildFramePlacementResult {
        self.child_band_layout
            .to_child_frame_placement_result([0.0, 0.0], self.fallback_content_size)
    }
}

impl CoordMeasurement for ConcatCoordMeasurement {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug)]
pub(crate) struct ConcatChildMeasurement {
    pub(crate) child_index: usize,
    pub(crate) key: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) container_path: Vec<ContainerPathSegment>,
    pub(crate) measurement: ComponentsMeasurement,
}

impl ConcatChildMeasurement {
    pub(crate) fn scope_key(&self) -> ChildFrameScopeKey {
        concat_child_scope_key(&self.container_path, self.child_index, self.key.as_deref())
    }

    pub(crate) fn debug_label(&self) -> String {
        match (&self.key, &self.label) {
            (Some(key), Some(label)) => {
                format!("{} (key={key:?}, label={label:?})", self.child_index)
            }
            (Some(key), None) => format!("{} (key={key:?})", self.child_index),
            (None, Some(label)) => format!("{} (label={label:?})", self.child_index),
            (None, None) => self.child_index.to_string(),
        }
    }
}

fn concat_child_scope_key(
    container_path: &[ContainerPathSegment],
    child_index: usize,
    key: Option<&str>,
) -> ChildFrameScopeKey {
    ChildFrameScopeKey::new(
        container_path.to_vec(),
        ChildFrameKey::ConcatChild {
            index: child_index,
            key: key.map(ToOwned::to_owned),
        },
    )
}

pub(crate) fn concat_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&ConcatCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
}

fn concat_label_placement(concat: &ConcatCoordMeasurement) -> ContainerLabelPlacement {
    match concat.child_band_layout.direction {
        BandDirection::Horizontal => ContainerLabelPlacement::Top,
        BandDirection::Vertical => ContainerLabelPlacement::Left,
    }
}

fn child_plot_area_size(
    direction: BandDirection,
    plot_width: f32,
    plot_height: f32,
    child_count: usize,
) -> Size2D {
    let child_count = child_count.max(1) as f32;
    match direction {
        BandDirection::Horizontal => Size2D::new(plot_width / child_count, plot_height),
        BandDirection::Vertical => Size2D::new(plot_width, plot_height / child_count),
    }
}

fn boundary_demand_for_child(
    direction: BandDirection,
    measurement: &ComponentsMeasurement,
) -> BoundaryDemand1D {
    let slabs = measurement.frame_demand().rendered_envelope;
    match direction {
        BandDirection::Horizontal => BoundaryDemand1D {
            before: slabs.left,
            after: slabs.right,
        },
        BandDirection::Vertical => BoundaryDemand1D {
            before: slabs.top,
            after: slabs.bottom,
        },
    }
}

fn band_input_for_child(
    direction: BandDirection,
    child: &ConcatChildMeasurement,
) -> BandChildFrameInput {
    let main_axis_size = match direction {
        BandDirection::Horizontal => child.measurement.plot_area_width,
        BandDirection::Vertical => child.measurement.plot_area_height,
    };
    let cross_axis_size = match direction {
        BandDirection::Horizontal => child.measurement.plot_area_height,
        BandDirection::Vertical => child.measurement.plot_area_width,
    };

    BandChildFrameInput {
        child_index: child.child_index,
        main_axis_size,
        cross_axis_size,
        boundary: boundary_demand_for_child(direction, &child.measurement),
    }
}

struct PreparedConcatChild<'a> {
    subplot: &'a CompiledConcatSubplot,
    child_plot: PreparedChildFramePlot<'a>,
    container_path: Vec<ContainerPathSegment>,
    relative_facet_child_frame_path: Vec<ContainerPathSegment>,
}

impl PreparedConcatChild<'_> {
    fn child_index(&self) -> usize {
        self.subplot.child_index()
    }

    fn key(&self) -> Option<&str> {
        self.subplot.key()
    }

    fn label(&self) -> Option<&str> {
        self.subplot.label()
    }

    fn scope_key(&self) -> ChildFrameScopeKey {
        concat_child_scope_key(&self.container_path, self.child_index(), self.key())
    }

    fn sharing_level(
        &self,
        direction: BandDirection,
        child_count: usize,
    ) -> ChildFrameSharingLevel {
        match direction {
            BandDirection::Horizontal => {
                ChildFrameSharingLevel::hconcat_child(self.child_index(), child_count, self.key())
            }
            BandDirection::Vertical => {
                ChildFrameSharingLevel::vconcat_child(self.child_index(), child_count, self.key())
            }
        }
    }
}

fn coordinated_domain_extents_for_concat_children(
    children: &[PreparedConcatChild<'_>],
) -> Vec<HashMap<String, DomainExtent>> {
    let scope_keys = children
        .iter()
        .map(PreparedConcatChild::scope_key)
        .collect::<Vec<_>>();
    let inputs = children
        .iter()
        .zip(scope_keys.iter())
        .map(|(child, scope_key)| {
            ChildFrameDomainSharingInput::new(
                scope_key,
                child.child_plot.local_domain_extents(),
                child.child_plot.channel_domain_sharing_levels(),
            )
        })
        .collect::<Vec<_>>();

    coordinated_child_frame_domain_extents(&inputs)
}

async fn prepare_concat_child<'a>(
    subplot: &'a CompiledConcatSubplot,
    eval_ctx: &EvaluationContext,
    inherited_data: Option<&DataFrame>,
) -> Result<PreparedConcatChild<'a>, AvengerChartError> {
    let child_plot = subplot.compiled_subplot();
    let data_selection = if subplot.inherits_parent_data() {
        ChildFrameDataSelection::InheritParent
    } else {
        ChildFrameDataSelection::ExplicitChild
    };
    let runtime = ChildFrameRuntime::new();
    let child_plot = runtime
        .prepare_plot(child_plot, data_selection, inherited_data, eval_ctx)
        .await?;
    let mut relative_facet_child_frame_path =
        container_path_without_facet_segments(eval_ctx.child_frame_container_path());
    relative_facet_child_frame_path.push(ContainerPathSegment::concat_child(
        subplot.child_index(),
        subplot.key(),
    ));

    Ok(PreparedConcatChild {
        subplot,
        child_plot,
        container_path: eval_ctx.child_frame_container_path().to_vec(),
        relative_facet_child_frame_path,
    })
}

async fn measure_prepared_concat_child(
    prepared: &PreparedConcatChild<'_>,
    direction: BandDirection,
    child_count: usize,
    child_plot_area: Size2D,
    eval_ctx: &EvaluationContext,
    facet_path: &[ScalarValue],
    coordinated_domain_extents: &HashMap<String, DomainExtent>,
    facet_scoped_domain_extents: &HashMap<String, DomainExtent>,
) -> Result<ConcatChildMeasurement, AvengerChartError> {
    let runtime = ChildFrameRuntime::new();
    let child_layout_spec =
        runtime.fixed_plot_area_layout_spec(child_plot_area.width, child_plot_area.height);
    let child_eval_ctx =
        runtime.eval_context(eval_ctx, prepared.sharing_level(direction, child_count));
    let measurement = prepared
        .child_plot
        .measure(
            &child_eval_ctx,
            &child_layout_spec,
            facet_path,
            &[coordinated_domain_extents, facet_scoped_domain_extents],
        )
        .await?;

    Ok(ConcatChildMeasurement {
        child_index: prepared.child_index(),
        key: prepared.key().map(ToOwned::to_owned),
        label: prepared.label().map(ToOwned::to_owned),
        container_path: prepared.container_path.clone(),
        measurement,
    })
}

async fn measure_concat_coord_system(
    direction: BandDirection,
    plot_width: f32,
    plot_height: f32,
    eval_ctx: &EvaluationContext,
    data: Option<&DataFrame>,
    compiled_marks: &[Arc<dyn CompiledMark>],
    facet_path: &[ScalarValue],
) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
    let subplots = compiled_marks
        .iter()
        .filter_map(|mark| compiled_subplot(mark.as_ref()))
        .collect::<Vec<_>>();
    let child_plot_area = child_plot_area_size(direction, plot_width, plot_height, subplots.len());

    let mut prepared_children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        prepared_children.push(prepare_concat_child(subplot, eval_ctx, data).await?);
    }

    let coordinated_domain_extents =
        coordinated_domain_extents_for_concat_children(&prepared_children);

    let mut children = Vec::with_capacity(prepared_children.len());
    let child_count = prepared_children.len();
    for (prepared, coordinated_extents) in prepared_children
        .iter()
        .zip(coordinated_domain_extents.iter())
    {
        let facet_scoped_extents = eval_ctx
            .facet_scale_precompute_store()
            .coordinated_child_frame_domain_extents(
                &prepared.relative_facet_child_frame_path,
                facet_path,
            );
        children.push(
            measure_prepared_concat_child(
                prepared,
                direction,
                child_count,
                child_plot_area,
                eval_ctx,
                facet_path,
                coordinated_extents,
                &facet_scoped_extents,
            )
            .await?,
        );
    }

    let inputs = children
        .iter()
        .map(|child| band_input_for_child(direction, child))
        .collect::<Vec<_>>();
    let child_band_layout =
        BandChildFramePlacement::from_sized_children(direction, &inputs, BandSpacing::default());

    Ok(Box::new(ConcatCoordMeasurement {
        children,
        child_band_layout,
        fallback_content_size: Size2D::new(plot_width, plot_height),
    }))
}

fn container_point_geometry(
    position_channels: &HashMap<&str, ScalarOrArray<f32>>,
    position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
    plot_width: f32,
    plot_height: f32,
) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
    let _ = position_values;
    let len = position_channels
        .values()
        .find_map(|v| match v.value() {
            ScalarOrArrayValue::Array(arr) => Some(arr.len()),
            ScalarOrArrayValue::Scalar(_) => None,
        })
        .unwrap_or(1);
    let x = plot_width / 2.0;
    let y = plot_height / 2.0;
    let geometry = if len == 1 {
        PointGeometry {
            x: ScalarOrArray::new_scalar(x),
            y: ScalarOrArray::new_scalar(y),
        }
    } else {
        PointGeometry {
            x: ScalarOrArray::new_array(vec![x; len]),
            y: ScalarOrArray::new_array(vec![y; len]),
        }
    };
    Ok(Box::new(geometry))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_scenegraph::marks::{mark::SceneMark, symbol::SceneSymbolMark};
    use datafusion::{
        arrow::{
            array::{Array, Float64Array, StringArray},
            record_batch::RecordBatch as ArrowRecordBatch,
        },
        prelude::{SessionContext, col},
    };

    use super::*;
    use crate::{
        cartesian::{Cartesian, CartesianLinePositionChannels, CartesianSymbolPositionChannels},
        chart_core::ScaleSharing,
        coords::FacetAxis,
        facet::{
            coord::{FacetBandCoordMeasurement, FacetColumn, FacetRow},
            evaluated_facet_tree::EvaluatedFacetTree,
        },
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        marks::{Subplot, line::Line, symbol::Symbol},
        plot::{
            CompiledPlot, Plot,
            compiled::{
                CoordinationAxis, CoordinationKind, CoordinationScopeKey,
                child_frame_container_view_from_concat,
                container_label_items_from_child_frame_container,
                scale_provider::DynamicScaleProvider, scales::build_scale_builder_from_marks,
            },
        },
        render::EvaluationContext,
        scales::Linear,
        zerod::ZeroDCoord,
    };

    async fn measurement_for_plot(
        compiled: &CompiledPlot,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        measurement_for_plot_with_container_path(compiled, plot_width, plot_height, ctx, &[]).await
    }

    async fn measurement_for_plot_with_container_path(
        compiled: &CompiledPlot,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
        container_path: &[ContainerPathSegment],
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        let params = compiled.get_default_params().clone();
        let facet_tree = EvaluatedFacetTree::from_compiled_plot(compiled, ctx).await?;
        let mut eval_ctx = EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            params.clone(),
            Arc::new(facet_tree),
        );
        for segment in container_path {
            eval_ctx = eval_ctx.with_child_frame_container_path_appended(segment.clone());
        }
        let layout_spec = EvaluatedLayoutSpec {
            canvas: EvaluatedSizeMode::Auto,
            plot_area: EvaluatedSizeMode::Fixed {
                width: plot_width,
                height: plot_height,
            },
            margins: EvaluatedMargins {
                top: 0.0,
                right: 0.0,
                bottom: 0.0,
                left: 0.0,
            },
        };
        let scale_builder = build_scale_builder_from_marks(
            &compiled.marks,
            &compiled.scale_specs,
            &compiled.coord_transform,
            &compiled.data,
            None,
            &eval_ctx,
            compiled.get_theme().as_ref(),
        )
        .await?;
        let provider = DynamicScaleProvider {
            builder: &scale_builder,
            plot: compiled,
        };

        compiled
            .measure_plot_components(&eval_ctx, &layout_spec, &provider, None, &[])
            .await
    }

    fn xy_dataframe(
        ctx: &SessionContext,
        x_values: Vec<f64>,
        y_values: Vec<f64>,
    ) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(x_values)) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(y_values)) as Arc<dyn Array>,
            ),
        ])
        .expect("create xy record batch");
        ctx.read_batch(batch).expect("read xy test batch")
    }

    fn grouped_xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 100.0, 101.0])) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.0, 2.0])) as Arc<dyn Array>,
            ),
            (
                "group",
                Arc::new(StringArray::from(vec!["left", "left", "right", "right"]))
                    as Arc<dyn Array>,
            ),
        ])
        .expect("create grouped xy record batch");
        ctx.read_batch(batch).expect("read grouped xy test batch")
    }

    fn nested_grouped_xy_dataframe(ctx: &SessionContext) -> datafusion::dataframe::DataFrame {
        let batch = ArrowRecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 100.0, 101.0])) as Arc<dyn Array>,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![1.0, 2.0, 1.0, 2.0])) as Arc<dyn Array>,
            ),
            (
                "group",
                Arc::new(StringArray::from(vec!["left", "left", "right", "right"]))
                    as Arc<dyn Array>,
            ),
            (
                "subgroup",
                Arc::new(StringArray::from(vec!["top", "bottom", "top", "bottom"]))
                    as Arc<dyn Array>,
            ),
        ])
        .expect("create nested grouped xy record batch");
        ctx.read_batch(batch)
            .expect("read nested grouped xy test batch")
    }

    fn find_symbol_mark(mark: &SceneMark) -> Option<&SceneSymbolMark> {
        match mark {
            SceneMark::Symbol(symbol) => Some(symbol),
            SceneMark::Group(group) => group.marks.iter().find_map(find_symbol_mark),
            _ => None,
        }
    }

    fn child_scatter_plot(
        data: datafusion::dataframe::DataFrame,
        share_x: bool,
    ) -> Plot<Cartesian> {
        let symbol = if share_x {
            Symbol::new()
                .x_with(col("x"), |c| c.with_scale_sharing(ScaleSharing::Shared))
                .y(col("y"))
        } else {
            Symbol::new().x(col("x")).y(col("y"))
        };
        Plot::<Cartesian>::new().data(data).mark(symbol)
    }

    fn line_mark(share_x: bool) -> Line<Cartesian> {
        let mark = Line::<Cartesian>::new().y(col("y"));
        if share_x {
            mark.x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
                    .with_scale_sharing(ScaleSharing::Shared)
            })
        } else {
            mark.x_with(col("x"), |c| {
                c.scale_with::<Linear>(|s| s.nice(false).zero(false))
            })
        }
    }

    fn line_child_plot(data: datafusion::dataframe::DataFrame, share_x: bool) -> Plot<Cartesian> {
        Plot::<Cartesian>::new().data(data).mark(line_mark(share_x))
    }

    fn child_x_domain(child: &ConcatChildMeasurement) -> (f32, f32) {
        measurement_x_domain(&child.measurement)
    }

    fn measurement_x_domain(measurement: &ComponentsMeasurement) -> (f32, f32) {
        measurement
            .scales
            .get("x")
            .expect("x scale should exist")
            .configured()
            .numeric_interval_domain()
            .expect("x scale should have a numeric interval domain")
    }

    fn zero_plot() -> Plot<ZeroDCoord> {
        Plot::<ZeroDCoord>::new()
    }

    fn keyed_hconcat(left_key: &str, right_key: &str) -> Plot<HConcat> {
        Plot::<HConcat>::new()
            .mark(Subplot::new(zero_plot()).key(left_key))
            .mark(Subplot::new(zero_plot()).key(right_key))
    }

    #[tokio::test]
    async fn hconcat_measurement_exposes_child_frame_container_view()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .key("left")
                    .label("Left"),
            )
            .mark(
                Subplot::new(Plot::<ZeroDCoord>::new())
                    .key("right")
                    .label("Right"),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        assert_eq!(
            concat.child_band_layout.direction,
            BandDirection::Horizontal
        );
        assert_eq!(concat.children.len(), 2);
        assert_eq!(concat.children[0].key.as_deref(), Some("left"));
        assert_eq!(concat.children[1].key.as_deref(), Some("right"));
        let left_scope = concat
            .child_scope_key(0)
            .expect("left child should have a scope key");
        assert!(left_scope.container_path.is_empty());
        assert_eq!(
            &left_scope.child_key,
            &ChildFrameKey::ConcatChild {
                index: 0,
                key: Some("left".to_string())
            }
        );

        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert_eq!(container.placement().render_placements().len(), 2);
        assert_eq!(
            container.placement().render_placements()[0].origin,
            [0.0, 0.0]
        );
        assert!(container.child_measurement(0).is_some());
        assert!(container.child_measurement(1).is_some());
        assert_eq!(container.child_scope_key(0), Some(&left_scope));
        assert_eq!(container.child_label(0), Some("Left"));
        assert_eq!(container.child_label(1), Some("Right"));
        assert_eq!(container.child_scope_keys().count(), 2);

        let direct_container = child_frame_container_view_from_concat(concat)?;
        assert_eq!(direct_container.placement(), container.placement());
        let label_items = container_label_items_from_child_frame_container(&direct_container)?;
        assert_eq!(label_items.len(), 2);
        assert_eq!(label_items[0].text, "Left");
        assert_eq!(label_items[1].text, "Right");
        Ok(())
    }

    #[tokio::test]
    async fn concat_coordination_scope_hooks_group_children_by_container()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("left"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 200.0, 100.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_scope = concat
            .child_scope_key(0)
            .expect("left child should have a scope key");
        let right_scope = concat
            .child_scope_key(1)
            .expect("right child should have a scope key");

        let shared_left =
            CoordinationScopeKey::child_frame_container(CoordinationKind::ScaleDomain, &left_scope)
                .with_channel("x");
        let shared_right = CoordinationScopeKey::child_frame_container(
            CoordinationKind::ScaleDomain,
            &right_scope,
        )
        .with_channel("x");
        let free_left =
            CoordinationScopeKey::child_frame(CoordinationKind::ScaleDomain, &left_scope)
                .with_channel("x");
        let free_right =
            CoordinationScopeKey::child_frame(CoordinationKind::ScaleDomain, &right_scope)
                .with_channel("x");

        assert_eq!(shared_left, shared_right);
        assert_ne!(free_left, free_right);

        let legend_left = CoordinationScopeKey::child_frame_container(
            CoordinationKind::LegendOwnership,
            &left_scope,
        )
        .with_channel("fill:Right");
        let legend_right = CoordinationScopeKey::child_frame_container(
            CoordinationKind::LegendOwnership,
            &right_scope,
        )
        .with_channel("fill:Right");
        assert_eq!(legend_left, legend_right);

        let horizontal_lane = CoordinationScopeKey::container_lane(
            CoordinationKind::GuideLane,
            CoordinationAxis::Horizontal,
            vec![],
        );
        let vertical_lane = CoordinationScopeKey::container_lane(
            CoordinationKind::GuideLane,
            CoordinationAxis::Vertical,
            vec![],
        );
        assert_ne!(horizontal_lane, vertical_lane);
        Ok(())
    }

    #[tokio::test]
    async fn nested_concat_child_scopes_include_outer_concat_child() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(keyed_hconcat("inner-left", "inner-right")).key("outer-left"))
            .mark(Subplot::new(keyed_hconcat("inner-left", "inner-right")).key("outer-right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 120.0, &ctx).await?;
        let outer = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("outer HConcat should measure as ConcatCoordMeasurement");
        let left_inner = outer.children()[0]
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("left child should contain an inner concat measurement");
        let right_inner = outer.children()[1]
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("right child should contain an inner concat measurement");

        let left_inner_scope = left_inner
            .child_scope_key(0)
            .expect("left inner child should have a scope key");
        let right_inner_scope = right_inner
            .child_scope_key(0)
            .expect("right inner child should have a scope key");
        assert_eq!(
            left_inner_scope.container_path,
            vec![ContainerPathSegment::concat_child(0, Some("outer-left"))]
        );
        assert_eq!(
            right_inner_scope.container_path,
            vec![ContainerPathSegment::concat_child(1, Some("outer-right"))]
        );
        assert_ne!(left_inner_scope, right_inner_scope);
        Ok(())
    }

    #[tokio::test]
    async fn facet_child_scopes_include_existing_concat_container_path()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<FacetColumn>::new()
            .data(grouped_xy_dataframe(&ctx))
            .mark(
                Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).column(col("group")),
            )
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot_with_container_path(
            &compiled,
            320.0,
            120.0,
            &ctx,
            &[ContainerPathSegment::concat_child(0, Some("faceted-child"))],
        )
        .await?;
        let facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let first_cell_scope = facet
            .child_scope_key(0)
            .expect("facet cell should have a scope key");
        assert_eq!(
            first_cell_scope.container_path,
            vec![ContainerPathSegment::concat_child(0, Some("faceted-child"))]
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_inside_facet_child_scopes_include_outer_facet_cell()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let concat_child = keyed_hconcat("inner-left", "inner-right");
        let compiled = Plot::<FacetColumn>::new()
            .data(grouped_xy_dataframe(&ctx))
            .mark(Subplot::new(concat_child).column(col("group")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 120.0, &ctx).await?;
        let facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let first_cell = facet
            .cells
            .first()
            .expect("facet should have at least one cell");
        let inner_concat = first_cell
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("facet cell should contain a concat measurement");
        let inner_scope = inner_concat
            .child_scope_key(0)
            .expect("inner concat child should have a scope key");
        assert_eq!(
            inner_scope.container_path,
            vec![ContainerPathSegment::facet_value(
                FacetAxis::Column,
                1,
                first_cell.plan.value.clone(),
            )]
        );
        Ok(())
    }

    #[tokio::test]
    async fn nested_facet_child_scopes_do_not_duplicate_outer_facet_cell()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let inner = Plot::<FacetRow>::new().mark(
            Subplot::new(Plot::<Cartesian>::new().mark(line_mark(false))).row(col("subgroup")),
        );
        let compiled = Plot::<FacetColumn>::new()
            .data(nested_grouped_xy_dataframe(&ctx))
            .mark(Subplot::new(inner).column(col("group")))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 320.0, 160.0, &ctx).await?;
        let outer_facet = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("outer FacetColumn should measure as FacetBandCoordMeasurement");
        let outer_cell = outer_facet
            .cells
            .first()
            .expect("outer facet should have at least one cell");
        let inner_facet = outer_cell
            .measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("outer facet cell should contain an inner facet measurement");
        let inner_scope = inner_facet
            .child_scope_key(0)
            .expect("inner facet cell should have a scope key");

        assert_eq!(
            inner_scope.container_path,
            vec![ContainerPathSegment::facet_value(
                FacetAxis::Column,
                1,
                outer_cell.plan.value.clone(),
            )]
        );
        assert_eq!(
            &inner_scope.child_key,
            &ChildFrameKey::FacetValue {
                axis: FacetAxis::Row,
                level: 2,
                value: inner_facet.cells[0].plan.value.clone(),
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn vconcat_measurement_places_children_on_vertical_band() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let compiled = Plot::<VConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("top"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("bottom"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 120.0, 200.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("VConcat should measure as ConcatCoordMeasurement");
        assert_eq!(concat.child_band_layout.direction, BandDirection::Vertical);

        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert_eq!(
            container.placement().render_placements()[0].origin,
            [0.0, 0.0]
        );
        assert_eq!(container.placement().render_placements()[1].origin[0], 0.0);
        assert!(container.placement().render_placements()[1].origin[1] > 0.0);
        Ok(())
    }

    #[tokio::test]
    async fn concat_measurement_preserves_sparse_mark_indexes() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("first"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 100.0, 100.0, &ctx).await?;
        let container = measurement
            .child_frame_container_view()?
            .expect("concat measurement should expose a child-frame container view");
        assert!(container.child_measurement(0).is_some());
        assert!(container.child_measurement(1).is_none());
        Ok(())
    }

    #[tokio::test]
    async fn hconcat_renders_child_subplot_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("left"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("right"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let left_path = group_names
            .get("concat_subplot_0_left")
            .expect("left subplot group should render");
        let right_path = group_names
            .get("concat_subplot_1_right")
            .expect("right subplot group should render");
        let left_origin = evaluated
            .scene_graph
            .get_absolute_origin(left_path)
            .expect("left subplot should have an absolute origin");
        let right_origin = evaluated
            .scene_graph
            .get_absolute_origin(right_path)
            .expect("right subplot should have an absolute origin");

        assert_eq!(left_origin[1], right_origin[1]);
        assert!(
            right_origin[0] > left_origin[0],
            "horizontal concat should place the second subplot to the right"
        );
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_rendering_inherits_parent_data() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![3.0, 4.0]);
        let child_plot = Plot::<Cartesian>::new().mark(Symbol::new().x(col("x")).y(col("y")));
        let compiled = Plot::<HConcat>::new()
            .data(data)
            .mark(Subplot::new(child_plot).key("points"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let child_path = group_names
            .get("concat_subplot_0_points")
            .expect("child subplot group should render");
        let child_group = evaluated
            .scene_graph
            .get_mark(child_path)
            .expect("child subplot group path should resolve");
        let symbol = find_symbol_mark(child_group).expect("child subplot should render symbols");

        assert_eq!(symbol.len, 2);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_rendering_preserves_explicit_child_data() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let parent_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![3.0, 4.0]);
        let child_data = xy_dataframe(&ctx, vec![5.0], vec![6.0]);
        let child_plot = Plot::<Cartesian>::new()
            .data(child_data)
            .mark(Symbol::new().x(col("x")).y(col("y")));
        let compiled = Plot::<HConcat>::new()
            .data(parent_data)
            .mark(Subplot::new(child_plot).key("points"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let child_path = group_names
            .get("concat_subplot_0_points")
            .expect("child subplot group should render");
        let child_group = evaluated
            .scene_graph
            .get_mark(child_path)
            .expect("child subplot group path should resolve");
        let symbol = find_symbol_mark(child_group).expect("child subplot should render symbols");

        assert_eq!(symbol.len, 1);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_domains_are_free_by_default() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(child_scatter_plot(left_data, false)).key("left"))
            .mark(Subplot::new(child_scatter_plot(right_data, false)).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 160.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_domain = child_x_domain(&concat.children()[0]);
        let right_domain = child_x_domain(&concat.children()[1]);

        assert_ne!(left_domain, right_domain);
        assert!(left_domain.1 < 50.0);
        assert!(right_domain.1 > 50.0);
        Ok(())
    }

    #[tokio::test]
    async fn concat_child_domains_share_when_channel_requests_sharing()
    -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(child_scatter_plot(left_data, true)).key("left"))
            .mark(Subplot::new(child_scatter_plot(right_data, true)).key("right"))
            .compile(&ctx)
            .await?;

        let measurement = measurement_for_plot(&compiled, 400.0, 160.0, &ctx).await?;
        let concat = measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let left_domain = child_x_domain(&concat.children()[0]);
        let right_domain = child_x_domain(&concat.children()[1]);

        assert_eq!(left_domain, right_domain);
        assert!(left_domain.0 <= 1.0);
        assert!(left_domain.1 >= 101.0);
        Ok(())
    }

    #[tokio::test]
    async fn concat_and_facet_shared_domains_use_equivalent_unions() -> Result<(), AvengerChartError>
    {
        let ctx = SessionContext::new();
        let left_data = xy_dataframe(&ctx, vec![1.0, 2.0], vec![1.0, 2.0]);
        let right_data = xy_dataframe(&ctx, vec![100.0, 101.0], vec![1.0, 2.0]);
        let concat_compiled = Plot::<HConcat>::new()
            .mark(Subplot::new(line_child_plot(left_data, true)).key("left"))
            .mark(Subplot::new(line_child_plot(right_data, true)).key("right"))
            .compile(&ctx)
            .await?;

        let grouped_data = grouped_xy_dataframe(&ctx);
        let facet_compiled = Plot::<FacetColumn>::new()
            .data(grouped_data)
            .mark(Subplot::new(Plot::<Cartesian>::new().mark(line_mark(true))).column(col("group")))
            .compile(&ctx)
            .await?;

        let concat_measurement = measurement_for_plot(&concat_compiled, 400.0, 160.0, &ctx).await?;
        let concat = concat_measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<ConcatCoordMeasurement>()
            .expect("HConcat should measure as ConcatCoordMeasurement");
        let concat_domains = concat
            .children()
            .iter()
            .map(child_x_domain)
            .collect::<Vec<_>>();

        let facet_measurement = measurement_for_plot(&facet_compiled, 400.0, 160.0, &ctx).await?;
        let facet = facet_measurement
            .coord_measurement
            .as_any()
            .downcast_ref::<FacetBandCoordMeasurement>()
            .expect("FacetColumn should measure as FacetBandCoordMeasurement");
        let facet_domains = facet
            .cells
            .iter()
            .map(|cell| measurement_x_domain(&cell.measurement))
            .collect::<Vec<_>>();

        assert_eq!(concat_domains.len(), 2);
        assert_eq!(facet_domains.len(), 2);
        assert_eq!(concat_domains[0], concat_domains[1]);
        assert_eq!(facet_domains[0], facet_domains[1]);
        assert_eq!(concat_domains[0], facet_domains[0]);
        assert_eq!(concat_domains[0], (1.0, 101.0));
        Ok(())
    }

    #[tokio::test]
    async fn vconcat_renders_child_subplot_groups() -> Result<(), AvengerChartError> {
        let ctx = SessionContext::new();
        let compiled = Plot::<VConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("top"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("bottom"))
            .compile(&ctx)
            .await?;

        let evaluated = compiled.evaluate(&ctx, None).await?;
        let group_names = evaluated.scene_graph.group_names();
        let top_path = group_names
            .get("concat_subplot_0_top")
            .expect("top subplot group should render");
        let bottom_path = group_names
            .get("concat_subplot_1_bottom")
            .expect("bottom subplot group should render");
        let top_origin = evaluated
            .scene_graph
            .get_absolute_origin(top_path)
            .expect("top subplot should have an absolute origin");
        let bottom_origin = evaluated
            .scene_graph
            .get_absolute_origin(bottom_path)
            .expect("bottom subplot should have an absolute origin");

        assert_eq!(top_origin[0], bottom_origin[0]);
        assert!(
            bottom_origin[1] > top_origin[1],
            "vertical concat should place the second subplot below the first"
        );
        Ok(())
    }
}
