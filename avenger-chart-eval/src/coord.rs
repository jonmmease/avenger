//! The contract between the evaluator and the coordinate systems of its plots.
//!
//! A position scale maps data along one channel, and a plot's projection joins the positional
//! scales of the plot into places. The evaluator fits the domain of every position scale, and the
//! coordinate system of a plot owns the rest of its projection: the ranges of its positional
//! scales at a size, its aspect, its clipping, the guides that it places, and how it draws marks.
//! Each kind of [`CoordKind`] has one [`CoordSystem`], which prepares the plots of its kind
//! without a size and fits them at the sizes of an evaluation, and gives each plot a
//! [`FittedPlot`] at its size.

use std::{collections::BTreeMap, sync::Arc};

use arrow::array::RecordBatch;
use avenger_coords::PlotProjection;
use avenger_guides::{GuideSide, PartExtent};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_typst_label::LabelEngine;

use avenger_chart::{
    proto,
    resolve::{ScaleGroup, Tree},
    Chart, CoordKind,
};

use crate::{
    cartesian::CartesianPlots,
    encode::{Drawn, Projected, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord},
    fit::{Position, Reach},
    EvalError, Theme,
};

#[cfg(feature = "geo")]
use crate::geo::MapPlots;

type Result<T> = std::result::Result<T, EvalError>;

/// What the layout reads of the projection of a plot: the aspect ratio that the plot's leaf
/// keeps, if any, and the sides of the slabs that the projection draws beside the plot, such as
/// the labels of a map's graticule, each with its extent at every view.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectionGuide {
    pub(crate) aspect: Option<f32>,
    pub(crate) slabs: Vec<(GuideSide, PartExtent)>,
}

/// A unit of a chart, with its drawn rows and the reach of its marks.
// Only map plots read the units, and the theme and the label engine of the inputs.
#[cfg_attr(not(feature = "geo"), allow(dead_code))]
pub(crate) struct PlotUnit<'a> {
    pub(crate) rows: UnitRows<'a>,
    pub(crate) reach: Option<&'a [Reach]>,
    /// The plot that the unit draws into.
    pub(crate) plot: usize,
}

/// What a coordinate system prepares its plots from.
#[cfg_attr(not(feature = "geo"), allow(dead_code))]
pub(crate) struct SystemInputs<'a> {
    pub(crate) tree: &'a Tree<'a>,
    pub(crate) groups: &'a [ScaleGroup],
    /// The units of the chart, in tree order.
    pub(crate) units: &'a [PlotUnit<'a>],
    pub(crate) theme: &'a Theme,
    pub(crate) engine: &'a LabelEngine,
}

/// The plots of one kind of coordinate system in a chart, prepared without a size.
pub(crate) trait CoordSystem {
    /// Prepares the plots of the system's kind from `inputs`.
    fn prepare(inputs: &SystemInputs) -> Result<Self>
    where
        Self: Sized;

    /// The guide of the projection of the plot `plot`.
    fn guide(&self, plot: usize) -> ProjectionGuide;

    /// Fits the projection and the positional scales of each plot at its size in `sizes`, from
    /// the fitting scales of `positions`, by group.
    fn fit(
        &mut self,
        positions: &BTreeMap<usize, Position>,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()>;

    /// The plot `plot` fitted at its size at the last fit.
    fn fitted(&self, plot: usize) -> Option<Box<dyn FittedPlot + '_>>;
}

/// One plot of a coordinate system fitted at one size: its positional scales at that size, its
/// projection, and how it draws its marks.
pub(crate) trait FittedPlot {
    /// The positional scale of the position group `group` at the plot's size, when the plot
    /// draws through it.
    fn scale(&self, group: usize) -> Option<&ConfiguredScale>;

    /// Whether the plot clips its marks to its rectangle.
    fn clips(&self) -> bool;

    /// What the plot draws behind its data, before the grids of its axes.
    fn backdrop(&self) -> Result<Vec<SceneMark>> {
        Ok(Vec::new())
    }

    /// The rows of `table`, the drawn rows of `unit`, that the plot draws, and their positions
    /// when the projection places them: every row, which the marks place through the scales.
    fn locate(
        &self,
        _chart: &Chart,
        _unit: &proto::Unit,
        table: &RecordBatch,
    ) -> Result<(RecordBatch, Option<Projected>)> {
        Ok((table.clone(), None))
    }

    /// The scene marks of a unit of the plot.
    fn marks(&self, rows: &UnitRows) -> Result<Vec<Drawn>>;

    /// The slab of the projection on `side` of the plot, in the rectangle `slot`.
    fn slab(&self, _side: GuideSide, _slot: [f32; 4]) -> Option<SceneGroup> {
        None
    }

    /// The coordinate system of the plot in the evaluation.
    fn coord(&self) -> PlotCoord;

    /// The projection between the plot's pixels, from its top left, and the range positions of its
    /// two positional dimensions, at `size`, the plot's size, or `None` for a plot whose pixels
    /// map to no positions.
    fn projection(&self, size: [f32; 2]) -> Option<Arc<dyn PlotProjection>>;
}

/// The coordinate systems of a chart, one for each kind of its plots. The coordinate system of a
/// feature exists with the feature.
pub(crate) struct CoordSystems {
    /// The kind of each plot, by plot.
    kinds: Vec<CoordKind>,
    cartesian: CartesianPlots,
    #[cfg(feature = "geo")]
    map: MapPlots,
}

impl CoordSystems {
    /// Prepares the coordinate system of each kind of plot in `inputs`. A plot of a kind
    /// without a coordinate system fails.
    pub(crate) fn prepare(inputs: &SystemInputs) -> Result<Self> {
        let kinds: Vec<CoordKind> = inputs.tree.plots.iter().map(|plot| plot.kind).collect();
        for (plot, kind) in inputs.tree.plots.iter().zip(&kinds) {
            match kind {
                CoordKind::Cartesian => {}
                #[cfg(feature = "geo")]
                CoordKind::Geo => {}
                #[cfg(not(feature = "geo"))]
                CoordKind::Geo => {
                    return Err(EvalError::Evaluation(format!(
                        "the map plot `{}` needs the `geo` feature of avenger-chart-eval",
                        plot.path
                    )))
                }
                CoordKind::Polar => {
                    return Err(EvalError::Evaluation(format!(
                        "the evaluator draws no {} plots, and `{}` is one",
                        kind.name(),
                        plot.path
                    )))
                }
            }
        }
        Ok(Self {
            kinds,
            cartesian: CartesianPlots::prepare(inputs)?,
            #[cfg(feature = "geo")]
            map: MapPlots::prepare(inputs)?,
        })
    }

    /// The coordinate system of the plot `plot`.
    fn system(&self, plot: usize) -> Option<&dyn CoordSystem> {
        match self.kinds.get(plot)? {
            CoordKind::Cartesian => Some(&self.cartesian),
            #[cfg(feature = "geo")]
            CoordKind::Geo => Some(&self.map),
            #[cfg(not(feature = "geo"))]
            CoordKind::Geo => None,
            CoordKind::Polar => None,
        }
    }

    /// The guide of the projection of each plot, by plot.
    pub(crate) fn guides(&self) -> Vec<ProjectionGuide> {
        (0..self.kinds.len())
            .map(|plot| {
                self.system(plot)
                    .map(|system| system.guide(plot))
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Loads the tiles of the map plots through `resolver`, the image resolver of the host.
    #[cfg(feature = "geo")]
    pub(crate) fn set_image_resolver(
        &mut self,
        resolver: std::sync::Arc<dyn avenger_image::ImageResolver>,
    ) -> Result<()> {
        self.map.set_image_resolver(resolver)
    }

    /// Fits each coordinate system at the sizes of its plots in `sizes`.
    pub(crate) fn fit(
        &mut self,
        positions: &BTreeMap<usize, Position>,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
        #[cfg(feature = "geo")]
        self.map.fit(positions, sizes, warnings)?;
        self.cartesian.fit(positions, sizes, warnings)
    }

    /// The plot `plot` fitted at its size at the last fit.
    pub(crate) fn fitted(&self, plot: usize) -> Result<Box<dyn FittedPlot + '_>> {
        self.system(plot)
            .and_then(|system| system.fitted(plot))
            .ok_or_else(|| {
                EvalError::Evaluation("a plot has no fitted coordinate system".to_string())
            })
    }
}
