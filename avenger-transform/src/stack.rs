use datafusion::{
    common::{plan_err, Column, Result, ScalarValue},
    functions::core::expr_fn::coalesce,
    functions_aggregate::sum::sum_udaf,
    logical_expr::{
        expr::Sort, expr::WindowFunction, lit, when, Expr, ExprFunctionExt, LogicalPlan,
        LogicalPlanBuilder, WindowFrame, WindowFrameBound, WindowFrameUnits,
    },
};

/// Append zero-based stack intervals, accumulating positive and negative values separately.
///
/// `order_by` must provide a stable total order within each partition. Null and NaN
/// contribute zero. Output names must be distinct and absent from the input.
pub fn stack_zero(
    input: LogicalPlan,
    partition_by: Vec<Expr>,
    value: Expr,
    order_by: Vec<Sort>,
    output: [impl Into<String>; 2],
) -> Result<LogicalPlan> {
    let [start, end] = output.map(Into::into);
    if order_by.is_empty()
        || start == end
        || [start.as_str(), end.as_str()]
            .iter()
            .any(|name| input.schema().fields().iter().any(|f| f.name() == name))
    {
        return plan_err!("stack_zero requires an explicit order and distinct new output names");
    }
    let mut prefix = "__stack_".to_string();
    while input
        .schema()
        .fields()
        .iter()
        .any(|f| f.name().starts_with(&prefix))
        || start.starts_with(&prefix)
        || end.starts_with(&prefix)
    {
        prefix.push('_');
    }
    let value = coalesce(vec![
        crate::udf::Function::Numeric.call(vec![value]),
        lit(0.0),
    ]);
    let columns = input
        .schema()
        .columns()
        .into_iter()
        .map(Expr::Column)
        .collect::<Vec<_>>();
    let frame = WindowFrame::new_bounds(
        WindowFrameUnits::Rows,
        WindowFrameBound::Preceding(ScalarValue::UInt64(None)),
        WindowFrameBound::CurrentRow,
    );
    let positive = value.clone().gt_eq(lit(0.0));
    let window = |condition: Expr, name: &str| -> Result<Expr> {
        let arg = when(condition, value.clone()).otherwise(lit(0.0))?;
        Ok(
            Expr::WindowFunction(Box::new(WindowFunction::new(sum_udaf(), vec![arg])))
                .partition_by(partition_by.clone())
                .order_by(order_by.clone())
                .window_frame(frame.clone())
                .build()?
                .alias(name),
        )
    };
    let pos = format!("{prefix}positive");
    let neg = format!("{prefix}negative");
    let plan = LogicalPlanBuilder::from(input).window(vec![
        window(positive.clone(), &pos)?,
        window(value.clone().lt(lit(0.0)), &neg)?,
    ])?;
    let upper = when(positive, Expr::Column(Column::from_name(pos)))
        .otherwise(Expr::Column(Column::from_name(neg)))?;
    let mut columns = columns;
    columns.push((upper.clone() - value).alias(start));
    columns.push(upper.alias(end));
    plan.project(columns)?.build()
}
