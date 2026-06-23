use std::{any::Any, collections::HashMap};

use avenger_chart_core::{
    AvengerChartError, CartesianUnitAspect, CoordinateDomainBinding, CoordinateDomainCellRequest,
    CoordinateDomainCellResolution, CoordinateDomainDescriptor, CoordinateDomainGroupRequest,
    CoordinateDomainGroupResolution, CoordinateDomainMaterialization, CoordinateDomainProvider,
    CoordinateDomainRole, CoordinateDomainScaleState, CoordinateDomainScaleType,
    CoordinateDomainSharingPolicy, CoordinateMetricDescriptor, CoordinateSystem,
    CoordinateSystemCore, CoordinateSystemTransform, CoordinateSystemTransformCore, DomainExtent,
    InteractionPointInversionRequest, NumericDomainSpanEquation, PlotAreaRangeEndpoint,
    PlotGeometry, PointGeometry, ScaleRangeBinding, solve_numeric_domain_span_graph,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::common::ScalarValue;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::marks::subplot::{CARTESIAN_SUBPLOT_X_CHANNEL, CARTESIAN_SUBPLOT_Y_CHANNEL};

fn cartesian_range_channel(channel: &str) -> &str {
    match channel {
        CARTESIAN_SUBPLOT_X_CHANNEL => "x",
        CARTESIAN_SUBPLOT_Y_CHANNEL => "y",
        _ => channel,
    }
}

/// Cartesian coordinate system with x and y axes.
#[derive(Clone, Serialize, Deserialize)]
pub struct Cartesian {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unit_aspect: Option<CartesianUnitAspect>,
}

impl Cartesian {
    pub const fn new() -> Self {
        Self { unit_aspect: None }
    }

    pub fn unit_aspect(mut self, ratio: f64) -> Self {
        self.unit_aspect = Some(CartesianUnitAspect { ratio });
        self
    }

    pub fn equal_units(self) -> Self {
        self.unit_aspect(1.0)
    }

    pub fn without_unit_aspect(mut self) -> Self {
        self.unit_aspect = None;
        self
    }

    pub fn unit_aspect_constraint(&self) -> Option<CartesianUnitAspect> {
        self.unit_aspect
    }
}

impl Default for Cartesian {
    fn default() -> Self {
        Self::new()
    }
}

impl CoordinateSystemCore for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn validate(&self) -> Result<(), AvengerChartError> {
        if let Some(unit_aspect) = self.unit_aspect
            && (!unit_aspect.ratio.is_finite() || unit_aspect.ratio <= 0.0)
        {
            return Err(AvengerChartError::InvalidArgument(format!(
                "Cartesian unit_aspect ratio must be positive and finite, got {}",
                unit_aspect.ratio
            )));
        }
        Ok(())
    }
}

impl CoordinateSystem for Cartesian {
    type Guide = crate::guide::CartesianGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for Cartesian {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        position_values: Option<&HashMap<&str, Vec<datafusion::common::ScalarValue>>>,
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
        match cartesian_range_channel(channel) {
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
        let domain_kind = scale_impl.domain_kind();
        let range_kind = scale_impl.range_kind();
        let scale_type = scale_impl.scale_type();

        let mut options = HashMap::new();

        let channel = cartesian_range_channel(channel);
        if channel == "x" || channel == "y" {
            if domain_kind == DomainKind::Numeric && range_kind == RangeKind::Continuous {
                if channel == "y" && scale_type == "linear" {
                    options.insert("zero".to_string(), ScalarValue::Boolean(Some(true)));
                }

                options.insert("nice".to_string(), ScalarValue::Boolean(Some(true)));
                options.insert("round".to_string(), ScalarValue::Boolean(Some(true)));
            } else if domain_kind == DomainKind::Categorical && range_kind == RangeKind::Continuous
            {
                let option_defs = scale_impl.option_definitions();
                if option_defs.iter().any(|def| def.name == "padding") && scale_type == "band" {
                    options.insert("padding".to_string(), ScalarValue::Float64(Some(0.1)));
                }
            }
        }

        options
    }

    fn domain_provider(&self) -> Option<&dyn CoordinateDomainProvider> {
        self.unit_aspect
            .map(|_| self as &dyn CoordinateDomainProvider)
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
                        "Cartesian coordinate inversion does not support channel '{other}'"
                    )));
                }
            };
            let scale = request.scales.get(*channel).ok_or_else(|| {
                AvengerChartError::InvalidArgument(format!(
                    "Missing configured scale for coordinate channel '{channel}'"
                ))
            })?;
            // The configured scale handles reversed ranges; categorical scales
            // use interval inversion so a point inside a band returns its
            // domain value.
            let value = if matches!(
                scale.scale_impl.domain_kind(),
                DomainKind::Categorical | DomainKind::NestedCategorical
            ) {
                let values = scale
                    .invert_range_interval((range_value, range_value))
                    .map_err(|err| {
                        AvengerChartError::InvalidArgument(format!(
                            "Cannot invert coordinate channel '{channel}' (scale type {}): {err}",
                            scale.scale_impl.scale_type()
                        ))
                    })?;
                if values.is_empty() {
                    continue;
                }
                ScalarValue::try_from_array(values.as_ref(), 0).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "Cannot convert inverted coordinate channel '{channel}' value: {err}"
                    ))
                })?
            } else {
                let value = scale.invert_scalar(range_value).map_err(|err| {
                    AvengerChartError::InvalidArgument(format!(
                        "Cannot invert coordinate channel '{channel}' (scale type {}): {err}",
                        scale.scale_impl.scale_type()
                    ))
                })?;
                ScalarValue::Float64(Some(value as f64))
            };
            inverted.insert((*channel).to_string(), value);
        }
        Ok(inverted)
    }
}

impl CoordinateDomainProvider for Cartesian {
    fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
        let Some(_) = self.unit_aspect else {
            return Vec::new();
        };

        let mut descriptor = CoordinateDomainDescriptor::new("cartesian_unit_aspect");
        descriptor.bindings = vec![
            CoordinateDomainBinding::observed("x", CoordinateDomainRole::X)
                .requiring_scale_type(CoordinateDomainScaleType::LinearNumeric),
            CoordinateDomainBinding::observed("y", CoordinateDomainRole::Y)
                .requiring_scale_type(CoordinateDomainScaleType::LinearNumeric),
        ];
        descriptor.metrics = vec![CoordinateMetricDescriptor::new(
            "cartesian_unit_aspect",
            "x",
            "y",
        )];
        descriptor.sharing_policy = CoordinateDomainSharingPolicy::FacetRepeatGroups;
        descriptor.depends_on_plot_area = true;
        descriptor.bindings.iter_mut().for_each(|binding| {
            binding.materialize = CoordinateDomainMaterialization::ExistingOnly
        });
        vec![descriptor]
    }

    fn resolve_domain_group(
        &self,
        request: CoordinateDomainGroupRequest<'_>,
    ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
        let Some(unit_aspect) = self.unit_aspect else {
            return Ok(CoordinateDomainGroupResolution::default());
        };
        if !unit_aspect.ratio.is_finite() || unit_aspect.ratio <= 0.0 {
            return Err(AvengerChartError::InvalidArgument(format!(
                "unit_aspect ratio must be positive and finite, got {}",
                unit_aspect.ratio
            )));
        }

        let mut equations = Vec::new();
        for cell in request.cells {
            let x_state = one_metric_state(cell, "x")?;
            let y_state = one_metric_state(cell, "y")?;
            equations.push(NumericDomainSpanEquation {
                x_node: x_state.node.clone(),
                y_node: y_state.node.clone(),
                x_extent: required_domain_extent(x_state, "x")?,
                y_extent: required_domain_extent(y_state, "y")?,
                x_range_span: required_range_span(x_state, "x")?,
                y_range_span: required_range_span(y_state, "y")?,
                ratio: unit_aspect.ratio,
            });
        }

        let solved = solve_numeric_domain_span_graph(&equations)?;
        let mut cells = Vec::with_capacity(request.cells.len());
        for cell in request.cells {
            let x_state = one_metric_state(cell, "x")?;
            let y_state = one_metric_state(cell, "y")?;
            let mut domain_overrides = HashMap::new();
            for state in [x_state, y_state] {
                if let Some(extent) = solved.get(&state.node) {
                    domain_overrides.insert(state.scale_name.clone(), extent.clone());
                }
            }
            cells.push(CoordinateDomainCellResolution {
                cell_key: cell.cell_key.clone(),
                domain_overrides,
                metadata: Vec::new(),
            });
        }

        Ok(CoordinateDomainGroupResolution { cells })
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
            "unit_aspect {channel} channel does not resolve to a scale"
        ))),
        states => Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect {channel} channel resolves to multiple scales: {}",
            states
                .iter()
                .map(|state| state.scale_name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn required_domain_extent(
    state: &CoordinateDomainScaleState,
    axis: &str,
) -> Result<DomainExtent, AvengerChartError> {
    state.base_domain.clone().ok_or_else(|| {
        AvengerChartError::InvalidArgument(format!(
            "unit_aspect {axis} scale '{}' requires a numeric interval domain",
            state.scale_name
        ))
    })
}

fn required_range_span(
    state: &CoordinateDomainScaleState,
    axis: &str,
) -> Result<f64, AvengerChartError> {
    let Some((start, end)) = state.range else {
        return Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect {axis} scale '{}' requires a numeric interval range",
            state.scale_name
        )));
    };
    let span = (end - start).abs();
    if span.is_finite() && span > 0.0 {
        Ok(span)
    } else {
        Err(AvengerChartError::InvalidArgument(format!(
            "unit_aspect {axis} range for scale '{}' must have positive finite span, got {:?}",
            state.scale_name,
            (start, end)
        )))
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for Cartesian {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use avenger_scales::scales::{
        band::BandScale, linear::LinearScale, nested_band::NestedBandScale,
    };
    use datafusion::arrow::{
        array::{Array, ArrayRef, StringArray, StructArray},
        datatypes::{DataType, Field},
    };

    fn utf8_struct(fields: &[(&str, Vec<Option<&str>>)]) -> ArrayRef {
        let columns = fields
            .iter()
            .map(|(name, values)| {
                (
                    Arc::new(Field::new(*name, DataType::Utf8, true)),
                    Arc::new(StringArray::from(values.clone())) as ArrayRef,
                )
            })
            .collect::<Vec<_>>();
        Arc::new(StructArray::from(columns)) as ArrayRef
    }

    fn struct_scalar_utf8_value(value: &ScalarValue, field_name: &str) -> Option<String> {
        let ScalarValue::Struct(struct_array) = value else {
            panic!("expected struct scalar, got {value:?}");
        };
        let column = struct_array.column_by_name(field_name).unwrap();
        let strings = column.as_any().downcast_ref::<StringArray>().unwrap();
        (!strings.is_null(0)).then(|| strings.value(0).to_string())
    }

    #[test]
    fn cartesian_interaction_invertible_channels_are_x_and_y() {
        assert_eq!(
            Cartesian::new().interaction_invertible_channels(),
            vec!["x", "y"]
        );
    }

    #[test]
    fn cartesian_unit_aspect_builders_store_constraint() {
        let coord = Cartesian::new().unit_aspect(2.0);
        assert_eq!(
            coord.unit_aspect_constraint(),
            Some(CartesianUnitAspect { ratio: 2.0 })
        );
        assert_eq!(
            coord
                .domain_provider()
                .expect("unit_aspect coordinate domain provider")
                .domain_descriptors()
                .into_iter()
                .flat_map(|descriptor| descriptor.metrics)
                .collect::<Vec<_>>(),
            vec![CoordinateMetricDescriptor::new(
                "cartesian_unit_aspect",
                "x",
                "y"
            )]
        );
        assert_eq!(coord.without_unit_aspect().unit_aspect_constraint(), None);
        assert_eq!(
            Cartesian::new().equal_units().unit_aspect_constraint(),
            Some(CartesianUnitAspect { ratio: 1.0 })
        );
    }

    #[test]
    fn cartesian_unit_aspect_validate_rejects_non_positive_or_non_finite() {
        for ratio in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let err = Cartesian::new()
                .unit_aspect(ratio)
                .validate()
                .expect_err("invalid unit aspect ratio should fail validation");
            assert!(
                matches!(err, AvengerChartError::InvalidArgument(_)),
                "unexpected error for ratio {ratio}: {err:?}"
            );
        }
        Cartesian::new()
            .unit_aspect(1.0)
            .validate()
            .expect("positive finite ratio");
    }

    #[test]
    fn interaction_invert_x_maps_local_point_through_scale() {
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            LinearScale::configured((0.0, 10.0), (0.0, 100.0)),
        );
        let inverted = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [25.0, 0.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect("invert x");
        match inverted.get("x") {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 2.5).abs() < 1e-6),
            other => panic!("expected x=2.5, got {other:?}"),
        }
    }

    #[test]
    fn interaction_invert_y_handles_reversed_range() {
        // Cartesian y range binds to [plot_area_height, 0], so a local y near the
        // top of the plot inverts to a high domain value.
        let mut scales = HashMap::new();
        scales.insert(
            "y".to_string(),
            LinearScale::configured((0.0, 10.0), (100.0, 0.0)),
        );
        let inverted = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [0.0, 25.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["y"],
                scales: &scales,
            })
            .expect("invert y");
        match inverted.get("y") {
            Some(ScalarValue::Float64(Some(value))) => assert!((value - 7.5).abs() < 1e-6),
            other => panic!("expected y=7.5, got {other:?}"),
        }
    }

    #[test]
    fn interaction_invert_band_x_returns_categorical_domain_value() {
        let domain = Arc::new(StringArray::from(vec!["A", "B", "C"])) as ArrayRef;
        let mut scales = HashMap::new();
        scales.insert("x".to_string(), BandScale::configured(domain, (0.0, 300.0)));

        let inverted = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [150.0, 0.0],
                plot_area_width: 300.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect("invert categorical x");

        match inverted.get("x") {
            Some(ScalarValue::Utf8(Some(value))) => assert_eq!(value, "B"),
            other => panic!("expected x=B, got {other:?}"),
        }
    }

    #[test]
    fn interaction_invert_nested_band_x_returns_struct_domain_value() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A"), Some("B")]),
            ("series", vec![Some("x"), Some("y"), Some("x")]),
        ]);
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            NestedBandScale::configured(domain, (0.0, 300.0)),
        );

        let inverted = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [150.0, 0.0],
                plot_area_width: 300.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect("invert nested categorical x");
        let x = inverted.get("x").expect("x value");

        assert_eq!(struct_scalar_utf8_value(x, "group"), Some("A".to_string()));
        assert_eq!(struct_scalar_utf8_value(x, "series"), Some("y".to_string()));
    }

    #[test]
    fn interaction_invert_nested_band_x_gap_omits_channel_value() {
        let domain = utf8_struct(&[
            ("group", vec![Some("A"), Some("A")]),
            ("series", vec![Some("x"), Some("y")]),
        ]);
        let mut scales = HashMap::new();
        scales.insert(
            "x".to_string(),
            NestedBandScale::configured(domain, (0.0, 220.0))
                .with_option("padding_inner_px_levels", ",20"),
        );

        let inverted = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [105.0, 0.0],
                plot_area_width: 220.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect("invert nested categorical x gap");

        assert!(!inverted.contains_key("x"));
    }

    #[test]
    fn interaction_invert_missing_scale_is_invalid_argument() {
        let scales = HashMap::new();
        let err = Cartesian::new()
            .invert_interaction_point(InteractionPointInversionRequest {
                local_point: [25.0, 0.0],
                plot_area_width: 100.0,
                plot_area_height: 100.0,
                channels: &["x"],
                scales: &scales,
            })
            .expect_err("missing scale should be invalid");
        assert!(matches!(err, AvengerChartError::InvalidArgument(_)));
    }
}
