//! Chart-owned uniform-track policy: `count` equally sized slots sharing
//! one [`Spacing`].
//!
//! This is the chart's band/facet lane policy, where per-track geometry is
//! uniform by construction and merging coordinates the policy (max count,
//! max spacing) rather than per-track values — so merging across different
//! counts is well-defined (ragged facets). In the unified layout API the
//! same semantics live behind `.uniform_columns()/.uniform_rows()` +
//! `.share(key)`; this struct remains for the chart's scale-backed
//! placements, which derive positions from band scales and only verify them
//! against the arithmetic progression.

use avenger_layout::Spacing;

/// Uniform single-span tracks: the policy-level requirement.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct UniformTracks {
    pub count: usize,
    pub spacing: Spacing,
}

impl UniformTracks {
    pub(crate) fn merge_max(self, other: Self) -> Self {
        Self {
            count: self.count.max(other.count),
            spacing: self.spacing.merge_max(other.spacing),
        }
    }

    /// Solve the uniform arrangement for a given per-track size: starts form
    /// an arithmetic progression with the spacing's `min_gap` between
    /// tracks, offset by `outer_start`; the extent includes both outers.
    pub(crate) fn solve(&self, track_size: f32) -> UniformTrackSolution {
        let track_size = track_size.max(0.0);
        let gap = self.spacing.min_gap.max(0.0);
        let outer_start = self.spacing.outer_start.max(0.0);
        let starts = (0..self.count)
            .map(|index| outer_start + index as f32 * (track_size + gap))
            .collect::<Vec<_>>();
        let extent = if self.count == 0 {
            outer_start + self.spacing.outer_end.max(0.0)
        } else {
            outer_start
                + self.count as f32 * track_size
                + (self.count - 1) as f32 * gap
                + self.spacing.outer_end.max(0.0)
        };
        UniformTrackSolution {
            starts,
            track_size,
            extent,
        }
    }
}

/// Solved positions for a uniform arrangement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UniformTrackSolution {
    pub starts: Vec<f32>,
    pub track_size: f32,
    pub extent: f32,
}
