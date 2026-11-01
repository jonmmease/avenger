//! Cartesian plots. A Cartesian plot's projection is its rectangle: each position group of `x`
//! and `y` fits its scale, or its view, at the plot's length along the channel's side, and the
//! plot draws its marks through the scales of its groups, and clips them when a scale has an
//! explicit domain or a view.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use avenger_coords::{CartesianProjection, PlotProjection};
use avenger_scales::scales::{fitting::FittingScale, ConfiguredScale};

use avenger_chart::{
    proto::{coord, Channel},
    CoordKind,
};

use crate::{
    coord::{CoordSystem, FittedPlot, PlotUnit, ProjectionGuide, SystemInputs},
    encode::{self, Drawn, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord, ScaleViews},
    fit::Position,
    scale::eval_error,
    EvalError,
};

type Result<T> = std::result::Result<T, EvalError>;

/// The Cartesian plots of a chart, prepared without a size, with the scale of each of their
/// position groups at the sizes of the last fit.
pub(crate) struct CartesianPlots {
    /// The Cartesian plots, by their index in the plots of the chart.
    plots: BTreeMap<usize, CartesianPlot>,
    /// The position groups of the plots, by group.
    groups: BTreeMap<usize, CartesianGroup>,
    /// The scale of each position group at the sizes of the last fit, by group.
    fitted: BTreeMap<usize, ConfiguredScale>,
}

/// A Cartesian plot, prepared without a size.
struct CartesianPlot {
    path: String,
    /// The aspect ratio of its spec, which its leaf keeps.
    aspect: Option<f32>,
    /// Its position groups, in the order of the groups.
    groups: Vec<usize>,
    /// Whether it clips its marks at the last fit, because a scale has an explicit domain that
    /// can leave out data, or a view.
    clip: bool,
}

/// A position group of the Cartesian plots: its channel, `x` or `y`, and its first plot, whose
/// length along the channel's side it fits at. The layout gives the plots of a shared scale one
/// length along it.
struct CartesianGroup {
    channel: Channel,
    plot: usize,
}

/// A Cartesian plot at a size: its scales there, and whether it clips its marks.
struct FittedCartesian<'a> {
    plots: &'a CartesianPlots,
    plot: &'a CartesianPlot,
}

impl CoordSystem for CartesianPlots {
    fn prepare(inputs: &SystemInputs, _units: &[PlotUnit]) -> Result<Self> {
        let tree = inputs.tree;
        let mut plots = BTreeMap::new();
        for (index, plot) in tree.plots.iter().enumerate() {
            if plot.kind != CoordKind::Cartesian {
                continue;
            }
            let aspect = match plot.coord.and_then(|coord| coord.coord.as_ref()) {
                Some(coord::Coord::Cartesian(cartesian)) => cartesian.aspect_ratio,
                _ => None,
            };
            let plot = CartesianPlot {
                path: plot.path.clone(),
                aspect,
                groups: Vec::new(),
                clip: false,
            };
            plots.insert(index, plot);
        }
        let mut groups = BTreeMap::new();
        for (index, group) in inputs.groups.iter().enumerate() {
            let Some(first) = group.members.first().filter(|_| group.is_position()) else {
                continue;
            };
            let plot = tree.units[first.unit].plot;
            if !plots.contains_key(&plot) {
                continue;
            }
            // A shared scale draws in each plot of its members.
            let members: BTreeSet<usize> = (group.members.iter())
                .map(|member| tree.units[member.unit].plot)
                .collect();
            for member in members {
                if let Some(cartesian) = plots.get_mut(&member) {
                    cartesian.groups.push(index);
                }
            }
            let channel = group.channel;
            groups.insert(index, CartesianGroup { channel, plot });
        }
        Ok(Self {
            plots,
            groups,
            fitted: BTreeMap::new(),
        })
    }

    fn guide(&self, plot: usize) -> ProjectionGuide {
        ProjectionGuide {
            aspect: self.plots.get(&plot).and_then(|plot| plot.aspect),
            slabs: Vec::new(),
        }
    }

    fn fit(
        &mut self,
        positions: &BTreeMap<usize, Position>,
        views: &mut ScaleViews,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
        self.fitted.clear();
        for (index, group) in &self.groups {
            let length = group.length(sizes[group.plot]);
            let view = views.get_mut(index);
            let fitted = match &view {
                Some(view) => FittingScale::from(&view.scale).at(length),
                None => positions[index].fitting.at(length),
            }
            .map_err(|error| eval_error("the fit of a position scale", error))?;
            if let Some(view) = view {
                view.scale.clone_from(&fitted.scale);
            }
            if !fitted.marks_fit {
                warnings.push(EvaluationWarning {
                    path: self.plots[&group.plot].path.clone(),
                    message: format!(
                        "the plot is too small for the marks along `{}`, so they reach past its \
                         edges",
                        group.channel.as_str_name()
                    ),
                });
            }
            self.fitted.insert(*index, fitted.scale);
        }
        for plot in self.plots.values_mut() {
            plot.clip = (plot.groups.iter())
                .any(|group| positions[group].clip || views.contains_key(group));
        }
        Ok(())
    }

    fn fitted(&self, plot: usize) -> Option<Box<dyn FittedPlot + '_>> {
        let plot = self.plots.get(&plot)?;
        Some(Box::new(FittedCartesian { plots: self, plot }))
    }

    /// The fitting scale of `group` at the plot's length along the side of its channel.
    fn home_scale(
        &self,
        positions: &BTreeMap<usize, Position>,
        _plot: usize,
        group: usize,
        size: [f32; 2],
    ) -> Option<ConfiguredScale> {
        let length = self.groups.get(&group)?.length(size);
        Some(positions.get(&group)?.fitting.at(length).ok()?.scale)
    }
}

impl CartesianGroup {
    /// The length of a plot of `size` along the side of the group's channel.
    fn length(&self, size: [f32; 2]) -> f32 {
        match self.channel {
            Channel::Y => size[1],
            _ => size[0],
        }
    }
}

impl FittedPlot for FittedCartesian<'_> {
    fn scale(&self, group: usize) -> Option<&ConfiguredScale> {
        self.plot
            .groups
            .contains(&group)
            .then(|| self.plots.fitted.get(&group))?
    }

    fn clips(&self) -> bool {
        self.plot.clip
    }

    fn marks(&self, rows: &mut UnitRows) -> Result<Vec<Drawn>> {
        encode::marks(rows)
    }

    /// The scales of `x` and `y`, and the second scale of each, in the order of their groups.
    fn coord(&self) -> PlotCoord {
        let scales = |channel: Channel| {
            let mut scales = (self.plot.groups.iter())
                .filter(|group| self.plots.groups[*group].channel == channel)
                .filter_map(|group| self.plots.fitted.get(group).cloned());
            (scales.next(), scales.next())
        };
        let (x, second_x) = scales(Channel::X);
        let (y, second_y) = scales(Channel::Y);
        PlotCoord::Cartesian {
            x,
            y,
            second_x,
            second_y,
        }
    }

    /// The plot's rectangle, whose range positions are its pixels, as the scales of `x` and `y`
    /// take them.
    fn projection(&self, size: [f32; 2]) -> Option<Arc<dyn PlotProjection>> {
        Some(Arc::new(CartesianProjection::new(size)))
    }
}
