use std::collections::BTreeSet;

use datafusion::{
    common::{
        tree_node::{TreeNode, TreeNodeRecursion},
        DFSchemaRef,
    },
    logical_expr::{Expr, LogicalPlan, Volatility},
};

use super::{
    reference::{GraphRead, ScalarRef, TableRef},
    GraphDef, InputKind,
};
use crate::{Error, Result, ReuseScope};

#[derive(Clone, Debug)]
pub(crate) struct Analysis {
    pub dependencies: BTreeSet<usize>,
    pub inputs: BTreeSet<usize>,
    pub base_inputs: BTreeSet<usize>,
    pub base_outputs: BTreeSet<usize>,
    pub has_source: bool,
    pub direct_volatility: Volatility,
    pub reuse_scope: ReuseScope,
}

pub(crate) fn validate_scalar(expr: &Expr) -> Result<()> {
    let mut invalid = false;
    expr.apply(|expr| {
        if matches!(
            expr,
            Expr::Column(_)
                | Expr::OuterReferenceColumn(..)
                | Expr::AggregateFunction(_)
                | Expr::WindowFunction(_)
        ) {
            invalid = true;
            return Ok(TreeNodeRecursion::Stop);
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    if invalid {
        return Err(Error::InvalidExpression(
            "columns, aggregates, and windows require a logical plan context".into(),
        ));
    }
    Ok(())
}

pub(crate) fn analyze(plan: &LogicalPlan, graph: &GraphDef, owner: usize) -> Result<Analysis> {
    let mut result = Analysis {
        dependencies: BTreeSet::new(),
        inputs: BTreeSet::new(),
        base_inputs: BTreeSet::new(),
        base_outputs: BTreeSet::new(),
        direct_volatility: Volatility::Immutable,
        has_source: false,
        reuse_scope: ReuseScope::Reusable,
    };
    visit_plan(plan, graph, &[], &mut result)?;
    for index in &result.inputs {
        graph.check_visible(
            graph.inputs[*index].scope,
            owner,
            &graph.inputs[*index].name,
        )?;
    }
    for index in &result.dependencies {
        graph.check_visible(graph.nodes[*index].scope, owner, &graph.nodes[*index].name)?;
    }
    if let Some(discovery) = graph.scopes[owner].discovery {
        if graph.nodes[discovery].analysis.reuse_scope == ReuseScope::EvaluationLocal {
            result.reuse_scope = ReuseScope::EvaluationLocal;
        }
    }
    for dependency in &result.dependencies {
        let upstream = &graph.nodes[*dependency].analysis;
        result.inputs.extend(&upstream.inputs);
        result.base_inputs.extend(&upstream.base_inputs);
        result.base_outputs.extend(&upstream.base_outputs);
        if upstream.reuse_scope == ReuseScope::EvaluationLocal {
            result.reuse_scope = ReuseScope::EvaluationLocal;
        }
    }
    if result.direct_volatility != Volatility::Immutable {
        result.reuse_scope = ReuseScope::EvaluationLocal;
    }
    Ok(result)
}

fn visit_plan(
    plan: &LogicalPlan,
    graph: &GraphDef,
    outer: &[DFSchemaRef],
    result: &mut Analysis,
) -> Result<()> {
    match plan {
        LogicalPlan::Extension(extension) => {
            let read = extension
                .node
                .as_any()
                .downcast_ref::<GraphRead>()
                .ok_or_else(|| {
                    Error::UnsupportedPlan(format!("custom extension {}", extension.node.name()))
                })?;
            if read.graph != graph.id {
                let base = graph.base.as_ref().ok_or(Error::ForeignHandle)?;
                if read.graph != base.inner.id {
                    return Err(Error::ForeignHandle);
                }
                let TableRef::Input(index) = read.source else {
                    return Err(Error::ForeignHandle);
                };
                let input = base_input(graph, index)?;
                if !matches!(&input.kind, InputKind::Table(schema) if schema == &read.schema) {
                    return Err(Error::SchemaMismatch(read.label.to_string()));
                }
                result.base_inputs.insert(index);
                return Ok(());
            }
            match read.source {
                TableRef::Import(index) => {
                    let output = &graph
                        .base
                        .as_ref()
                        .ok_or(Error::BaseRequired)?
                        .inner
                        .outputs[index];
                    if output.scope != 0
                        || output.kind != super::NodeKind::Table
                        || output.schema != read.schema
                    {
                        return Err(Error::InvalidReference(read.label.to_string()));
                    }
                    result.base_outputs.insert(index);
                }
                TableRef::Asset(index) => {
                    let asset = graph
                        .assets
                        .get(index)
                        .ok_or_else(|| Error::InvalidReference(read.label.to_string()))?;
                    if read.schema.as_arrow() != asset.schema().as_ref() {
                        return Err(Error::SchemaMismatch(read.label.to_string()));
                    }
                }
                TableRef::Input(index) => {
                    if !matches!(
                        graph.inputs.get(index).map(|input| &input.kind),
                        Some(InputKind::Table(schema)) if schema == &read.schema
                    ) {
                        return Err(Error::InvalidReference(read.label.to_string()));
                    }
                    result.inputs.insert(index);
                }
                TableRef::Rows(scope) => {
                    let discovery = graph
                        .scopes
                        .get(scope)
                        .and_then(|scope| scope.discovery)
                        .ok_or_else(|| Error::InvalidReference(read.label.to_string()))?;
                    if discovery >= graph.nodes.len() {
                        return Err(Error::InvalidReference(read.label.to_string()));
                    }
                    result.dependencies.insert(discovery);
                }
                TableRef::Node(index) => {
                    if graph.nodes.get(index).is_none_or(|node| {
                        !matches!(node.kind, super::NodeKind::Table)
                            || !super::same_schema(node.plan.schema(), &read.schema)
                    }) {
                        return Err(Error::InvalidReference(read.label.to_string()));
                    }
                    result.dependencies.insert(index);
                }
            }
        }
        LogicalPlan::TableScan(scan) => {
            result.has_source = true;
            if scan.source.get_logical_plan().is_some() {
                return Err(Error::UnsupportedPlan(
                    "provider contains a logical program: register that plan directly".into(),
                ));
            }
        }
        LogicalPlan::Ddl(_)
        | LogicalPlan::Dml(_)
        | LogicalPlan::Copy(_)
        | LogicalPlan::Statement(_)
        | LogicalPlan::Explain(_)
        | LogicalPlan::Analyze(_)
        | LogicalPlan::DescribeTable(_)
        | LogicalPlan::RecursiveQuery(_) => {
            return Err(Error::UnsupportedPlan(plan.display().to_string()))
        }
        _ => {}
    }
    let mut nested_scope = outer.to_vec();
    nested_scope.extend(plan.inputs().iter().map(|input| input.schema().clone()));
    for expr in plan.expressions() {
        let mut failure = None;
        expr.apply(|expr| {
            if let Err(error) = visit_expr(expr, graph, outer, &nested_scope, result) {
                failure = Some(error);
                return Ok(TreeNodeRecursion::Stop);
            }
            Ok(TreeNodeRecursion::Continue)
        })?;
        if let Some(error) = failure {
            return Err(error);
        }
    }
    for input in plan.inputs() {
        visit_plan(input, graph, outer, result)?;
    }
    Ok(())
}

fn visit_expr(
    expr: &Expr,
    graph: &GraphDef,
    outer: &[DFSchemaRef],
    nested_scope: &[DFSchemaRef],
    result: &mut Analysis,
) -> Result<()> {
    let volatility = match expr {
        Expr::ScalarFunction(function) => function.func.signature().volatility,
        Expr::AggregateFunction(function) => function.func.signature().volatility,
        Expr::WindowFunction(function) => function.fun.signature().volatility,
        Expr::HigherOrderFunction(function) => function.func.signature().volatility,
        _ => Volatility::Immutable,
    };
    result.direct_volatility = result.direct_volatility.max(volatility);
    match expr {
        Expr::Placeholder(placeholder) => {
            let (source, field) = graph
                .placeholders
                .get(&placeholder.id)
                .ok_or_else(|| Error::UnknownPlaceholder(placeholder.id.clone()))?;
            if placeholder.field.as_ref() != Some(field) {
                return Err(Error::InvalidReference(format!(
                    "placeholder field mismatch for {}",
                    placeholder.id
                )));
            }
            match source {
                ScalarRef::BaseInput(index) => {
                    base_input(graph, *index)?;
                    result.base_inputs.insert(*index);
                }
                ScalarRef::Import(index) => {
                    result.base_outputs.insert(*index);
                }
                ScalarRef::Input(index) => {
                    result.inputs.insert(*index);
                }
                ScalarRef::Node(index) => {
                    result.dependencies.insert(*index);
                }
            }
        }
        Expr::ScalarVariable(..) => {
            return Err(Error::InvalidExpression(
                "session variables must be explicit scalar inputs".into(),
            ))
        }
        Expr::OuterReferenceColumn(_, column) => {
            if !outer
                .iter()
                .rev()
                .any(|schema| schema.index_of_column(column).is_ok())
            {
                return Err(Error::InvalidExpression(format!(
                    "unresolved outer reference {column}"
                )));
            }
        }
        Expr::ScalarSubquery(subquery) => {
            if subquery.subquery.schema().fields().len() != 1 {
                return Err(Error::InvalidExpression(
                    "a scalar subquery must return exactly one column".into(),
                ));
            }
            visit_plan(&subquery.subquery, graph, nested_scope, result)?;
        }
        Expr::Exists(expr) => visit_plan(&expr.subquery.subquery, graph, nested_scope, result)?,
        Expr::InSubquery(expr) => visit_plan(&expr.subquery.subquery, graph, nested_scope, result)?,
        Expr::SetComparison(expr) => {
            visit_plan(&expr.subquery.subquery, graph, nested_scope, result)?
        }
        _ => {}
    }
    Ok(())
}

fn base_input(graph: &GraphDef, index: usize) -> Result<&super::InputDef> {
    let input = graph
        .base
        .as_ref()
        .ok_or(Error::BaseRequired)?
        .inner
        .inputs
        .get(index)
        .ok_or_else(|| Error::InvalidReference(format!("base input {index}")))?;
    if input.scope != 0 {
        return Err(Error::OutOfScope(format!(
            "base input {}: only root imports are supported",
            input.name
        )));
    }
    Ok(input)
}
