//! Labels for band and point axes, which put a tick and a label at the center of each band.

use arrow::array::ArrayRef;
use arrow::row::{RowConverter, SortField};
use avenger_common::types::TextSyntaxMode;
use avenger_format::PrecisionSharing;
use avenger_typst_label::{bind, LabelEngine, LabelValue, LabelValues};

use super::{label_text, CategoryLabels};
use crate::{error::AvengerGuidesError, label_variant::vary_labels};

/// The gap between the labels and the title.
pub(crate) const TITLE_MARGIN: f32 = 4.0;

/// The label of each of `categories`. The text engine formats numeric and temporal categories
/// with `tick_format`, each on its own, and other categories show as text. A category that
/// `labels` holds binds its values in place of its formatted value, in its variant if it has one.
pub(crate) fn category_text(
    categories: &ArrayRef,
    tick_format: &str,
    tick_label: Option<&str>,
    labels: Option<&CategoryLabels>,
    engine: &LabelEngine,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    let Some(labels) = labels else {
        let ticks = engine.format_array(categories, tick_format, PrecisionSharing::PerValue)?;
        return label_text(ticks, tick_label);
    };
    let rows = |values: &ArrayRef| -> Result<Vec<Vec<u8>>, AvengerGuidesError> {
        let converter = RowConverter::new(vec![SortField::new(values.data_type().clone())])?;
        let rows = converter.convert_columns(std::slice::from_ref(values))?;
        Ok(rows.iter().map(|row| row.as_ref().to_vec()).collect())
    };
    let known = rows(&labels.categories)?;
    let mut values = Vec::with_capacity(categories.len());
    let mut variants = Vec::with_capacity(categories.len());
    for (index, row) in rows(categories)?.iter().enumerate() {
        let position = known.iter().position(|known| known == row);
        variants
            .push(position.and_then(|position| labels.variants.get(position).cloned().flatten()));
        values.push(match position {
            Some(position) => labels.values[position].clone(),
            None => {
                let value = (engine.format_array(
                    &categories.slice(index, 1),
                    tick_format,
                    PrecisionSharing::PerValue,
                )?)
                .remove(0);
                LabelValues::from([("value".into(), value)])
            }
        });
    }
    let (text, syntax) = bound_text(values, tick_label)?;
    Ok(vary_labels(text, syntax, &variants))
}

/// The labels of maps of values: each map bound into `template`, or without one, its `value` as
/// text or markup.
fn bound_text(
    values: Vec<LabelValues>,
    template: Option<&str>,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    match template {
        Some(template) => Ok((
            (values.iter())
                .map(|values| bind(template, values))
                .collect::<Result<_, _>>()?,
            TextSyntaxMode::TypstMarkup,
        )),
        None => {
            let values = (values.into_iter())
                .map(|mut values| values.shift_remove("value").unwrap_or(LabelValue::None))
                .map(|value| match value {
                    LabelValue::Str(_) | LabelValue::Markup(_) => value,
                    LabelValue::None => LabelValue::Str(String::new()),
                    value => LabelValue::Str(format!("{value:?}")),
                })
                .collect();
            label_text(values, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::StringArray;

    use super::*;

    #[test]
    fn category_labels_bind_each_category() {
        let categories = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
        let (text, _) = category_text(
            &categories,
            ",",
            Some("*#value*"),
            None,
            &crate::test_engine(),
        )
        .unwrap();
        assert_eq!(text, [r#"*#("a")*"#, r#"*#("b")*"#]);
    }
}
