//! DataFusion physical-expression helpers for hot scalar evaluation paths.
//!
//! These utilities compile logical `Expr` values once against a fixed one-row
//! schema, then evaluate the resulting `PhysicalExpr`s against small
//! `RecordBatch` inputs. This complements `eval_to_scalars`, which remains the
//! broader logical-plan path for low-frequency scalar evaluation.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use datafusion::logical_expr::ColumnarValue;
use datafusion::{
    arrow::{
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    common::{
        Column, DFSchema, Result as DataFusionResult, ScalarValue,
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
    },
    error::DataFusionError,
    logical_expr::{Expr, cast, try_cast},
    physical_expr_common::physical_expr::PhysicalExpr,
    prelude::{SessionContext, col},
};

/// One logical expression to compile into a reusable physical expression.
#[derive(Clone, Debug)]
pub struct PhysicalScalarExpressionSpec {
    pub name: String,
    pub expr: Expr,
    pub expected_type: Option<DataType>,
    pub null_on_cast_failure: bool,
}

impl PhysicalScalarExpressionSpec {
    pub fn new(name: impl Into<String>, expr: Expr) -> Self {
        Self {
            name: name.into(),
            expr,
            expected_type: None,
            null_on_cast_failure: false,
        }
    }

    pub fn with_expected_type(mut self, data_type: DataType) -> Self {
        self.expected_type = Some(data_type);
        self
    }

    pub fn with_nullable_cast(mut self) -> Self {
        self.null_on_cast_failure = true;
        self
    }
}

/// Mapping from DataFusion placeholders to columns in the runtime batch.
#[derive(Clone, Debug)]
pub struct PlaceholderColumn {
    pub placeholder_id: String,
    pub column_name: String,
}

impl PlaceholderColumn {
    pub fn new(placeholder_id: impl Into<String>, column_name: impl Into<String>) -> Self {
        Self {
            placeholder_id: placeholder_id.into(),
            column_name: column_name.into(),
        }
    }
}

/// Options for compiling scalar physical expressions.
#[derive(Clone, Debug, Default)]
pub struct PhysicalScalarProgramOptions {
    pub allowed_columns: Option<HashSet<String>>,
    pub placeholder_columns: HashMap<String, String>,
}

impl PhysicalScalarProgramOptions {
    pub fn with_allowed_columns(mut self, allowed_columns: HashSet<String>) -> Self {
        self.allowed_columns = Some(allowed_columns);
        self
    }

    pub fn with_placeholder_columns(
        mut self,
        placeholder_columns: impl IntoIterator<Item = PlaceholderColumn>,
    ) -> Self {
        self.placeholder_columns = placeholder_columns
            .into_iter()
            .map(|mapping| (mapping.placeholder_id, mapping.column_name))
            .collect();
        self
    }
}

/// A compiled scalar expression.
#[derive(Debug)]
pub struct CompiledScalarExpression {
    pub name: String,
    physical: Arc<dyn PhysicalExpr>,
}

impl CompiledScalarExpression {
    pub fn evaluate(&self, batch: &RecordBatch) -> DataFusionResult<ScalarValue> {
        scalar_from_columnar_value(self.physical.evaluate(batch)?, batch.num_rows())
    }
}

/// A reusable physical-expression program for scalar outputs.
#[derive(Debug)]
pub struct CompiledScalarExpressionProgram {
    schema: Arc<Schema>,
    expressions: Vec<CompiledScalarExpression>,
}

impl CompiledScalarExpressionProgram {
    pub fn compile(
        ctx: &SessionContext,
        schema: Arc<Schema>,
        specs: Vec<PhysicalScalarExpressionSpec>,
        options: PhysicalScalarProgramOptions,
    ) -> DataFusionResult<Self> {
        let df_schema = DFSchema::try_from(schema.as_ref().clone())?;
        let mut expressions = Vec::with_capacity(specs.len());

        for spec in specs {
            let mut expr = validate_row_local_expr(spec.expr, options.allowed_columns.as_ref())?;
            expr = rewrite_placeholders(expr, &options.placeholder_columns)?;
            if let Some(expected_type) = spec.expected_type {
                expr = if spec.null_on_cast_failure {
                    try_cast(expr, expected_type)
                } else {
                    cast(expr, expected_type)
                };
            }
            let physical = ctx.create_physical_expr(expr, &df_schema)?;
            expressions.push(CompiledScalarExpression {
                name: spec.name,
                physical,
            });
        }

        Ok(Self {
            schema,
            expressions,
        })
    }

    pub fn schema(&self) -> &Arc<Schema> {
        &self.schema
    }

    pub fn expression_count(&self) -> usize {
        self.expressions.len()
    }

    pub fn evaluate(&self, batch: &RecordBatch) -> DataFusionResult<Vec<(String, ScalarValue)>> {
        self.expressions
            .iter()
            .map(|expr| {
                expr.evaluate(batch)
                    .map(|scalar| (expr.name.clone(), scalar))
            })
            .collect()
    }

    pub fn evaluate_values(&self, batch: &RecordBatch) -> DataFusionResult<Vec<ScalarValue>> {
        self.expressions
            .iter()
            .map(|expr| expr.evaluate(batch))
            .collect()
    }
}

pub fn scalar_from_columnar_value(
    value: ColumnarValue,
    input_rows: usize,
) -> DataFusionResult<ScalarValue> {
    match value {
        ColumnarValue::Scalar(scalar) => Ok(scalar),
        ColumnarValue::Array(array) => {
            if array.is_empty() || input_rows == 0 {
                return Err(DataFusionError::Internal(
                    "Physical scalar expression returned no rows".to_string(),
                ));
            }
            ScalarValue::try_from_array(array.as_ref(), 0)
        }
    }
}

pub fn one_row_batch_from_scalars(
    schema: Arc<Schema>,
    values: &HashMap<String, ScalarValue>,
) -> DataFusionResult<RecordBatch> {
    let arrays = schema
        .fields()
        .iter()
        .map(|field| {
            let scalar = values
                .get(field.name())
                .cloned()
                .unwrap_or_else(|| ScalarValue::try_new_null(field.data_type()).unwrap());
            scalar.to_array_of_size(1)
        })
        .collect::<DataFusionResult<Vec<_>>>()?;
    Ok(RecordBatch::try_new(schema, arrays)?)
}

pub fn collect_placeholder_ids(expr: &Expr) -> DataFusionResult<HashSet<String>> {
    let mut placeholders = HashSet::new();
    expr.apply(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate {
            placeholders.insert(placeholder.id.clone());
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(placeholders)
}

pub fn rewrite_placeholders(
    expr: Expr,
    placeholder_columns: &HashMap<String, String>,
) -> DataFusionResult<Expr> {
    expr.transform(|candidate| {
        if let Expr::Placeholder(placeholder) = candidate {
            let Some(column_name) = placeholder_columns.get(&placeholder.id) else {
                return Err(DataFusionError::Plan(format!(
                    "No column mapping was provided for placeholder '{}'",
                    placeholder.id
                )));
            };
            Ok(Transformed::yes(col(column_name)))
        } else {
            Ok(Transformed::no(candidate))
        }
    })
    .map(|transformed| transformed.data)
}

pub fn validate_row_local_expr(
    expr: Expr,
    allowed_columns: Option<&HashSet<String>>,
) -> DataFusionResult<Expr> {
    expr.apply(|candidate| {
        match candidate {
            Expr::AggregateFunction(_) => {
                return Err(DataFusionError::Plan(
                    "Aggregate expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            Expr::WindowFunction(_) => {
                return Err(DataFusionError::Plan(
                    "Window expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            Expr::Exists(_) | Expr::InSubquery(_) | Expr::ScalarSubquery(_) => {
                return Err(DataFusionError::Plan(
                    "Subquery expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            #[allow(deprecated)]
            Expr::Wildcard { .. } => {
                return Err(DataFusionError::Plan(
                    "Wildcard expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            Expr::GroupingSet(_) => {
                return Err(DataFusionError::Plan(
                    "Grouping-set expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            Expr::OuterReferenceColumn(_, _) => {
                return Err(DataFusionError::Plan(
                    "Outer-reference expressions are not supported in row-local physical scalar programs"
                        .to_string(),
                ));
            }
            Expr::Column(column) => validate_column(column, allowed_columns)?,
            _ => {}
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(expr)
}

fn validate_column(
    column: &Column,
    allowed_columns: Option<&HashSet<String>>,
) -> DataFusionResult<()> {
    if let Some(allowed_columns) = allowed_columns {
        if !allowed_columns.contains(&column.name) {
            return Err(DataFusionError::Plan(format!(
                "Unknown physical scalar expression column '{}'",
                column.name
            )));
        }
    }
    Ok(())
}

pub fn schema_from_fields(fields: impl IntoIterator<Item = Field>) -> Arc<Schema> {
    Arc::new(Schema::new(fields.into_iter().collect::<Vec<_>>()))
}

#[cfg(test)]
mod tests {
    use datafusion::{
        arrow::datatypes::{DataType, Field},
        functions_aggregate::min_max::max,
        logical_expr::Expr,
        prelude::{SessionContext, col, lit},
        scalar::ScalarValue,
    };

    use super::*;

    fn test_schema() -> Arc<Schema> {
        schema_from_fields([
            Field::new("x", DataType::Float64, true),
            Field::new("flag", DataType::Boolean, true),
            Field::new("__param_width", DataType::Float64, true),
        ])
    }

    fn test_batch(schema: Arc<Schema>, x: f64, flag: bool, width: f64) -> RecordBatch {
        one_row_batch_from_scalars(
            schema,
            &HashMap::from([
                ("x".to_string(), ScalarValue::Float64(Some(x))),
                ("flag".to_string(), ScalarValue::Boolean(Some(flag))),
                (
                    "__param_width".to_string(),
                    ScalarValue::Float64(Some(width)),
                ),
            ]),
        )
        .expect("test batch")
    }

    #[test]
    fn physical_program_evaluates_columns_and_literals() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let program = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema.clone(),
            vec![
                PhysicalScalarExpressionSpec::new("sum", col("x") + lit(2.0)),
                PhysicalScalarExpressionSpec::new("flag", col("flag")),
            ],
            PhysicalScalarProgramOptions::default(),
        )
        .expect("compile physical program");

        let values = program
            .evaluate_values(&test_batch(schema, 3.0, true, 10.0))
            .expect("evaluate physical program");
        assert_eq!(values[0], ScalarValue::Float64(Some(5.0)));
        assert_eq!(values[1], ScalarValue::Boolean(Some(true)));
    }

    #[test]
    fn physical_program_rewrites_placeholders_to_columns() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let program = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema.clone(),
            vec![PhysicalScalarExpressionSpec::new(
                "updated",
                Expr::Placeholder(datafusion::logical_expr::expr::Placeholder {
                    id: "$width".to_string(),
                    data_type: Some(DataType::Float64),
                }) + lit(5.0),
            )],
            PhysicalScalarProgramOptions::default()
                .with_placeholder_columns([PlaceholderColumn::new("$width", "__param_width")]),
        )
        .expect("compile physical program");

        let values = program
            .evaluate_values(&test_batch(schema.clone(), 3.0, true, 10.0))
            .expect("evaluate physical program");
        assert_eq!(values[0], ScalarValue::Float64(Some(15.0)));

        let values = program
            .evaluate_values(&test_batch(schema, 3.0, true, 20.0))
            .expect("evaluate physical program");
        assert_eq!(values[0], ScalarValue::Float64(Some(25.0)));
    }

    #[test]
    fn physical_program_rejects_unknown_columns() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let err = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema,
            vec![PhysicalScalarExpressionSpec::new("bad", col("missing"))],
            PhysicalScalarProgramOptions::default()
                .with_allowed_columns(HashSet::from(["x".to_string()])),
        )
        .expect_err("unknown column should fail");

        assert!(
            err.to_string()
                .contains("Unknown physical scalar expression column")
        );
    }

    #[test]
    fn physical_program_rejects_unmapped_placeholders() {
        let ctx = SessionContext::new();
        let schema = test_schema();
        let err = CompiledScalarExpressionProgram::compile(
            &ctx,
            schema,
            vec![PhysicalScalarExpressionSpec::new(
                "bad",
                Expr::Placeholder(datafusion::logical_expr::expr::Placeholder {
                    id: "$width".to_string(),
                    data_type: Some(DataType::Float64),
                }),
            )],
            PhysicalScalarProgramOptions::default(),
        )
        .expect_err("unmapped placeholder should fail");

        assert!(err.to_string().contains("No column mapping"));
    }

    #[test]
    fn physical_program_rejects_aggregates() {
        let err = validate_row_local_expr(max(col("x")), None).expect_err("aggregate rejected");
        assert!(err.to_string().contains("Aggregate expressions"));
    }

    #[test]
    fn one_row_batch_uses_typed_null_for_missing_values() {
        let schema = test_schema();
        let batch = one_row_batch_from_scalars(
            schema,
            &HashMap::from([("x".to_string(), ScalarValue::Float64(Some(1.0)))]),
        )
        .expect("one row batch");

        assert_eq!(batch.num_rows(), 1);
        let flag = ScalarValue::try_from_array(batch.column_by_name("flag").unwrap(), 0)
            .expect("flag scalar");
        assert_eq!(flag, ScalarValue::Boolean(None));
    }
}
