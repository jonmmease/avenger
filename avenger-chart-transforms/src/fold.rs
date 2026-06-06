use crate::common::{expr_node, validate_generated_name, validate_unique_generated_names};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, DefaultLogicalExprNodeExt, IntoExpr,
    SerializableExpr,
};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::{Expr, LogicalPlanBuilder, col, lit},
};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledFoldTransform {
    pub fields: Vec<FoldFieldSpec>,
    pub key_name: String,
    pub value_name: String,
    pub index_name: Option<String>,
}

#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FoldFieldSpec {
    pub key: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub value: LogicalExprNode,
}

#[derive(Clone, Debug)]
pub struct Fold {
    fields: Vec<(String, Expr)>,
    key_name: String,
    value_name: String,
    index_name: Option<String>,
}

impl Default for Fold {
    fn default() -> Self {
        Self {
            fields: Vec::new(),
            key_name: "key".to_string(),
            value_name: "value".to_string(),
            index_name: None,
        }
    }
}

impl Fold {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn field(mut self, key: impl Into<String>, value: impl IntoExpr) -> Self {
        self.fields.push((key.into(), value.into_expr()));
        self
    }

    pub fn as_key(mut self, name: impl Into<String>) -> Self {
        self.key_name = name.into();
        self
    }

    pub fn as_value(mut self, name: impl Into<String>) -> Self {
        self.value_name = name.into();
        self
    }

    pub fn index(mut self, name: impl Into<String>) -> Self {
        self.index_name = Some(name.into());
        self
    }
}

impl DataTransform for Fold {
    type Output = FoldOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        validate_fold_output_names(&self.key_name, &self.value_name, self.index_name.as_deref())?;
        if self.fields.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Fold transform requires at least one field".to_string(),
            ));
        }
        for (key, _) in &self.fields {
            if key.is_empty() {
                return Err(AvengerChartError::InvalidArgument(
                    "Fold field key must not be empty".to_string(),
                ));
            }
        }

        let fields = self
            .fields
            .into_iter()
            .map(|(key, value)| FoldFieldSpec {
                key,
                value: expr_node(value, "fold value expression"),
            })
            .collect();
        let output = FoldOutput {
            key_name: self.key_name.clone(),
            value_name: self.value_name.clone(),
            index_name: self.index_name.clone(),
        };
        Ok((
            Box::new(CompiledFoldTransform {
                fields,
                key_name: self.key_name,
                value_name: self.value_name,
                index_name: self.index_name,
            }),
            output,
        ))
    }
}

#[derive(Clone, Debug)]
pub struct FoldOutput {
    key_name: String,
    value_name: String,
    index_name: Option<String>,
}

impl FoldOutput {
    pub fn key(&self) -> Expr {
        col(&self.key_name)
    }

    pub fn value(&self) -> Expr {
        col(&self.value_name)
    }

    pub fn index(&self) -> Expr {
        let Some(index_name) = &self.index_name else {
            panic!("Fold index output was requested, but no index column was configured");
        };
        col(index_name)
    }
}

#[typetag::serde(name = "fold")]
#[async_trait]
impl CompiledDataTransform for CompiledFoldTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        validate_fold_output_names(&self.key_name, &self.value_name, self.index_name.as_deref())?;
        if self.fields.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Fold transform requires at least one field".to_string(),
            ));
        }

        let output_names =
            fold_output_names(&self.key_name, &self.value_name, self.index_name.as_deref());
        let preserved_names = dataframe
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().to_string())
            .filter(|name| !output_names.iter().any(|output| output == name))
            .collect::<Vec<_>>();

        let mut branch_plans = Vec::with_capacity(self.fields.len());
        for (index, field) in self.fields.iter().enumerate() {
            let mut exprs = preserved_names
                .iter()
                .map(|name| col(name).alias(name))
                .collect::<Vec<_>>();
            exprs.push(lit(field.key.clone()).alias(&self.key_name));
            exprs.push(
                field
                    .value
                    .to_default_expr(ctx.session_context)?
                    .alias(&self.value_name),
            );
            if let Some(index_name) = &self.index_name {
                exprs.push(lit(index as i64).alias(index_name));
            }
            let branch = dataframe
                .clone()
                .select(exprs)
                .map_err(AvengerChartError::DataFusionError)?;
            branch_plans.push(branch.logical_plan().clone());
        }

        let mut iter = branch_plans.into_iter();
        let Some(first_plan) = iter.next() else {
            return Err(AvengerChartError::InvalidArgument(
                "Fold transform requires at least one field".to_string(),
            ));
        };
        let mut builder = LogicalPlanBuilder::from(first_plan);
        for plan in iter {
            builder = builder
                .union(plan)
                .map_err(AvengerChartError::DataFusionError)?;
        }
        let plan = builder
            .build()
            .map_err(AvengerChartError::DataFusionError)?;
        Ok(DataTransformResult::dataframe(DataFrame::new(
            ctx.session_context.state(),
            plan,
        )))
    }
}

fn fold_output_names<'a>(
    key_name: &'a str,
    value_name: &'a str,
    index_name: Option<&'a str>,
) -> Vec<&'a str> {
    let mut names = vec![key_name, value_name];
    if let Some(index_name) = index_name {
        names.push(index_name);
    }
    names
}

fn validate_fold_output_names(
    key_name: &str,
    value_name: &str,
    index_name: Option<&str>,
) -> Result<(), AvengerChartError> {
    for name in fold_output_names(key_name, value_name, index_name) {
        validate_generated_name(name)?;
    }
    validate_unique_generated_names(fold_output_names(key_name, value_name, index_name))
}
