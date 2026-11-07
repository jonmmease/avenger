//! Map plots, which need the `geo` feature. Without it, a module of the same interface has no map
//! plots, because the evaluator rejects a chart with one.

use avenger_chart::resolve::Resolved;
use avenger_guides::{GuideSide, PartExtent};
use avenger_style::ComputedStyles;
use avenger_typst_label::LabelEngine;

use crate::style::tree::StyleNodes;

#[cfg(feature = "geo")]
mod maps;
#[cfg(feature = "geo")]
pub(crate) use maps::{marks, Maps, ProjectionState};

#[cfg(not(feature = "geo"))]
mod none;
#[cfg(not(feature = "geo"))]
pub(crate) use none::{marks, Maps, ProjectionState};

/// What preparing the map plots reads: the chart's models, its style nodes and their computed
/// styles, and the label engine.
#[cfg_attr(not(feature = "geo"), allow(dead_code))]
pub(crate) struct MapInputs<'a> {
    pub(crate) resolved: &'a Resolved<'a>,
    pub(crate) nodes: &'a StyleNodes,
    pub(crate) styles: &'a ComputedStyles,
    pub(crate) engine: &'a LabelEngine,
}

/// What the layout keeps for a map plot: the aspect ratio of its leaf, or `None` for a leaf of
/// any shape, and the extent of the labels of its graticule on each side that has them.
#[derive(Clone, Debug, Default)]
pub(crate) struct MapGuide {
    pub(crate) aspect: Option<f32>,
    pub(crate) labels: Vec<(GuideSide, PartExtent)>,
}

/// The rows of a mark at places that a map's projection keeps, by their positions in the layer's
/// drawn rows, and the pixels of their places.
pub(crate) type Located = (Vec<u32>, [Vec<f32>; 2]);

/// Whether the evaluator draws map plots, with the `geo` feature.
pub(crate) const SUPPORTED: bool = cfg!(feature = "geo");

/// The view of a projection, such as a pan or a zoom, by the kind of its coordinate system. A
/// feature adds the kind of its plots, so a match has an arm for the others.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ProjectionView {
    /// The projection that the map plots of the projection show.
    #[cfg(feature = "geo")]
    Geo(avenger_geo::Projection),
}

/// The navigation of a projection at the size of its plot, from the specs of its coordinate
/// system, by the kind of its coordinate system. A feature adds the kind of its plots, so a match
/// has an arm for the others.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ProjectionNavigation {
    /// How a drag moves a map: the pan mode of the spec, or the projection's own without one. The
    /// smallest and the largest scale of the view are the zoom limits of the spec times the scale
    /// of the fitted projection at the plot's size.
    #[cfg(feature = "geo")]
    Geo {
        pan_mode: avenger_geo::PanMode,
        min_scale: Option<f64>,
        max_scale: Option<f64>,
    },
}
