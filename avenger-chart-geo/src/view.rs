//! View realization for the `Geo` coordinate system.
//!
//! Mirrors `avenger-chart-webmercator/src/viewport.rs`, generalized: the
//! view lives in the authored projection's *raw planar units* (the space
//! produced by [`avenger_geo::Projection::project_raw_units`], y-up), and
//! the world fallback fits the projection's own sphere bounds instead of a
//! fixed mercator limit.

use std::any::Any;

use avenger_chart_core::{AvengerChartError, CoordMeasurement, DomainExtent};
use avenger_geo::projector::Projection;
use serde::{Deserialize, Serialize};

use crate::coord::{GraticuleStyle, SphereStyle};

/// Tile-size denominator of the zoom convention (matches the slippy-map /
/// WebMercator convention so mercator-projection zoom levels line up).
const ZOOM_TILE_SIZE: f64 = 256.0;

/// The projected span of the whole sphere in raw planar units, used for
/// zoom conversion and the empty-data world fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldSpan {
    pub width: f64,
    pub height: f64,
}

pub fn world_span(projection: &Projection) -> WorldSpan {
    // Two candidate world extents:
    // 1. The true sphere-outline bounds — correct for pole-bounded world
    //    projections (equal earth reaches ±90°), but conformal conics blow
    //    up toward the antipodal pole.
    // 2. A latitude-clamped (±85°) lon/lat sample grid — always sane, and
    //    for mercator it matches the WebMercator convention exactly (the
    //    2π×2π world square).
    // Use the sphere bounds when they are within a sanity factor of the
    // clamped bounds; otherwise fall back to the clamped grid.
    let clamped = clamped_grid_span(projection);
    let sphere = sphere_outline_span(projection);
    match sphere {
        Some(sphere)
            if sphere.width <= 4.0 * clamped.width && sphere.height <= 4.0 * clamped.height =>
        {
            sphere
        }
        _ => clamped,
    }
}

fn sphere_outline_span(projection: &Projection) -> Option<WorldSpan> {
    use avenger_geo::projector::BoundsSink;
    use avenger_geo::streamable::Sphere;
    let reference = Projection {
        scale: 1.0,
        translate: [0.0, 0.0],
        center: [0.0, 0.0],
        clip_extent: None,
        ..projection.clone()
    };
    let projector = reference.build();
    let mut bounds = BoundsSink::default();
    projector.stream(&Sphere, &mut bounds);
    let [[x0, y0], [x1, y1]] = bounds.result()?;
    if [x0, y0, x1, y1].into_iter().all(f64::is_finite) && x1 > x0 && y1 > y0 {
        Some(WorldSpan {
            width: x1 - x0,
            height: y1 - y0,
        })
    } else {
        None
    }
}

fn clamped_grid_span(projection: &Projection) -> WorldSpan {
    let mut xmin = f64::INFINITY;
    let mut xmax = f64::NEG_INFINITY;
    let mut ymin = f64::INFINITY;
    let mut ymax = f64::NEG_INFINITY;
    let mut lon = -180.0_f64;
    while lon <= 180.0 {
        let mut lat = -85.0_f64;
        while lat <= 85.0 {
            let (x, y) = projection.project_raw_units(lon, lat);
            if x.is_finite() && y.is_finite() {
                xmin = xmin.min(x);
                xmax = xmax.max(x);
                ymin = ymin.min(y);
                ymax = ymax.max(y);
            }
            lat += 5.0;
        }
        lon += 5.0;
    }
    if xmin.is_finite() && xmax > xmin && ymax > ymin {
        WorldSpan {
            width: positive_span(xmax - xmin),
            height: positive_span(ymax - ymin),
        }
    } else {
        WorldSpan {
            width: 2.0 * std::f64::consts::PI,
            height: 2.0 * std::f64::consts::PI,
        }
    }
}

/// Zoom ↔ resolution conversion: `zoom` follows the slippy-map convention
/// generalized by the projection's world width (`upp = world_width /
/// (256 · 2^zoom)`); for mercator this matches the WebMercator zoom scale
/// up to the raw-units/meters factor.
pub fn units_per_pixel_for_zoom(zoom: f64, world: WorldSpan) -> f64 {
    world.width.max(world.height) / (ZOOM_TILE_SIZE * 2.0_f64.powf(zoom))
}

pub fn zoom_for_units_per_pixel(units_per_pixel: f64, world: WorldSpan) -> f64 {
    (world.width.max(world.height) / (ZOOM_TILE_SIZE * units_per_pixel)).log2()
}

/// Authorable view state (mirrors `WebMercatorViewport`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GeoViewport {
    #[serde(default)]
    pub center_x: Option<f64>,
    #[serde(default)]
    pub center_y: Option<f64>,
    #[serde(default)]
    pub zoom: Option<f64>,
}

impl GeoViewport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Center in raw projected units.
    pub fn center_projected(mut self, x: f64, y: f64) -> Self {
        self.center_x = Some(x);
        self.center_y = Some(y);
        self
    }

    pub fn zoom(mut self, zoom: f64) -> Self {
        self.zoom = Some(zoom);
        self
    }
}

/// A realized view: center/resolution plus derived domains in raw units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GeoView {
    pub center_x: f64,
    pub center_y: f64,
    pub units_per_pixel: f64,
    pub zoom: f64,
    /// Raw-unit x domain (west → east).
    pub x_domain: (f64, f64),
    /// Raw-unit y domain (south → north; range binding flips to pixels).
    pub y_domain: (f64, f64),
    pub plot_width: f32,
    pub plot_height: f32,
}

impl GeoView {
    pub fn new(
        center_x: f64,
        center_y: f64,
        units_per_pixel: f64,
        plot_width: f32,
        plot_height: f32,
        world: WorldSpan,
    ) -> Self {
        let width = positive_dimension(plot_width);
        let height = positive_dimension(plot_height);
        let units_per_pixel = positive_resolution(units_per_pixel, world);
        let half_width = units_per_pixel * f64::from(width) / 2.0;
        let half_height = units_per_pixel * f64::from(height) / 2.0;
        Self {
            center_x,
            center_y,
            units_per_pixel,
            zoom: zoom_for_units_per_pixel(units_per_pixel, world),
            x_domain: (center_x - half_width, center_x + half_width),
            y_domain: (center_y - half_height, center_y + half_height),
            plot_width: width,
            plot_height: height,
        }
    }

    pub fn from_center_zoom(
        center_x: f64,
        center_y: f64,
        zoom: f64,
        plot_width: f32,
        plot_height: f32,
        world: WorldSpan,
    ) -> Self {
        Self::new(
            center_x,
            center_y,
            units_per_pixel_for_zoom(zoom, world),
            plot_width,
            plot_height,
            world,
        )
    }
}

/// The coordinate measurement handed to the guide and tools.
#[derive(Clone, Debug, PartialEq)]
pub struct GeoCoordMeasurement {
    pub viewport_id: String,
    pub view: GeoView,
    /// The authored projection (kind + rotate + precision); scale/translate
    /// are not meaningful here — consumers build view projectors via
    /// [`avenger_geo::Projection::build_view`].
    pub projection: Projection,
    pub graticule: Option<GraticuleStyle>,
    pub sphere: Option<SphereStyle>,
}

impl GeoCoordMeasurement {
    pub fn downcast(measurement: &dyn CoordMeasurement) -> Option<&Self> {
        measurement.as_any().downcast_ref::<Self>()
    }

    pub fn downcast_mut(measurement: &mut dyn CoordMeasurement) -> Option<&mut Self> {
        measurement.as_any_mut().downcast_mut::<Self>()
    }

    /// The pixel-space projector for this measurement's view.
    pub fn view_projector(&self) -> avenger_geo::projector::Projector {
        self.projection.build_view(
            (self.view.center_x, self.view.center_y),
            self.view.units_per_pixel,
            f64::from(self.view.plot_width),
            f64::from(self.view.plot_height),
        )
    }
}

impl CoordMeasurement for GeoCoordMeasurement {
    fn clone_box(&self) -> Box<dyn CoordMeasurement> {
        Box::new(self.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ViewAuthoring {
    pub center_x: Option<f64>,
    pub center_y: Option<f64>,
    pub zoom: Option<f64>,
}

pub(crate) fn realize_view(
    authoring: ViewAuthoring,
    x_extent: Option<&DomainExtent>,
    y_extent: Option<&DomainExtent>,
    plot_width: f32,
    plot_height: f32,
    world: WorldSpan,
) -> Result<GeoView, AvengerChartError> {
    let data_bounds = numeric_bounds_pair(x_extent, y_extent)?;
    let width = positive_dimension(plot_width);
    let height = positive_dimension(plot_height);

    let center = authoring
        .center_x
        .zip(authoring.center_y)
        .or_else(|| data_bounds.map(|bounds| bounds.center()))
        .unwrap_or((0.0, 0.0));

    let units_per_pixel = if let Some(zoom) = authoring.zoom {
        if !zoom.is_finite() {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Geo zoom must be finite, got {zoom}"
            )));
        }
        units_per_pixel_for_zoom(zoom, world)
    } else if let Some(bounds) = data_bounds {
        if authoring.center_x.is_some() && authoring.center_y.is_some() {
            bounds.units_per_pixel_around_center(center, width, height)
        } else {
            bounds.fit_units_per_pixel(width, height)
        }
    } else {
        world_units_per_pixel(width, height, world)
    };

    Ok(GeoView::new(
        center.0,
        center.1,
        units_per_pixel,
        width,
        height,
        world,
    ))
}

#[derive(Clone, Copy, Debug)]
struct ProjectedBounds {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
}

impl ProjectedBounds {
    fn center(self) -> (f64, f64) {
        (
            (self.x_min + self.x_max) / 2.0,
            (self.y_min + self.y_max) / 2.0,
        )
    }

    fn fit_units_per_pixel(self, plot_width: f32, plot_height: f32) -> f64 {
        let x_span = positive_span(self.x_max - self.x_min);
        let y_span = positive_span(self.y_max - self.y_min);
        positive_fit((x_span / f64::from(plot_width)).max(y_span / f64::from(plot_height)))
    }

    fn units_per_pixel_around_center(
        self,
        center: (f64, f64),
        plot_width: f32,
        plot_height: f32,
    ) -> f64 {
        let x_half = (self.x_min - center.0)
            .abs()
            .max((self.x_max - center.0).abs());
        let y_half = (self.y_min - center.1)
            .abs()
            .max((self.y_max - center.1).abs());
        positive_fit(
            (2.0 * positive_span(x_half) / f64::from(plot_width))
                .max(2.0 * positive_span(y_half) / f64::from(plot_height)),
        )
    }
}

fn numeric_bounds_pair(
    x_extent: Option<&DomainExtent>,
    y_extent: Option<&DomainExtent>,
) -> Result<Option<ProjectedBounds>, AvengerChartError> {
    let (Some(x_extent), Some(y_extent)) = (x_extent, y_extent) else {
        return Ok(None);
    };
    let Some((x_min, x_max)) = x_extent.numeric_bounds() else {
        return Err(AvengerChartError::InvalidArgument(
            "Geo x domain must be numeric".to_string(),
        ));
    };
    let Some((y_min, y_max)) = y_extent.numeric_bounds() else {
        return Err(AvengerChartError::InvalidArgument(
            "Geo y domain must be numeric".to_string(),
        ));
    };
    if [x_min, x_max, y_min, y_max].into_iter().all(f64::is_finite) {
        Ok(Some(ProjectedBounds {
            x_min: x_min.min(x_max),
            x_max: x_min.max(x_max),
            y_min: y_min.min(y_max),
            y_max: y_min.max(y_max),
        }))
    } else {
        Ok(None)
    }
}

fn world_units_per_pixel(plot_width: f32, plot_height: f32, world: WorldSpan) -> f64 {
    positive_fit((world.width / f64::from(plot_width)).max(world.height / f64::from(plot_height)))
}

fn positive_dimension(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        1.0
    }
}

fn positive_span(span: f64) -> f64 {
    if span.is_finite() && span.abs() > 0.0 {
        span.abs()
    } else {
        1.0
    }
}

fn positive_fit(units_per_pixel: f64) -> f64 {
    if units_per_pixel.is_finite() && units_per_pixel > 0.0 {
        units_per_pixel
    } else {
        1.0
    }
}

fn positive_resolution(units_per_pixel: f64, world: WorldSpan) -> f64 {
    if units_per_pixel.is_finite() && units_per_pixel > 0.0 {
        units_per_pixel
    } else {
        world_units_per_pixel(1.0, 1.0, world)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_geo::raw::ProjectionKind;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    fn test_world() -> WorldSpan {
        WorldSpan {
            width: 100.0,
            height: 50.0,
        }
    }

    #[test]
    fn infers_center_and_resolution_from_data_bounds() {
        let x = DomainExtent::numeric(-10.0, 30.0);
        let y = DomainExtent::numeric(-20.0, 20.0);
        let view = realize_view(
            ViewAuthoring::default(),
            Some(&x),
            Some(&y),
            200.0,
            100.0,
            test_world(),
        )
        .expect("view");
        assert_close(view.center_x, 10.0);
        assert_close(view.center_y, 0.0);
        assert_close(view.units_per_pixel, 0.4);
        assert_close(view.x_domain.0, -30.0);
        assert_close(view.x_domain.1, 50.0);
        assert_close(view.y_domain.0, -20.0);
        assert_close(view.y_domain.1, 20.0);
    }

    #[test]
    fn fixed_center_infers_zoom_that_contains_data() {
        let x = DomainExtent::numeric(20.0, 30.0);
        let y = DomainExtent::numeric(0.0, 10.0);
        let view = realize_view(
            ViewAuthoring {
                center_x: Some(0.0),
                center_y: Some(0.0),
                zoom: None,
            },
            Some(&x),
            Some(&y),
            100.0,
            100.0,
            test_world(),
        )
        .expect("view");
        assert_close(view.units_per_pixel, 0.6);
        assert_close(view.x_domain.0, -30.0);
        assert_close(view.x_domain.1, 30.0);
    }

    #[test]
    fn empty_data_defaults_to_world_fit() {
        let world = test_world();
        let view =
            realize_view(ViewAuthoring::default(), None, None, 200.0, 100.0, world).expect("view");
        assert_close(view.center_x, 0.0);
        assert_close(view.center_y, 0.0);
        // World is 100 wide / 50 tall into 200x100 plot: 0.5 units/px.
        assert_close(view.units_per_pixel, 0.5);
        assert_close(view.x_domain.0, -50.0);
        assert_close(view.x_domain.1, 50.0);
    }

    #[test]
    fn zoom_round_trips() {
        let world = test_world();
        for zoom in [0.0, 1.0, 4.5, 12.0] {
            let upp = units_per_pixel_for_zoom(zoom, world);
            assert_close(zoom_for_units_per_pixel(upp, world), zoom);
        }
    }

    #[test]
    fn world_span_of_equirectangular() {
        let world = world_span(&Projection::new(ProjectionKind::Equirectangular));
        // Sphere-outline bounds: 2π wide, π tall.
        assert_close(world.width, 2.0 * std::f64::consts::PI);
        assert_close(world.height, std::f64::consts::PI);
        // Conformal conics fall back to the clamped grid (sane span).
        let cc = world_span(&Projection::new(ProjectionKind::ConicConformal {
            parallels: (35.0, 65.0),
        }));
        assert!(cc.width.max(cc.height) < 100.0, "cc span {cc:?}");
    }

    #[test]
    fn mercator_zoom_matches_webmercator_convention() {
        // For mercator, raw world width is 2π and the WebMercator formula is
        // 2·π·R / (256·2^z) in meters; ratio must be exactly R at every zoom.
        let world = world_span(&Projection::new(ProjectionKind::Mercator));
        assert_close(world.width, 2.0 * std::f64::consts::PI);
        const EARTH_RADIUS_M: f64 = 6_378_137.0;
        for zoom in [0.0, 3.0, 7.5] {
            let ours = units_per_pixel_for_zoom(zoom, world);
            let webmercator =
                2.0 * std::f64::consts::PI * EARTH_RADIUS_M / (256.0 * 2.0_f64.powf(zoom));
            assert_close(webmercator / ours, EARTH_RADIUS_M);
        }
    }
}
