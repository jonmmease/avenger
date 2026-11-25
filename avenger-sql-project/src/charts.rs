//! What the charts of a project share wherever they draw, in an editor, a window, or a file: the
//! label engine that typesets their text. The cascade of their styles is
//! `avenger_style::chart_cascade`, and the preferences of a reader's color scheme are
//! `ColorScheme::preferences`, which every host builds alike.

use std::sync::Arc;

use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_typst_label::LabelEngine;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn label_engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}
