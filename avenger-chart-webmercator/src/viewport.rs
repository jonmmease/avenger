use std::any::Any;

use avenger_chart_core::{AvengerChartError, CoordMeasurement, DomainExtent};
use serde::{Deserialize, Serialize};

use crate::projection::{WEB_MERCATOR_LIMIT, units_per_pixel_for_zoom, zoom_for_units_per_pixel};
use crate::tiles::RasterTileLayer;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WebMercatorViewport {
    #[serde(default)]
    pub center_x: Option<f64>,
    #[serde(default)]
    pub center_y: Option<f64>,
    #[serde(default)]
    pub zoom: Option<f64>,
}

impl WebMercatorViewport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn center_projected(mut self, x: f64, y: f64) -> Self {
        self.center_x = Some(x);
        self.center_y = Some(y);
        self
    }

    pub fn center_lon_lat(self, lon: f64, lat: f64) -> Self {
        let projected = crate::projection::project_lon_lat(lon, lat);
        self.center_projected(projected.x, projected.y)
    }

    pub fn zoom(mut self, zoom: f64) -> Self {
        self.zoom = Some(zoom);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WebMercatorView {
    pub center_x: f64,
    pub center_y: f64,
    pub units_per_pixel: f64,
    pub zoom: f64,
    pub x_domain: (f64, f64),
    pub y_domain: (f64, f64),
    pub plot_width: f32,
    pub plot_height: f32,
}

impl WebMercatorView {
    pub fn new(
        center_x: f64,
        center_y: f64,
        units_per_pixel: f64,
        plot_width: f32,
        plot_height: f32,
    ) -> Self {
        let width = positive_dimension(plot_width);
        let height = positive_dimension(plot_height);
        let units_per_pixel = positive_resolution(units_per_pixel);
        let half_width = units_per_pixel * f64::from(width) / 2.0;
        let half_height = units_per_pixel * f64::from(height) / 2.0;
        Self {
            center_x,
            center_y,
            units_per_pixel,
            zoom: zoom_for_units_per_pixel(units_per_pixel),
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
    ) -> Self {
        Self::new(
            center_x,
            center_y,
            units_per_pixel_for_zoom(zoom),
            plot_width,
            plot_height,
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WebMercatorCoordMeasurement {
    pub viewport_id: String,
    pub view: WebMercatorView,
    pub tile_layers: Vec<RasterTileLayer>,
}

impl WebMercatorCoordMeasurement {
    pub fn downcast(measurement: &dyn CoordMeasurement) -> Option<&Self> {
        measurement.as_any().downcast_ref::<Self>()
    }

    pub fn downcast_mut(measurement: &mut dyn CoordMeasurement) -> Option<&mut Self> {
        measurement.as_any_mut().downcast_mut::<Self>()
    }
}

impl CoordMeasurement for WebMercatorCoordMeasurement {
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
) -> Result<WebMercatorView, AvengerChartError> {
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
                "WebMercator zoom must be finite, got {zoom}"
            )));
        }
        units_per_pixel_for_zoom(zoom)
    } else if let Some(bounds) = data_bounds {
        if authoring.center_x.is_some() && authoring.center_y.is_some() {
            bounds.units_per_pixel_around_center(center, width, height)
        } else {
            bounds.fit_units_per_pixel(width, height)
        }
    } else {
        world_units_per_pixel(width, height)
    };

    Ok(WebMercatorView::new(
        center.0,
        center.1,
        units_per_pixel,
        width,
        height,
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
        positive_resolution((x_span / f64::from(plot_width)).max(y_span / f64::from(plot_height)))
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
        positive_resolution(
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
            "WebMercator x domain must be numeric".to_string(),
        ));
    };
    let Some((y_min, y_max)) = y_extent.numeric_bounds() else {
        return Err(AvengerChartError::InvalidArgument(
            "WebMercator y domain must be numeric".to_string(),
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

fn world_units_per_pixel(plot_width: f32, plot_height: f32) -> f64 {
    let world_span = 2.0 * WEB_MERCATOR_LIMIT;
    positive_resolution(
        (world_span / f64::from(plot_width)).max(world_span / f64::from(plot_height)),
    )
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

fn positive_resolution(units_per_pixel: f64) -> f64 {
    if units_per_pixel.is_finite() && units_per_pixel > 0.0 {
        units_per_pixel
    } else {
        world_units_per_pixel(1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::DomainExtent;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn infers_center_and_resolution_from_data_bounds() {
        let x = DomainExtent::numeric(-10.0, 30.0);
        let y = DomainExtent::numeric(-20.0, 20.0);
        let view =
            realize_view(ViewAuthoring::default(), Some(&x), Some(&y), 200.0, 100.0).expect("view");
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
        )
        .expect("view");
        assert_close(view.units_per_pixel, 0.6);
        assert_close(view.x_domain.0, -30.0);
        assert_close(view.x_domain.1, 30.0);
    }

    #[test]
    fn fixed_zoom_infers_center_from_data() {
        let x = DomainExtent::numeric(20.0, 30.0);
        let y = DomainExtent::numeric(0.0, 10.0);
        let view = realize_view(
            ViewAuthoring {
                center_x: None,
                center_y: None,
                zoom: Some(0.0),
            },
            Some(&x),
            Some(&y),
            256.0,
            256.0,
        )
        .expect("view");
        assert_close(view.center_x, 25.0);
        assert_close(view.center_y, 5.0);
    }

    #[test]
    fn degenerate_data_bounds_get_positive_span() {
        let x = DomainExtent::numeric(10.0, 10.0);
        let y = DomainExtent::numeric(20.0, 20.0);
        let view =
            realize_view(ViewAuthoring::default(), Some(&x), Some(&y), 100.0, 100.0).expect("view");
        assert_close(view.center_x, 10.0);
        assert_close(view.center_y, 20.0);
        assert!(view.units_per_pixel > 0.0);
        assert!(view.x_domain.0 < 10.0 && view.x_domain.1 > 10.0);
        assert!(view.y_domain.0 < 20.0 && view.y_domain.1 > 20.0);
    }

    #[test]
    fn empty_data_defaults_to_world_fit() {
        let view = realize_view(ViewAuthoring::default(), None, None, 256.0, 128.0).expect("view");
        assert_close(view.center_x, 0.0);
        assert_close(view.center_y, 0.0);
        assert_close(view.y_domain.0, -WEB_MERCATOR_LIMIT);
        assert_close(view.y_domain.1, WEB_MERCATOR_LIMIT);
    }
}
