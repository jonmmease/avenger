use crate::common::{
    expr_node, map_expr_node, map_expr_nodes, map_optional_expr_node, simple_column_name,
    stable_hash_hex, validate_output_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    MaterializationExecutionContext, MaterializationExecutor, MaterializationIdentity,
    MaterializationKey, MaterializationOutputKind, MaterializationRequest, MaterializationResult,
    RasterDim, SerializableDataFrame, SerializableExpr, SerializableScalarMap, TimeContext,
    ViewMaterializationContext, ViewMaterializationRequest, dim, eval_to_scalars,
    params_to_datafusion,
};
use avenger_common::time::Instant;
use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, BooleanArray, Float64Array, ListArray, ListBuilder, StringArray,
            StringBuilder, StructArray, UInt32Array, UInt64Array, new_empty_array,
        },
        buffer::OffsetBuffer,
        compute::concat_batches,
        datatypes::{DataType, Field, FieldRef, Fields, Schema},
        record_batch::RecordBatch,
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
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Duration;
use std::{
    collections::HashMap,
    fmt::Debug,
    hash::{Hash, Hasher},
    mem::size_of,
    sync::Arc,
};

pub const RASTERIZE_2D_MATERIALIZATION_KIND: &str = "rasterize-2d";

const RASTERIZE_X: &str = "__avenger_rasterize2d_x";
const RASTERIZE_Y: &str = "__avenger_rasterize2d_y";
const RASTERIZE_VALUE: &str = "__avenger_rasterize2d_value";
const RASTERIZE_BY: &str = "__avenger_rasterize2d_by";
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
    /// Declared CRS of the x/y input expressions and extents (e.g. "epsg:3857").
    /// Stamped into the output raster's `geometry.crs` field. Serialized as part
    /// of the spec so it participates in the materialization identity/key hashes
    /// and round-trips through the async executor. Skipped when `None` so
    /// untagged specs (and their hashes) are byte-identical to before this
    /// field existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// Optional categorical plane dimension: rows are grouped by this
    /// expression's (stringified) value and the output raster gains a third,
    /// categorical dimension with one plane per observed category
    /// (plane-major `values.data`). Skipped when `None` for untagged-spec
    /// byte identity, like `frame`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub by: Option<LogicalExprNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_dim_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rasterize2DMaterializationSpec {
    pub version: u32,
    pub source: SerializableDataFrame,
    pub transform: CompiledRasterize2DTransform,
    pub params: SerializableScalarMap,
}

impl Rasterize2DMaterializationSpec {
    pub fn new(
        source: DataFrame,
        transform: CompiledRasterize2DTransform,
        params: SerializableScalarMap,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self {
            version: 1,
            source: SerializableDataFrame::from_dataframe(source)?,
            transform,
            params,
        })
    }

    fn key(&self) -> Result<MaterializationKey, AvengerChartError> {
        let bytes = serde_json::to_vec(self).map_err(|err| {
            AvengerChartError::InternalError(format!(
                "Failed to serialize Rasterize2D materialization spec: {err}"
            ))
        })?;
        Ok(MaterializationKey::new(format!(
            "{RASTERIZE_2D_MATERIALIZATION_KIND}/v{}/{}",
            self.version,
            stable_hash_hex(&bytes)
        )))
    }

    fn identity(&self) -> Result<MaterializationIdentity, AvengerChartError> {
        let fingerprint = Rasterize2DMaterializationIdentity {
            version: self.version,
            source: self.source.clone(),
            transform: self.transform.clone(),
        };
        let bytes = serde_json::to_vec(&fingerprint).map_err(|err| {
            AvengerChartError::InternalError(format!(
                "Failed to serialize Rasterize2D materialization identity: {err}"
            ))
        })?;
        Ok(MaterializationIdentity::new(format!(
            "{RASTERIZE_2D_MATERIALIZATION_KIND}/v{}/{}",
            self.version,
            stable_hash_hex(&bytes)
        )))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Rasterize2DMaterializationIdentity {
    version: u32,
    source: SerializableDataFrame,
    transform: CompiledRasterize2DTransform,
}

#[derive(Clone, Debug, Default)]
pub struct Rasterize2DExecutor;

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl MaterializationExecutor for Rasterize2DExecutor {
    fn kind(&self) -> &'static str {
        RASTERIZE_2D_MATERIALIZATION_KIND
    }

    async fn run(
        &self,
        request: MaterializationRequest,
        ctx: MaterializationExecutionContext<'_>,
    ) -> Result<MaterializationResult, AvengerChartError> {
        #[cfg(not(target_arch = "wasm32"))]
        let started = Instant::now();
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            key = %request.key,
            priority = request.priority,
            "Rasterize2D materialization started"
        );
        maybe_sleep_for_demo_delay();

        let spec: Rasterize2DMaterializationSpec = serde_json::from_value(request.spec.clone())
            .map_err(|err| {
                AvengerChartError::InvalidArgument(format!(
                    "Invalid Rasterize2D materialization spec: {err}"
                ))
            })?;
        if spec.version != 1 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Unsupported Rasterize2D materialization spec version {}",
                spec.version
            )));
        }

        let params = IndexMap::from(spec.params.clone());
        let dataframe = spec.source.to_dataframe(ctx.session_context)?;
        let transform_ctx = DataTransformExecutionContext {
            session_context: ctx.session_context,
            params: &params,
            time_context: TimeContext::default(),
            facet_context: None,
        };
        let result = spec
            .transform
            .apply_to_dataframe(dataframe, &transform_ctx)
            .await?;
        let batch = collect_single_batch(result.dataframe, &params).await?;
        #[cfg(target_arch = "wasm32")]
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            key = %request.key,
            rows = batch.num_rows(),
            columns = batch.num_columns(),
            "Rasterize2D materialization finished"
        );
        #[cfg(not(target_arch = "wasm32"))]
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            key = %request.key,
            rows = batch.num_rows(),
            columns = batch.num_columns(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "Rasterize2D materialization finished"
        );
        Ok(MaterializationResult::RecordBatch(batch))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn maybe_sleep_for_demo_delay() {
    let Some(delay) = std::env::var("AVENGER_TAXI_RASTER_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|delay| *delay > 0)
    else {
        return;
    };

    std::thread::sleep(Duration::from_millis(delay));
}

#[cfg(target_arch = "wasm32")]
fn maybe_sleep_for_demo_delay() {}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    frame: Option<String>,
    by: Option<Expr>,
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
            frame: None,
            by: None,
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

    /// Declare the coordinate reference system of the x/y input expressions and
    /// extents (e.g. `"epsg:3857"`). The tag is stamped into the output raster's
    /// `geometry.crs` field; consuming marks validate and convert it. Binning is
    /// unit-agnostic, so this is an assertion by the author, not an inference.
    pub fn frame(mut self, crs: impl Into<String>) -> Self {
        self.frame = Some(crs.into());
        self
    }

    /// Add a categorical plane dimension: rows are grouped by this
    /// expression's value (stringified to Utf8) and the raster gains a
    /// third, categorical dimension with one plane per OBSERVED category.
    /// `values.data` becomes plane-major (`[by, y, x]` dims order) and the
    /// categorical coords carry the sorted category names. Composes with
    /// `partition_by` (planes within each partition row) and `frame`.
    pub fn by(mut self, expr: impl IntoExpr) -> Self {
        self.by = Some(expr.into_expr());
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

        let by_dim_name = self
            .by
            .as_ref()
            .map(|by| by.name_for_alias())
            .transpose()
            .map_err(AvengerChartError::DataFusionError)?;
        if let Some(by_name) = &by_dim_name
            && (by_name == &x_dim_name || by_name == &y_dim_name)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Rasterize2D by dimension name \"{by_name}\" must be distinct from the x/y dimension names"
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
            frame: self.frame.clone(),
            by: self.by.map(|by| expr_node(by, "rasterize by expression")),
            by_dim_name: by_dim_name.clone(),
        };

        Ok((
            Box::new(transform),
            Rasterize2DOutput {
                raster_name: self.raster_name,
                x_dim_name,
                y_dim_name,
                by_dim_name,
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
    by_dim_name: Option<String>,
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

    /// The categorical plane dimension declared with [`Rasterize2D::by`].
    ///
    /// Panics when the transform was built without `by(...)` — referencing a
    /// plane dimension that does not exist is an authoring error.
    pub fn by_dim(&self) -> RasterDim {
        dim(self
            .by_dim_name
            .clone()
            .expect("Rasterize2D::by(...) was not configured; by_dim() has no dimension"))
    }
}

impl CompiledRasterize2DTransform {
    async fn apply_to_dataframe(
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
                RASTERIZE_BY,
                INFER_VALUE,
                INFER_MIN,
                INFER_MAX,
            ],
        )?;
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
            self.frame.clone(),
            self.by_dim_name.clone(),
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
        if let Some(by) = &self.by {
            // Stringify category values so the plane dimension's coords are
            // Utf8 regardless of the source column type.
            let by_expr = datafusion::logical_expr::cast(
                by.to_default_expr(ctx.session_context)?,
                DataType::Utf8,
            );
            prepared = prepared
                .with_column(RASTERIZE_BY, by_expr)
                .map_err(AvengerChartError::DataFusionError)?;
        }

        let mut args = vec![col(RASTERIZE_X), col(RASTERIZE_Y)];
        if self.value.is_some() {
            args.push(col(RASTERIZE_VALUE));
        }
        if self.by.is_some() {
            args.push(col(RASTERIZE_BY));
        }

        let udf = AggregateUDF::new_from_impl(Rasterize2DUdf::new(config, self.agg));
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

    fn materialization_spec(
        &self,
        dataframe: DataFrame,
        ctx: &ViewMaterializationContext<'_>,
    ) -> Result<Rasterize2DMaterializationSpec, AvengerChartError> {
        Rasterize2DMaterializationSpec::new(
            dataframe,
            self.clone(),
            SerializableScalarMap::from(ctx.params.clone()),
        )
    }

    fn empty_materialized_dataframe(
        &self,
        source: &DataFrame,
        ctx: &ViewMaterializationContext<'_>,
    ) -> Result<DataFrame, AvengerChartError> {
        let mut fields = Vec::new();
        for expr_node in &self.partition_by {
            let expr = expr_node.to_default_expr(ctx.session_context)?;
            let name = simple_column_name(&expr).ok_or_else(|| {
                AvengerChartError::InvalidArgument(
                    "Rasterize2D materialization partition expressions must be simple columns"
                        .to_string(),
                )
            })?;
            let field = source
                .schema()
                .fields()
                .iter()
                .find(|field| field.name().as_str() == name.as_str())
                .ok_or_else(|| {
                    AvengerChartError::InvalidArgument(format!(
                        "Rasterize2D partition column \"{name}\" was not found in the input"
                    ))
                })?;
            fields.push(Field::new(
                name,
                field.data_type().clone(),
                field.is_nullable(),
            ));
        }
        fields.push(Field::new(
            self.raster_name.clone(),
            raster_data_type(self.agg.output_cell_type(), self.agg.output_cell_nullable()),
            false,
        ));
        let schema = Arc::new(Schema::new(fields));
        let columns = schema
            .fields()
            .iter()
            .map(|field| new_empty_array(field.data_type()))
            .collect::<Vec<_>>();
        let batch = RecordBatch::try_new(schema, columns).map_err(AvengerChartError::ArrowError)?;
        ctx.session_context
            .read_batch(batch)
            .map_err(AvengerChartError::DataFusionError)
    }
}

#[typetag::serde(name = "rasterize_2d")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
            frame: self.frame.clone(),
            by: map_optional_expr_node(&self.by, f)?,
            by_dim_name: self.by_dim_name.clone(),
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        self.apply_to_dataframe(dataframe, ctx).await
    }

    fn view_materialization_request(
        &self,
        dataframe: &DataFrame,
        ctx: &ViewMaterializationContext<'_>,
    ) -> Result<Option<ViewMaterializationRequest>, AvengerChartError> {
        let spec = self.materialization_spec(dataframe.clone(), ctx)?;
        let key = spec.key()?;
        let identity = spec.identity()?;
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            key = %key,
            identity = %identity,
            reducer = self.agg.name(),
            priority = ctx.priority,
            allow_stale = ctx.policy.allow_stale,
            "created Rasterize2D materialization request"
        );
        let request = MaterializationRequest::new(
            key,
            RASTERIZE_2D_MATERIALIZATION_KIND,
            MaterializationOutputKind::RecordBatch,
        )
        .identity(identity)
        .priority(ctx.priority)
        .policy(ctx.policy.clone())
        .spec(serde_json::to_value(&spec).map_err(|err| {
            AvengerChartError::InternalError(format!(
                "Failed to encode Rasterize2D materialization spec: {err}"
            ))
        })?);
        Ok(Some(ViewMaterializationRequest {
            request,
            empty_dataframe: Some(self.empty_materialized_dataframe(dataframe, ctx)?),
        }))
    }

    fn view_materialization_identity(
        &self,
        dataframe: &DataFrame,
        ctx: &ViewMaterializationContext<'_>,
    ) -> Result<Option<avenger_chart_core::MaterializationIdentity>, AvengerChartError> {
        // Called on the unresolved transform: derived-scalar placeholders
        // stay symbolic, so the identity is stable while runtime scalars
        // (e.g. a density normalizer fed by an eager in-view count) vary.
        self.materialization_spec(dataframe.clone(), ctx)?
            .identity()
            .map(Some)
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
    crs: Option<String>,
    by_dim_name: Option<String>,
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
        crs: Option<String>,
        by_dim_name: Option<String>,
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
            crs,
            by_dim_name,
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
            && self.crs == other.crs
            && self.by_dim_name == other.by_dim_name
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
        self.crs.hash(state);
        self.by_dim_name.hash(state);
    }
}

#[derive(Clone)]
struct Rasterize2DUdf {
    config: Rasterize2DGridConfig,
    agg: Rasterize2DAgg,
    signature: Signature,
}

impl Debug for Rasterize2DUdf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rasterize2DUdf")
            .field("config", &self.config)
            .field("agg", &self.agg)
            .field("signature", &self.signature)
            .finish()
    }
}

impl Rasterize2DUdf {
    fn new(config: Rasterize2DGridConfig, agg: Rasterize2DAgg) -> Self {
        Self {
            config,
            agg,
            signature: Signature::one_of(
                vec![
                    TypeSignature::Exact(vec![DataType::Float64, DataType::Float64]),
                    TypeSignature::Exact(vec![
                        DataType::Float64,
                        DataType::Float64,
                        DataType::Float64,
                    ]),
                    TypeSignature::Exact(vec![
                        DataType::Float64,
                        DataType::Float64,
                        DataType::Utf8,
                    ]),
                    TypeSignature::Exact(vec![
                        DataType::Float64,
                        DataType::Float64,
                        DataType::Float64,
                        DataType::Utf8,
                    ]),
                ],
                Volatility::Immutable,
            ),
        }
    }
}

impl PartialEq for Rasterize2DUdf {
    fn eq(&self, other: &Self) -> bool {
        self.config == other.config && self.agg == other.agg
    }
}

impl Eq for Rasterize2DUdf {}

impl Hash for Rasterize2DUdf {
    fn hash<H: Hasher>(&self, state: &mut H) {
        "avenger_rasterize2d".hash(state);
        self.config.hash(state);
        self.agg.hash(state);
    }
}

impl AggregateUDFImpl for Rasterize2DUdf {
    fn name(&self) -> &str {
        "avenger_rasterize2d"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> DataFusionResult<DataType> {
        Ok(raster_data_type(
            self.agg.output_cell_type(),
            self.agg.output_cell_nullable(),
        ))
    }

    fn return_field(&self, _arg_fields: &[FieldRef]) -> DataFusionResult<FieldRef> {
        Ok(Arc::new(Field::new(
            self.name(),
            raster_data_type(self.agg.output_cell_type(), self.agg.output_cell_nullable()),
            false,
        )))
    }

    fn is_nullable(&self) -> bool {
        false
    }

    fn accumulator(&self, _acc_args: AccumulatorArgs) -> DataFusionResult<Box<dyn Accumulator>> {
        Ok(Box::new(Rasterize2DAccumulator::new(
            self.config.clone(),
            self.agg,
        )))
    }

    fn state_fields(&self, args: StateFieldsArgs) -> DataFusionResult<Vec<FieldRef>> {
        let mut fields: Vec<FieldRef> = Vec::new();
        if self.config.by_dim_name.is_some() {
            // Plane categories in accumulator-table order; the per-agg state
            // lists below hold K * grid_len entries, plane-major, aligned to
            // this list. Merge unifies planes by category VALUE, so states
            // from partitions that discovered different category sets (or
            // orders) combine correctly.
            fields.push(Arc::new(Field::new(
                format!("{}_categories", args.name),
                DataType::new_list(DataType::Utf8, false),
                false,
            )));
        }
        fields.extend(self.agg.state_specs().into_iter().map(|spec| {
            Arc::new(Field::new(
                format!("{}_{}", args.name, spec.name),
                DataType::new_list(spec.data_type, false),
                false,
            )) as FieldRef
        }));
        Ok(fields)
    }

    fn groups_accumulator_supported(&self, _args: AccumulatorArgs) -> bool {
        true
    }

    fn create_groups_accumulator(
        &self,
        _args: AccumulatorArgs,
    ) -> DataFusionResult<Box<dyn GroupsAccumulator>> {
        Ok(Box::new(Rasterize2DGroupsAccumulator::new(
            self.config.clone(),
            self.agg,
        )))
    }
}

#[derive(Clone)]
struct Rasterize2DStateSpec {
    name: &'static str,
    data_type: DataType,
}

impl Rasterize2DAgg {
    fn output_cell_type(self) -> DataType {
        match self {
            Self::Count => DataType::UInt64,
            Self::Sum
            | Self::Min
            | Self::Max
            | Self::Mean
            | Self::VarPop
            | Self::StddevPop
            | Self::VarSamp
            | Self::StddevSamp => DataType::Float64,
        }
    }

    fn output_cell_nullable(self) -> bool {
        !matches!(self, Self::Count)
    }

    fn state_specs(self) -> Vec<Rasterize2DStateSpec> {
        let counts = Rasterize2DStateSpec {
            name: "counts",
            data_type: DataType::UInt64,
        };
        let sums = Rasterize2DStateSpec {
            name: "sums",
            data_type: DataType::Float64,
        };
        let values = Rasterize2DStateSpec {
            name: "values",
            data_type: DataType::Float64,
        };
        let means = Rasterize2DStateSpec {
            name: "means",
            data_type: DataType::Float64,
        };
        let m2s = Rasterize2DStateSpec {
            name: "m2s",
            data_type: DataType::Float64,
        };
        match self {
            Self::Count => vec![counts],
            Self::Sum | Self::Mean => vec![counts, sums],
            Self::Min | Self::Max => vec![counts, values],
            Self::VarPop | Self::StddevPop | Self::VarSamp | Self::StddevSamp => {
                vec![counts, means, m2s]
            }
        }
    }

    fn uses_sums(self) -> bool {
        matches!(self, Self::Sum | Self::Mean)
    }

    fn uses_values(self) -> bool {
        matches!(self, Self::Min | Self::Max)
    }

    fn uses_moments(self) -> bool {
        matches!(
            self,
            Self::VarPop | Self::StddevPop | Self::VarSamp | Self::StddevSamp
        )
    }
}

#[derive(Debug)]
struct Rasterize2DAccumulator {
    config: Rasterize2DGridConfig,
    state: GridState,
}

impl Rasterize2DAccumulator {
    fn new(config: Rasterize2DGridConfig, agg: Rasterize2DAgg) -> Self {
        let state = GridState::new(&config, agg, 1);
        Self { config, state }
    }
}

impl Accumulator for Rasterize2DAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> DataFusionResult<()> {
        self.state.update(&self.config, values, None, None)
    }

    fn evaluate(&mut self) -> DataFusionResult<ScalarValue> {
        let started = Instant::now();
        let raster = self.state.build_raster_array(&self.config)?;
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            rows = 1usize,
            cells = self.config.grid_len,
            reducer = self.state.agg().name(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "constructed Rasterize2D raster struct"
        );
        Ok(ScalarValue::Struct(raster))
    }

    fn size(&self) -> usize {
        size_of::<Self>() + self.state.size()
    }

    fn state(&mut self) -> DataFusionResult<Vec<ScalarValue>> {
        self.state.state_scalars()
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> DataFusionResult<()> {
        self.state.merge(states, None)
    }
}

#[derive(Debug)]
struct Rasterize2DGroupsAccumulator {
    config: Rasterize2DGridConfig,
    state: GridState,
}

impl Rasterize2DGroupsAccumulator {
    fn new(config: Rasterize2DGridConfig, agg: Rasterize2DAgg) -> Self {
        let state = GridState::new(&config, agg, 0);
        Self { config, state }
    }
}

impl GroupsAccumulator for Rasterize2DGroupsAccumulator {
    fn update_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        opt_filter: Option<&BooleanArray>,
        total_num_groups: usize,
    ) -> DataFusionResult<()> {
        self.state.resize_groups(total_num_groups);
        self.state
            .update(&self.config, values, Some(group_indices), opt_filter)
    }

    fn evaluate(&mut self, emit_to: EmitTo) -> DataFusionResult<ArrayRef> {
        let emitted = self.state.take_emit(emit_to)?;
        let rows = emitted.group_count();
        let started = Instant::now();
        let raster = emitted.build_raster_array(&self.config)?;
        tracing::debug!(
            target: "avenger_chart::transforms::rasterize_2d",
            rows,
            cells = self.config.grid_len,
            reducer = emitted.agg().name(),
            elapsed_ms = started.elapsed().as_secs_f64() * 1000.0,
            "constructed grouped Rasterize2D raster structs"
        );
        Ok(raster as ArrayRef)
    }

    fn state(&mut self, emit_to: EmitTo) -> DataFusionResult<Vec<ArrayRef>> {
        self.state.take_emit(emit_to)?.state_arrays()
    }

    fn merge_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        _opt_filter: Option<&BooleanArray>,
        total_num_groups: usize,
    ) -> DataFusionResult<()> {
        self.state.resize_groups(total_num_groups);
        self.state.merge(values, Some(group_indices))
    }

    fn size(&self) -> usize {
        size_of::<Self>() + self.state.size()
    }
}

#[derive(Debug)]
struct DenseGridState {
    agg: Rasterize2DAgg,
    grid_len: usize,
    counts: Vec<u64>,
    sums: Vec<f64>,
    values: Vec<f64>,
    means: Vec<f64>,
    m2s: Vec<f64>,
}

impl DenseGridState {
    fn new(agg: Rasterize2DAgg, grid_len: usize, group_count: usize) -> Self {
        let len = grid_len * group_count;
        Self {
            agg,
            grid_len,
            counts: vec![0; len],
            sums: if agg.uses_sums() {
                vec![0.0; len]
            } else {
                Vec::new()
            },
            values: if agg.uses_values() {
                vec![0.0; len]
            } else {
                Vec::new()
            },
            means: if agg.uses_moments() {
                vec![0.0; len]
            } else {
                Vec::new()
            },
            m2s: if agg.uses_moments() {
                vec![0.0; len]
            } else {
                Vec::new()
            },
        }
    }

    fn group_count(&self) -> usize {
        self.counts.len() / self.grid_len
    }

    fn resize_groups(&mut self, group_count: usize) {
        let len = self.grid_len * group_count;
        self.counts.resize(len, 0);
        if self.agg.uses_sums() {
            self.sums.resize(len, 0.0);
        }
        if self.agg.uses_values() {
            self.values.resize(len, 0.0);
        }
        if self.agg.uses_moments() {
            self.means.resize(len, 0.0);
            self.m2s.resize(len, 0.0);
        }
    }

    fn size(&self) -> usize {
        self.counts.capacity() * size_of::<u64>()
            + self.sums.capacity() * size_of::<f64>()
            + self.values.capacity() * size_of::<f64>()
            + self.means.capacity() * size_of::<f64>()
            + self.m2s.capacity() * size_of::<f64>()
    }

    fn update(
        &mut self,
        config: &Rasterize2DGridConfig,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
        opt_filter: Option<&BooleanArray>,
    ) -> DataFusionResult<()> {
        if arrays.len() != 2 && arrays.len() != 3 {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D expected 2 or 3 arguments, got {}",
                arrays.len()
            )));
        }
        if self.agg != Rasterize2DAgg::Count && arrays.len() != 3 {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D reducer \"{}\" requires a value argument",
                self.agg.name()
            )));
        }
        let x = f64_array(arrays.first(), "Rasterize2D x argument")?;
        let y = f64_array(arrays.get(1), "Rasterize2D y argument")?;
        let value = arrays
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
            let value = match value {
                Some(value) => {
                    if value.is_null(row) || !value.value(row).is_finite() {
                        continue;
                    }
                    Some(value.value(row))
                }
                None => None,
            };
            let offset = group_indices.map_or(0, |indices| indices[row] * self.grid_len);
            self.update_cell(offset + cell_index, value);
        }
        Ok(())
    }

    fn update_cell(&mut self, index: usize, value: Option<f64>) {
        match self.agg {
            Rasterize2DAgg::Count => {
                self.counts[index] = self.counts[index].saturating_add(1);
            }
            Rasterize2DAgg::Sum => {
                self.counts[index] = self.counts[index].saturating_add(1);
                self.sums[index] += value.expect("sum has a value");
            }
            Rasterize2DAgg::Min => {
                let value = value.expect("min has a value");
                if self.counts[index] == 0 || value < self.values[index] {
                    self.values[index] = value;
                }
                self.counts[index] = self.counts[index].saturating_add(1);
            }
            Rasterize2DAgg::Max => {
                let value = value.expect("max has a value");
                if self.counts[index] == 0 || value > self.values[index] {
                    self.values[index] = value;
                }
                self.counts[index] = self.counts[index].saturating_add(1);
            }
            Rasterize2DAgg::Mean => {
                self.counts[index] = self.counts[index].saturating_add(1);
                self.sums[index] += value.expect("mean has a value");
            }
            Rasterize2DAgg::VarPop
            | Rasterize2DAgg::StddevPop
            | Rasterize2DAgg::VarSamp
            | Rasterize2DAgg::StddevSamp => {
                let value = value.expect("variance has a value");
                let count = self.counts[index] + 1;
                let delta = value - self.means[index];
                self.means[index] += delta / count as f64;
                let delta2 = value - self.means[index];
                self.m2s[index] += delta * delta2;
                self.counts[index] = count;
            }
        }
    }

    /// Merge one cell's incoming state into `target`. `count` must be
    /// non-zero; unused per-agg inputs are ignored.
    fn merge_cell(&mut self, target: usize, count: u64, sum: f64, value: f64, mean: f64, m2: f64) {
        match self.agg {
            Rasterize2DAgg::Count => {
                self.counts[target] = self.counts[target].saturating_add(count);
            }
            Rasterize2DAgg::Sum | Rasterize2DAgg::Mean => {
                self.counts[target] = self.counts[target].saturating_add(count);
                self.sums[target] += sum;
            }
            Rasterize2DAgg::Min => {
                if self.counts[target] == 0 || value < self.values[target] {
                    self.values[target] = value;
                }
                self.counts[target] = self.counts[target].saturating_add(count);
            }
            Rasterize2DAgg::Max => {
                if self.counts[target] == 0 || value > self.values[target] {
                    self.values[target] = value;
                }
                self.counts[target] = self.counts[target].saturating_add(count);
            }
            Rasterize2DAgg::VarPop
            | Rasterize2DAgg::StddevPop
            | Rasterize2DAgg::VarSamp
            | Rasterize2DAgg::StddevSamp => {
                merge_moments(
                    &mut self.counts[target],
                    &mut self.means[target],
                    &mut self.m2s[target],
                    count,
                    mean,
                    m2,
                );
            }
        }
    }

    /// Output cell value at a flat index for f64-valued reducers.
    fn output_cell(&self, index: usize) -> Option<f64> {
        let count = self.counts[index];
        match self.agg {
            Rasterize2DAgg::Count => unreachable!("count output is UInt64"),
            Rasterize2DAgg::Sum => (count > 0).then_some(self.sums[index]),
            Rasterize2DAgg::Min | Rasterize2DAgg::Max => (count > 0).then_some(self.values[index]),
            Rasterize2DAgg::Mean => (count > 0).then_some(self.sums[index] / count as f64),
            Rasterize2DAgg::VarPop | Rasterize2DAgg::StddevPop => (count > 0).then(|| {
                let variance = self.m2s[index] / count as f64;
                if self.agg == Rasterize2DAgg::VarPop {
                    variance
                } else {
                    variance.sqrt()
                }
            }),
            Rasterize2DAgg::VarSamp | Rasterize2DAgg::StddevSamp => (count > 1).then(|| {
                let variance = self.m2s[index] / (count - 1) as f64;
                if self.agg == Rasterize2DAgg::VarSamp {
                    variance
                } else {
                    variance.sqrt()
                }
            }),
        }
    }

    fn state_scalars(&self) -> DataFusionResult<Vec<ScalarValue>> {
        self.state_arrays()?
            .into_iter()
            .map(|array| {
                let array = array
                    .as_any()
                    .downcast_ref::<ListArray>()
                    .expect("state array is a ListArray")
                    .clone();
                Ok(ScalarValue::List(Arc::new(array)))
            })
            .collect()
    }

    fn state_arrays(&self) -> DataFusionResult<Vec<ArrayRef>> {
        let mut arrays =
            vec![
                u64_list_array_from_rows(self.counts.chunks(self.grid_len), self.grid_len, false)?
                    as ArrayRef,
            ];
        if self.agg.uses_sums() {
            arrays.push(f64_list_array_from_rows(
                self.sums.chunks(self.grid_len),
                self.grid_len,
                false,
            )? as ArrayRef);
        }
        if self.agg.uses_values() {
            arrays.push(f64_list_array_from_rows(
                self.values.chunks(self.grid_len),
                self.grid_len,
                false,
            )? as ArrayRef);
        }
        if self.agg.uses_moments() {
            arrays.push(f64_list_array_from_rows(
                self.means.chunks(self.grid_len),
                self.grid_len,
                false,
            )? as ArrayRef);
            arrays.push(f64_list_array_from_rows(
                self.m2s.chunks(self.grid_len),
                self.grid_len,
                false,
            )? as ArrayRef);
        }
        Ok(arrays)
    }

    fn merge(
        &mut self,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
    ) -> DataFusionResult<()> {
        let counts = list_array(arrays.first(), "Rasterize2D counts state")?;
        let rows = counts.len();
        if let Some(group_indices) = group_indices
            && rows != group_indices.len()
        {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D state length {rows} did not match group index length {}",
                group_indices.len()
            )));
        }

        let sums = if self.agg.uses_sums() {
            Some(list_array(arrays.get(1), "Rasterize2D sums state")?)
        } else {
            None
        };
        let values = if self.agg.uses_values() {
            Some(list_array(arrays.get(1), "Rasterize2D values state")?)
        } else {
            None
        };
        let (means, m2s) = if self.agg.uses_moments() {
            (
                Some(list_array(arrays.get(1), "Rasterize2D means state")?),
                Some(list_array(arrays.get(2), "Rasterize2D m2s state")?),
            )
        } else {
            (None, None)
        };

        for row in 0..rows {
            if counts.is_null(row) {
                continue;
            }
            let target_group = group_indices.map_or(0, |indices| indices[row]);
            let target_offset = target_group * self.grid_len;
            let counts_row = u64_list_value(counts, row, self.grid_len, "counts")?;
            let sums_row = sums
                .map(|array| f64_list_value(array, row, self.grid_len, "sums"))
                .transpose()?;
            let values_row = values
                .map(|array| f64_list_value(array, row, self.grid_len, "values"))
                .transpose()?;
            let means_row = means
                .map(|array| f64_list_value(array, row, self.grid_len, "means"))
                .transpose()?;
            let m2s_row = m2s
                .map(|array| f64_list_value(array, row, self.grid_len, "m2s"))
                .transpose()?;
            for cell in 0..self.grid_len {
                let count = counts_row.value(cell);
                if count == 0 {
                    continue;
                }
                self.merge_cell(
                    target_offset + cell,
                    count,
                    sums_row.as_ref().map_or(0.0, |row| row.value(cell)),
                    values_row.as_ref().map_or(0.0, |row| row.value(cell)),
                    means_row.as_ref().map_or(0.0, |row| row.value(cell)),
                    m2s_row.as_ref().map_or(0.0, |row| row.value(cell)),
                );
            }
        }
        Ok(())
    }

    fn take_emit(&mut self, emit_to: EmitTo) -> DataFusionResult<Self> {
        let emit_groups = match emit_to {
            EmitTo::All => self.group_count(),
            EmitTo::First(count) => count,
        };
        if emit_groups > self.group_count() {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D emit requested {emit_groups} groups but only {} are available",
                self.group_count()
            )));
        }
        let emit_len = emit_groups * self.grid_len;
        let all = matches!(emit_to, EmitTo::All);
        Ok(Self {
            agg: self.agg,
            grid_len: self.grid_len,
            counts: take_emit_values(&mut self.counts, emit_len, all),
            sums: take_emit_values(&mut self.sums, emit_len, all),
            values: take_emit_values(&mut self.values, emit_len, all),
            means: take_emit_values(&mut self.means, emit_len, all),
            m2s: take_emit_values(&mut self.m2s, emit_len, all),
        })
    }

    fn build_raster_array(
        &self,
        config: &Rasterize2DGridConfig,
    ) -> DataFusionResult<Arc<StructArray>> {
        match self.agg {
            Rasterize2DAgg::Count => {
                build_count_raster_array(config, self.counts.chunks(self.grid_len))
            }
            Rasterize2DAgg::Sum
            | Rasterize2DAgg::Min
            | Rasterize2DAgg::Max
            | Rasterize2DAgg::Mean
            | Rasterize2DAgg::VarPop
            | Rasterize2DAgg::StddevPop
            | Rasterize2DAgg::VarSamp
            | Rasterize2DAgg::StddevSamp => build_f64_raster_array(config, self.output_f64_rows()),
        }
    }

    fn output_f64_rows(&self) -> Vec<Vec<Option<f64>>> {
        (0..self.group_count())
            .map(|group| {
                let offset = group * self.grid_len;
                (0..self.grid_len)
                    .map(|cell| self.output_cell(offset + cell))
                    .collect()
            })
            .collect()
    }
}

/// Accumulator state: dense 2-D grid, or one dense grid per discovered
/// category when `Rasterize2D::by(...)` is configured.
#[derive(Debug)]
enum GridState {
    Dense(DenseGridState),
    Categorical(CategoricalGridState),
}

impl GridState {
    fn new(config: &Rasterize2DGridConfig, agg: Rasterize2DAgg, group_count: usize) -> Self {
        if config.by_dim_name.is_some() {
            Self::Categorical(CategoricalGridState::new(agg, config.grid_len, group_count))
        } else {
            Self::Dense(DenseGridState::new(agg, config.grid_len, group_count))
        }
    }

    fn agg(&self) -> Rasterize2DAgg {
        match self {
            Self::Dense(state) => state.agg,
            Self::Categorical(state) => state.agg,
        }
    }

    fn resize_groups(&mut self, group_count: usize) {
        match self {
            Self::Dense(state) => state.resize_groups(group_count),
            Self::Categorical(state) => state.resize_groups(group_count),
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::Dense(state) => state.size(),
            Self::Categorical(state) => state.size(),
        }
    }

    fn group_count(&self) -> usize {
        match self {
            Self::Dense(state) => state.group_count(),
            Self::Categorical(state) => state.group_count,
        }
    }

    fn update(
        &mut self,
        config: &Rasterize2DGridConfig,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
        opt_filter: Option<&BooleanArray>,
    ) -> DataFusionResult<()> {
        match self {
            Self::Dense(state) => state.update(config, arrays, group_indices, opt_filter),
            Self::Categorical(state) => state.update(config, arrays, group_indices, opt_filter),
        }
    }

    fn state_scalars(&self) -> DataFusionResult<Vec<ScalarValue>> {
        match self {
            Self::Dense(state) => state.state_scalars(),
            Self::Categorical(state) => state.state_scalars(),
        }
    }

    fn state_arrays(&self) -> DataFusionResult<Vec<ArrayRef>> {
        match self {
            Self::Dense(state) => state.state_arrays(),
            Self::Categorical(state) => state.state_arrays(),
        }
    }

    fn merge(
        &mut self,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
    ) -> DataFusionResult<()> {
        match self {
            Self::Dense(state) => state.merge(arrays, group_indices),
            Self::Categorical(state) => state.merge(arrays, group_indices),
        }
    }

    fn take_emit(&mut self, emit_to: EmitTo) -> DataFusionResult<Self> {
        match self {
            Self::Dense(state) => state.take_emit(emit_to).map(Self::Dense),
            Self::Categorical(state) => state.take_emit(emit_to).map(Self::Categorical),
        }
    }

    fn build_raster_array(
        &self,
        config: &Rasterize2DGridConfig,
    ) -> DataFusionResult<Arc<StructArray>> {
        match self {
            Self::Dense(state) => state.build_raster_array(config),
            Self::Categorical(state) => state.build_raster_array(config),
        }
    }
}

/// One dense grid per discovered category, sharing a global category table.
///
/// Planes are keyed by the STRINGIFIED category value; discovery order is
/// arbitrary (input order, then merge order), so serialized state carries
/// the category list and [`CategoricalGridState::merge`] unifies planes by
/// value. Emission sorts categories per group so output is deterministic.
#[derive(Debug)]
struct CategoricalGridState {
    agg: Rasterize2DAgg,
    grid_len: usize,
    group_count: usize,
    categories: Vec<String>,
    lookup: HashMap<String, usize>,
    planes: Vec<DenseGridState>,
}

impl CategoricalGridState {
    fn new(agg: Rasterize2DAgg, grid_len: usize, group_count: usize) -> Self {
        Self {
            agg,
            grid_len,
            group_count,
            categories: Vec::new(),
            lookup: HashMap::new(),
            planes: Vec::new(),
        }
    }

    fn plane_index(&mut self, category: &str) -> usize {
        if let Some(index) = self.lookup.get(category) {
            return *index;
        }
        let index = self.planes.len();
        let mut plane = DenseGridState::new(self.agg, self.grid_len, 0);
        plane.resize_groups(self.group_count);
        self.categories.push(category.to_string());
        self.lookup.insert(category.to_string(), index);
        self.planes.push(plane);
        index
    }

    fn resize_groups(&mut self, group_count: usize) {
        self.group_count = group_count;
        for plane in &mut self.planes {
            plane.resize_groups(group_count);
        }
    }

    fn size(&self) -> usize {
        size_of::<Self>()
            + self
                .categories
                .iter()
                .map(|category| category.len() * 2)
                .sum::<usize>()
            + self.planes.iter().map(DenseGridState::size).sum::<usize>()
    }

    fn update(
        &mut self,
        config: &Rasterize2DGridConfig,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
        opt_filter: Option<&BooleanArray>,
    ) -> DataFusionResult<()> {
        if arrays.len() != 3 && arrays.len() != 4 {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D with by(...) expected 3 or 4 arguments, got {}",
                arrays.len()
            )));
        }
        if self.agg != Rasterize2DAgg::Count && arrays.len() != 4 {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D reducer \"{}\" requires a value argument",
                self.agg.name()
            )));
        }
        let x = f64_array(arrays.first(), "Rasterize2D x argument")?;
        let y = f64_array(arrays.get(1), "Rasterize2D y argument")?;
        let value = (arrays.len() == 4)
            .then(|| f64_array(arrays.get(2), "Rasterize2D value argument"))
            .transpose()?;
        let category = arrays
            .last()
            .expect("length checked above")
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| {
                DataFusionError::Internal("Rasterize2D by argument must be Utf8".to_string())
            })?;
        if x.len() != y.len()
            || category.len() != x.len()
            || value.is_some_and(|value| value.len() != x.len())
        {
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
            if x.is_null(row) || y.is_null(row) || category.is_null(row) {
                continue;
            }
            let Some(cell_index) = config.cell_index(x.value(row), y.value(row)) else {
                continue;
            };
            let value = match value {
                Some(value) => {
                    if value.is_null(row) || !value.value(row).is_finite() {
                        continue;
                    }
                    Some(value.value(row))
                }
                None => None,
            };
            let plane = self.plane_index(category.value(row));
            let offset = group_indices.map_or(0, |indices| indices[row] * self.grid_len);
            self.planes[plane].update_cell(offset + cell_index, value);
        }
        Ok(())
    }

    /// State rows: `categories` (table order) plus each agg state as a
    /// plane-major `K * grid_len` list per group.
    fn state_arrays(&self) -> DataFusionResult<Vec<ArrayRef>> {
        let group_count = self.group_count;
        let plane_count = self.planes.len();
        let row_len = plane_count * self.grid_len;

        let categories = string_list_array_from_rows(
            (0..group_count).map(|_| {
                self.categories
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            }),
            false,
        )?;
        let mut arrays: Vec<ArrayRef> = vec![categories];

        let concat_u64 = |select: &dyn Fn(&DenseGridState) -> &Vec<u64>| -> Vec<u64> {
            let mut flat = Vec::with_capacity(group_count * row_len);
            for group in 0..group_count {
                let offset = group * self.grid_len;
                for plane in &self.planes {
                    flat.extend_from_slice(&select(plane)[offset..offset + self.grid_len]);
                }
            }
            flat
        };
        let concat_f64 = |select: &dyn Fn(&DenseGridState) -> &Vec<f64>| -> Vec<f64> {
            let mut flat = Vec::with_capacity(group_count * row_len);
            for group in 0..group_count {
                let offset = group * self.grid_len;
                for plane in &self.planes {
                    flat.extend_from_slice(&select(plane)[offset..offset + self.grid_len]);
                }
            }
            flat
        };

        let empty_u64: Vec<u64> = Vec::new();
        let empty_f64: Vec<f64> = Vec::new();
        fn rows_of<'a, T>(
            flat: &'a [T],
            row_len: usize,
            group_count: usize,
            empty: &'a [T],
        ) -> Vec<&'a [T]> {
            if row_len == 0 {
                (0..group_count).map(|_| empty).collect()
            } else {
                flat.chunks(row_len).collect()
            }
        }

        let counts = concat_u64(&|plane| &plane.counts);
        arrays.push(u64_list_array_from_rows(
            rows_of(&counts, row_len, group_count, &empty_u64),
            row_len,
            false,
        )? as ArrayRef);
        if self.agg.uses_sums() {
            let sums = concat_f64(&|plane| &plane.sums);
            arrays.push(f64_list_array_from_rows(
                rows_of(&sums, row_len, group_count, &empty_f64),
                row_len,
                false,
            )? as ArrayRef);
        }
        if self.agg.uses_values() {
            let values = concat_f64(&|plane| &plane.values);
            arrays.push(f64_list_array_from_rows(
                rows_of(&values, row_len, group_count, &empty_f64),
                row_len,
                false,
            )? as ArrayRef);
        }
        if self.agg.uses_moments() {
            let means = concat_f64(&|plane| &plane.means);
            let m2s = concat_f64(&|plane| &plane.m2s);
            arrays.push(f64_list_array_from_rows(
                rows_of(&means, row_len, group_count, &empty_f64),
                row_len,
                false,
            )? as ArrayRef);
            arrays.push(f64_list_array_from_rows(
                rows_of(&m2s, row_len, group_count, &empty_f64),
                row_len,
                false,
            )? as ArrayRef);
        }
        Ok(arrays)
    }

    fn state_scalars(&self) -> DataFusionResult<Vec<ScalarValue>> {
        self.state_arrays()?
            .into_iter()
            .map(|array| {
                let array = array
                    .as_any()
                    .downcast_ref::<ListArray>()
                    .expect("state array is a ListArray")
                    .clone();
                Ok(ScalarValue::List(Arc::new(array)))
            })
            .collect()
    }

    fn merge(
        &mut self,
        arrays: &[ArrayRef],
        group_indices: Option<&[usize]>,
    ) -> DataFusionResult<()> {
        let categories = list_array(arrays.first(), "Rasterize2D categories state")?;
        let counts = list_array(arrays.get(1), "Rasterize2D counts state")?;
        let rows = counts.len();
        if let Some(group_indices) = group_indices
            && rows != group_indices.len()
        {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D state length {rows} did not match group index length {}",
                group_indices.len()
            )));
        }

        let sums = self
            .agg
            .uses_sums()
            .then(|| list_array(arrays.get(2), "Rasterize2D sums state"))
            .transpose()?;
        let values = self
            .agg
            .uses_values()
            .then(|| list_array(arrays.get(2), "Rasterize2D values state"))
            .transpose()?;
        let (means, m2s) = if self.agg.uses_moments() {
            (
                Some(list_array(arrays.get(2), "Rasterize2D means state")?),
                Some(list_array(arrays.get(3), "Rasterize2D m2s state")?),
            )
        } else {
            (None, None)
        };

        for row in 0..rows {
            if counts.is_null(row) || categories.is_null(row) {
                continue;
            }
            let categories_row = categories.value(row);
            let categories_row = categories_row
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(|| {
                    DataFusionError::Internal(
                        "Rasterize2D categories state must be Utf8 lists".to_string(),
                    )
                })?;
            let plane_count = categories_row.len();
            let row_len = plane_count * self.grid_len;
            let target_group = group_indices.map_or(0, |indices| indices[row]);
            let target_offset = target_group * self.grid_len;

            let counts_row = u64_list_value(counts, row, row_len, "counts")?;
            let sums_row = sums
                .map(|array| f64_list_value(array, row, row_len, "sums"))
                .transpose()?;
            let values_row = values
                .map(|array| f64_list_value(array, row, row_len, "values"))
                .transpose()?;
            let means_row = means
                .map(|array| f64_list_value(array, row, row_len, "means"))
                .transpose()?;
            let m2s_row = m2s
                .map(|array| f64_list_value(array, row, row_len, "m2s"))
                .transpose()?;

            for incoming_plane in 0..plane_count {
                let local_plane = self.plane_index(categories_row.value(incoming_plane));
                let plane_offset = incoming_plane * self.grid_len;
                for cell in 0..self.grid_len {
                    let source = plane_offset + cell;
                    let count = counts_row.value(source);
                    if count == 0 {
                        continue;
                    }
                    self.planes[local_plane].merge_cell(
                        target_offset + cell,
                        count,
                        sums_row.as_ref().map_or(0.0, |row| row.value(source)),
                        values_row.as_ref().map_or(0.0, |row| row.value(source)),
                        means_row.as_ref().map_or(0.0, |row| row.value(source)),
                        m2s_row.as_ref().map_or(0.0, |row| row.value(source)),
                    );
                }
            }
        }
        Ok(())
    }

    fn take_emit(&mut self, emit_to: EmitTo) -> DataFusionResult<Self> {
        let emit_groups = match emit_to {
            EmitTo::All => self.group_count,
            EmitTo::First(count) => count,
        };
        if emit_groups > self.group_count {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D emit requested {emit_groups} groups but only {} are available",
                self.group_count
            )));
        }
        let planes = self
            .planes
            .iter_mut()
            .map(|plane| plane.take_emit(emit_to))
            .collect::<DataFusionResult<Vec<_>>>()?;
        let emitted = Self {
            agg: self.agg,
            grid_len: self.grid_len,
            group_count: emit_groups,
            categories: self.categories.clone(),
            lookup: self.lookup.clone(),
            planes,
        };
        self.group_count -= emit_groups;
        Ok(emitted)
    }

    /// Per-group emission: only OBSERVED categories (any nonzero count in
    /// the group) get planes, sorted by category value for determinism.
    fn build_raster_array(
        &self,
        config: &Rasterize2DGridConfig,
    ) -> DataFusionResult<Arc<StructArray>> {
        let mut rows = Vec::with_capacity(self.group_count);
        for group in 0..self.group_count {
            let offset = group * self.grid_len;
            let mut observed = self
                .planes
                .iter()
                .enumerate()
                .filter(|(_, plane)| {
                    plane.counts[offset..offset + self.grid_len]
                        .iter()
                        .any(|count| *count > 0)
                })
                .map(|(index, _)| (self.categories[index].clone(), index))
                .collect::<Vec<_>>();
            observed.sort_by(|left, right| left.0.cmp(&right.0));

            let categories = observed
                .iter()
                .map(|(category, _)| category.clone())
                .collect::<Vec<_>>();
            let data = if self.agg == Rasterize2DAgg::Count {
                let mut data = Vec::with_capacity(observed.len() * self.grid_len);
                for (_, plane_index) in &observed {
                    data.extend_from_slice(
                        &self.planes[*plane_index].counts[offset..offset + self.grid_len],
                    );
                }
                CategoricalRowData::U64(data)
            } else {
                let mut data = Vec::with_capacity(observed.len() * self.grid_len);
                for (_, plane_index) in &observed {
                    let plane = &self.planes[*plane_index];
                    data.extend((0..self.grid_len).map(|cell| plane.output_cell(offset + cell)));
                }
                CategoricalRowData::F64(data)
            };
            rows.push(CategoricalRasterRow { categories, data });
        }
        build_categorical_raster_array(config, rows)
    }
}

struct CategoricalRasterRow {
    categories: Vec<String>,
    data: CategoricalRowData,
}

enum CategoricalRowData {
    U64(Vec<u64>),
    F64(Vec<Option<f64>>),
}

fn take_emit_values<T>(values: &mut Vec<T>, emit_len: usize, all: bool) -> Vec<T> {
    if values.is_empty() {
        Vec::new()
    } else if all {
        std::mem::take(values)
    } else {
        values.drain(0..emit_len).collect()
    }
}

fn merge_moments(
    count_a: &mut u64,
    mean_a: &mut f64,
    m2_a: &mut f64,
    count_b: u64,
    mean_b: f64,
    m2_b: f64,
) {
    if count_b == 0 {
        return;
    }
    if *count_a == 0 {
        *count_a = count_b;
        *mean_a = mean_b;
        *m2_a = m2_b;
        return;
    }
    let count = *count_a + count_b;
    let delta = mean_b - *mean_a;
    *mean_a += delta * count_b as f64 / count as f64;
    *m2_a += m2_b + delta * delta * *count_a as f64 * count_b as f64 / count as f64;
    *count_a = count;
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

fn raster_data_type(cell_type: DataType, cell_nullable: bool) -> DataType {
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
        Field::new("crs", DataType::Utf8, true),
        Field::new("dimensions", dimensions_type, false),
    ]));
    let values_type = DataType::Struct(Fields::from(vec![
        Field::new("dims", DataType::new_list(DataType::Utf8, false), false),
        Field::new("data", DataType::new_list(cell_type, cell_nullable), false),
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
    let data = u64_list_array_from_rows(rows, config.grid_len, false)? as ArrayRef;
    build_raster_array(config, data)
}

fn build_f64_raster_array(
    config: &Rasterize2DGridConfig,
    rows: Vec<Vec<Option<f64>>>,
) -> DataFusionResult<Arc<StructArray>> {
    let data = f64_option_list_array_from_rows(rows, config.grid_len)? as ArrayRef;
    build_raster_array(config, data)
}

fn build_raster_array(
    config: &Rasterize2DGridConfig,
    data: ArrayRef,
) -> DataFusionResult<Arc<StructArray>> {
    let row_count = data.len();
    let dimensions = dimensions_list_array(config, row_count)?;
    let crs_values = vec![config.crs.as_deref(); row_count];
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"; row_count])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("crs", DataType::Utf8, true)),
            Arc::new(StringArray::from(crs_values)) as ArrayRef,
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

/// Raster rows with a trailing categorical plane dimension: geometry dims
/// `[x uniform, y uniform, by categorical]`, `values.dims = [by, y, x]`,
/// plane-major data with per-row plane counts.
fn build_categorical_raster_array(
    config: &Rasterize2DGridConfig,
    rows: Vec<CategoricalRasterRow>,
) -> DataFusionResult<Arc<StructArray>> {
    let by_dim_name = config.by_dim_name.as_deref().ok_or_else(|| {
        DataFusionError::Internal(
            "Rasterize2D categorical emission requires a by dimension name".to_string(),
        )
    })?;
    let row_count = rows.len();

    // Data list with per-row lengths K_i * grid_len.
    let mut lengths = Vec::with_capacity(row_count);
    // Cell type + nullability mirror raster_data_type: count -> UInt64
    // non-nullable, all other reducers -> nullable Float64.
    let (data, value_nullable): (ArrayRef, bool) = match rows
        .first()
        .map(|row| matches!(row.data, CategoricalRowData::U64(_)))
    {
        Some(true) | None => {
            let mut flat: Vec<u64> = Vec::new();
            for row in &rows {
                let CategoricalRowData::U64(values) = &row.data else {
                    return Err(DataFusionError::Internal(
                        "Rasterize2D categorical rows mixed cell types".to_string(),
                    ));
                };
                lengths.push(values.len());
                flat.extend_from_slice(values);
            }
            (Arc::new(UInt64Array::from(flat)) as ArrayRef, false)
        }
        Some(false) => {
            let mut flat: Vec<Option<f64>> = Vec::new();
            for row in &rows {
                let CategoricalRowData::F64(values) = &row.data else {
                    return Err(DataFusionError::Internal(
                        "Rasterize2D categorical rows mixed cell types".to_string(),
                    ));
                };
                lengths.push(values.len());
                flat.extend(values.iter().copied());
            }
            (Arc::new(Float64Array::from(flat)) as ArrayRef, true)
        }
    };
    let data = list_array_from_lengths(data, lengths, value_nullable);

    // Geometry dimensions: per row, x + y uniform dims followed by the
    // categorical by dim carrying that row's observed categories.
    let mut names = Vec::with_capacity(row_count * 3);
    let mut kinds = Vec::with_capacity(row_count * 3);
    let mut samplings = Vec::with_capacity(row_count * 3);
    let mut starts = Vec::with_capacity(row_count * 3);
    let mut stops = Vec::with_capacity(row_count * 3);
    let mut counts = Vec::with_capacity(row_count * 3);
    let mut coord_values_builder = ListBuilder::new(StringBuilder::new());
    for row in &rows {
        names.push(config.x_dim_name.as_str());
        kinds.push("uniform");
        samplings.push(Some(config.x_sampling.as_str()));
        starts.push(Some(config.x_start));
        stops.push(Some(config.x_stop));
        counts.push(Some(config.x_bins));
        coord_values_builder.append(false);

        names.push(config.y_dim_name.as_str());
        kinds.push("uniform");
        samplings.push(Some(config.y_sampling.as_str()));
        starts.push(Some(config.y_start));
        stops.push(Some(config.y_stop));
        counts.push(Some(config.y_bins));
        coord_values_builder.append(false);

        names.push(by_dim_name);
        kinds.push("categorical");
        samplings.push(None);
        starts.push(None);
        stops.push(None);
        counts.push(None);
        for category in &row.categories {
            coord_values_builder.values().append_value(category);
        }
        coord_values_builder.append(true);
    }
    let coord_values = Arc::new(coord_values_builder.finish()) as ArrayRef;
    let total_dimensions = row_count * 3;
    debug_assert_eq!(names.len(), total_dimensions);
    let coords = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(kinds)) as ArrayRef,
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
    let dimensions = list_array_from_lengths(dimensions, vec![3; row_count], false);

    let crs_values = vec![config.crs.as_deref(); row_count];
    let geometry = Arc::new(StructArray::from(vec![
        (
            Arc::new(Field::new("kind", DataType::Utf8, false)),
            Arc::new(StringArray::from(vec!["grid"; row_count])) as ArrayRef,
        ),
        (
            Arc::new(Field::new("crs", DataType::Utf8, true)),
            Arc::new(StringArray::from(crs_values)) as ArrayRef,
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
        (0..row_count).map(|_| {
            vec![
                by_dim_name,
                config.y_dim_name.as_str(),
                config.x_dim_name.as_str(),
            ]
        }),
        false,
    )?;
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

fn f64_list_array_from_rows<'a>(
    rows: impl IntoIterator<Item = &'a [f64]>,
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
    let values = Arc::new(Float64Array::from(values)) as ArrayRef;
    Ok(
        list_array_from_lengths(values, vec![expected_len; rows.len()], value_nullable)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("list_array_from_lengths returned a ListArray")
            .clone()
            .into(),
    )
}

fn f64_option_list_array_from_rows(
    rows: Vec<Vec<Option<f64>>>,
    expected_len: usize,
) -> DataFusionResult<Arc<ListArray>> {
    let mut values = Vec::with_capacity(rows.len() * expected_len);
    for row in &rows {
        if row.len() != expected_len {
            return Err(DataFusionError::Internal(format!(
                "Rasterize2D row length {} did not match expected length {expected_len}",
                row.len()
            )));
        }
        values.extend(row.iter().copied());
    }
    let values = Arc::new(Float64Array::from(values)) as ArrayRef;
    Ok(
        list_array_from_lengths(values, vec![expected_len; rows.len()], true)
            .as_any()
            .downcast_ref::<ListArray>()
            .expect("list_array_from_lengths returned a ListArray")
            .clone()
            .into(),
    )
}

fn u64_list_value(
    array: &ListArray,
    row: usize,
    expected_len: usize,
    label: &str,
) -> DataFusionResult<UInt64Array> {
    if array.is_null(row) {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D {label} state row {row} was null"
        )));
    }
    let values = array.value(row);
    let values = values
        .as_any()
        .downcast_ref::<UInt64Array>()
        .ok_or_else(|| {
            DataFusionError::Internal(format!("Rasterize2D {label} state must contain UInt64"))
        })?;
    if values.len() != expected_len {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D {label} state length {} did not match expected grid length {expected_len}",
            values.len()
        )));
    }
    Ok(values.clone())
}

fn f64_list_value(
    array: &ListArray,
    row: usize,
    expected_len: usize,
    label: &str,
) -> DataFusionResult<Float64Array> {
    if array.is_null(row) {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D {label} state row {row} was null"
        )));
    }
    let values = array.value(row);
    let values = values
        .as_any()
        .downcast_ref::<Float64Array>()
        .ok_or_else(|| {
            DataFusionError::Internal(format!("Rasterize2D {label} state must contain Float64"))
        })?;
    if values.len() != expected_len {
        return Err(DataFusionError::Internal(format!(
            "Rasterize2D {label} state length {} did not match expected grid length {expected_len}",
            values.len()
        )));
    }
    Ok(values.clone())
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

async fn collect_single_batch(
    dataframe: DataFrame,
    params: &IndexMap<String, ScalarValue>,
) -> Result<RecordBatch, AvengerChartError> {
    let dataframe = if let Some(param_values) = params_to_datafusion(params) {
        dataframe
            .with_param_values(param_values)
            .map_err(AvengerChartError::DataFusionError)?
    } else {
        dataframe
    };
    let batches = dataframe
        .collect()
        .await
        .map_err(AvengerChartError::DataFusionError)?;
    let Some(first) = batches.first() else {
        return Err(AvengerChartError::InternalError(
            "Rasterize2D materialization produced no RecordBatches".to_string(),
        ));
    };
    concat_batches(&first.schema(), &batches).map_err(AvengerChartError::ArrowError)
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
        test_config_with_crs(None)
    }

    fn test_config_with_crs(crs: Option<&str>) -> Rasterize2DGridConfig {
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
            crs.map(str::to_string),
            None,
        )
        .unwrap()
    }

    fn input_arrays(xs: Vec<f64>, ys: Vec<f64>) -> Vec<ArrayRef> {
        vec![
            Arc::new(Float64Array::from(xs)) as ArrayRef,
            Arc::new(Float64Array::from(ys)) as ArrayRef,
        ]
    }

    fn value_input_arrays(xs: Vec<f64>, ys: Vec<f64>, values: Vec<f64>) -> Vec<ArrayRef> {
        vec![
            Arc::new(Float64Array::from(xs)) as ArrayRef,
            Arc::new(Float64Array::from(ys)) as ArrayRef,
            Arc::new(Float64Array::from(values)) as ArrayRef,
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

    fn f64_values_from_raster(raster: &StructArray, row: usize) -> Vec<Option<f64>> {
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
        let row_values = row_values.as_any().downcast_ref::<Float64Array>().unwrap();
        (0..row_values.len())
            .map(|index| {
                if row_values.is_null(index) {
                    None
                } else {
                    Some(row_values.value(index))
                }
            })
            .collect()
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

    fn scalar_state_arrays(state: Vec<ScalarValue>) -> Vec<ArrayRef> {
        state
            .into_iter()
            .map(|value| match value {
                ScalarValue::List(array) => array as ArrayRef,
                other => panic!("expected list state, got {other:?}"),
            })
            .collect()
    }

    fn merged_value_reducer_output(agg: Rasterize2DAgg) -> Vec<Option<f64>> {
        let config = test_config();
        let mut partial_a = Rasterize2DAccumulator::new(config.clone(), agg);
        partial_a
            .update_batch(&value_input_arrays(
                vec![0.0, 2.0],
                vec![0.0, 2.0],
                vec![1.0, 5.0],
            ))
            .unwrap();
        let state_a = scalar_state_arrays(partial_a.state().unwrap());

        let mut partial_b = Rasterize2DAccumulator::new(config.clone(), agg);
        partial_b
            .update_batch(&value_input_arrays(
                vec![0.2, 2.0, 2.0, 2.0],
                vec![0.2, 0.0, 2.0, 2.0],
                vec![3.0, -2.0, 7.0, 9.0],
            ))
            .unwrap();
        let state_b = scalar_state_arrays(partial_b.state().unwrap());

        let mut merged = Rasterize2DAccumulator::new(config, agg);
        merged.merge_batch(&state_a).unwrap();
        merged.merge_batch(&state_b).unwrap();
        let ScalarValue::Struct(raster) = merged.evaluate().unwrap() else {
            panic!("expected struct scalar");
        };
        f64_values_from_raster(&raster, 0)
    }

    fn assert_option_f64_close(actual: &[Option<f64>], expected: &[Option<f64>]) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            match (actual, expected) {
                (Some(actual), Some(expected)) => assert!(
                    (actual - expected).abs() < 1e-12,
                    "index {index}: expected {expected}, got {actual}"
                ),
                (None, None) => {}
                _ => panic!("index {index}: expected {expected:?}, got {actual:?}"),
            }
        }
    }

    #[test]
    fn count_accumulator_merges_state_and_evaluates_struct_scalar() {
        let config = test_config();
        let mut partial = Rasterize2DAccumulator::new(config.clone(), Rasterize2DAgg::Count);
        partial
            .update_batch(&input_arrays(vec![0.0, 2.0], vec![0.0, 2.0]))
            .unwrap();
        assert!(partial.size() > 0);
        let mut state = partial.state().unwrap();
        let state_array = match state.pop().unwrap() {
            ScalarValue::List(array) => array as ArrayRef,
            other => panic!("expected list state, got {other:?}"),
        };

        let mut final_acc = Rasterize2DAccumulator::new(config, Rasterize2DAgg::Count);
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
        let mut groups = Rasterize2DGroupsAccumulator::new(config, Rasterize2DAgg::Count);
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
        let mut partial = Rasterize2DGroupsAccumulator::new(config.clone(), Rasterize2DAgg::Count);
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

        let mut merged = Rasterize2DGroupsAccumulator::new(config, Rasterize2DAgg::Count);
        merged.merge_batch(&state, &[0, 1], None, 2).unwrap();
        let raster = merged.evaluate(EmitTo::All).unwrap();
        let raster = raster.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(counts_from_raster(raster, 0), vec![1, 0, 0, 1]);
        assert_eq!(counts_from_raster(raster, 1), vec![1, 1, 0, 0]);
    }

    #[test]
    fn count_groups_accumulator_handles_empty_batch_with_known_groups() {
        let config = test_config();
        let mut groups = Rasterize2DGroupsAccumulator::new(config, Rasterize2DAgg::Count);
        groups
            .update_batch(&input_arrays(vec![], vec![]), &[], None, 2)
            .unwrap();

        let raster = groups.evaluate(EmitTo::All).unwrap();
        let raster = raster.as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(raster.len(), 2);
        assert_eq!(counts_from_raster(raster, 0), vec![0, 0, 0, 0]);
        assert_eq!(counts_from_raster(raster, 1), vec![0, 0, 0, 0]);
    }

    #[test]
    fn value_accumulator_merges_reducer_states() {
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::Sum),
            &[Some(4.0), Some(-2.0), None, Some(21.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::Min),
            &[Some(1.0), Some(-2.0), None, Some(5.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::Max),
            &[Some(3.0), Some(-2.0), None, Some(9.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::Mean),
            &[Some(2.0), Some(-2.0), None, Some(7.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::VarPop),
            &[Some(1.0), Some(0.0), None, Some(8.0 / 3.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::VarSamp),
            &[Some(2.0), None, None, Some(4.0)],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::StddevPop),
            &[Some(1.0), Some(0.0), None, Some((8.0_f64 / 3.0).sqrt())],
        );
        assert_option_f64_close(
            &merged_value_reducer_output(Rasterize2DAgg::StddevSamp),
            &[Some(2.0_f64.sqrt()), None, None, Some(2.0)],
        );
    }

    fn compiled_transform_with_frame(frame: Option<&str>) -> CompiledRasterize2DTransform {
        CompiledRasterize2DTransform {
            x: expr_node(col("x"), "rasterize x expression"),
            y: expr_node(col("y"), "rasterize y expression"),
            x_dim: Rasterize2DDimension::default().into_spec("x").unwrap(),
            y_dim: Rasterize2DDimension::default().into_spec("y").unwrap(),
            value: None,
            agg: Rasterize2DAgg::Count,
            partition_by: Vec::new(),
            raster_name: "raster".to_string(),
            x_dim_name: "x".to_string(),
            y_dim_name: "y".to_string(),
            frame: frame.map(str::to_string),
            by: None,
            by_dim_name: None,
        }
    }

    fn crs_from_raster(raster: &StructArray, row: usize) -> Option<String> {
        let geometry = raster
            .column_by_name("geometry")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let crs = geometry
            .column_by_name("crs")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        if crs.is_null(row) {
            None
        } else {
            Some(crs.value(row).to_string())
        }
    }

    #[test]
    fn frame_round_trips_through_serialized_compiled_spec() {
        use avenger_chart_core::CoordinationScope;

        // The builder threads frame() into the compiled transform, and the
        // typetag-serialized spec (what the async executor deserializes)
        // carries it.
        let (compiled, _output) = Rasterize2D::new(col("x"), col("y"))
            .frame("epsg:3857")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Shared))
            .unwrap();
        let json = serde_json::to_value(&compiled).unwrap();
        assert_eq!(json["frame"], serde_json::json!("epsg:3857"));

        let tagged = compiled_transform_with_frame(Some("epsg:3857"));
        let json = serde_json::to_value(&tagged).unwrap();
        assert_eq!(json["frame"], serde_json::json!("epsg:3857"));
        let round_tripped: CompiledRasterize2DTransform = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, tagged);

        // Untagged specs serialize without the key (so their bytes and hashes
        // are identical to pre-frame specs), and specs serialized before the
        // field existed still deserialize.
        let untagged = compiled_transform_with_frame(None);
        let json = serde_json::to_value(&untagged).unwrap();
        assert!(json.get("frame").is_none());
        let round_tripped: CompiledRasterize2DTransform = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped.frame, None);
    }

    #[test]
    fn frame_is_stamped_into_output_geometry_and_schema() {
        let mut acc = Rasterize2DAccumulator::new(
            test_config_with_crs(Some("epsg:3857")),
            Rasterize2DAgg::Count,
        );
        acc.update_batch(&input_arrays(vec![0.0, 2.0], vec![0.0, 2.0]))
            .unwrap();
        let ScalarValue::Struct(raster) = acc.evaluate().unwrap() else {
            panic!("expected struct scalar");
        };
        assert_eq!(crs_from_raster(&raster, 0), Some("epsg:3857".to_string()));
        // The built struct matches the declared UDAF return type (which is also
        // what empty_materialized_dataframe builds its schema from).
        assert_eq!(
            raster.data_type(),
            &raster_data_type(DataType::UInt64, false)
        );

        // Untagged rasters carry a null crs and the same schema.
        let mut acc = Rasterize2DAccumulator::new(test_config(), Rasterize2DAgg::Count);
        acc.update_batch(&input_arrays(vec![0.0], vec![0.0]))
            .unwrap();
        let ScalarValue::Struct(raster) = acc.evaluate().unwrap() else {
            panic!("expected struct scalar");
        };
        assert_eq!(crs_from_raster(&raster, 0), None);
        assert_eq!(
            raster.data_type(),
            &raster_data_type(DataType::UInt64, false)
        );
    }

    #[test]
    fn differing_frame_changes_identity_and_key_hashes() {
        use datafusion::prelude::SessionContext;

        let ctx = SessionContext::new();
        let batch = RecordBatch::try_from_iter(vec![
            (
                "x",
                Arc::new(Float64Array::from(vec![0.0, 1.0])) as ArrayRef,
            ),
            (
                "y",
                Arc::new(Float64Array::from(vec![0.0, 1.0])) as ArrayRef,
            ),
        ])
        .unwrap();
        let df = ctx.read_batch(batch).unwrap();
        let spec_for = |frame: Option<&str>| {
            Rasterize2DMaterializationSpec::new(
                df.clone(),
                compiled_transform_with_frame(frame),
                SerializableScalarMap::from(IndexMap::<String, ScalarValue>::new()),
            )
            .unwrap()
        };

        let untagged = spec_for(None);
        let mercator = spec_for(Some("epsg:3857"));
        let degrees = spec_for(Some("epsg:4326"));

        assert_ne!(
            untagged.identity().unwrap(),
            mercator.identity().unwrap(),
            "frame() must participate in the identity hash"
        );
        assert_ne!(
            mercator.identity().unwrap(),
            degrees.identity().unwrap(),
            "different frames must produce different identity hashes"
        );
        assert_ne!(untagged.key().unwrap(), mercator.key().unwrap());
        assert_ne!(mercator.key().unwrap(), degrees.key().unwrap());
    }
}
