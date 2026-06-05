use avenger_chart_core::{AvengerChartError, DefaultLogicalExprNodeExt};
use datafusion::logical_expr::Expr;
use datafusion_proto::protobuf::LogicalExprNode;

pub(crate) fn expr_node(expr: Expr, label: &str) -> LogicalExprNode {
    LogicalExprNode::from_default_expr(expr).expect(label)
}

pub(crate) fn simple_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(column) => Some(column.name.clone()),
        _ => None,
    }
}

pub(crate) fn sanitize_output_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "value_stack".to_string()
    } else {
        out.to_string()
    }
}

pub(crate) fn validate_output_names<'a>(
    existing: impl IntoIterator<Item = &'a String>,
    proposed: impl IntoIterator<Item = &'a str>,
) -> Result<(), AvengerChartError> {
    let existing = existing.into_iter().collect::<Vec<_>>();
    let mut seen = indexmap::IndexMap::<&str, ()>::new();
    for name in proposed {
        if name.is_empty() || name.starts_with("__unused") {
            continue;
        }
        if existing.iter().any(|field| field.as_str() == name) {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{name}' conflicts with an input column; choose a different output name"
            )));
        }
        if seen.insert(name, ()).is_some() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Data transform output name '{name}' is duplicated"
            )));
        }
    }
    Ok(())
}
