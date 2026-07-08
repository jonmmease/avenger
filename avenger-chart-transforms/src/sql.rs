use async_trait::async_trait;
use avenger_chart_core::{
    AvengerChartError, CompiledDataTransform, DataTransform, DataTransformCompileContext,
    DataTransformExecutionContext, DataTransformResult, ExecutionShape,
};
use datafusion::{
    dataframe::DataFrame,
    datasource::provider_as_source,
    execution::session_state::SessionState,
    logical_expr::{
        AggregateUDF, Expr, HigherOrderUDF, LogicalPlan, ScalarUDF, SubqueryAlias, TableSource,
        WindowUDF, builder::LogicalTableSource,
    },
    sql::{
        parser::{DFParser, DFParserBuilder, Statement as DFStatement},
        planner::{ContextProvider, SqlToRel},
        sqlparser::{
            ast::{Query, Select, SetExpr, Statement as SqlStatement},
            dialect::{
                AnsiDialect, BigQueryDialect, ClickHouseDialect, DatabricksDialect, DuckDbDialect,
                GenericDialect, HiveDialect, MsSqlDialect, MySqlDialect, PostgreSqlDialect,
                RedshiftSqlDialect, SQLiteDialect, SnowflakeDialect,
            },
        },
    },
};
use datafusion_common::{
    DataFusionError, ResolvedTableReference, TableReference,
    config::{ConfigOptions, Dialect as DataFusionDialect},
    tree_node::{Transformed, TreeNode},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

const INPUT_TABLE: &str = "input";

#[derive(Clone, Debug)]
pub struct Sql {
    query: String,
}

impl Sql {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SqlOutput;

impl SqlOutput {
    /// Column reference into the stage's output schema.
    pub fn field(&self, name: &str) -> Expr {
        datafusion::prelude::col(name)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompiledSqlTransform {
    pub query: String,
}

impl DataTransform for Sql {
    type Output = SqlOutput;

    fn into_compiled_and_output(
        self,
        _ctx: DataTransformCompileContext,
    ) -> Result<(Box<dyn CompiledDataTransform>, Self::Output), AvengerChartError> {
        parse_single_query(&self.query)?;
        Ok((
            Box::new(CompiledSqlTransform { query: self.query }),
            SqlOutput,
        ))
    }
}

#[typetag::serde(name = "sql")]
#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
impl CompiledDataTransform for CompiledSqlTransform {
    fn clone_box(&self) -> Box<dyn CompiledDataTransform> {
        Box::new(self.clone())
    }

    fn execution_shape(&self) -> ExecutionShape {
        ExecutionShape::PlanRewrite
    }

    async fn apply(
        &self,
        dataframe: DataFrame,
        ctx: &DataTransformExecutionContext<'_>,
    ) -> Result<DataTransformResult, AvengerChartError> {
        let state = ctx.session_context.state();
        let statement = parse_single_query_for_state(&self.query, &state)?;
        let table_refs = state.resolve_table_references(&statement)?;

        let mut tables = HashMap::with_capacity(table_refs.len());
        let input_ref = resolved_table_ref(TableReference::bare(INPUT_TABLE), &state);
        let input_schema = Arc::new(dataframe.schema().as_arrow().clone());

        for table_ref in table_refs {
            let resolved = resolved_table_ref(table_ref.clone(), &state);
            if resolved.table.as_ref() == INPUT_TABLE
                && table_ref.schema().is_none()
                && table_ref.catalog().is_none()
            {
                tables.insert(
                    input_ref.clone(),
                    Arc::new(LogicalTableSource::new(Arc::clone(&input_schema)))
                        as Arc<dyn TableSource>,
                );
                continue;
            }

            let provider = ctx
                .session_context
                .table_provider(TableReference::from(resolved.clone()))
                .await
                .map_err(|err| {
                    AvengerChartError::DataFusionError(DataFusionError::Plan(format!(
                        "Unknown table '{table_ref}' in sql transform: {err}"
                    )))
                })?;
            tables.insert(resolved, provider_as_source(provider));
        }

        tables.entry(input_ref.clone()).or_insert_with(|| {
            Arc::new(LogicalTableSource::new(Arc::clone(&input_schema))) as Arc<dyn TableSource>
        });

        let provider = SqlStageContextProvider {
            state: &state,
            tables,
        };
        let plan = SqlToRel::new(&provider).statement_to_plan(statement)?;
        let upstream_plan = dataframe.logical_plan().clone();
        let spliced_plan = splice_input_table(plan, upstream_plan, &state)?;

        Ok(DataTransformResult::dataframe(DataFrame::new(
            state,
            spliced_plan,
        )))
    }
}

struct SqlStageContextProvider<'a> {
    state: &'a SessionState,
    tables: HashMap<ResolvedTableReference, Arc<dyn TableSource>>,
}

impl ContextProvider for SqlStageContextProvider<'_> {
    fn get_table_source(
        &self,
        name: TableReference,
    ) -> datafusion_common::Result<Arc<dyn TableSource>> {
        let resolved = resolved_table_ref(name.clone(), self.state);
        self.tables.get(&resolved).cloned().ok_or_else(|| {
            DataFusionError::Plan(format!("Unknown table '{name}' in sql transform"))
        })
    }

    fn get_function_meta(&self, name: &str) -> Option<Arc<ScalarUDF>> {
        self.state.scalar_functions().get(name).cloned()
    }

    fn get_higher_order_meta(&self, name: &str) -> Option<Arc<HigherOrderUDF>> {
        self.state.higher_order_functions().get(name).cloned()
    }

    fn get_aggregate_meta(&self, name: &str) -> Option<Arc<AggregateUDF>> {
        self.state.aggregate_functions().get(name).cloned()
    }

    fn get_window_meta(&self, name: &str) -> Option<Arc<WindowUDF>> {
        self.state.window_functions().get(name).cloned()
    }

    fn get_variable_type(
        &self,
        _variable_names: &[String],
    ) -> Option<datafusion::arrow::datatypes::DataType> {
        None
    }

    fn options(&self) -> &ConfigOptions {
        self.state.config_options().as_ref()
    }

    fn udf_names(&self) -> Vec<String> {
        self.state.scalar_functions().keys().cloned().collect()
    }

    fn higher_order_function_names(&self) -> Vec<String> {
        self.state
            .higher_order_functions()
            .keys()
            .cloned()
            .collect()
    }

    fn udaf_names(&self) -> Vec<String> {
        self.state.aggregate_functions().keys().cloned().collect()
    }

    fn udwf_names(&self) -> Vec<String> {
        self.state.window_functions().keys().cloned().collect()
    }
}

pub(crate) fn parse_single_query(query: &str) -> Result<DFStatement, AvengerChartError> {
    let mut statements = DFParser::parse_sql(query).map_err(AvengerChartError::DataFusionError)?;
    validate_statement_count(statements.len())?;
    let statement = statements.pop_front().expect("validated one statement");
    validate_query_statement(&statement)?;
    Ok(statement)
}

fn parse_single_query_for_state(
    query: &str,
    state: &SessionState,
) -> Result<DFStatement, AvengerChartError> {
    let dialect = dialect_from_config(state.config().options().sql_parser.dialect);
    let recursion_limit = state.config().options().sql_parser.recursion_limit;
    let mut statements = DFParserBuilder::new(query)
        .with_dialect(dialect.as_ref())
        .with_recursion_limit(recursion_limit)
        .build()
        .map_err(AvengerChartError::DataFusionError)?
        .parse_statements()
        .map_err(AvengerChartError::DataFusionError)?;
    validate_statement_count(statements.len())?;
    let statement = statements.pop_front().expect("validated one statement");
    validate_query_statement(&statement)?;
    Ok(statement)
}

fn validate_statement_count(count: usize) -> Result<(), AvengerChartError> {
    match count {
        1 => Ok(()),
        0 => Err(sql_policy_error("empty input")),
        _ => Err(sql_policy_error("multiple statements")),
    }
}

fn validate_query_statement(statement: &DFStatement) -> Result<(), AvengerChartError> {
    match statement {
        DFStatement::Statement(statement) => match statement.as_ref() {
            SqlStatement::Query(query) if query_is_select_or_values(query) => {
                if query_has_select_into(query) {
                    Err(sql_policy_error("SELECT INTO"))
                } else {
                    Ok(())
                }
            }
            SqlStatement::Query(query) => Err(sql_policy_error(query_body_name(&query.body))),
            statement => Err(sql_policy_error(sql_statement_name(statement))),
        },
        DFStatement::CreateExternalTable(_) => Err(sql_policy_error("CREATE EXTERNAL TABLE")),
        DFStatement::CopyTo(_) => Err(sql_policy_error("COPY")),
        DFStatement::Explain(_) => Err(sql_policy_error("EXPLAIN")),
        DFStatement::Reset(_) => Err(sql_policy_error("RESET")),
    }
}

fn sql_policy_error(found: impl AsRef<str>) -> AvengerChartError {
    AvengerChartError::InvalidArgument(format!(
        "sql transform accepts exactly one SELECT or VALUES statement, found: {}",
        found.as_ref()
    ))
}

fn query_is_select_or_values(query: &Query) -> bool {
    set_expr_is_select_or_values(&query.body)
}

fn set_expr_is_select_or_values(set_expr: &SetExpr) -> bool {
    match set_expr {
        SetExpr::Select(_) | SetExpr::Values(_) => true,
        SetExpr::Query(query) => query_is_select_or_values(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_is_select_or_values(left) && set_expr_is_select_or_values(right)
        }
        SetExpr::Insert(_)
        | SetExpr::Update(_)
        | SetExpr::Delete(_)
        | SetExpr::Merge(_)
        | SetExpr::Table(_) => false,
    }
}

fn query_has_select_into(query: &Query) -> bool {
    set_expr_has_select_into(&query.body)
}

fn set_expr_has_select_into(set_expr: &SetExpr) -> bool {
    match set_expr {
        SetExpr::Select(select) => select_has_into(select),
        SetExpr::Query(query) => query_has_select_into(query),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_has_select_into(left) || set_expr_has_select_into(right)
        }
        SetExpr::Values(_)
        | SetExpr::Insert(_)
        | SetExpr::Update(_)
        | SetExpr::Delete(_)
        | SetExpr::Merge(_)
        | SetExpr::Table(_) => false,
    }
}

fn select_has_into(select: &Select) -> bool {
    select.into.is_some()
}

fn query_body_name(set_expr: &SetExpr) -> &'static str {
    match set_expr {
        SetExpr::Select(_) => "SELECT",
        SetExpr::Values(_) => "VALUES",
        SetExpr::Query(query) => query_body_name(&query.body),
        SetExpr::SetOperation { .. } => "set operation",
        SetExpr::Insert(_) => "INSERT",
        SetExpr::Update(_) => "UPDATE",
        SetExpr::Delete(_) => "DELETE",
        SetExpr::Merge(_) => "MERGE",
        SetExpr::Table(_) => "TABLE",
    }
}

fn sql_statement_name(statement: &SqlStatement) -> &'static str {
    match statement {
        SqlStatement::Query(_) => "SELECT",
        SqlStatement::Insert(_) => "INSERT",
        SqlStatement::CreateTable(_) => "CREATE TABLE",
        SqlStatement::Explain { .. } => "EXPLAIN",
        SqlStatement::Set(_) => "SET",
        _ => "non-query statement",
    }
}

fn resolved_table_ref(table_ref: TableReference, state: &SessionState) -> ResolvedTableReference {
    let catalog = &state.config_options().catalog;
    table_ref.resolve(&catalog.default_catalog, &catalog.default_schema)
}

fn splice_input_table(
    plan: LogicalPlan,
    upstream_plan: LogicalPlan,
    state: &SessionState,
) -> Result<LogicalPlan, AvengerChartError> {
    let input_ref = resolved_table_ref(TableReference::bare(INPUT_TABLE), state);
    plan.transform(|node| {
        if let LogicalPlan::TableScan(scan) = &node {
            let resolved = resolved_table_ref(scan.table_name.clone(), state);
            if resolved == input_ref {
                debug_assert!(
                    scan.projection.is_none(),
                    "sql transform input scan unexpectedly had a projection"
                );
                let alias = SubqueryAlias::try_new(
                    Arc::new(upstream_plan.clone()),
                    TableReference::bare(INPUT_TABLE),
                )?;
                return Ok(Transformed::yes(LogicalPlan::SubqueryAlias(alias)));
            }
        }
        Ok(Transformed::no(node))
    })
    .map(|transformed| transformed.data)
    .map_err(AvengerChartError::DataFusionError)
}

fn dialect_from_config(
    dialect: DataFusionDialect,
) -> Box<dyn datafusion::sql::sqlparser::dialect::Dialect> {
    match dialect {
        DataFusionDialect::Generic => Box::new(GenericDialect {}),
        DataFusionDialect::MySQL => Box::new(MySqlDialect {}),
        DataFusionDialect::PostgreSQL => Box::new(PostgreSqlDialect {}),
        DataFusionDialect::Hive => Box::new(HiveDialect {}),
        DataFusionDialect::SQLite => Box::new(SQLiteDialect {}),
        DataFusionDialect::Snowflake => Box::new(SnowflakeDialect {}),
        DataFusionDialect::Redshift => Box::new(RedshiftSqlDialect {}),
        DataFusionDialect::MsSQL => Box::new(MsSqlDialect {}),
        DataFusionDialect::ClickHouse => Box::new(ClickHouseDialect {}),
        DataFusionDialect::BigQuery => Box::new(BigQueryDialect {}),
        DataFusionDialect::Ansi => Box::new(AnsiDialect {}),
        DataFusionDialect::DuckDB => Box::new(DuckDbDialect {}),
        DataFusionDialect::Databricks => Box::new(DatabricksDialect {}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::{
        CoordinationScope, DataTransformStage, TimeContext, apply_compiled_data_transforms,
    };
    use datafusion::{
        arrow::{
            array::{Float64Array, Int64Array, StringArray},
            datatypes::{DataType, Field, Schema},
            record_batch::RecordBatch,
        },
        common::ScalarValue,
        datasource::MemTable,
        prelude::{SessionContext, col},
    };
    use indexmap::IndexMap;

    fn input_batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("a", DataType::Int64, false),
                Field::new("b", DataType::Int64, false),
                Field::new("k", DataType::Int64, false),
                Field::new("category", DataType::Utf8, false),
                Field::new("value", DataType::Float64, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3, 4])) as _,
                Arc::new(Int64Array::from(vec![10, 20, 30, 40])) as _,
                Arc::new(Int64Array::from(vec![1, 2, 1, 2])) as _,
                Arc::new(StringArray::from(vec!["A", "A", "B", "B"])) as _,
                Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0, 4.0])) as _,
            ],
        )
        .unwrap()
    }

    fn execution_context<'a>(
        ctx: &'a SessionContext,
        params: &'a IndexMap<String, ScalarValue>,
    ) -> DataTransformExecutionContext<'a> {
        DataTransformExecutionContext {
            session_context: ctx,
            params,
            time_context: TimeContext::default(),
            facet_context: None,
        }
    }

    async fn collect_sql(query: &str) -> Result<Vec<RecordBatch>, AvengerChartError> {
        let ctx = SessionContext::new();
        let dataframe = ctx.read_batch(input_batch())?;
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: query.to_string(),
        };
        let result = transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await?;
        Ok(result.dataframe.collect().await?)
    }

    #[tokio::test]
    async fn select_star_from_input_is_identity() {
        let ctx = SessionContext::new();
        let dataframe = ctx.read_batch(input_batch()).unwrap();
        let expected = dataframe.clone().collect().await.unwrap();
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: "SELECT * FROM input".to_string(),
        };

        let result = transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await
            .unwrap();
        let actual = result.dataframe.collect().await.unwrap();

        assert_eq!(expected, actual);
    }

    #[tokio::test]
    async fn projection_computed_column_and_alias_execute() {
        let batches = collect_sql("SELECT a, a + b AS c FROM input ORDER BY a")
            .await
            .unwrap();
        assert_eq!(batches[0].num_columns(), 2);
        assert_eq!(
            batches[0].column(0).as_ref(),
            &Int64Array::from(vec![1, 2, 3, 4])
        );
        assert_eq!(
            batches[0].column(1).as_ref(),
            &Int64Array::from(vec![11, 22, 33, 44])
        );
    }

    #[tokio::test]
    async fn cte_and_window_function_execute() {
        let batches = collect_sql(
            "WITH ranked AS (
                SELECT category, value,
                       row_number() OVER (PARTITION BY category ORDER BY value) AS rn
                FROM input
             )
             SELECT category, rn FROM ranked ORDER BY category, rn",
        )
        .await
        .unwrap();

        assert_eq!(batches[0].num_rows(), 4);
    }

    #[tokio::test]
    async fn named_placeholder_survives_planning_and_binds_at_collect() {
        let ctx = SessionContext::new();
        let dataframe = ctx.read_batch(input_batch()).unwrap();
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: "SELECT a FROM input WHERE value >= $min ORDER BY a".to_string(),
        };

        let result = transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await
            .unwrap();
        assert!(
            result
                .dataframe
                .logical_plan()
                .display_indent()
                .to_string()
                .contains("$min")
        );
        let unbound_err = result
            .dataframe
            .clone()
            .collect()
            .await
            .expect_err("unbound placeholder should fail");
        assert!(unbound_err.to_string().contains("$min"), "{unbound_err}");

        let batches = result
            .dataframe
            .with_param_values(vec![("min", ScalarValue::Float64(Some(2.0)))])
            .unwrap()
            .collect()
            .await
            .unwrap();
        assert_eq!(
            batches[0].column(0).as_ref(),
            &Int64Array::from(vec![2, 3, 4])
        );
    }

    #[tokio::test]
    async fn registered_table_join_executes() {
        let ctx = SessionContext::new();
        let dims = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                Field::new("k", DataType::Int64, false),
                Field::new("label", DataType::Utf8, false),
            ])),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])) as _,
                Arc::new(StringArray::from(vec!["one", "two"])) as _,
            ],
        )
        .unwrap();
        ctx.register_table(
            "dims",
            Arc::new(MemTable::try_new(dims.schema(), vec![vec![dims]]).unwrap()),
        )
        .unwrap();
        let dataframe = ctx.read_batch(input_batch()).unwrap();
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: "SELECT input.a, dims.label FROM input JOIN dims ON input.k = dims.k ORDER BY input.a"
                .to_string(),
        };

        let result = transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await
            .unwrap();
        let batches = result.dataframe.collect().await.unwrap();

        assert_eq!(batches[0].num_rows(), 4);
        assert_eq!(
            batches[0].column(1).as_ref(),
            &StringArray::from(vec!["one", "two", "one", "two"])
        );
    }

    #[tokio::test]
    async fn input_shadows_registered_table() {
        let ctx = SessionContext::new();
        let registered = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("a", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from(vec![999])) as _],
        )
        .unwrap();
        ctx.register_table(
            "input",
            Arc::new(MemTable::try_new(registered.schema(), vec![vec![registered]]).unwrap()),
        )
        .unwrap();
        let dataframe = ctx.read_batch(input_batch()).unwrap();
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: "SELECT a FROM input ORDER BY a".to_string(),
        };

        let result = transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await
            .unwrap();
        let batches = result.dataframe.collect().await.unwrap();

        assert_eq!(batches[0].num_rows(), 4);
        assert_eq!(
            batches[0].column(0).as_ref(),
            &Int64Array::from(vec![1, 2, 3, 4])
        );
    }

    #[tokio::test]
    async fn unknown_table_error_mentions_table_name() {
        let ctx = SessionContext::new();
        let dataframe = ctx.read_batch(input_batch()).unwrap();
        let params = IndexMap::new();
        let transform = CompiledSqlTransform {
            query: "SELECT * FROM missing_dimension".to_string(),
        };

        let err = match transform
            .apply(dataframe, &execution_context(&ctx, &params))
            .await
        {
            Ok(_) => panic!("unknown table should fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains("missing_dimension"), "{err}");
    }

    #[tokio::test]
    async fn chained_sql_stages_use_previous_output() {
        let ctx = SessionContext::new();
        let transforms = vec![
            DataTransformStage::new(
                CoordinationScope::Free,
                Box::new(CompiledSqlTransform {
                    query: "SELECT a, a + b AS c FROM input".to_string(),
                }),
            ),
            DataTransformStage::new(
                CoordinationScope::Free,
                Box::new(CompiledSqlTransform {
                    query: "SELECT c FROM input WHERE c > 20 ORDER BY c".to_string(),
                }),
            ),
        ];
        let params = IndexMap::new();
        let result = apply_compiled_data_transforms(
            ctx.read_batch(input_batch()).unwrap(),
            &transforms,
            &execution_context(&ctx, &params),
        )
        .await
        .unwrap();
        let batches = result.dataframe.collect().await.unwrap();

        assert_eq!(
            batches[0].column(0).as_ref(),
            &Int64Array::from(vec![22, 33, 44])
        );
    }

    #[test]
    fn rejects_non_query_statements() {
        for (query, expected) in [
            ("CREATE TABLE t AS SELECT 1", "CREATE TABLE"),
            ("INSERT INTO t VALUES (1)", "INSERT"),
            ("SELECT 1; SELECT 2", "multiple statements"),
            ("EXPLAIN SELECT 1", "EXPLAIN"),
        ] {
            let err = parse_single_query(query).unwrap_err();
            let message = err.to_string();
            assert!(
                message.contains("sql transform accepts exactly one SELECT or VALUES statement")
            );
            assert!(message.contains(expected), "{message}");
        }
    }

    #[tokio::test]
    async fn aggregate_equivalence_spot_check() {
        use crate::Aggregate;
        use avenger_chart_core::{DataTransform, DataTransformCompileContext};

        let ctx = SessionContext::new();
        let params = IndexMap::new();
        let sql = DataTransformStage::new(
            CoordinationScope::Free,
            Box::new(CompiledSqlTransform {
                query: "SELECT category, sum(value) AS total FROM input GROUP BY category"
                    .to_string(),
            }),
        );
        let (aggregate, _) = Aggregate::new()
            .group_by([col("category")])
            .sum("total", col("value"))
            .into_compiled_and_output(DataTransformCompileContext::new(CoordinationScope::Free))
            .unwrap();
        let aggregate = DataTransformStage::new(CoordinationScope::Free, aggregate);

        let sql_batches = apply_compiled_data_transforms(
            ctx.read_batch(input_batch()).unwrap(),
            &[sql],
            &execution_context(&ctx, &params),
        )
        .await
        .unwrap()
        .dataframe
        .sort(vec![col("category").sort(true, true)])
        .unwrap()
        .collect()
        .await
        .unwrap();
        let aggregate_batches = apply_compiled_data_transforms(
            ctx.read_batch(input_batch()).unwrap(),
            &[aggregate],
            &execution_context(&ctx, &params),
        )
        .await
        .unwrap()
        .dataframe
        .sort(vec![col("category").sort(true, true)])
        .unwrap()
        .collect()
        .await
        .unwrap();

        assert_eq!(sql_batches, aggregate_batches);
    }

    #[test]
    fn serde_round_trips_typetag_box() {
        let transform: Box<dyn CompiledDataTransform> = Box::new(CompiledSqlTransform {
            query: "SELECT * FROM input".to_string(),
        });
        let json = serde_json::to_string(&transform).unwrap();
        let decoded: Box<dyn CompiledDataTransform> = serde_json::from_str(&json).unwrap();
        let decoded_json = serde_json::to_string(&decoded).unwrap();

        assert!(decoded_json.contains("\"type\":\"sql\""));
        assert!(decoded_json.contains("SELECT * FROM input"));
    }
}
