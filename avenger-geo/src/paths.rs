//! Screen-space geometry as Lyon paths and points, for scene marks.
//!
//! The geometry is the output of [`crate::Projector::project`] or
//! [`crate::Projector::project_geometry`], or other geometry already in output
//! units. Nothing here projects it: coordinates become path coordinates as
//! they are.

use crate::error::AvengerGeoError;
use crate::rows::for_each_row;
use crate::sinks::LyonPathSink;
use crate::stream::GeoSink;
use crate::walk::{walk, Rings};
use geo_traits::GeometryTrait;
use geoarrow_array::GeoArrowArray;
use lyon_path::Path;

/// `geometry`'s lines and polygons as a path. Polygon rings close and lines
/// stay open. Points are left out, and a non-finite vertex breaks its line.
pub fn geometry_path(geometry: &impl GeometryTrait<T = f64>) -> Path {
    let mut sink = LyonPathSink::default();
    walk(geometry, &mut sink, Rings::AsWritten);
    sink.finish()
}

/// The rows of a screen-space GeoArrow array, split for path marks and a
/// symbol mark that share row-aligned styles.
#[derive(Debug, Clone, Default)]
pub struct RowParts {
    /// Each row's polygons as one path of closed rings, for a mark that fills
    /// them. A row without polygons, and a null row, has an empty path.
    pub polygons: Vec<Path>,
    /// Each row's lines as one path of open lines, for a mark that only
    /// strokes them, since a fill would close them. A row without lines has
    /// an empty path. A non-finite vertex breaks its line.
    pub lines: Vec<Path>,
    /// Every finite point of every row, in row order, as `[x, y]`.
    pub points: Vec<[f32; 2]>,
    /// The row of each point.
    pub point_rows: Vec<usize>,
}

/// Split each row of a screen-space GeoArrow array, in any encoding, into
/// its polygons, its lines, and its points.
///
/// # Errors
/// Returns an error for a row that can't be read.
pub fn row_parts(geometry: &dyn GeoArrowArray) -> Result<RowParts, AvengerGeoError> {
    let mut parts = RowParts::default();
    let mut row = 0;
    for_each_row(geometry, Rings::AsWritten, |geometry| {
        let mut sink = PartsSink::default();
        if let Some(geometry) = geometry {
            geometry.stream(&mut sink);
        }
        parts.polygons.push(sink.polygons.finish());
        parts.lines.push(sink.lines.finish());
        for point in sink.points {
            parts.points.push(point);
            parts.point_rows.push(row);
        }
        row += 1;
    })?;
    Ok(parts)
}

/// A row's polygons, its lines, and its points.
#[derive(Default)]
struct PartsSink {
    polygons: LyonPathSink,
    lines: LyonPathSink,
    points: Vec<[f32; 2]>,
    in_polygon: bool,
    in_line: bool,
}

impl PartsSink {
    /// The path that the current line or ring goes to.
    fn path(&mut self) -> &mut LyonPathSink {
        if self.in_polygon {
            &mut self.polygons
        } else {
            &mut self.lines
        }
    }
}

impl GeoSink for PartsSink {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        if self.in_line {
            self.path().point(x, y, m);
        } else {
            let point = [x as f32, y as f32];
            if point.iter().all(|v| v.is_finite()) {
                self.points.push(point);
            }
        }
    }
    fn line_start(&mut self) {
        self.in_line = true;
        self.path().line_start();
    }
    fn line_end(&mut self) {
        self.in_line = false;
        self.path().line_end();
    }
    fn polygon_start(&mut self) {
        self.in_polygon = true;
        self.polygons.polygon_start();
    }
    fn polygon_end(&mut self) {
        self.polygons.polygon_end();
        self.in_polygon = false;
    }
}
