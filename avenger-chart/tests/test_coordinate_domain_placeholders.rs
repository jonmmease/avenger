use std::{any::Any, collections::HashMap};

use avenger_chart::prelude::Plot;
use avenger_chart_core::{
    AvengerChartError, CoordinateDomainBinding, CoordinateDomainCellRequest,
    CoordinateDomainCellResolution, CoordinateDomainDescriptor, CoordinateDomainGroupRequest,
    CoordinateDomainGroupResolution, CoordinateDomainMaterialization, CoordinateDomainProvider,
    CoordinateDomainRole, CoordinateDomainScaleType, CoordinateSystem, CoordinateSystemCore,
    CoordinateSystemTransform, CoordinateSystemTransformCore, DomainExtent, PlotAreaRangeEndpoint,
    PlotGeometry, PointGeometry, ScaleRangeBinding,
};
use avenger_common::value::ScalarOrArray;
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::{common::ScalarValue, prelude::SessionContext};

#[tokio::test]
async fn coordinate_domain_provider_can_materialize_owned_absent_scales() {
    let ctx = SessionContext::new();
    let df = ctx.sql("SELECT 1 AS value").await.expect("dataframe");
    let compiled = Plot::with_coord(MaterializingCoord)
        .compile(&ctx)
        .await
        .expect("compile plot");

    let scales = compiled
        .build_scales_for_dataframe(
            &df,
            200.0,
            100.0,
            &ctx,
            &indexmap::IndexMap::<String, ScalarValue>::new(),
        )
        .await
        .expect("coordinate provider should materialize x/y scales");

    assert_eq!(
        scales
            .get("x")
            .expect("x scale")
            .configured()
            .numeric_interval_domain()
            .expect("x numeric domain"),
        (-10.0, 10.0)
    );
    assert_eq!(
        scales
            .get("y")
            .expect("y scale")
            .configured()
            .numeric_interval_domain()
            .expect("y numeric domain"),
        (-5.0, 5.0)
    );
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct MaterializingCoord;

impl CoordinateSystemCore for MaterializingCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }
}

impl CoordinateSystem for MaterializingCoord {
    type Guide = avenger_chart_core::NoGuide;

    fn create_transform(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

impl CoordinateSystemTransformCore for MaterializingCoord {
    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn transform(
        &self,
        position_channels: &HashMap<&str, ScalarOrArray<f32>>,
        _position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
        _plot_width: f32,
        _plot_height: f32,
    ) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
        Ok(Box::new(PointGeometry {
            x: position_channels
                .get("x")
                .cloned()
                .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0)),
            y: position_channels
                .get("y")
                .cloned()
                .unwrap_or_else(|| ScalarOrArray::new_scalar(0.0)),
        }))
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
            options.insert("nice".to_string(), ScalarValue::Boolean(Some(false)));
            options.insert("zero".to_string(), ScalarValue::Boolean(Some(false)));
        }
        options
    }

    fn domain_provider(&self) -> Option<&dyn CoordinateDomainProvider> {
        Some(self)
    }
}

impl CoordinateDomainProvider for MaterializingCoord {
    fn domain_descriptors(&self) -> Vec<CoordinateDomainDescriptor> {
        let mut descriptor = CoordinateDomainDescriptor::new("test_materializing_coord");
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
        vec![descriptor]
    }

    fn resolve_domain_group(
        &self,
        request: CoordinateDomainGroupRequest<'_>,
    ) -> Result<CoordinateDomainGroupResolution, AvengerChartError> {
        let cells = request
            .cells
            .iter()
            .map(|cell| materialized_cell_resolution(cell))
            .collect::<Vec<_>>();
        Ok(CoordinateDomainGroupResolution { cells })
    }
}

#[typetag::serde]
impl CoordinateSystemTransform for MaterializingCoord {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn clone_box(&self) -> Box<dyn CoordinateSystemTransform> {
        Box::new(self.clone())
    }
}

fn materialized_cell_resolution(
    cell: &CoordinateDomainCellRequest<'_>,
) -> CoordinateDomainCellResolution {
    let mut domain_overrides = HashMap::new();
    domain_overrides.insert("x".to_string(), DomainExtent::numeric(-10.0, 10.0));
    domain_overrides.insert("y".to_string(), DomainExtent::numeric(-5.0, 5.0));
    CoordinateDomainCellResolution {
        cell_key: cell.cell_key.clone(),
        domain_overrides,
        metadata: Vec::new(),
    }
}
