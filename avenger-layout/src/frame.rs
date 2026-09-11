//! Leaf frame solver: one content rectangle surrounded by per-side chrome.
//!
//! A frame is the leaf geometry of the content-plus-overflow model: where
//! [`crate::region::EdgeDemand`] declares how much chrome a region demands
//! per side (and [`crate::region::EdgeGrant`] is the solved counterpart), a
//! [`Frame`] solve turns declared chrome layers into concrete per-layer
//! positions around a content rectangle.
//!
//! Each side stacks the same layers, ordered outside-in:
//!
//! ```text
//! margin → strips… → legend → guide → content
//! ```
//!
//! - `guide` is the stratum adjacent to the content (for charts:
//!   axis ticks and labels overflowing the plot),
//! - `legend` is the stratum outside it,
//! - `strips` are zero or more discrete strips outside the legend stratum (for
//!   charts: title and subtitle rows on the top side),
//! - `margin` is the outermost strip.
//!
//! The two axes are independent: the horizontal axis solves the left
//! (leading) and right (trailing) sides plus the content width, the vertical
//! axis the top/bottom sides plus the content height. Per axis exactly one
//! of three sizing modes applies; see [`FrameAxisSizing`].
//!
//! All slab inputs are clamped to zero before solving. Solved positions are
//! exact (unrounded); pixel snapping is a caller policy.

/// Per-axis sizing input for a frame solve.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrameAxisSizing {
    /// The envelope extent is given; the content takes what remains after
    /// the declared chrome, floored at [`FrameAxis::content_min`]. When the
    /// chrome alone exceeds the given extent, the solved extent grows past
    /// it — the floor wins over the envelope.
    Envelope { extent: f32 },
    /// Both the envelope extent and the content extent are given; the two
    /// margins absorb the slack, half each. The margins' declared sizes are
    /// **ignored** in this mode — each margin becomes exactly half of
    /// `max(extent - (strips + legend + guide + content), 0)`.
    EnvelopeAndContent { extent: f32, content: f32 },
    /// The content extent is given; the envelope is the sum of all layers.
    Content { content: f32 },
}

/// One side's declared chrome, ordered outside-in.
///
/// `strips` run from the margin inward: `strips[0]` is adjacent to the margin,
/// the last strip is adjacent to the `legend` stratum. A slab of zero occupies
/// no space; absence and zero are equivalent to the solver.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameSide {
    pub margin: f32,
    pub strips: Vec<f32>,
    pub legend: f32,
    pub guide: f32,
}

/// One axis of a frame: sizing mode, the two sides, and the content floor.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameAxis {
    pub sizing: FrameAxisSizing,
    /// The side before the content on this axis (left, or top).
    pub leading: FrameSide,
    /// The side after the content on this axis (right, or bottom).
    pub trailing: FrameSide,
    /// Minimum content extent. Applies only in
    /// [`FrameAxisSizing::Envelope`] mode; the other modes take the
    /// declared content as-is (clamped to zero).
    pub content_min: f32,
}

/// One solved strip on an axis: its start position and extent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SolvedSlab {
    pub start: f32,
    pub size: f32,
}

/// One side's solved chrome. Same layer names as [`FrameSide`]; every slab
/// carries an absolute start position on the axis (envelope origin = 0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SolvedFrameSide {
    pub margin: SolvedSlab,
    pub strips: Vec<SolvedSlab>,
    pub legend: SolvedSlab,
    pub guide: SolvedSlab,
}

/// A solved frame axis: leading chrome, content, trailing chrome, and the
/// solved envelope extent (which can exceed a requested envelope when the
/// content floor wins).
#[derive(Clone, Debug, PartialEq)]
pub struct FrameAxisSolution {
    pub leading: SolvedFrameSide,
    pub content: SolvedSlab,
    pub trailing: SolvedFrameSide,
    pub extent: f32,
}

impl FrameAxis {
    pub fn solve(&self) -> FrameAxisSolution {
        // Spatial slab order on the axis: leading outside-in, content,
        // trailing inside-out. Fixed sizes accumulate in this order; the
        // position cursor walks it in this order. Keeping one canonical
        // order makes solved starts reproducible regardless of mode.
        let lead_sizes = side_sizes_outside_in(&self.leading);
        let trail_sizes = side_sizes_outside_in(&self.trailing);

        // In EnvelopeAndContent mode the declared margins are replaced
        // by the slack split, so they are flexible and must not count as
        // fixed chrome. Fixed sizes accumulate in spatial order with the
        // flexible entries skipped, never added-then-subtracted: float
        // addition does not cancel exactly.
        let margins_flex = matches!(self.sizing, FrameAxisSizing::EnvelopeAndContent { .. });
        let content_fixed = match self.sizing {
            FrameAxisSizing::Envelope { .. } => None,
            FrameAxisSizing::EnvelopeAndContent { content, .. }
            | FrameAxisSizing::Content { content } => Some(content.max(0.0)),
        };

        let mut fixed_total = 0.0f32;
        for (index, size) in lead_sizes.iter().enumerate() {
            if margins_flex && index == 0 {
                continue;
            }
            fixed_total += *size;
        }
        if let Some(content) = content_fixed {
            fixed_total += content;
        }
        for (index, size) in trail_sizes.iter().enumerate().rev() {
            if margins_flex && index == 0 {
                continue;
            }
            fixed_total += *size;
        }

        let (content_size, margin_override) = match self.sizing {
            FrameAxisSizing::Envelope { extent } => {
                let min = self.content_min.max(0.0);
                let available = (extent - fixed_total).max(min);
                let extra = (available - min).max(0.0);
                (min + extra, None)
            }
            FrameAxisSizing::EnvelopeAndContent { extent, content } => {
                let slack = (extent - fixed_total).max(0.0);
                (content.max(0.0), Some(slack / 2.0))
            }
            FrameAxisSizing::Content { content } => (content.max(0.0), None),
        };

        let mut cursor = 0.0f32;
        let mut place = |size: f32| {
            let slab = SolvedSlab {
                start: cursor,
                size,
            };
            cursor += size;
            slab
        };

        let lead_margin = place(margin_override.unwrap_or(lead_sizes[0]));
        let lead_strips: Vec<SolvedSlab> = lead_sizes[1..lead_sizes.len() - 2]
            .iter()
            .map(|size| place(*size))
            .collect();
        let lead_outer = place(lead_sizes[lead_sizes.len() - 2]);
        let lead_inner = place(lead_sizes[lead_sizes.len() - 1]);
        let content = place(content_size);
        let trail_inner = place(trail_sizes[trail_sizes.len() - 1]);
        let trail_outer = place(trail_sizes[trail_sizes.len() - 2]);
        let mut trail_strips: Vec<SolvedSlab> = trail_sizes[1..trail_sizes.len() - 2]
            .iter()
            .rev()
            .map(|size| place(*size))
            .collect();
        // Walked spatially (inside-out); store outside-in to mirror inputs.
        trail_strips.reverse();
        let trail_margin = place(margin_override.unwrap_or(trail_sizes[0]));

        FrameAxisSolution {
            leading: SolvedFrameSide {
                margin: lead_margin,
                strips: lead_strips,
                legend: lead_outer,
                guide: lead_inner,
            },
            content,
            trailing: SolvedFrameSide {
                margin: trail_margin,
                strips: trail_strips,
                legend: trail_outer,
                guide: trail_inner,
            },
            extent: cursor,
        }
    }
}

/// One side's clamped slab sizes in outside-in order:
/// `[margin, strips…, legend, guide]`. Always at least three entries.
fn side_sizes_outside_in(side: &FrameSide) -> Vec<f32> {
    let mut sizes = Vec::with_capacity(side.strips.len() + 3);
    sizes.push(side.margin.max(0.0));
    for strip in &side.strips {
        sizes.push(strip.max(0.0));
    }
    sizes.push(side.legend.max(0.0));
    sizes.push(side.guide.max(0.0));
    sizes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(margin: f32, strips: &[f32], legend: f32, guide: f32) -> FrameSide {
        FrameSide {
            margin,
            strips: strips.to_vec(),
            legend,
            guide,
        }
    }

    fn axis(sizing: FrameAxisSizing, leading: FrameSide, trailing: FrameSide) -> FrameAxis {
        FrameAxis {
            sizing,
            leading,
            trailing,
            content_min: 50.0,
        }
    }

    #[test]
    fn envelope_fixed_gives_remainder_to_content() {
        let solution = axis(
            FrameAxisSizing::Envelope { extent: 330.0 },
            side(10.0, &[], 0.0, 0.0),
            side(20.0, &[], 0.0, 0.0),
        )
        .solve();

        assert_eq!(solution.leading.margin.size, 10.0);
        assert_eq!(solution.content.size, 300.0);
        assert_eq!(solution.trailing.margin.size, 20.0);
        assert_eq!(solution.extent, 330.0);
    }

    #[test]
    fn envelope_fixed_floors_content_when_chrome_overflows() {
        let solution = axis(
            FrameAxisSizing::Envelope { extent: 100.0 },
            side(120.0, &[], 0.0, 0.0),
            side(0.0, &[], 0.0, 0.0),
        )
        .solve();

        assert_eq!(solution.content.size, 50.0);
        assert_eq!(solution.extent, 170.0);
    }

    #[test]
    fn envelope_and_content_fixed_splits_slack_between_margins() {
        let solution = axis(
            FrameAxisSizing::EnvelopeAndContent {
                extent: 500.0,
                content: 300.0,
            },
            side(7.0, &[], 0.0, 0.0),
            side(13.0, &[], 0.0, 0.0),
        )
        .solve();

        // Declared margins are ignored; each margin is half the slack.
        assert_eq!(solution.leading.margin.size, 100.0);
        assert_eq!(solution.content.size, 300.0);
        assert_eq!(solution.trailing.margin.size, 100.0);
        assert_eq!(solution.extent, 500.0);
    }

    #[test]
    fn envelope_and_content_fixed_clamps_negative_slack() {
        let solution = axis(
            FrameAxisSizing::EnvelopeAndContent {
                extent: 100.0,
                content: 300.0,
            },
            FrameSide::default(),
            FrameSide::default(),
        )
        .solve();

        assert_eq!(solution.leading.margin.size, 0.0);
        assert_eq!(solution.trailing.margin.size, 0.0);
        assert_eq!(solution.extent, 300.0);
    }

    #[test]
    fn content_fixed_extent_is_the_spatial_sum() {
        let solution = axis(
            FrameAxisSizing::Content { content: 200.0 },
            side(10.0, &[21.0, 15.4], 30.0, 5.5),
            side(10.0, &[], 12.0, 7.25),
        )
        .solve();

        assert_eq!(solution.content.size, 200.0);
        assert_eq!(
            solution.extent,
            10.0 + 21.0 + 15.4 + 30.0 + 5.5 + 200.0 + 7.25 + 12.0 + 10.0
        );
    }

    #[test]
    fn content_min_does_not_apply_outside_envelope_fixed() {
        let solution = axis(
            FrameAxisSizing::Content { content: 3.0 },
            FrameSide::default(),
            FrameSide::default(),
        )
        .solve();

        assert_eq!(solution.content.size, 3.0);
    }

    #[test]
    fn solved_starts_walk_outside_in_then_inside_out() {
        let solution = axis(
            FrameAxisSizing::Content { content: 100.0 },
            side(10.0, &[20.0, 15.0], 30.0, 5.0),
            side(40.0, &[8.0], 25.0, 6.0),
        )
        .solve();

        assert_eq!(solution.leading.margin.start, 0.0);
        assert_eq!(solution.leading.strips[0].start, 10.0);
        assert_eq!(solution.leading.strips[1].start, 30.0);
        assert_eq!(solution.leading.legend.start, 45.0);
        assert_eq!(solution.leading.guide.start, 75.0);
        assert_eq!(solution.content.start, 80.0);
        assert_eq!(solution.trailing.guide.start, 180.0);
        assert_eq!(solution.trailing.legend.start, 186.0);
        // Trailing strips stay outside-in like the input: strips[0] is
        // adjacent to the margin, so spatially it comes after legend.
        assert_eq!(solution.trailing.strips[0].start, 211.0);
        assert_eq!(solution.trailing.margin.start, 219.0);
        assert_eq!(solution.extent, 259.0);
        assert_eq!(
            solution.trailing.margin.start + solution.trailing.margin.size,
            solution.extent
        );
    }

    #[test]
    fn negative_slabs_clamp_to_zero() {
        let solution = axis(
            FrameAxisSizing::Content { content: -10.0 },
            side(-5.0, &[-1.0], -2.0, -3.0),
            FrameSide::default(),
        )
        .solve();

        assert_eq!(solution.content.size, 0.0);
        assert_eq!(solution.extent, 0.0);
        assert_eq!(
            solution.leading.margin.size
                + solution.leading.legend.size
                + solution.leading.guide.size,
            0.0
        );
    }
}
