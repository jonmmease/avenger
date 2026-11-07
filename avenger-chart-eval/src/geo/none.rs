//! The map plots of a chart without the `geo` feature, which has none, because the evaluator
//! rejects a chart with a map plot.

use std::collections::BTreeMap;

use avenger_chart::proto::geo_layer;
use avenger_coords::PlotCoords;
use avenger_guides::GuideSide;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use super::{Located, MapGuide, MapInputs};
use crate::{
    encode::{LayerRows, Result},
    marks::Drawn,
    reach::Reach,
    EvalWarning,
};

/// No map plots.
pub(crate) struct Maps;

/// A map plot at a size, which no chart has without the `geo` feature.
pub(crate) enum MapFit {}

impl Maps {
    pub(crate) fn prepare<'r>(
        _inputs: &MapInputs,
        _layer_rows: impl Fn(usize) -> LayerRows<'r>,
        _reach: &[Option<Vec<Reach>>],
    ) -> Result<Self> {
        Ok(Self)
    }

    pub(crate) fn guide(&self, _plot: usize) -> Option<&MapGuide> {
        None
    }

    pub(crate) fn fit(
        &self,
        _sizes: &[[f32; 2]],
        _warnings: &mut Vec<EvalWarning>,
    ) -> Result<BTreeMap<usize, MapFit>> {
        Ok(BTreeMap::new())
    }

    pub(crate) fn backdrop(
        &self,
        _plot: usize,
        fit: &MapFit,
        _warnings: &mut Vec<EvalWarning>,
    ) -> Result<Vec<SceneMark>> {
        match *fit {}
    }

    pub(crate) fn labels(
        &self,
        _plot: usize,
        fit: &MapFit,
        _side: GuideSide,
        _slot: [f32; 4],
    ) -> Option<SceneGroup> {
        match *fit {}
    }
}

impl MapFit {
    pub(crate) fn clip(&self) -> bool {
        match *self {}
    }

    pub(crate) fn coords(&self) -> PlotCoords {
        match *self {}
    }

    pub(crate) fn locate(&self, _rows: &LayerRows) -> Result<Option<Located>> {
        match *self {}
    }
}

pub(crate) fn marks(
    _rows: &LayerRows,
    _mark: &geo_layer::Mark,
    fit: &MapFit,
    _points: Option<[Vec<f32>; 2]>,
) -> Result<Vec<Drawn>> {
    match *fit {}
}
