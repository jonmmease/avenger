//! The `Geo` coordinate system: general map projections for charts.
//!
//! Architecture (scratch/geo/README.md decision 1): a direct generalization
//! of `avenger-chart-webmercator` — positions flow as *raw projected planar
//! units* through coordinate-owned x/y linear domains driven by
//! center/units-per-pixel params; marks author positions via
//! `.longitude()`/`.latitude()` builders backed by the `geo_project` UDF;
//! resampled geometry (graticules, geodesic lines, GeoShape) streams
//! render-side through [`avenger_geo`]'s projection pipeline.

use std::{any::Any, collections::HashMap};

use avenger_chart_core::{
    AvengerChartError, CoordinateDomainBinding, CoordinateDomainCellRequest,
    CoordinateDomainCellResolution, CoordinateDomainDescriptor, CoordinateDomainGroupRequest,
    CoordinateDomainGroupResolution, CoordinateDomainMaterialization, CoordinateDomainProvider,
    CoordinateDomainRole, CoordinateDomainScaleState, CoordinateDomainScaleType,
    CoordinateDomainSharingPolicy, CoordinateMeasureRequest, CoordinateMeasurementProvider,
    CoordinateMetricDescriptor, CoordinateSystem, CoordinateSystemCore, CoordinateSystemTransform,
    CoordinateSystemTransformCore, DomainExtent, InteractionPointInversionRequest,
    PlotAreaRangeEndpoint, PlotGeometry, PointGeometry, ScaleRangeBinding,
};
use avenger_common::value::ScalarOrArray;
use avenger_geo::projector::Projection;
use avenger_geo::raw::ProjectionKind;
use avenger_scales::scales::ConfiguredScale;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    guide::GeoGuide,
    tiles::RasterTileLayer,
    view::{GeoCoordMeasurement, GeoViewport, ViewAuthoring, realize_view, world_span},
};

const GEO_DESCRIPTOR_ID: &str = "geo_viewport";
const GEO_METRIC_ID: &str = "geo_projected_units";

/// Adaptive Web Mercator blend configuration (doc §8.3): as the view zooms
/// past `z0`, the authored projection blends pointwise toward Mercator,
/// reaching pure Mercator at `z1`. Anchored at the view center so
/// position, scale, and north stay fixed there for every `t`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlendConfig {
    /// Zoom at (and below) which the authored projection shows pure.
    pub z0: f64,
    /// Zoom at (and above) which the view is pure Mercator.
    pub z1: f64,
    /// Test hook: force the blend parameter regardless of zoom.
    #[serde(default)]
    pub force_t: Option<f64>,
}

impl Default for BlendConfig {
    fn default() -> Self {
        BlendConfig {
            z0: 4.0,
            z1: 7.0,
            force_t: None,
        }
    }
}

/// Graticule styling carried on the coordinate system.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraticuleStyle {
    /// Degrees between graticule lines `[lon_step, lat_step]`.
    pub step: [f64; 2],
    /// Stroke RGBA (0..1).
    pub stroke: [f32; 4],
    pub stroke_width: f32,
}

impl Default for GraticuleStyle {
    fn default() -> Self {
        GraticuleStyle {
            step: [10.0, 10.0],
            stroke: [0.0, 0.0, 0.0, 0.18],
            stroke_width: 0.7,
        }
    }
}

/// Sphere-outline styling (the projection's world boundary).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SphereStyle {
    /// Fill RGBA (0..1).
    pub fill: [f32; 4],
    /// Stroke RGBA (0..1).
    pub stroke: [f32; 4],
    pub stroke_width: f32,
}

impl Default for SphereStyle {
    fn default() -> Self {
        SphereStyle {
            fill: [0.94, 0.97, 1.0, 1.0],
            stroke: [0.45, 0.45, 0.45, 1.0],
            stroke_width: 1.0,
        }
    }
}

/// The `Geo` coordinate system.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Geo {
    #[serde(default = "default_viewport_id")]
    viewport_id: String,
    #[serde(default)]
    kind: ProjectionKind,
    #[serde(default = "default_rotate")]
    rotate: [f64; 3],
    /// Resampling precision in pixels (default `sqrt(0.5)`).
    #[serde(default)]
    precision: Option<f64>,
    #[serde(default)]
    center_x: Option<f64>,
    #[serde(default)]
    center_y: Option<f64>,
    #[serde(default)]
    zoom: Option<f64>,
    #[serde(default)]
    graticule: Option<GraticuleStyle>,
    #[serde(default)]
    sphere: Option<SphereStyle>,
    #[serde(default)]
    blend: Option<BlendConfig>,
    #[serde(default)]
    tile_layers: Vec<RasterTileLayer>,
}

impl Geo {
    /// Equal Earth, the default projection.
    pub fn new() -> Self {
        Self {
            viewport_id: default_viewport_id(),
            kind: ProjectionKind::default(),
            rotate: default_rotate(),
            precision: None,
            center_x: None,
            center_y: None,
            zoom: None,
            graticule: None,
            sphere: None,
            blend: None,
            tile_layers: Vec::new(),
        }
    }

    pub fn with_kind(kind: ProjectionKind) -> Self {
        Self {
            kind,
            ..Self::new()
        }
    }

    pub fn equal_earth() -> Self {
        Self::with_kind(ProjectionKind::EqualEarth)
    }

    pub fn natural_earth() -> Self {
        Self::with_kind(ProjectionKind::NaturalEarth1)
    }

    pub fn winkel_tripel() -> Self {
        Self::with_kind(ProjectionKind::WinkelTripel)
    }

    pub fn equirectangular() -> Self {
        Self::with_kind(ProjectionKind::Equirectangular)
    }

    pub fn mercator() -> Self {
        Self::with_kind(ProjectionKind::Mercator)
    }

    /// Conic equal-area with the standard CONUS Albers aspect
    /// (parallels 29.5°/45.5°, central meridian 96°W).
    pub fn albers_usa_conus() -> Self {
        Self::with_kind(ProjectionKind::albers()).rotate([96.0, 0.0, 0.0])
    }

    pub fn conic_conformal(parallels: (f64, f64)) -> Self {
        Self::with_kind(ProjectionKind::ConicConformal { parallels })
    }

    pub fn conic_equal_area(parallels: (f64, f64)) -> Self {
        Self::with_kind(ProjectionKind::ConicEqualArea { parallels })
    }

    pub fn projection_kind(&self) -> &ProjectionKind {
        &self.kind
    }

    pub fn viewport_id(mut self, id: impl Into<String>) -> Self {
        self.viewport_id = id.into();
        self
    }

    /// Three-axis spherical rotation `[λ, φ, γ]` in degrees.
    pub fn rotate(mut self, rotate: [f64; 3]) -> Self {
        self.rotate = rotate;
        self
    }

    /// Resampling precision in pixels (0 disables adaptive resampling).
    pub fn precision(mut self, precision: f64) -> Self {
        self.precision = Some(precision);
        self
    }

    /// View center in raw projected units.
    pub fn center_projected(mut self, x: f64, y: f64) -> Self {
        self.center_x = Some(x);
        self.center_y = Some(y);
        self
    }

    /// View center by geographic coordinate.
    pub fn center_lon_lat(self, lon: f64, lat: f64) -> Self {
        let (x, y) = self.projection().project_raw_units(lon, lat);
        self.center_projected(x, y)
    }

    /// Slippy-style zoom level (world width / (256 · 2^zoom) resolution).
    pub fn zoom(mut self, zoom: f64) -> Self {
        self.zoom = Some(zoom);
        self
    }

    pub fn viewport(mut self, viewport: GeoViewport) -> Self {
        self.center_x = viewport.center_x;
        self.center_y = viewport.center_y;
        self.zoom = viewport.zoom;
        self
    }

    /// Draw a graticule under the marks.
    pub fn graticule(mut self, style: GraticuleStyle) -> Self {
        self.graticule = Some(style);
        self
    }

    /// Draw the projection's sphere outline / world background.
    pub fn sphere(mut self, style: SphereStyle) -> Self {
        self.sphere = Some(style);
        self
    }

    /// Enable the adaptive Web Mercator blend: zooming in morphs the
    /// authored projection into Mercator so street-level interaction
    /// behaves like a slippy map (doc §8.3). Opt-in.
    pub fn adaptive_blend(mut self, config: BlendConfig) -> Self {
        self.blend = Some(config);
        self
    }

    /// Add a raster tile layer, drawn under the marks and warped through
    /// the projection (plain slippy tiles on unrotated Mercator).
    pub fn tiles(mut self, layer: RasterTileLayer) -> Self {
        self.tile_layers.push(layer);
        self
    }

    /// The authored [`avenger_geo::Projection`] (kind + rotate + precision;
    /// scale/translate are view state and stay at defaults).
    pub fn projection(&self) -> Projection {
        let mut projection = Projection::new(self.kind.clone()).with_rotate(self.rotate);
        if let Some(precision) = self.precision {
            projection = projection.with_precision(precision);
        }
        projection
    }

    /// Invert raw projected units to (lon, lat) degrees where defined.
    pub fn unproject(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        self.projection().invert_raw_units(x, y)
    }

    pub fn center_x_param(&self) -> String {
        format!("__geo_{}_center_x", self.viewport_id)
    }

    pub fn center_y_param(&self) -> String {
        format!("__geo_{}_center_y", self.viewport_id)
    }

    pub fn units_per_pixel_param(&self) -> String {
        format!("__geo_{}_units_per_pixel", self.viewport_id)
    }

    pub fn focus_x_param(&self) -> String {
        format!("__geo_{}_focus_x", self.viewport_id)
    }

    pub fn focus_y_param(&self) -> String {
        format!("__geo_{}_focus_y", self.viewport_id)
    }

    fn authored_view(&self, params: &IndexMap<String, ScalarValue>) -> ViewAuthoring {
        let world = world_span(&self.projection());
        let center_x = param_f64(params, &self.center_x_param()).or(self.center_x);
        let center_y = param_f64(params, &self.center_y_param()).or(self.center_y);
        let zoom = param_f64(params, &self.units_per_pixel_param())
            .map(|upp| crate::view::zoom_for_units_per_pixel(upp, world))
            .or(self.zoom);
        ViewAuthoring {
            center_x,
            center_y,
            zoom,
        }
    }
}

impl Default for Geo {
    fn default() -> Self {
        Self::new()
    }
}

impl CoordinateSystemCore for Geo {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn validate(&self) -> Result<(), AvengerChartError> {
        if self.viewport_id.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "Geo viewport_id must not be empty".to_string(),
            ));
        }
        for (label, value) in [
            ("center_x", self.center_x),
            ("center_y", self.center_y),
            ("zoom", self.zoom),
            ("precision", self.precision),
        ] {
            if let Some(value) = value
                && !value.is_finite()
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "Geo {label} must be finite, got {value}"
                )));
            }
        }
        if let Some(precision) = self.precision
            && precision < 0.0
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Geo precision must be non-negative, got {precision}"
            )));
        }
        if let Some(graticule) = &self.graticule
            && (graticule.step[0] <= 0.0 || graticule.step[1] <= 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Geo graticule steps must be positive, got {:?}",
                graticule.step
            )));
        }
        for layer in &self.tile_layers {
            layer.validate()?;
        }
        Ok(())
    }
}

impl CoordinateSystem for Geo {
    type Guide = GeoGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for Geo {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        let _ = position_values;
        let x = position_channels
            .get("x")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing x position channel".to_string())
            })?
            .clone();
        let y = position_channels
            .get("y")
            .ok_or_else(|| {
                AvengerChartError::InternalError("Missing y position channel".to_string())
            })?
            .clone();
        Ok(Box::new(PointGeometry { x, y }))
    }

    fn interaction_frame(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
    ) -> Option<[f64; 4]> {
        // Only meaningful when the adaptive blend can be active.
        self.blend?;
        // Reconstruct the realized view from the evaluated x/y domains
        // (the domains ARE the view: symmetric around the center at
        // units_per_pixel resolution).
        let (x_min, x_max) = scales.get("x")?.numeric_interval_domain().ok()?;
        let (y_min, y_max) = scales.get("y")?.numeric_interval_domain().ok()?;
        if plot_width <= 0.0 || plot_height <= 0.0 {
            return None;
        }
        let center_x = f64::from(x_min + x_max) / 2.0;
        let center_y = f64::from(y_min + y_max) / 2.0;
        let units_per_pixel = f64::from(x_max - x_min) / f64::from(plot_width);
        if !units_per_pixel.is_finite() || units_per_pixel <= 0.0 {
            return None;
        }
        let projection = self.projection();
        let world = world_span(&projection);
        let measurement = GeoCoordMeasurement {
            viewport_id: self.viewport_id.clone(),
            view: crate::view::GeoView::new(
                center_x,
                center_y,
                units_per_pixel,
                plot_width,
                plot_height,
                world,
            ),
            projection,
            graticule: None,
            sphere: None,
            blend: self.blend,
            tile_layers: Vec::new(),
            zoom_focus: None,
        };
        measurement.interaction_frame()
    }

    fn default_range_binding(&self, channel: &str) -> Option<ScaleRangeBinding> {
        match channel {
            "x" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::ZERO,
                PlotAreaRangeEndpoint::WIDTH,
            )),
            "y" => Some(ScaleRangeBinding::plot_area(
                PlotAreaRangeEndpoint::HEIGHT,
                PlotAreaRangeEndpoint::ZERO,
            )),
            _ => None,
        }
    }

    fn default_scale_options(
        &self,
        channel: &str,
        scale_impl: &dyn ScaleImpl,
    ) -> HashMap<String, ScalarValue> {
        let mut options = HashMap::new();
        if matches!(channel, "x" | "y")
            && scale_impl.domain_kind() == DomainKind::Numeric
            && scale_impl.range_kind() == RangeKind::Continuous
        {
            options.insert("zero".to_string(), ScalarValue::Boolean(Some(false)));
            options.insert("nice".to_string(), ScalarValue::Boolean(Some(false)));
            options.insert("round".to_string(), ScalarValue::Boolean(Some(false)));
            // Raw Mercator units sit near ±π with meter-scale f32 epsilon;
            // at deep zoom the f32 affine map re-rounds differently as the
            // viewport slides, wobbling marks against the (f64-placed)
            // tiles. Opt position scales into the f64 linear-scale path.
            options.insert(
                "f64_precision".to_string(),
                ScalarValue::Boolean(Some(true)),
            );
        }
        options
    }

    fn runtime_params_retarget_cached_marks(&self) -> bool {
        // Center/units-per-pixel (and the focus hints) are a pure viewport
        // window over a fixed projection: pixel positions change by the
        // same affine transform the x/y linear scales encode, so previews
        // can retarget cached marks instead of rebuilding mark data.
        true
    }

    fn runtime_param_dependencies(&self) -> Vec<String> {
        vec![
            self.center_x_param(),
            self.center_y_param(),
            self.units_per_pixel_param(),
            // Focus params only ever change inside a patch that also
            // changes center/upp (written solely by the pan/zoom
            // bindings), so listing them adds no re-evaluations.
            self.focus_x_param(),
            self.focus_y_param(),
        ]
    }

    fn measurement_provider(&self) -> Option<&dyn CoordinateMeasurementProvider> {
        Some(self)
    }

    fn domain_provider(&self) -> Option<&dyn CoordinateDomainProvider> {
        Some(self)
    }

    fn interaction_invertible_channels(&self) -> Vec<String> {
        vec!["x".to_string(), "y".to_string()]
    }

    fn invert_interaction_point(
        &self,
        request: InteractionPointInversionRequest<'_>,
    ) -> Result<IndexMap<String, ScalarValue>, AvengerChartError> {
        let mut inverted = IndexMap::new();
        for channel in request.channels {
            let range_value = match *channel {
                "x" => request.local_point[0],
                "y" => request.local_point[1],
                other => {
                    return Err(AvengerChartError::InvalidArgument(format!(
                        "Geo coordinate inversion does not support channel '{other}'"
                    )));
                }
            };
            let scale = request.scales.get(*channel).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Missing configured scale for coordinate channel '{channel}'"
                ))
            })?;
            let value = scale.invert_scalar(range_value).map_err(|err| {
                AvengerChartError::InvalidArgument(format!(
                    "Cannot invert Geo channel '{channel}' (scale type {}): {err}",
                    scale.scale_impl.scale_type()
                ))
            })?;
            inverted.insert(
                (*channel).to_string(),
                ScalarValue::Float64(Some(value as f64)),
            );
        }
        Ok(inverted)
    }
}

impl CoordinateDomainProvider for Geo {
    fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
        let mut descriptor = CoordinateDomainDescriptor::new(GEO_DESCRIPTOR_ID);
        descriptor.bindings = vec![
            CoordinateDomainBinding::owned(
                "x",
                CoordinateDomainRole::X,
                CoordinateDomainMaterialization::CreateIfAbsent {
                    scale_type: CoordinateDomainScaleType::LinearNumeric,
                },
            )
            .requiring_scale_type(CoordinateDomainScaleType::LinearNumeric),
            CoordinateDomainBinding::owned(
                "y",
                CoordinateDomainRole::Y,
                CoordinateDomainMaterialization::CreateIfAbsent {
                    scale_type: CoordinateDomainScaleType::LinearNumeric,
                },
            )
            .requiring_scale_type(CoordinateDomainScaleType::LinearNumeric),
        ];
        descriptor.metrics = vec![CoordinateMetricDescriptor::new(GEO_METRIC_ID, "x", "y")];
        descriptor.sharing_policy = CoordinateDomainSharingPolicy::FacetRepeatGroups;
        descriptor.depends_on_plot_area = true;
        vec![descriptor]
    }

    fn resolve_domain_group(
        &self,
        request: CoordinateDomainGroupRequest<'_>,
    ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
        if request.descriptor_id != GEO_DESCRIPTOR_ID {
            return Ok(CoordinateDomainGroupResolution::default());
        }

        let world = world_span(&self.projection());
        let group_domains = geo_domain_groups(request.cells)?;
        let mut cells = Vec::with_capacity(request.cells.len());
        for cell in request.cells {
            let x_state = one_metric_state(cell, "x")?;
            let y_state = one_metric_state(cell, "y")?;
            let group_key = GeoDomainGroupKey {
                x_node: x_state.node.clone(),
                y_node: y_state.node.clone(),
            };
            let domains = group_domains.get(&group_key).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing Geo domain group for cell {}",
                    cell.cell_key.as_str()
                ))
            })?;
            let view = realize_view(
                self.authored_view(cell.params),
                domains.x.as_ref(),
                domains.y.as_ref(),
                cell.plot_area_width,
                cell.plot_area_height,
                world,
            )?;
            cells.push(cell_resolution(cell, view)?);
        }
        Ok(CoordinateDomainGroupResolution { cells })
    }
}

#[async_trait::async_trait]
impl CoordinateMeasurementProvider for Geo {
    async fn measure_coordinate(
        &self,
        request: CoordinateMeasureRequest<'_>,
    ) -> Result<Option<Box<dyn avenger_chart_core::CoordMeasurement>>, AvengerChartError> {
        let world = world_span(&self.projection());
        let x_extent = request
            .scales
            .get("x")
            .and_then(numeric_extent_from_configured);
        let y_extent = request
            .scales
            .get("y")
            .and_then(numeric_extent_from_configured);
        let view = realize_view(
            self.authored_view(request.params),
            x_extent.as_ref(),
            y_extent.as_ref(),
            request.plot_width,
            request.plot_height,
            world,
        )?;
        let zoom_focus = match (
            param_f64(request.params, &self.focus_x_param()),
            param_f64(request.params, &self.focus_y_param()),
        ) {
            (Some(fx), Some(fy)) if fx.is_finite() && fy.is_finite() => {
                Some([fx as f32, fy as f32])
            }
            _ => None,
        };
        Ok(Some(Box::new(GeoCoordMeasurement {
            viewport_id: self.viewport_id.clone(),
            view,
            projection: self.projection(),
            graticule: self.graticule,
            sphere: self.sphere,
            blend: self.blend,
            tile_layers: self.tile_layers.clone(),
            zoom_focus,
        })))
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for Geo {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

fn default_viewport_id() -> String {
    "default".to_string()
}

fn default_rotate() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}

fn param_f64(params: &IndexMap<String, ScalarValue>, name: &str) -> Option<f64> {
    match params.get(name) {
        Some(ScalarValue::Float64(Some(value))) => Some(*value),
        Some(ScalarValue::Float32(Some(value))) => Some(f64::from(*value)),
        Some(ScalarValue::Int64(Some(value))) => Some(*value as f64),
        Some(ScalarValue::Int32(Some(value))) => Some(f64::from(*value)),
        Some(ScalarValue::UInt64(Some(value))) => Some(*value as f64),
        Some(ScalarValue::UInt32(Some(value))) => Some(f64::from(*value)),
        _ => None,
    }
}

fn one_metric_state<'a>(
    cell: &'a CoordinateDomainCellRequest<'_>,
    channel: &str,
) -> Result<&'a CoordinateDomainScaleState, AvengerChartError> {
    let matches = cell
        .scale_states
        .iter()
        .filter(|state| state.coord_channel == channel)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [state] => Ok(*state),
        [] => Err(AvengerChartError::InvalidArgument(format!(
            "Geo {channel} channel does not resolve to a scale"
        ))),
        states => Err(AvengerChartError::InvalidArgument(format!(
            "Geo {channel} channel resolves to multiple scales: {}",
            states
                .iter()
                .map(|state| state.scale_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct GeoDomainGroupKey {
    x_node: avenger_chart_core::CoordinateDomainNode,
    y_node: avenger_chart_core::CoordinateDomainNode,
}

#[derive(Clone, Debug, Default)]
struct GeoDomainGroup {
    x: Option<DomainExtent>,
    y: Option<DomainExtent>,
}

fn geo_domain_groups(
    cells: &[CoordinateDomainCellRequest<'_>],
) -> Result<HashMap<GeoDomainGroupKey, GeoDomainGroup>, AvengerChartError> {
    let mut groups: HashMap<GeoDomainGroupKey, GeoDomainGroup> = HashMap::new();
    for cell in cells {
        let x_state = one_metric_state(cell, "x")?;
        let y_state = one_metric_state(cell, "y")?;
        let key = GeoDomainGroupKey {
            x_node: x_state.node.clone(),
            y_node: y_state.node.clone(),
        };
        let group = groups.entry(key).or_default();
        union_numeric_base_domain_into(&mut group.x, x_state, "x")?;
        union_numeric_base_domain_into(&mut group.y, y_state, "y")?;
    }
    Ok(groups)
}

fn union_numeric_base_domain_into(
    target: &mut Option<DomainExtent>,
    state: &CoordinateDomainScaleState,
    channel: &str,
) -> Result<(), AvengerChartError> {
    let Some(extent) = state.base_domain.clone() else {
        return Ok(());
    };
    if extent.numeric_bounds().is_none() {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Geo {channel} channel requires a numeric interval domain"
        )));
    }
    *target = Some(match target.take() {
        Some(existing) => avenger_chart_core::union_domain_extents(&existing, &extent),
        None => extent,
    });
    Ok(())
}

fn cell_resolution(
    cell: &CoordinateDomainCellRequest<'_>,
    view: crate::view::GeoView,
) -> Result<CoordinateDomainCellResolution, AvengerChartError> {
    let x_state = one_metric_state(cell, "x")?;
    let y_state = one_metric_state(cell, "y")?;
    let mut domain_overrides = HashMap::new();
    domain_overrides.insert(
        x_state.scale_name.clone(),
        DomainExtent::numeric(view.x_domain.0, view.x_domain.1),
    );
    domain_overrides.insert(
        y_state.scale_name.clone(),
        DomainExtent::numeric(view.y_domain.0, view.y_domain.1),
    );
    Ok(CoordinateDomainCellResolution {
        cell_key: cell.cell_key.clone(),
        domain_overrides,
        metadata: Vec::new(),
    })
}

fn numeric_extent_from_configured(
    scale: &avenger_scales::scales::ConfiguredScale,
) -> Option<DomainExtent> {
    scale
        .numeric_interval_domain()
        .ok()
        .map(|(min, max)| DomainExtent::numeric(f64::from(min), f64::from(max)))
}
