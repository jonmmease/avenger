//! What every chart of the editor shares, the stylesheet and the label engine, and the static
//! chart of a table cell, which evaluates at the size that the editor gives it.

use std::sync::Arc;

use avenger_chart::Chart;
use avenger_chart_eval::{ChartEvaluator, EvalError, Registry};
use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_style::{Cascade, Env, HostInputs, ParseOptions, Sheet, SourceId, StyleTable};
use avenger_typst_label::LabelEngine;

/// The label engine of the charts: the bundled fonts, with GoogleSQL's number and date formats,
/// whose syntaxes the charts of avenger-sql name.
pub fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()))
}

/// The cascade of the base stylesheet, `std:base.css`.
pub fn cascade() -> Arc<Cascade> {
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
    Arc::new(Cascade::new(
        &StyleTable::new().with_author(sheet, Env::new()),
    ))
}

/// The chart of a table cell, which draws without an app, at any size.
pub struct StaticChart {
    evaluator: ChartEvaluator,
}

impl StaticChart {
    /// Prepares `chart`, styled by `cascade`, with the labels of `engine`.
    pub fn new(
        chart: Chart,
        cascade: Arc<Cascade>,
        engine: LabelEngine,
    ) -> Result<Self, EvalError> {
        let evaluator = ChartEvaluator::new(chart, cascade, engine, Registry::new())?;
        Ok(Self { evaluator })
    }

    /// The scene of the chart at `size`, in logical pixels.
    pub fn scene(&mut self, size: [f32; 2]) -> Result<SceneGraph, EvalError> {
        let host = HostInputs::sized(size[0], size[1]);
        Ok(self.evaluator.evaluate(&host)?.scene)
    }
}
