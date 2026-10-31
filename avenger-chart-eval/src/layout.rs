//! The lowering of a plot and its guides to an avenger-layout tree, the `Measure` that answers
//! each slab with the extent of its guide, and the drawing of each guide in its solved slab.

use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::{
    Direction, Gap, Layout, Leaf, Measure, Measurement, RegionKind, Side, Slab, SlabStack,
    Solution, Track,
};
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};

use crate::{
    guides::{Guides, LegendGuide, AXIS_PARTS},
    Theme,
};

/// The id of a node of a plot's layout: the plot, or a slab of one of its guides.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SlabId {
    Plot(usize),
    /// A part of an axis, by the axis and the index of the part in [`AXIS_PARTS`].
    Axis(usize, usize),
    Legend(usize),
    Title(usize),
    Attribution(usize),
}

fn layout_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
    }
}

/// The layout of a plot with its guides: the slabs of its axes nearest the plot, then a stack of
/// its legends and colorbars on each side that has some, then its attribution, and outermost
/// its title, which runs the whole side.
pub(crate) fn lower(guides: &Guides, aspect: Option<f32>, theme: &Theme) -> Layout<SlabId> {
    let mut plot = Leaf::new().id(SlabId::Plot(0));
    if let Some(ratio) = aspect {
        plot = plot.aspect(ratio);
    }
    for (index, axis) in guides.axes.iter().enumerate() {
        for part in 0..AXIS_PARTS.len() {
            let key = format!("axis {index} {part}");
            plot = plot.slab(
                layout_side(axis.side),
                Slab::new()
                    .id(SlabId::Axis(index, part))
                    .tag(key.as_str())
                    .content_key(key.as_str()),
            );
        }
    }
    for side in [
        GuideSide::Right,
        GuideSide::Bottom,
        GuideSide::Top,
        GuideSide::Left,
    ] {
        let mut slabs = Vec::new();
        let mut tracks = Vec::new();
        for (index, slot) in guides.legends.iter().enumerate() {
            if slot.side != side {
                continue;
            }
            let mut slab = Slab::new().id(SlabId::Legend(index));
            if let Some(length) = slot.max_length {
                slab = slab.max_length(length);
            }
            tracks.push(match slot.guide {
                LegendGuide::Legend(_) => Track::auto(),
                LegendGuide::Colorbar(_) => Track::fr(1.0),
            });
            slabs.push(slab);
        }
        if slabs.is_empty() {
            continue;
        }
        let mut stack = SlabStack::new(slabs).gap(Gap::new(theme.legend_gap));
        for (index, track) in tracks.into_iter().enumerate() {
            stack = stack.track(index, track);
        }
        plot = plot
            .slab(
                layout_side(side),
                Slab::new().min_width(theme.legend_offset),
            )
            .slab_stack(layout_side(side), stack);
    }
    if let Some(attribution) = &guides.attribution {
        let side = match guides.attribution_side {
            GuideSide::Top => Side::Top,
            _ => Side::Bottom,
        };
        plot = plot.slab(
            side,
            Slab::new()
                .id(SlabId::Attribution(0))
                .tag("attribution")
                .content_key(attribution.content_key()),
        );
    }
    if let Some(title) = &guides.title {
        plot = plot.slab(
            layout_side(title.side),
            Slab::new().id(SlabId::Title(0)).full(),
        );
    }
    plot.into()
}

/// The answers of a plot's guides to the layout.
pub(crate) struct Measurer<'a> {
    pub(crate) guides: &'a Guides,
}

impl Measurer<'_> {
    fn extent(&self, id: &SlabId, length: f32) -> Option<PartExtent> {
        let guides = self.guides;
        match id {
            SlabId::Axis(axis, part) => {
                Some(guides.axes[*axis].axis.extent(AXIS_PARTS[*part], length))
            }
            SlabId::Legend(index) => Some(guides.legends[*index].extent(length)),
            SlabId::Title(_) => guides.title.as_ref().map(|title| title.extent(length)),
            SlabId::Attribution(_) => guides
                .attribution
                .as_ref()
                .map(|attribution| attribution.extent(length)),
            SlabId::Plot(_) => None,
        }
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
        let along = if slot.side.is_vertical() {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        (direction == along).then(|| slot.max_content()).flatten()
    }
}

/// The guides of a plot, each drawn in its slab when the slab draws itself.
pub(crate) fn draw(guides: &Guides, solution: &Solution<SlabId>) -> Vec<SceneMark> {
    let mut ids: Vec<(SlabId, GuideSide)> = Vec::new();
    for (index, axis) in guides.axes.iter().enumerate() {
        for part in 0..AXIS_PARTS.len() {
            ids.push((SlabId::Axis(index, part), axis.side));
        }
    }
    for (index, slot) in guides.legends.iter().enumerate() {
        ids.push((SlabId::Legend(index), slot.side));
    }
    if guides.attribution.is_some() {
        ids.push((SlabId::Attribution(0), guides.attribution_side));
    }
    if let Some(title) = &guides.title {
        ids.push((SlabId::Title(0), title.side));
    }
    let mut marks = Vec::new();
    for (id, side) in ids {
        let Some(region) = solution.region(&id) else {
            continue;
        };
        if !matches!(
            region.kind,
            RegionKind::Slab {
                represented_by: None,
                ..
            }
        ) {
            continue;
        }
        let rect = region.content;
        let slot = [rect.x, rect.y, rect.width, rect.height];
        let length = if side.is_vertical() {
            rect.height
        } else {
            rect.width
        };
        let group = match &id {
            SlabId::Axis(axis, part) => {
                let axis = &guides.axes[*axis].axis;
                axis.part(AXIS_PARTS[*part], length)
                    .map(|group| SceneGroup {
                        origin: axis.origin(slot),
                        ..group
                    })
            }
            SlabId::Legend(index) => {
                let slot_guide = &guides.legends[*index];
                Some(SceneGroup {
                    origin: slot_guide.origin(slot),
                    ..slot_guide.group(length)
                })
            }
            SlabId::Title(_) => guides.title.as_ref().map(|title| SceneGroup {
                origin: title.origin(slot),
                ..title.part(length)
            }),
            SlabId::Attribution(_) => guides.attribution.as_ref().and_then(|attribution| {
                attribution.part(length).map(|group| SceneGroup {
                    origin: attribution.origin(slot),
                    ..group
                })
            }),
            SlabId::Plot(_) => None,
        };
        if let Some(group) = group {
            marks.push(group.into());
        }
    }
    marks
}
