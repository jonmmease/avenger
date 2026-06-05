use crate::aggregate::{AggregateMeasureSpec, AggregateOp, aggregate_expr};
use crate::common::{expr_node, sanitize_output_name, simple_column_name, validate_generated_name};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt,
    SerializableExpr,
};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::{Expr, JoinType, Operator, binary_expr, col, lit, when},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

const INPUT_KEY: &str = "__avenger_impute_input_key";
const DOMAIN_KEY: &str = "__avenger_impute_domain_key";
const INPUT_VALUE: &str = "__avenger_impute_input_value";
const PRESENT: &str = "__avenger_impute_present";
const FILL_VALUE: &str = "__avenger_impute_fill_value";

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledImputeTransform {
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub field: LogicalExprNode,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub key: LogicalExprNode,
    #[serde_as(as = "Vec<FromInto<SerializableExpr>>")]
    pub group_by: Vec<LogicalExprNode>,
    pub method: ImputeMethodSpec,
    pub value_name: String,
    pub flag_name: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ImputeMethodSpec {
    Value {
        #[serde_as(as = "FromInto<SerializableExpr>")]
        expr: LogicalExprNode,
    },
    Mean,
    Min,
    Max,
}

#[derive(Clone, Debug)]
pub struct Impute {
    field: Expr,
    key: Option<Expr>,
    group_by: Vec<Expr>,
    method: Option<ImputeMethod>,
    value_name: Option<String>,
    flag_name: Option<String>,
}

#[derive(Clone, Debug)]
enum ImputeMethod {
    Value(Expr),
    Mean,
    Min,
    Max,
}

impl Impute {
    pub fn new(field: Expr) -> Self {
        Self {
            field,
            key: None,
            group_by: Vec::new(),
            method: None,
            value_name: None,
            flag_name: None,
        }
    }

    pub fn key(mut self, expr: Expr) -> Self {
        self.key = Some(expr);
        self
    }

    pub fn group_by<I>(mut self, exprs: I) -> Self
    where
        I: IntoIterator<Item = Expr>,
    {
        self.group_by.extend(exprs);
        self
    }

    pub fn value(mut self, expr: Expr) -> Self {
        self.method = Some(ImputeMethod::Value(expr));
        self
    }

    pub fn mean(mut self) -> Self {
        self.method = Some(ImputeMethod::Mean);
        self
    }

    pub fn min(mut self) -> Self {
        self.method = Some(ImputeMethod::Min);
        self
    }

    pub fn max(mut self) -> Self {
        self.method = Some(ImputeMethod::Max);
        self
    }

    pub fn as_value(mut self, name: impl Into<String>) -> Self {
        self.value_name = Some(name.into());
        self
    }

    pub fn flag(mut self, name: impl Into<String>) -> Self {
        self.flag_name = Some(name.into());
        self
    }
}

impl DataTransform for Impute {
    type Output = ImputeOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        let Some(key) = self.key else {
            return Err(AvengerChartError::InvalidArgument(
                "Impute transform requires a key(...) expression".to_string(),
            ));
        };
        let Some(method) = self.method else {
            return Err(AvengerChartError::InvalidArgument(
                "Impute transform requires a fill method such as value(...), mean(), min(), or max()"
                    .to_string(),
            ));
        };
        let value_name = self.value_name.unwrap_or_else(|| {
            simple_column_name(&self.field)
                .unwrap_or_else(|| sanitize_output_name(&format!("{}_impute", self.field)))
        });
        validate_generated_name(&value_name)?;
        if let Some(flag_name) = &self.flag_name {
            validate_generated_name(flag_name)?;
            if flag_name == &value_name {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Data transform output name '{flag_name}' is duplicated"
                )));
            }
        }

        let output = ImputeOutput {
            value_name: value_name.clone(),
            flag_name: self.flag_name.clone(),
        };
        let method = match method {
            ImputeMethod::Value(expr) => ImputeMethodSpec::Value {
                expr: expr_node(expr, "impute value expression"),
            },
            ImputeMethod::Mean => ImputeMethodSpec::Mean,
            ImputeMethod::Min => ImputeMethodSpec::Min,
            ImputeMethod::Max => ImputeMethodSpec::Max,
        };
        Ok((
            Box::new(CompiledImputeTransform {
                field: expr_node(self.field, "impute field expression"),
                key: expr_node(key, "impute key expression"),
                group_by: self
                    .group_by
                    .into_iter()
                    .map(|expr| expr_node(expr, "impute group_by expression"))
                    .collect(),
                method,
                value_name,
                flag_name: self.flag_name,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct ImputeOutput {
    value_name: String,
    flag_name: Option<String>,
}

impl ImputeOutput {
    pub fn value(&self) -> Expr {
        col(&self.value_name)
    }

    pub fn flag(&self) -> Expr {
        let Some(flag_name) = &self.flag_name else {
            panic!("Impute flag output was requested, but no flag column was configured");
        };
        col(flag_name)
    }
}

#[typetag::serde(name = "impute")]
#[async_trait]
impl CompiledDataTransform for CompiledImputeTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_generated_name(&self.value_name)?;
        if let Some(flag_name) = &self.flag_name {
            validate_generated_name(flag_name)?;
            if flag_name == &self.value_name {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Data transform output name '{flag_name}' is duplicated"
                )));
            }
        }

        let original_names = dataframe
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().clone())
            .collect::<Vec<_>>();
        let group_names = group_hidden_names(self.group_by.len());
        let domain_group_names = group_domain_names(self.group_by.len());

        let keyed = add_hidden_columns(dataframe, self, &group_names, ctx)?;
        let key_domain = keyed
            .clone()
            .select(vec![col(INPUT_KEY).alias(DOMAIN_KEY)])
            .map_err(AvengerChartError::DataFusionError)?
            .distinct()
            .map_err(AvengerChartError::DataFusionError)?;

        let domain = if group_names.is_empty() {
            key_domain
        } else {
            let group_domain = keyed
                .clone()
                .select(
                    group_names
                        .iter()
                        .zip(domain_group_names.iter())
                        .map(|(input, domain)| col(input).alias(domain))
                        .collect::<Vec<_>>(),
                )
                .map_err(AvengerChartError::DataFusionError)?
                .distinct()
                .map_err(AvengerChartError::DataFusionError)?;
            group_domain
                .join_on(key_domain, JoinType::Inner, [lit(true)])
                .map_err(AvengerChartError::DataFusionError)?
        };

        let domain = add_statistical_fill(
            domain,
            keyed.clone(),
            self,
            &group_names,
            &domain_group_names,
            ctx,
        )?;

        let joined = domain
            .join_on(
                keyed,
                JoinType::Left,
                join_predicates(&group_names, &domain_group_names),
            )
            .map_err(AvengerChartError::DataFusionError)?;

        let projection = output_projection(
            &original_names,
            self,
            &group_names,
            &domain_group_names,
            ctx,
        )?;
        let result = joined
            .select(projection)
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(result))
    }
}

fn add_hidden_columns(
    mut dataframe: DataFrame,
    spec: &CompiledImputeTransform,
    group_names: &[String],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    dataframe = dataframe
        .with_column(INPUT_KEY, spec.key.to_default_expr(ctx.session_context)?)
        .map_err(AvengerChartError::DataFusionError)?;
    dataframe = dataframe
        .with_column(
            INPUT_VALUE,
            spec.field.to_default_expr(ctx.session_context)?,
        )
        .map_err(AvengerChartError::DataFusionError)?;
    dataframe = dataframe
        .with_column(PRESENT, lit(true))
        .map_err(AvengerChartError::DataFusionError)?;
    for (group, name) in spec.group_by.iter().zip(group_names.iter()) {
        dataframe = dataframe
            .with_column(name, group.to_default_expr(ctx.session_context)?)
            .map_err(AvengerChartError::DataFusionError)?;
    }
    Ok(dataframe)
}

fn add_statistical_fill(
    domain: DataFrame,
    keyed: DataFrame,
    spec: &CompiledImputeTransform,
    group_names: &[String],
    domain_group_names: &[String],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<DataFrame, AvengerChartError> {
    let op = match &spec.method {
        ImputeMethodSpec::Value { .. } => return Ok(domain),
        ImputeMethodSpec::Mean => AggregateOp::Mean,
        ImputeMethodSpec::Min => AggregateOp::Min,
        ImputeMethodSpec::Max => AggregateOp::Max,
    };
    let aggregate = keyed
        .aggregate(
            group_names
                .iter()
                .map(|name| col(name).alias(format!("{name}_fill")))
                .collect::<Vec<_>>(),
            vec![aggregate_expr(
                &AggregateMeasureSpec {
                    name: FILL_VALUE.to_string(),
                    op,
                    expr: Some(expr_node(col(INPUT_VALUE), "impute aggregate value")),
                },
                ctx.session_context,
            )?],
        )
        .map_err(AvengerChartError::DataFusionError)?;

    if group_names.is_empty() {
        domain
            .join_on(aggregate, JoinType::Left, [lit(true)])
            .map_err(AvengerChartError::DataFusionError)
    } else {
        let predicates = domain_group_names
            .iter()
            .zip(group_names.iter())
            .map(|(domain, input)| {
                binary_expr(
                    col(domain),
                    Operator::IsNotDistinctFrom,
                    col(format!("{input}_fill")),
                )
            })
            .collect::<Vec<_>>();
        domain
            .join_on(aggregate, JoinType::Left, predicates)
            .map_err(AvengerChartError::DataFusionError)
    }
}

fn output_projection(
    original_names: &[String],
    spec: &CompiledImputeTransform,
    group_names: &[String],
    domain_group_names: &[String],
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Vec<Expr>, AvengerChartError> {
    let mut projection = Vec::new();
    let key_source = simple_column_name(&spec.key.to_default_expr(ctx.session_context)?);
    let group_sources = spec
        .group_by
        .iter()
        .map(|expr| {
            Ok(simple_column_name(
                &expr.to_default_expr(ctx.session_context)?,
            ))
        })
        .collect::<Result<Vec<_>, AvengerChartError>>()?;

    for name in original_names {
        if name == &spec.value_name {
            continue;
        }
        if key_source.as_deref() == Some(name.as_str()) {
            projection.push(coalesce_expr(col(name), col(DOMAIN_KEY)).alias(name));
            continue;
        }
        if let Some(index) = group_sources
            .iter()
            .position(|source| source.as_deref() == Some(name.as_str()))
        {
            projection.push(coalesce_expr(col(name), col(&domain_group_names[index])).alias(name));
            continue;
        }
        projection.push(col(name).alias(name));
    }

    projection.push(coalesce_expr(col(INPUT_VALUE), fill_expr(spec, ctx)?).alias(&spec.value_name));
    if let Some(flag_name) = &spec.flag_name {
        projection.push(col(PRESENT).is_null().alias(flag_name));
    }

    let _ = group_names;
    Ok(projection)
}

fn fill_expr(
    spec: &CompiledImputeTransform,
    ctx: &DataTransformExecutionContext<'_>,
) -> Result<Expr, AvengerChartError> {
    match &spec.method {
        ImputeMethodSpec::Value { expr } => Ok(expr.to_default_expr(ctx.session_context)?),
        ImputeMethodSpec::Mean | ImputeMethodSpec::Min | ImputeMethodSpec::Max => {
            Ok(col(FILL_VALUE))
        }
    }
}

fn coalesce_expr(value: Expr, fallback: Expr) -> Expr {
    when(value.clone().is_null(), fallback)
        .otherwise(value)
        .expect("valid impute coalesce expression")
}

fn join_predicates(group_names: &[String], domain_group_names: &[String]) -> Vec<Expr> {
    let mut predicates = vec![binary_expr(
        col(DOMAIN_KEY),
        Operator::IsNotDistinctFrom,
        col(INPUT_KEY),
    )];
    predicates.extend(
        domain_group_names
            .iter()
            .zip(group_names.iter())
            .map(|(domain, input)| {
                binary_expr(col(domain), Operator::IsNotDistinctFrom, col(input))
            }),
    );
    predicates
}

fn group_hidden_names(len: usize) -> Vec<String> {
    (0..len)
        .map(|index| format!("__avenger_impute_input_group_{index}"))
        .collect()
}

fn group_domain_names(len: usize) -> Vec<String> {
    (0..len)
        .map(|index| format!("__avenger_impute_domain_group_{index}"))
        .collect()
}
