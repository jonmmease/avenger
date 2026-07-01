use crate::common::{
    expr_node, map_expr_node, map_expr_nodes, map_optional_expr_node, simple_column_name,
    validate_output_names,
};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, IntoExpr, RasterDim, SerializableExpr, dim,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr, prelude::col};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

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
        _ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_output_names(
            dataframe.schema().fields().iter().map(|field| field.name()),
            [self.raster_name.as_str()],
        )?;
        Err(AvengerChartError::InternalError(
            "Rasterize2D transform execution is not implemented yet".to_string(),
        ))
    }
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
