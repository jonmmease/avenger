use avenger_chart_core::{
    AvengerChartError, ChannelValue, ScaleInferenceHint, ScaleTypePreference,
};
use datafusion::prelude::{Expr, SessionContext, col};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompoundGrouping {
    pub(crate) key_exprs: Vec<Expr>,
    pub(crate) key_names: Vec<String>,
    pub(crate) is_nested: bool,
}

pub(crate) fn compound_grouping_from_position(
    label: &str,
    expr: &Expr,
    value: &ChannelValue,
) -> Result<CompoundGrouping, AvengerChartError> {
    if let Some(nested) = value.get_nested_band_config() {
        let key_names = nested.source_columns.clone();
        return Ok(CompoundGrouping {
            key_exprs: key_names.iter().map(|name| col(name.clone())).collect(),
            key_names,
            is_nested: true,
        });
    }

    let Some(name) = simple_column_name(expr) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be a source column or nested([...]) expression"
        )));
    };
    Ok(CompoundGrouping {
        key_exprs: vec![col(name.clone())],
        key_names: vec![name],
        is_nested: false,
    })
}

pub(crate) fn validate_preserved_style_channel(
    label: &str,
    value: Option<&ChannelValue>,
    grouping: &CompoundGrouping,
) -> Result<(), AvengerChartError> {
    let Some(value) = value else {
        return Ok(());
    };
    if matches!(value, ChannelValue::Value { .. }) {
        return Ok(());
    }

    let ctx = SessionContext::new();
    let Some(expr) = value.expr(&ctx) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} must be a scalar value or a preserved grouping column"
        )));
    };
    if matches!(expr, Expr::Placeholder(_)) {
        return Ok(());
    }
    let Some(column) = simple_column_name(&expr) else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "{label} expressions must be scalar values or preserved grouping columns"
        )));
    };
    if grouping.key_names.iter().any(|name| name == &column) {
        Ok(())
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "{label} references column '{column}', but compound summary rows are grouped only by {}",
            if grouping.key_names.is_empty() {
                "the categorical position expression".to_string()
            } else {
                grouping.key_names.join(", ")
            }
        )))
    }
}

pub(crate) fn band_scale_hint_for_grouping(
    grouping: &CompoundGrouping,
    value: &ChannelValue,
    default_scale_name: &str,
) -> Option<ScaleInferenceHint> {
    (!grouping.is_nested).then(|| {
        ScaleInferenceHint::new(
            value
                .get_scale_name(default_scale_name)
                .unwrap_or_else(|| default_scale_name.to_string()),
            ScaleTypePreference::Band,
        )
    })
}

pub(crate) fn simple_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(column) => Some(column.name.clone()),
        _ => None,
    }
}
