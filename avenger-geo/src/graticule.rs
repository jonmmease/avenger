//! Graticule generation.
//!
//! Ported from d3-geo `src/graticule.js` (ISC),
//! https://github.com/d3/d3-geo

use crate::math::EPSILON;
use crate::streamable::MultiLine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graticule {
    /// Minor extent `[[x0, y0], [x1, y1]]` in degrees.
    pub extent_minor: [[f64; 2]; 2],
    /// Major extent (meridians spanning to the poles) in degrees.
    pub extent_major: [[f64; 2]; 2],
    /// Minor steps (degrees between graticule lines).
    pub step_minor: [f64; 2],
    /// Major steps.
    pub step_major: [f64; 2],
    /// Densification step along each line, degrees.
    pub precision: f64,
}

impl Default for Graticule {
    fn default() -> Self {
        Graticule {
            extent_minor: [[-180.0, -80.0 - EPSILON], [180.0, 80.0 + EPSILON]],
            extent_major: [[-180.0, -90.0 + EPSILON], [180.0, 90.0 - EPSILON]],
            step_minor: [10.0, 10.0],
            step_major: [90.0, 360.0],
            precision: 2.5,
        }
    }
}

fn range(start: f64, stop: f64, step: f64) -> Vec<f64> {
    let mut out = Vec::new();
    if step <= 0.0 {
        return out;
    }
    let n = ((stop - start) / step).ceil().max(0.0) as usize;
    for i in 0..n {
        out.push(start + (i as f64) * step);
    }
    out
}

impl Graticule {
    /// Uniform steps for both axes.
    pub fn with_step(mut self, step: f64) -> Self {
        self.step_minor = [step, step];
        self
    }

    fn meridian(&self, x: f64, y0: f64, y1: f64) -> Vec<[f64; 2]> {
        // Meridians are great circles: sample coarsely (90°) and let the
        // projection pipeline's adaptive resampling densify them
        // (d3 `graticuleX(y0, y1, 90)`). Parallels are small circles and
        // must be pre-densified at `precision` instead.
        let mut line: Vec<[f64; 2]> = range(y0, y1 - EPSILON, 90.0)
            .into_iter()
            .map(|y| [x, y])
            .collect();
        line.push([x, y1]);
        line
    }

    fn parallel(&self, y: f64, x0: f64, x1: f64) -> Vec<[f64; 2]> {
        let mut line: Vec<[f64; 2]> = range(x0, x1 - EPSILON, self.precision)
            .into_iter()
            .map(|x| [x, y])
            .collect();
        line.push([x1, y]);
        line
    }

    /// Generate the graticule lines (d3 `graticule().lines()` as one
    /// MultiLine).
    pub fn lines(&self) -> MultiLine {
        let [[x0, y0], [x1, y1]] = self.extent_minor;
        let [[bx0, by0], [bx1, by1]] = self.extent_major;
        let [dx, dy] = self.step_minor;
        let [bdx, bdy] = self.step_major;

        let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();

        // Major meridians (full pole-to-pole span).
        for x in range((bx0 / bdx).ceil() * bdx, bx1, bdx) {
            lines.push(self.meridian(x, by0, by1));
        }
        // Major parallels.
        for y in range((by0 / bdy).ceil() * bdy, by1, bdy) {
            lines.push(self.parallel(y, bx0, bx1));
        }
        // Minor meridians (skip multiples of the major step).
        for x in range((x0 / dx).ceil() * dx, x1, dx) {
            if (x % bdx).abs() > EPSILON {
                lines.push(self.meridian(x, y0, y1));
            }
        }
        // Minor parallels (skip multiples of the major step).
        for y in range((y0 / dy).ceil() * dy, y1, dy) {
            if (y % bdy).abs() > EPSILON {
                lines.push(self.parallel(y, x0, x1));
            }
        }

        MultiLine(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_graticule_structure() {
        let lines = Graticule::default().lines().0;
        // 4 major meridians (-180, -90, 0, 90), 1 major parallel (0),
        // minor meridians every 10 excluding multiples of 90:
        // 36 - 4 = 32; minor parallels -80..80 every 10 excluding 0: 16.
        assert_eq!(lines.len(), 4 + 1 + 32 + 16, "line count");
        // Major meridians span to near the poles, coarsely sampled
        // (adaptive resampling densifies them at projection time).
        let major = &lines[0];
        assert!(major.first().unwrap()[1] < -89.0);
        assert!(major.last().unwrap()[1] > 89.0);
        assert!(
            major.len() <= 4,
            "meridians sampled at 90°: {}",
            major.len()
        );
        // Parallels are small circles, pre-densified at 2.5 degrees.
        let minor_parallel = lines.last().unwrap();
        assert!(minor_parallel.len() > 100);
    }
}
