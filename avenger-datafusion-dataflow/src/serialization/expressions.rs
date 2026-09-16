//! Narrow compatibility envelope for subquery variants absent from DataFusion's protobuf.
use super::*;
use datafusion::{
    arrow::datatypes::DataType,
    logical_expr::{
        expr::{Exists, InSubquery, Placeholder, SetComparison, SetQuantifier},
        Operator, Subquery,
    },
};
const PREFIX: &str = "$dataflow_extension_";
fn subquery_wire(
    subquery: &Subquery,
    codec: &Codec,
) -> Result<datafusion_proto::protobuf::SubqueryNode> {
    Ok(datafusion_proto::protobuf::SubqueryNode {
        subquery: Some(Box::new(LogicalPlanNode::try_from_logical_plan(
            &subquery.subquery,
            codec,
        )?)),
        outer_ref_columns: subquery
            .outer_ref_columns
            .iter()
            .map(|e| serialize_expr(e, codec).map_err(invalid))
            .collect::<Result<_>>()?,
    })
}
fn outer_refs(expr: &mut Expr, mut f: impl FnMut(Expr) -> Result<Expr>) -> Result<()> {
    let query = match expr {
        Expr::ScalarSubquery(q) => Some(q),
        Expr::InSubquery(q) => Some(&mut q.subquery),
        Expr::Exists(q) => Some(&mut q.subquery),
        Expr::SetComparison(q) => Some(&mut q.subquery),
        _ => None,
    };
    if let Some(query) = query {
        query.outer_ref_columns = query
            .outer_ref_columns
            .iter()
            .cloned()
            .map(&mut f)
            .collect::<Result<_>>()?;
    }
    Ok(())
}
fn encode_expr(
    mut expr: Expr,
    codec: &Codec,
    extensions: &mut Vec<wire::ExpressionExtension>,
) -> Result<Expr> {
    outer_refs(&mut expr, |expr| encode_expr(expr, codec, extensions))?;
    let boolean = Arc::new(Field::new("", DataType::Boolean, true));
    let (extension, field) = match &expr {
        Expr::InSubquery(q) => (
            wire::ExpressionExtension {
                kind: wire::expression_extension::Kind::In as i32,
                operand: Some(serialize_expr(&q.expr, codec).map_err(invalid)?),
                subquery: Some(subquery_wire(&q.subquery, codec)?),
                negated: q.negated,
                ..Default::default()
            },
            boolean,
        ),
        Expr::Exists(q) => (
            wire::ExpressionExtension {
                kind: wire::expression_extension::Kind::Exists as i32,
                subquery: Some(subquery_wire(&q.subquery, codec)?),
                negated: q.negated,
                ..Default::default()
            },
            Arc::new(Field::new("", DataType::Boolean, false)),
        ),
        Expr::OuterReferenceColumn(field, column) => (
            wire::ExpressionExtension {
                kind: wire::expression_extension::Kind::OuterColumn as i32,
                field: Some(field.as_ref().try_into().map_err(invalid)?),
                operand: Some(
                    serialize_expr(&Expr::Column(column.clone()), codec).map_err(invalid)?,
                ),
                ..Default::default()
            },
            field.clone(),
        ),
        Expr::SetComparison(q) => (
            wire::ExpressionExtension {
                kind: wire::expression_extension::Kind::SetComparison as i32,
                operand: Some(serialize_expr(&q.expr, codec).map_err(invalid)?),
                subquery: Some(subquery_wire(&q.subquery, codec)?),
                comparison: q.op.to_string(),
                all: matches!(q.quantifier, SetQuantifier::All),
                ..Default::default()
            },
            boolean,
        ),
        _ => return Ok(expr),
    };
    let id = extensions.len();
    extensions.push(extension);
    Ok(Expr::Placeholder(Placeholder::new_with_field(
        format!("{PREFIX}{id}"),
        Some(field),
    )))
}
pub(super) fn encode(
    plan: LogicalPlan,
    codec: &Codec,
    extensions: &mut Vec<wire::ExpressionExtension>,
) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up_with_subqueries(|plan| {
            let names = NamePreserver::new(&plan);
            plan.map_expressions(|expr| {
                let name = names.save(&expr);
                expr.transform_up(|expr| {
                    encode_expr(expr, codec, extensions)
                        .map(Transformed::yes)
                        .map_err(df_error)
                })
                .map(|r| r.update_data(|e| name.restore(e)))
            })?
            .map_data(LogicalPlan::recompute_schema)
        })?
        .data)
}
fn restore_expr(mut expr: Expr, extensions: &[Expr]) -> Result<Expr> {
    outer_refs(&mut expr, |e| restore_expr(e, extensions))?;
    if let Expr::Placeholder(p) = &expr {
        if let Some(index) = p.id.strip_prefix(PREFIX) {
            return extensions
                .get(index.parse::<usize>().map_err(invalid)?)
                .cloned()
                .ok_or_else(|| invalid("invalid or forward expression extension reference"));
        }
    }
    Ok(expr)
}
fn restore(plan: LogicalPlan, extensions: &[Expr]) -> Result<LogicalPlan> {
    Ok(plan
        .transform_up_with_subqueries(|plan| {
            let names = NamePreserver::new(&plan);
            plan.map_expressions(|expr| {
                let name = names.save(&expr);
                expr.transform_up(|e| {
                    restore_expr(e, extensions)
                        .map(Transformed::yes)
                        .map_err(df_error)
                })
                .map(|r| r.update_data(|e| name.restore(e)))
            })?
            .map_data(LogicalPlan::recompute_schema)
        })?
        .data)
}
pub(super) fn decode(
    plan: LogicalPlan,
    wire: &[wire::ExpressionExtension],
    task: &TaskContext,
    codec: &Codec,
) -> Result<LogicalPlan> {
    let mut extensions = Vec::new();
    for extension in wire {
        let operand = || -> Result<Expr> {
            let expr = parse_expr(
                extension
                    .operand
                    .as_ref()
                    .ok_or_else(|| invalid("missing extension operand"))?,
                task,
                codec,
            )
            .map_err(invalid)?;
            Ok(expr
                .transform_up(|e| {
                    restore_expr(e, &extensions)
                        .map(Transformed::yes)
                        .map_err(df_error)
                })?
                .data)
        };
        let subquery = || -> Result<Subquery> {
            let q = extension
                .subquery
                .as_ref()
                .ok_or_else(|| invalid("missing extension subquery"))?;
            let plan = q
                .subquery
                .as_ref()
                .ok_or_else(|| invalid("missing subquery plan"))?
                .try_into_logical_plan(task, codec)?;
            let columns = q
                .outer_ref_columns
                .iter()
                .map(|e| restore_expr(parse_expr(e, task, codec).map_err(invalid)?, &extensions))
                .collect::<Result<_>>()?;
            Ok(Subquery {
                subquery: Arc::new(restore(plan, &extensions)?),
                outer_ref_columns: columns,
                spans: Default::default(),
            })
        };
        let expr =
            match wire::expression_extension::Kind::try_from(extension.kind).map_err(invalid)? {
                wire::expression_extension::Kind::In => Expr::InSubquery(InSubquery::new(
                    Box::new(operand()?),
                    subquery()?,
                    extension.negated,
                )),
                wire::expression_extension::Kind::Exists => {
                    Expr::Exists(Exists::new(subquery()?, extension.negated))
                }
                wire::expression_extension::Kind::OuterColumn => {
                    let Expr::Column(column) = operand()? else {
                        return Err(invalid("outer reference must name a column"));
                    };
                    Expr::OuterReferenceColumn(
                        Arc::new(
                            Field::try_from(
                                extension
                                    .field
                                    .as_ref()
                                    .ok_or_else(|| invalid("missing outer field"))?,
                            )
                            .map_err(invalid)?,
                        ),
                        column,
                    )
                }
                wire::expression_extension::Kind::SetComparison => {
                    let op = match extension.comparison.as_str() {
                        "=" => Operator::Eq,
                        "!=" => Operator::NotEq,
                        "<" => Operator::Lt,
                        "<=" => Operator::LtEq,
                        ">" => Operator::Gt,
                        ">=" => Operator::GtEq,
                        _ => return Err(invalid("unsupported set comparison")),
                    };
                    Expr::SetComparison(SetComparison::new(
                        Box::new(operand()?),
                        subquery()?,
                        op,
                        if extension.all {
                            SetQuantifier::All
                        } else {
                            SetQuantifier::Any
                        },
                    ))
                }
            };
        extensions.push(expr);
    }
    restore(plan, &extensions)
}
