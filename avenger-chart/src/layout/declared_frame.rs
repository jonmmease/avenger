//! Chart-owned declared frame chrome, solved through the unified
//! `avenger_layout::Layout` API.
//!
//! The chart declares one content rectangle plus per-side chrome layers
//! (margin → bands → outer → inner, outside-in) and reads back a per-axis
//! view of the solved slabs. The chart owns this struct so coordination
//! retargets can mutate the declaration and re-solve; the layout crate only
//! ever sees a freshly built [`Layout`].

use avenger_layout::{ChromeLayer, Layout, Rect, Side, Size as LayoutSize, SolveFor, SolveOptions};

/// Per-axis sizing declaration (the chart's canvas/plot sizing policy).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum DeclaredAxisSizing {
    /// The envelope extent is given; the content takes what remains after
    /// the declared chrome, floored at [`DeclaredAxis::content_min`].
    EnvelopeFixed { extent: f32 },
    /// Both extents are given; the two margins absorb the slack (declared
    /// margins are ignored on this axis).
    EnvelopeAndContentFixed { extent: f32, content: f32 },
    /// The content extent is given; the envelope is the sum of all layers.
    ContentFixed { content: f32 },
}

/// One side's declared chrome, ordered outside-in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DeclaredSide {
    pub margin: f32,
    pub strips: Vec<f32>,
    pub legend: f32,
    pub guide: f32,
}

/// One axis of the declared frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeclaredAxis {
    pub sizing: DeclaredAxisSizing,
    pub leading: DeclaredSide,
    pub trailing: DeclaredSide,
    /// Minimum content extent; applies only in
    /// [`DeclaredAxisSizing::EnvelopeFixed`].
    pub content_min: f32,
}

/// The declared chart frame: both axes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeclaredFrame {
    pub horizontal: DeclaredAxis,
    pub vertical: DeclaredAxis,
}

/// One solved strip on an axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Slab {
    pub start: f32,
    pub size: f32,
}

/// One side's solved chrome on an axis.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct SolvedSideView {
    pub margin: Slab,
    pub strips: Vec<Slab>,
    pub legend: Slab,
    pub guide: Slab,
}

/// One solved axis: leading chrome, content, trailing chrome, extent.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SolvedAxisView {
    pub leading: SolvedSideView,
    pub content: Slab,
    pub trailing: SolvedSideView,
    pub extent: f32,
}

/// The per-axis view of a solved frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SolvedFrameView {
    pub horizontal: SolvedAxisView,
    pub vertical: SolvedAxisView,
}

impl DeclaredFrame {
    /// Solve the declared chrome through `avenger_layout::Layout` and
    /// project the per-axis view the chart's rect policies consume.
    pub(crate) fn solve(&self) -> SolvedFrameView {
        let axis_inputs = |axis: &DeclaredAxis| -> (SolveFor, Option<f32>, f32) {
            match axis.sizing {
                DeclaredAxisSizing::EnvelopeFixed { extent } => {
                    (SolveFor::Content, Some(extent), 0.0)
                }
                DeclaredAxisSizing::EnvelopeAndContentFixed { extent, content } => {
                    (SolveFor::Margins, Some(extent), content)
                }
                DeclaredAxisSizing::ContentFixed { content } => (SolveFor::Envelope, None, content),
            }
        };
        let (sizing_x, width, content_w) = axis_inputs(&self.horizontal);
        let (sizing_y, height, content_h) = axis_inputs(&self.vertical);

        let mut leaf: Layout<u8> = Layout::leaf(LayoutSize::new(content_w, content_h))
            .margin_edges(avenger_layout::Edges::new(
                self.vertical.leading.margin,
                self.horizontal.trailing.margin,
                self.vertical.trailing.margin,
                self.horizontal.leading.margin,
            ))
            .legend(Side::Top, self.vertical.leading.legend)
            .legend(Side::Right, self.horizontal.trailing.legend)
            .legend(Side::Bottom, self.vertical.trailing.legend)
            .legend(Side::Left, self.horizontal.leading.legend)
            .guide(Side::Top, self.vertical.leading.guide)
            .guide(Side::Right, self.horizontal.trailing.guide)
            .guide(Side::Bottom, self.vertical.trailing.guide)
            .guide(Side::Left, self.horizontal.leading.guide)
            .sizing_x(sizing_x)
            .sizing_y(sizing_y)
            .content_min(LayoutSize::new(
                self.horizontal.content_min,
                self.vertical.content_min,
            ));
        for (side, declared) in [
            (Side::Top, &self.vertical.leading),
            (Side::Right, &self.horizontal.trailing),
            (Side::Bottom, &self.vertical.trailing),
            (Side::Left, &self.horizontal.leading),
        ] {
            for strip in &declared.strips {
                leaf = leaf.strip(side, *strip);
            }
        }

        let solved = leaf
            .solve(&SolveOptions { width, height })
            .expect("a chromed leaf always solves");
        let region = solved.at_path(&[]).expect("the root region always exists");
        let content = region.content;

        let slab_1d = |rect: Rect, vertical: bool| -> Slab {
            if vertical {
                Slab {
                    start: rect.y,
                    size: rect.height,
                }
            } else {
                Slab {
                    start: rect.x,
                    size: rect.width,
                }
            }
        };
        let find = |layer: ChromeLayer, side: Side, strip_index: usize| -> Option<Slab> {
            let vertical = matches!(side, Side::Top | Side::Bottom);
            region
                .slabs
                .iter()
                .find(|slab| {
                    slab.layer == layer && slab.side == side && slab.strip_index == strip_index
                })
                .map(|slab| slab_1d(slab.rect, vertical))
        };
        // Absent (zero-size) slabs anchor at the adjacent content boundary;
        // every chart read of slab positions is guarded by an existence
        // flag, so the anchor only feeds zero-contribution arithmetic.
        let side_view = |side: Side, declared: &DeclaredSide| -> SolvedSideView {
            let boundary = match side {
                Side::Top => content.y,
                Side::Bottom => content.y + content.height,
                Side::Left => content.x,
                Side::Right => content.x + content.width,
            };
            let absent = Slab {
                start: boundary,
                size: 0.0,
            };
            SolvedSideView {
                margin: find(ChromeLayer::Margin, side, 0).unwrap_or(absent),
                strips: (0..declared.strips.len())
                    .map(|index| find(ChromeLayer::Strip, side, index).unwrap_or(absent))
                    .collect(),
                legend: find(ChromeLayer::Legend, side, 0).unwrap_or(absent),
                guide: find(ChromeLayer::Guide, side, 0).unwrap_or(absent),
            }
        };

        SolvedFrameView {
            horizontal: SolvedAxisView {
                leading: side_view(Side::Left, &self.horizontal.leading),
                content: Slab {
                    start: content.x,
                    size: content.width,
                },
                trailing: side_view(Side::Right, &self.horizontal.trailing),
                extent: solved.size.width,
            },
            vertical: SolvedAxisView {
                leading: side_view(Side::Top, &self.vertical.leading),
                content: Slab {
                    start: content.y,
                    size: content.height,
                },
                trailing: side_view(Side::Bottom, &self.vertical.trailing),
                extent: solved.size.height,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared_side(margin: f32, strips: &[f32], legend: f32, guide: f32) -> DeclaredSide {
        DeclaredSide {
            margin,
            strips: strips.to_vec(),
            legend,
            guide,
        }
    }

    fn chart(sizing_x: DeclaredAxisSizing, sizing_y: DeclaredAxisSizing) -> DeclaredFrame {
        DeclaredFrame {
            horizontal: DeclaredAxis {
                sizing: sizing_x,
                leading: declared_side(10.0, &[], 0.0, 35.0),
                trailing: declared_side(10.0, &[], 57.0, 7.0),
                content_min: 50.0,
            },
            vertical: DeclaredAxis {
                sizing: sizing_y,
                leading: declared_side(10.0, &[21.0, 13.0], 0.0, 16.0),
                trailing: declared_side(10.0, &[], 24.0, 12.0),
                content_min: 50.0,
            },
        }
    }

    /// Pinned frame-solver values across the three sizing modes (exact
    /// expected outputs, kept as literals).
    #[test]
    fn declared_frame_solves_all_modes() {
        let view = chart(
            DeclaredAxisSizing::EnvelopeFixed { extent: 400.0 },
            DeclaredAxisSizing::EnvelopeFixed { extent: 300.0 },
        )
        .solve();
        // Horizontal: 10 + 35 | content 281 | 7 + 57 + 10 = 400.
        assert_eq!(view.horizontal.content.start, 45.0);
        assert_eq!(view.horizontal.content.size, 281.0);
        assert_eq!(view.horizontal.extent, 400.0);
        assert_eq!(view.horizontal.leading.margin.start, 0.0);
        assert_eq!(view.horizontal.leading.guide.start, 10.0);
        assert_eq!(view.horizontal.trailing.guide.start, 326.0);
        assert_eq!(view.horizontal.trailing.legend.start, 333.0);
        // Vertical: 10 + 21 + 13 + 16 | content 194 | 12 + 24 + 10 = 300.
        assert_eq!(view.vertical.content.start, 60.0);
        assert_eq!(view.vertical.content.size, 194.0);
        assert_eq!(view.vertical.leading.strips[0].start, 10.0);
        assert_eq!(view.vertical.leading.strips[0].size, 21.0);
        assert_eq!(view.vertical.leading.strips[1].start, 31.0);
        assert_eq!(view.vertical.extent, 300.0);

        let plot_sized = chart(
            DeclaredAxisSizing::ContentFixed { content: 338.0 },
            DeclaredAxisSizing::ContentFixed { content: 203.0 },
        )
        .solve();
        assert_eq!(plot_sized.horizontal.content.start, 45.0);
        assert_eq!(plot_sized.horizontal.extent, 45.0 + 338.0 + 74.0);
        assert_eq!(plot_sized.vertical.extent, 60.0 + 203.0 + 46.0);

        let both = chart(
            DeclaredAxisSizing::EnvelopeAndContentFixed {
                extent: 500.0,
                content: 300.0,
            },
            DeclaredAxisSizing::ContentFixed { content: 203.0 },
        )
        .solve();
        // Slack 500 - (35 + 300 + 7 + 57) = 101, split 50.5 per margin.
        assert_eq!(both.horizontal.leading.margin.size, 50.5);
        assert_eq!(both.horizontal.content.start, 85.5);
        assert_eq!(both.horizontal.content.size, 300.0);
        assert_eq!(both.horizontal.extent, 500.0);

        let floored = chart(
            DeclaredAxisSizing::EnvelopeFixed { extent: 80.0 },
            DeclaredAxisSizing::ContentFixed { content: 203.0 },
        )
        .solve();
        // Chrome (119) exceeds the 80 envelope; the 50 floor wins.
        assert_eq!(floored.horizontal.content.size, 50.0);
        assert_eq!(floored.horizontal.extent, 169.0);
    }
}
