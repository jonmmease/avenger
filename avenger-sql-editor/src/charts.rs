//! What every chart of the editor shares, the stylesheet, its theme, and the label engine, and the
//! static chart of a table cell, which evaluates at the size that the editor gives it.

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_style::{
    Cascade, ColorScheme, Env, HostInputs, ParseOptions, Preferences, Sheet, SourceId, StyleTable,
};
use avenger_typst_label::LabelEngine;
use eframe::egui;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}

/// The theme of the charts, from the editor's visuals: the background of the panels, so that a
/// chart and the panel around it are one surface, and whether the theme is dark. The charts' text
/// takes the contrast of the background.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChartTheme {
    pub background: egui::Color32,
    pub dark: bool,
}

impl ChartTheme {
    pub fn of(visuals: &egui::Visuals) -> Self {
        Self {
            background: visuals.panel_fill,
            dark: visuals.dark_mode,
        }
    }

    /// The preferences of the charts' reader: the color scheme of the theme.
    pub fn preferences(&self) -> Preferences {
        let color_scheme = match self.dark {
            true => ColorScheme::Dark,
            false => ColorScheme::Light,
        };
        Preferences {
            color_scheme,
            ..Preferences::default()
        }
    }

    /// The cascade of the base stylesheet, `std:base.css`, on the theme's background.
    pub fn cascade(&self) -> Arc<Cascade> {
        let [r, g, b, _] = self.background.to_array();
        let background = format!("#{r:02x}{g:02x}{b:02x}");
        cascade_on(Env::from([("background".to_string(), background)]))
    }
}

impl Default for ChartTheme {
    /// The theme of egui's light visuals.
    fn default() -> Self {
        Self::of(&egui::Visuals::light())
    }
}

/// The cascade of the base stylesheet, `std:base.css`, with the `env()` values `env`.
fn cascade_on(env: Env) -> Arc<Cascade> {
    let mut no_files = |url: &str, _: Option<&str>| Err(format!("no file `{url}`"));
    let options = ParseOptions {
        allow_import: true,
        ..Default::default()
    };
    let (sheet, diagnostics) = Sheet::load(
        "@import 'std:base.css';",
        None,
        SourceId(1),
        options,
        &mut no_files,
    );
    debug_assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    Arc::new(Cascade::new(&StyleTable::new().with_author(sheet, env)))
}

/// The chart of a table cell, which draws without an app, at any size.
pub struct StaticChart {
    evaluator: ChartEvaluator,
    preferences: Preferences,
}

impl StaticChart {
    /// Prepares `chart`, styled by `cascade` for a reader with `preferences`, with the labels of
    /// `engine`.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        preferences: Preferences,
        engine: LabelEngine,
    ) -> Result<Self, EvalError> {
        let evaluator = ChartEvaluator::new(chart, cascade, engine, Registry::new())?;
        Ok(Self {
            evaluator,
            preferences,
        })
    }

    /// The scene of the chart at `size`, in logical pixels.
    pub fn scene(&mut self, size: [f32; 2]) -> Result<SceneGraph, EvalError> {
        let host = HostInputs {
            preferences: self.preferences.clone(),
            ..HostInputs::sized(size[0], size[1])
        };
        Ok(self.evaluator.evaluate(&host)?.scene)
    }
}
