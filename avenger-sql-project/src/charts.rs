//! What the charts of a project share wherever they draw, in an editor, a window, or a file: the
//! label engine that typesets their text, and the preferences of the reader's color scheme. The
//! cascade of their styles is `avenger_style::chart_cascade`, which every host builds alike.

use std::sync::Arc;

use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_style::{ColorScheme, Preferences};
use avenger_typst_label::LabelEngine;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn label_engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}

/// The preferences of a reader who prefers the color scheme `scheme`, which the
/// `prefers-color-scheme` conditions of the stylesheets test.
pub fn preferences(scheme: ColorScheme) -> Preferences {
    Preferences {
        color_scheme: scheme,
        ..Preferences::default()
    }
}
