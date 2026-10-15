//! Graticule generation.
//!
//! Ported from d3-geo `src/graticule.js` (ISC),
//! <https://github.com/d3/d3-geo>

use crate::math::EPSILON;
use crate::AvengerGeoError;
use geo_types::{LineString, MultiLineString};
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

/// Whether a graticule line follows a meridian or a parallel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraticuleLineKind {
    Meridian,
    Parallel,
}

/// One graticule line: the meridian at `value` degrees of longitude, or the
/// parallel at `value` degrees of latitude.
#[derive(Debug, Clone, PartialEq)]
pub struct GraticuleLine {
    pub kind: GraticuleLineKind,
    pub value: f64,
    /// Longitude/latitude points in degrees.
    pub line: LineString<f64>,
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

    /// The graticule lines (d3 `graticule().lines()`).
    ///
    /// # Errors
    /// Returns the errors described by [`Self::graticule_lines`].
    pub fn lines(&self) -> Result<MultiLineString<f64>, AvengerGeoError> {
        Ok(MultiLineString(
            self.graticule_lines()?
                .into_iter()
                .map(|line| line.line)
                .collect(),
        ))
    }

    /// The graticule lines with the meridian or parallel each one follows, in
    /// the order of [`Self::lines`].
    ///
    /// # Errors
    /// Returns an error for invalid extents, steps, or precision, or when
    /// the upper bound on generated vertices exceeds one million.
    pub fn graticule_lines(&self) -> Result<Vec<GraticuleLine>, AvengerGeoError> {
        let steps = [
            self.step_minor[0],
            self.step_minor[1],
            self.step_major[0],
            self.step_major[1],
            self.precision,
        ];
        if !steps.iter().all(|v| v.is_finite() && *v > 0.0) {
            return Err(AvengerGeoError::InvalidConfig(
                "graticule steps and precision must be finite and positive".into(),
            ));
        }
        let mut max_vertices = 0.0;
        for (extent, step) in [
            (self.extent_minor, self.step_minor),
            (self.extent_major, self.step_major),
        ] {
            if !extent.iter().flatten().all(|v| v.is_finite())
                || extent[0][0] > extent[1][0]
                || extent[0][1] > extent[1][1]
                || extent[0][0] < -180.0
                || extent[1][0] > 180.0
                || extent[0][1] < -90.0
                || extent[1][1] > 90.0
            {
                return Err(AvengerGeoError::InvalidConfig(
                    "graticule extents must be ordered longitude/latitude bounds".into(),
                ));
            }
            let width = extent[1][0] - extent[0][0];
            let height = extent[1][1] - extent[0][1];
            max_vertices += (width / step[0]).ceil() * ((height / 90.0).ceil() + 1.0)
                + (height / step[1]).ceil() * ((width / self.precision).ceil() + 1.0);
        }
        if !max_vertices.is_finite() || max_vertices > 1_000_000.0 {
            return Err(AvengerGeoError::InvalidConfig(
                "graticule exceeds the one-million-vertex limit".into(),
            ));
        }
        let [[x0, y0], [x1, y1]] = self.extent_minor;
        let [[bx0, by0], [bx1, by1]] = self.extent_major;
        let [dx, dy] = self.step_minor;
        let [bdx, bdy] = self.step_major;

        let mut lines = Vec::new();
        let meridian = |x: f64, y0: f64, y1: f64| GraticuleLine {
            kind: GraticuleLineKind::Meridian,
            value: x,
            line: self.meridian(x, y0, y1).into(),
        };
        let parallel = |y: f64, x0: f64, x1: f64| GraticuleLine {
            kind: GraticuleLineKind::Parallel,
            value: y,
            line: self.parallel(y, x0, x1).into(),
        };

        // Major meridians (full pole-to-pole span).
        for x in range((bx0 / bdx).ceil() * bdx, bx1, bdx) {
            lines.push(meridian(x, by0, by1));
        }
        // Major parallels.
        for y in range((by0 / bdy).ceil() * bdy, by1, bdy) {
            lines.push(parallel(y, bx0, bx1));
        }
        // Minor meridians (skip multiples of the major step).
        for x in range((x0 / dx).ceil() * dx, x1, dx) {
            if (x % bdx).abs() > EPSILON {
                lines.push(meridian(x, y0, y1));
            }
        }
        // Minor parallels (skip multiples of the major step).
        for y in range((y0 / dy).ceil() * dy, y1, dy) {
            if (y % bdy).abs() > EPSILON {
                lines.push(parallel(y, x0, x1));
            }
        }

        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_graticule_structure() {
        let graticule_lines = Graticule::default().graticule_lines().unwrap();
        let majors: Vec<_> = graticule_lines[..5]
            .iter()
            .map(|line| (line.kind, line.value))
            .collect();
        use GraticuleLineKind::{Meridian, Parallel};
        assert_eq!(
            majors,
            [
                (Meridian, -180.0),
                (Meridian, -90.0),
                (Meridian, 0.0),
                (Meridian, 90.0),
                (Parallel, 0.0)
            ]
        );
        let lines = Graticule::default().lines().unwrap().0;
        // 4 major meridians (-180, -90, 0, 90), 1 major parallel (0),
        // minor meridians every 10 excluding multiples of 90:
        // 36 - 4 = 32; minor parallels -80..80 every 10 excluding 0: 16.
        assert_eq!(lines.len(), 4 + 1 + 32 + 16, "line count");
        // Major meridians span to near the poles, coarsely sampled
        // (adaptive resampling densifies them at projection time).
        let major = &lines[0];
        assert!(major.0.first().unwrap().y < -89.0);
        assert!(major.0.last().unwrap().y > 89.0);
        assert!(
            major.0.len() <= 4,
            "meridians sampled at 90°: {}",
            major.0.len()
        );
        // Parallels are small circles, pre-densified at 2.5 degrees.
        let minor_parallel = lines.last().unwrap();
        assert!(minor_parallel.0.len() > 100);
    }
}
