//! Integration test to verify external scales can be defined and used

use avenger_chart::{
    cartesian::Cartesian,
    marks::symbol::Symbol,
    plot::Plot,
    scales::{Auto, Scale},
};
use avenger_scales::{
    error::AvengerScaleError,
    scales::{InferDomainFromDataMethod, OptionDefinition, ScaleConfig, ScaleImpl},
};
use datafusion::arrow::array::{Array, ArrayRef, Float32Array};
use datafusion::logical_expr::lit;
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;

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

    fn option_definitions(&self) -> &[OptionDefinition] {
        // Return empty for simplicity - in a real implementation,
        // you'd define these properly
        &[]
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
        let mut result = Vec::with_capacity(values.len());
        for i in 0..values.len() {
            if values.is_valid(i) {
                let val = values.value(i);
                // Smooth log: log(x + smoothing)
                let log_val = (val - domain_min + smoothing).ln();
                let log_min = smoothing.ln();
                let log_max = (domain_max - domain_min + smoothing).ln();

                // Normalize to range
                let normalized = (log_val - log_min) / (log_max - log_min);
                let scaled = range_min + normalized * (range_max - range_min);
                result.push(Some(scaled));
            } else {
                result.push(None);
            }
        }

        Ok(Arc::new(Float32Array::from(result)))
    }
}

/// A custom musical scale that maps frequencies to note positions
#[derive(Debug, Clone)]
pub struct ChromaticScale;

impl ScaleImpl for ChromaticScale {
    fn scale_type(&self) -> &'static str {
        "chromatic"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Simple implementation: map frequencies to chromatic scale positions
        // A4 = 440Hz, each semitone is 2^(1/12) ratio
        let values = values
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| AvengerScaleError::InternalError("Expected Float32Array".to_string()))?;

        let range = config
            .range
            .as_any()
            .downcast_ref::<Float32Array>()
            .ok_or_else(|| {
                AvengerScaleError::InternalError("Expected Float32Array range".to_string())
            })?;

        if range.len() != 2 {
            return Err(AvengerScaleError::InternalError(
                "Range must have exactly 2 elements".to_string(),
            ));
        }

        let range_min = range.value(0);
        let range_max = range.value(1);
        let a4_freq = 440.0_f32;

        // Convert frequencies to semitone positions relative to A4
        let mut result = Vec::with_capacity(values.len());
        for i in 0..values.len() {
            if values.is_valid(i) {
                let freq = values.value(i);
                // Calculate semitones from A4
                let semitones = 12.0 * (freq / a4_freq).log2();
                // Map to range (assuming 88 piano keys, ~7 octaves)
                let position = (semitones + 48.0) / 88.0; // Center around middle of range
                let scaled = range_min + position * (range_max - range_min);
                result.push(Some(scaled));
            } else {
                result.push(None);
            }
        }

        Ok(Arc::new(Float32Array::from(result)))
    }
}

#[test]
fn test_external_scale_can_be_created() {
    // Create custom scales
    let smooth_log = SmoothLogScale::new();
    let chromatic = ChromaticScale;

    // Verify scale types
    assert_eq!(smooth_log.scale_type(), "smooth_log");
    assert_eq!(chromatic.scale_type(), "chromatic");

    // Create Scale wrappers using from_impl
    let log_scale = Scale::<Auto>::from_impl(Arc::new(smooth_log));
    let music_scale = Scale::<Auto>::from_impl(Arc::new(chromatic));

    // Verify they work with Scale API
    assert_eq!(log_scale.get_scale_type(), "smooth_log");
    assert_eq!(music_scale.get_scale_type(), "chromatic");
}

#[test]
fn test_external_scale_in_plot() {
    // Create a plot with custom scales
    let _plot = Plot::new(Cartesian)
        .mark(
            Symbol::new()
                .x("frequency")
                .y("amplitude")
                .fill("instrument"),
        )
        .scale_x(|_| {
            Scale::<Auto>::from_impl(Arc::new(ChromaticScale))
                .domain((220.0_f32, 880.0_f32)) // A3 to A5
                .range_interval(lit(0.0), lit(500.0))
        })
        .scale_y(|_| {
            Scale::<Auto>::from_impl(Arc::new(SmoothLogScale::with_smoothing(0.1)))
                .domain((0.1_f32, 100.0_f32))
                .range_interval(lit(400.0), lit(0.0))
        });

    // The plot compiles with external scales - success!
}

#[test]
fn test_external_scale_with_options() {
    let scale = Scale::<Auto>::from_impl(Arc::new(SmoothLogScale::new()))
        .option("smoothing", lit(0.5))
        .option("clamp", lit(true))
        .option("nice", lit(false));

    // Verify options are stored
    assert!(scale.get_options().contains_key("smoothing"));
    assert!(scale.get_options().contains_key("clamp"));
    assert!(scale.get_options().contains_key("nice"));
}

#[test]
fn test_external_scale_transformation() {
    use avenger_scales::scales::{ScaleConfig, ScaleContext};

    let scale_impl = SmoothLogScale::with_smoothing(1.0);

    // Create test data
    let values = Arc::new(Float32Array::from(vec![1.0, 10.0, 100.0])) as ArrayRef;
    let domain = Arc::new(Float32Array::from(vec![1.0, 100.0])) as ArrayRef;
    let range = Arc::new(Float32Array::from(vec![0.0, 100.0])) as ArrayRef;

    let config = ScaleConfig {
        domain,
        range,
        options: HashMap::new(),
        context: ScaleContext::default(),
    };

    // Apply the scale
    let result = scale_impl.scale(&config, &values).unwrap();

    // Check that we got results
    assert_eq!(result.len(), 3);
    let result_array = result.as_any().downcast_ref::<Float32Array>().unwrap();
    assert!(result_array.is_valid(0));
    assert!(result_array.is_valid(1));
    assert!(result_array.is_valid(2));
}

/// Custom ScaleSpec marker type for SmoothLog scale
/// This demonstrates that external crates can define their own scale spec types
pub struct SmoothLog;

impl avenger_chart::scales::ScaleSpec for SmoothLog {
    fn name() -> &'static str {
        "smooth_log"
    }

    fn create_impl() -> Arc<dyn ScaleImpl> {
        Arc::new(SmoothLogScale::new())
    }
}

/// Extension trait to add typed methods for SmoothLog scale
/// In external crates, you need to define a trait to add methods
pub trait SmoothLogScaleExt {
    /// Set the smoothing parameter
    fn smoothing(self, value: f32) -> Self;

    /// Set whether to clamp values outside the domain
    fn clamp(self, value: bool) -> Self;
}

impl SmoothLogScaleExt for Scale<SmoothLog> {
    fn smoothing(self, value: f32) -> Self {
        self._option("smoothing", lit(value))
    }

    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

#[test]
fn test_external_scale_spec() {
    // Create a scale using the custom ScaleSpec type
    // Need to import the extension trait to use its methods
    use self::SmoothLogScaleExt;
    
    let scale = Scale::<SmoothLog>::new()
        .smoothing(0.5)
        .clamp(true)
        .domain((0.1_f32, 100.0_f32))
        .range_interval(lit(0.0), lit(500.0));

    // Verify it has the correct type
    assert_eq!(scale.get_scale_type(), "smooth_log");
    
    // Verify options were set
    assert!(scale.get_options().contains_key("smoothing"));
    assert!(scale.get_options().contains_key("clamp"));
}

#[test]
fn test_external_scale_spec_in_plot() {
    use self::SmoothLogScaleExt;
    
    // Create a plot using the typed external scale
    let _plot = Plot::new(Cartesian)
        .mark(
            Symbol::new()
                .x("value")
                .y("result")
                .fill("category"),
        )
        .scale_y_with::<SmoothLog>(|scale| {
            scale
                .smoothing(0.1)
                .clamp(true)
                .domain((0.1_f32, 100.0_f32))
                .range_interval(lit(400.0), lit(0.0))
        });

    // The plot compiles with typed external scale - success!
}

#[test]
fn test_scale_type_method_with_external_scale() {
    // Test that scale_type method works with external scales
    use avenger_chart::scales::Linear;

    let scale = Scale::<Linear>::new()
        .scale_type(SmoothLogScale::new()) // Switch to external scale
        .domain((1.0_f32, 100.0_f32));

    assert_eq!(scale.get_scale_type(), "smooth_log");
}
