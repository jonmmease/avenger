//! External crate demonstrating custom scale implementation with avenger-chart
//!
//! This crate shows that external crates can:
//! 1. Define custom ScaleImpl implementations
//! 2. Define custom ScaleSpec marker types
//! 3. Implement methods directly on Scale<CustomType>
//! 4. Use the scale in plots with full type safety

use std::sync::Arc;

use avenger_chart_scales::{Scale, ScaleSpec};
use avenger_scales::{
    error::AvengerScaleError,
    scales::{DomainKind, InferDomainFromDataMethod, RangeKind, ScaleConfig, ScaleImpl},
};
use datafusion::{
    arrow::array::{Array, ArrayRef, Float32Array},
    logical_expr::lit,
};

/// A custom logarithmic scale with configurable smoothing
/// This demonstrates that external crates can define new scale types
#[derive(Debug, Clone)]
pub struct SmoothLogScale {
    smoothing: f32,
}

impl Default for SmoothLogScale {
    fn default() -> Self {
        Self { smoothing: 1.0 }
    }
}

impl SmoothLogScale {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_smoothing(smoothing: f32) -> Self {
        Self { smoothing }
    }
}

impl ScaleImpl for SmoothLogScale {
    fn scale_type(&self) -> &'static str {
        "smooth_log"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn domain_kind(&self) -> DomainKind {
        DomainKind::Numeric
    }

    fn range_kind(&self) -> RangeKind {
        RangeKind::Continuous
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get smoothing parameter from options
        let smoothing = config
            .options
            .get("smoothing")
            .and_then(|s| s.as_f32().ok())
            .unwrap_or(self.smoothing);

        // Simple implementation: log(x + smoothing) normalized to range
        let values = values
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| AvengerScaleError::InternalError("Expected Float32Array".to_string()))?;

        let domain = config
            .domain
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| {
                AvengerScaleError::InternalError("Expected Float32Array domain".to_string())
            })?;

        let range = config
            .range
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| {
                AvengerScaleError::InternalError("Expected Float32Array range".to_string())
            })?;

        if domain.len() != 2 || range.len() != 2 {
            return Err(AvengerScaleError::InternalError(
                "Domain and range must have exactly 2 elements".to_string(),
            ));
        }

        let domain_min = domain.value(0);
        let domain_max = domain.value(1);
        let range_min = range.value(0);
        let range_max = range.value(1);

        // Apply smooth log transformation
        let log_domain_min = (domain_min + smoothing).ln();
        let log_domain_max = (domain_max + smoothing).ln();

        let mut result = Vec::with_capacity(values.len());
        for i in 0..values.len() {
            if values.is_null(i) {
                result.push(None);
            } else {
                let val = values.value(i);
                let log_val = (val + smoothing).ln();
                let normalized = (log_val - log_domain_min) / (log_domain_max - log_domain_min);
                let scaled = range_min + normalized * (range_max - range_min);
                result.push(Some(scaled));
            }
        }

        Ok(Arc::new(Float32Array::from(result)))
    }
}

/// Custom ScaleSpec marker type for SmoothLog scale
/// This demonstrates that external crates can define their own scale spec types
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SmoothLog;

#[typetag::serde]
impl ScaleSpec for SmoothLog {
    fn clone_box(&self) -> Box<dyn ScaleSpec> {
        Box::new(self.clone())
    }

    fn name(&self) -> &'static str {
        "smooth_log"
    }

    fn create_impl(&self) -> Arc<dyn ScaleImpl> {
        Arc::new(SmoothLogScale::new())
    }
}

/// Extension trait for Scale<SmoothLog> to add typed methods
pub trait SmoothLogExt {
    /// Set the smoothing parameter
    fn smoothing(self, value: f32) -> Self;

    /// Set whether to clamp values outside the domain
    fn clamp(self, value: bool) -> Self;

    /// Set whether to nice the domain  
    fn nice(self, value: bool) -> Self;
}

impl SmoothLogExt for Scale<SmoothLog> {
    fn smoothing(self, value: f32) -> Self {
        self._option("smoothing", lit(value))
    }

    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }

    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
}
