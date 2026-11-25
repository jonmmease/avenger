//! The cascade of a chart's styles, which every host of charts builds the same way, so a chart in
//! a browser and its image from the CLI style alike.

use std::sync::Arc;

use crate::{
    table::{CHART_BASE, CHART_TRANSPARENT},
    Cascade, Env, ParseOptions, Sheet, SourceId, StyleTable,
};

/// The cascade of a chart: the base stylesheet, `std:base.css`, then, for a `transparent` chart,
/// `chart { background: transparent; }`, which leaves the chart's own background to the surface
/// around it, then the author stylesheets `sheets`, in order, each with `background`, a CSS color,
/// as its `env(background)`, so a chart and the surface around it are one. The base stylesheet
/// mixes `env(background)` into the colors of guide lines, so a transparent chart keeps the
/// background of the surface that it draws on.
pub fn chart_cascade<'a>(
    background: &str,
    transparent: bool,
    sheets: impl IntoIterator<Item = &'a Sheet>,
) -> Arc<Cascade> {
    let env = Env::from([("background".to_string(), background.to_string())]);
    let mut table =
        StyleTable::new().with_author(builtin("@import 'std:base.css';", CHART_BASE), env.clone());
    if transparent {
        let rule = builtin("chart { background: transparent; }", CHART_TRANSPARENT);
        table = table.with_author(rule, env.clone());
    }
    let table = (sheets.into_iter()).fold(table, |table, sheet| {
        table.with_author(sheet.clone(), env.clone())
    });
    Arc::new(Cascade::new(&table))
}

/// The stylesheet of `text`, which imports only the standard stylesheets.
fn builtin(text: &str, source: SourceId) -> Sheet {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(text, None, source, options, &mut no_files);
    debug_assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    sheet
}
