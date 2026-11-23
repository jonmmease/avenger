//! What the charts of a project share wherever they draw, in an editor, a window, or a file: the
//! label engine that typesets their text, the reader's color scheme and the background that
//! goes with it, and the cascade of the base stylesheet and the project's stylesheets.

use std::sync::Arc;

use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_style::{
    Cascade, ColorScheme, Env, ParseOptions, Preferences, Sheet, SourceId, StyleTable,
};
use avenger_typst_label::LabelEngine;

use crate::style::ProjectStylesheet;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn label_engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}

/// The background that charts draw on for a reader who prefers the color scheme `scheme`:
/// `#ffffff` for light and `#1e1e1e` for dark. A host that draws charts on a surface of its own,
/// such as an editor's panel, passes that surface's color to [`cascade`] instead.
pub fn background(scheme: ColorScheme) -> &'static str {
    match scheme {
        ColorScheme::Light => "#ffffff",
        ColorScheme::Dark => "#1e1e1e",
    }
}

/// The preferences of a reader who prefers the color scheme `scheme`, which the
/// `prefers-color-scheme` conditions of the stylesheets test.
pub fn preferences(scheme: ColorScheme) -> Preferences {
    Preferences {
        color_scheme: scheme,
        ..Preferences::default()
    }
}

/// The cascade of the base stylesheet, `std:base.css`, then the author stylesheets `sheets`, in
/// order, each with `background`, a CSS color, as its `env(background)`, so a chart and the
/// surface around it are one.
pub fn cascade<'a>(background: &str, sheets: impl IntoIterator<Item = &'a Sheet>) -> Arc<Cascade> {
    let env = Env::from([("background".to_string(), background.to_string())]);
    let table = sheets.into_iter().fold(
        StyleTable::new().with_author(base(), env.clone()),
        |table, sheet| table.with_author(sheet.clone(), env.clone()),
    );
    Arc::new(Cascade::new(&table))
}

impl ProjectStylesheet {
    /// The cascade of the charts of the project on `background`: the base stylesheet, the
    /// project's entry stylesheet, when it has one, and `extra`, such as a stylesheet that a
    /// command adds, after it.
    pub fn cascade(&self, background: &str, extra: Option<&Sheet>) -> Arc<Cascade> {
        cascade(background, self.sheet.iter().chain(extra))
    }
}

/// The base stylesheet, `std:base.css`.
fn base() -> Sheet {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(u32::MAX),
        options,
        &mut no_files,
    );
    debug_assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    sheet
}
