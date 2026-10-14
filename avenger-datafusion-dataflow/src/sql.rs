//! SQL diagnostics for individual computation boundaries, without expanding lineage.
use std::{collections::HashSet, sync::Arc};

use datafusion::{
    arrow::datatypes::Schema,
    common::{
        tree_node::{Transformed, TreeNode},
        Column, DFSchemaRef, DataFusionError, Result as DFResult, TableReference,
    },
    logical_expr::{Expr, LogicalPlan, LogicalPlanBuilder, LogicalTableSource, ScalarUDF},
    sql::unparser::Unparser,
};

use crate::{
    graph::{
        normalize::NullableField,
        reference::{GraphRead, ScalarRef, TableRef},
        GraphDef, NodeKind,
    },
    Dataflow, Error, PlanNode, Result, ScalarNode, ScalarOutput, TableOutput,
};

impl Dataflow {
    /// Format this definition's nodes or native plans and expressions as SQL.
    /// Formatting does not prepare queries, read sources, or change the dataflow.
    pub fn sql(&self) -> SqlFormatter<'_> {
        SqlFormatter::new(&self.inner)
    }
}

/// Pretty SQL for computations within one dataflow definition.
///
/// Graph dependencies remain named relations (`nodes.totals`, `inputs.sales`,
/// `assets.sales`, or `base.outputs.sales`). Scalar and expression references
/// become named display parameters, such as `$input__fraction` and
/// `$scalar__threshold`. Scoped names include the definition path.
///
/// This is diagnostic SQL, not an executable export. It does not inline data,
/// expand upstream lineage, bind parameter values, or define UDFs. Unsupported
/// DataFusion plans and extensions return errors through the native unparser.
#[derive(Debug)]
pub struct SqlFormatter<'a> {
    graph: &'a GraphDef,
}
impl<'a> SqlFormatter<'a> {
    pub(crate) fn new(graph: &'a GraphDef) -> Self {
        Self { graph }
    }

    /// Render a native logical plan, including references into this definition.
    /// Output is a pretty SELECT statement terminated by a semicolon.
    pub fn plan(&self, plan: &LogicalPlan) -> Result<String> {
        let plan = self.rewrite_plan(plan.clone(), &[])?;
        let plan = project_query(plan)?;
        let statement = Unparser::default().with_pretty(true).plan_to_sql(&plan)?;
        Ok(format!("{statement:#};"))
    }

    /// Render a native expression, including parameters and scalar subqueries.
    /// Row-column references need no schema. Output has no SELECT wrapper or semicolon.
    pub fn expr(&self, expr: &Expr) -> Result<String> {
        let expr = self.rewrite_expr(expr.clone(), &ColumnMap::default(), &[])?;
        let sql = Unparser::default().with_pretty(true).expr_to_sql(&expr)?;
        Ok(format!("{sql:#}"))
    }

    /// Render the stored definition of a table node, not its reference alone.
    pub fn table(&self, node: &PlanNode) -> Result<String> {
        self.check_graph(node.read.graph)?;
        let TableRef::Node(index) = node.read.source else {
            return Err(Error::InvalidReference(node.name().into()));
        };
        self.node_sql(index, NodeKind::Table)
    }

    /// Render the stored expression of a scalar node without its output alias.
    pub fn scalar(&self, node: &ScalarNode) -> Result<String> {
        self.check_graph(node.graph)?;
        self.node_sql(node.index, NodeKind::Scalar)
    }

    /// Render the table computation published by an output handle.
    pub fn table_output(&self, output: &TableOutput) -> Result<String> {
        self.check_graph(output.graph)?;
        self.node_sql(self.graph.outputs[output.index].node, NodeKind::Table)
    }

    /// Render the scalar expression published by an output handle.
    pub fn scalar_output(&self, output: &ScalarOutput) -> Result<String> {
        self.check_graph(output.graph)?;
        self.node_sql(self.graph.outputs[output.index].node, NodeKind::Scalar)
    }

    fn check_graph(&self, graph: u64) -> Result<()> {
        if graph != self.graph.id {
            Err(Error::ForeignHandle)
        } else {
            Ok(())
        }
    }

    fn node_sql(&self, index: usize, kind: NodeKind) -> Result<String> {
        let node = &self.graph.nodes[index];
        if node.kind != kind {
            return Err(Error::InvalidReference(node.name.to_string()));
        }
        if kind == NodeKind::Table {
            return self.plan(&node.plan);
        }
        let LogicalPlan::Projection(projection) = &node.plan else {
            return Err(Error::UnsupportedPlan(
                "scalar definition has no expression projection".into(),
            ));
        };
        let expr = &projection.expr[0];
        self.expr(match expr {
            Expr::Alias(a) => &a.expr,
            other => other,
        })
    }

    fn rewrite_plan(&self, plan: LogicalPlan, outer: &[ColumnMap]) -> DFResult<LogicalPlan> {
        if let LogicalPlan::Extension(e) = &plan {
            if let Some(read) = e.node.as_any().downcast_ref::<GraphRead>() {
                return self.read_plan(read).map_err(df_error);
            }
        }
        let old_schemas: Vec<_> = plan.inputs().iter().map(|p| p.schema().clone()).collect();
        let plan = plan
            .map_children(|child| self.rewrite_plan(child, outer).map(Transformed::yes))?
            .data;
        let new_schemas: Vec<_> = plan.inputs().iter().map(|p| p.schema().clone()).collect();
        let columns = ColumnMap::new(&old_schemas, &new_schemas);
        // Equijoin keys are resolved against their own side. Two named branches
        // can retain identical qualifiers from their shared upstream source.
        if let LogicalPlan::Join(mut join) = plan {
            let left = ColumnMap::new(&old_schemas[..1], &new_schemas[..1]);
            let right = ColumnMap::new(&old_schemas[1..], &new_schemas[1..]);
            join.on = join
                .on
                .into_iter()
                .map(|(l, r)| {
                    Ok((
                        self.rewrite_expr(l, &left, outer)?,
                        self.rewrite_expr(r, &right, outer)?,
                    ))
                })
                .collect::<DFResult<_>>()?;
            join.filter = join
                .filter
                .map(|expr| self.rewrite_expr(expr, &columns, outer))
                .transpose()?;
            return LogicalPlan::Join(join).recompute_schema();
        }
        plan.map_expressions(|expr| {
            self.rewrite_expr(expr, &columns, outer)
                .map(Transformed::yes)
        })?
        .data
        .recompute_schema()
    }

    fn rewrite_expr(&self, expr: Expr, columns: &ColumnMap, outer: &[ColumnMap]) -> DFResult<Expr> {
        expr.transform_up(|mut expr| {
            // Subquery plans are not children in Expr's ordinary tree traversal.
            let subquery = match &mut expr {
                Expr::ScalarSubquery(q) => Some(q),
                Expr::Exists(q) => Some(&mut q.subquery),
                Expr::InSubquery(q) => Some(&mut q.subquery),
                Expr::SetComparison(q) => Some(&mut q.subquery),
                _ => None,
            };
            if let Some(q) = subquery {
                let mut context = vec![columns.clone()];
                context.extend_from_slice(outer);
                q.subquery = Arc::new(project_query(
                    self.rewrite_plan(q.subquery.as_ref().clone(), &context)?,
                )?);
                q.outer_ref_columns = q
                    .outer_ref_columns
                    .iter()
                    .cloned()
                    .map(|e| self.rewrite_expr(e, &ColumnMap::default(), &context))
                    .collect::<DFResult<_>>()?;
            }
            let expr = match expr {
                Expr::Column(c) => Expr::Column(columns.replace(&c)?.unwrap_or(c)),
                Expr::OuterReferenceColumn(field, c) => {
                    let mut replacement = None;
                    for map in outer {
                        replacement = map.replace(&c)?;
                        if replacement.is_some() {
                            break;
                        }
                    }
                    Expr::OuterReferenceColumn(field, replacement.unwrap_or(c))
                }
                Expr::Placeholder(mut p) => {
                    if let Some((source, _)) = self.graph.placeholders.get(&p.id) {
                        p.id = self.parameter(*source);
                    } else if p.id.starts_with("$__avenger_") {
                        return Err(df_error(Error::UnknownPlaceholder(p.id)));
                    }
                    Expr::Placeholder(p)
                }
                // This identity function only widens field nullability for
                // DataFusion analysis. SQL scalar subqueries already permit null.
                Expr::ScalarFunction(f) if f.func.as_ref() == &ScalarUDF::from(NullableField) => {
                    f.args.into_iter().next().expect("nullable field argument")
                }
                other => other,
            };
            Ok(Transformed::yes(expr))
        })
        .map(|r| r.data)
    }

    fn read_plan(&self, read: &GraphRead) -> Result<LogicalPlan> {
        let (scope, kind, name) = if read.graph == self.graph.id {
            match read.source {
                TableRef::Input(i) => (
                    Some(self.graph.inputs[i].scope),
                    "inputs",
                    self.graph.inputs[i].name.to_string(),
                ),
                TableRef::Node(i) => (
                    Some(self.graph.nodes[i].scope),
                    "nodes",
                    self.graph.nodes[i].name.to_string(),
                ),
                TableRef::Rows(i) => (Some(i), "rows", "local".into()),
                TableRef::Asset(i) => {
                    let owner = self.graph.nodes.iter().find(|node| {
                        matches!(&node.plan,
                        LogicalPlan::Extension(e) if e.node.as_any().downcast_ref::<GraphRead>()
                            .is_some_and(|r| r.source == TableRef::Asset(i)))
                    });
                    let owner =
                        owner.ok_or_else(|| Error::InvalidReference(read.label.to_string()))?;
                    (Some(owner.scope), "assets", read.label.to_string())
                }
                TableRef::Import(i) => {
                    let base = self.graph.base.as_ref().ok_or(Error::BaseRequired)?;
                    (None, "outputs", base.inner.outputs[i].name.to_string())
                }
            }
        } else if let Some(base) = &self.graph.base {
            if read.graph != base.inner.id {
                return Err(Error::ForeignHandle);
            }
            let TableRef::Input(i) = read.source else {
                return Err(Error::ForeignHandle);
            };
            (None, "inputs", base.inner.inputs[i].name.to_string())
        } else {
            return Err(Error::ForeignHandle);
        };
        let table = match scope {
            Some(0) => TableReference::partial(kind, name),
            Some(scope) => TableReference::full(self.scope_path(scope), kind, name),
            None => TableReference::full("base", kind, name),
        };
        // A materialized join can retain two qualified columns named "id".
        // Give those columns distinct display names at this relation boundary.
        let schema = read.schema.as_ref();
        let mut used = HashSet::new();
        let fields = schema
            .iter()
            .enumerate()
            .map(|(i, (_, field))| {
                let mut name = field.name().clone();
                if schema.fields().iter().filter(|f| f.name() == &name).count() > 1 {
                    name = schema.columns()[i].flat_name();
                }
                while !used.insert(name.clone()) {
                    name.push('_');
                }
                field.as_ref().clone().with_name(name)
            })
            .collect::<Vec<_>>();
        let schema = Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()));
        Ok(
            LogicalPlanBuilder::scan(table, Arc::new(LogicalTableSource::new(schema)), None)?
                .build()?,
        )
    }

    fn scope_parts(&self, scope: usize) -> Vec<&str> {
        self.graph.scopes[scope]
            .handle
            .as_ref()
            .map(|h| h.path.iter().map(|p| p.name.as_ref()).collect())
            .unwrap_or_default()
    }
    fn scope_path(&self, scope: usize) -> String {
        self.scope_parts(scope)
            .into_iter()
            .map(|p| p.replace('~', "~0").replace('/', "~1"))
            .collect::<Vec<_>>()
            .join("/")
    }
    fn parameter(&self, source: ScalarRef) -> String {
        let (kind, mut parts, name) = match source {
            ScalarRef::Input(i) => (
                "input",
                self.scope_parts(self.graph.inputs[i].scope),
                self.graph.inputs[i].name.as_ref(),
            ),
            ScalarRef::Node(i) => (
                "scalar",
                self.scope_parts(self.graph.nodes[i].scope),
                self.graph.nodes[i].name.as_ref(),
            ),
            ScalarRef::BaseInput(i) => (
                "base_input",
                vec![],
                self.graph
                    .base
                    .as_ref()
                    .expect("base reference")
                    .inner
                    .inputs[i]
                    .name
                    .as_ref(),
            ),
            ScalarRef::Import(i) => (
                "base_output",
                vec![],
                self.graph
                    .base
                    .as_ref()
                    .expect("base reference")
                    .inner
                    .outputs[i]
                    .name
                    .as_ref(),
            ),
        };
        parts.push(name);
        format!(
            "${kind}__{}",
            parts
                .into_iter()
                .map(parameter_part)
                .collect::<Vec<_>>()
                .join("__")
        )
    }
}

// An explicit SELECT list preserves aggregate column order and avoids an empty
// list when the unparser rewrites semi/anti joins. Keep the projection below
// outer sort/limit operators so those clauses stay in the same SELECT statement.
fn project_query(plan: LogicalPlan) -> DFResult<LogicalPlan> {
    match plan {
        LogicalPlan::Sort(_) | LogicalPlan::Limit(_) => plan
            .map_children(|child| project_query(child).map(Transformed::yes))
            .map(|r| r.data),
        LogicalPlan::Projection(_) => Ok(plan),
        _ => {
            let columns = plan.schema().columns().into_iter().map(Expr::Column);
            LogicalPlanBuilder::from(plan).project(columns)?.build()
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ColumnMap(Vec<(Column, Column)>);
impl ColumnMap {
    fn new(old: &[DFSchemaRef], new: &[DFSchemaRef]) -> Self {
        Self(
            old.iter()
                .zip(new)
                .flat_map(|(old, new)| old.columns().into_iter().zip(new.columns()))
                .collect(),
        )
    }
    fn replace(&self, column: &Column) -> DFResult<Option<Column>> {
        let mut candidates = self.0.iter().filter(|(old, _)| {
            old == column || (column.relation.is_none() && old.name == column.name)
        });
        let Some((_, first)) = candidates.next() else {
            return Ok(None);
        };
        if candidates.any(|(_, other)| other != first) {
            return datafusion::common::plan_err!("ambiguous column in SQL display: {column}");
        }
        Ok(Some(first.clone()))
    }
}

// Escaping underscores keeps component separators and escaped bytes unambiguous.
fn parameter_part(name: &str) -> String {
    name.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("_{b:02x}")
            }
        })
        .collect()
}
fn df_error(error: Error) -> DataFusionError {
    DataFusionError::External(Box::new(error))
}
