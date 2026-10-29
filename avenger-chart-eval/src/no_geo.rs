//! Map plots without the `geo` feature: the evaluator draws none, and preparing a chart with one
//! fails with an error that names the feature.

use std::collections::BTreeMap;

use arrow::array::RecordBatch;
use avenger_guides::GuideSide;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_typst_label::LabelEngine;

use avenger_chart::{proto, resolve::Tree, Chart};

use crate::{
    encode::{Drawn, Projected, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord},
    fit::Reach,
    guides::MapGuide,
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

/// A unit of a map plot, with its drawn rows and the reach of its marks. The evaluator builds one
/// for each unit of a map plot, which this module rejects, so it reads none of them.
#[allow(dead_code)]
pub(crate) struct MapUnit<'a> {
    pub(crate) rows: UnitRows<'a>,
    pub(crate) reach: Option<&'a [Reach]>,
    pub(crate) plot: usize,
}

/// The map plots of a chart, of which there are none.
pub(crate) struct Maps;

/// A map plot at a size, of which there are none.
pub(crate) enum PlotView {}

impl Maps {
    /// Fails for a chart with a map plot.
    pub(crate) fn prepare(
        _units: &[MapUnit],
        tree: &Tree,
        _theme: &Theme,
        _engine: &LabelEngine,
    ) -> Result<Self> {
        for plot in &tree.plots {
            if let Some(proto::coord::Coord::Geo(_)) =
                plot.coord.and_then(|coord| coord.coord.as_ref())
            {
                return Err(EvalError::Evaluation(format!(
                    "the map plot `{}` needs the `geo` feature of avenger-chart-eval",
                    plot.path
                )));
            }
        }
        Ok(Self)
    }

    pub(crate) fn guides(&self) -> BTreeMap<usize, MapGuide> {
        BTreeMap::new()
    }

    pub(crate) fn views(
        &self,
        _sizes: &[[f32; 2]],
        _warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<BTreeMap<usize, PlotView>> {
        Ok(BTreeMap::new())
    }
}

impl PlotView {
    pub(crate) fn clips(&self) -> bool {
        match *self {}
    }

    pub(crate) fn coord(&self) -> PlotCoord {
        match *self {}
    }

    pub(crate) fn backdrop(&self, _maps: &Maps) -> Result<Vec<SceneMark>> {
        match *self {}
    }

    pub(crate) fn labels(
        &self,
        _maps: &Maps,
        _side: GuideSide,
        _slot: [f32; 4],
    ) -> Option<SceneGroup> {
        match *self {}
    }

    pub(crate) fn locate(
        &self,
        _chart: &Chart,
        _unit: &proto::Unit,
        _table: &RecordBatch,
    ) -> Result<(RecordBatch, Option<Projected>)> {
        match *self {}
    }

    pub(crate) fn marks(&self, _rows: &UnitRows) -> Result<Vec<Drawn>> {
        match *self {}
    }
}
