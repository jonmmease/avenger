use std::sync::Arc;

use arrow::array::{ArrayRef, AsArray, Float64Array};
use arrow::datatypes::Float64Type;
use avenger_scales::error::AvengerScaleError;
use avenger_scales::scales::band::BandScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scales::scales::log::LogScale;
use avenger_scales::scales::nested_band::NestedBandScale;
use avenger_scales::scales::ordinal::OrdinalScale;
use avenger_scales::scales::point::PointScale;
use avenger_scales::scales::pow::PowScale;
use avenger_scales::scales::quantile::QuantileScale;
use avenger_scales::scales::quantize::QuantizeScale;
use avenger_scales::scales::symlog::SymlogScale;
use avenger_scales::scales::threshold::ThresholdScale;
use avenger_scales::scales::time::TimeScale;
use avenger_scales::scales::{DomainKind, ScaleConfig, ScaleImpl};
use serde::{Deserialize, Serialize};

#[test]
fn built_in_scales_round_trip_under_their_scale_type() {
    let scales: Vec<Box<dyn ScaleImpl>> = vec![
        Box::new(BandScale),
        Box::new(LinearScale),
        Box::new(LogScale),
        Box::new(NestedBandScale),
        Box::new(OrdinalScale),
        Box::new(PointScale),
        Box::new(PowScale),
        Box::new(QuantileScale),
        Box::new(QuantizeScale),
        Box::new(SymlogScale),
        Box::new(ThresholdScale),
        Box::new(TimeScale),
    ];
    for scale in scales {
        let json = serde_json::to_value(&*scale).unwrap();
        assert_eq!(json["type"], scale.scale_type());
        let restored: Box<dyn ScaleImpl> = serde_json::from_value(json).unwrap();
        assert_eq!(restored.scale_type(), scale.scale_type());
    }
}

/// A scale defined outside avenger-scales whose configuration is a field: it adds an offset.
#[derive(Debug, Serialize, Deserialize)]
struct OffsetScale {
    offset: f64,
}

#[typetag::serde(name = "offset")]
impl ScaleImpl for OffsetScale {
    fn scale_type(&self) -> &'static str {
        "offset"
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn scale(
        &self,
        _config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        let values = values.as_primitive::<Float64Type>();
        Ok(Arc::new(
            values.unary::<_, Float64Type>(|v| v + self.offset),
        ))
    }
}

#[test]
fn scales_from_other_crates_decode_with_their_configuration() {
    let json = serde_json::to_string(&OffsetScale { offset: 2.5 } as &dyn ScaleImpl).unwrap();
    let restored: Box<dyn ScaleImpl> = serde_json::from_str(&json).unwrap();
    let values: ArrayRef = Arc::new(Float64Array::from(vec![1.0]));
    let config = LinearScale::unfitted((0.0, 1.0)).config;
    let scaled = restored.scale(&config, &values).unwrap();
    assert_eq!(scaled.as_primitive::<Float64Type>().value(0), 3.5);
}
