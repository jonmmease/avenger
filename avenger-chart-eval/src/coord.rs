//! The contract between the evaluator and the coordinate systems of its plots.
//!
//! A position scale maps data along one channel, and a plot's projection joins the positional
//! scales of the plot into places. The evaluator fits the domain of every position scale, and the
//! coordinate system of a plot owns the rest of its projection: the ranges of its positional
//! scales at a size, its aspect, its clipping, and how it draws marks. Each kind of
//! [`CoordKind`] has one [`CoordSystem`], which prepares the plots of its kind without a size and
//! fits them at the sizes of an evaluation, and gives each plot a [`FittedPlot`] at its size.

use std::{collections::BTreeMap, sync::Arc};

use avenger_coords::PlotProjection;
use avenger_scales::scales::ConfiguredScale;

use avenger_chart::{
    resolve::{ScaleGroup, UnitRef},
    CoordKind,
};

use crate::{
    cartesian::CartesianPlots,
    encode::{Drawn, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord},
    fit::Position,
    EvalError,
};

type Result<T> = std::result::Result<T, EvalError>;

/// What the layout reads of the projection of a plot: the aspect ratio that the plot's leaf
/// keeps, if any.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectionGuide {
    pub(crate) aspect: Option<f32>,
}

/// What a coordinate system prepares its plots from.
pub(crate) struct SystemInputs<'a> {
    pub(crate) units: &'a [UnitRef<'a>],
    pub(crate) groups: &'a [ScaleGroup],
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

    /// The scene marks of a unit of the plot.
    fn marks(&self, rows: &UnitRows) -> Result<Vec<Drawn>>;

    /// The coordinate system of the plot in the evaluation.
    fn coord(&self) -> PlotCoord;

    /// The projection between the plot's pixels, from its top left, and the range positions of its
    /// two positional dimensions, at `size`, the plot's size, or `None` for a plot whose pixels
    /// map to no positions.
    fn projection(&self, size: [f32; 2]) -> Option<Arc<dyn PlotProjection>>;
}

/// The coordinate systems of a chart, one for each kind of its plots.
pub(crate) struct CoordSystems {
    /// The kind of each plot, by plot.
    kinds: Vec<CoordKind>,
    cartesian: CartesianPlots,
}

impl CoordSystems {
    /// Prepares the coordinate system of each kind of plot in `inputs`. A plot of a kind
    /// without a coordinate system fails.
    pub(crate) fn prepare(inputs: &SystemInputs) -> Result<Self> {
        let mut kinds = Vec::new();
        for unit in inputs.units {
            if unit.plot == kinds.len() {
                kinds.push(CoordKind::of(unit.unit.coord.as_ref()));
            }
        }
        if let Some(kind) = (kinds.iter()).find(|kind| **kind != CoordKind::Cartesian) {
            return Err(EvalError::Evaluation(format!(
                "the evaluator draws no {} plots",
                kind.name()
            )));
        }
        Ok(Self {
            kinds,
            cartesian: CartesianPlots::prepare(inputs)?,
        })
    }

    /// The coordinate system of the plot `plot`.
    fn system(&self, plot: usize) -> Option<&dyn CoordSystem> {
        match self.kinds.get(plot)? {
            CoordKind::Cartesian => Some(&self.cartesian),
            CoordKind::Polar | CoordKind::Geo => None,
        }
    }

    /// The guide of the projection of the plot `plot`.
    pub(crate) fn guide(&self, plot: usize) -> ProjectionGuide {
        self.system(plot)
            .map(|system| system.guide(plot))
            .unwrap_or_default()
    }

    /// Fits each coordinate system at the sizes of its plots in `sizes`.
    pub(crate) fn fit(
        &mut self,
        positions: &BTreeMap<usize, Position>,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
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
