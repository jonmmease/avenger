use arrow_schema::DataType;
use avenger_runtime::{context::TaskEvaluationContext, variable::Variable};
use datafusion::{
    execution::{
        SessionState,
        context::{FunctionFactory, RegisterFunction},
    },
    logical_expr::{
        ColumnarValue, CreateFunction, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature,
        Volatility,
        simplify::{ExprSimplifyResult, SimplifyInfo},
        sort_properties::{ExprProperties, SortProperties},
    },
    prelude::Expr,
};
use datafusion_common::{
    DataFusionError, ScalarValue, exec_err, internal_err,
    tree_node::{Transformed, TreeNode},
};

#[tokio::test]
async fn run_sequence_query() {
    let context = TaskEvaluationContext::new();

    context
        .register_val(
            &Variable::new(vec!["start".to_string()]),
            ScalarValue::from(4.0),
        )
        .unwrap();

    context
        .register_val(
            &Variable::new(vec!["stop".to_string()]),
            ScalarValue::from(98.0),
        )
        .unwrap();

    context
        .register_val(
            &Variable::new(vec!["count".to_string()]),
            ScalarValue::from(50.0),
        )
        .unwrap();

    let ctx = context.session_ctx();

    // First we must configure the SessionContext with our function factory
    let ctx = ctx
        .clone()
        .with_function_factory(Arc::new(CustomFunctionFactory::default()));

    ctx.sql(
        r#"
        CREATE FUNCTION f1(BIGINT)
            RETURNS BIGINT
            RETURN $1 + 1
    "#,
    )
    .await
    .unwrap();
    ctx.sql(
        r#"
        CREATE FUNCTION f2(BIGINT, BIGINT)
            RETURNS BIGINT
            RETURN $1 + f1($2);
    "#,
    )
    .await
    .unwrap();

    let sql2 = r#"
        SELECT f2(1, 2)
    "#;

    let sql = r#"
        WITH RECURSIVE nice_iterations AS (
          -- Base case: iteration 0
          SELECT
            0 as iter,
            @start as start,
            @stop as stop,
            0.0 as prestep,
            -- Calculate initial tick_increment
            POWER(10.0, FLOOR(LOG(10, (@stop - @start) / GREATEST(@count, 1)))) *
            CASE
              WHEN ((@stop - @start) / GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10, (@stop - @start) / GREATEST(@count, 1)))) >= SQRT(50)
              THEN 10.0
              WHEN ((@stop - @start) / GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10, (@stop - @start) / GREATEST(@count, 1)))) >= SQRT(10)
              THEN 5.0
              WHEN ((@stop - @start) / GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10, (@stop - @start) / GREATEST(@count, 1)))) >= SQRT(2)
              THEN 2.0
              ELSE 1.0
            END as step

          UNION ALL

          -- Recursive case
          SELECT
            iter + 1,
            CASE
              WHEN step > 0 THEN FLOOR(start / step) * step
              WHEN step < 0 THEN CEIL(start * step) / step
              ELSE start
            END as new_start,
            CASE
              WHEN step > 0 THEN CEIL(stop / step) * step
              WHEN step < 0 THEN FLOOR(stop * step) / step
              ELSE stop
            END as new_stop,
            step as prestep,
            -- Recalculate tick_increment for new bounds
            POWER(10.0, FLOOR(LOG(10,
              (CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
               CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
              GREATEST(@count, 1)
            ))) *
            CASE
              WHEN ((CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                     CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                    GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10,
                     (CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                      CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                     GREATEST(@count, 1)
                   ))) >= SQRT(50)
              THEN 10.0
              WHEN ((CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                     CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                    GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10,
                     (CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                      CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                     GREATEST(@count, 1)
                   ))) >= SQRT(10)
              THEN 5.0
              WHEN ((CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                     CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                    GREATEST(@count, 1)) /
                   POWER(10.0, FLOOR(LOG(10,
                     (CASE WHEN step > 0 THEN CEIL(stop / step) * step WHEN step < 0 THEN FLOOR(stop * step) / step ELSE stop END -
                      CASE WHEN step > 0 THEN FLOOR(start / step) * step WHEN step < 0 THEN CEIL(start * step) / step ELSE start END) /
                     GREATEST(@count, 1)
                   ))) >= SQRT(2)
              THEN 2.0
              ELSE 1.0
            END as step
          FROM nice_iterations
          WHERE iter < 9 AND step != prestep
        )
        -- Select the final result - assuming @start was already the lesser value
        SELECT
          start as final_start,
          stop as final_stop
        FROM nice_iterations
        ORDER BY iter DESC
        LIMIT 1;
    "#;
    let df = ctx.sql(sql).await.unwrap();
    df.show().await.unwrap();

    // let schema = df.schema().inner().clone();
    // let partitions = df.collect().await.unwrap();
    // let table = ArrowTable::try_new(schema, partitions).unwrap();
    // table.show();
}

use std::{result::Result as RResult, sync::Arc};

/// This is our FunctionFactory that is responsible for converting `CREATE
/// FUNCTION` statements into function instances
#[derive(Debug, Default)]
struct CustomFunctionFactory {}

#[async_trait::async_trait]
impl FunctionFactory for CustomFunctionFactory {
    /// This function takes the parsed `CREATE FUNCTION` statement and returns
    /// the function instance.
    async fn create(
        &self,
        _state: &SessionState,
        statement: CreateFunction,
    ) -> Result<RegisterFunction, DataFusionError> {
        let f: ScalarFunctionWrapper = statement.try_into()?;

        Ok(RegisterFunction::Scalar(Arc::new(ScalarUDF::from(f))))
    }
}

/// this function represents the newly created execution engine.
#[derive(Debug)]
struct ScalarFunctionWrapper {
    /// The text of the function body, `$1 + f1($2)` in our example
    name: String,
    expr: Expr,
    signature: Signature,
    return_type: DataType,
}

impl ScalarUDFImpl for ScalarFunctionWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType, DataFusionError> {
        Ok(self.return_type.clone())
    }

    fn invoke_with_args(
        &self,
        _args: ScalarFunctionArgs,
    ) -> Result<ColumnarValue, DataFusionError> {
        // Since this function is always simplified to another expression, it
        // should never actually be invoked
        internal_err!("This function should not get invoked!")
    }

    /// The simplify function is called to simply a call such as `f2(2)`. This
    /// function parses the string and returns the resulting expression
    fn simplify(
        &self,
        args: Vec<Expr>,
        _info: &dyn SimplifyInfo,
    ) -> Result<ExprSimplifyResult, DataFusionError> {
        let replacement = Self::replacement(&self.expr, &args)?;

        Ok(ExprSimplifyResult::Simplified(replacement))
    }

    fn aliases(&self) -> &[String] {
        &[]
    }

    fn output_ordering(
        &self,
        _input: &[ExprProperties],
    ) -> Result<SortProperties, DataFusionError> {
        Ok(SortProperties::Unordered)
    }
}

impl ScalarFunctionWrapper {
    // replaces placeholders such as $1 with actual arguments (args[0]
    fn replacement(expr: &Expr, args: &[Expr]) -> Result<Expr, DataFusionError> {
        let result = expr.clone().transform(|e| {
            let r = match e {
                Expr::Placeholder(placeholder) => {
                    let placeholder_position = Self::parse_placeholder_identifier(&placeholder.id)?;
                    if placeholder_position < args.len() {
                        Transformed::yes(args[placeholder_position].clone())
                    } else {
                        exec_err!(
                            "Function argument {} not provided, argument missing!",
                            placeholder.id
                        )?
                    }
                }
                _ => Transformed::no(e),
            };

            Ok(r)
        })?;

        Ok(result.data)
    }
    // Finds placeholder identifier such as `$X` format where X >= 1
    fn parse_placeholder_identifier(placeholder: &str) -> Result<usize, DataFusionError> {
        if let Some(value) = placeholder.strip_prefix('$') {
            Ok(value.parse().map(|v: usize| v - 1).map_err(|e| {
                DataFusionError::Execution(format!(
                    "Placeholder `{}` parsing error: {}!",
                    placeholder, e
                ))
            })?)
        } else {
            exec_err!("Placeholder should start with `$`!")
        }
    }
}

/// This impl block creates a scalar function from
/// a parsed `CREATE FUNCTION` statement (`CreateFunction`)
impl TryFrom<CreateFunction> for ScalarFunctionWrapper {
    type Error = DataFusionError;

    fn try_from(definition: CreateFunction) -> RResult<Self, Self::Error> {
        Ok(Self {
            name: definition.name,
            expr: definition
                .params
                .function_body
                .expect("Expression has to be defined!"),
            return_type: definition
                .return_type
                .expect("Return type has to be defined!"),
            signature: Signature::exact(
                definition
                    .args
                    .unwrap_or_default()
                    .into_iter()
                    .map(|a| a.data_type)
                    .collect(),
                definition.params.behavior.unwrap_or(Volatility::Volatile),
            ),
        })
    }
}
