//! Projection configuration and the composed pipeline.
//!
//! Ported from d3-geo `src/projection/index.js` and `src/projection/fit.js`
//! (ISC), <https://github.com/d3/d3-geo>. The pipeline composes:
//!
//! ```text
//! finite vertices → degrees→radians → reverse rings → rotate(λ,φ,γ) →
//! fixed rotation → spherical clip → resample(projection ∘ view, δ²) →
//! [rectangle clip] → sink
//! ```
//!
//! The first stage leaves out non-finite vertices, and the ring stage leaves
//! out the parts of rings without area, such as the spike that data cut for
//! planar maps runs to a pole. The fixed rotation is the projection's own,
//! such as transverse Mercator's roll. The spherical
//! clip is the antimeridian cut, a circle, or a region. A
//! [`Surface::Planar`] projection skips the spherical stages and transforms
//! points planar-to-planar.
//!
//! Polygon exteriors have positive signed area (counter-clockwise with y up)
//! and holes negative, as RFC 7946 specifies. The clip stages follow d3's
//! opposite convention, so the pipeline reverses each ring before them. A
//! reflected planar projection also reverses its rings, so exteriors keep
//! positive area on screen, where the rectangle clip needs it.

use crate::clip::antimeridian::AntimeridianPolicy;
use crate::clip::circle::CirclePolicy;
use crate::clip::rectangle::{ClipRectangle, CLIP_MAX};
use crate::clip::{Clip, ClipPolicy};
use crate::collect::{Collector, Kind, ScreenBuilder};
use crate::error::AvengerGeoError;
use crate::math::{DEGREES, RADIANS};
use crate::projections::ProjectionImpl;
use crate::resample::Resample;
use crate::rotation::{Composed, Rotation};
use crate::rows::for_each_row;
use crate::sinks::BoundsSink;
use crate::stream::{FiniteVertices, GeoSink, TransformRadians};
use crate::walk::{walk, Rings};
use geo_traits::GeometryTrait;
use geo_types::{Coord, Geometry, LineString, MultiPolygon, Polygon};
use geoarrow_array::GeoArrowArray;
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

/// Which part of the sphere a projection keeps, before it projects: the clip
/// that every method uses, fitting included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SphericalClip {
    /// Cut at the antimeridian, the default for cylindrical, pseudocylindrical,
    /// and conic projections.
    Antimeridian,
    /// Keep the cap within `angle` degrees of the projection's center, which
    /// `rotate` places, as d3's `clipAngle` does. The angle must lie strictly
    /// between 0 and 180.
    Circle { angle: f64 },
    /// Keep the inside of a spherical polygon in longitude and latitude degrees,
    /// whose edges are great circles and whose rings follow RFC 7946 winding,
    /// in the frame that `frame` names. The polygons have no holes.
    Region {
        polygon: MultiPolygon<f64>,
        frame: RegionFrame,
    },
}

/// The frame that a region clip's coordinates belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegionFrame {
    /// The earth: the projection's rotation turns the region with the geometry,
    /// so it stays on the same places, as a region that keeps a continent does.
    Earth,
    /// The rotated frame, after the projection's fixed rotation too: the region
    /// stays in the same part of the map as the sphere turns, as a band of
    /// latitudes does.
    View,
}

impl SphericalClip {
    /// A region in `frame` from `west` to `east` and `south` to `north`, in
    /// degrees, whose north and south edges have a vertex a degree or less
    /// apart, so that they follow their parallels. The box crosses the
    /// antimeridian when `east` is less than `west`.
    ///
    /// # Errors
    /// Returns an error unless the longitudes lie from -180 to 180 and differ,
    /// and the latitudes lie from -90 to 90 with `south` below `north`.
    pub fn lon_lat_box(
        frame: RegionFrame,
        west: f64,
        south: f64,
        east: f64,
        north: f64,
    ) -> Result<Self, AvengerGeoError> {
        let longitude = |lon: f64| (-180.0..=180.0).contains(&lon);
        if !(longitude(west)
            && longitude(east)
            && west != east
            && (-90.0..=90.0).contains(&south)
            && (-90.0..=90.0).contains(&north)
            && south < north)
        {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "a longitude/latitude box needs longitudes from -180 to 180 that differ, and \
                 latitudes from -90 to 90 with south below north, not west {west}, south \
                 {south}, east {east}, north {north}"
            )));
        }
        let east = if east < west { east + 360.0 } else { east };
        let wrap = |lon: f64| if lon > 180.0 { lon - 360.0 } else { lon };
        let steps = ((east - west).ceil() as usize).max(1);
        let along = |i: usize| (east - west) * i as f64 / steps as f64;
        let mut ring = Vec::with_capacity(2 * steps + 3);
        // The south edge west to east, then the north edge east to west:
        // counter-clockwise, as RFC 7946 winds an exterior.
        ring.extend((0..=steps).map(|i| Coord {
            x: wrap(west + along(i)),
            y: south,
        }));
        ring.extend((0..=steps).map(|i| Coord {
            x: wrap(east - along(i)),
            y: north,
        }));
        ring.push(ring[0]);
        Ok(SphericalClip::Region {
            polygon: MultiPolygon(vec![Polygon::new(LineString(ring), vec![])]),
            frame,
        })
    }

    /// The clip as a pipeline stage's policy, with an earth region turned by
    /// `rotation`, or an error for an angle or a region that can't clip.
    fn prepare(&self, rotation: &Composed) -> Result<PreClip, AvengerGeoError> {
        match self {
            SphericalClip::Antimeridian => Ok(PreClip::Antimeridian),
            SphericalClip::Circle { angle } => {
                if !(angle.is_finite() && *angle > 0.0 && *angle < 180.0) {
                    return Err(AvengerGeoError::InvalidConfig(format!(
                        "a circle clip's angle must lie strictly between 0 and 180 degrees, not {angle}"
                    )));
                }
                Ok(PreClip::Circle(CirclePolicy::new(angle * RADIANS)))
            }
            SphericalClip::Region { polygon, frame } => {
                let policies = match frame {
                    RegionFrame::Earth => crate::clip::polygon::prepare(polygon, |lambda, phi| {
                        rotation.rotate(lambda, phi)
                    })?,
                    RegionFrame::View => {
                        crate::clip::polygon::prepare(polygon, |lambda, phi| (lambda, phi))?
                    }
                };
                Ok(PreClip::Region {
                    policies: Arc::new(policies),
                    turns: *frame == RegionFrame::Earth,
                })
            }
        }
    }
}

/// A spherical clip prepared for the pipeline.
#[derive(Debug, Clone)]
enum PreClip {
    Antimeridian,
    Circle(CirclePolicy),
    Region {
        policies: Arc<Vec<crate::clip::polygon::PolygonPolicy>>,
        /// Whether the region turns with the sphere: an earth region.
        turns: bool,
    },
}

impl PreClip {
    /// Whether the clip keeps a point of the rotated frame, in radians.
    fn keeps(&self, lambda: f64, phi: f64) -> bool {
        match self {
            PreClip::Antimeridian => true,
            PreClip::Circle(policy) => policy.point_visible(lambda, phi),
            PreClip::Region { policies, .. } => policies
                .iter()
                .any(|policy| policy.point_visible(lambda, phi)),
        }
    }

    /// Whether a turn of the sphere can bring a kept place to a point of the rotated frame, in
    /// radians: whether the clip keeps the point, except that an earth region turns with the
    /// places in it.
    fn reaches(&self, lambda: f64, phi: f64) -> bool {
        match self {
            PreClip::Region { turns: true, .. } => true,
            clip => clip.keeps(lambda, phi),
        }
    }
}

/// How [`Projection::pan`] and [`Projection::zoom`] move a map. Each keeps the
/// place under the cursor at the start of the gesture under the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanMode {
    /// Slide the view, and zoom about the cursor.
    Translate,
    /// Turn the sphere about its axis for horizontal movement, and slide the
    /// view for vertical movement, as world maps pan. A zoom keeps the view's
    /// horizontal translation and turns the sphere instead, so the
    /// antimeridian stays in place.
    Longitude,
    /// Turn the sphere the shortest way that carries the place to the cursor,
    /// as a trackball turns, which can roll it. A zoom keeps the view's
    /// translation and turns the place under the cursor to stay there.
    Sphere,
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
    /// Optional planar clip extent `[[x0, y0], [x1, y1]]` in output units,
    /// within ±1e9, where the rectangle clip clamps the points it draws, as
    /// d3's does.
    pub clip_extent: Option<[[f64; 2]; 2]>,
    /// The spherical clip, or `None` for the projection's default, which
    /// [`ProjectionImpl::clip`] gives. Planar projections ignore it.
    #[serde(default)]
    pub clip: Option<SphericalClip>,
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
            clip: None,
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

    /// Clip the sphere with `clip`, or with the projection's default for `None`.
    pub fn with_clip(mut self, clip: Option<SphericalClip>) -> Self {
        self.clip = clip;
        self
    }

    /// The pan mode that suits the projection: [`PanMode::Translate`] for a
    /// planar projection, [`PanMode::Sphere`] when the projection's default
    /// clip ([`ProjectionImpl::clip`]) is a circle, as an azimuthal
    /// projection's is, and [`PanMode::Longitude`] otherwise. A map's clip
    /// override doesn't change it.
    pub fn pan_mode(&self) -> PanMode {
        match &self.surface {
            Surface::Planar { .. } => PanMode::Translate,
            Surface::Spherical(projection) => match projection.clip() {
                SphericalClip::Circle { .. } => PanMode::Sphere,
                _ => PanMode::Longitude,
            },
        }
    }

    /// This projection, at the start of a drag, moved so that the place under
    /// `from`, where the drag started, lies under `to`, the cursor's position
    /// now, both in output units. Each frame of a drag pans the projection at
    /// its start, so the drag doesn't drift.
    ///
    /// - [`PanMode::Translate`] slides the view by `to - from`.
    /// - [`PanMode::Longitude`] slides the view vertically, and turns the
    ///   sphere by the longitude between the place under `from` and the place
    ///   under `to`. The place follows the cursor exactly when the projection's
    ///   y depends on latitude only, as a cylindrical or pseudocylindrical
    ///   projection's does with no latitude or roll angle, and approximately
    ///   otherwise. Where `from` or `to` misses the map, the turn is `(to[0] -
    ///   from[0]) / scale` radians.
    /// - [`PanMode::Sphere`] turns the sphere the shortest way that carries the
    ///   place to the cursor, as a trackball turns, so the place follows the
    ///   cursor anywhere on the globe. The turn can change all three rotation
    ///   angles, the roll among them. A cursor off the globe acts as the
    ///   nearest point on its edge toward its center, and a drag that starts
    ///   off the globe doesn't turn it. An earth region turns with the place,
    ///   so it doesn't limit the cursor.
    ///
    /// # Errors
    /// Returns an error when the projection has no view, for points that aren't
    /// finite, for [`PanMode::Longitude`] or [`PanMode::Sphere`] on a planar
    /// projection, and when the projection doesn't build.
    pub fn pan(
        &self,
        mode: PanMode,
        from: [f64; 2],
        to: [f64; 2],
    ) -> Result<Projection, AvengerGeoError> {
        crate::gesture::pan(self, mode, from, to)
    }

    /// This projection with its scale multiplied by `factor`, keeping the place
    /// under `anchor`, in output units, where it is. [`PanMode::Translate`]
    /// scales the view about the anchor, which keeps the place exactly.
    /// [`PanMode::Longitude`] keeps the view's horizontal translation, so the
    /// antimeridian stays where pans keep it. It scales the view vertically
    /// about the anchor and turns the sphere by the longitude between the place
    /// and the place that the scaled view puts under the anchor, which keeps
    /// the place exactly where a [`PanMode::Longitude`] pan does. Where the
    /// anchor misses the map, the turn is the cylindrical relation of the
    /// anchor's horizontal distance from the view's translation.
    /// [`PanMode::Sphere`] keeps the view's translation, so a globe stays in
    /// place, and turns the sphere so that the place stays under the anchor,
    /// as a [`PanMode::Sphere`] pan turns it.
    ///
    /// # Errors
    /// Returns the errors of [`Self::pan`], and an error for a factor that
    /// isn't finite and positive.
    pub fn zoom(
        &self,
        mode: PanMode,
        anchor: [f64; 2],
        factor: f64,
    ) -> Result<Projection, AvengerGeoError> {
        crate::gesture::zoom(self, mode, anchor, factor)
    }

    /// Build the runnable pipeline.
    ///
    /// # Errors
    /// Returns an error when the projection has no view, the view's scale isn't
    /// finite and positive or its translation isn't finite, the precision isn't
    /// finite and non-negative, the rotation or the center isn't finite, the
    /// center doesn't project to a finite point, the clip extent isn't a
    /// rectangle with positive width and height within ±1e9, the projection's
    /// bounds aren't a finite rectangle with positive width and height, the
    /// spherical clip can't clip, or [`ProjectionImpl::validate`] rejects the
    /// projection's parameters.
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
        if !self
            .rotate
            .iter()
            .chain(&self.center)
            .all(|v| v.is_finite())
        {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "the rotation and the center must be finite, not {:?} and {:?}",
                self.rotate, self.center
            )));
        }
        if self.clip_extent.is_some_and(|extent| {
            !is_rectangle(extent) || extent.iter().flatten().any(|v| v.abs() > CLIP_MAX)
        }) {
            return Err(AvengerGeoError::InvalidConfig(
                "the clip extent must be a rectangle with positive width and height within ±1e9"
                    .into(),
            ));
        }
        let (fixed, clip) = match &self.surface {
            Surface::Spherical(projection) => {
                projection.validate()?;
                let clip = self.clip.clone().unwrap_or_else(|| projection.clip());
                (Rotation::from_degrees(projection.rotation()), clip)
            }
            Surface::Planar { .. } => (Rotation::new(0.0, 0.0, 0.0), SphericalClip::Antimeridian),
        };
        let rotation = Composed::new(Rotation::from_degrees(self.rotate), fixed);
        let clip = clip.prepare(&rotation)?;
        // d3 recenter(): solve the transform so that the projected center
        // lands on the view's translation. The projection's fixed rotation
        // turns the center first, so a center is a longitude and latitude for
        // every projection.
        let center = match &self.surface {
            Surface::Spherical(projection) => {
                let (lambda, phi) =
                    fixed.rotate(self.center[0] * RADIANS, self.center[1] * RADIANS);
                projection.project(lambda, phi)
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
        if !(transform.dx.is_finite() && transform.dy.is_finite()) {
            return Err(AvengerGeoError::InvalidConfig(format!(
                "the center {:?} doesn't project to a finite point at this view",
                self.center
            )));
        }
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
            rotation,
            clip,
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
        for_each_row(geometry, Rings::Rewound, |row| {
            if let Some(row) = row {
                projector.run(&mut sink, |head| row.stream(head));
            }
        })?;
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
    /// The caller's rotation, then the projection's fixed rotation.
    rotation: Composed,
    clip: PreClip,
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

    /// The point of the caller's rotated frame, in radians, under `point` in
    /// output units: before the projection's fixed rotation. `None` where the
    /// point misses the map: off the projection's range, or outside its
    /// spherical clip.
    pub(crate) fn user_frame_at(&self, point: [f64; 2]) -> Option<(f64, f64)> {
        self.frame_at(point, PreClip::keeps)
    }

    /// The point of the caller's rotated frame under `point`, as
    /// [`Self::user_frame_at`] finds it, where a turn of the sphere can bring a
    /// place that the map shows: an earth region turns with its places, so only
    /// the projection's range and the clips fixed in the frame limit it.
    pub(crate) fn reachable_frame_at(&self, point: [f64; 2]) -> Option<(f64, f64)> {
        self.frame_at(point, PreClip::reaches)
    }

    /// The point of the caller's rotated frame under `point`, where it lies in
    /// the projection's range and `clip` holds for it in the rotated frame.
    fn frame_at(
        &self,
        point: [f64; 2],
        clip: impl Fn(&PreClip, f64, f64) -> bool,
    ) -> Option<(f64, f64)> {
        let Surface::Spherical(projection) = &self.surface else {
            return None;
        };
        let unit = self.transform.invert((point[0], point[1]));
        let (lambda, phi) = projection.invert(unit.0, unit.1)?;
        let on_sphere =
            lambda.abs() <= crate::math::PI + 1e-9 && phi.abs() <= crate::math::HALF_PI + 1e-9;
        let (x, y) = projection.project(lambda, phi);
        let round_trip = (x - unit.0).abs() <= 1e-6 && (y - unit.1).abs() <= 1e-6;
        (on_sphere && round_trip && clip(&self.clip, lambda, phi))
            .then(|| self.rotation.invert_second(lambda, phi))
    }

    /// The place under `point` in output units, as longitude and latitude in
    /// radians, or `None` where the point misses the map.
    pub(crate) fn place_at(&self, point: [f64; 2]) -> Option<(f64, f64)> {
        let (lambda, phi) = self.user_frame_at(point)?;
        Some(self.rotation.invert_first(lambda, phi))
    }

    /// The center of the rotated frame in output units, where an azimuthal
    /// projection's circle clip is centered.
    pub(crate) fn frame_origin(&self) -> [f64; 2] {
        let unit = match &self.surface {
            Surface::Spherical(projection) => projection.project(0.0, 0.0),
            Surface::Planar { .. } => (0.0, 0.0),
        };
        let (x, y) = self.transform.apply(unit);
        [x, y]
    }

    /// The rectangle that streamed geometry is clipped to, in output units: the
    /// projection's clip extent within its projection's bounds, or whichever of
    /// the two it has. Where the two don't overlap, the rectangle has no area,
    /// and nothing draws.
    pub fn clip_extent(&self) -> Option<[[f64; 2]; 2]> {
        self.clip_extent
    }

    /// `geometry` through the full pipeline, in output units with y down, as
    /// geo-types geometry for [`crate::paths`] to draw. Polygon rings stream as
    /// written, which RFC 7946 winds.
    ///
    /// A point gives a point, which has NaN coordinates when the clip leaves it
    /// out. A multipoint gives the points that the clip keeps. Lines give a
    /// multilinestring and polygons a multipolygon, since the antimeridian cut
    /// and the clip can split one into several. A collection gives a
    /// collection of the points, lines, and polygons that remain. In the
    /// output, exterior rings have positive signed area and holes negative.
    pub fn project(&self, geometry: &impl GeometryTrait<T = f64>) -> Geometry<f64> {
        let mut sink = Collector::default();
        self.project_to_sink(geometry, &mut sink);
        sink.into_geometry(Kind::of(geometry))
    }

    /// The whole sphere through the full pipeline: the area that the projection
    /// draws, clipped like any other geometry, in output units.
    pub fn project_sphere(&self) -> MultiPolygon<f64> {
        let mut sink = Collector::default();
        self.run(&mut sink, |head| head.sphere());
        sink.into_multi_polygon()
    }

    /// Project `geometry` through the full pipeline into `sink`, for outputs
    /// that geometry doesn't give.
    pub fn project_to_sink(&self, geometry: &impl GeometryTrait<T = f64>, sink: &mut dyn GeoSink) {
        self.run(sink, |head| walk(geometry, head, Rings::AsWritten));
    }

    /// Each row of a GeoArrow array, in any encoding, through the full
    /// pipeline, as [`Self::project`] projects one geometry. Rings are rewound
    /// to RFC 7946 by planar signed area first.
    ///
    /// The output has a row for each input row, and a null row stays null. It
    /// is a native GeoArrow array with separated XY coordinates and no CRS:
    /// points stay points, multipoints stay multipoints, lines become
    /// multilinestrings, and polygons and boxes become multipolygons. Mixed
    /// geometry, collections, WKB, and WKT become mixed geometry, whose rows
    /// follow their input rows' types. The output is in output units, so it
    /// must not be summarized or projected again.
    ///
    /// # Errors
    /// Returns an error for a row that can't be read.
    pub fn project_geometry(
        &self,
        geometry: &dyn GeoArrowArray,
    ) -> Result<Arc<dyn GeoArrowArray>, AvengerGeoError> {
        let mut builder = ScreenBuilder::for_type(&geometry.data_type());
        let mut pushed = Ok(());
        for_each_row(geometry, Rings::Rewound, |row| {
            if pushed.is_err() {
                return;
            }
            let projected = row.map(|row| {
                let mut sink = Collector::default();
                self.run(&mut sink, |head| row.stream(head));
                sink.into_geometry(row.kind())
            });
            pushed = builder.push(projected.as_ref());
        })?;
        pushed?;
        Ok(builder.finish())
    }

    /// Build the pipeline into `sink` and pass its first stage to `feed`.
    pub(crate) fn run(&self, sink: &mut dyn GeoSink, feed: impl FnOnce(&mut dyn GeoSink)) {
        let feed = |head: &mut dyn GeoSink| feed(&mut FiniteVertices::new(head));
        let sink = postclip(self.clip_extent, sink);
        match &self.surface {
            Surface::Spherical(projection) => {
                let projection = projection.as_ref();
                let transform = self.transform;
                let project = move |l: f64, p: f64| transform.apply(projection.project(l, p));
                let resample = Resample::new(project, self.delta2, sink);
                match &self.clip {
                    PreClip::Antimeridian => {
                        self.feed_clipped(Clip::new(AntimeridianPolicy, resample), feed)
                    }
                    PreClip::Circle(policy) => {
                        self.feed_clipped(Clip::new(*policy, resample), feed)
                    }
                    PreClip::Region { policies, .. } => self.feed_clipped(
                        Regions {
                            clips: policies
                                .iter()
                                .map(|policy| Clip::new(policy.clone(), Buffered::default()))
                                .collect(),
                            sink: resample,
                            in_polygon: false,
                            in_line: false,
                        },
                        feed,
                    ),
                }
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

impl Projector {
    /// Feed geometry through the rotation into `clip`.
    fn feed_clipped(&self, clip: impl GeoSink, feed: impl FnOnce(&mut dyn GeoSink)) {
        let rotate = RotateStream {
            rotation: self.rotation,
            sink: clip,
        };
        feed(&mut TransformRadians {
            sink: ReverseRings::spherical(rotate),
        });
    }
}

/// What one region polygon's clip emits, kept until the geometry it clips ends.
#[derive(Default)]
struct Buffered(Vec<Event>);

enum Event {
    Point(f64, f64, Option<f64>),
    LineStart,
    LineEnd,
    PolygonStart,
    PolygonEnd,
    Sphere,
}

impl GeoSink for Buffered {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        self.0.push(Event::Point(x, y, m));
    }
    fn line_start(&mut self) {
        self.0.push(Event::LineStart);
    }
    fn line_end(&mut self) {
        self.0.push(Event::LineEnd);
    }
    fn polygon_start(&mut self) {
        self.0.push(Event::PolygonStart);
    }
    fn polygon_end(&mut self) {
        self.0.push(Event::PolygonEnd);
    }
    fn sphere(&mut self) {
        self.0.push(Event::Sphere);
    }
}

/// A region's clips, one for each of its polygons, as d3-geo-polygon clips: every event goes to
/// each clip. A clip can emit part of a polygon before the polygon ends, so each clip writes to
/// its own buffer, and when a geometry ends, the buffers go to `sink` one clip after another.
/// d3-geo-polygon writes them to the stream as they come, which interleaves the clips' polygons.
struct Regions<S> {
    clips: Vec<Clip<crate::clip::polygon::PolygonPolicy, Buffered>>,
    sink: S,
    in_polygon: bool,
    in_line: bool,
}

impl<S: GeoSink> Regions<S> {
    fn flush(&mut self) {
        for clip in &mut self.clips {
            for event in clip.sink.0.drain(..) {
                match event {
                    Event::Point(x, y, m) => self.sink.point(x, y, m),
                    Event::LineStart => self.sink.line_start(),
                    Event::LineEnd => self.sink.line_end(),
                    Event::PolygonStart => self.sink.polygon_start(),
                    Event::PolygonEnd => self.sink.polygon_end(),
                    Event::Sphere => self.sink.sphere(),
                }
            }
        }
    }
}

impl<S: GeoSink> GeoSink for Regions<S> {
    fn point(&mut self, x: f64, y: f64, m: Option<f64>) {
        self.clips.iter_mut().for_each(|clip| clip.point(x, y, m));
        if !self.in_polygon && !self.in_line {
            self.flush();
        }
    }
    fn line_start(&mut self) {
        self.in_line = !self.in_polygon;
        self.clips.iter_mut().for_each(|clip| clip.line_start());
    }
    fn line_end(&mut self) {
        self.clips.iter_mut().for_each(|clip| clip.line_end());
        if !self.in_polygon {
            self.in_line = false;
            self.flush();
        }
    }
    fn polygon_start(&mut self) {
        self.in_polygon = true;
        self.clips.iter_mut().for_each(|clip| clip.polygon_start());
    }
    fn polygon_end(&mut self) {
        self.clips.iter_mut().for_each(|clip| clip.polygon_end());
        self.in_polygon = false;
        self.flush();
    }
    fn sphere(&mut self) {
        self.clips.iter_mut().for_each(|clip| clip.sphere());
        self.flush();
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
/// geometry through. On the sphere, it also leaves out the parts of a ring
/// without area: see [`without_spikes`].
struct ReverseRings<S> {
    sink: S,
    spherical: bool,
    in_polygon: bool,
    in_ring: bool,
    ring: Vec<(f64, f64, Option<f64>)>,
}

impl<S> ReverseRings<S> {
    /// For planar rings.
    fn new(sink: S) -> Self {
        ReverseRings {
            sink,
            spherical: false,
            in_polygon: false,
            in_ring: false,
            ring: Vec::new(),
        }
    }

    /// For rings of longitudes and latitudes in radians.
    fn spherical(sink: S) -> Self {
        ReverseRings {
            spherical: true,
            ..Self::new(sink)
        }
    }
}

/// How close two points of the sphere are, in radians, for a ring to repeat a
/// point.
const SAME_POINT: f64 = 1e-12;

/// Whether two points of the sphere, as longitudes and latitudes in radians,
/// are one point: a pole is one point at every longitude, and the
/// antimeridian is one meridian on either side.
fn same_point(a: (f64, f64, Option<f64>), b: (f64, f64, Option<f64>)) -> bool {
    use crate::math::{HALF_PI, TAU};
    if (a.1 - b.1).abs() > SAME_POINT {
        return false;
    }
    let turn = (a.0 - b.0).rem_euclid(TAU);
    a.1.abs() >= HALF_PI - SAME_POINT || turn.min(TAU - turn) <= SAME_POINT
}

/// `ring` without the parts that have no area: a point that repeats the one
/// before it, and a spike, a point between two that are one point. Data cut
/// for planar maps has both: Natural Earth's Antarctica runs along the
/// antimeridian to the south pole, along the pole to the other side, and back.
/// The antimeridian clip decides what a polygon covers by whether it contains
/// the south pole, which such a ring passes through, so the decision would
/// turn on rounding.
fn without_spikes(ring: &mut Vec<(f64, f64, Option<f64>)>) {
    let mut kept: Vec<(f64, f64, Option<f64>)> = Vec::with_capacity(ring.len());
    // Removing a spike can leave another, so the passes repeat until none removes a point.
    let mut changed = true;
    while changed && ring.len() >= 3 {
        changed = false;
        kept.clear();
        for &point in ring.iter() {
            if kept.last().is_some_and(|&last| same_point(last, point)) {
                changed = true;
                continue;
            }
            if kept.len() >= 2 && same_point(kept[kept.len() - 2], point) {
                kept.pop();
                changed = true;
                continue;
            }
            kept.push(point);
        }
        // Across the closing point: the last point against the first, and the
        // first two and last two as spikes.
        while kept.len() >= 3 {
            let n = kept.len();
            if same_point(kept[n - 1], kept[0]) || same_point(kept[n - 2], kept[0]) {
                kept.pop();
            } else if same_point(kept[n - 1], kept[1]) {
                kept.remove(0);
            } else {
                break;
            }
            changed = true;
        }
        std::mem::swap(ring, &mut kept);
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
        if self.spherical {
            without_spikes(&mut self.ring);
        }
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
    rotation: Composed,
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

    fn radians(points: &[(f64, f64)]) -> Vec<(f64, f64, Option<f64>)> {
        points
            .iter()
            .map(|&(lon, lat)| (lon * RADIANS, lat * RADIANS, None))
            .collect()
    }

    /// A ring cut at the antimeridian and the south pole loses its spike, and keeps its first
    /// point first.
    #[test]
    fn rings_leave_out_their_spikes() {
        let mut ring = radians(&[
            (180.0, -84.7),
            (180.0, -90.0),
            (-179.99999999999994, -90.0),
            (-179.99999999999994, -84.7),
            (-90.0, -70.0),
            (0.0, -70.0),
            (90.0, -70.0),
        ]);
        without_spikes(&mut ring);
        assert_eq!(
            ring,
            radians(&[(180.0, -84.7), (-90.0, -70.0), (0.0, -70.0), (90.0, -70.0)])
        );
    }

    /// Repeated points go, across the closing point too, and a spike there goes as well. A pole
    /// between two different points stays.
    #[test]
    fn rings_leave_out_repeated_points() {
        let mut ring = radians(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 0.0),
        ]);
        without_spikes(&mut ring);
        assert_eq!(ring, radians(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]));
        let mut ring = radians(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (5.0, 20.0),
            (10.0, 10.0),
        ]);
        without_spikes(&mut ring);
        assert_eq!(ring, radians(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]));
        let cap = radians(&[(0.0, 90.0), (0.0, 60.0), (120.0, 60.0)]);
        let mut ring = cap.clone();
        without_spikes(&mut ring);
        assert_eq!(ring, cap);
    }

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
