use std::{any::Any, collections::HashMap, marker::PhantomData, sync::Arc};

use avenger_scales::scales::{ConfiguredScale, ScaleImpl};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use datafusion::{
    arrow::record_batch::RecordBatch, dataframe::DataFrame, logical_expr::Expr,
    prelude::SessionContext,
};
use serde::{Deserialize, Serialize};

use crate::{
    cartesian::{
        Cartesian, CartesianPositionConfig, positioned_subplot::cartesian_positioned_coord_ref,
    },
    concat::{HConcat, VConcat, concat_coord_ref},
    coords::{CoordinateSystem, CoordinateSystemTransform},
    error::AvengerChartError,
    facet::dimension_config::{ColumnDimensionConfig, FacetDimensionConfig, RowDimensionConfig},
    layout::BandDirection,
    legend::LegendRenderer,
    marks::{
        ChannelDescriptor, ChannelValue, CompiledDataContext, CompiledMark, CompiledMarkState,
        DataContext, FacetStrategy, Mark, MarkState, RadiusExpression,
    },
    plot::{CompiledPlot, Plot, compiled::ChildFrameSharingLevel},
    render::RenderContext,
    scales::{ResolvedDomain, ScaleRange, ScaleSpec},
    theme::Theme,
};

/// Data source selected for a compiled subplot's child plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubplotDataSource {
    /// The child plot has explicit plot-level data.
    ExplicitChild,
    /// The child plot has no plot-level data and should inherit container data.
    InheritParent,
}

#[async_trait::async_trait]
pub(crate) trait SubplotPlotSpec: Send + Sync {
    fn clone_box(&self) -> Box<dyn SubplotPlotSpec>;
    fn has_plot_level_data(&self) -> bool;
    async fn compile_boxed(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<CompiledPlot>, AvengerChartError>;
}

impl Clone for Box<dyn SubplotPlotSpec> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

#[async_trait::async_trait]
impl<C> SubplotPlotSpec for Plot<C>
where
    C: CoordinateSystem + Clone + 'static,
{
    fn clone_box(&self) -> Box<dyn SubplotPlotSpec> {
        Box::new(self.clone())
    }

    fn has_plot_level_data(&self) -> bool {
        self.data.is_some()
    }

    async fn compile_boxed(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<CompiledPlot>, AvengerChartError> {
        Ok(Arc::new(self.clone().compile(session_context).await?))
    }
}

#[derive(Clone, Default)]
pub(crate) struct SubplotConfig {
    pub(crate) label: Option<String>,
    pub(crate) key: Option<String>,
    pub(crate) plot_width: Option<f32>,
    pub(crate) plot_height: Option<f32>,
    pub(crate) facet_row_title: Option<String>,
    pub(crate) facet_col_title: Option<String>,
    pub(crate) facet_row_slot_sharing: Option<crate::channel::config_traits::ScaleSharing>,
    pub(crate) facet_col_slot_sharing: Option<crate::channel::config_traits::ScaleSharing>,
    pub(crate) facet_row_position: Option<String>,
    pub(crate) facet_col_position: Option<String>,
    pub(crate) facet_row_empty_cell_policy:
        Option<crate::facet::empty_cell_policy::FacetEmptyCellPolicy>,
    pub(crate) facet_col_empty_cell_policy:
        Option<crate::facet::empty_cell_policy::FacetEmptyCellPolicy>,
}

/// Mark that owns one child plot inside an outer coordinate system.
///
/// `Subplot<OuterC>` is parameterized by the coordinate system that positions
/// the child plot, not by the child plot's own coordinate system. This keeps
/// concat, facet, and future coordinate-positioned subplots under one public
/// mark concept while still allowing mixed child coordinate systems.
#[derive(Clone)]
pub struct Subplot<OuterC: CoordinateSystem> {
    state: MarkState,
    subplot: Box<dyn SubplotPlotSpec>,
    config: SubplotConfig,
    _outer: PhantomData<fn() -> OuterC>,
}

impl<OuterC: CoordinateSystem> Subplot<OuterC> {
    pub fn new<C>(subplot: Plot<C>) -> Self
    where
        C: CoordinateSystem + Clone + 'static,
    {
        Self {
            state: MarkState {
                data: DataContext::default(),
                facet_strategy: FacetStrategy::Filter,
                details: None,
                zindex: None,
                axis_configs: HashMap::new(),
            },
            subplot: Box::new(subplot),
            config: SubplotConfig::default(),
            _outer: PhantomData,
        }
    }

    /// Set explicit data for this subplot mark.
    pub fn data(mut self, dataframe: datafusion::dataframe::DataFrame) -> Self {
        self.state.data = DataContext::new(dataframe);
        self
    }

    /// Set zindex.
    pub fn zindex(mut self, zindex: i32) -> Self {
        self.state.zindex = Some(zindex);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.config.label = Some(label.into());
        self
    }

    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.config.key = Some(key.into());
        self
    }

    pub(crate) fn state_ref(&self) -> &MarkState {
        &self.state
    }

    pub(crate) fn state_mut_ref(&mut self) -> &mut MarkState {
        &mut self.state
    }

    pub(crate) fn data_context_ref(&self) -> &DataContext {
        &self.state.data
    }

    pub(crate) fn config(&self) -> &SubplotConfig {
        &self.config
    }

    pub(crate) fn config_mut(&mut self) -> &mut SubplotConfig {
        &mut self.config
    }

    pub(crate) fn has_plot_level_data(&self) -> bool {
        self.subplot.has_plot_level_data()
    }

    pub(crate) async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<CompiledPlot>, AvengerChartError> {
        self.subplot.compile_boxed(session_context).await
    }

    pub(crate) fn with_channel_value(
        mut self,
        channel_name: &'static str,
        value: ChannelValue,
    ) -> Self {
        self.state.data = self.state.data.with_channel_value(channel_name, value);
        self
    }

    fn validate_no_facet_channels(&self, outer_label: &str) -> Result<(), AvengerChartError> {
        let channels = self.state.data.channels();
        for channel_name in [
            RowDimensionConfig::channel_name(),
            ColumnDimensionConfig::channel_name(),
        ] {
            if channels.contains_key(channel_name) {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "{outer_label} subplots do not support facet channel `{channel_name}`"
                )));
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl Mark<HConcat> for Subplot<HConcat> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.validate_no_facet_channels("HConcat")?;
        let data_source = if self.has_plot_level_data() {
            SubplotDataSource::ExplicitChild
        } else {
            SubplotDataSource::InheritParent
        };
        let compiled_subplot = self.compile_child_plot(session_context).await?;

        Ok(Arc::new(CompiledConcatSubplot {
            state: compiled_state,
            compiled_subplot,
            label: self.config.label.clone(),
            key: self.config.key.clone(),
            data_source,
        }))
    }
}

#[async_trait::async_trait]
impl Mark<VConcat> for Subplot<VConcat> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.validate_no_facet_channels("VConcat")?;
        let data_source = if self.has_plot_level_data() {
            SubplotDataSource::ExplicitChild
        } else {
            SubplotDataSource::InheritParent
        };
        let compiled_subplot = self.compile_child_plot(session_context).await?;

        Ok(Arc::new(CompiledConcatSubplot {
            state: compiled_state,
            compiled_subplot,
            label: self.config.label.clone(),
            key: self.config.key.clone(),
            data_source,
        }))
    }
}

impl Subplot<Cartesian> {
    /// Set the parent x-position for coordinate-positioned child plot frames.
    pub fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    /// Set the parent y-position for coordinate-positioned child plot frames.
    pub fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    /// Configure the parent x-position channel for coordinate-positioned child plot frames.
    pub fn x_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.with_position_config("x", value.into(), f)
    }

    /// Configure the parent y-position channel for coordinate-positioned child plot frames.
    pub fn y_with<V, F>(self, value: V, f: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        self.with_position_config("y", value.into(), f)
    }

    /// Set the child plot-area width used for each positioned child frame.
    pub fn plot_width(mut self, width: f32) -> Self {
        self.config.plot_width = Some(width);
        self
    }

    /// Set the child plot-area height used for each positioned child frame.
    pub fn plot_height(mut self, height: f32) -> Self {
        self.config.plot_height = Some(height);
        self
    }

    /// Set both child plot-area dimensions used for each positioned child frame.
    pub fn plot_size(mut self, width: f32, height: f32) -> Self {
        self.config.plot_width = Some(width);
        self.config.plot_height = Some(height);
        self
    }

    fn with_position_config<F>(self, channel: &'static str, value: ChannelValue, f: F) -> Self
    where
        F: FnOnce(CartesianPositionConfig) -> CartesianPositionConfig,
    {
        use crate::channel::PositionConfig;

        let configured = f(CartesianPositionConfig::new(value));
        let (channel_value, axis_config) = configured.take_axis_config();
        let mut mark = self.with_channel_value(channel, channel_value);
        if let Some(axis_config) = axis_config {
            mark.state
                .axis_configs
                .insert(channel.to_string(), Arc::new(axis_config));
        }
        mark
    }
}

#[async_trait::async_trait]
impl Mark<Cartesian> for Subplot<Cartesian> {
    fn state(&self) -> &MarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut MarkState {
        &mut self.state
    }

    fn data_context(&self) -> &DataContext {
        &self.state.data
    }

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        self.validate_no_facet_channels("Cartesian")?;
        let data_source = if self.has_plot_level_data() {
            SubplotDataSource::ExplicitChild
        } else {
            SubplotDataSource::InheritParent
        };
        let compiled_subplot = self.compile_child_plot(session_context).await?;

        Ok(Arc::new(CompiledCartesianSubplot {
            state: compiled_state,
            compiled_subplot,
            label: self.config.label.clone(),
            key: self.config.key.clone(),
            data_source,
            plot_width: self.config.plot_width.unwrap_or(80.0).max(1.0),
            plot_height: self.config.plot_height.unwrap_or(80.0).max(1.0),
        }))
    }
}

/// Compiled child-plot mark for container coordinate systems.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledConcatSubplot {
    state: CompiledMarkState,
    compiled_subplot: Arc<CompiledPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
}

impl CompiledConcatSubplot {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn data_source(&self) -> SubplotDataSource {
        self.data_source
    }

    pub fn child_index(&self) -> usize {
        self.state.mark_index()
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.data_source == SubplotDataSource::ExplicitChild
    }

    fn group_name(&self) -> String {
        match self.key() {
            Some(key) => format!("concat_subplot_{}_{}", self.child_index(), key),
            None => format!("concat_subplot_{}", self.child_index()),
        }
    }

    fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        context: &RenderContext<'_>,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        if !self.inherits_parent_data() {
            return Ok(None);
        }

        data.map(|batch| {
            context
                .session_context()
                .read_batch(batch.clone())
                .map_err(AvengerChartError::DataFusionError)
        })
        .transpose()
    }
}

pub fn compiled_subplot(mark: &dyn CompiledMark) -> Option<&CompiledConcatSubplot> {
    if mark.mark_type() != "subplot" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledConcatSubplot>()
}

/// Compiled child-plot mark positioned by Cartesian x/y channels.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledCartesianSubplot {
    state: CompiledMarkState,
    compiled_subplot: Arc<CompiledPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
    plot_width: f32,
    plot_height: f32,
}

impl CompiledCartesianSubplot {
    pub fn compiled_subplot(&self) -> &Arc<CompiledPlot> {
        &self.compiled_subplot
    }

    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }

    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    pub fn mark_index(&self) -> usize {
        self.state.mark_index()
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    fn group_name(&self, child_index: usize) -> String {
        match self.key() {
            Some(key) => format!(
                "cartesian_subplot_{}_{}_{}",
                self.mark_index(),
                child_index,
                key
            ),
            None => format!("cartesian_subplot_{}_{}", self.mark_index(), child_index),
        }
    }
}

pub fn compiled_cartesian_subplot(mark: &dyn CompiledMark) -> Option<&CompiledCartesianSubplot> {
    if mark.mark_type() != "subplot" {
        return None;
    }
    mark.as_any().downcast_ref::<CompiledCartesianSubplot>()
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledConcatSubplot {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        Vec::new()
    }

    fn wants_full_data_batch(&self) -> bool {
        true
    }

    async fn render_from_data(
        &self,
        data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let concat_measurement =
            concat_coord_ref(context.coord_measurement()).ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Subplot marks require ConcatCoordMeasurement in coord_measurement".to_string(),
                )
            })?;
        let child = concat_measurement
            .child(self.child_index())
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing concat child measurement for subplot child index {}",
                    self.child_index()
                ))
            })?;
        let child_frame_placement = concat_measurement.child_frame_placement();
        let render_placement =
            child_frame_placement
                .child(self.child_index())
                .ok_or_else(|| {
                    AvengerChartError::InternalError(format!(
                        "Missing concat child-frame placement for subplot child index {}",
                        self.child_index()
                    ))
                })?;

        let mut params = self.compiled_subplot.get_default_params().clone();
        params.extend(context.eval.params.clone());
        let child_count = concat_measurement.children().len();
        let sharing_level = match concat_measurement.child_band_layout.direction {
            BandDirection::Horizontal => {
                ChildFrameSharingLevel::hconcat_child(self.child_index(), child_count, self.key())
            }
            BandDirection::Vertical => {
                ChildFrameSharingLevel::vconcat_child(self.child_index(), child_count, self.key())
            }
        };
        let child_eval_ctx = context
            .eval
            .with_params(params)
            .with_child_frame_sharing_level_appended(sharing_level);
        let data_override = self.inherited_data_override(data, context)?;
        let components = self
            .compiled_subplot
            .build_plot_components(
                &child_eval_ctx,
                &child.measurement,
                data_override.as_ref(),
                true,
                context.facet_path,
            )
            .await?;

        let data_marks_group = SceneGroup {
            origin: [0.0, 0.0],
            marks: components.data_marks,
            clip: components.clip,
            zindex: Some(0),
            ..Default::default()
        };
        let mut all_marks = vec![SceneMark::Group(data_marks_group)];
        all_marks.extend(components.guide_marks);
        all_marks.extend(components.legend_marks);
        all_marks.extend(components.title_marks);
        all_marks.extend(components.subtitle_marks);
        all_marks.extend(components.debug_marks);

        Ok(vec![SceneMark::Group(SceneGroup {
            name: self.group_name(),
            origin: render_placement.origin,
            clip: avenger_scenegraph::marks::group::Clip::None,
            marks: all_marks,
            gradients: Vec::new(),
            fill: None,
            stroke: None,
            stroke_width: None,
            stroke_offset: None,
            zindex: None,
        })])
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<Arc<dyn LegendRenderer>> {
        None
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }

    fn preferred_scale_type(
        &self,
        _channel: &str,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> Option<Box<dyn ScaleSpec>> {
        None
    }

    fn default_scale_options(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _data_type: &datafusion::arrow::datatypes::DataType,
    ) -> HashMap<String, Expr> {
        HashMap::new()
    }

    fn default_channel_range(
        &self,
        _channel: &str,
        _scale_impl: &dyn ScaleImpl,
        _domain: &ResolvedDomain,
        _data_type: &datafusion::arrow::datatypes::DataType,
        _theme: &Theme,
        _params: &indexmap::IndexMap<String, datafusion::scalar::ScalarValue>,
    ) -> Option<ScaleRange> {
        None
    }
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledCartesianSubplot {
    fn state(&self) -> &CompiledMarkState {
        &self.state
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.state.data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        vec![
            ChannelDescriptor {
                name: "x",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
            ChannelDescriptor {
                name: "y",
                required: true,
                default_value: None,
                allow_column_ref: true,
            },
        ]
    }

    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &RenderContext,
        _coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        let cartesian_measurement = cartesian_positioned_coord_ref(context.coord_measurement())
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Cartesian subplot marks require CartesianPositionedCoordMeasurement"
                        .to_string(),
                )
            })?;
        let child_frame_placement = cartesian_measurement.child_frame_placement();
        let child_count = cartesian_measurement.children().len();
        let mut marks = Vec::new();

        for child in cartesian_measurement.children_for_mark(self.mark_index()) {
            let render_placement =
                child_frame_placement
                    .child(child.child_index)
                    .ok_or_else(|| {
                        AvengerChartError::InternalError(format!(
                            "Missing Cartesian child-frame placement for child index {}",
                            child.child_index
                        ))
                    })?;

            let mut params = self.compiled_subplot.get_default_params().clone();
            params.extend(context.eval.params.clone());
            let sharing_level = ChildFrameSharingLevel::positioned_subplot(
                child.child_index,
                child_count,
                child.mark_index,
                child.row_index,
                child.key.as_deref(),
            );
            let child_eval_ctx = context
                .eval
                .with_params(params)
                .with_child_frame_sharing_level_appended(sharing_level);
            let components = self
                .compiled_subplot
                .build_plot_components(
                    &child_eval_ctx,
                    &child.measurement,
                    child.data_override.as_ref(),
                    true,
                    context.facet_path,
                )
                .await?;

            let data_marks_group = SceneGroup {
                origin: [0.0, 0.0],
                marks: components.data_marks,
                clip: components.clip,
                zindex: Some(0),
                ..Default::default()
            };
            let mut all_marks = vec![SceneMark::Group(data_marks_group)];
            all_marks.extend(components.guide_marks);
            all_marks.extend(components.legend_marks);
            all_marks.extend(components.title_marks);
            all_marks.extend(components.subtitle_marks);
            all_marks.extend(components.debug_marks);

            marks.push(SceneMark::Group(SceneGroup {
                name: self.group_name(child.child_index),
                origin: render_placement.origin,
                clip: avenger_scenegraph::marks::group::Clip::None,
                marks: all_marks,
                gradients: Vec::new(),
                fill: None,
                stroke: None,
                stroke_width: None,
                stroke_offset: None,
                zindex: None,
            }));
        }

        Ok(marks)
    }

    fn preferred_legend_renderer(
        &self,
        _channel: &str,
        _scale: &ConfiguredScale,
    ) -> Option<Arc<dyn LegendRenderer>> {
        None
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use datafusion::{
        arrow::{array::Float32Array, record_batch::RecordBatch},
        prelude::{SessionContext, col},
    };

    use super::*;
    use crate::{
        concat::HConcat,
        facet::dimension_config::{FacetDimensionConfig, RowDimensionConfig},
        zerod::ZeroDCoord,
    };

    fn single_column_df(ctx: &SessionContext, value: f32) -> datafusion::dataframe::DataFrame {
        let batch = RecordBatch::try_from_iter(vec![(
            "x",
            Arc::new(Float32Array::from(vec![value])) as Arc<dyn datafusion::arrow::array::Array>,
        )])
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    #[tokio::test]
    async fn subplot_compilation_preserves_label_key_and_child_plot() {
        let ctx = SessionContext::new();
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new())
            .label("overview")
            .key("overview-key");

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.mark_type(), "subplot");
        assert_eq!(compiled.label(), Some("overview"));
        assert_eq!(compiled.key(), Some("overview-key"));
        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(compiled.inherits_parent_data());
        assert_eq!(compiled.compiled_subplot().marks().len(), 0);
    }

    #[tokio::test]
    async fn subplot_compilation_keeps_explicit_child_plot_data() {
        let ctx = SessionContext::new();
        let child_data = single_column_df(&ctx, 1.0);
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new().data(child_data));

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let compiled = <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled.as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::ExplicitChild);
        assert!(compiled.has_explicit_child_data());
        assert!(compiled.compiled_subplot().data.is_some());
    }

    #[tokio::test]
    async fn concat_subplot_rejects_facet_channels() {
        let ctx = SessionContext::new();
        let mut subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());
        subplot.state.data = subplot
            .state
            .data
            .with_channel_value(RowDimensionConfig::channel_name(), col("group").into());

        let compiled_state = CompiledMarkState::from_mark_state(&subplot.state, None);
        let result =
            <Subplot<HConcat> as Mark<HConcat>>::compile(&subplot, compiled_state, &ctx).await;

        assert!(matches!(result, Err(AvengerChartError::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn plot_compile_passes_parent_data_to_subplot_mark_state() {
        let ctx = SessionContext::new();
        let parent_data = single_column_df(&ctx, 2.0);
        let subplot = Subplot::<HConcat>::new(Plot::<ZeroDCoord>::new());

        let compiled_plot = Plot::<HConcat>::new()
            .data(parent_data)
            .mark(subplot)
            .compile(&ctx)
            .await
            .unwrap();
        let compiled = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();

        assert_eq!(compiled.data_source(), SubplotDataSource::InheritParent);
        assert!(
            compiled
                .compiled_state()
                .data
                .dataframe_with_context(&ctx)
                .is_some()
        );
        assert!(compiled.compiled_subplot().data.is_none());
    }

    #[tokio::test]
    async fn repeated_subplot_marks_receive_stable_child_indexes() {
        let ctx = SessionContext::new();
        let compiled_plot = Plot::<HConcat>::new()
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("first"))
            .mark(Subplot::new(Plot::<ZeroDCoord>::new()).key("second"))
            .compile(&ctx)
            .await
            .unwrap();

        let first = compiled_subplot(compiled_plot.marks()[0].as_ref()).unwrap();
        let second = compiled_subplot(compiled_plot.marks()[1].as_ref()).unwrap();

        assert_eq!(first.child_index(), 0);
        assert_eq!(second.child_index(), 1);
        assert_eq!(first.key(), Some("first"));
        assert_eq!(second.key(), Some("second"));
    }
}
