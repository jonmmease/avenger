use crate::common::{
    expr_node, map_expr_node, map_expr_nodes, map_optional_expr_node, simple_column_name,
    validate_output_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    RasterDim, SerializableExpr, dim, eval_to_scalars, params_to_datafusion,
};
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BooleanArray, Float64Array, ListArray, ListBuilder, StringArray,
            StringBuilder, StructArray, UInt32Array, UInt64Array,
        },
        buffer::OffsetBuffer,
        datatypes::{DataType, Field, FieldRef, Fields},
    },
    common::{DataFusionError, Result as DataFusionResult, ScalarValue},
    dataframe::DataFrame,
    functions_aggregate::expr_fn::{max, min},
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, EmitTo, Expr, ExprSchemable,
        GroupsAccumulator, Signature, TypeSignature, Volatility,
        function::{AccumulatorArgs, StateFieldsArgs},
    },
    prelude::{col, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::{
    any::Any,
    collections::hash_map::DefaultHasher,
    fmt::Debug,
    hash::{Hash, Hasher},
    mem::size_of,
    sync::Arc,
    time::Instant,
};

const RASTERIZE_X: &str = "__avenger_rasterize2d_x";
const RASTERIZE_Y: &str = "__avenger_rasterize2d_y";
const RASTERIZE_VALUE: &str = "__avenger_rasterize2d_value";
const INFER_VALUE: &str = "__avenger_rasterize2d_infer_value";
const INFER_MIN: &str = "__avenger_rasterize2d_min";
const INFER_MAX: &str = "__avenger_rasterize2d_max";
const MAX_RASTERIZE_CELLS: usize = 16_777_216;

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledRasterize2DTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub x: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub y: LogicalExprNode,
    pub x_dim: Rasterize2DDimensionSpec,
    pub y_dim: Rasterize2DDimensionSpec,
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub value: Option<LogicalExprNode>,
    pub agg: Rasterize2DAgg,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub partition_by: Vec<LogicalExprNode>,
    pub raster_name: String,
    pub x_dim_name: String,
    pub y_dim_name: String,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rasterize2DDimensionSpec {
    pub extent: Option<Rasterize2DExtentSpec>,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub bins: LogicalExprNode,
    pub sampling: String,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rasterize2DExtentSpec {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub start: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub stop: LogicalExprNode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rasterize2DAgg {
    Count,
    Sum,
    Min,
    Max,
    Mean,
    VarPop,
    StddevPop,
    VarSamp,
    StddevSamp,
}

impl Rasterize2DAgg {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, AvengerChartError> {
        match value.as_ref().to_ascii_lowercase().as_str() {
            "count" => Ok(Self::Count),
            "sum" => Ok(Self::Sum),
            "min" => Ok(Self::Min),
            "max" => Ok(Self::Max),
            "mean" | "avg" => Ok(Self::Mean),
            "var_pop" | "variance_pop" => Ok(Self::VarPop),
            "stddev_pop" | "std_pop" => Ok(Self::StddevPop),
            "var_samp" | "variance_samp" => Ok(Self::VarSamp),
            "stddev_samp" | "std_samp" => Ok(Self::StddevSamp),
            other => Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D reducer \"{other}\" is not supported"
            ))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Min => "min",
            Self::Max => "max",
            Self::Mean => "mean",
            Self::VarPop => "var_pop",
            Self::StddevPop => "stddev_pop",
            Self::VarSamp => "var_samp",
            Self::StddevSamp => "stddev_samp",
        }
    }

    fn requires_value(self) -> bool {
        !matches!(self, Self::Count)
    }
}

#[derive(Clone, Debug)]
pub struct Rasterize2D {
    x: Expr,
    y: Expr,
    x_dim: Rasterize2DDimension,
    y_dim: Rasterize2DDimension,
    value: Option<Expr>,
    agg: String,
    partition_by: Vec<Expr>,
    raster_name: String,
}

impl Rasterize2D {
    pub fn new(x: impl IntoExpr, y: impl IntoExpr) -> Self {
        Self {
            x: x.into_expr(),
            y: y.into_expr(),
            x_dim: Rasterize2DDimension::default(),
            y_dim: Rasterize2DDimension::default(),
            value: None,
            agg: "count".to_string(),
            partition_by: Vec::new(),
            raster_name: "raster".to_string(),
        }
    }

    pub fn x<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Rasterize2DDimension) -> Rasterize2DDimension,
    {
        self.x_dim = f(self.x_dim);
        self
    }

    pub fn y<F>(mut self, f: F) -> Self
    where
        F: FnOnce(Rasterize2DDimension) -> Rasterize2DDimension,
    {
        self.y_dim = f(self.y_dim);
        self
    }

    pub fn value(mut self, value: impl IntoExpr) -> Self {
        self.value = Some(value.into_expr());
        self
    }

    pub fn agg(mut self, agg: impl Into<String>) -> Self {
        self.agg = agg.into();
        self
    }

    pub fn partition_by<I, E>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = E>,
        E: IntoExpr,
    {
        self.partition_by
            .extend(exprs.into_iter().map(IntoExpr::into_expr));
        self
    }

    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.raster_name = name.into();
        self
    }
}

#[derive(Clone, Debug)]
pub struct Rasterize2DDimension {
    extent: Option<(Expr, Expr)>,
    bins: Expr,
    sampling: String,
}

impl Default for Rasterize2DDimension {
    fn default() -> Self {
        Self {
            extent: None,
            bins: 256usize.into_expr(),
            sampling: "linear".to_string(),
        }
    }
}

impl Rasterize2DDimension {
    pub fn extent(mut self, start: impl IntoExpr, stop: impl IntoExpr) -> Self {
        self.extent = Some((start.into_expr(), stop.into_expr()));
        self
    }

    pub fn extent_expr(self, start: impl IntoExpr, stop: impl IntoExpr) -> Self {
        self.extent(start, stop)
    }

    pub fn bins(mut self, bins: impl IntoExpr) -> Self {
        self.bins = bins.into_expr();
        self
    }

    pub fn sampling(mut self, sampling: impl Into<String>) -> Self {
        self.sampling = sampling.into();
        self
    }
}

impl DataTransform for Rasterize2D {
    type Output = Rasterize2DOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        let agg = Rasterize2DAgg::parse(&self.agg)?;
        if agg.requires_value() && self.value.is_none() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D reducer \"{}\" requires value(...)",
                agg.name()
            )));
        }
        validate_partition_by_columns(&self.partition_by)?;
        validate_sampling("x", &self.x_dim.sampling)?;
        validate_sampling("y", &self.y_dim.sampling)?;

        let x_dim_name = self
            .x
            .name_for_alias()
            .map_err(AvengerChartError::DataFusionError)?;
        let y_dim_name = self
            .y
            .name_for_alias()
            .map_err(AvengerChartError::DataFusionError)?;
        if x_dim_name == y_dim_name {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D x and y dimension names must be distinct, got \"{x_dim_name}\""
            )));
        }

        let transform = CompiledRasterize2DTransform {
            x: expr_node(self.x, "rasterize x expression"),
            y: expr_node(self.y, "rasterize y expression"),
            x_dim: self.x_dim.into_spec("x")?,
            y_dim: self.y_dim.into_spec("y")?,
            value: self
                .value
                .map(|value| expr_node(value, "rasterize value expression")),
            agg,
            partition_by: self
                .partition_by
                .into_iter()
                .map(|expr| expr_node(expr, "rasterize partition expression"))
                .collect(),
            raster_name: self.raster_name.clone(),
            x_dim_name: x_dim_name.clone(),
            y_dim_name: y_dim_name.clone(),
        };

        Ok((
            Box::new(transform),
            Rasterize2DOutput {
                raster_name: self.raster_name,
                x_dim_name,
                y_dim_name,
            },
        ))
    }
}

impl Rasterize2DDimension {
    fn into_spec(self, axis: &str) -> Result<Rasterize2DDimensionSpec, AvengerChartError> {
        validate_sampling(axis, &self.sampling)?;
        Ok(Rasterize2DDimensionSpec {
            extent: self.extent.map(|(start, stop)| Rasterize2DExtentSpec {
                start: expr_node(start, "rasterize extent start expression"),
                stop: expr_node(stop, "rasterize extent stop expression"),
            }),
            bins: expr_node(self.bins, "rasterize bins expression"),
            sampling: self.sampling,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Rasterize2DOutput {
    raster_name: String,
    x_dim_name: String,
    y_dim_name: String,
}

impl Rasterize2DOutput {
    pub fn raster(&self) -> Expr {
        col(&self.raster_name)
    }

    pub fn x_dim(&self) -> RasterDim {
        dim(self.x_dim_name.clone())
    }

    pub fn y_dim(&self) -> RasterDim {
        dim(self.y_dim_name.clone())
    }
}

#[typetag::serde(name = "rasterize_2d")]
#[async_trait]
impl CompiledDataTransform for CompiledRasterize2DTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            x: map_expr_node(&self.x, f)?,
            y: map_expr_node(&self.y, f)?,
            x_dim: map_dimension_spec(&self.x_dim, f)?,
            y_dim: map_dimension_spec(&self.y_dim, f)?,
            value: map_optional_expr_node(&self.value, f)?,
            agg: self.agg,
            partition_by: map_expr_nodes(&self.partition_by, f)?,
            raster_name: self.raster_name.clone(),
            x_dim_name: self.x_dim_name.clone(),
            y_dim_name: self.y_dim_name.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            [
                self.raster_name.as_str(),
                RASTERIZE_X,
                RASTERIZE_Y,
                RASTERIZE_VALUE,
                INFER_VALUE,
                INFER_MIN,
                INFER_MAX,
            ],
        )?;
        if self.agg != Rasterize2DAgg::Count {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D reducer \"{}\" is planned but not implemented yet",
                self.agg.name()
            )));
        }

        let x_expr = cast_to_f64(self.x.to_default_expr(ctx.session_context)?, &dataframe)?;
        let y_expr = cast_to_f64(self.y.to_default_expr(ctx.session_context)?, &dataframe)?;
        let value_expr = self
            .value
            .as_ref()
            .map(|value| cast_to_f64(value.to_default_expr(ctx.session_context)?, &dataframe))
            .transpose()?;

        let x_bins = eval_bins(&self.x_dim.bins, "x", ctx).await?;
        let y_bins = eval_bins(&self.y_dim.bins, "y", ctx).await?;
        let (x_start, x_stop) =
            resolve_extent(&dataframe, x_expr.clone(), &self.x_dim.extent, "x", ctx).await?;
        let (y_start, y_stop) =
            resolve_extent(&dataframe, y_expr.clone(), &self.y_dim.extent, "y", ctx).await?;
        let config = Rasterize2DGridConfig::new(
            self.x_dim_name.clone(),
            self.y_dim_name.clone(),
            self.x_dim.sampling.clone(),
            self.y_dim.sampling.clone(),
            x_start,
            x_stop,
            x_bins,
            y_start,
            y_stop,
            y_bins,
        )?;

        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            reducer = self.agg.name(),
            x_start = config.x_start,
            x_stop = config.x_stop,
            x_bins = config.x_bins,
            y_start = config.y_start,
            y_stop = config.y_stop,
            y_bins = config.y_bins,
            cells = config.grid_len,
            "building Rasterize2D aggregate"
        );

        let mut prepared = dataframe
            .with_column(RASTERIZE_X, x_expr)
            .map_err(AvengerChartError::DataFusionError)?
            .with_column(RASTERIZE_Y, y_expr)
            .map_err(AvengerChartError::DataFusionError)?;
        if let Some(value_expr) = value_expr {
            prepared = prepared
                .with_column(RASTERIZE_VALUE, value_expr)
                .map_err(AvengerChartError::DataFusionError)?;
        }

        let mut args = vec![col(RASTERIZE_X), col(RASTERIZE_Y)];
        if self.value.is_some() {
            args.push(col(RASTERIZE_VALUE));
        }

        let udf = AggregateUDF::new_from_impl(Rasterize2DCountUdf::new(config));
        let aggregate_expr = udf.call(args).alias(&self.raster_name);
        let group_exprs = self
            .partition_by
            .iter()
            .map(|expr| expr.to_default_expr(ctx.session_context))
            .collect::<Result<Vec<_>, _>>()?;
        let dataframe = prepared
            .aggregate(group_exprs, vec![aggregate_expr])
            .map_err(AvengerChartError::DataFusionError)?;

        Ok(DataTransformResult::dataframe(dataframe))
    }
}

#[derive(Clone, Debug)]
struct Rasterize2DGridConfig {
    x_dim_name: String,
    y_dim_name: String,
    x_sampling: String,
    y_sampling: String,
    x_start: f64,
    x_stop: f64,
    x_bins: u32,
    y_start: f64,
    y_stop: f64,
    y_bins: u32,
    grid_len: usize,
}

impl Rasterize2DGridConfig {
    #[allow(clippy::too_many_arguments)]
    fn new(
        x_dim_name: String,
        y_dim_name: String,
        x_sampling: String,
        y_sampling: String,
        x_start: f64,
        x_stop: f64,
        x_bins: u32,
        y_start: f64,
        y_stop: f64,
        y_bins: u32,
    ) -> Result<Self, AvengerChartError> {
        validate_extent("x", x_start, x_stop)?;
        validate_extent("y", y_start, y_stop)?;
        let x_bins_usize = usize::try_from(x_bins).map_err(|_| {
            AvengerChartError::InvalidArgument("Rasterize2D x bins must fit in usize".to_string())
        })?;
        let y_bins_usize = usize::try_from(y_bins).map_err(|_| {
            AvengerChartError::InvalidArgument("Rasterize2D y bins must fit in usize".to_string())
        })?;
        let grid_len = x_bins_usize.checked_mul(y_bins_usize).ok_or_else(|| {
            AvengerChartError::InvalidArgument(
                "Rasterize2D bin counts overflowed usize".to_string(),
            )
        })?;
        if grid_len > MAX_RASTERIZE_CELLS {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D requested {grid_len} cells, above the current limit of {MAX_RASTERIZE_CELLS}"
            )));
        }
        Ok(Self {
            x_dim_name,
            y_dim_name,
            x_sampling,
            y_sampling,
            x_start,
            x_stop,
            x_bins,
            y_start,
            y_stop,
            y_bins,
            grid_len,
        })
    }

    fn x_bin(&self, value: f64) -> Option<usize> {
        bin_index(value, self.x_start, self.x_stop, self.x_bins)
    }

    fn y_bin(&self, value: f64) -> Option<usize> {
        bin_index(value, self.y_start, self.y_stop, self.y_bins)
    }

    fn cell_index(&self, x: f64, y: f64) -> Option<usize> {
        let x_index = self.x_bin(x)?;
        let y_index = self.y_bin(y)?;
        Some(y_index * self.x_bins as usize + x_index)
    }
}

impl PartialEq for Rasterize2DGridConfig {
    fn eq(&self, other: &Self) -> bool {
        self.x_dim_name == other.x_dim_name
            && self.y_dim_name == other.y_dim_name
            && self.x_sampling == other.x_sampling
            && self.y_sampling == other.y_sampling
            && self.x_start.to_bits() == other.x_start.to_bits()
            && self.x_stop.to_bits() == other.x_stop.to_bits()
            && self.x_bins == other.x_bins
            && self.y_start.to_bits() == other.y_start.to_bits()
            && self.y_stop.to_bits() == other.y_stop.to_bits()
            && self.y_bins == other.y_bins
    }
}

impl Eq for Rasterize2DGridConfig {}

impl Hash for Rasterize2DGridConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.x_dim_name.hash(state);
        self.y_dim_name.hash(state);
        self.x_sampling.hash(state);
        self.y_sampling.hash(state);
        self.x_start.to_bits().hash(state);
        self.x_stop.to_bits().hash(state);
        self.x_bins.hash(state);
        self.y_start.to_bits().hash(state);
        self.y_stop.to_bits().hash(state);
        self.y_bins.hash(state);
    }
}

#[derive(Clone)]
struct Rasterize2DCountUdf {
    config: Rasterize2DGridConfig,
    signature: Signature,
}

impl Debug for Rasterize2DCountUdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rasterize2DCountUdf")
            .field("config", &self.config)
            .field("signature", &self.signature)
            .finish()
    }
}

impl Rasterize2DCountUdf {
    fn new(config: Rasterize2DGridConfig) -> Self {
        Self {
            config,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Float64, DataType::Float64]),
                    TypeSignature::Exact(vec![
                        DataType::Float64,
                        DataType::Float64,
                        DataType::Float64,
                    ]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl AggregateUDFImpl for Rasterize2DCountUdf {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "avenger_rasterize2d_count"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DataFusionResult<DataType> {
        Ok(raster_data_type(DataType::UInt64))
    }

    fn return_field(&self, _arg_fields: &[FieldRef]) -> DataFusionResult<FieldRef> {
        Ok(Arc::new(Field::new(
            self.name(),
            raster_data_type(DataType::UInt64),
            false,
        )))
    }

    fn is_nullable(&self) -> bool {
        false
    }

    fn accumulator(&self, _acc_args: AccumulatorArgs) -> DataFusionResult<Box<dyn Accumulator>> {
        Ok(Box::new(Rasterize2DCountAccumulator::new(
            self.config.clone(),
        )))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> DataFusionResult<Vec<FieldRef>> {
        Ok(vec![Arc::new(Field::new(
            format!("{}_counts", args.name),
            DataType::new_list(DataType::UInt64, false),
            false,
        ))])
    }

    fn groups_accumulator_supported(&self, _args: AccumulatorArgs) -> bool {
        true
    }

    fn create_groups_accumulator(
        &self,
        _args: AccumulatorArgs,
    ) -> DataFusionResult<Box<dyn GroupsAccumulator>> {
        Ok(Box::new(Rasterize2DCountGroupsAccumulator::new(
            self.config.clone(),
        )))
    }

    fn equals(&self, other: &dyn AggregateUDFImpl) -> bool {
        other
            .as_any()
            .downcast_ref::<Self>()
            .is_some_and(|other| self.config == other.config)
    }

    fn hash_value(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.name().hash(&mut hasher);
        self.config.hash(&mut hasher);
        hasher.finish()
    }
}

#[derive(Debug)]
struct Rasterize2DCountAccumulator {
    config: Rasterize2DGridConfig,
    counts: Vec<u64>,
}

impl Rasterize2DCountAccumulator {
    fn new(config: Rasterize2DGridConfig) -> Self {
        let counts = vec![0; config.grid_len];
        Self { config, counts }
    }
}

impl Accumulator for Rasterize2DCountAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DataFusionResult<()> {
        update_count_grid(&self.config, &mut self.counts, values, None, None)
    }

    fn evaluate(&mut self) -> DataFusionResult<ScalarValue> {
        let started = Instant::now();
        let raster = build_count_raster_array(&self.config, [&self.counts[..]])?;
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            rows = 1usize,
            cells = self.config.grid_len,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "constructed Rasterize2D raster struct"
        );
        Ok(ScalarValue::Struct(raster))
    }

    fn size(&self) -> usize {
        size_of::<Self>() + self.counts.capacity() * size_of::<u64>()
    }

    fn state(&mut self) -> DataFusionResult<Vec<ScalarValue>> {
        Ok(vec![ScalarValue::List(u64_list_array_from_rows(
            [&self.counts[..]],
            self.config.grid_len,
            false,
        )?)])
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DataFusionResult<()> {
        let counts = list_array(states.first(), "Rasterize2D count state")?;
        for row_index in 0..counts.len() {
            if counts.is_null(row_index) {
                continue;
            }
            let partial = counts.value(row_index);
            let partial = partial
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal(
                        "Rasterize2D count state must contain UInt64 values".to_string(),
                    )
                })?;
            merge_count_grid(&mut self.counts, partial, self.config.grid_len)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Rasterize2DCountGroupsAccumulator {
    config: Rasterize2DGridConfig,
    counts: Vec<u64>,
}

impl Rasterize2DCountGroupsAccumulator {
    fn new(config: Rasterize2DGridConfig) -> Self {
        Self {
            config,
            counts: Vec::new(),
        }
    }

    fn resize_groups(&mut self, total_num_groups: usize) {
        self.counts
            .resize(total_num_groups * self.config.grid_len, 0);
    }

    fn group_count(&self) -> usize {
        self.counts.len() / self.config.grid_len
    }

    fn emit_group_count(&self, emit_to: EmitTo) -> usize {
        match emit_to {
            EmitTo::All => self.group_count(),
            EmitTo::First(count) => count,
        }
    }

    fn emitted_counts(&mut self, emit_to: EmitTo) -> Vec<u64> {
        let emit_groups = self.emit_group_count(emit_to);
        let emit_values = emit_groups * self.config.grid_len;
        match emit_to {
            EmitTo::All => std::mem::take(&mut self.counts),
            EmitTo::First(_) => self.counts.drain(0..emit_values).collect(),
        }
    }
}

impl GroupsAccumulator for Rasterize2DCountGroupsAccumulator {
    fn update_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        opt_filter: Option<&BooleanArray>,
        total_num_groups: usize,
    ) -> DataFusionResult<()> {
        self.resize_groups(total_num_groups);
        update_count_grid(
            &self.config,
            &mut self.counts,
            values,
            Some(group_indices),
            opt_filter,
        )
    }

    fn evaluate(&mut self, emit_to: EmitTo) -> DataFusionResult<ArrayRef> {
        let emit_groups = self.emit_group_count(emit_to);
        let counts = self.emitted_counts(emit_to);
        let rows = counts
            .chunks(self.config.grid_len)
            .take(emit_groups)
            .collect::<Vec<_>>();
        let started = Instant::now();
        let raster = build_count_raster_array(&self.config, rows)?;
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            rows = emit_groups,
            cells = self.config.grid_len,
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "constructed grouped Rasterize2D raster structs"
        );
        Ok(raster as ArrayRef)
    }

    fn state(&mut self, emit_to: EmitTo) -> DataFusionResult<Vec<ArrayRef>> {
        let emit_groups = self.emit_group_count(emit_to);
        let counts = self.emitted_counts(emit_to);
        let rows = counts
            .chunks(self.config.grid_len)
            .take(emit_groups)
            .collect::<Vec<_>>();
        Ok(vec![
            u64_list_array_from_rows(rows, self.config.grid_len, false)? as ArrayRef,
        ])
    }

    fn merge_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        _opt_filter: Option<&BooleanArray>,
        total_num_groups: usize,
    ) -> DataFusionResult<()> {
        self.resize_groups(total_num_groups);
        let partials = list_array(values.first(), "Rasterize2D grouped count state")?;
        if partials.len() != group_indices.len() {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D grouped count state length {} did not match group index length {}",
                partials.len(),
                group_indices.len()
            )));
        }
        for (row_index, group_index) in group_indices.iter().copied().enumerate() {
            if partials.is_null(row_index) {
                continue;
            }
            let partial = partials.value(row_index);
            let partial = partial
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or_else(|| {
                    DataFusionError::Internal(
                        "Rasterize2D grouped count state must contain UInt64 values".to_string(),
                    )
                })?;
            let start = group_index * self.config.grid_len;
            let end = start + self.config.grid_len;
            merge_count_grid(&mut self.counts[start..end], partial, self.config.grid_len)?;
        }
        Ok(())
    }

    fn size(&self) -> usize {
        size_of::<Self>() + self.counts.capacity() * size_of::<u64>()
    }
}

fn update_count_grid(
    config: &Rasterize2DGridConfig,
    counts: &mut [u64],
    values: &[ArrayRef],
    group_indices: Option<&[usize]>,
    opt_filter: Option<&BooleanArray>,
) -> DataFusionResult<()> {
    if values.len() != 2 && values.len() != 3 {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D count expected 2 or 3 arguments, got {}",
            values.len()
        )));
    }
    let x = f64_array(values.first(), "Rasterize2D x argument")?;
    let y = f64_array(values.get(1), "Rasterize2D y argument")?;
    let value = values
        .get(2)
        .map(|value| f64_array(Some(value), "Rasterize2D value argument"))
        .transpose()?;
    if x.len() != y.len() || value.is_some_and(|value| value.len() != x.len()) {
        return Err(DataFusionError::Internal(
            "Rasterize2D arguments must have equal lengths".to_string(),
        ));
    }
    if let Some(group_indices) = group_indices
        && group_indices.len() != x.len()
    {
        return Err(DataFusionError::Internal(
            "Rasterize2D group index length must match argument length".to_string(),
        ));
    }

    for row in 0..x.len() {
        if let Some(filter) = opt_filter
            && (filter.is_null(row) || !filter.value(row))
        {
            continue;
        }
        if x.is_null(row) || y.is_null(row) {
            continue;
        }
        let Some(cell_index) = config.cell_index(x.value(row), y.value(row)) else {
            continue;
        };
        if let Some(value) = value
            && (value.is_null(row) || !value.value(row).is_finite())
        {
            continue;
        }
        let offset = group_indices.map_or(0, |indices| indices[row] * config.grid_len);
        counts[offset + cell_index] = counts[offset + cell_index].saturating_add(1);
    }
    Ok(())
}

fn merge_count_grid(
    target: &mut [u64],
    partial: &UInt64Array,
    grid_len: usize,
) -> DataFusionResult<()> {
    if partial.len() != grid_len || target.len() < grid_len {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D count state length {} did not match expected grid length {grid_len}",
            partial.len()
        )));
    }
    for index in 0..grid_len {
        if partial.is_valid(index) {
            target[index] = target[index].saturating_add(partial.value(index));
        }
    }
    Ok(())
}

fn f64_array<'a>(value: Option<&'a ArrayRef>, label: &str) -> DataFusionResult<&'a Float64Array> {
    value
        .ok_or_else(|| DataFusionError::Internal(format!("{label} is missing")))?
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| DataFusionError::Internal(format!("{label} must be Float64")))
}

fn list_array<'a>(value: Option<&'a ArrayRef>, label: &str) -> DataFusionResult<&'a ListArray> {
    value
        .ok_or_else(|| DataFusionError::Internal(format!("{label} is missing")))?
        .as_any()
        .downcast_ref::<ListArray>()
        .ok_or_else(|| DataFusionError::Internal(format!("{label} must be a ListArray")))
}

fn map_dimension_spec(
    spec: &Rasterize2DDimensionSpec,
    f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
) -> Result<Rasterize2DDimensionSpec, AvengerChartError> {
    Ok(Rasterize2DDimensionSpec {
        extent: spec
            .extent
            .as_ref()
            .map(|extent| {
                Ok::<_, AvengerChartError>(Rasterize2DExtentSpec {
                    start: map_expr_node(&extent.start, f)?,
                    stop: map_expr_node(&extent.stop, f)?,
                })
            })
            .transpose()?,
        bins: map_expr_node(&spec.bins, f)?,
        sampling: spec.sampling.clone(),
    })
}

fn validate_partition_by_columns(exprs: &[Expr]) -> Result<(), AvengerChartError> {
    for expr in exprs {
        if simple_column_name(expr).is_none() {
            return Err(AvengerChartError::InvalidArgument(
                "Rasterize2D::partition_by(...) only supports simple column expressions for now"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_sampling(axis: &str, sampling: &str) -> Result<(), AvengerChartError> {
    if sampling != "linear" {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D only supports {axis} sampling(\"linear\") for now"
        )));
    }
    Ok(())
}

fn validate_extent(axis: &str, start: f64, stop: f64) -> Result<(), AvengerChartError> {
    if !start.is_finite() || !stop.is_finite() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D {axis} extent must be finite"
        )));
    }
    if start == stop {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D {axis} extent start and stop must be different"
        )));
    }
    if start > stop {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D reversed {axis} extents are not supported yet"
        )));
    }
    Ok(())
}

fn bin_index(value: f64, start: f64, stop: f64, count: u32) -> Option<usize> {
    if !value.is_finite() || value < start || value > stop {
        return None;
    }
    let span = stop - start;
    if span <= 0.0 || count == 0 {
        return None;
    }
    let raw = ((value - start) / span * f64::from(count)).floor();
    Some((raw as isize).clamp(0, count as isize - 1) as usize)
}

fn raster_data_type(cell_type: DataType) -> DataType {
    let coord_values_type = DataType::new_list(DataType::Utf8, true);
    let coords_type = DataType::Struct(Fields::from(vec![
        Field::new("kind", DataType::Utf8, false),
        Field::new("sampling", DataType::Utf8, true),
        Field::new("start", DataType::Float64, true),
        Field::new("stop", DataType::Float64, true),
        Field::new("count", DataType::UInt32, true),
        Field::new("values", coord_values_type, true),
    ]));
    let dimension_type = DataType::Struct(Fields::from(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("coords", coords_type, false),
    ]));
    let dimensions_type = DataType::new_list(dimension_type, false);
    let geometry_type = DataType::Struct(Fields::from(vec![
        Field::new("kind", DataType::Utf8, false),
        Field::new("dimensions", dimensions_type, false),
    ]));
    let values_type = DataType::Struct(Fields::from(vec![
        Field::new("dims", DataType::new_list(DataType::Utf8, false), false),
        Field::new("data", DataType::new_list(cell_type, false), false),
    ]));
    DataType::Struct(Fields::from(vec![
        Field::new("geometry", geometry_type, false),
        Field::new("values", values_type, false),
    ]))
}

fn build_count_raster_array<'a>(
    config: &Rasterize2DGridConfig,
    rows: impl IntoIterator<Item = &'a [u64]>,
) -> DataFusionResult<Arc<StructArray>> {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let row_count = rows.len();
    let dimensions = dimensions_list_array(config, row_count)?;
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"; row_count])) as ArrayRef,
        ),
        (
            Arc::new(Field::new(
                "dimensions",
                dimensions.data_type().clone(),
                false,
            )),
            dimensions,
        ),
    ])) as ArrayRef;

    let dims = string_list_array_from_rows(
        (0..row_count).map(|_| vec![config.y_dim_name.as_str(), config.x_dim_name.as_str()]),
        false,
    )?;
    let data = u64_list_array_from_rows(rows, config.grid_len, false)? as ArrayRef;
    let values = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("dims", dims.data_type().clone(), false)),
            dims,
        ),
        (
            Arc::new(Field::new("data", data.data_type().clone(), false)),
            data,
        ),
    ])) as ArrayRef;

    Ok(Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("geometry", geometry.data_type().clone(), false)),
            geometry,
        ),
        (
            Arc::new(Field::new("values", values.data_type().clone(), false)),
            values,
        ),
    ])))
}

fn dimensions_list_array(
    config: &Rasterize2DGridConfig,
    row_count: usize,
) -> DataFusionResult<ArrayRef> {
    let total_dimensions = row_count * 2;
    let names = (0..row_count)
        .flat_map(|_| [config.x_dim_name.as_str(), config.y_dim_name.as_str()])
        .collect::<Vec<_>>();
    let samplings = (0..row_count)
        .flat_map(|_| {
            [
                Some(config.x_sampling.as_str()),
                Some(config.y_sampling.as_str()),
            ]
        })
        .collect::<Vec<_>>();
    let starts = (0..row_count)
        .flat_map(|_| [Some(config.x_start), Some(config.y_start)])
        .collect::<Vec<_>>();
    let stops = (0..row_count)
        .flat_map(|_| [Some(config.x_stop), Some(config.y_stop)])
        .collect::<Vec<_>>();
    let counts = (0..row_count)
        .flat_map(|_| [Some(config.x_bins), Some(config.y_bins)])
        .collect::<Vec<_>>();
    let mut coord_values_builder = ListBuilder::new(StringBuilder::new());
    for _ in 0..total_dimensions {
        coord_values_builder.append(false);
    }
    let coord_values = Arc::new(coord_values_builder.finish()) as ArrayRef;
    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["uniform"; total_dimensions])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("sampling", DataType::Utf8, true)),
            Arc::new(StringArray::from(samplings)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("start", DataType::Float64, true)),
            Arc::new(Float64Array::from(starts)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("stop", DataType::Float64, true)),
            Arc::new(Float64Array::from(stops)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("count", DataType::UInt32, true)),
            Arc::new(UInt32Array::from(counts)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("values", coord_values.data_type().clone(), true)),
            coord_values,
        ),
    ])) as ArrayRef;
    let dimensions = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("name", DataType::Utf8, false)),
            Arc::new(StringArray::from(names)) as ArrayRef,
        ),
        (
            Arc::new(Field::new("coords", coords.data_type().clone(), false)),
            coords,
        ),
    ])) as ArrayRef;
    Ok(list_array_from_lengths(
        dimensions,
        vec![2; row_count],
        false,
    ))
}

fn u64_list_array_from_rows<'a>(
    rows: impl IntoIterator<Item = &'a [u64]>,
    expected_len: usize,
    value_nullable: bool,
) -> DataFusionResult<Arc<ListArray>> {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let mut values = Vec::with_capacity(rows.len() * expected_len);
    for row in &rows {
        if row.len() != expected_len {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D row length {} did not match expected length {expected_len}",
                row.len()
            )));
        }
        values.extend_from_slice(row);
    }
    let values = Arc::new(UInt64Array::from(values)) as ArrayRef;
    Ok(
        list_array_from_lengths(values, vec![expected_len; rows.len()], value_nullable)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("list_array_from_lengths returned a ListArray")
            .clone()
            .into(),
    )
}

fn string_list_array_from_rows<'a>(
    rows: impl IntoIterator<Item = Vec<&'a str>>,
    value_nullable: bool,
) -> DataFusionResult<ArrayRef> {
    let rows = rows.into_iter().collect::<Vec<_>>();
    let lengths = rows.iter().map(Vec::len).collect::<Vec<_>>();
    let values = rows.into_iter().flatten().collect::<Vec<_>>();
    let values = Arc::new(StringArray::from(values)) as ArrayRef;
    Ok(list_array_from_lengths(values, lengths, value_nullable))
}

fn list_array_from_lengths(
    values: ArrayRef,
    lengths: Vec<usize>,
    value_nullable: bool,
) -> ArrayRef {
    let offsets = OffsetBuffer::from_lengths(lengths);
    Arc::new(
        ListArray::try_new(
            Arc::new(Field::new_list_field(
                values.data_type().clone(),
                value_nullable,
            )),
            offsets,
            values,
            None,
        )
        .expect("valid Rasterize2D list array"),
    ) as ArrayRef
}

fn cast_to_f64(expr: Expr, dataframe: &DataFrame) -> Result<Expr, AvengerChartError> {
    expr.cast_to(&DataType::Float64, dataframe.schema())
        .map_err(AvengerChartError::DataFusionError)
}

async fn eval_bins(
    expr: &LogicalExprNode,
    axis: &str,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<u32, AvengerChartError> {
    let scalar = eval_config_scalar(expr, &format!("{axis} bins"), ctx).await?;
    let bins = scalar_to_u64(&scalar, &format!("Rasterize2D {axis} bins"))?;
    let bins = u32::try_from(bins).map_err(|_| {
        AvengerChartError::InvalidArgument(format!("Rasterize2D {axis} bins must fit in UInt32"))
    })?;
    if bins == 0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D {axis} bins must be greater than zero"
        )));
    }
    Ok(bins)
}

async fn resolve_extent(
    dataframe: &DataFrame,
    expr: Expr,
    extent: &Option<Rasterize2DExtentSpec>,
    axis: &str,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<(f64, f64), AvengerChartError> {
    if let Some(extent) = extent {
        let start = scalar_to_f64(
            &eval_config_scalar(&extent.start, &format!("{axis} extent start"), ctx).await?,
            &format!("Rasterize2D {axis} extent start"),
        )?;
        let stop = scalar_to_f64(
            &eval_config_scalar(&extent.stop, &format!("{axis} extent stop"), ctx).await?,
            &format!("Rasterize2D {axis} extent stop"),
        )?;
        validate_extent(axis, start, stop)?;
        Ok((start, stop))
    } else {
        infer_extent(dataframe.clone(), expr, axis).await
    }
}

async fn infer_extent(
    dataframe: DataFrame,
    expr: Expr,
    axis: &str,
) -> Result<(f64, f64), AvengerChartError> {
    let dataframe = dataframe
        .select(vec![expr.alias(INFER_VALUE)])
        .map_err(AvengerChartError::DataFusionError)?
        .filter(
            col(INFER_VALUE)
                .gt(lit(f64::NEG_INFINITY))
                .and(col(INFER_VALUE).lt(lit(f64::INFINITY))),
        )
        .map_err(AvengerChartError::DataFusionError)?
        .aggregate(
            vec![],
            vec![
                min(col(INFER_VALUE)).alias(INFER_MIN),
                max(col(INFER_VALUE)).alias(INFER_MAX),
            ],
        )
        .map_err(AvengerChartError::DataFusionError)?;
    let batches = dataframe
        .collect()
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    let batch = batches.first().ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "Rasterize2D could not infer {axis} extent because the input was empty"
        ))
    })?;
    if batch.num_rows() == 0 {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Rasterize2D could not infer {axis} extent because no finite values were found"
        )));
    }
    let start = ScalarValue::try_from_array(batch.column(0), 0)
        .map_err(AvengerChartError::DataFusionError)
        .and_then(|scalar| scalar_to_f64(&scalar, &format!("Rasterize2D inferred {axis} min")))?;
    let stop = ScalarValue::try_from_array(batch.column(1), 0)
        .map_err(AvengerChartError::DataFusionError)
        .and_then(|scalar| scalar_to_f64(&scalar, &format!("Rasterize2D inferred {axis} max")))?;
    validate_extent(axis, start, stop)?;
    Ok((start, stop))
}

async fn eval_config_scalar(
    expr: &LogicalExprNode,
    label: &str,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<ScalarValue, AvengerChartError> {
    let expr = expr.to_default_expr(ctx.session_context)?;
    let params = params_to_datafusion(ctx.params);
    let mut values = eval_to_scalars(vec![expr], Some(ctx.session_context), params.as_ref())
        .await
        .map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "Rasterize2D {label} must be a literal or parameter expression; column references are not supported in Rasterize2D configuration expressions: {err}"
            ))
        })?;
    values.pop().ok_or_else(|| {
        AvengerChartError::InternalError(format!("Rasterize2D {label} did not produce a scalar"))
    })
}

fn scalar_to_f64(scalar: &ScalarValue, label: &str) -> Result<f64, AvengerChartError> {
    let value = match scalar {
        ScalarValue::Float64(Some(value)) => *value,
        ScalarValue::Float32(Some(value)) => f64::from(*value),
        ScalarValue::Int64(Some(value)) => *value as f64,
        ScalarValue::Int32(Some(value)) => f64::from(*value),
        ScalarValue::Int16(Some(value)) => f64::from(*value),
        ScalarValue::Int8(Some(value)) => f64::from(*value),
        ScalarValue::UInt64(Some(value)) => *value as f64,
        ScalarValue::UInt32(Some(value)) => f64::from(*value),
        ScalarValue::UInt16(Some(value)) => f64::from(*value),
        ScalarValue::UInt8(Some(value)) => f64::from(*value),
        other if other.is_null() => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must not be null"
            )));
        }
        other => {
            return Err(AvengerChartError::InvalidArgument(format!(
                "{label} must be numeric, got {other:?}"
            )));
        }
    };
    if !value.is_finite() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be finite"
        )));
    }
    Ok(value)
}

fn scalar_to_u64(scalar: &ScalarValue, label: &str) -> Result<u64, AvengerChartError> {
    match scalar {
        ScalarValue::UInt64(Some(value)) => Ok(*value),
        ScalarValue::UInt32(Some(value)) => Ok(u64::from(*value)),
        ScalarValue::UInt16(Some(value)) => Ok(u64::from(*value)),
        ScalarValue::UInt8(Some(value)) => Ok(u64::from(*value)),
        ScalarValue::Int64(Some(value)) if *value >= 0 => Ok(*value as u64),
        ScalarValue::Int32(Some(value)) if *value >= 0 => Ok(*value as u64),
        ScalarValue::Int16(Some(value)) if *value >= 0 => Ok(*value as u64),
        ScalarValue::Int8(Some(value)) if *value >= 0 => Ok(*value as u64),
        ScalarValue::Float64(Some(value))
            if value.is_finite() && *value >= 0.0 && value.fract() == 0.0 =>
        {
            Ok(*value as u64)
        }
        ScalarValue::Float32(Some(value))
            if value.is_finite() && *value >= 0.0 && value.fract() == 0.0 =>
        {
            Ok(*value as u64)
        }
        other if other.is_null() => Err(AvengerChartError::InvalidArgument(format!(
            "{label} must not be null"
        ))),
        other => Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be a non-negative integer, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Rasterize2DGridConfig {
        Rasterize2DGridConfig::new(
            "x".to_string(),
            "y".to_string(),
            "linear".to_string(),
            "linear".to_string(),
            0.0,
            2.0,
            2,
            0.0,
            2.0,
            2,
        )
        .unwrap()
    }

    fn input_arrays(xs: Vec<f64>, ys: Vec<f64>) -> Vec<ArrayRef> {
        vec![
            Arc::new(Float64Array::from(xs)) as ArrayRef,
            Arc::new(Float64Array::from(ys)) as ArrayRef,
        ]
    }

    fn counts_from_raster(raster: &StructArray, row: usize) -> Vec<u64> {
        let values = raster
            .column_by_name("values")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let data = values
            .column_by_name("data")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row_values = data.value(row);
        row_values
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values()
            .to_vec()
    }

    fn counts_from_state(state: &ArrayRef, row: usize) -> Vec<u64> {
        let state = state.as_any().downcast_ref::<ListArray>().unwrap();
        let values = state.value(row);
        values
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values()
            .to_vec()
    }

    #[test]
    fn count_accumulator_merges_state_and_evaluates_struct_scalar() {
        let config = test_config();
        let mut partial = Rasterize2DCountAccumulator::new(config.clone());
        partial
            .update_batch(&input_arrays(vec![0.0, 2.0], vec![0.0, 2.0]))
            .unwrap();
        assert!(partial.size() > 0);
        let mut state = partial.state().unwrap();
        let state_array = match state.pop().unwrap() {
            ScalarValue::List(array) => array as ArrayRef,
            other => panic!("expected list state, got {other:?}"),
        };

        let mut final_acc = Rasterize2DCountAccumulator::new(config);
        final_acc.merge_batch(&[state_array]).unwrap();
        let value = final_acc.evaluate().unwrap();
        let ScalarValue::Struct(raster) = value else {
            panic!("expected struct scalar");
        };
        assert_eq!(counts_from_raster(&raster, 0), vec![1, 0, 0, 1]);
    }

    #[test]
    fn count_groups_accumulator_emits_state_prefix_and_grouped_struct_array() {
        let config = test_config();
        let mut groups = Rasterize2DCountGroupsAccumulator::new(config);
        groups
            .update_batch(
                &input_arrays(vec![0.0, 2.0, 0.0, 2.0], vec![0.0, 2.0, 0.0, 0.0]),
                &[0, 0, 1, 1],
                None,
                2,
            )
            .unwrap();
        assert!(groups.size() > 0);

        let state = groups.state(EmitTo::First(1)).unwrap();
        assert_eq!(counts_from_state(&state[0], 0), vec![1, 0, 0, 1]);

        let remaining = groups.evaluate(EmitTo::All).unwrap();
        let remaining = remaining.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(counts_from_raster(remaining, 0), vec![1, 1, 0, 0]);
    }

    #[test]
    fn count_groups_accumulator_merges_grouped_state() {
        let config = test_config();
        let mut partial = Rasterize2DCountGroupsAccumulator::new(config.clone());
        let empty_size = partial.size();
        partial
            .update_batch(
                &input_arrays(vec![0.0, 2.0, 0.0, 2.0], vec![0.0, 2.0, 0.0, 0.0]),
                &[0, 0, 1, 1],
                None,
                2,
            )
            .unwrap();
        assert!(partial.size() > empty_size);
        let state = partial.state(EmitTo::All).unwrap();

        let mut merged = Rasterize2DCountGroupsAccumulator::new(config);
        merged.merge_batch(&state, &[0, 1], None, 2).unwrap();
        let raster = merged.evaluate(EmitTo::All).unwrap();
        let raster = raster.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(counts_from_raster(raster, 0), vec![1, 0, 0, 1]);
        assert_eq!(counts_from_raster(raster, 1), vec![1, 1, 0, 0]);
    }
}
