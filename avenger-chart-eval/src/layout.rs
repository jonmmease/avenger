//! The layout of a chart of one plot: the lowering of the plot and its guides to an avenger-layout
//! tree, the `Measure` that answers each slab with the extent of its guide, and the drawing of each
//! guide in its slab.
//!
//! The plot is a `Leaf` with a slab for each part of each axis nearest the plot, then a stack of
//! the legends and colorbars of each side, then the attribution, and outermost the title, which
//! runs the whole side. A guide's computed `offset` widens its slab on the side of the plot, so the
//! guide stands that far from what is inside it, and an axis's offset widens the slab of its
//! ticks, which moves its labels and title with it.

use avenger_guides::{axis::AxisPart, GuideSide, PartExtent};
use avenger_layout::{
    Direction, Edges, Gap, Layout, Leaf, Measure, Measurement, Side, Size, Slab, SlabStack,
    Solution, SolveOptions, Track,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_style::{Computed, ComputedStyle, Property};

use crate::{
    guides::{Guides, LegendGuide},
    style::number as style_number,
};

/// The parts of an axis, from the plot outward, which stand in a slab each.
pub(crate) const AXIS_PARTS: [AxisPart; 3] = [AxisPart::Ticks, AxisPart::Labels, AxisPart::Title];

/// The id of a node of a chart's layout: the plot, or a slab of one of its guides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlabId {
    Plot,
    /// A part of an axis: the position of the axis in `Guides::axes`, and the position of the part
    /// in [`AXIS_PARTS`].
    Axis {
        axis: usize,
        part: usize,
    },
    /// A legend or a colorbar, by its position in `Guides::legends`.
    Legend(usize),
    Title,
    Attribution,
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// The plot's leaf with the slabs of its guides.
pub(crate) fn lower(guides: &Guides, plot: &ComputedStyle) -> Layout<SlabId> {
    let mut leaf = Leaf::new().id(SlabId::Plot);
    if let Some(width) = plot.number(Property::Width) {
        leaf = leaf.min_width(width).max_width(width);
    }
    if let Some(height) = plot.number(Property::Height) {
        leaf = leaf.min_height(height).max_height(height);
    }
    if let Computed::Ratio(ratio) = plot.get(Property::AspectRatio) {
        leaf = leaf.aspect(*ratio);
    }
    // The computed `contain-intrinsic-size` is the size that the plot counts at in a chart that
    // takes its natural size, which a host's size or the chart's size stretches.
    let intrinsic = [
        plot.number(Property::ContainIntrinsicWidth),
        plot.number(Property::ContainIntrinsicHeight),
    ];
    if intrinsic.iter().any(Option::is_some) {
        let [width, height] = intrinsic.map(|size| size.unwrap_or(0.0));
        leaf = leaf.max_content(Size::new(width, height));
    }
    for (axis, guide) in guides.axes.iter().enumerate() {
        for part in 0..AXIS_PARTS.len() {
            leaf = leaf.slab(
                layout_side(guide.side),
                Slab::new().id(SlabId::Axis { axis, part }),
            );
        }
    }
    let mut sides: Vec<(GuideSide, Vec<usize>)> = Vec::new();
    for (index, legend) in guides.legends.iter().enumerate() {
        match sides.iter_mut().find(|(side, _)| *side == legend.side) {
            Some((_, members)) => members.push(index),
            None => sides.push((legend.side, vec![index])),
        }
    }
    for (side, members) in sides {
        leaf = leaf.slab_stack(layout_side(side), legend_stack(guides, &members));
    }
    if let Some(attribution) = &guides.attribution {
        leaf = leaf.slab(
            layout_side(attribution.side),
            Slab::new().id(SlabId::Attribution),
        );
    }
    if let Some(title) = &guides.title {
        leaf = leaf.slab(
            layout_side(title.side),
            Slab::new().id(SlabId::Title).full(),
        );
    }
    leaf.into()
}

/// A stack of the legends `members` along their side, apart by the largest offset among them. A
/// legend takes the length that its entries need, and a colorbar shares the rest, up to its longest
/// length.
fn legend_stack(guides: &Guides, members: &[usize]) -> SlabStack<SlabId> {
    let mut slabs = Vec::with_capacity(members.len());
    let mut tracks = Vec::with_capacity(members.len());
    let mut gap: f32 = 0.0;
    for index in members {
        let legend = &guides.legends[*index];
        let mut slab = Slab::new().id(SlabId::Legend(*index));
        if let Some(length) = legend.max_length {
            slab = slab.max_length(length);
        }
        tracks.push(match legend.guide {
            LegendGuide::Legend(_) => Track::auto(),
            LegendGuide::Colorbar(_) => Track::fr(1.0),
        });
        slabs.push(slab);
        gap = gap.max(legend.offset);
    }
    let mut stack = SlabStack::new(slabs).gap(Gap::new(gap));
    for (index, track) in tracks.into_iter().enumerate() {
        stack = stack.track(index, track);
    }
    stack
}

/// The options of the solve: the size of each dimension that the host or the chart gives, and the
/// max-content size of the others, inside the chart's computed padding.
pub(crate) fn solve_options(size: [Option<f32>; 2], chart: &ComputedStyle) -> SolveOptions {
    let padding = |property| style_number(chart, property);
    SolveOptions {
        width: size[0],
        height: size[1],
        padding: Edges {
            top: padding(Property::PaddingTop),
            right: padding(Property::PaddingRight),
            bottom: padding(Property::PaddingBottom),
            left: padding(Property::PaddingLeft),
        },
        ..SolveOptions::default()
    }
}

/// The side of the guide of a slab, and its offset from what stands inside it.
fn placement(guides: &Guides, id: &SlabId) -> Option<(GuideSide, f32)> {
    Some(match id {
        SlabId::Axis { axis, part } => {
            let axis = &guides.axes[*axis];
            (axis.side, if *part == 0 { axis.offset } else { 0.0 })
        }
        SlabId::Legend(index) => {
            let legend = &guides.legends[*index];
            (legend.side, legend.offset)
        }
        SlabId::Title => {
            let title = guides.title.as_ref()?;
            (title.side, title.offset)
        }
        SlabId::Attribution => {
            let attribution = guides.attribution.as_ref()?;
            (attribution.side, attribution.offset)
        }
        SlabId::Plot => return None,
    })
}

/// The answers of a chart's guides to the layout.
pub(crate) struct Measurer<'a> {
    pub(crate) guides: &'a Guides,
}

impl Measurer<'_> {
    fn extent(&self, id: &SlabId, length: f32) -> Option<PartExtent> {
        let guides = self.guides;
        let extent = match id {
            SlabId::Axis { axis, part } => {
                guides.axes[*axis].axis.extent(AXIS_PARTS[*part], length)
            }
            SlabId::Legend(index) => match &guides.legends[*index].guide {
                LegendGuide::Legend(legend) => legend.extent(length),
                LegendGuide::Colorbar(colorbar) => colorbar.extent(length),
            },
            SlabId::Title => guides.title.as_ref()?.title.extent(length),
            SlabId::Attribution => guides.attribution.as_ref()?.attribution.extent(length),
            SlabId::Plot => return None,
        };
        let (_, offset) = placement(guides, id)?;
        Some(PartExtent {
            thickness: extent.thickness + offset,
            ..extent
        })
    }
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<SlabId> for Measurer<'_> {
    fn height(&mut self, id: &SlabId, width: f32) -> Option<Measurement> {
        self.extent(id, width).map(answer)
    }

    fn width(&mut self, id: &SlabId, height: f32) -> Option<Measurement> {
        self.extent(id, height).map(answer)
    }

    /// A legend's length with its entries on as few lines as its wrap allows, along its side.
    fn max_content(&mut self, id: &SlabId, direction: Direction) -> Option<f32> {
        let SlabId::Legend(index) = id else {
            return None;
        };
        let slot = &self.guides.legends[*index];
        let LegendGuide::Legend(legend) = &slot.guide else {
            return None;
        };
        let along = match slot.side.is_vertical() {
            true => Direction::Vertical,
            false => Direction::Horizontal,
        };
        (direction == along).then(|| legend.max_content())
    }
}

/// `slot` without the offset on its side nearest the plot.
fn past_offset(slot: [f32; 4], side: GuideSide, offset: f32) -> [f32; 4] {
    let [x, y, width, height] = slot;
    match side {
        GuideSide::Right => [x + offset, y, width - offset, height],
        GuideSide::Left => [x, y, width - offset, height],
        GuideSide::Bottom => [x, y + offset, width, height - offset],
        GuideSide::Top => [x, y, width, height - offset],
    }
}

/// The guides of the chart, each drawn in its slab of `solution`.
pub(crate) fn draw(guides: &Guides, solution: &Solution<SlabId>) -> Vec<SceneMark> {
    let mut ids: Vec<SlabId> = Vec::new();
    for axis in 0..guides.axes.len() {
        ids.extend((0..AXIS_PARTS.len()).map(|part| SlabId::Axis { axis, part }));
    }
    ids.extend((0..guides.legends.len()).map(SlabId::Legend));
    ids.extend([SlabId::Attribution, SlabId::Title]);
    let mut marks = Vec::new();
    for id in ids {
        let (Some(region), Some((side, offset))) = (solution.region(&id), placement(guides, &id))
        else {
            continue;
        };
        let rect = region.content;
        let slot = past_offset([rect.x, rect.y, rect.width, rect.height], side, offset);
        let length = match side.is_vertical() {
            true => slot[3],
            false => slot[2],
        };
        let group = match id {
            SlabId::Axis { axis, part } => {
                let axis = &guides.axes[axis].axis;
                (axis.part(AXIS_PARTS[part], length)).map(|group| SceneGroup {
                    origin: axis.origin(slot),
                    ..group
                })
            }
            SlabId::Legend(index) => Some(match &guides.legends[index].guide {
                LegendGuide::Legend(legend) => SceneGroup {
                    origin: legend.origin(slot),
                    ..legend.marks(length).group
                },
                LegendGuide::Colorbar(colorbar) => SceneGroup {
                    origin: colorbar.origin(slot),
                    ..colorbar.marks(length).group
                },
            }),
            SlabId::Title => guides.title.as_ref().and_then(|title| {
                let title = &title.title;
                (title.part(length)).map(|group| SceneGroup {
                    origin: title.origin(slot),
                    ..group
                })
            }),
            SlabId::Attribution => guides.attribution.as_ref().and_then(|attribution| {
                let attribution = &attribution.attribution;
                (attribution.part(length)).map(|group| SceneGroup {
                    origin: attribution.origin(slot),
                    ..group
                })
            }),
            SlabId::Plot => None,
        };
        marks.extend(group.map(SceneMark::from));
    }
    marks
}
