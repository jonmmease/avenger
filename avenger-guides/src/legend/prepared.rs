//! Symbol and line legends prepared once, which arrange their entries at any length.

use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_typst_label::LabelEngine;

use super::{
    arrange::{arrange, Arrangement},
    compute_encoding_length, make_background, make_title, title_group, GuideLegendItem,
    GuideLegendOutput, LegendConfig, LegendSwatches, LegendWrap, Title,
};
use crate::{error::AvengerGuidesError, BackgroundStyle, GuideSide, PartExtent};

/// A symbol or line legend prepared from a configuration and a text engine, without a length.
///
/// It sizes its title and every entry once. A call then arranges the entries at its length in
/// the configuration's [`LegendLayout`](super::LegendLayout), without a layout solver, and its
/// answer states exactly the lengths over which the arrangement holds. The legend draws its
/// title above its entries on every side, inside its background.
#[derive(Debug)]
pub struct Legend {
    side: GuideSide,
    wrap: LegendWrap,
    /// The space between entries across and down the legend.
    gaps: [f32; 2],
    background: BackgroundStyle,
    title: Option<Title>,
    entries: Vec<Entry>,
}

#[derive(Debug)]
struct Entry {
    /// The entry's hit rect, swatch and label, at the origin.
    group: SceneGroup,
    label: String,
    /// The entry's box, `[width, height]`, from its group's origin.
    size: [f32; 2],
    /// Where the group's origin lies past its box's top left corner.
    offset: [f32; 2],
}

/// Where everything lies at one length, in the legend's frame with its top left corner at 0.
struct Placed {
    arrangement: Arrangement,
    /// The entries' top left corner.
    entries_origin: [f32; 2],
    /// The background's size.
    size: [f32; 2],
    extent: PartExtent,
}

impl Legend {
    /// A legend of `config`'s entries. It fails when the labels and swatch encodings are
    /// arrays of different lengths, or when the title does not typeset.
    pub fn new(config: &LegendConfig, engine: &LabelEngine) -> Result<Self, AvengerGuidesError> {
        let style = &config.style;
        let mut lengths = vec![config.text.len()];
        match &config.swatches {
            LegendSwatches::Symbols(symbols) => lengths.extend(symbols.lengths()),
            LegendSwatches::Lines(lines) => lengths.extend(lines.lengths()),
        }
        let len = compute_encoding_length(&lengths)?;
        let texts = config.text.as_vec(len, None);
        let entries = match &config.swatches {
            LegendSwatches::Symbols(symbols) => symbols
                .entries(&texts, config.text_padding, style, engine)
                .into_iter()
                .map(|group| {
                    // Rows step by their hit rects' heights, rounded as the rows' starts are.
                    let [right, height] = hit_extent(&group);
                    (group, [right.ceil(), height.round()], [0.0, 0.0])
                })
                .collect::<Vec<_>>(),
            LegendSwatches::Lines(lines) => {
                let (row_height, groups) =
                    lines.entries(&texts, config.text_padding, style, engine);
                // Every row is as tall as the tallest label, with the line at its middle.
                groups
                    .into_iter()
                    .map(|group| {
                        let [right, _] = hit_extent(&group);
                        let size = [right.ceil(), row_height.round()];
                        (group, size, [0.0, (row_height / 2.0).round()])
                    })
                    .collect()
            }
        };
        let entries = entries
            .into_iter()
            .zip(texts)
            .map(|((group, size, offset), label)| Entry {
                group,
                label,
                size,
                offset,
            })
            .collect();
        let title = config
            .title
            .as_deref()
            .map(|title| make_title(title, &style.title_font(), engine))
            .transpose()?;
        Ok(Self {
            side: config.layout.side,
            wrap: config.layout.wrap,
            gaps: [style.column_gap, style.row_gap],
            background: style.background.clone(),
            title,
            entries,
        })
    }

    /// The legend's thickness and overhang at `length`, and the lengths where they hold.
    ///
    /// The range is exact: the entries keep their places at every length in it, and move just
    /// past either end. A legend whose entries pass the length, as a fixed column count's do
    /// beside a short plot, reports the excess as its end overhang, at `length` only.
    pub fn extent(&self, length: f32) -> PartExtent {
        self.placed(length).extent
    }

    /// The legend's max-content length along its side: its length with no limit, with its
    /// entries on as few lines as its wrap allows. It is the shortest length at which the
    /// legend keeps that arrangement, and the max-content length that a layout asks of the
    /// legend's slab.
    pub fn max_content(&self) -> f32 {
        let [width, height] = self.placed(f32::INFINITY).size;
        if self.side.is_vertical() {
            height
        } else {
            width
        }
    }

    /// The origin for the legend in the slab with rectangle `slot`, `[x, y, width, height]`: the
    /// legend's inner edge, nearest the plot, lies on the slab's inner edge, at its start.
    pub fn origin(&self, slot: [f32; 4]) -> [f32; 2] {
        let length = if self.side.is_vertical() {
            slot[3]
        } else {
            slot[2]
        };
        self.side.box_origin(slot, self.extent(length).thickness)
    }

    /// The legend at `length`, with its background, its title and its entries in the
    /// arrangement that [`Legend::extent`] reports for `length`, and each entry's scene paths.
    pub fn marks(&self, length: f32) -> GuideLegendOutput {
        let placed = self.placed(length);
        let padding = self.background.padding;
        let mut marks = vec![make_background(
            &self.background,
            placed.size[0],
            placed.size[1],
        )];
        if let Some(title) = &self.title {
            marks.push(title_group(title, padding));
        }
        let mut items = Vec::with_capacity(self.entries.len());
        for (index, (entry, position)) in self
            .entries
            .iter()
            .zip(&placed.arrangement.positions)
            .enumerate()
        {
            let [x, y] = self.xy(*position);
            let mut group = entry.group.clone();
            group.origin = [
                placed.entries_origin[0] + x + entry.offset[0],
                placed.entries_origin[1] + y + entry.offset[1],
            ];
            let group_path = vec![marks.len()];
            items.push(GuideLegendItem {
                index,
                label: entry.label.clone(),
                hit_rect_path: vec![group_path[0], 0],
                group_path,
            });
            marks.push(SceneMark::Group(group));
        }
        GuideLegendOutput {
            group: SceneGroup {
                marks,
                ..Default::default()
            },
            items,
        }
    }

    /// A position along and across the side as `[x, y]`.
    fn xy(&self, [along, across]: [f32; 2]) -> [f32; 2] {
        if self.side.is_vertical() {
            [across, along]
        } else {
            [along, across]
        }
    }

    fn placed(&self, length: f32) -> Placed {
        let padding = self.background.padding;
        let (title_width, title_block) = self
            .title
            .as_ref()
            .map_or((0.0, 0.0), |title| (title.width, title.block));
        let vertical = self.side.is_vertical();
        // Along a top or bottom legend's lines, entries step by their widths and the column
        // gap. Down a left or right legend's lines, by their heights and the row gap.
        let sizes: Vec<[f32; 2]> = self
            .entries
            .iter()
            .map(|entry| {
                let [width, height] = entry.size;
                if vertical {
                    [height, width]
                } else {
                    [width, height]
                }
            })
            .collect();
        let gaps = if vertical {
            [self.gaps[1], self.gaps[0]]
        } else {
            self.gaps
        };
        // The title stands above the entries: across them on a top or bottom legend, and
        // along them beside the plot.
        let before = if vertical { title_block } else { 0.0 };
        let entries_length = length - 2.0 * padding - before;
        let arrangement = arrange(&sizes, gaps, self.wrap, vertical, entries_length);
        let [along, across] = arrangement.size;
        let (content_along, content_across) = if vertical {
            (title_block + along, across.max(title_width))
        } else {
            (along.max(title_width), title_block + across)
        };
        let background_along = content_along + 2.0 * padding;
        let thickness = content_across + 2.0 * padding;
        let shift = 2.0 * padding + before;
        let entries_holds = arrangement.holds.shifted(shift, length);
        let overhang = (background_along - length).max(0.0);
        let holds = if overhang > 0.0 {
            length..=length
        } else if vertical {
            entries_holds
        } else {
            // A title wider than the entries also has to fit.
            entries_holds.start().max(title_width + 2.0 * padding)..=*entries_holds.end()
        };
        let size = if vertical {
            [thickness, background_along]
        } else {
            [background_along, thickness]
        };
        Placed {
            entries_origin: [padding, padding + title_block],
            arrangement,
            size,
            extent: PartExtent {
                thickness,
                overhang: [0.0, overhang],
                holds,
            },
        }
    }
}

/// The right edge and the height of an entry group's hit rect, its first mark.
fn hit_extent(group: &SceneGroup) -> [f32; 2] {
    match group.marks.first() {
        Some(SceneMark::Rect(rect)) => {
            let value = |values: &avenger_common::value::ScalarOrArray<f32>| {
                values.as_vec(1, None).first().copied().unwrap_or(0.0)
            };
            let x = value(&rect.x);
            let width = rect.width.as_ref().map_or(0.0, value);
            let height = rect.height.as_ref().map_or(0.0, value);
            [x + width, height]
        }
        _ => [0.0, 0.0],
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use avenger_color::ColorOrGradient;
    use avenger_common::value::ScalarOrArray;

    use super::*;
    use crate::legend::{LegendLayout, LineSwatches, SymbolSwatches};

    const SIDES: [GuideSide; 4] = [
        GuideSide::Top,
        GuideSide::Bottom,
        GuideSide::Left,
        GuideSide::Right,
    ];

    fn wraps() -> [LegendWrap; 3] {
        [
            LegendWrap::Flow,
            LegendWrap::Grid { columns: None },
            LegendWrap::Grid {
                columns: NonZeroUsize::new(2),
            },
        ]
    }

    fn legend(side: GuideSide, wrap: LegendWrap, title: Option<&str>, lines: bool) -> Legend {
        let text: Vec<String> = ["alpha", "beta", "a much longer gamma", "delta", "ε"]
            .map(String::from)
            .into();
        let colors =
            ScalarOrArray::new_array(vec![ColorOrGradient::Color([0.2, 0.4, 0.8, 1.0]); 5]);
        let swatches = if lines {
            LegendSwatches::Lines(LineSwatches {
                stroke: colors,
                ..Default::default()
            })
        } else {
            LegendSwatches::Symbols(SymbolSwatches {
                fill: colors,
                ..Default::default()
            })
        };
        Legend::new(
            &LegendConfig {
                title: title.map(String::from),
                text: ScalarOrArray::new_array(text),
                swatches,
                layout: LegendLayout { side, wrap },
                ..Default::default()
            },
            &crate::test_engine(),
        )
        .unwrap()
    }

    /// The background's size, `[width, height]`.
    fn background(output: &GuideLegendOutput) -> [f32; 2] {
        let SceneMark::Rect(rect) = &output.group.marks[0] else {
            panic!("the background comes first");
        };
        let value =
            |values: &Option<ScalarOrArray<f32>>| values.as_ref().unwrap().as_vec(1, None)[0];
        [value(&rect.width), value(&rect.height)]
    }

    /// Each entry group's origin.
    fn origins(output: &GuideLegendOutput) -> Vec<[f32; 2]> {
        output
            .items
            .iter()
            .map(|item| match &output.group.marks[item.group_path[0]] {
                SceneMark::Group(group) => group.origin,
                _ => panic!("an entry group"),
            })
            .collect()
    }

    /// The answer is the size of the drawn background: its thickness across the side, and how
    /// far it passes the length along the side. The background rounds to whole pixels.
    #[test]
    fn extents_equal_the_drawn_background() {
        for side in SIDES {
            for wrap in wraps() {
                for (title, lines) in [(None, false), (Some("A legend title"), true)] {
                    let legend = legend(side, wrap, title, lines);
                    for length in (30..400).step_by(17).map(|length| length as f32) {
                        let extent = legend.extent(length);
                        let [width, height] = background(&legend.marks(length));
                        let (along, across) = if side.is_vertical() {
                            (height, width)
                        } else {
                            (width, height)
                        };
                        let case = format!("{side:?} {wrap:?} {title:?} {length}");
                        assert!((extent.thickness - across).abs() <= 0.5, "{case}");
                        assert!(
                            (extent.overhang[1] - (along - length).max(0.0)).abs() <= 0.5,
                            "{case}"
                        );
                        assert_eq!(extent.overhang[0], 0.0, "{case}");
                    }
                }
            }
        }
    }

    /// Every length in a range draws the entries at the same places, and the answer is the same.
    #[test]
    fn every_length_in_holds_keeps_the_entries_in_place() {
        for side in SIDES {
            for wrap in wraps() {
                let legend = legend(side, wrap, Some("Title"), false);
                for length in (30..400).step_by(23).map(|length| length as f32) {
                    let extent = legend.extent(length);
                    let (low, high) = (*extent.holds.start(), *extent.holds.end());
                    for other in [low, high.min(2_000.0), (low + high.min(2_000.0)) / 2.0] {
                        assert_eq!(
                            origins(&legend.marks(other)),
                            origins(&legend.marks(length)),
                            "{side:?} {wrap:?} {length} at {other}"
                        );
                        let at_other = legend.extent(other);
                        assert_eq!(at_other.thickness, extent.thickness);
                        assert_eq!(at_other.overhang, extent.overhang);
                    }
                }
            }
        }
    }

    /// A side legend with a fixed column count keeps its columns at every length, and its
    /// entries past a short plot drop below it as end overhang.
    #[test]
    fn fixed_columns_drop_their_extra_entries_past_the_end() {
        let wrap = LegendWrap::Grid {
            columns: NonZeroUsize::new(1),
        };
        let legend = legend(GuideSide::Right, wrap, None, false);
        let tall = legend.extent(1_000.0);
        assert_eq!(tall.overhang, [0.0, 0.0]);
        let short = legend.extent(40.0);
        assert_eq!(short.thickness, tall.thickness);
        assert!(short.overhang[1] > 0.0);
        assert_eq!(short.holds, 40.0..=40.0);
        assert_eq!(
            origins(&legend.marks(40.0)),
            origins(&legend.marks(1_000.0))
        );
        // The open grid adds columns instead.
        let open = super::super::LegendWrap::Grid { columns: None };
        let legend = super::tests::legend(GuideSide::Right, open, None, false);
        assert!(legend.extent(40.0).thickness > legend.extent(1_000.0).thickness);
    }

    /// At its max-content length, a legend fits with nothing changing at any longer length, and
    /// one pixel shorter it takes another line or passes the length.
    #[test]
    fn max_content_is_the_shortest_length_of_the_fewest_lines() {
        for side in SIDES {
            for wrap in wraps() {
                for title in [None, Some("A legend title")] {
                    let legend = legend(side, wrap, title, false);
                    let length = legend.max_content();
                    let at = legend.extent(length);
                    let case = format!("{side:?} {wrap:?} {title:?}");
                    assert_eq!(at.overhang, [0.0, 0.0], "{case}");
                    assert!(at.holds.contains(&length), "{case}");
                    assert_eq!(*at.holds.end(), f32::INFINITY, "{case}");
                    let shorter = legend.extent(length - 1.0);
                    assert!(
                        shorter.overhang[1] > 0.0 || shorter.thickness > at.thickness,
                        "{case}"
                    );
                }
            }
        }
    }

    #[test]
    fn item_paths_resolve_to_each_entrys_group_and_hit_rect() {
        let output = legend(GuideSide::Bottom, LegendWrap::Flow, Some("Title"), true).marks(150.0);
        assert_eq!(output.items.len(), 5);
        for item in &output.items {
            let SceneMark::Group(group) = &output.group.marks[item.hit_rect_path[0]] else {
                panic!("an entry group");
            };
            assert_eq!(item.group_path, vec![item.hit_rect_path[0]]);
            assert!(matches!(
                group.marks[item.hit_rect_path[1]],
                SceneMark::Rect(_)
            ));
        }
    }

    /// The legend's inner edge lies on its slab's inner edge on every side.
    #[test]
    fn the_origin_puts_the_legend_against_the_plot() {
        let slot = [100.0, 50.0, 300.0, 200.0];
        for side in SIDES {
            let legend = legend(side, LegendWrap::Flow, None, false);
            let length = if side.is_vertical() { slot[3] } else { slot[2] };
            let thickness = legend.extent(length).thickness;
            let origin = legend.origin(slot);
            let expected = match side {
                GuideSide::Bottom | GuideSide::Right => [100.0, 50.0],
                GuideSide::Top => [100.0, 250.0 - thickness],
                GuideSide::Left => [400.0 - thickness, 50.0],
            };
            assert_eq!(origin, expected, "{side:?}");
        }
    }
}
