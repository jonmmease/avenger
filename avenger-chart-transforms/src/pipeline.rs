use crate::common::{expr_node, map_expr_node, validate_generated_name};
use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, CoordinationScope, DataTransform,
    DataTransformCompileContext, DataTransformExecutionContext, DataTransformResult,
    DataTransformStage, DefaultLogicalExprNodeExt, ExecutionShape, IntoExpr, SerializableExpr,
    apply_compiled_data_transforms,
};
use datafusion::{dataframe::DataFrame, logical_expr::Expr, prelude::col};
use datafusion_proto::protobuf::LogicalExprNode;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

/// One public relation column exported by a compiled pipeline.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineOutputSpec {
    pub name: String,
    #[serde_as(as = "FromInto<SerializableExpr>")]
    pub expr: LogicalExprNode,
}

/// A single compiled transform stage containing an ordered internal chain.
///
/// Child scopes are relative metadata inside the pipeline. The scope of the
/// outer [`DataTransformStage`] remains the only sharing boundary visible to
/// the chart runtime.
#[derive(Clone, Serialize, Deserialize)]
pub struct CompiledPipelineTransform {
    pub stages: Vec<DataTransformStage>,
    pub outputs: Vec<PipelineOutputSpec>,
}

impl std::fmt::Debug for CompiledPipelineTransform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompiledPipelineTransform")
            .field("stage_count", &self.stages.len())
            .field("outputs", &self.outputs)
            .finish()
    }
}

impl CompiledPipelineTransform {
    /// Canonical semantic bytes used by tests and cache adapters. Source-only
    /// stage aliases are deliberately absent from the compiled representation.
    pub fn semantic_identity(&self) -> Result<Vec<u8>, AvengerChartError> {
        serde_json::to_vec(self)
            .map_err(|err| AvengerChartError::SerializationError(err.to_string()))
    }
}

#[typetag::serde(name = "pipeline")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledPipelineTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn execution_shape(&self) -> ExecutionShape {
        if self
            .stages
            .iter()
            .all(|stage| stage.transform.execution_shape() == ExecutionShape::PlanRewrite)
        {
            ExecutionShape::PlanRewrite
        } else {
            ExecutionShape::PlanBreak
        }
    }

    fn referenced_session_tables(&self) -> Vec<String> {
        let mut tables = self
            .stages
            .iter()
            .flat_map(|stage| stage.transform.referenced_session_tables())
            .collect::<Vec<_>>();
        tables.sort();
        tables.dedup();
        tables
    }

    fn map_exprs(
        &self,
        f: &mut dyn FnMut(Expr) -> Result<Expr, AvengerChartError>,
    ) -> Result<Box<dyn CompiledDataTransform>, AvengerChartError> {
        Ok(Box::new(Self {
            stages: self
                .stages
                .iter()
                .map(|stage| stage.map_exprs(f))
                .collect::<Result<_, _>>()?,
            outputs: self
                .outputs
                .iter()
                .map(|output| {
                    Ok(PipelineOutputSpec {
                        name: output.name.clone(),
                        expr: map_expr_node(&output.expr, f)?,
                    })
                })
                .collect::<Result<_, AvengerChartError>>()?,
        }))
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let result = apply_compiled_data_transforms(dataframe, &self.stages, ctx).await?;
        let projection = self
            .outputs
            .iter()
            .map(|output| {
                validate_generated_name(&output.name)?;
                let expr = output.expr.to_default_expr(ctx.session_context)?;
                // DataFusion's select validation provides the authoritative
                // final-relation schema check, including nested expressions.
                Ok(expr.alias(&output.name))
            })
            .collect::<Result<Vec<_>, AvengerChartError>>()?;
        let dataframe = result.dataframe.select(projection).map_err(|err| {
            AvengerChartError::InvalidArgument(format!(
                "pipeline output expression is not valid for the final relation: {err}"
            ))
        })?;
        Ok(DataTransformResult {
            dataframe,
            derived_scalars: result.derived_scalars,
        })
    }
}

/// Authoring builder for one definition-like transform stage.
#[derive(Clone, Default)]
pub struct Pipeline {
    stages: Vec<DataTransformStage>,
    outputs: IndexMap<String, Expr>,
    // Diagnostics-only names are intentionally not lowered into the compiled
    // transform and therefore cannot perturb semantic/cache identity.
    stage_aliases: Vec<Option<String>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn transform<T, F>(self, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.transform_with_scope(CoordinationScope::Free, transform, f)
    }

    pub fn named_transform<T, F>(self, alias: impl Into<String>, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.named_transform_with_scope(alias, CoordinationScope::Free, transform, f)
    }

    pub fn transform_with_scope<T, F>(self, scope: CoordinationScope, transform: T, f: F) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.push_transform(None, scope, transform, f)
    }

    pub fn named_transform_with_scope<T, F>(
        self,
        alias: impl Into<String>,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        self.push_transform(Some(alias.into()), scope, transform, f)
    }

    fn push_transform<T, F>(
        mut self,
        alias: Option<String>,
        scope: CoordinationScope,
        transform: T,
        f: F,
    ) -> Self
    where
        T: DataTransform,
        F: FnOnce(Self, T::Output) -> Self,
    {
        let scope = scope.to_normalized();
        let (compiled, output) = transform
            .into_compiled_and_output(DataTransformCompileContext::new(scope))
            .expect("failed to compile pipeline child transform");
        self.stages.push(DataTransformStage::new(scope, compiled));
        self.stage_aliases.push(alias);
        f(self, output)
    }

    pub fn output(mut self, name: impl Into<String>, expr: impl IntoExpr) -> Self {
        self.outputs.insert(name.into(), expr.into_expr());
        self
    }
}

#[derive(Clone, Debug)]
pub struct PipelineOutput {
    names: Vec<String>,
}

impl PipelineOutput {
    pub fn field(&self, name: &str) -> Expr {
        if !self.names.iter().any(|candidate| candidate == name) {
            panic!("Unknown pipeline output '{name}'");
        }
        col(name)
    }
}

impl DataTransform for Pipeline {
    type Output = PipelineOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        if self.stages.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "pipeline must contain at least one child transform".to_string(),
            ));
        }
        if self.outputs.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "pipeline must declare at least one public output".to_string(),
            ));
        }
        for name in self.outputs.keys() {
            validate_generated_name(name)?;
        }
        let names = self.outputs.keys().cloned().collect::<Vec<_>>();
        let outputs = self
            .outputs
            .into_iter()
            .map(|(name, expr)| PipelineOutputSpec {
                name,
                expr: expr_node(expr, "pipeline output expression"),
            })
            .collect();
        Ok((
            Box::new(CompiledPipelineTransform {
                stages: self.stages,
                outputs,
            }),
            PipelineOutput { names },
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sql;
    use avenger_chart_core::{TimeContext, apply_compiled_data_transforms};
    use datafusion::{
        arrow::{
            array::{Float64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        prelude::SessionContext,
    };
    use std::sync::Arc;

    fn sample(ctx: &SessionContext) -> DataFrame {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("category", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["a", "a", "b"])) as _,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 4.0])) as _,
            ],
        )
        .unwrap();
        ctx.read_batch(batch).unwrap()
    }

    fn pipeline(first_alias: &str) -> Pipeline {
        Pipeline::new()
            .named_transform(
                first_alias,
                Sql::new("SELECT *, value * 2 AS doubled FROM input"),
                |pipeline, _| pipeline,
            )
            .named_transform(
                "summary",
                Sql::new(
                    "SELECT category, SUM(doubled) AS total FROM input GROUP BY category ORDER BY category",
                ),
                |pipeline, _| pipeline,
            )
            .output("category", col("category"))
            .output("total", col("total"))
    }

    fn execution_context(ctx: &SessionContext) -> DataTransformExecutionContext<'_> {
        DataTransformExecutionContext {
            session_context: ctx,
            params: Box::leak(Box::new(IndexMap::new())),
            time_context: TimeContext::default(),
            facet_context: None,
        }
    }

    #[tokio::test]
    async fn two_sql_children_match_manual_chain_and_hide_internal_columns() {
        let ctx = SessionContext::new();
        let (compiled, output) = pipeline("prepare")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Level(
                2,
            )))
            .unwrap();
        assert_eq!(output.field("total"), col("total"));
        let result = compiled
            .apply(sample(&ctx), &execution_context(&ctx))
            .await
            .unwrap();
        assert!(
            result
                .dataframe
                .schema()
                .field_with_name(None, "doubled")
                .is_err()
        );

        let (first, _) = Sql::new("SELECT *, value * 2 AS doubled FROM input")
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        let (second, _) = Sql::new(
            "SELECT category, SUM(doubled) AS total FROM input GROUP BY category ORDER BY category",
        )
        .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
        .unwrap();
        let manual = apply_compiled_data_transforms(
            sample(&ctx),
            &[
                DataTransformStage::new(CoordinationScope::Free, first),
                DataTransformStage::new(CoordinationScope::Free, second),
            ],
            &execution_context(&ctx),
        )
        .await
        .unwrap();
        assert_eq!(
            result.dataframe.collect().await.unwrap(),
            manual
                .dataframe
                .select(vec![col("category"), col("total")])
                .unwrap()
                .collect()
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn missing_declared_output_fails_against_final_relation() {
        let ctx = SessionContext::new();
        let (compiled, _) = Pipeline::new()
            .transform(Sql::new("SELECT category FROM input"), |pipeline, _| {
                pipeline
            })
            .output("missing", col("missing"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        let error = compiled
            .apply(sample(&ctx), &execution_context(&ctx))
            .await
            .err()
            .expect("missing final output must fail");
        assert!(error.to_string().contains("final relation"), "{error}");
    }

    #[test]
    fn typetag_scope_and_semantic_identity_are_canonical() {
        let compile = |alias| {
            let (compiled, _) = pipeline(alias)
                .into_compiled_and_output(DataTransformCompileContext::new(
                    CoordinationScope::Level(2),
                ))
                .unwrap();
            DataTransformStage::new(CoordinationScope::Level(2), compiled)
        };
        let left = compile("prepare");
        let right = compile("renamed_prepare");
        assert_eq!(
            serde_json::to_value(&left).unwrap(),
            serde_json::to_value(&right).unwrap()
        );
        assert_eq!(left.scope, CoordinationScope::Level(2));
        let json = serde_json::to_value(&left).unwrap();
        assert_eq!(json["transform"]["type"], "pipeline");
        assert_eq!(json["transform"]["stages"].as_array().unwrap().len(), 2);

        let bytes = bincode::serialize(&left).unwrap();
        let restored: DataTransformStage = bincode::deserialize(&bytes).unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), json);
    }

    #[test]
    fn referenced_tables_are_censused_through_one_parent_stage() {
        let (compiled, _) = Pipeline::new()
            .transform(Sql::new("SELECT * FROM input"), |pipeline, _| pipeline)
            .transform(Sql::new("SELECT * FROM analytics.movies"), |pipeline, _| {
                pipeline
            })
            .output("title", col("title"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        assert_eq!(
            compiled.referenced_session_tables(),
            vec!["analytics.movies".to_string()]
        );
    }
}
