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
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    guide::WebMercatorGuide,
    projection::project_lon_lat,
    tiles::RasterTileLayer,
    viewport::{
        ViewAuthoring, WebMercatorCoordMeasurement, WebMercatorView, WebMercatorViewport,
        realize_view,
    },
};

const WEB_MERCATOR_DESCRIPTOR_ID: &str = "webmercator_viewport";
const WEB_MERCATOR_METRIC_ID: &str = "webmercator_projected_units";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebMercator {
    #[serde(default = "default_viewport_id")]
    viewport_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    center_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    center_y: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    zoom: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tile_layers: Vec<RasterTileLayer>,
}

impl WebMercator {
    pub fn new() -> Self {
        Self {
            viewport_id: default_viewport_id(),
            center_x: None,
            center_y: None,
            zoom: None,
            tile_layers: Vec::new(),
        }
    }

    pub fn viewport_id(mut self, id: impl Into<String>) -> Self {
        self.viewport_id = id.into();
        self
    }

    pub fn center_projected(mut self, x: f64, y: f64) -> Self {
        self.center_x = Some(x);
        self.center_y = Some(y);
        self
    }

    pub fn center_lon_lat(self, lon: f64, lat: f64) -> Self {
        let projected = project_lon_lat(lon, lat);
        self.center_projected(projected.x, projected.y)
    }

    pub fn zoom(mut self, zoom: f64) -> Self {
        self.zoom = Some(zoom);
        self
    }

    pub fn viewport(mut self, viewport: WebMercatorViewport) -> Self {
        self.center_x = viewport.center_x;
        self.center_y = viewport.center_y;
        self.zoom = viewport.zoom;
        self
    }

    pub fn tiles(mut self, layer: RasterTileLayer) -> Self {
        self.tile_layers.push(layer);
        self
    }

    pub fn tile_layer(self, layer: RasterTileLayer) -> Self {
        self.tiles(layer)
    }

    pub fn tile_layers(&self) -> &[RasterTileLayer] {
        &self.tile_layers
    }

    pub fn center_x_param(&self) -> String {
        format!("__webmercator_{}_center_x", self.viewport_id)
    }

    pub fn center_y_param(&self) -> String {
        format!("__webmercator_{}_center_y", self.viewport_id)
    }

    pub fn units_per_pixel_param(&self) -> String {
        format!("__webmercator_{}_units_per_pixel", self.viewport_id)
    }

    fn authored_view(&self, params: &IndexMap<String, ScalarValue>) -> ViewAuthoring {
        let center_x = param_f64(params, &self.center_x_param()).or(self.center_x);
        let center_y = param_f64(params, &self.center_y_param()).or(self.center_y);
        let zoom = param_f64(params, &self.units_per_pixel_param())
            .map(zoom_from_units_per_pixel)
            .or(self.zoom);
        ViewAuthoring {
            center_x,
            center_y,
            zoom,
        }
    }
}

impl Default for WebMercator {
    fn default() -> Self {
        Self::new()
    }
}

impl CoordinateSystemCore for WebMercator {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn validate(&self) -> Result<(), AvengerChartError> {
        if self.viewport_id.is_empty() {
            return Err(AvengerChartError::InvalidArgument(
                "WebMercator viewport_id must not be empty".to_string(),
            ));
        }
        for (label, value) in [
            ("center_x", self.center_x),
            ("center_y", self.center_y),
            ("zoom", self.zoom),
        ] {
            if let Some(value) = value
                && !value.is_finite()
            {
                return Err(AvengerChartError::InvalidArgument(format!(
                    "WebMercator {label} must be finite, got {value}"
                )));
            }
        }
        for layer in &self.tile_layers {
            layer.validate()?;
        }
        Ok(())
    }
}

impl CoordinateSystem for WebMercator {
    type Guide = WebMercatorGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for WebMercator {
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
        }
        options
    }

    fn runtime_param_dependencies(&self) -> Vec<String> {
        vec![
            self.center_x_param(),
            self.center_y_param(),
            self.units_per_pixel_param(),
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
                        "WebMercator coordinate inversion does not support channel '{other}'"
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
                    "Cannot invert WebMercator channel '{channel}' (scale type {}): {err}",
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

impl CoordinateDomainProvider for WebMercator {
    fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
        let mut descriptor = CoordinateDomainDescriptor::new(WEB_MERCATOR_DESCRIPTOR_ID);
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
        descriptor.metrics = vec![CoordinateMetricDescriptor::new(
            WEB_MERCATOR_METRIC_ID,
            "x",
            "y",
        )];
        descriptor.sharing_policy = CoordinateDomainSharingPolicy::FacetRepeatGroups;
        descriptor.depends_on_plot_area = true;
        vec![descriptor]
    }

    fn resolve_domain_group(
        &self,
        request: CoordinateDomainGroupRequest<'_>,
    ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
        if request.descriptor_id != WEB_MERCATOR_DESCRIPTOR_ID {
            return Ok(CoordinateDomainGroupResolution::default());
        }

        let group_domains = webmercator_domain_groups(request.cells)?;
        let mut cells = Vec::with_capacity(request.cells.len());
        for cell in request.cells {
            let x_state = one_metric_state(cell, "x")?;
            let y_state = one_metric_state(cell, "y")?;
            let group_key = WebMercatorDomainGroupKey {
                x_node: x_state.node.clone(),
                y_node: y_state.node.clone(),
            };
            let domains = group_domains.get(&group_key).ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Missing WebMercator domain group for cell {}",
                    cell.cell_key.as_str()
                ))
            })?;
            let view = realize_view(
                self.authored_view(cell.params),
                domains.x.as_ref(),
                domains.y.as_ref(),
                cell.plot_area_width,
                cell.plot_area_height,
            )?;
            cells.push(cell_resolution(cell, view)?);
        }
        Ok(CoordinateDomainGroupResolution { cells })
    }
}

#[async_trait::async_trait]
impl CoordinateMeasurementProvider for WebMercator {
    async fn measure_coordinate(
        &self,
        request: CoordinateMeasureRequest<'_>,
    ) -> Result<Option<Box<dyn avenger_chart_core::CoordMeasurement>>, AvengerChartError> {
        let x_extent = request
            .scales
            .get("x")
            .and_then(|scale| numeric_extent_from_configured(scale));
        let y_extent = request
            .scales
            .get("y")
            .and_then(|scale| numeric_extent_from_configured(scale));
        let view = realize_view(
            self.authored_view(request.params),
            x_extent.as_ref(),
            y_extent.as_ref(),
            request.plot_width,
            request.plot_height,
        )?;
        Ok(Some(Box::new(WebMercatorCoordMeasurement {
            viewport_id: self.viewport_id.clone(),
            view,
            tile_layers: self.tile_layers.clone(),
        })))
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for WebMercator {
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

fn zoom_from_units_per_pixel(units_per_pixel: f64) -> f64 {
    crate::projection::zoom_for_units_per_pixel(units_per_pixel)
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
            "WebMercator {channel} channel does not resolve to a scale"
        ))),
        states => Err(AvengerChartError::InvalidArgument(format!(
            "WebMercator {channel} channel resolves to multiple scales: {}",
            states
                .iter()
                .map(|state| state.scale_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WebMercatorDomainGroupKey {
    x_node: avenger_chart_core::CoordinateDomainNode,
    y_node: avenger_chart_core::CoordinateDomainNode,
}

#[derive(Clone, Debug, Default)]
struct WebMercatorDomainGroup {
    x: Option<DomainExtent>,
    y: Option<DomainExtent>,
}

fn webmercator_domain_groups(
    cells: &[CoordinateDomainCellRequest<'_>],
) -> Result<HashMap<WebMercatorDomainGroupKey, WebMercatorDomainGroup>, AvengerChartError> {
    let mut groups: HashMap<WebMercatorDomainGroupKey, WebMercatorDomainGroup> = HashMap::new();
    for cell in cells {
        let x_state = one_metric_state(cell, "x")?;
        let y_state = one_metric_state(cell, "y")?;
        let key = WebMercatorDomainGroupKey {
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
            "WebMercator {channel} channel requires a numeric interval domain"
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
    view: WebMercatorView,
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

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::{CoordinateDomainCellKey, CoordinateDomainNode};
    use avenger_scales::scales::linear::LinearScale;
    use datafusion::prelude::SessionContext;

    fn assert_close(actual: f64, expected: f64, epsilon: f64) {
        assert!(
            (actual - expected).abs() <= epsilon,
            "expected {expected}, got {actual}"
        );
    }

    fn assert_domain_close(actual: (f64, f64), expected: (f64, f64), epsilon: f64) {
        assert_close(actual.0, expected.0, epsilon);
        assert_close(actual.1, expected.1, epsilon);
    }

    fn state(channel: &str, extent: Option<DomainExtent>) -> CoordinateDomainScaleState {
        CoordinateDomainScaleState {
            scale_name: channel.to_string(),
            coord_channel: channel.to_string(),
            role: if channel == "x" {
                CoordinateDomainRole::X
            } else {
                CoordinateDomainRole::Y
            },
            base_domain: extent,
            range: Some((0.0, 100.0)),
            node: CoordinateDomainNode::Local {
                cell_key: CoordinateDomainCellKey::new("root"),
                scale_name: channel.to_string(),
            },
            has_explicit_domain: false,
            raw_domain_param: None,
        }
    }

    fn measure_request<'a>(
        ctx: &'a SessionContext,
        params: &'a IndexMap<String, ScalarValue>,
        plot_width: f32,
        plot_height: f32,
        x_domain: (f32, f32),
        y_domain: (f32, f32),
    ) -> CoordinateMeasureRequest<'a> {
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            LinearScale::configured(x_domain, (0.0, plot_width)),
        );
        scales.insert(
            "y".to_string(),
            LinearScale::configured(y_domain, (plot_height, 0.0)),
        );
        CoordinateMeasureRequest {
            plot_width,
            plot_height,
            params,
            session_context: ctx,
            data: None,
            compiled_marks: &[],
            facet_path: &[],
            scales,
        }
    }

    #[test]
    fn descriptor_owns_and_materializes_x_y() {
        let descriptors = WebMercator::new().domain_descriptors();
        assert_eq!(descriptors.len(), 1);
        let descriptor = &descriptors[0];
        assert_eq!(descriptor.id, WEB_MERCATOR_DESCRIPTOR_ID);
        assert!(descriptor.depends_on_plot_area);
        assert_eq!(
            descriptor.sharing_policy,
            CoordinateDomainSharingPolicy::FacetRepeatGroups
        );
        assert_eq!(descriptor.bindings.len(), 2);
        assert!(descriptor.bindings.iter().all(|binding| matches!(
            binding.materialize,
            CoordinateDomainMaterialization::CreateIfAbsent { .. }
        )));
    }

    #[test]
    fn resolves_domains_from_projected_data() {
        let coord = WebMercator::new();
        let cell_key = CoordinateDomainCellKey::new("root");
        let states = [
            state("x", Some(DomainExtent::numeric(-10.0, 30.0))),
            state("y", Some(DomainExtent::numeric(-20.0, 20.0))),
        ];
        let params = IndexMap::new();
        let cell = CoordinateDomainCellRequest {
            cell_key: &cell_key,
            plot_area_width: 200.0,
            plot_area_height: 100.0,
            params: &params,
            scale_states: &states,
        };
        let cells = [cell];
        let resolution = coord
            .resolve_domain_group(CoordinateDomainGroupRequest {
                descriptor_id: WEB_MERCATOR_DESCRIPTOR_ID,
                cells: &cells,
            })
            .expect("resolve");
        assert_eq!(resolution.cells.len(), 1);
        assert_eq!(
            resolution.cells[0].domain_overrides["x"],
            DomainExtent::numeric(-30.0, 50.0)
        );
        assert_eq!(
            resolution.cells[0].domain_overrides["y"],
            DomainExtent::numeric(-20.0, 20.0)
        );
    }

    #[test]
    fn fixed_center_infers_zoom_to_contain_all_group_cells() {
        let coord = WebMercator::new().center_projected(0.0, 0.0);
        let key_a = CoordinateDomainCellKey::new("a");
        let key_b = CoordinateDomainCellKey::new("b");
        let states_a = [
            state("x", Some(DomainExtent::numeric(10.0, 20.0))),
            state("y", Some(DomainExtent::numeric(0.0, 5.0))),
        ];
        let states_b = [
            state("x", Some(DomainExtent::numeric(40.0, 50.0))),
            state("y", Some(DomainExtent::numeric(0.0, 5.0))),
        ];
        let params = IndexMap::new();
        let cells = [
            CoordinateDomainCellRequest {
                cell_key: &key_a,
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                params: &params,
                scale_states: &states_a,
            },
            CoordinateDomainCellRequest {
                cell_key: &key_b,
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                params: &params,
                scale_states: &states_b,
            },
        ];
        let resolution = coord
            .resolve_domain_group(CoordinateDomainGroupRequest {
                descriptor_id: WEB_MERCATOR_DESCRIPTOR_ID,
                cells: &cells,
            })
            .expect("resolve");
        for cell in resolution.cells {
            assert_eq!(
                cell.domain_overrides["x"],
                DomainExtent::numeric(-50.0, 50.0)
            );
        }
    }

    #[tokio::test]
    async fn measurement_matches_inferred_fit_view() {
        let coord = WebMercator::new();
        let ctx = SessionContext::new();
        let params = IndexMap::new();
        let measurement = coord
            .measure_coordinate(measure_request(
                &ctx,
                &params,
                200.0,
                100.0,
                (-10.0, 30.0),
                (-20.0, 20.0),
            ))
            .await
            .expect("measure")
            .expect("measurement");
        let measurement = WebMercatorCoordMeasurement::downcast(measurement.as_ref())
            .expect("webmercator measurement");

        assert_eq!(measurement.viewport_id, "default");
        assert_close(measurement.view.center_x, 10.0, 1e-12);
        assert_close(measurement.view.center_y, 0.0, 1e-12);
        assert_close(measurement.view.units_per_pixel, 0.4, 1e-12);
        assert_eq!(measurement.view.x_domain, (-30.0, 50.0));
        assert_eq!(measurement.view.y_domain, (-20.0, 20.0));
    }

    #[tokio::test]
    async fn measurement_matches_authored_view_and_carries_tile_layers() {
        let coord = WebMercator::new()
            .viewport_id("map")
            .viewport(
                WebMercatorViewport::new()
                    .center_projected(5.0, -6.0)
                    .zoom(3.0),
            )
            .tiles(crate::tiles::RasterTileLayer::xyz("https://tiles/{z}/{x}/{y}.png").id("base"));
        let ctx = SessionContext::new();
        let params = IndexMap::new();
        let measurement = coord
            .measure_coordinate(measure_request(
                &ctx,
                &params,
                320.0,
                200.0,
                (-10.0, 30.0),
                (-20.0, 20.0),
            ))
            .await
            .expect("measure")
            .expect("measurement");
        let measurement = WebMercatorCoordMeasurement::downcast(measurement.as_ref())
            .expect("webmercator measurement");
        let expected = WebMercatorView::from_center_zoom(5.0, -6.0, 3.0, 320.0, 200.0);

        assert_eq!(measurement.viewport_id, "map");
        assert_eq!(measurement.tile_layers.len(), 1);
        assert_eq!(measurement.tile_layers[0].layer_id(), "base");
        assert_close(measurement.view.center_x, expected.center_x, 1e-12);
        assert_close(measurement.view.center_y, expected.center_y, 1e-12);
        assert_close(
            measurement.view.units_per_pixel,
            expected.units_per_pixel,
            1e-12,
        );
        assert_eq!(measurement.view.x_domain, expected.x_domain);
        assert_eq!(measurement.view.y_domain, expected.y_domain);
    }

    #[tokio::test]
    async fn measurement_runtime_params_preserve_resolution_across_resize() {
        let coord = WebMercator::new().viewport_id("main");
        let ctx = SessionContext::new();
        let mut params = IndexMap::new();
        params.insert(coord.center_x_param(), ScalarValue::Float64(Some(100.0)));
        params.insert(coord.center_y_param(), ScalarValue::Float64(Some(-50.0)));
        params.insert(
            coord.units_per_pixel_param(),
            ScalarValue::Float64(Some(2.0)),
        );

        let small = coord
            .measure_coordinate(measure_request(
                &ctx,
                &params,
                100.0,
                50.0,
                (-10.0, 30.0),
                (-20.0, 20.0),
            ))
            .await
            .expect("measure")
            .expect("measurement");
        let large = coord
            .measure_coordinate(measure_request(
                &ctx,
                &params,
                200.0,
                100.0,
                (-10.0, 30.0),
                (-20.0, 20.0),
            ))
            .await
            .expect("measure")
            .expect("measurement");
        let small =
            WebMercatorCoordMeasurement::downcast(small.as_ref()).expect("webmercator measurement");
        let large =
            WebMercatorCoordMeasurement::downcast(large.as_ref()).expect("webmercator measurement");

        for measurement in [small, large] {
            assert_close(measurement.view.center_x, 100.0, 1e-12);
            assert_close(measurement.view.center_y, -50.0, 1e-12);
            assert_close(measurement.view.units_per_pixel, 2.0, 1e-12);
        }
        assert_domain_close(small.view.x_domain, (0.0, 200.0), 1e-9);
        assert_domain_close(small.view.y_domain, (-100.0, 0.0), 1e-9);
        assert_domain_close(large.view.x_domain, (-100.0, 300.0), 1e-9);
        assert_domain_close(large.view.y_domain, (-150.0, 50.0), 1e-9);
    }

    #[test]
    fn lon_lat_center_builder_projects_center() {
        let coord = WebMercator::new().center_lon_lat(180.0, 0.0);
        assert!((coord.center_x.unwrap() - crate::projection::WEB_MERCATOR_LIMIT).abs() < 1e-6);
        assert!(coord.center_y.unwrap().abs() < 1e-8);
    }

    #[test]
    fn runtime_param_names_are_stable() {
        let coord = WebMercator::new().viewport_id("main");
        assert_eq!(
            coord.runtime_param_dependencies(),
            vec![
                "__webmercator_main_center_x",
                "__webmercator_main_center_y",
                "__webmercator_main_units_per_pixel",
            ]
        );
    }

    #[test]
    fn coordinate_serializes_round_trip() {
        let coord = WebMercator::new().viewport_id("map").viewport(
            WebMercatorViewport::new()
                .center_projected(1.0, 2.0)
                .zoom(3.0),
        );
        let json = serde_json::to_string(&coord).expect("serialize");
        assert!(json.contains("map"));
        let restored: WebMercator = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.viewport_id, "map");
        assert_eq!(restored.center_x, Some(1.0));
        assert_eq!(restored.center_y, Some(2.0));
        assert_eq!(restored.zoom, Some(3.0));
    }
}
