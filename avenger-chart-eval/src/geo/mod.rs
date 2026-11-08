//! Geo plots, which need the `geo` feature. Without it, a module of the same interface has no geo
//! plots, because the evaluator rejects a chart with one.

use avenger_chart::resolve::Resolved;
use avenger_guides::{GuideSide, PartExtent};
use avenger_style::ComputedStyles;
use avenger_typst_label::LabelEngine;

use crate::style::tree::StyleNodes;

#[cfg(feature = "geo")]
mod plots;
#[cfg(feature = "geo")]
pub(crate) use plots::{marks, GeoPlots};

#[cfg(not(feature = "geo"))]
mod none;
#[cfg(not(feature = "geo"))]
pub(crate) use none::{marks, GeoPlots};

/// What preparing the geo plots reads: the chart's models, its style nodes and their computed
/// styles, and the label engine.
#[cfg_attr(not(feature = "geo"), allow(dead_code))]
pub(crate) struct GeoInputs<'a> {
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) nodes: &'a StyleNodes,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) engine: &'a LabelEngine,
}

/// What the layout keeps for a geo plot: the aspect ratio of its leaf, or `None` for a leaf of
/// any shape, and the extent of the labels of its graticule on each side that has them.
#[derive(Clone, Debug, Default)]
pub(crate) struct GeoGuide {
    pub(crate) aspect: Option<f32>,
    pub(crate) labels: Vec<(GuideSide, PartExtent)>,
}

/// The rows of a mark at places that a geo plot's projection keeps, by their positions in the
/// layer's drawn rows, and the pixels of their places.
pub(crate) type Located = (Vec<u32>, [Vec<f32>; 2]);

/// Whether the evaluator draws geo plots, with the `geo` feature.
pub(crate) const SUPPORTED: bool = cfg!(feature = "geo");
