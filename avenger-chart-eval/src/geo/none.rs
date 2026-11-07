//! The geo plots of a chart without the `geo` feature, which has none, because the evaluator
//! rejects a chart with a geo plot.

use std::collections::BTreeMap;

use avenger_chart::proto::geo_layer;
use avenger_coords::PlotCoords;
use avenger_guides::GuideSide;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use super::{GeoGuide, GeoInputs, Located, ProjectionNavigation, ProjectionView};
use crate::{
    encode::{LayerRows, Result},
    marks::Drawn,
    reach::Reach,
    EvalWarning,
};

/// No geo plots.
pub(crate) struct GeoPlots;

/// No views and no carried fits of projections.
#[derive(Default)]
pub(crate) struct ProjectionState {}

/// A geo plot at a size, which no chart has without the `geo` feature.
pub(crate) enum GeoFit {}

impl GeoPlots {
    pub(crate) fn prepare<'r>(
        _inputs: &GeoInputs,
        _layer_rows: impl Fn(usize) -> LayerRows<'r>,
        _reach: &[Option<Vec<Reach>>],
        _state: &ProjectionState,
    ) -> Result<Self> {
        Ok(Self)
    }

    pub(crate) fn guide(&self, _plot: usize) -> Option<&GeoGuide> {
        None
    }

    pub(crate) fn group_of(&self, _plot: usize) -> Option<usize> {
        None
    }

    pub(crate) fn home(&self, _plot: usize, _size: [f32; 2]) -> Option<ProjectionView> {
        None
    }

    pub(crate) fn view_mut<'s>(
        &self,
        _state: &'s mut ProjectionState,
        _plot: usize,
        _size: [f32; 2],
    ) -> Option<&'s mut ProjectionView> {
        None
    }

    pub(crate) fn view<'s>(
        &self,
        _state: &'s ProjectionState,
        _plot: usize,
    ) -> Option<&'s ProjectionView> {
        None
    }

    pub(crate) fn clear_view(&self, _state: &mut ProjectionState, _plot: usize) {}

    pub(crate) fn navigation(&self, _plot: usize, _size: [f32; 2]) -> Option<ProjectionNavigation> {
        None
    }

    pub(crate) fn fit(
        &self,
        _sizes: &[[f32; 2]],
        _state: &mut ProjectionState,
        _warnings: &mut Vec<EvalWarning>,
    ) -> Result<BTreeMap<usize, GeoFit>> {
        Ok(BTreeMap::new())
    }

    pub(crate) fn backdrop(
        &self,
        _plot: usize,
        fit: &GeoFit,
        _warnings: &mut Vec<EvalWarning>,
    ) -> Result<Vec<SceneMark>> {
        match *fit {}
    }

    pub(crate) fn labels(
        &self,
        _plot: usize,
        fit: &GeoFit,
        _side: GuideSide,
        _slot: [f32; 4],
    ) -> Option<SceneGroup> {
        match *fit {}
    }
}

impl GeoFit {
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
    fit: &GeoFit,
    _points: Option<[Vec<f32>; 2]>,
) -> Result<Vec<Drawn>> {
    match *fit {}
}
