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
    pub bands: Vec<f32>,
    pub outer: f32,
    pub inner: f32,
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
    pub bands: Vec<Slab>,
    pub outer: Slab,
    pub inner: Slab,
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
            .outer(Side::Top, self.vertical.leading.outer)
            .outer(Side::Right, self.horizontal.trailing.outer)
            .outer(Side::Bottom, self.vertical.trailing.outer)
            .outer(Side::Left, self.horizontal.leading.outer)
            .inner(Side::Top, self.vertical.leading.inner)
            .inner(Side::Right, self.horizontal.trailing.inner)
            .inner(Side::Bottom, self.vertical.trailing.inner)
            .inner(Side::Left, self.horizontal.leading.inner)
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
            for band in &declared.bands {
                leaf = leaf.band(side, *band);
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
        let find = |layer: ChromeLayer, side: Side, band_index: usize| -> Option<Slab> {
            let vertical = matches!(side, Side::Top | Side::Bottom);
            region
                .slabs
                .iter()
                .find(|slab| {
                    slab.layer == layer && slab.side == side && slab.band_index == band_index
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
                bands: (0..declared.bands.len())
                    .map(|index| find(ChromeLayer::Band, side, index).unwrap_or(absent))
                    .collect(),
                outer: find(ChromeLayer::Outer, side, 0).unwrap_or(absent),
                inner: find(ChromeLayer::Inner, side, 0).unwrap_or(absent),
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
    use avenger_layout::{Frame, FrameAxis, FrameAxisSizing, FrameSide};

    fn declared_side(margin: f32, bands: &[f32], outer: f32, inner: f32) -> DeclaredSide {
        DeclaredSide {
            margin,
            bands: bands.to_vec(),
            outer,
            inner,
        }
    }

    fn frame_side(side: &DeclaredSide) -> FrameSide {
        FrameSide {
            margin: side.margin,
            bands: side.bands.clone(),
            outer: side.outer,
            inner: side.inner,
        }
    }

    fn old_sizing(sizing: DeclaredAxisSizing) -> FrameAxisSizing {
        match sizing {
            DeclaredAxisSizing::EnvelopeFixed { extent } => {
                FrameAxisSizing::EnvelopeFixed { extent }
            }
            DeclaredAxisSizing::EnvelopeAndContentFixed { extent, content } => {
                FrameAxisSizing::EnvelopeAndContentFixed { extent, content }
            }
            DeclaredAxisSizing::ContentFixed { content } => {
                FrameAxisSizing::ContentFixed { content }
            }
        }
    }

    /// The unified-API solve reproduces the legacy frame solver exactly for
    /// every present slab, the content, and the extent.
    fn assert_parity(declared: &DeclaredFrame) {
        let view = declared.solve();
        let legacy = Frame {
            horizontal: FrameAxis {
                sizing: old_sizing(declared.horizontal.sizing),
                leading: frame_side(&declared.horizontal.leading),
                trailing: frame_side(&declared.horizontal.trailing),
                content_min: declared.horizontal.content_min,
            },
            vertical: FrameAxis {
                sizing: old_sizing(declared.vertical.sizing),
                leading: frame_side(&declared.vertical.leading),
                trailing: frame_side(&declared.vertical.trailing),
                content_min: declared.vertical.content_min,
            },
        }
        .solve();

        for (axis_view, axis_legacy) in [
            (&view.horizontal, &legacy.horizontal),
            (&view.vertical, &legacy.vertical),
        ] {
            assert_eq!(axis_view.extent, axis_legacy.extent);
            assert_eq!(axis_view.content.start, axis_legacy.content.start);
            assert_eq!(axis_view.content.size, axis_legacy.content.size);
            for (side_view, side_legacy) in [
                (&axis_view.leading, &axis_legacy.leading),
                (&axis_view.trailing, &axis_legacy.trailing),
            ] {
                for (slab_view, slab_legacy) in [
                    (&side_view.margin, &side_legacy.margin),
                    (&side_view.outer, &side_legacy.outer),
                    (&side_view.inner, &side_legacy.inner),
                ] {
                    assert_eq!(slab_view.size, slab_legacy.size);
                    if slab_legacy.size > 0.0 {
                        assert_eq!(slab_view.start, slab_legacy.start);
                    }
                }
                assert_eq!(side_view.bands.len(), side_legacy.bands.len());
                for (band_view, band_legacy) in side_view.bands.iter().zip(side_legacy.bands.iter())
                {
                    assert_eq!(band_view.size, band_legacy.size);
                    if band_legacy.size > 0.0 {
                        assert_eq!(band_view.start, band_legacy.start);
                    }
                }
            }
        }
    }

    #[test]
    fn declared_frame_matches_legacy_solver_in_all_modes() {
        let chart = DeclaredFrame {
            horizontal: DeclaredAxis {
                sizing: DeclaredAxisSizing::EnvelopeFixed { extent: 400.0 },
                leading: declared_side(10.0, &[], 0.0, 35.0),
                trailing: declared_side(10.0, &[], 57.0, 7.0),
                content_min: 50.0,
            },
            vertical: DeclaredAxis {
                sizing: DeclaredAxisSizing::EnvelopeFixed { extent: 300.0 },
                leading: declared_side(10.0, &[21.0, 13.0], 0.0, 16.0),
                trailing: declared_side(10.0, &[], 24.0, 12.0),
                content_min: 50.0,
            },
        };
        assert_parity(&chart);

        let mut plot_sized = chart.clone();
        plot_sized.horizontal.sizing = DeclaredAxisSizing::ContentFixed { content: 338.0 };
        plot_sized.vertical.sizing = DeclaredAxisSizing::ContentFixed { content: 203.0 };
        assert_parity(&plot_sized);

        let mut both = chart.clone();
        both.horizontal.sizing = DeclaredAxisSizing::EnvelopeAndContentFixed {
            extent: 500.0,
            content: 300.0,
        };
        both.vertical.sizing = DeclaredAxisSizing::EnvelopeAndContentFixed {
            extent: 400.0,
            content: 200.0,
        };
        assert_parity(&both);

        let mut floored = chart.clone();
        floored.horizontal.sizing = DeclaredAxisSizing::EnvelopeFixed { extent: 80.0 };
        assert_parity(&floored);
    }
}
