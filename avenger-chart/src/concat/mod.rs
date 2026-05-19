//! Concatenation coordinate systems.
//!
//! `HConcat` and `VConcat` are container coordinate systems: their marks are
//! child `Subplot` marks, and their coordinate measurement produces child-frame
//! placement metadata for later rendering/debug consumers.

use std::{any::Any, collections::HashMap, sync::Arc};

use avenger_common::value::{ScalarOrArray, ScalarOrArrayValue};
use avenger_scales::scales::ScaleImpl;
use datafusion::{common::ScalarValue, dataframe::DataFrame};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    coords::{
        CoordMeasurement, CoordinateSystem, CoordinateSystemTransform, PlotGeometry, PointGeometry,
    },
    error::AvengerChartError,
    guide::NoGuide,
    layout::{
        BandChildFrameInput, BandChildFramePlacement, BandDirection, BandSpacing, BoundaryDemand1D,
        ChildFramePlacementResult, EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode,
        Size2D,
    },
    marks::{
        CompiledMark, CompiledSubplot, SubplotContainerCoordinateSystem, subplot::compiled_subplot,
    },
    plot::compiled::{
        ComponentsMeasurement, scale_provider::DynamicScaleProvider,
        scales::build_scale_builder_from_marks,
    },
    render::EvaluationContext,
    scales::{ConfiguredScaleWithSpec, ScaleRangeBinding},
};

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

impl SubplotContainerCoordinateSystem for HConcat {}
impl SubplotContainerCoordinateSystem for VConcat {}

impl CoordinateSystem for HConcat {
    type Guide = NoGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystem for VConcat {
    type Guide = NoGuide;

    fn required_channels(&self) -> &'static [&'static str] {
        &[]
    }

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
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
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Horizontal,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
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
        _scales: &HashMap<String, ConfiguredScaleWithSpec>,
        plot_width: f32,
        plot_height: f32,
        eval_ctx: &EvaluationContext,
        data: Option<&DataFrame>,
        compiled_marks: &[Arc<dyn CompiledMark>],
        facet_path: &[ScalarValue],
    ) -> Result<Box<dyn CoordMeasurement>, AvengerChartError> {
        measure_concat_coord_system(
            BandDirection::Vertical,
            plot_width,
            plot_height,
            eval_ctx,
            data,
            compiled_marks,
            facet_path,
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
    pub(crate) measurement: ComponentsMeasurement,
}

impl ConcatChildMeasurement {
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

pub(crate) fn concat_coord_ref(
    coord_measurement: &dyn CoordMeasurement,
) -> Option<&ConcatCoordMeasurement> {
    coord_measurement
        .as_any()
        .downcast_ref::<ConcatCoordMeasurement>()
}

fn fixed_plot_area_layout_spec(width: f32, height: f32) -> EvaluatedLayoutSpec {
    EvaluatedLayoutSpec {
        canvas: EvaluatedSizeMode::Auto,
        plot_area: EvaluatedSizeMode::Fixed {
            width: width.max(1.0),
            height: height.max(1.0),
        },
        margins: EvaluatedMargins {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        },
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

async fn measure_concat_child(
    subplot: &CompiledSubplot,
    child_plot_area: Size2D,
    eval_ctx: &EvaluationContext,
    inherited_data: Option<&DataFrame>,
    facet_path: &[ScalarValue],
) -> Result<ConcatChildMeasurement, AvengerChartError> {
    let child_plot = subplot.compiled_subplot();
    let child_data_override = if subplot.inherits_parent_data() {
        inherited_data
    } else {
        None
    };
    let child_layout_spec =
        fixed_plot_area_layout_spec(child_plot_area.width, child_plot_area.height);
    let child_params: IndexMap<String, ScalarValue> = eval_ctx.params.clone();
    let scale_builder = build_scale_builder_from_marks(
        &child_plot.marks,
        &child_plot.scale_specs,
        &child_plot.coord_transform,
        &child_plot.data,
        child_data_override.cloned(),
        eval_ctx.session_context.as_ref(),
        &child_params,
        child_plot.get_theme().as_ref(),
    )
    .await?;
    let scale_provider = DynamicScaleProvider {
        builder: &scale_builder,
        plot: child_plot,
    };
    let measurement = child_plot
        .measure_plot_components(
            eval_ctx,
            &child_layout_spec,
            &scale_provider,
            child_data_override,
            facet_path,
        )
        .await?;

    Ok(ConcatChildMeasurement {
        child_index: subplot.child_index(),
        key: subplot.key().map(ToOwned::to_owned),
        label: subplot.label().map(ToOwned::to_owned),
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

    let mut children = Vec::with_capacity(subplots.len());
    for subplot in subplots {
        children.push(
            measure_concat_child(subplot, child_plot_area, eval_ctx, data, facet_path).await?,
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

    use datafusion::prelude::SessionContext;

    use super::*;
    use crate::{
        facet::evaluated_facet_tree::EvaluatedFacetTree,
        layout::{EvaluatedLayoutSpec, EvaluatedMargins, EvaluatedSizeMode},
        marks::Subplot,
        plot::{CompiledPlot, Plot, compiled::scales::build_scale_builder_from_marks},
        render::EvaluationContext,
        zerod::ZeroDCoord,
    };

    async fn measurement_for_plot(
        compiled: &CompiledPlot,
        plot_width: f32,
        plot_height: f32,
        ctx: &SessionContext,
    ) -> Result<ComponentsMeasurement, AvengerChartError> {
        let params = compiled.get_default_params().clone();
        let eval_ctx = EvaluationContext::new(
            compiled.get_theme(),
            Arc::new(ctx.clone()),
            params.clone(),
            Arc::new(EvaluatedFacetTree::empty()),
        );
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
            ctx,
            &params,
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

    #[tokio::test]
    async fn hconcat_measurement_exposes_child_frame_container_view()
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
        assert_eq!(
            concat.child_band_layout.direction,
            BandDirection::Horizontal
        );
        assert_eq!(concat.children.len(), 2);
        assert_eq!(concat.children[0].key.as_deref(), Some("left"));
        assert_eq!(concat.children[1].key.as_deref(), Some("right"));

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
}
