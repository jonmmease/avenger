//! Projection configuration and the composed pipeline.
//!
//! Ported from d3-geo `src/projection/index.js` and `src/projection/fit.js`
//! (ISC), https://github.com/d3/d3-geo. The pipeline composes:
//!
//! ```text
//! degrees→radians → rotate(λ,φ,γ) → antimeridian clip →
//! resample(raw ∘ affine, δ²) → [rectangle clip] → sink
//! ```
//!
//! For [`crate::raw::ProjectionKind::Identity`] the spherical stages are
//! skipped and points are transformed planar-to-planar.

use crate::clip::antimeridian::AntimeridianPolicy;
use crate::clip::rectangle::ClipRectangle;
use crate::clip::Clip;
use crate::math::{DEGREES, RADIANS};
use crate::raw::{ProjectionKind, RawProjection};
use crate::resample::Resample;
use crate::rotation::Rotation;
use crate::stream::{GeoStream, TransformRadians};
use serde::{Deserialize, Serialize};

/// Planar affine `x' = a·x − b·y + dx`, `y' = dy − b·x − a·y`
/// (d3 `scaleTranslateRotate` with reflection omitted; note the built-in
/// y flip: raw projections are y-up, screens are y-down).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub a: f64,
    pub b: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Affine {
    pub fn new(k: f64, dx: f64, dy: f64, alpha: f64) -> Self {
        if alpha == 0.0 {
            Affine {
                a: k,
                b: 0.0,
                dx,
                dy,
            }
        } else {
            Affine {
                a: alpha.cos() * k,
                b: alpha.sin() * k,
                dx,
                dy,
            }
        }
    }

    pub fn identity() -> Self {
        Affine {
            a: 1.0,
            b: 0.0,
            dx: 0.0,
            dy: 0.0,
        }
    }

    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        let (x, y) = p;
        if self.b == 0.0 {
            // d3 `scaleTranslate` — no b terms, so infinite coordinates
            // (e.g. mercator poles) propagate as ±inf instead of NaN.
            (self.a * x + self.dx, self.dy - self.a * y)
        } else {
            (
                self.a * x - self.b * y + self.dx,
                self.dy - self.b * x - self.a * y,
            )
        }
    }

    pub fn invert(&self, p: (f64, f64)) -> (f64, f64) {
        let (x, y) = (p.0 - self.dx, p.1 - self.dy);
        if self.b == 0.0 {
            (x / self.a, -y / self.a)
        } else {
            let d = self.a * self.a + self.b * self.b;
            (
                (self.a * x - self.b * y) / d,
                (-self.b * x - self.a * y) / d,
            )
        }
    }

    /// Compose: apply `self` after `other` — both in this y-flipping form
    /// is awkward, so composition helpers work on the linear pieces.
    pub fn scale(&self) -> f64 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

/// Serializable projection configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Projection {
    pub kind: ProjectionKind,
    /// Three-axis rotation in degrees `[λ, φ, γ]`.
    pub rotate: [f64; 3],
    /// Projection center in degrees `[lon, lat]` (d3 `.center()`).
    pub center: [f64; 2],
    /// Scale factor (d3 `.scale()`, default 150).
    pub scale: f64,
    /// Translate in output units (d3 `.translate()`, default `[480, 250]`).
    pub translate: [f64; 2],
    /// Resampling precision in output units (d3 `.precision()`,
    /// default `sqrt(0.5)`). Zero disables resampling.
    pub precision: f64,
    /// Optional planar post-clip extent `[[x0, y0], [x1, y1]]`.
    pub clip_extent: Option<[[f64; 2]; 2]>,
}

impl Default for Projection {
    fn default() -> Self {
        Projection {
            kind: ProjectionKind::default(),
            rotate: [0.0, 0.0, 0.0],
            center: [0.0, 0.0],
            scale: 150.0,
            translate: [480.0, 250.0],
            precision: 0.5_f64.sqrt(),
            clip_extent: None,
        }
    }
}

impl Projection {
    pub fn new(kind: ProjectionKind) -> Self {
        Projection {
            kind,
            ..Default::default()
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

    pub fn with_scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    pub fn with_translate(mut self, translate: [f64; 2]) -> Self {
        self.translate = translate;
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
    pub fn build(&self) -> Projector {
        let raw = self.kind.raw();
        // d3 recenter(): solve the affine so the projected center lands on
        // `translate`.
        let base = Affine::new(self.scale, 0.0, 0.0, 0.0);
        let center_p = if self.kind.is_identity() {
            raw.project(self.center[0], self.center[1])
        } else {
            raw.project(self.center[0] * RADIANS, self.center[1] * RADIANS)
        };
        let c = base.apply(center_p);
        let transform = Affine::new(
            self.scale,
            self.translate[0] - c.0,
            self.translate[1] - c.1,
            0.0,
        );
        // d3's mercator family auto-installs a world-square clip extent
        // (mercator.js `reclip()`): without it the poles project to
        // infinity. An explicit clip_extent overrides.
        let clip_extent =
            if self.clip_extent.is_none() && matches!(self.kind, ProjectionKind::Mercator) {
                let k = std::f64::consts::PI * self.scale;
                let t = transform.apply(raw.project(0.0, 0.0));
                Some([[t.0 - k, t.1 - k], [t.0 + k, t.1 + k]])
            } else {
                self.clip_extent
            };
        Projector {
            raw,
            rotation: Rotation::from_degrees(self.rotate),
            transform,
            delta2: self.precision * self.precision,
            clip_extent,
            identity: self.kind.is_identity(),
        }
    }

    /// Build a projector for a *view*: spherical degrees in, y-down pixels
    /// out, with `center` — in raw planar units, the space produced by
    /// [`Projection::project_raw_units`] — at the plot midpoint and
    /// `units_per_pixel` resolution. The pipeline clips to the plot
    /// rectangle. This is the chart-view analog of d3's scale/translate:
    /// `scale = 1/upp`, translate solved so the view center lands mid-plot.
    pub fn build_view(
        &self,
        center: (f64, f64),
        units_per_pixel: f64,
        plot_width: f64,
        plot_height: f64,
    ) -> Projector {
        let raw = self.kind.raw();
        let k = 1.0 / units_per_pixel;
        // x' = k·u_x + dx, y' = dy − k·u_y (Affine with b = 0):
        // center maps to the plot midpoint.
        let transform = Affine::new(
            k,
            plot_width / 2.0 - k * center.0,
            plot_height / 2.0 + k * center.1,
            0.0,
        );
        // Clip to the plot rectangle; for mercator additionally bound by
        // the world square (raw ±π), the d3 `reclip()` behavior — without
        // it the poles stretch to infinity.
        let mut clip = [[0.0, 0.0], [plot_width, plot_height]];
        if matches!(self.kind, ProjectionKind::Mercator) {
            let limit = std::f64::consts::PI;
            let (wx0, wy1) = transform.apply((-limit, -limit));
            let (wx1, wy0) = transform.apply((limit, limit));
            clip[0][0] = clip[0][0].max(wx0);
            clip[0][1] = clip[0][1].max(wy0);
            clip[1][0] = clip[1][0].min(wx1);
            clip[1][1] = clip[1][1].min(wy1);
        }
        Projector {
            raw,
            rotation: Rotation::from_degrees(self.rotate),
            transform,
            delta2: self.precision * self.precision,
            clip_extent: Some(clip),
            identity: self.kind.is_identity(),
        }
    }

    /// `build_view` over a custom raw projection (e.g. a
    /// [`crate::blend::CorrectedBlendRaw`] whose anchoring keeps authored
    /// planar units valid at the view center). The custom raw must consume
    /// rotated spherical radians like the catalog raws.
    pub fn build_view_with_raw(
        &self,
        raw: Box<dyn RawProjection>,
        center: (f64, f64),
        units_per_pixel: f64,
        plot_width: f64,
        plot_height: f64,
    ) -> Projector {
        let k = 1.0 / units_per_pixel;
        let transform = Affine::new(
            k,
            plot_width / 2.0 - k * center.0,
            plot_height / 2.0 + k * center.1,
            0.0,
        );
        Projector {
            raw,
            rotation: Rotation::from_degrees(self.rotate),
            transform,
            delta2: self.precision * self.precision,
            clip_extent: Some([[0.0, 0.0], [plot_width, plot_height]]),
            identity: false,
        }
    }

    /// Project spherical degrees to raw planar units (rotation applied, no
    /// scale/translate, y-up). This is the coordinate space that view scales
    /// map to pixels; see `build_view`.
    pub fn project_raw_units(&self, lon: f64, lat: f64) -> (f64, f64) {
        let raw = self.kind.raw();
        if self.kind.is_identity() {
            return raw.project(lon, lat);
        }
        let rotation = Rotation::from_degrees(self.rotate);
        let (l, p) = rotation.rotate(lon * RADIANS, lat * RADIANS);
        raw.project(l, p)
    }

    /// Invert raw planar units back to spherical degrees where defined.
    pub fn invert_raw_units(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let raw = self.kind.raw();
        if self.kind.is_identity() {
            return raw.invert(x, y);
        }
        let rotation = Rotation::from_degrees(self.rotate);
        let (l, p) = raw.invert(x, y)?;
        let (l, p) = rotation.invert(l, p);
        Some((l * DEGREES, p * DEGREES))
    }

    /// Solve scale+translate so `object`'s projected bounds fill `extent`
    /// (d3 `fitExtent`).
    pub fn fit_extent(
        &mut self,
        extent: [[f64; 2]; 2],
        object: &dyn crate::streamable::Streamable,
    ) {
        let saved_clip = self.clip_extent.take();
        self.scale = 150.0;
        self.translate = [0.0, 0.0];
        let projector = self.build();
        let mut bounds = BoundsSink::default();
        projector.stream(object, &mut bounds);
        if let Some([[bx0, by0], [bx1, by1]]) = bounds.result() {
            let w = extent[1][0] - extent[0][0];
            let h = extent[1][1] - extent[0][1];
            let k = f64::min(w / (bx1 - bx0), h / (by1 - by0));
            let x = extent[0][0] + (w - k * (bx1 + bx0)) / 2.0;
            let y = extent[0][1] + (h - k * (by1 + by0)) / 2.0;
            self.scale = 150.0 * k;
            self.translate = [x, y];
        }
        self.clip_extent = saved_clip;
    }

    /// `fit_extent` with a `[[0, 0], size]` extent (d3 `fitSize`).
    pub fn fit_size(&mut self, size: [f64; 2], object: &dyn crate::streamable::Streamable) {
        self.fit_extent([[0.0, 0.0], size], object)
    }
}

/// A built projection pipeline: point projection/inversion plus geometry
/// streaming.
pub struct Projector {
    raw: Box<dyn RawProjection>,
    pub rotation: Rotation,
    pub transform: Affine,
    pub delta2: f64,
    pub clip_extent: Option<[[f64; 2]; 2]>,
    identity: bool,
}

impl Projector {
    /// Project a point. Input is degrees (or planar units for identity);
    /// output is display units. Returns `None` for non-finite results.
    pub fn project(&self, lon: f64, lat: f64) -> Option<(f64, f64)> {
        let planar = if self.identity {
            self.raw.project(lon, lat)
        } else {
            let (l, p) = self.rotation.rotate(lon * RADIANS, lat * RADIANS);
            self.raw.project(l, p)
        };
        let (x, y) = self.transform.apply(planar);
        if x.is_finite() && y.is_finite() {
            Some((x, y))
        } else {
            None
        }
    }

    /// Invert a display-space point back to degrees (or planar units for
    /// identity).
    pub fn invert(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let planar = self.transform.invert((x, y));
        let inv = self.raw.invert(planar.0, planar.1)?;
        if self.identity {
            Some(inv)
        } else {
            let (l, p) = self.rotation.invert(inv.0, inv.1);
            Some((l * DEGREES, p * DEGREES))
        }
    }

    /// Stream `object` through the full pipeline into `sink`.
    pub fn stream(&self, object: &dyn crate::streamable::Streamable, sink: &mut dyn GeoStream) {
        if self.identity {
            let mut chain = PlanarPoints {
                raw: self.raw.as_ref(),
                transform: self.transform,
                sink: postclip(self.clip_extent, sink),
            };
            object.stream(&mut chain);
        } else {
            let raw = self.raw.as_ref();
            let transform = self.transform;
            let project = move |l: f64, p: f64| transform.apply(raw.project(l, p));
            let resample = Resample::new(project, self.delta2, postclip(self.clip_extent, sink));
            let clip = Clip::new(AntimeridianPolicy, resample);
            let rotate = RotateStream {
                rotation: self.rotation,
                sink: clip,
            };
            let mut chain = TransformRadians { sink: rotate };
            object.stream(&mut chain);
        }
    }
}

fn postclip<'a>(
    clip_extent: Option<[[f64; 2]; 2]>,
    sink: &'a mut dyn GeoStream,
) -> Box<dyn GeoStream + 'a> {
    match clip_extent {
        Some(extent) => Box::new(ClipRectangle::new(extent, sink)),
        None => Box::new(sink),
    }
}

/// Rotation stage (radians in, radians out).
struct RotateStream<S> {
    rotation: Rotation,
    sink: S,
}

impl<S: GeoStream> GeoStream for RotateStream<S> {
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

/// Identity-kind stage: planar input points through raw+affine, no
/// spherical machinery.
struct PlanarPoints<'r, S> {
    raw: &'r dyn RawProjection,
    transform: Affine,
    sink: S,
}

impl<S: GeoStream> GeoStream for PlanarPoints<'_, S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        let (x, y) = self.transform.apply(self.raw.project(x, y));
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

/// Planar bounds accumulator (d3 `path/bounds.js`).
#[derive(Debug, Default)]
pub struct BoundsSink {
    x0: Option<f64>,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl BoundsSink {
    pub fn result(&self) -> Option<[[f64; 2]; 2]> {
        self.x0.map(|x0| [[x0, self.y0], [self.x1, self.y1]])
    }
}

impl GeoStream for BoundsSink {
    fn point(&mut self, x: f64, y: f64, _m: Option<f64>) {
        match self.x0 {
            None => {
                self.x0 = Some(x);
                self.x1 = x;
                self.y0 = y;
                self.y1 = y;
            }
            Some(x0) => {
                if x < x0 {
                    self.x0 = Some(x);
                }
                if x > self.x1 {
                    self.x1 = x;
                }
                if y < self.y0 {
                    self.y0 = y;
                }
                if y > self.y1 {
                    self.y1 = y;
                }
            }
        }
    }
    fn line_start(&mut self) {}
    fn line_end(&mut self) {}
    fn polygon_start(&mut self) {}
    fn polygon_end(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streamable::Sphere;

    #[test]
    fn affine_round_trip() {
        let t = Affine::new(120.0, 33.0, -7.0, 0.4);
        let p = (3.5, -2.25);
        let q = t.apply(p);
        let back = t.invert(q);
        assert!((back.0 - p.0).abs() < 1e-12 && (back.1 - p.1).abs() < 1e-12);
    }

    #[test]
    fn default_projection_matches_d3_defaults() {
        // d3.geoEqualEarth() default: scale 150 (d3 uses 177.158 for
        // equalEarth's own default, but the shared projectionMutator default
        // before .scale() is 150 — our Projection keeps the shared default
        // and callers/fit set scale).
        let proj = Projection::new(ProjectionKind::Equirectangular);
        let p = proj.build();
        // Equirectangular at scale 150 translate [480, 250]:
        // (0,0) -> (480, 250); (180, 0) -> (480 + PI*150, 250)
        let (x, y) = p.project(0.0, 0.0).unwrap();
        assert!((x - 480.0).abs() < 1e-9 && (y - 250.0).abs() < 1e-9);
        let (x, y) = p.project(180.0, 0.0).unwrap();
        assert!((x - (480.0 + std::f64::consts::PI * 150.0)).abs() < 1e-9);
        assert!((y - 250.0).abs() < 1e-9);
        // y flip: north is up.
        let (_, y) = p.project(0.0, 45.0).unwrap();
        assert!(y < 250.0);
    }

    #[test]
    fn project_invert_round_trip() {
        for kind in [
            ProjectionKind::Equirectangular,
            ProjectionKind::Mercator,
            ProjectionKind::EqualEarth,
            ProjectionKind::NaturalEarth1,
            ProjectionKind::albers(),
        ] {
            let proj = Projection::new(kind.clone())
                .with_rotate([96.0, 0.0, 0.0])
                .with_scale(400.0)
                .with_translate([300.0, 200.0]);
            let p = proj.build();
            for &(lon, lat) in &[(-96.0, 38.0), (-120.0, 30.0), (-75.0, 45.0)] {
                let (x, y) = p.project(lon, lat).unwrap();
                let (lon2, lat2) = p.invert(x, y).unwrap();
                assert!(
                    (lon2 - lon).abs() < 1e-6 && (lat2 - lat).abs() < 1e-6,
                    "{kind:?}: ({lon}, {lat}) -> ({lon2}, {lat2})"
                );
            }
        }
    }

    #[test]
    fn fit_size_sphere_centers_world() {
        let mut proj = Projection::new(ProjectionKind::Equirectangular);
        proj.fit_size([720.0, 360.0], &Sphere);
        let p = proj.build();
        let (x, y) = p.project(0.0, 0.0).unwrap();
        assert!((x - 360.0).abs() < 1e-6, "x = {x}");
        assert!((y - 180.0).abs() < 1e-6, "y = {y}");
        // World spans the full extent.
        let (x0, _) = p.project(-180.0, 0.0).unwrap();
        let (x1, _) = p.project(180.0, 0.0).unwrap();
        assert!((x0 - 0.0).abs() < 1e-6 && (x1 - 720.0).abs() < 1e-6);
    }
}
