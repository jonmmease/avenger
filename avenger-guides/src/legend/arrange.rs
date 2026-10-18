//! Legend entries arranged along a side at a length: plain arithmetic on their sizes.
//!
//! Lines of entries run along the side: across a top or bottom legend, and down a left or
//! right one. Sizes and positions are `[along, across]`.

use std::ops::RangeInclusive;

use super::LegendWrap;

/// Where each entry lies in the entries' block, the block's size, and the lengths over which
/// the arrangement holds.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arrangement {
    /// Each entry's position, `[along, across]`, from the block's start.
    pub(crate) positions: Vec<[f32; 2]>,
    /// The block's size, `[along, across]`.
    pub(crate) size: [f32; 2],
    /// The lengths where every entry keeps its place.
    pub(crate) holds: Holds,
}

/// The lengths over which an arrangement holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Holds {
    /// From `low` up to, but not including, `below`, which is infinite when nothing changes
    /// above `low`.
    From { low: f32, below: f32 },
    /// Only at the asked length, since entries pass it.
    At,
}

impl Holds {
    /// The lengths as an inclusive range of a length `shift` longer than the arranged one, for
    /// `length`, the asked length.
    pub(crate) fn shifted(self, shift: f32, length: f32) -> RangeInclusive<f32> {
        match self {
            Self::From { low, below } if below.is_finite() => {
                (low + shift)..=(below + shift).next_down()
            }
            Self::From { low, .. } => (low + shift)..=f32::INFINITY,
            Self::At => length..=length,
        }
    }
}

/// Arranges entries of `sizes` in `length`, with `gaps` between entries along a line and
/// between lines. `columns_across` is whether a fixed column count counts lines, as on a left or
/// right side, rather than cells along a line.
pub(crate) fn arrange(
    sizes: &[[f32; 2]],
    gaps: [f32; 2],
    wrap: LegendWrap,
    columns_across: bool,
    length: f32,
) -> Arrangement {
    if sizes.is_empty() {
        return Arrangement {
            positions: Vec::new(),
            size: [0.0; 2],
            holds: Holds::From {
                low: 0.0,
                below: f32::INFINITY,
            },
        };
    }
    match wrap {
        LegendWrap::Flow => flow(sizes, gaps, length),
        LegendWrap::Grid { columns } => {
            let count = sizes.len();
            let cell = sizes.iter().fold([0.0f32; 2], |cell, size| {
                [cell[0].max(size[0]), cell[1].max(size[1])]
            });
            let line_length = |cells: usize| cells as f32 * cell[0] + (cells - 1) as f32 * gaps[0];
            let (cells, holds) = match columns {
                Some(columns) => {
                    let columns = columns.get();
                    let cells = if columns_across {
                        count.div_ceil(columns)
                    } else {
                        columns.min(count)
                    };
                    let size = line_length(cells);
                    let holds = if size <= length {
                        Holds::From {
                            low: size,
                            below: f32::INFINITY,
                        }
                    } else {
                        Holds::At
                    };
                    (cells, holds)
                }
                None => {
                    // The most cells that fit, at least one and at most every entry.
                    let mut cells = (((length + gaps[0]) / (cell[0] + gaps[0])).floor().max(1.0)
                        as usize)
                        .min(count);
                    while cells > 1 && line_length(cells) > length {
                        cells -= 1;
                    }
                    while cells < count && line_length(cells + 1) <= length {
                        cells += 1;
                    }
                    let holds = if line_length(cells) > length {
                        Holds::At
                    } else if cells == count {
                        Holds::From {
                            low: line_length(cells),
                            below: f32::INFINITY,
                        }
                    } else {
                        Holds::From {
                            low: line_length(cells),
                            below: line_length(cells + 1),
                        }
                    };
                    (cells, holds)
                }
            };
            let lines = count.div_ceil(cells);
            let positions = (0..count)
                .map(|index| {
                    let (line, slot) = (index / cells, index % cells);
                    [
                        slot as f32 * (cell[0] + gaps[0]),
                        line as f32 * (cell[1] + gaps[1]),
                    ]
                })
                .collect();
            Arrangement {
                positions,
                size: [
                    line_length(cells.min(count)),
                    lines as f32 * cell[1] + (lines - 1) as f32 * gaps[1],
                ],
                holds,
            }
        }
    }
}

/// Entries in lines along the side, each at its own size. A line breaks before an entry that
/// would pass `length`, and every line takes at least one entry.
fn flow(sizes: &[[f32; 2]], gaps: [f32; 2], length: f32) -> Arrangement {
    // Each line's first entry, length and thickness.
    let mut lines: Vec<(usize, f32, f32)> = Vec::new();
    let mut along = Vec::with_capacity(sizes.len());
    for (index, size) in sizes.iter().enumerate() {
        match lines.last_mut() {
            Some((_, used, thickness)) if *used + gaps[0] + size[0] <= length => {
                along.push(*used + gaps[0]);
                // The same sum as the fit test, so that a line's length fits exactly.
                *used = *used + gaps[0] + size[0];
                *thickness = thickness.max(size[1]);
            }
            _ => {
                along.push(0.0);
                lines.push((index, size[0], size[1]));
            }
        }
    }
    let mut across = Vec::with_capacity(lines.len());
    let mut offset = 0.0;
    for (_, _, thickness) in &lines {
        across.push(offset);
        offset += thickness + gaps[1];
    }
    let longest = lines
        .iter()
        .fold(0.0f32, |longest, line| longest.max(line.1));
    // A line takes the next line's first entry from the length where it fits.
    let next_fit = lines
        .windows(2)
        .map(|pair| pair[0].1 + gaps[0] + sizes[pair[1].0][0])
        .fold(f32::INFINITY, f32::min);
    let holds = if longest > length {
        Holds::At
    } else {
        Holds::From {
            low: longest,
            below: next_fit,
        }
    };
    let mut line = 0;
    let positions = (0..sizes.len())
        .map(|index| {
            while line + 1 < lines.len() && lines[line + 1].0 <= index {
                line += 1;
            }
            [along[index], across[line]]
        })
        .collect();
    Arrangement {
        positions,
        size: [longest, offset - gaps[1]],
        holds,
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;

    use super::*;

    const GAPS: [f32; 2] = [10.0, 2.0];

    fn sizes() -> Vec<[f32; 2]> {
        vec![
            [40.0, 12.0],
            [70.0, 12.0],
            [25.0, 16.0],
            [55.0, 12.0],
            [30.0, 12.0],
        ]
    }

    fn wraps() -> Vec<(LegendWrap, bool)> {
        let fixed = |n| LegendWrap::Grid {
            columns: NonZeroUsize::new(n),
        };
        vec![
            (LegendWrap::Flow, false),
            (LegendWrap::Flow, true),
            (fixed(0), false),
            (fixed(0), true),
            (fixed(2), false),
            (fixed(2), true),
        ]
    }

    /// What the layout sees of an arrangement: the entries' places, the block's size, and
    /// whether the block passes the length.
    fn answer(arrangement: &Arrangement, length: f32) -> (Vec<[f32; 2]>, [f32; 2], bool) {
        let passes = arrangement.size[0] > length;
        (arrangement.positions.clone(), arrangement.size, passes)
    }

    /// The answer is the same at both ends of each range, and differs just past each end of a
    /// range wider than one length.
    #[test]
    fn holds_is_exactly_the_range_that_keeps_the_arrangement() {
        for (wrap, columns_across) in wraps() {
            for length in (20..400).step_by(3).map(|length| length as f32) {
                let at = |length| {
                    let arrangement = arrange(&sizes(), GAPS, wrap, columns_across, length);
                    answer(&arrangement, length)
                };
                let arrangement = arrange(&sizes(), GAPS, wrap, columns_across, length);
                let expected = answer(&arrangement, length);
                let holds = arrangement.holds.shifted(0.0, length);
                let (low, high) = (*holds.start(), *holds.end());
                assert!(low <= length && length <= high, "{wrap:?} {length}");
                assert_eq!(at(low), expected, "{wrap:?} {length} at {low}");
                if low == high {
                    continue;
                }
                if high.is_finite() {
                    assert_eq!(at(high), expected, "{wrap:?} {length} at {high}");
                    assert_ne!(at(high.next_up()), expected, "{wrap:?} {length}");
                }
                if low > 0.0 {
                    assert_ne!(at(low.next_down()), expected, "{wrap:?} {length}");
                }
            }
        }
    }

    #[test]
    fn flow_breaks_lines_before_an_entry_that_would_pass_the_length() {
        let arrangement = arrange(&sizes(), GAPS, LegendWrap::Flow, false, 130.0);
        // 40 + 10 + 70 = 120 fits and 120 + 10 + 25 does not, while 25 + 10 + 55 + 10 + 30 =
        // 130 fits exactly.
        assert_eq!(
            arrangement.positions,
            [
                [0.0, 0.0],
                [50.0, 0.0],
                [0.0, 14.0],
                [35.0, 14.0],
                [100.0, 14.0]
            ]
        );
        assert_eq!(arrangement.size, [130.0, 30.0]);
        // The second line needs 130, and the first takes the third entry from 155.
        assert_eq!(
            arrangement.holds,
            Holds::From {
                low: 130.0,
                below: 155.0
            }
        );
    }

    #[test]
    fn grids_fill_cells_along_the_side_and_fixed_columns_ignore_the_length() {
        // Cells are 70 by 16.
        let open = arrange(
            &sizes(),
            GAPS,
            LegendWrap::Grid { columns: None },
            false,
            160.0,
        );
        assert_eq!(open.positions[2], [0.0, 18.0]);
        assert_eq!(open.size, [150.0, 52.0]);
        let fixed = |columns_across, length| {
            arrange(
                &sizes(),
                GAPS,
                LegendWrap::Grid {
                    columns: NonZeroUsize::new(2),
                },
                columns_across,
                length,
            )
        };
        // Above or below the plot, two columns of cells along the side.
        assert_eq!(fixed(false, 500.0).size, [150.0, 52.0]);
        // Beside the plot, two columns across it: three cells down each.
        let beside = fixed(true, 30.0);
        assert_eq!(beside.size, [230.0, 34.0]);
        assert_eq!(beside.positions[3], [0.0, 18.0]);
        // Past its length, the grid holds only at the asked length.
        assert_eq!(beside.holds, Holds::At);
        assert_eq!(
            fixed(true, 230.0).holds,
            Holds::From {
                low: 230.0,
                below: f32::INFINITY
            }
        );
    }
}
