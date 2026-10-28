//! Labels for band and point axes, which put a tick and a label at the center of each band.

use arrow::array::ArrayRef;
use avenger_common::types::TextSyntaxMode;
use avenger_format::PrecisionSharing;
use avenger_typst_label::LabelEngine;

use super::tick_label_text;
use crate::error::AvengerGuidesError;

/// The gap between the labels and the title.
pub(crate) const TITLE_MARGIN: f32 = 4.0;

/// The label of each of `categories`. The text engine formats numeric and temporal categories
/// with `tick_format`, each on its own, and other categories show as text.
pub(crate) fn category_text(
    categories: &ArrayRef,
    tick_format: &str,
    tick_label: Option<&str>,
    engine: &LabelEngine,
) -> Result<(Vec<String>, TextSyntaxMode), AvengerGuidesError> {
    let ticks = engine.format_array(categories, tick_format, PrecisionSharing::PerValue)?;
    tick_label_text(ticks, tick_label)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::StringArray;

    use super::*;

    #[test]
    fn category_labels_bind_each_category() {
        let categories = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
        let (text, _) =
            category_text(&categories, ",", Some("*#tick*"), &crate::test_engine()).unwrap();
        assert_eq!(text, [r#"*#("a")*"#, r#"*#("b")*"#]);
    }
}
