use datafusion::prelude::{Expr, SessionContext, col};

use crate::{AvengerChartError, ChannelValue, ScaleInferenceHint, ScaleTypePreference};

/// Grouping metadata used by aggregate-backed compound marks.
///
/// Compound marks such as box plots and violins expand to ordinary marks and
/// transforms, but they still need a stable definition of the source columns
/// preserved through their internal aggregate branches. A grouping channel may
/// be either a simple source column or an explicit `nested([...])` channel.
#[derive(Clone, Debug, PartialEq)]
pub struct CompoundGrouping {
    pub key_exprs: Vec<Expr>,
    pub key_names: Vec<String>,
    pub is_nested: bool,
}

impl CompoundGrouping {
    /// Resolve compound mark grouping from a positional channel.
    pub fn from_position(
        label: &str,
        expr: &Expr,
        value: &ChannelValue,
    ) -> Result<Self, AvengerChartError> {
        if let Some(nested) = value.get_nested_band_config() {
            let key_names = nested.source_columns.clone();
            return Ok(Self {
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
        Ok(Self {
            key_exprs: vec![col(name.clone())],
            key_names: vec![name],
            is_nested: false,
        })
    }

    /// Return the band-scale hint needed for non-nested categorical grouping.
    pub fn band_scale_hint(
        &self,
        value: &ChannelValue,
        default_scale_name: &str,
    ) -> Option<ScaleInferenceHint> {
        band_scale_hint(self, value, default_scale_name)
    }
}

/// Validate that an aggregate-backed compound mark style channel can be
/// evaluated from its internal summary rows.
pub fn validate_preserved_style_channel(
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

/// Return a scale inference hint for non-nested compound mark grouping.
pub fn band_scale_hint(
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

fn simple_column_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Column(column) => Some(column.name.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelValue, nested};
    use datafusion::prelude::col;

    #[test]
    fn grouping_from_simple_column() {
        let value = ChannelValue::from(col("group"));
        let grouping = CompoundGrouping::from_position("test grouping", &col("group"), &value)
            .expect("grouping");
        assert_eq!(grouping.key_exprs, vec![col("group")]);
        assert_eq!(grouping.key_names, vec!["group".to_string()]);
        assert!(!grouping.is_nested);
    }

    #[test]
    fn grouping_from_nested_source_columns() {
        let nested = nested(["category", "segment"]);
        let expr = nested.data_expr().clone();
        let value = nested.into_channel_value();
        let grouping =
            CompoundGrouping::from_position("test grouping", &expr, &value).expect("grouping");
        assert_eq!(
            grouping.key_names,
            vec!["category".to_string(), "segment".to_string()]
        );
        assert_eq!(grouping.key_exprs, vec![col("category"), col("segment")]);
        assert!(grouping.is_nested);
    }

    #[test]
    fn grouping_rejects_arbitrary_expression() {
        let err = CompoundGrouping::from_position(
            "test grouping",
            &(col("a") + col("b")),
            &ChannelValue::from(col("a") + col("b")),
        )
        .expect_err("arbitrary expression should fail");
        assert!(
            err.to_string()
                .contains("test grouping must be a source column or nested"),
            "{err}"
        );
    }

    #[test]
    fn preserved_style_validation() {
        let grouping = CompoundGrouping {
            key_exprs: vec![col("category")],
            key_names: vec!["category".to_string()],
            is_nested: false,
        };
        validate_preserved_style_channel(
            "compound fill",
            Some(&ChannelValue::from(col("category"))),
            &grouping,
        )
        .expect("preserved grouping column");
        validate_preserved_style_channel(
            "compound fill",
            Some(&ChannelValue::from("red")),
            &grouping,
        )
        .expect("scalar style");
        let err = validate_preserved_style_channel(
            "compound fill",
            Some(&ChannelValue::from(col("region"))),
            &grouping,
        )
        .expect_err("unpreserved column");
        assert!(err.to_string().contains("region"), "{err}");
    }

    #[test]
    fn band_hint_only_for_non_nested_grouping() {
        let non_nested = CompoundGrouping {
            key_exprs: vec![col("group")],
            key_names: vec!["group".to_string()],
            is_nested: false,
        };
        assert_eq!(
            band_scale_hint(&non_nested, &ChannelValue::from(col("group")), "x"),
            Some(ScaleInferenceHint::new("x", ScaleTypePreference::Band))
        );
        let nested = CompoundGrouping {
            key_exprs: vec![col("category"), col("segment")],
            key_names: vec!["category".to_string(), "segment".to_string()],
            is_nested: true,
        };
        assert!(band_scale_hint(&nested, &ChannelValue::from(col("category")), "x").is_none());
    }
}
