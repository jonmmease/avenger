use super::{invalid, sources::SourceResolver, spec::*, values};
use crate::{
    Dataflow, DataflowBuilder, PlanNode, Result, Runtime, ScalarNode, SemanticConfig, TableSnapshot,
};
use datafusion::{
    arrow::datatypes::{DataType, Field, SchemaRef},
    catalog::TableProvider,
    common::{
        config::ConfigOptions,
        tree_node::{Transformed, TreeNode},
        DataFusionError, TableReference,
    },
    datasource::provider_as_source,
    execution::SessionState,
    logical_expr::{
        expr_rewriter::NamePreserver, planner::ContextProvider, AggregateUDF, Expr, ExprSchemable,
        HigherOrderUDF, LogicalPlan, LogicalPlanBuilder, ScalarUDF, TableSource, TableType,
        WindowUDF,
    },
    sql::{
        parser::{DFParser, Statement},
        planner::{PlannerContext, SqlToRel},
        sqlparser::{
            ast,
            dialect::GenericDialect,
            tokenizer::{Token, Tokenizer},
        },
    },
};
use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone)]
enum Source {
    Fixed(TableSnapshot),
    File(Arc<dyn TableProvider>),
}
#[derive(Clone, Default)]
struct Symbols {
    tables: HashMap<String, LogicalPlan>,
    scalars: HashMap<String, Expr>,
}
struct Body<'a> {
    inputs: &'a BTreeMap<String, InputSpec>,
    sources: &'a BTreeMap<String, SourceSpec>,
    tables: &'a BTreeMap<String, String>,
    scalars: &'a BTreeMap<String, String>,
    outputs: &'a OutputSpec,
    scopes: &'a BTreeMap<String, ScopeSpec>,
}
impl DataflowSpec {
    fn body(&self) -> Body<'_> {
        Body {
            inputs: &self.inputs,
            sources: &self.sources,
            tables: &self.tables,
            scalars: &self.scalars,
            outputs: &self.outputs,
            scopes: &self.scopes,
        }
    }
}
impl ScopeSpec {
    fn body(&self) -> Body<'_> {
        Body {
            inputs: &self.inputs,
            sources: &self.sources,
            tables: &self.tables,
            scalars: &self.scalars,
            outputs: &self.outputs,
            scopes: &self.scopes,
        }
    }
}

impl Runtime {
    /// Resolve source schemas and lower SQL once into a native dataflow definition.
    pub async fn load_spec(
        &self,
        spec: &DataflowSpec,
        resolver: &dyn SourceResolver,
    ) -> Result<Dataflow> {
        if spec.version != 1 || spec.dialect != "datafusion" {
            return Err(invalid("expected version 1 and dialect datafusion"));
        }
        let mut resolved = HashMap::new();
        let mut assets = HashMap::new();
        let mut pending = vec![(Vec::<String>::new(), spec.body())];
        while let Some((path, body)) = pending.pop() {
            for (name, source) in body.sources {
                let source = match source {
                    SourceSpec::Inline(s) => {
                        Source::Fixed(values::table(values::schema(&s.schema)?, &s.values)?)
                    }
                    SourceSpec::Asset(s) => {
                        if !assets.contains_key(&s.asset) {
                            assets.insert(s.asset.clone(), resolver.asset(&s.asset)?);
                        }
                        Source::Fixed(assets[&s.asset].clone())
                    }
                    SourceSpec::File(s) => {
                        let schema = match &s.schema {
                            Some(schema) => values::schema(schema)?,
                            None => resolver.infer_schema(s).await?,
                        };
                        Source::File(resolver.file_provider(s, schema)?)
                    }
                };
                resolved.insert((path.clone(), name.clone()), source);
            }
            for (name, scope) in body.scopes {
                let mut child = path.clone();
                child.push(name.clone());
                pending.push((child, scope.body()));
            }
        }
        let semantics = SemanticConfig {
            time_zone: self
                .inner
                .state
                .config_options()
                .execution
                .time_zone
                .clone()
                .unwrap_or_else(|| "UTC".into()),
            function_versions: self.inner.config.function_versions.clone(),
        };
        let mut builder = DataflowBuilder::with_semantics(semantics);
        load_body(
            &mut builder,
            &self.inner.state,
            spec.body(),
            &[],
            Symbols::default(),
            None,
            &resolved,
        )?;
        builder.finish()
    }
}

struct Query {
    statement: ast::Statement,
    parameters: Vec<String>,
}
fn parse(sql: &str, scalar: bool) -> Result<Query> {
    let text = if scalar {
        format!("SELECT {sql}")
    } else {
        sql.to_owned()
    };
    let mut statements = DFParser::parse_sql(&text)?;
    if statements.len() != 1 {
        return Err(invalid("SQL must contain one query"));
    }
    let Some(Statement::Statement(statement)) = statements.pop_front() else {
        return Err(invalid("only SQL queries are supported"));
    };
    if !matches!(statement.as_ref(), ast::Statement::Query(_)) {
        return Err(invalid("only SQL queries are supported"));
    }
    let tokens = Tokenizer::new(&GenericDialect {}, &text)
        .tokenize()
        .map_err(invalid)?;
    let mut parameters = Vec::new();
    for token in tokens {
        if let Token::Placeholder(p) = token {
            let name = p
                .strip_prefix('$')
                .ok_or_else(|| invalid("use named $parameters"))?
                .to_string();
            if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
                return Err(invalid("use named $parameters"));
            }
            if !parameters.contains(&name) {
                parameters.push(name);
            }
        }
    }
    Ok(Query {
        statement: *statement,
        parameters,
    })
}

#[derive(Debug)]
struct NamedSource(LogicalPlan);
impl TableSource for NamedSource {
    fn schema(&self) -> SchemaRef {
        Arc::new(self.0.schema().as_arrow().clone())
    }
    fn table_type(&self) -> TableType {
        TableType::Temporary
    }
}
struct Context<'a> {
    state: &'a SessionState,
    symbols: &'a Symbols,
    missing: RefCell<Vec<String>>,
}
impl ContextProvider for Context<'_> {
    fn get_table_source(
        &self,
        name: TableReference,
    ) -> datafusion::common::Result<Arc<dyn TableSource>> {
        if let TableReference::Bare { table } = &name {
            if let Some(plan) = self.symbols.tables.get(table.as_ref()) {
                return Ok(Arc::new(NamedSource(plan.clone())));
            }
        }
        self.missing.borrow_mut().push(name.to_string());
        Err(DataFusionError::Plan(format!(
            "undeclared or unresolved relation {name}"
        )))
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
    fn get_variable_type(&self, _: &[String]) -> Option<DataType> {
        None
    }
    fn options(&self) -> &ConfigOptions {
        self.state.config_options()
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
fn plan(query: &Query, symbols: &Symbols, state: &SessionState) -> Result<Option<LogicalPlan>> {
    let mut parameters = Vec::new();
    let mut fields = Vec::new();
    for name in &query.parameters {
        let Some(expr) = symbols.scalars.get(name) else {
            return Ok(None);
        };
        let field = expr.to_field(&datafusion::common::DFSchema::empty())?.1;
        fields.push(Some(Arc::new(Field::new(
            name,
            field.data_type().clone(),
            field.is_nullable(),
        ))));
        parameters.push(expr.clone());
    }
    let context = Context {
        state,
        symbols,
        missing: RefCell::default(),
    };
    let mut planner_context = PlannerContext::new().with_prepare_param_data_types(fields);
    let result = SqlToRel::new(&context)
        .sql_statement_to_plan_with_context(query.statement.clone(), &mut planner_context);
    if result.is_err() && !context.missing.borrow().is_empty() {
        return Ok(None);
    }
    let plan = result?;
    Ok(Some(lower(plan, &parameters)?))
}
fn lower(plan: LogicalPlan, parameters: &[Expr]) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up_with_subqueries(|plan| {
            if let LogicalPlan::TableScan(scan) = &plan {
                let source = (scan.source.as_ref() as &dyn Any)
                    .downcast_ref::<NamedSource>()
                    .ok_or_else(|| {
                        DataFusionError::Plan("SQL may only reference declared relations".into())
                    })?;
                let mut builder =
                    LogicalPlanBuilder::from(source.0.clone()).alias(scan.table_name.clone())?;
                for filter in &scan.filters {
                    builder = builder.filter(filter.clone())?;
                }
                if let Some(indices) = &scan.projection {
                    let columns = builder.schema().columns();
                    builder = builder
                        .project(indices.iter().map(|i| Expr::Column(columns[*i].clone())))?;
                }
                if let Some(limit) = scan.fetch {
                    builder = builder.limit(0, Some(limit))?;
                }
                return Ok(Transformed::yes(builder.build()?));
            }
            let names = NamePreserver::new(&plan);
            plan.map_expressions(|expr| {
                let name = names.save(&expr);
                expr.transform_up(|expr| {
                    if let Expr::Placeholder(p) = &expr {
                        let index =
                            p.id.strip_prefix('$')
                                .and_then(|p| p.parse::<usize>().ok())
                                .and_then(|i| i.checked_sub(1))
                                .ok_or_else(|| DataFusionError::Plan("invalid parameter".into()))?;
                        return Ok(Transformed::yes(
                            parameters
                                .get(index)
                                .ok_or_else(|| DataFusionError::Plan("unknown parameter".into()))?
                                .clone(),
                        ));
                    }
                    Ok(Transformed::no(expr))
                })
                .map(|r| r.update_data(|e| name.restore(e)))
            })?
            .map_data(LogicalPlan::recompute_schema)
        })?
        .data)
}
fn scalar_expr(plan: LogicalPlan) -> Result<Expr> {
    let LogicalPlan::Projection(p) = plan else {
        return Err(invalid("expected a standalone scalar expression"));
    };
    if p.expr.len() != 1 || !matches!(p.input.as_ref(), LogicalPlan::EmptyRelation(_)) {
        return Err(invalid("expected one standalone scalar expression"));
    }
    Ok(p.expr[0].clone())
}

fn load_body(
    builder: &mut DataflowBuilder,
    state: &SessionState,
    body: Body<'_>,
    path: &[String],
    mut symbols: Symbols,
    rows: Option<(&str, PlanNode)>,
    sources: &HashMap<(Vec<String>, String), Source>,
) -> Result<()> {
    let mut names = HashSet::new();
    for name in body
        .inputs
        .keys()
        .chain(body.sources.keys())
        .chain(body.tables.keys())
        .chain(body.scalars.keys())
        .map(String::as_str)
        .chain(rows.as_ref().map(|r| r.0))
    {
        if name.is_empty() || !names.insert(name) {
            return Err(invalid(format!(
                "scope {path:?}: duplicate or empty symbol {name}"
            )));
        }
        symbols.tables.remove(name);
        symbols.scalars.remove(name);
    }
    let mut local_tables = HashMap::<String, PlanNode>::new();
    let mut local_scalars = HashMap::<String, ScalarNode>::new();
    if let Some((name, rows)) = rows {
        symbols.tables.insert(name.into(), rows.plan_ref());
        local_tables.insert(name.into(), rows);
    }
    for (name, input) in body.inputs {
        match input {
            InputSpec::Expr { data_type } => {
                symbols.scalars.insert(
                    name.clone(),
                    builder.expr_input(name, data_type.arrow_type())?.expr_ref(),
                );
            }
            InputSpec::Scalar { data_type } => {
                symbols.scalars.insert(
                    name.clone(),
                    builder
                        .scalar_input(name, data_type.arrow_type())?
                        .expr_ref(),
                );
            }
            InputSpec::Table { schema } => {
                symbols.tables.insert(
                    name.clone(),
                    builder
                        .table_input(name, values::schema(schema)?)?
                        .plan_ref(),
                );
            }
        }
    }
    for name in body.sources.keys() {
        let node = match &sources[&(path.to_vec(), name.clone())] {
            Source::Fixed(t) => builder.table_snapshot(name, t.clone())?,
            Source::File(provider) => builder.add_plan(
                name,
                LogicalPlanBuilder::scan(
                    name.as_str(),
                    provider_as_source(provider.clone()),
                    None,
                )?
                .build()?,
            )?,
        };
        symbols.tables.insert(name.clone(), node.plan_ref());
        local_tables.insert(name.clone(), node);
    }
    let mut pending = body
        .tables
        .iter()
        .map(|(n, s)| (n, false, s))
        .chain(body.scalars.iter().map(|(n, s)| (n, true, s)))
        .map(|(n, is_scalar, sql)| {
            Ok((
                n,
                is_scalar,
                parse(sql, is_scalar).map_err(|e| invalid(format!("scope {path:?}, {n}: {e}")))?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    while !pending.is_empty() {
        let mut remaining = Vec::new();
        let mut progress = false;
        for (name, is_scalar, query) in pending {
            match plan(&query, &symbols, state)
                .map_err(|e| invalid(format!("scope {path:?}, {name}: {e}")))?
            {
                None => remaining.push((name, is_scalar, query)),
                Some(plan) => {
                    if is_scalar {
                        let node = builder.add_scalar(name, scalar_expr(plan)?)?;
                        symbols.scalars.insert(name.clone(), node.expr_ref());
                        local_scalars.insert(name.clone(), node);
                    } else {
                        let node = builder.add_plan(name, plan)?;
                        symbols.tables.insert(name.clone(), node.plan_ref());
                        local_tables.insert(name.clone(), node);
                    }
                    progress = true;
                }
            }
        }
        if !progress {
            return Err(invalid(format!(
                "scope {path:?}: cyclic or unresolved SQL dependencies in {:?}",
                remaining.iter().map(|p| p.0).collect::<Vec<_>>()
            )));
        }
        pending = remaining;
    }
    for (alias, name) in &body.outputs.tables {
        let node = match local_tables.get(name) {
            Some(node) => node.clone(),
            None => builder.add_plan(
                format!("$output_table_{alias}"),
                symbols
                    .tables
                    .get(name)
                    .ok_or_else(|| invalid(format!("unknown table output {name}")))?
                    .clone(),
            )?,
        };
        builder.table_output(alias, &node)?;
    }
    for (alias, name) in &body.outputs.scalars {
        let node = match local_scalars.get(name) {
            Some(node) => node.clone(),
            None => builder.add_scalar(
                format!("$output_scalar_{alias}"),
                symbols
                    .scalars
                    .get(name)
                    .ok_or_else(|| invalid(format!("unknown scalar output {name}")))?
                    .clone(),
            )?,
        };
        builder.scalar_output(alias, &node)?;
    }
    for (name, scope) in body.scopes {
        let input = symbols
            .tables
            .get(&scope.partition.source)
            .ok_or_else(|| {
                invalid(format!(
                    "unknown partition source {}",
                    scope.partition.source
                ))
            })?
            .clone();
        let mut keys = Vec::new();
        for sql in &scope.partition.keys {
            // Plan key expressions against a named source through the same SQL parser.
            // The relation is an identifier from the spec, quoted to preserve its spelling.
            let query = parse(
                &format!(
                    "SELECT {sql} FROM \"{}\"",
                    scope.partition.source.replace('"', "\"\"")
                ),
                false,
            )?;
            let key_plan = plan(&query, &symbols, state)?
                .ok_or_else(|| invalid("unresolved partition key"))?;
            let LogicalPlan::Projection(p) = key_plan else {
                return Err(invalid("invalid partition key"));
            };
            if p.expr.len() != 1 {
                return Err(invalid("each partition key must be one expression"));
            }
            // Strip the SQL-only relation qualifier; the core builder uses the producer schema.
            let key = p.expr[0]
                .clone()
                .transform_up(|e| {
                    if let Expr::Column(c) = e {
                        Ok(Transformed::yes(Expr::Column(
                            datafusion::common::Column::from_name(c.name),
                        )))
                    } else {
                        Ok(Transformed::no(e))
                    }
                })?
                .data;
            keys.push(key);
        }
        let mut child = path.to_vec();
        child.push(name.clone());
        builder.partition_by(name, input, keys, |s| {
            let rows = s.rows();
            load_body(
                s.builder,
                state,
                scope.body(),
                &child,
                symbols.clone(),
                Some((&scope.rows, rows)),
                sources,
            )
        })?;
    }
    Ok(())
}

/// Lower columns through temporary parameters so SQL planning cannot capture one site's aliases.
pub(crate) fn binding_expr(
    sql: &str,
    state: &SessionState,
    sites: &[crate::expr_input::ExprSite],
) -> Result<Expr> {
    use datafusion::common::{Column, DFSchema};
    use datafusion::sql::sqlparser::parser::Parser;
    use std::ops::ControlFlow;

    let dialect = GenericDialect {};
    let mut parser = Parser::new(&dialect).try_with_sql(sql).map_err(invalid)?;
    let mut sql = parser.parse_expr().map_err(invalid)?;
    parser.expect_token(&Token::EOF).map_err(invalid)?;
    let mut columns = Vec::new();
    let mut fields = Vec::new();
    let visited = ast::visit_expressions_mut(&mut sql, |expr| {
        let name = match expr {
            ast::Expr::Identifier(id) => Some(id.to_string()),
            ast::Expr::CompoundIdentifier(ids) => Some(
                ids.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            ast::Expr::Value(value) if matches!(value.value, ast::Value::Placeholder(_)) => {
                return ControlFlow::Break(invalid(
                    "expression bindings cannot reference parameters",
                ));
            }
            ast::Expr::Subquery(_) | ast::Expr::Exists { .. } | ast::Expr::InSubquery { .. } => {
                return ControlFlow::Break(invalid(
                    "expression bindings cannot contain subqueries",
                ));
            }
            _ => None,
        };
        if let Some(name) = name {
            if name.starts_with('@') {
                return ControlFlow::Break(invalid(
                    "expression bindings cannot reference session variables",
                ));
            }
            let column = Column::from_qualified_name_ignore_case(name);
            let field = if let Some(site) = sites.first() {
                match site.schema.qualified_field_from_column(&column) {
                    Ok((_, field)) => field.clone(),
                    Err(e) => return ControlFlow::Break(invalid(e)),
                }
            } else {
                Arc::new(Field::new("unresolved", DataType::Null, true))
            };
            columns.push(Expr::Column(column));
            fields.push(Some(field));
            *expr = ast::Expr::Value(ast::Value::Placeholder(format!("${}", columns.len())).into());
        }
        ControlFlow::Continue(())
    });
    if let ControlFlow::Break(error) = visited {
        return Err(error);
    }
    let symbols = Symbols::default();
    let context = Context {
        state,
        symbols: &symbols,
        missing: RefCell::default(),
    };
    let expr = SqlToRel::new(&context).sql_to_expr(
        sql,
        &DFSchema::empty(),
        &mut PlannerContext::new().with_prepare_param_data_types(fields),
    )?;
    Ok(expr
        .transform_up(|expr| {
            if let Expr::Placeholder(p) = &expr {
                let index =
                    p.id.strip_prefix('$')
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|n| n.checked_sub(1));
                let column = index.and_then(|i| columns.get(i)).ok_or_else(|| {
                    DataFusionError::Plan("invalid binding column parameter".into())
                })?;
                Ok(Transformed::yes(column.clone()))
            } else {
                Ok(Transformed::no(expr))
            }
        })?
        .data)
}
