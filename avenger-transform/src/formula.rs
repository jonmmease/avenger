use datafusion::{
    arrow::datatypes::DataType,
    common::{plan_err, Column, Result},
    logical_expr::{
        expr_rewriter::normalize_col, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder,
    },
};

/// Filter rows using Vega truthiness for Boolean, numeric, and string values.
/// Null, NaN, zero, and empty strings do not pass. The schema is unchanged.
pub fn filter(input: LogicalPlan, predicate: Expr) -> Result<LogicalPlan> {
    let predicate = normalize_col(predicate, &input)?;
    let predicate = if predicate.get_type(input.schema())? == DataType::Boolean {
        predicate
    } else {
        crate::expr_fn::truthy(predicate)
    };
    LogicalPlanBuilder::from(input).filter(predicate)?.build()
}

/// Add or replace a field with an expression evaluated against the input schema.
/// Existing fields retain their positions. New fields are appended.
pub fn formula(input: LogicalPlan, value: Expr, output_name: &str) -> Result<LogicalPlan> {
    let expressions = replace_fields(&input, vec![(output_name, value)])?;
    LogicalPlanBuilder::from(input)
        .project(expressions)?
        .build()
}

pub(crate) fn replace_fields(
    input: &LogicalPlan,
    replacements: Vec<(&str, Expr)>,
) -> Result<Vec<Expr>> {
    for (i, (name, _)) in replacements.iter().enumerate() {
        if replacements[..i].iter().any(|(prior, _)| prior == name) {
            return plan_err!("Transform output field {name} is repeated");
        }
        if input
            .schema()
            .fields()
            .iter()
            .filter(|f| f.name() == name)
            .count()
            > 1
        {
            return plan_err!("Transform output field {name} is ambiguous");
        }
    }
    let mut expressions = input
        .schema()
        .iter()
        .map(|(qualifier, field)| {
            replacements
                .iter()
                .find(|(name, _)| *name == field.name())
                .map(|(name, value)| value.clone().alias(*name))
                .unwrap_or_else(|| Expr::Column(Column::from((qualifier, field))))
        })
        .collect::<Vec<_>>();
    for (name, value) in replacements {
        if !input.schema().fields().iter().any(|f| f.name() == name) {
            expressions.push(value.alias(name));
        }
    }
    Ok(expressions)
}

pub(crate) fn internal_name(input: &LogicalPlan, base: &str) -> String {
    let mut name = base.to_owned();
    while input.schema().fields().iter().any(|f| f.name() == &name) {
        name.push('_');
    }
    name
}
