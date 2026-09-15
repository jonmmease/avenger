//! Per-axis chrome sizing. Rectangle placement happens in `carve_slabs`.

use crate::build::ChromeSide;

pub(crate) enum FrameAxisSizing {
    /// Given the outer extent, solve content subject to its minimum.
    Envelope { extent: f32 },
    /// Given both extents, replace declared margins with an equal slack split.
    EnvelopeAndContent { extent: f32, content: f32 },
    /// Given content, derive the outer extent.
    Content { content: f32 },
}

pub(crate) struct FrameAxis<'a> {
    pub sizing: FrameAxisSizing,
    pub leading: &'a ChromeSide,
    pub trailing: &'a ChromeSide,
    pub content_min: f32,
}

pub(crate) struct SolvedSlab {
    pub start: f32,
    pub size: f32,
}

pub(crate) struct FrameAxisSolution {
    pub leading: ChromeSide,
    pub content: SolvedSlab,
    pub trailing: ChromeSide,
    pub extent: f32,
}

impl ChromeSide {
    fn clamped(&self) -> Self {
        Self {
            margin: self.margin.max(0.0),
            strips: self.strips.iter().map(|size| size.max(0.0)).collect(),
            legend: self.legend.max(0.0),
            guide: self.guide.max(0.0),
        }
    }

    /// Slab sizes from outside to inside.
    pub(crate) fn sizes(&self) -> impl DoubleEndedIterator<Item = f32> + '_ {
        std::iter::once(self.margin)
            .chain(self.strips.iter().copied())
            .chain([self.legend, self.guide])
    }
}

impl FrameAxis<'_> {
    pub(crate) fn solve(&self) -> FrameAxisSolution {
        let mut leading = self.leading.clamped();
        let mut trailing = self.trailing.clamped();
        if matches!(self.sizing, FrameAxisSizing::EnvelopeAndContent { .. }) {
            leading.margin = 0.0;
            trailing.margin = 0.0;
        }
        let content_fixed = match self.sizing {
            FrameAxisSizing::Envelope { .. } => 0.0,
            FrameAxisSizing::EnvelopeAndContent { content, .. }
            | FrameAxisSizing::Content { content } => content.max(0.0),
        };
        // Sum in spatial order, excluding flexible margins before adding.
        let fixed_total = leading
            .sizes()
            .chain([content_fixed])
            .chain(trailing.sizes().rev())
            .sum::<f32>();
        let size = match self.sizing {
            FrameAxisSizing::Envelope { extent } => {
                (extent - fixed_total).max(self.content_min.max(0.0))
            }
            FrameAxisSizing::EnvelopeAndContent { extent, .. } => {
                let margin = (extent - fixed_total).max(0.0) / 2.0;
                leading.margin = margin;
                trailing.margin = margin;
                content_fixed
            }
            FrameAxisSizing::Content { .. } => content_fixed,
        };
        let start = leading.sizes().sum::<f32>();
        let extent = trailing
            .sizes()
            .rev()
            .fold(start + size, |sum, size| sum + size);
        FrameAxisSolution {
            leading,
            content: SolvedSlab { start, size },
            trailing,
            extent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(margin: f32, strips: &[f32], legend: f32, guide: f32) -> ChromeSide {
        ChromeSide {
            margin,
            strips: strips.to_vec(),
            legend,
            guide,
        }
    }

    fn axis(
        sizing: FrameAxisSizing,
        leading: ChromeSide,
        trailing: ChromeSide,
    ) -> FrameAxisSolution {
        FrameAxis {
            sizing,
            leading: &leading,
            trailing: &trailing,
            content_min: 50.0,
        }
        .solve()
    }

    #[test]
    fn envelope_fixed_gives_remainder_to_content() {
        let solution = axis(
            FrameAxisSizing::Envelope { extent: 330.0 },
            side(10.0, &[], 0.0, 0.0),
            side(20.0, &[], 0.0, 0.0),
        );

        assert_eq!(solution.leading.margin, 10.0);
        assert_eq!(solution.content.size, 300.0);
        assert_eq!(solution.trailing.margin, 20.0);
        assert_eq!(solution.extent, 330.0);
    }

    #[test]
    fn envelope_fixed_floors_content_when_chrome_overflows() {
        let solution = axis(
            FrameAxisSizing::Envelope { extent: 100.0 },
            side(120.0, &[], 0.0, 0.0),
            side(0.0, &[], 0.0, 0.0),
        );

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
        );

        // Declared margins are ignored; each margin is half the slack.
        assert_eq!(solution.leading.margin, 100.0);
        assert_eq!(solution.content.size, 300.0);
        assert_eq!(solution.trailing.margin, 100.0);
        assert_eq!(solution.extent, 500.0);
    }

    #[test]
    fn envelope_and_content_fixed_clamps_negative_slack() {
        let solution = axis(
            FrameAxisSizing::EnvelopeAndContent {
                extent: 100.0,
                content: 300.0,
            },
            ChromeSide::default(),
            ChromeSide::default(),
        );

        assert_eq!(solution.leading.margin, 0.0);
        assert_eq!(solution.trailing.margin, 0.0);
        assert_eq!(solution.extent, 300.0);
    }

    #[test]
    fn content_fixed_extent_is_the_spatial_sum() {
        let solution = axis(
            FrameAxisSizing::Content { content: 200.0 },
            side(10.0, &[21.0, 15.4], 30.0, 5.5),
            side(10.0, &[], 12.0, 7.25),
        );

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
            ChromeSide::default(),
            ChromeSide::default(),
        );

        assert_eq!(solution.content.size, 3.0);
    }

    #[test]
    fn negative_slabs_clamp_to_zero() {
        let solution = axis(
            FrameAxisSizing::Content { content: -10.0 },
            side(-5.0, &[-1.0], -2.0, -3.0),
            ChromeSide::default(),
        );

        assert_eq!(solution.content.size, 0.0);
        assert_eq!(solution.extent, 0.0);
        assert_eq!(
            solution.leading.margin + solution.leading.legend + solution.leading.guide,
            0.0
        );
    }
}
