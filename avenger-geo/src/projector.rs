//! Projection configuration and the composed pipeline.
//!
//! Ported from d3-geo `src/projection/index.js` and `src/projection/fit.js`
//! (ISC), <https://github.com/d3/d3-geo>. The pipeline composes:
//!
//! ```text
//! degrees→radians → reverse rings → rotate(λ,φ,γ) → antimeridian clip →
//! resample(projection ∘ view, δ²) → [rectangle clip] → sink
//! ```
//!
//! A [`Surface::Planar`] projection skips the spherical stages and transforms
//! points planar-to-planar.
//!
//! Polygon exteriors have positive signed area (counter-clockwise with y up)
//! and holes negative, as RFC 7946 specifies. The clip stages follow d3's
//! opposite convention, so the pipeline reverses each ring before them. A
//! reflected planar projection also reverses its rings, so exteriors keep
//! positive area on screen, where the rectangle clip needs it.

use crate::clip::antimeridian::AntimeridianPolicy;
use crate::clip::rectangle::ClipRectangle;
use crate::clip::Clip;
use crate::error::AvengerGeoError;
use crate::math::{DEGREES, RADIANS};
use crate::projections::ProjectionImpl;
use crate::resample::Resample;
use crate::rotation::Rotation;
use crate::rows::{for_each_row, lon_lat};
use crate::sinks::{BoundsSink, LyonPathSink};
use crate::stream::{GeoSink, TransformRadians};
use crate::walk::{walk, Rings};
use arrow::array::ArrayRef;
use avenger_common::value::ScalarOrArray;
use geo_traits::GeometryTrait;
use geoarrow_array::GeoArrowArray;
use lyon_path::Path;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Where a projection draws: projected units scale by `scale`, and the
/// projection's center lands at `translate`, in y-down output units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub scale: f64,
    pub translate: [f64; 2],
}

/// The coordinates that a projection takes as input.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// Longitude/latitude degrees, projected by a [`ProjectionImpl`].
    Spherical(Arc<dyn ProjectionImpl>),
    /// Planar coordinates that are already projected (d3 `geoIdentity`). They
    /// skip rotation, antimeridian cutting, and resampling. `reflect_y` flips
    /// the y axis, for y-up data on a y-down screen.
    Planar { reflect_y: bool },
}

/// Serializable projection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Projection {
    pub surface: Surface,
    /// Three-axis rotation in degrees `[λ, φ, γ]`. Planar projections ignore
    /// it.
    pub rotate: [f64; 3],
    /// Projection center in degrees `[lon, lat]`, which the view's translation
    /// places.
    pub center: [f64; 2],
    /// Where the projection draws. A fit or [`Projection::with_view`] sets it,
    /// and a projection without one doesn't build.
    pub view: Option<View>,
    /// Resampling precision in output units, `sqrt(0.5)` by default, which
    /// suits views in pixels. Zero disables resampling.
    pub precision: f64,
    /// Optional planar clip extent `[[x0, y0], [x1, y1]]` in output units.
    pub clip_extent: Option<[[f64; 2]; 2]>,
}

impl Projection {
    /// A spherical projection through `projection`, without a view, rotation,
    /// or clip extent.
    pub fn new(projection: impl ProjectionImpl) -> Self {
        Self::from_surface(Surface::Spherical(Arc::new(projection)))
    }

    /// A planar projection for pre-projected coordinates, without a view or
    /// clip extent.
    pub fn planar(reflect_y: bool) -> Self {
        Self::from_surface(Surface::Planar { reflect_y })
    }

    fn from_surface(surface: Surface) -> Self {
        Projection {
            surface,
            rotate: [0.0, 0.0, 0.0],
            center: [0.0, 0.0],
            view: None,
            precision: 0.5_f64.sqrt(),
            clip_extent: None,
        }
    }

    pub fn with_rotate(mut self, rotate: [f64; 3]) -> Self {
        self.rotate = rotate;
        self
    }

    pub fn with_center(mut self, lon: f64, lat: f64) -> Self {
        self.center = [lon, lat];
        self
    }

    pub fn with_view(mut self, scale: f64, translate: [f64; 2]) -> Self {
        self.view = Some(View { scale, translate });
        self
    }

    pub fn with_precision(mut self, precision: f64) -> Self {
        self.precision = precision;
        self
    }

    pub fn with_clip_extent(mut self, extent: Option<[[f64; 2]; 2]>) -> Self {
        self.clip_extent = extent;
        self
    }

    /// Build the runnable pipeline.
    ///
    /// # Errors
    /// Returns an error when the projection has no view, the view's scale isn't
    /// finite and positive or its translation isn't finite, the precision isn't
    /// finite and non-negative, the clip extent or the projection's bounds
    /// isn't a finite rectangle with positive width and height, or
    /// [`ProjectionImpl::validate`] rejects the projection's parameters.
    pub fn build(&self) -> Result<Projector, AvengerGeoError> {
        let view = self.view.ok_or_else(|| {
            AvengerGeoError::InvalidConfig(
                "the projection has no view: fit it, or set one with `with_view`".into(),
            )
        })?;
        if !(view.scale.is_finite()
            && view.scale > 0.0
            && view.translate.iter().all(|v| v.is_finite()))
        {
            return Err(AvengerGeoError::InvalidConfig(
                "a view's scale must be finite and positive, and its translation finite".into(),
            ));
        }
        if !(self.precision.is_finite() && self.precision >= 0.0) {
            return Err(AvengerGeoError::InvalidConfig(
                "precision must be finite and non-negative".into(),
            ));
        }
        if self.clip_extent.is_some_and(|extent| !is_rectangle(extent)) {
            return Err(AvengerGeoError::InvalidConfig(
                "the clip extent must be a finite rectangle with positive width and height".into(),
            ));
        }
        if let Surface::Spherical(projection) = &self.surface {
            projection.validate()?;
        }
        // d3 recenter(): solve the transform so that the projected center
        // lands on the view's translation.
        let center = match &self.surface {
            Surface::Spherical(projection) => {
                projection.project(self.center[0] * RADIANS, self.center[1] * RADIANS)
            }
            Surface::Planar { reflect_y } => {
                planar_unit(*reflect_y, self.center[0], self.center[1])
            }
        };
        let k = view.scale;
        let transform = ViewTransform {
            k,
            dx: view.translate[0] - k * center.0,
            dy: view.translate[1] + k * center.1,
        };
        // A projection's bounds always clip it, and a clip extent clips it
        // further: Mercator's poles project to infinity, and its sides are the
        // antimeridian.
        let bounds = match &self.surface {
            Surface::Spherical(projection) => projection.bounds().map(|[[x0, y0], [x1, y1]]| {
                let (left, top) = transform.apply((x0, y1));
                let (right, bottom) = transform.apply((x1, y0));
                [[left, top], [right, bottom]]
            }),
            Surface::Planar { .. } => None,
        };
        if bounds.is_some_and(|bounds| !is_rectangle(bounds)) {
            return Err(AvengerGeoError::InvalidConfig(
                "the projection's bounds must be a finite, positive rectangle".into(),
            ));
        }
        let clip_extent = match (bounds, self.clip_extent) {
            (Some(bounds), Some(extent)) => Some(overlap(bounds, extent)),
            (bounds, extent) => bounds.or(extent),
        };
        Ok(Projector {
            surface: self.surface.clone(),
            rotation: Rotation::from_degrees(self.rotate),
            transform,
            delta2: self.precision * self.precision,
            clip_extent,
        })
    }

    /// Summarize `geometry` for fitting this projection's view with
    /// [`Self::fit`]. The summary depends on everything but the view and the
    /// clip extent.
    ///
    /// # Errors
    /// Returns the errors of [`Self::build`] other than a missing view.
    pub fn summarize(
        &self,
        geometry: &impl GeometryTrait<T = f64>,
    ) -> Result<FitSummary, AvengerGeoError> {
        let mut sink = BoundsSink::default();
        self.trial_projector()?.project_to_sink(geometry, &mut sink);
        Ok(FitSummary {
            bounds: sink.result(),
        })
    }

    /// Summarize the whole sphere, for fitting a world map with [`Self::fit`].
    ///
    /// # Errors
    /// Returns the errors described by [`Self::summarize`].
    pub fn summarize_sphere(&self) -> Result<FitSummary, AvengerGeoError> {
        let mut sink = BoundsSink::default();
        self.trial_projector()?.run(&mut sink, |head| head.sphere());
        Ok(FitSummary {
            bounds: sink.result(),
        })
    }

    /// Summarize every row of a GeoArrow array, in any encoding, for fitting
    /// with [`Self::fit`]. Rings are rewound to RFC 7946 by planar signed area,
    /// and null rows are skipped.
    ///
    /// # Errors
    /// Returns the errors described by [`Self::summarize`], and an error for a
    /// row that can't be read.
    pub fn summarize_geometry(
        &self,
        geometry: &dyn GeoArrowArray,
    ) -> Result<FitSummary, AvengerGeoError> {
        let projector = self.trial_projector()?;
        let mut sink = BoundsSink::default();
        for_each_row(geometry, |row| {
            if let Some(row) = row {
                projector.run(&mut sink, |head| row.stream(head));
            }
        })?;
        Ok(FitSummary {
            bounds: sink.result(),
        })
    }

    /// Summarize points from longitude and latitude arrays, for fitting with
    /// [`Self::fit`]. Rows missing either value are skipped.
    ///
    /// # Errors
    /// Returns the errors described by [`Self::summarize`], and an error when
    /// either array isn't numeric or their lengths differ.
    pub fn summarize_points(
        &self,
        lon: &ArrayRef,
        lat: &ArrayRef,
    ) -> Result<FitSummary, AvengerGeoError> {
        let (lon, lat) = lon_lat(lon, lat)?;
        let mut sink = BoundsSink::default();
        self.trial_projector()?.run(&mut sink, |head| {
            for (lon, lat) in lon.iter().zip(lat.iter()) {
                if let (Some(lon), Some(lat)) = (lon, lat) {
                    head.point(lon, lat, None);
                }
            }
        });
        Ok(FitSummary {
            bounds: sink.result(),
        })
    }

    /// The projector that summaries measure with: the trial view at d3's
    /// fitting scale of 150, so that resampling at `precision` follows curves
    /// at about map size, and no clip extent. A projection's own bounds still
    /// clip, as Mercator's world square does.
    fn trial_projector(&self) -> Result<Projector, AvengerGeoError> {
        let mut trial = self.clone();
        trial.clip_extent = None;
        trial.view = Some(View {
            scale: FIT_SCALE,
            translate: [0.0, 0.0],
        });
        trial.build()
    }

    /// Set the view so that the summarized geometry lands in `range`
    /// `[[x0, y0], [x1, y1]]`, in output units. The projection keeps its
    /// aspect ratio, so the geometry fills the range in one dimension and is
    /// centered in the other. A horizontal or vertical line fills its nonzero
    /// dimension. The clip extent stays as it is.
    ///
    /// # Errors
    /// Returns an error for a range that isn't a finite rectangle with positive
    /// width and height, a summary without bounds, or bounds with zero width
    /// and height. On error, the projection stays unchanged.
    pub fn fit(
        &mut self,
        range: [[f64; 2]; 2],
        summary: &FitSummary,
    ) -> Result<(), AvengerGeoError> {
        if !is_rectangle(range) {
            return Err(AvengerGeoError::FitError(
                "the range must be a finite, positive rectangle".into(),
            ));
        }
        let [[bx0, by0], [bx1, by1]] = summary.bounds.ok_or_else(|| {
            AvengerGeoError::FitError("geometry has no finite projected bounds".into())
        })?;
        let [w, h] = [range[1][0] - range[0][0], range[1][1] - range[0][1]];
        let k = f64::min(w / (bx1 - bx0), h / (by1 - by0));
        let scale = FIT_SCALE * k;
        let x = range[0][0] + (w - k * (bx1 + bx0)) / 2.0;
        let y = range[0][1] + (h - k * (by1 + by0)) / 2.0;
        if !scale.is_finite() || scale <= 0.0 || !x.is_finite() || !y.is_finite() {
            return Err(AvengerGeoError::FitError(
                "geometry cannot be fitted at a finite, positive scale".into(),
            ));
        }
        self.view = Some(View {
            scale,
            translate: [x, y],
        });
        Ok(())
    }
}

/// The scale of the trial view that summaries measure with.
const FIT_SCALE: f64 = 150.0;

/// Projected bounds at the fitting view, for fitting a projection's view to
/// geometry with [`Projection::fit`]. The summaries of parts merge into the
/// summary of the whole. The default is the empty summary, which merging leaves
/// unchanged, so batches fold from it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FitSummary {
    bounds: Option<[[f64; 2]; 2]>,
}

impl FitSummary {
    /// The summary of both parts.
    pub fn merge(&self, other: &Self) -> Self {
        let bounds = match (self.bounds, other.bounds) {
            (Some([[ax0, ay0], [ax1, ay1]]), Some([[bx0, by0], [bx1, by1]])) => {
                Some([[ax0.min(bx0), ay0.min(by0)], [ax1.max(bx1), ay1.max(by1)]])
            }
            (bounds, None) | (None, bounds) => bounds,
        };
        FitSummary { bounds }
    }

    /// The width of the summarized bounds over their height: the aspect ratio of a view that
    /// [`Projection::fit`] fills in both dimensions. `None` for a summary without bounds, or
    /// with a width or a height that isn't finite and positive.
    pub fn aspect(&self) -> Option<f64> {
        let [[x0, y0], [x1, y1]] = self.bounds?;
        let [width, height] = [x1 - x0, y1 - y0];
        let positive = |v: f64| v.is_finite() && v > 0.0;
        (positive(width) && positive(height)).then(|| width / height)
    }
}

/// The overlap of rectangles `a` and `b`. Where they don't overlap, it has no
/// area.
fn overlap(a: [[f64; 2]; 2], b: [[f64; 2]; 2]) -> [[f64; 2]; 2] {
    let lower = [a[0][0].max(b[0][0]), a[0][1].max(b[0][1])];
    let upper = [
        a[1][0].min(b[1][0]).max(lower[0]),
        a[1][1].min(b[1][1]).max(lower[1]),
    ];
    [lower, upper]
}

/// Whether `extent` is a finite rectangle with positive width and height.
fn is_rectangle(extent: [[f64; 2]; 2]) -> bool {
    extent.iter().flatten().all(|v| v.is_finite())
        && extent[1][0] > extent[0][0]
        && extent[1][1] > extent[0][1]
}

/// Planar input in unit-scale y-up coordinates. The view flips y, so input
/// that isn't reflected flips first. The map is its own inverse.
fn planar_unit(reflect_y: bool, x: f64, y: f64) -> (f64, f64) {
    (x, if reflect_y { y } else { -y })
}

/// A view's planar transform: `x' = k·x + dx`, `y' = dy − k·y`. Projections
/// are y-up, and output is y-down.
#[derive(Debug, Clone, Copy)]
struct ViewTransform {
    k: f64,
    dx: f64,
    dy: f64,
}

impl ViewTransform {
    fn apply(&self, (x, y): (f64, f64)) -> (f64, f64) {
        (self.k * x + self.dx, self.dy - self.k * y)
    }

    fn invert(&self, (x, y): (f64, f64)) -> (f64, f64) {
        ((x - self.dx) / self.k, (self.dy - y) / self.k)
    }
}

/// A built projection pipeline: point projection/inversion plus geometry
/// streaming.
pub struct Projector {
    surface: Surface,
    rotation: Rotation,
    transform: ViewTransform,
    delta2: f64,
    clip_extent: Option<[[f64; 2]; 2]>,
}

impl Projector {
    /// Project a point, without clipping. Input is degrees (or input units for
    /// a planar projection); output is display units. Returns `None` for
    /// non-finite results.
    pub fn project_point(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let planar = match &self.surface {
            Surface::Spherical(projection) => {
                let (l, p) = self.rotation.rotate(lon * RADIANS, lat * RADIANS);
                projection.project(l, p)
            }
            Surface::Planar { reflect_y } => planar_unit(*reflect_y, lon, lat),
        };
        let (x, y) = self.transform.apply(planar);
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    /// Invert a display-space point back to degrees (or input units for a
    /// planar projection). Returns `None` for non-finite input or output.
    pub fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (px, py) = self.transform.invert((x, y));
        let result = match &self.surface {
            Surface::Spherical(projection) => {
                let (l, p) = projection.invert(px, py)?;
                let (l, p) = self.rotation.invert(l, p);
                (l * DEGREES, p * DEGREES)
            }
            Surface::Planar { reflect_y } => planar_unit(*reflect_y, px, py),
        };
        (result.0.is_finite() && result.1.is_finite()).then_some(result)
    }

    /// The rectangle that streamed geometry is clipped to, in output units: the
    /// projection's clip extent within its projection's bounds, or whichever of
    /// the two it has. Where the two don't overlap, the rectangle has no area,
    /// and nothing draws.
    pub fn clip_extent(&self) -> Option<[[f64; 2]; 2]> {
        self.clip_extent
    }

    /// `geometry` through the full pipeline as a path. Polygon rings close and
    /// lines stay open. Points outside lines are left out, and a non-finite
    /// vertex breaks its line without closing it.
    pub fn project_to_path(&self, geometry: &impl GeometryTrait<T = f64>) -> Path {
        let mut sink = LyonPathSink::default();
        self.project_to_sink(geometry, &mut sink);
        sink.finish()
    }

    /// The whole sphere through the full pipeline as a path: the area that the
    /// projection draws, clipped like any other geometry.
    pub fn sphere_path(&self) -> Path {
        let mut sink = LyonPathSink::default();
        self.run(&mut sink, |head| head.sphere());
        sink.finish()
    }

    /// Project `geometry` through the full pipeline into `sink`, for outputs
    /// that a path doesn't give.
    pub fn project_to_sink(&self, geometry: &impl GeometryTrait<T = f64>, sink: &mut dyn GeoSink) {
        self.run(sink, |head| walk(geometry, head, Rings::AsWritten));
    }

    /// Each row of a GeoArrow array, in any encoding, through the full pipeline
    /// as a path for a `ScenePathMark`: one path per row, as
    /// [`Self::project_to_path`] draws it, and an empty path for a null row.
    /// Rings are rewound to RFC 7946 by planar signed area.
    ///
    /// # Errors
    /// Returns an error for a row that can't be read.
    pub fn project_to_paths(
        &self,
        geometry: &dyn GeoArrowArray,
    ) -> Result<ScalarOrArray<Path>, AvengerGeoError> {
        let mut paths = Vec::with_capacity(geometry.len());
        for_each_row(geometry, |row| {
            paths.push(match row {
                Some(row) => {
                    let mut sink = LyonPathSink::default();
                    self.run(&mut sink, |head| row.stream(head));
                    sink.finish()
                }
                None => Path::new(),
            })
        })?;
        Ok(ScalarOrArray::new_array(paths))
    }

    /// Points from longitude and latitude arrays as `x` and `y` for a point
    /// mark, projected as [`Self::project_point`] projects them, without
    /// clipping. A row missing either value, or whose result isn't finite as
    /// `f32`, is NaN, so the mark draws nothing there.
    ///
    /// # Errors
    /// Returns an error when either array isn't numeric or their lengths
    /// differ.
    pub fn project_points(
        &self,
        lon: &ArrayRef,
        lat: &ArrayRef,
    ) -> Result<(ScalarOrArray<f32>, ScalarOrArray<f32>), AvengerGeoError> {
        let (lon, lat) = lon_lat(lon, lat)?;
        let (x, y): (Vec<f32>, Vec<f32>) = lon
            .iter()
            .zip(lat.iter())
            .map(|point| match point {
                (Some(lon), Some(lat)) => self
                    .project_point(lon, lat)
                    .map(|(x, y)| (x as f32, y as f32))
                    .filter(|(x, y)| x.is_finite() && y.is_finite())
                    .unwrap_or((f32::NAN, f32::NAN)),
                _ => (f32::NAN, f32::NAN),
            })
            .unzip();
        Ok((ScalarOrArray::new_array(x), ScalarOrArray::new_array(y)))
    }

    /// Build the pipeline into `sink` and pass its first stage to `feed`.
    pub(crate) fn run(&self, sink: &mut dyn GeoSink, feed: impl FnOnce(&mut dyn GeoSink)) {
        let sink = postclip(self.clip_extent, sink);
        match &self.surface {
            Surface::Spherical(projection) => {
                let projection = projection.as_ref();
                let transform = self.transform;
                let project = move |l: f64, p: f64| transform.apply(projection.project(l, p));
                let resample = Resample::new(project, self.delta2, sink);
                let rotate = RotateStream {
                    rotation: self.rotation,
                    sink: Clip::new(AntimeridianPolicy, resample),
                };
                feed(&mut TransformRadians {
                    sink: ReverseRings::new(rotate),
                });
            }
            Surface::Planar { reflect_y } => {
                let mut planar = PlanarPoints {
                    reflect_y: *reflect_y,
                    transform: self.transform,
                    sink,
                };
                // Reflection flips each ring's orientation.
                if *reflect_y {
                    feed(&mut ReverseRings::new(planar));
                } else {
                    feed(&mut planar);
                }
            }
        }
    }
}

fn postclip<'a>(
    clip_extent: Option<[[f64; 2]; 2]>,
    sink: &'a mut dyn GeoSink,
) -> Box<dyn GeoSink + 'a> {
    match clip_extent {
        Some(extent) => Box::new(ClipRectangle::new(extent, sink)),
        None => Box::new(sink),
    }
}

/// Reverses each polygon ring, keeping its first point first, and passes other
/// geometry through.
struct ReverseRings<S> {
    sink: S,
    in_polygon: bool,
    in_ring: bool,
    ring: Vec<(f64, f64, Option<f64>)>,
}

impl<S> ReverseRings<S> {
    fn new(sink: S) -> Self {
        ReverseRings {
            sink,
            in_polygon: false,
            in_ring: false,
            ring: Vec::new(),
        }
    }
}

impl<S: GeoSink> GeoSink for ReverseRings<S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        if self.in_ring {
            self.ring.push((x, y, m));
        } else {
            self.sink.point(x, y, m);
        }
    }
    fn line_start(&mut self) {
        if self.in_polygon {
            self.in_ring = true;
            self.ring.clear();
        } else {
            self.sink.line_start();
        }
    }
    fn line_end(&mut self) {
        if !self.in_ring {
            self.sink.line_end();
            return;
        }
        self.in_ring = false;
        self.sink.line_start();
        if let Some((&(x, y, m), rest)) = self.ring.split_first() {
            self.sink.point(x, y, m);
            for &(x, y, m) in rest.iter().rev() {
                self.sink.point(x, y, m);
            }
        }
        self.sink.line_end();
    }
    fn polygon_start(&mut self) {
        self.in_polygon = true;
        self.sink.polygon_start();
    }
    fn polygon_end(&mut self) {
        self.in_polygon = false;
        self.sink.polygon_end();
    }
    fn sphere(&mut self) {
        self.sink.sphere();
    }
}

/// Rotation stage (radians in, radians out).
struct RotateStream<S> {
    rotation: Rotation,
    sink: S,
}

impl<S: GeoSink> GeoSink for RotateStream<S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        let (l, p) = self.rotation.rotate(x, y);
        self.sink.point(l, p, m);
    }
    fn line_start(&mut self) {
        self.sink.line_start()
    }
    fn line_end(&mut self) {
        self.sink.line_end()
    }
    fn polygon_start(&mut self) {
        self.sink.polygon_start()
    }
    fn polygon_end(&mut self) {
        self.sink.polygon_end()
    }
    fn sphere(&mut self) {
        self.sink.sphere()
    }
}

/// Planar stage: input points through the view, no spherical machinery.
struct PlanarPoints<S> {
    reflect_y: bool,
    transform: ViewTransform,
    sink: S,
}

impl<S: GeoSink> GeoSink for PlanarPoints<S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        let (x, y) = self.transform.apply(planar_unit(self.reflect_y, x, y));
        self.sink.point(x, y, m);
    }
    fn line_start(&mut self) {
        self.sink.line_start()
    }
    fn line_end(&mut self) {
        self.sink.line_end()
    }
    fn polygon_start(&mut self) {
        self.sink.polygon_start()
    }
    fn polygon_end(&mut self) {
        self.sink.polygon_end()
    }
    fn sphere(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projections::{
        ConicEqualArea, EqualEarth, Equirectangular, Mercator, NaturalEarth1,
    };

    #[test]
    fn view_transform_round_trip() {
        let t = ViewTransform {
            k: 120.0,
            dx: 33.0,
            dy: -7.0,
        };
        let p = (3.5, -2.25);
        let back = t.invert(t.apply(p));
        assert!((back.0 - p.0).abs() < 1e-12 && (back.1 - p.1).abs() < 1e-12);
    }

    #[test]
    fn projections_build_only_with_a_valid_view() {
        let projection = Projection::new(Equirectangular);
        assert!(projection.build().is_err());
        for (scale, translate) in [
            (0.0, [0.0, 0.0]),
            (f64::NAN, [0.0, 0.0]),
            (1.0, [f64::INFINITY, 0.0]),
        ] {
            assert!(projection
                .clone()
                .with_view(scale, translate)
                .build()
                .is_err());
        }
        // The view places the projection's center, with north up.
        let p = projection.with_view(150.0, [480.0, 250.0]).build().unwrap();
        let (x, y) = p.project_point(0.0, 0.0).unwrap();
        assert!((x - 480.0).abs() < 1e-9 && (y - 250.0).abs() < 1e-9);
        let (x, y) = p.project_point(180.0, 0.0).unwrap();
        assert!((x - (480.0 + std::f64::consts::PI * 150.0)).abs() < 1e-9);
        assert!((y - 250.0).abs() < 1e-9);
        let (_, y) = p.project_point(0.0, 45.0).unwrap();
        assert!(y < 250.0);
    }

    #[test]
    fn project_invert_round_trip() {
        for projection in [
            Projection::new(Equirectangular),
            Projection::new(Mercator),
            Projection::new(EqualEarth),
            Projection::new(NaturalEarth1),
            Projection::new(ConicEqualArea::albers()),
        ] {
            let p = projection
                .clone()
                .with_rotate([96.0, 0.0, 0.0])
                .with_view(400.0, [300.0, 200.0])
                .build()
                .unwrap();
            for &(lon, lat) in &[(-96.0, 38.0), (-120.0, 30.0), (-75.0, 45.0)] {
                let (x, y) = p.project_point(lon, lat).unwrap();
                let (lon2, lat2) = p.invert(x, y).unwrap();
                assert!(
                    (lon2 - lon).abs() < 1e-6 && (lat2 - lat).abs() < 1e-6,
                    "{:?}: ({lon}, {lat}) -> ({lon2}, {lat2})",
                    projection.surface
                );
            }
        }
    }

    #[test]
    fn fitting_the_sphere_centers_the_world() {
        let mut proj = Projection::new(Equirectangular);
        let summary = proj.summarize_sphere().unwrap();
        proj.fit([[0.0, 0.0], [720.0, 360.0]], &summary).unwrap();
        let p = proj.build().unwrap();
        let (x, y) = p.project_point(0.0, 0.0).unwrap();
        assert!((x - 360.0).abs() < 1e-6, "x = {x}");
        assert!((y - 180.0).abs() < 1e-6, "y = {y}");
        // World spans the full extent.
        let (x0, _) = p.project_point(-180.0, 0.0).unwrap();
        let (x1, _) = p.project_point(180.0, 0.0).unwrap();
        assert!((x0 - 0.0).abs() < 1e-6 && (x1 - 720.0).abs() < 1e-6);
    }

    #[test]
    fn summaries_give_the_aspect_of_their_bounds() {
        let proj = Projection::new(Equirectangular);
        let aspect = proj.summarize_sphere().unwrap().aspect().unwrap();
        assert!((aspect - 2.0).abs() < 1e-9, "{aspect}");
        assert_eq!(FitSummary::default().aspect(), None);
        let point = geo_types::Point::new(10.0, 20.0);
        assert_eq!(proj.summarize(&point).unwrap().aspect(), None);
    }
}
