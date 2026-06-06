use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{
    arrow::record_batch::RecordBatch, dataframe::DataFrame, logical_expr::Expr,
    prelude::SessionContext,
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, AxisGuideVisibilityConfig, ChannelDescriptor, ColumnDimensionConfig,
    CompileContext, CompiledDataContext, CompiledMark, CompiledMarkCore, CompiledMarkState,
    CoordinateSystem, CoordinateSystemTransformCore, CoordinationScope, DataContext,
    DefaultLogicalExprNodeExt, FacetDimensionConfig, FacetEmptyCellPolicy, FacetWrapColumnMode,
    MarkRuntimeContext, RadiusExpression, RowDimensionConfig, SerializableExpr, contains_aggregate,
};

/// Data source selected for a compiled subplot's child plot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubplotDataSource {
    /// The child plot has explicit plot-level data.
    ExplicitChild,
    /// The child plot has no plot-level data and should inherit container data.
    InheritParent,
}

/// Serializable compiled child plot owned by a `Subplot` mark.
///
/// Core owns this erased handle so `Subplot` authoring and coordinate-specific
/// subplot compilation do not have to traffic in the top-level `CompiledPlot`
/// type. The facade still downcasts this handle for its built-in layout engine.
#[typetag::serde(tag = "type")]
pub trait CompiledSubplotChildPlot: Any + Send + Sync {
    /// Downcast support for the facade-owned layout/runtime engine.
    fn as_any(&self) -> &dyn Any;

    /// Convert an erased child plot handle into an erased `Any` handle for
    /// downcasting while preserving `Arc` ownership.
    fn into_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

#[async_trait::async_trait]
#[doc(hidden)]
pub trait SubplotChildPlotSpec: Send + Sync {
    fn clone_box(&self) -> Box<dyn SubplotChildPlotSpec>;
    fn has_plot_level_data(&self) -> bool;
    async fn compile_boxed(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError>;

    async fn compile_boxed_with_context(
        &self,
        session_context: &SessionContext,
        _compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.compile_boxed(session_context).await
    }
}

impl Clone for Box<dyn SubplotChildPlotSpec> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

/// Core view over a neutral subplot mark.
///
/// This lets shared subplot helpers live below the top-level facade without
/// making `avenger-chart-core` depend on the concrete `Subplot<C>` type.
#[async_trait::async_trait]
#[doc(hidden)]
pub trait SubplotMarkCore: Send + Sync {
    fn data_context_ref(&self) -> &DataContext;

    fn label_config(&self) -> Option<&str>;

    fn key_config(&self) -> Option<&str>;

    fn grid_row_config(&self) -> Option<usize> {
        None
    }

    fn grid_column_config(&self) -> Option<usize> {
        None
    }

    fn grid_row_span_config(&self) -> usize {
        1
    }

    fn grid_column_span_config(&self) -> usize {
        1
    }

    fn plot_width_config(&self) -> Option<f32> {
        None
    }

    fn plot_height_config(&self) -> Option<f32> {
        None
    }

    fn facet_row_title_config(&self) -> Option<&str> {
        None
    }

    fn facet_row_slot_sharing_config(&self) -> Option<CoordinationScope> {
        None
    }

    fn facet_row_position_config(&self) -> Option<&str> {
        None
    }

    fn facet_row_guide_visible_config(&self) -> Option<bool> {
        None
    }

    fn facet_row_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        None
    }

    fn facet_row_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        None
    }

    fn facet_row_order_expr_config(&self) -> Option<&LogicalExprNode> {
        None
    }

    fn facet_row_order_descending_config(&self) -> bool {
        false
    }

    fn facet_col_title_config(&self) -> Option<&str> {
        None
    }

    fn facet_col_slot_sharing_config(&self) -> Option<CoordinationScope> {
        None
    }

    fn facet_col_position_config(&self) -> Option<&str> {
        None
    }

    fn facet_col_guide_visible_config(&self) -> Option<bool> {
        None
    }

    fn facet_col_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        None
    }

    fn facet_col_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        None
    }

    fn facet_col_order_expr_config(&self) -> Option<&LogicalExprNode> {
        None
    }

    fn facet_col_order_descending_config(&self) -> bool {
        false
    }

    fn facet_wrap_title_config(&self) -> Option<&str> {
        None
    }

    fn facet_wrap_slot_sharing_config(&self) -> Option<CoordinationScope> {
        None
    }

    fn facet_wrap_position_config(&self) -> Option<&str> {
        None
    }

    fn facet_wrap_guide_visible_config(&self) -> Option<bool> {
        None
    }

    fn facet_wrap_axis_guide_visibility_config(&self) -> Option<AxisGuideVisibilityConfig> {
        None
    }

    fn facet_wrap_empty_cell_policy_config(&self) -> Option<FacetEmptyCellPolicy> {
        None
    }

    fn facet_wrap_order_expr_config(&self) -> Option<&LogicalExprNode> {
        None
    }

    fn facet_wrap_order_descending_config(&self) -> bool {
        false
    }

    fn facet_wrap_column_mode_config(&self) -> FacetWrapColumnMode {
        FacetWrapColumnMode::Auto
    }

    fn has_plot_level_data(&self) -> bool;

    async fn compile_child_plot(
        &self,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError>;

    async fn compile_child_plot_with_context(
        &self,
        session_context: &SessionContext,
        _compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledSubplotChildPlot>, AvengerChartError> {
        self.compile_child_plot(session_context).await
    }

    fn validate_no_facet_channels(&self, outer_label: &str) -> Result<(), AvengerChartError> {
        for channel_name in [
            RowDimensionConfig::channel_name(),
            ColumnDimensionConfig::channel_name(),
        ] {
            self.validate_no_channel(channel_name, outer_label)?;
        }
        Ok(())
    }

    fn validate_no_channel(
        &self,
        channel_name: &'static str,
        outer_label: &str,
    ) -> Result<(), AvengerChartError> {
        if self
            .data_context_ref()
            .channels()
            .contains_key(channel_name)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{outer_label} subplots do not support channel `{channel_name}`"
            )));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait SubplotContainerCoordinateSystem: CoordinateSystem + Sized {
    /// Compile a subplot mark for this coordinate system.
    ///
    /// Outer-coordinate-specific builder methods live on concrete
    /// `Subplot<...>` extension traits. This hook only owns the final
    /// conversion from a generic subplot mark view plus compiled mark state
    /// into the coordinate-system-specific compiled mark. The top-level layout
    /// engine keeps facet and concat behavior built in; external coordinate
    /// crates can implement this hook when their coordinate system supports
    /// positioned child plots.
    async fn compile_subplot_mark(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;

    async fn compile_subplot_mark_with_context(
        subplot: &dyn SubplotMarkCore,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
        _compile_context: Option<CompileContext<'_>>,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Self::compile_subplot_mark(subplot, compiled_state, session_context).await
    }
}

/// One source channel used to position child plot frames inside a coordinate system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionedSubplotChannel {
    pub channel: String,
    pub transform_channel: String,
}

impl PositionedSubplotChannel {
    pub fn new(channel: impl Into<String>, transform_channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            transform_channel: transform_channel.into(),
        }
    }
}

/// Coordinate-specific metadata for the generic positioned-subplot runtime.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PositionedSubplotSpec {
    pub outer_label: String,
    pub group_name_prefix: String,
    pub placement_channels: Vec<PositionedSubplotChannel>,
    pub partition_channel: Option<String>,
    pub default_plot_width: f32,
    pub default_plot_height: f32,
}

impl PositionedSubplotSpec {
    pub fn new(
        outer_label: impl Into<String>,
        group_name_prefix: impl Into<String>,
        placement_channels: Vec<PositionedSubplotChannel>,
    ) -> Self {
        Self {
            outer_label: outer_label.into(),
            group_name_prefix: group_name_prefix.into(),
            placement_channels,
            partition_channel: None,
            default_plot_width: 80.0,
            default_plot_height: 80.0,
        }
    }

    pub fn with_partition_channel(mut self, channel: impl Into<String>) -> Self {
        self.partition_channel = Some(channel.into());
        self
    }

    pub fn without_partition_channel(mut self) -> Self {
        self.partition_channel = None;
        self
    }

    pub fn with_default_plot_size(mut self, width: f32, height: f32) -> Self {
        self.default_plot_width = width;
        self.default_plot_height = height;
        self
    }
}

/// Object-safe view over a compiled coordinate-positioned subplot mark.
pub trait PositionedSubplotMarkCore: CompiledMark {
    fn as_compiled_mark(&self) -> &dyn CompiledMark;
    fn payload(&self) -> &CompiledSubplotPayload;
    fn spec(&self) -> &PositionedSubplotSpec;
    fn plot_width(&self) -> f32;
    fn plot_height(&self) -> f32;
    fn partition_expr(&self) -> Option<&LogicalExprNode>;

    fn label(&self) -> Option<&str> {
        self.payload().label()
    }

    fn key(&self) -> Option<&str> {
        self.payload().key()
    }

    fn mark_index(&self) -> usize {
        self.payload().mark_index()
    }

    fn inherits_parent_data(&self) -> bool {
        self.payload().inherits_parent_data()
    }

    fn is_partitioned(&self) -> bool {
        self.partition_expr().is_some()
    }
}

/// Reusable compiled mark for coordinate-positioned `Subplot<C>`.
#[serde_as]
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPositionedSubplot {
    payload: CompiledSubplotPayload,
    spec: PositionedSubplotSpec,
    plot_width: f32,
    plot_height: f32,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    partition_expr: Option<LogicalExprNode>,
}

impl CompiledPositionedSubplot {
    pub fn new(
        payload: CompiledSubplotPayload,
        spec: PositionedSubplotSpec,
        plot_width: f32,
        plot_height: f32,
        partition_expr: Option<LogicalExprNode>,
    ) -> Self {
        Self {
            payload,
            spec,
            plot_width,
            plot_height,
            partition_expr,
        }
    }

    pub fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    pub fn spec(&self) -> &PositionedSubplotSpec {
        &self.spec
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    #[doc(hidden)]
    pub fn partition_expr(&self) -> Option<&LogicalExprNode> {
        self.partition_expr.as_ref()
    }
}

impl PositionedSubplotMarkCore for CompiledPositionedSubplot {
    fn as_compiled_mark(&self) -> &dyn CompiledMark {
        self
    }

    fn payload(&self) -> &CompiledSubplotPayload {
        &self.payload
    }

    fn spec(&self) -> &PositionedSubplotSpec {
        &self.spec
    }

    fn plot_width(&self) -> f32 {
        self.plot_width
    }

    fn plot_height(&self) -> f32 {
        self.plot_height
    }

    fn partition_expr(&self) -> Option<&LogicalExprNode> {
        self.partition_expr.as_ref()
    }
}

impl CompiledMarkCore for CompiledPositionedSubplot {
    fn state(&self) -> &CompiledMarkState {
        self.payload.compiled_state()
    }

    fn state_mut(&mut self) -> &mut CompiledMarkState {
        self.payload.compiled_state_mut()
    }

    fn data_context(&self) -> &CompiledDataContext {
        &self.payload.compiled_state().data
    }

    fn mark_type(&self) -> &str {
        "subplot"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_positioned_subplot(&self) -> Option<&dyn PositionedSubplotMarkCore> {
        Some(self)
    }

    fn supported_channels(&self) -> Vec<ChannelDescriptor> {
        let mut channels = self
            .spec
            .placement_channels
            .iter()
            .map(|channel| ChannelDescriptor {
                name: intern_positioned_subplot_channel_name(&channel.channel),
                required: true,
                default_value: None,
                allow_column_ref: true,
            })
            .collect::<Vec<_>>();

        if let Some(partition_channel) = &self.spec.partition_channel {
            channels.push(ChannelDescriptor {
                name: intern_positioned_subplot_channel_name(partition_channel),
                required: false,
                default_value: None,
                allow_column_ref: true,
            });
        }

        channels
    }

    fn radius_expression(
        &self,
        _dimension: &str,
        _resolve_channel: &dyn Fn(&str) -> Expr,
    ) -> Option<RadiusExpression> {
        None
    }
}

fn intern_positioned_subplot_channel_name(channel: &str) -> &'static str {
    static INTERNED_CHANNEL_NAMES: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();

    let interner = INTERNED_CHANNEL_NAMES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut names = interner
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(name) = names.get(channel) {
        return name;
    }

    let owned = channel.to_string();
    let leaked = Box::leak(owned.clone().into_boxed_str());
    names.insert(owned, leaked);
    leaked
}

#[typetag::serde]
#[async_trait::async_trait]
impl CompiledMark for CompiledPositionedSubplot {
    async fn render_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        _context: &dyn MarkRuntimeContext,
        _coord: &dyn CoordinateSystemTransformCore,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Err(AvengerChartError::InternalError(
            "Positioned subplot marks require the top-level layout render dispatcher".to_string(),
        ))
    }
}

/// Shared compiled state for a child plot owned by a container subplot mark.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledSubplotPayload {
    state: CompiledMarkState,
    compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
    label: Option<String>,
    key: Option<String>,
    data_source: SubplotDataSource,
}

impl CompiledSubplotPayload {
    pub fn new(
        state: CompiledMarkState,
        compiled_subplot: Arc<dyn CompiledSubplotChildPlot>,
        label: Option<String>,
        key: Option<String>,
        data_source: SubplotDataSource,
    ) -> Self {
        Self {
            state,
            compiled_subplot,
            label,
            key,
            data_source,
        }
    }

    pub fn compiled_child_plot(&self) -> &Arc<dyn CompiledSubplotChildPlot> {
        &self.compiled_subplot
    }

    pub fn compiled_state(&self) -> &CompiledMarkState {
        &self.state
    }

    pub fn compiled_state_mut(&mut self) -> &mut CompiledMarkState {
        &mut self.state
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

    pub fn data_source(&self) -> SubplotDataSource {
        self.data_source
    }

    pub fn inherits_parent_data(&self) -> bool {
        self.data_source == SubplotDataSource::InheritParent
    }

    pub fn has_explicit_child_data(&self) -> bool {
        self.data_source == SubplotDataSource::ExplicitChild
    }

    #[doc(hidden)]
    pub fn inherited_data_override(
        &self,
        data: Option<&RecordBatch>,
        session_context: &SessionContext,
    ) -> Result<Option<DataFrame>, AvengerChartError> {
        if !self.inherits_parent_data() {
            return Ok(None);
        }

        data.map(|batch| {
            session_context
                .read_batch(batch.clone())
                .map_err(AvengerChartError::DataFusionError)
        })
        .transpose()
    }
}

pub async fn compile_subplot_payload<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<CompiledSubplotPayload, AvengerChartError> {
    let data_source = if subplot.has_plot_level_data() {
        SubplotDataSource::ExplicitChild
    } else {
        SubplotDataSource::InheritParent
    };
    let compiled_subplot = subplot.compile_child_plot(session_context).await?;

    Ok(CompiledSubplotPayload::new(
        compiled_state,
        compiled_subplot,
        subplot.label_config().map(ToOwned::to_owned),
        subplot.key_config().map(ToOwned::to_owned),
        data_source,
    ))
}

pub async fn compile_subplot_payload_with_context<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
    compile_context: Option<CompileContext<'_>>,
) -> Result<CompiledSubplotPayload, AvengerChartError> {
    let data_source = if subplot.has_plot_level_data() {
        SubplotDataSource::ExplicitChild
    } else {
        SubplotDataSource::InheritParent
    };
    let compiled_subplot = subplot
        .compile_child_plot_with_context(session_context, compile_context)
        .await?;

    Ok(CompiledSubplotPayload::new(
        compiled_state,
        compiled_subplot,
        subplot.label_config().map(ToOwned::to_owned),
        subplot.key_config().map(ToOwned::to_owned),
        data_source,
    ))
}

pub async fn compile_positioned_subplot_mark<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
    spec: PositionedSubplotSpec,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    subplot.validate_no_facet_channels(&spec.outer_label)?;
    let partition_expr = partition_expr_node(subplot, session_context, &spec)?;
    let plot_width = subplot
        .plot_width_config()
        .unwrap_or(spec.default_plot_width)
        .max(1.0);
    let plot_height = subplot
        .plot_height_config()
        .unwrap_or(spec.default_plot_height)
        .max(1.0);
    let payload = compile_subplot_payload(subplot, compiled_state, session_context).await?;

    Ok(Arc::new(CompiledPositionedSubplot::new(
        payload,
        spec,
        plot_width,
        plot_height,
        partition_expr,
    )))
}

pub async fn compile_positioned_subplot_mark_with_context<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
    spec: PositionedSubplotSpec,
    compile_context: Option<CompileContext<'_>>,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    subplot.validate_no_facet_channels(&spec.outer_label)?;
    let partition_expr = partition_expr_node(subplot, session_context, &spec)?;
    let plot_width = subplot
        .plot_width_config()
        .unwrap_or(spec.default_plot_width)
        .max(1.0);
    let plot_height = subplot
        .plot_height_config()
        .unwrap_or(spec.default_plot_height)
        .max(1.0);
    let payload = compile_subplot_payload_with_context(
        subplot,
        compiled_state,
        session_context,
        compile_context,
    )
    .await?;
    Ok(Arc::new(CompiledPositionedSubplot::new(
        payload,
        spec,
        plot_width,
        plot_height,
        partition_expr,
    )))
}

fn partition_expr_node<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    session_context: &SessionContext,
    spec: &PositionedSubplotSpec,
) -> Result<Option<LogicalExprNode>, AvengerChartError> {
    let Some(partition_channel) = spec.partition_channel.as_deref() else {
        return Ok(None);
    };
    let Some(channel) = subplot.data_context_ref().channels().get(partition_channel) else {
        return Ok(None);
    };
    let Some(expr) = channel.expr(session_context) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{} subplot partition channel `{partition_channel}` does not support conditional values",
            spec.outer_label
        )));
    };

    validate_partitioned_subplot(subplot, session_context, spec, &expr)?;
    LogicalExprNode::from_expr(expr).map(Some)
}

fn validate_partitioned_subplot<S: SubplotMarkCore + ?Sized>(
    subplot: &S,
    session_context: &SessionContext,
    spec: &PositionedSubplotSpec,
    partition_expr: &Expr,
) -> Result<(), AvengerChartError> {
    if subplot.has_plot_level_data() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Partitioned {} subplots inherit parent data; remove plot-level data from the child plot",
            spec.outer_label
        )));
    }

    for channel_spec in &spec.placement_channels {
        let channel_name = channel_spec.channel.as_str();
        let Some(channel) = subplot.data_context_ref().channels().get(channel_name) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned {} subplots require channel `{channel_name}`",
                spec.outer_label
            )));
        };
        let Some(expr) = channel.expr(session_context) else {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned {} subplot channel `{channel_name}` does not support conditional values",
                spec.outer_label
            )));
        };

        if !valid_partition_position_expr(&expr, partition_expr) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Partitioned {} subplot channel `{channel_name}` must be an aggregate, literal/constant, or the partition expression",
                spec.outer_label
            )));
        }
    }

    Ok(())
}

fn valid_partition_position_expr(expr: &Expr, partition_expr: &Expr) -> bool {
    contains_aggregate(expr)
        || !expr.any_column_refs()
        || expr == partition_expr
        || expr.to_string() == partition_expr.to_string()
}
