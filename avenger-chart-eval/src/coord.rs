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
    polar::PolarPlots,
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
pub(crate) struct PlotUnit<'a> {
    pub(crate) rows: UnitRows<'a>,
    pub(crate) reach: Option<&'a [Reach]>,
    /// The plot that the unit draws into.
    pub(crate) plot: usize,
}

/// What a coordinate system prepares its plots from: the chart, its scale groups, and the
/// fitting scale of each position group.
pub(crate) struct SystemInputs<'a> {
    pub(crate) chart: &'a Chart,
    pub(crate) tree: &'a Tree<'a>,
    pub(crate) groups: &'a [ScaleGroup],
    /// The fitting scale of each position group, by group.
    pub(crate) positions: &'a BTreeMap<usize, Position>,
    pub(crate) theme: &'a Theme,
    pub(crate) engine: &'a LabelEngine,
}

/// The plots of one kind of coordinate system in a chart, prepared without a size.
pub(crate) trait CoordSystem {
    /// Whether the position group `group` of `tree`, of the system's kind, wraps around its
    /// plots, so the end of its range is its start. The fit of its domain asks before the system
    /// prepares, because the system reads the fitting scales.
    fn wraps(_tree: &Tree, _group: &ScaleGroup) -> bool
    where
        Self: Sized,
    {
        false
    }

    /// Prepares the plots of the system's kind from `inputs` and the units of the chart, in tree
    /// order.
    fn prepare(inputs: &SystemInputs, units: &[PlotUnit]) -> Result<Self>
    where
        Self: Sized;

    /// Prepares again what follows the fitting scales of `inputs`, after the domain of a scale
    /// that reads a selection fits again.
    fn rescale(&mut self, _inputs: &SystemInputs) -> Result<()> {
        Ok(())
    }

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

    /// The scene marks of a unit of the plot, which can place the rows of `rows`.
    fn marks(&self, rows: &mut UnitRows) -> Result<Vec<Drawn>>;

    /// What the plot draws over its data, such as the axes inside a polar plot.
    fn overlay(&self) -> Vec<SceneMark> {
        Vec::new()
    }

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
    polar: PolarPlots,
    #[cfg(feature = "geo")]
    map: MapPlots,
}

impl CoordSystems {
    /// Whether the position group `group` of `tree` wraps around its plots, as the coordinate
    /// system of its plots says.
    pub(crate) fn wraps(tree: &Tree, group: &ScaleGroup) -> bool {
        let Some(member) = group.members.first() else {
            return false;
        };
        match tree.plots[tree.units[member.unit].plot].kind {
            CoordKind::Cartesian => CartesianPlots::wraps(tree, group),
            CoordKind::Polar => PolarPlots::wraps(tree, group),
            #[cfg(feature = "geo")]
            CoordKind::Geo => MapPlots::wraps(tree, group),
            #[cfg(not(feature = "geo"))]
            CoordKind::Geo => false,
        }
    }

    /// Prepares the coordinate system of each kind of plot in `inputs` from the units of the
    /// chart. A map plot without the `geo` feature fails.
    pub(crate) fn prepare(inputs: &SystemInputs, units: &[PlotUnit]) -> Result<Self> {
        let kinds: Vec<CoordKind> = inputs.tree.plots.iter().map(|plot| plot.kind).collect();
        #[cfg(not(feature = "geo"))]
        if let Some(plot) = (inputs.tree.plots.iter()).find(|plot| plot.kind == CoordKind::Geo) {
            return Err(EvalError::Evaluation(format!(
                "the map plot `{}` needs the `geo` feature of avenger-chart-eval",
                plot.path
            )));
        }
        Ok(Self {
            kinds,
            cartesian: CartesianPlots::prepare(inputs, units)?,
            polar: PolarPlots::prepare(inputs, units)?,
            #[cfg(feature = "geo")]
            map: MapPlots::prepare(inputs, units)?,
        })
    }

    /// The coordinate system of the plot `plot`.
    fn system(&self, plot: usize) -> Option<&dyn CoordSystem> {
        match self.kinds.get(plot)? {
            CoordKind::Cartesian => Some(&self.cartesian),
            CoordKind::Polar => Some(&self.polar),
            #[cfg(feature = "geo")]
            CoordKind::Geo => Some(&self.map),
            #[cfg(not(feature = "geo"))]
            CoordKind::Geo => None,
        }
    }

    /// Each coordinate system, in the order that they fit.
    fn systems_mut(&mut self) -> Vec<&mut dyn CoordSystem> {
        vec![
            #[cfg(feature = "geo")]
            &mut self.map,
            &mut self.polar,
            &mut self.cartesian,
        ]
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

    /// Prepares again what follows the fitting scales of `inputs` in each coordinate system.
    pub(crate) fn rescale(&mut self, inputs: &SystemInputs) -> Result<()> {
        for system in self.systems_mut() {
            system.rescale(inputs)?;
        }
        Ok(())
    }

    /// Fits each coordinate system at the sizes of its plots in `sizes`.
    pub(crate) fn fit(
        &mut self,
        positions: &BTreeMap<usize, Position>,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
        for system in self.systems_mut() {
            system.fit(positions, sizes, warnings)?;
        }
        Ok(())
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
