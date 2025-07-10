use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, Date32Array, Date64Array, Float32Array, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::datatypes::{DataType, TimeUnit};
use avenger_common::types::LinearScaleAdjustment;

use crate::error::AvengerScaleError;

use super::{ConfiguredScale, InferDomainFromDataMethod, ScaleConfig, ScaleContext, ScaleImpl};

/// Time scale for temporal data visualization.
///
/// The time scale is a variant of a linear scale that operates on temporal data types (dates,
/// timestamps). It provides intelligent handling of time-based data with calendar-aware operations.
///
/// Supported Arrow temporal types:
/// - Date32: Days since Unix epoch
/// - Date64: Milliseconds since Unix epoch  
/// - Timestamp: With units (s, ms, μs, ns) and optional timezone
///
/// ## Configuration Options
///
/// - **timezone** (string, default: "UTC"): Display timezone for scale operations. Can be an IANA
///   timezone string (e.g., "America/New_York"), "local" for system timezone, or "UTC".
///
/// - **nice** (boolean or f32, default: false): When true or a number, extends the domain to nice
///   calendar boundaries. If true, uses a default count of 10. If a number, uses that as the target
///   tick count for determining nice boundaries.
///
/// - **interval** (string, optional): Forces specific tick intervals. Examples: "day", "3 hours",
///   "15 minutes", "month", "quarter".
///
/// - **week_start** (string, default: "sunday"): Configures which day is considered the start of a
///   week for week-based operations. Options: "sunday", "monday", etc.
///
/// - **locale** (string, default: "en-US"): Locale for formatting dates and times.
#[derive(Debug)]
pub struct TimeScale;

impl TimeScale {
    /// Create a new time scale with the specified domain and range.
    pub fn configured(domain: (ArrayRef, ArrayRef), range: (f32, f32)) -> ConfiguredScale {
        // Create domain array from the two bounds
        let domain_array = create_temporal_domain_array(&domain.0, &domain.1)
            .expect("Failed to create temporal domain array");

        ConfiguredScale {
            scale_impl: Arc::new(TimeScale),
            config: ScaleConfig {
                domain: domain_array,
                range: Arc::new(Float32Array::from(vec![range.0, range.1])),
                options: vec![
                    ("timezone".to_string(), "UTC".into()),
                    ("nice".to_string(), false.into()),
                    ("week_start".to_string(), "sunday".into()),
                    ("locale".to_string(), "en-US".into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }

    /// Create a new time scale with the specified domain and color range.
    pub fn configured_color<I>(domain: (ArrayRef, ArrayRef), range: I) -> ConfiguredScale
    where
        I: IntoIterator,
        I::Item: Into<String>,
    {
        // Create domain array from the two bounds
        let domain_array = create_temporal_domain_array(&domain.0, &domain.1)
            .expect("Failed to create temporal domain array");

        ConfiguredScale {
            scale_impl: Arc::new(TimeScale),
            config: ScaleConfig {
                domain: domain_array,
                range: Arc::new(StringArray::from(
                    range.into_iter().map(Into::into).collect::<Vec<String>>(),
                )),
                options: vec![
                    ("timezone".to_string(), "UTC".into()),
                    ("nice".to_string(), false.into()),
                    ("week_start".to_string(), "sunday".into()),
                    ("locale".to_string(), "en-US".into()),
                ]
                .into_iter()
                .collect(),
                context: ScaleContext::default(),
            },
        }
    }
}

/// Handler for different temporal types
#[derive(Debug)]
enum TemporalHandler {
    Date32,
    Date64,
    Timestamp(TimeUnit),
}

impl TemporalHandler {
    /// Create handler from Arrow data type
    fn from_data_type(data_type: &DataType) -> Result<Self, AvengerScaleError> {
        match data_type {
            DataType::Date32 => Ok(TemporalHandler::Date32),
            DataType::Date64 => Ok(TemporalHandler::Date64),
            DataType::Timestamp(unit, _tz) => Ok(TemporalHandler::Timestamp(*unit)),
            _ => Err(AvengerScaleError::InvalidDataTypeError(
                data_type.clone(),
                "temporal".to_string(),
            )),
        }
    }

    /// Convert temporal value to Unix timestamp in milliseconds
    fn to_timestamp_millis(&self, value: i64) -> i64 {
        match self {
            TemporalHandler::Date32 => {
                // Date32 is days since epoch
                value * 86400 * 1000
            }
            TemporalHandler::Date64 => {
                // Date64 is already milliseconds since epoch
                value
            }
            TemporalHandler::Timestamp(unit) => {
                // Convert to milliseconds based on unit
                match unit {
                    TimeUnit::Second => value * 1000,
                    TimeUnit::Millisecond => value,
                    TimeUnit::Microsecond => value / 1000,
                    TimeUnit::Nanosecond => value / 1_000_000,
                }
            }
        }
    }
}

impl ScaleImpl for TimeScale {
    fn scale_type(&self) -> &'static str {
        "time"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn scale(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get temporal handler based on domain type
        let domain_type = config.domain.data_type();
        let handler = TemporalHandler::from_data_type(domain_type)?;

        // Get domain bounds
        let domain_start = get_temporal_value(&config.domain, 0, &handler)?;
        let domain_end = get_temporal_value(&config.domain, 1, &handler)?;

        // Get range bounds
        let (range_start, range_end) = config.numeric_interval_range()?;

        // Scale values
        let result = match values.data_type() {
            DataType::Date32 => {
                let values = values.as_any().downcast_ref::<Date32Array>().unwrap();
                let mut output = Vec::with_capacity(values.len());

                for i in 0..values.len() {
                    if values.is_null(i) {
                        output.push(None);
                    } else {
                        let value = handler.to_timestamp_millis(values.value(i) as i64);
                        let normalized =
                            (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                        output.push(Some(range_start + normalized * (range_end - range_start)));
                    }
                }

                Arc::new(Float32Array::from(output))
            }
            DataType::Date64 => {
                let values = values.as_any().downcast_ref::<Date64Array>().unwrap();
                let mut output = Vec::with_capacity(values.len());

                for i in 0..values.len() {
                    if values.is_null(i) {
                        output.push(None);
                    } else {
                        let value = handler.to_timestamp_millis(values.value(i));
                        let normalized =
                            (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                        output.push(Some(range_start + normalized * (range_end - range_start)));
                    }
                }

                Arc::new(Float32Array::from(output))
            }
            DataType::Timestamp(unit, _) => scale_timestamp_values(
                values,
                unit,
                &handler,
                domain_start,
                domain_end,
                range_start,
                range_end,
            )?,
            _ => {
                return Err(AvengerScaleError::InvalidDataTypeError(
                    values.data_type().clone(),
                    "temporal".to_string(),
                ))
            }
        };

        Ok(result)
    }

    fn invert(
        &self,
        _config: &ScaleConfig,
        _values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // TODO: Implement inversion from numeric to temporal
        Err(AvengerScaleError::NotImplementedError(
            "Inversion for time scale not yet implemented".to_string(),
        ))
    }

    fn ticks(
        &self,
        _config: &ScaleConfig,
        _count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // TODO: Implement calendar-aware tick generation
        Err(AvengerScaleError::NotImplementedError(
            "Tick generation for time scale not yet implemented".to_string(),
        ))
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        // TODO: Implement calendar-aware nice domain calculation
        // For now, just return the original domain
        Ok(config.domain.clone())
    }

    fn adjust(
        &self,
        _from_config: &ScaleConfig,
        _to_config: &ScaleConfig,
    ) -> Result<LinearScaleAdjustment, AvengerScaleError> {
        Err(AvengerScaleError::NotImplementedError(
            "Adjust for time scale not yet implemented".to_string(),
        ))
    }
}

// Helper functions

fn create_temporal_domain_array(
    start: &ArrayRef,
    end: &ArrayRef,
) -> Result<ArrayRef, AvengerScaleError> {
    // Validate that both arrays have same type
    if start.data_type() != end.data_type() {
        return Err(AvengerScaleError::InternalError(
            "Domain start and end must have same temporal type".to_string(),
        ));
    }

    // Get the single value from each array (they should be length 1)
    if start.len() != 1 || end.len() != 1 {
        return Err(AvengerScaleError::InternalError(
            "Domain arrays must have exactly one element".to_string(),
        ));
    }

    // Create new array with both values based on type
    match start.data_type() {
        DataType::Date32 => {
            let start_array = start.as_any().downcast_ref::<Date32Array>().unwrap();
            let end_array = end.as_any().downcast_ref::<Date32Array>().unwrap();
            Ok(Arc::new(Date32Array::from(vec![
                start_array.value(0),
                end_array.value(0),
            ])))
        }
        DataType::Date64 => {
            let start_array = start.as_any().downcast_ref::<Date64Array>().unwrap();
            let end_array = end.as_any().downcast_ref::<Date64Array>().unwrap();
            Ok(Arc::new(Date64Array::from(vec![
                start_array.value(0),
                end_array.value(0),
            ])))
        }
        DataType::Timestamp(TimeUnit::Second, tz) => {
            let start_array = start
                .as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap();
            let end_array = end.as_any().downcast_ref::<TimestampSecondArray>().unwrap();
            Ok(Arc::new(
                TimestampSecondArray::from(vec![start_array.value(0), end_array.value(0)])
                    .with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Millisecond, tz) => {
            let start_array = start
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .unwrap();
            let end_array = end
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .unwrap();
            Ok(Arc::new(
                TimestampMillisecondArray::from(vec![start_array.value(0), end_array.value(0)])
                    .with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let start_array = start
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            let end_array = end
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            Ok(Arc::new(
                TimestampMicrosecondArray::from(vec![start_array.value(0), end_array.value(0)])
                    .with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, tz) => {
            let start_array = start
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap();
            let end_array = end
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap();
            Ok(Arc::new(
                TimestampNanosecondArray::from(vec![start_array.value(0), end_array.value(0)])
                    .with_timezone_opt(tz.clone()),
            ))
        }
        _ => Err(AvengerScaleError::InvalidDataTypeError(
            start.data_type().clone(),
            "temporal domain".to_string(),
        )),
    }
}

fn get_temporal_value(
    array: &ArrayRef,
    index: usize,
    handler: &TemporalHandler,
) -> Result<i64, AvengerScaleError> {
    match array.data_type() {
        DataType::Date32 => {
            let array = array.as_any().downcast_ref::<Date32Array>().unwrap();
            Ok(handler.to_timestamp_millis(array.value(index) as i64))
        }
        DataType::Date64 => {
            let array = array.as_any().downcast_ref::<Date64Array>().unwrap();
            Ok(handler.to_timestamp_millis(array.value(index)))
        }
        DataType::Timestamp(TimeUnit::Second, _) => {
            let array = array
                .as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap();
            Ok(handler.to_timestamp_millis(array.value(index)))
        }
        DataType::Timestamp(TimeUnit::Millisecond, _) => {
            let array = array
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .unwrap();
            Ok(handler.to_timestamp_millis(array.value(index)))
        }
        DataType::Timestamp(TimeUnit::Microsecond, _) => {
            let array = array
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            Ok(handler.to_timestamp_millis(array.value(index)))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, _) => {
            let array = array
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap();
            Ok(handler.to_timestamp_millis(array.value(index)))
        }
        _ => Err(AvengerScaleError::InvalidDataTypeError(
            array.data_type().clone(),
            "temporal".to_string(),
        )),
    }
}

fn scale_timestamp_values(
    values: &ArrayRef,
    unit: &TimeUnit,
    handler: &TemporalHandler,
    domain_start: i64,
    domain_end: i64,
    range_start: f32,
    range_end: f32,
) -> Result<ArrayRef, AvengerScaleError> {
    let mut output = Vec::with_capacity(values.len());

    match unit {
        TimeUnit::Second => {
            let values = values
                .as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap();
            for i in 0..values.len() {
                if values.is_null(i) {
                    output.push(None);
                } else {
                    let value = handler.to_timestamp_millis(values.value(i));
                    let normalized =
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                    output.push(Some(range_start + normalized * (range_end - range_start)));
                }
            }
        }
        TimeUnit::Millisecond => {
            let values = values
                .as_any()
                .downcast_ref::<TimestampMillisecondArray>()
                .unwrap();
            for i in 0..values.len() {
                if values.is_null(i) {
                    output.push(None);
                } else {
                    let value = handler.to_timestamp_millis(values.value(i));
                    let normalized =
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                    output.push(Some(range_start + normalized * (range_end - range_start)));
                }
            }
        }
        TimeUnit::Microsecond => {
            let values = values
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap();
            for i in 0..values.len() {
                if values.is_null(i) {
                    output.push(None);
                } else {
                    let value = handler.to_timestamp_millis(values.value(i));
                    let normalized =
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                    output.push(Some(range_start + normalized * (range_end - range_start)));
                }
            }
        }
        TimeUnit::Nanosecond => {
            let values = values
                .as_any()
                .downcast_ref::<TimestampNanosecondArray>()
                .unwrap();
            for i in 0..values.len() {
                if values.is_null(i) {
                    output.push(None);
                } else {
                    let value = handler.to_timestamp_millis(values.value(i));
                    let normalized =
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32;
                    output.push(Some(range_start + normalized * (range_end - range_start)));
                }
            }
        }
    }

    Ok(Arc::new(Float32Array::from(output)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::TimestampSecondArray;

    #[test]
    fn test_time_scale_date32() -> Result<(), AvengerScaleError> {
        // Create domain from 2024-01-01 to 2024-12-31
        // Date32 is days since Unix epoch (1970-01-01)
        let start_date = 19723; // 2024-01-01 in days since epoch
        let end_date = 20088; // 2024-12-31 in days since epoch

        let domain_start = Arc::new(Date32Array::from(vec![start_date])) as ArrayRef;
        let domain_end = Arc::new(Date32Array::from(vec![end_date])) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));

        // Test scaling mid-year date
        let mid_date = 19905; // 2024-07-01 in days since epoch
        let values = Arc::new(Date32Array::from(vec![mid_date])) as ArrayRef;

        let result = scale.scale(&values)?;
        let result_array = result.as_any().downcast_ref::<Float32Array>().unwrap();

        // Should be approximately 50.0 (middle of the range)
        assert!((result_array.value(0) - 50.0).abs() < 1.0);

        Ok(())
    }

    #[test]
    fn test_time_scale_timestamp() -> Result<(), AvengerScaleError> {
        // Create domain from 2024-01-01 00:00:00 to 2024-01-02 00:00:00 (one day)
        let start_ts = 1704067200; // 2024-01-01 00:00:00 UTC
        let end_ts = 1704153600; // 2024-01-02 00:00:00 UTC

        let domain_start = Arc::new(TimestampSecondArray::from(vec![start_ts])) as ArrayRef;
        let domain_end = Arc::new(TimestampSecondArray::from(vec![end_ts])) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));

        // Test scaling noon
        let noon_ts = 1704110400; // 2024-01-01 12:00:00 UTC
        let values = Arc::new(TimestampSecondArray::from(vec![noon_ts])) as ArrayRef;

        let result = scale.scale(&values)?;
        let result_array = result.as_any().downcast_ref::<Float32Array>().unwrap();

        // Should be 50.0 (middle of the range)
        assert!((result_array.value(0) - 50.0).abs() < 0.1);

        Ok(())
    }
}
