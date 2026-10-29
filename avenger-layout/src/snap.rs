//! Snapping solved coordinates to device pixels.

use crate::tracks::Allocation;

/// How far, in device pixels, a value may sit past a pixel and still count as on it, so that
/// float error does not round a whole size up by a pixel.
const TOLERANCE: f64 = 1e-3;

/// Rounds layout units to the device pixels of one pixel ratio.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Snap {
    ratio: f64,
}

impl Snap {
    pub fn new(ratio: Option<f64>) -> Option<Self> {
        ratio.map(|ratio| Self { ratio })
    }

    /// The nearest device pixel.
    pub fn round(self, value: f64) -> f64 {
        (value * self.ratio).round() / self.ratio
    }

    /// The device pixel at or above `value`, for sizes that content must fit in.
    pub fn up(self, value: f64) -> f64 {
        (value * self.ratio - TOLERANCE).ceil() / self.ratio
    }

    /// The device pixel at or below `value`.
    pub fn down(self, value: f64) -> f64 {
        (value * self.ratio + TOLERANCE).floor() / self.ratio
    }

    /// Both ends of a range at their nearest device pixels.
    pub fn range(self, range: (f64, f64)) -> (f64, f64) {
        (self.round(range.0), self.round(range.1))
    }

    /// Snaps an owner's tracks: each track keeps at least its base size rounded up and
    /// otherwise rounds down, so that equal tracks stay equal. Each gap rounds its part from
    /// `gaps`, what overflow and minimums need, up so that the overflow fits, and the space that
    /// distribution added to it down, so that the distributed space never pushes the tracks
    /// past their owner. Starts accumulate from the snapped sizes, which leaves the remaining
    /// pixels after the last track.
    pub fn tracks(self, allocation: &mut Allocation, base: &[f64], gaps: &[f64]) {
        for (size, base) in allocation.sizes.iter_mut().zip(base) {
            *size = self.up(*base).max(self.down(*size));
        }
        for (gap, needed) in allocation.gaps.iter_mut().zip(gaps) {
            *gap = self.up(*needed) + self.down((*gap - needed).max(0.0));
        }
        let mut cursor = allocation
            .starts
            .first()
            .map_or(0.0, |&start| self.round(start));
        for index in 0..allocation.sizes.len() {
            if index > 0 {
                cursor += allocation.gaps[index - 1];
            }
            allocation.starts[index] = cursor;
            cursor += allocation.sizes[index];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_sizes_round_up_and_gaps_hold_their_overflow() {
        let snap = Snap { ratio: 2.0 };
        let mut allocation = Allocation {
            sizes: vec![10.2, 30.0],
            starts: vec![0.0, 15.3],
            gaps: vec![5.1],
        };
        snap.tracks(&mut allocation, &[10.2, 0.0], &[5.1]);
        assert_eq!(allocation.sizes, [10.5, 30.0]);
        assert_eq!(allocation.gaps, [5.5]);
        assert_eq!(allocation.starts, [0.0, 16.0]);
    }
}
