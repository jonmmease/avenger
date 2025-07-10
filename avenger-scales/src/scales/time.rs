use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, Date32Array, Date64Array, Float32Array, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::datatypes::{DataType, TimeUnit};
use avenger_common::types::LinearScaleAdjustment;
use chrono::{DateTime, Datelike, TimeZone, Timelike, Utc};
use chrono_tz::Tz;

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

/// Parse timezone string to chrono_tz::Tz
fn parse_timezone(tz_str: &str) -> Result<Tz, AvengerScaleError> {
    match tz_str {
        "UTC" | "utc" => Ok(Tz::UTC),
        "local" => {
            // For now, default to UTC when "local" is specified
            // TODO: Implement proper local timezone detection
            Ok(Tz::UTC)
        }
        tz => tz
            .parse::<Tz>()
            .map_err(|_| AvengerScaleError::InvalidTimezoneError(tz.to_string())),
    }
}

/// Convert timestamp to target timezone for display/calculation
fn convert_to_timezone(timestamp_millis: i64, tz: &Tz) -> DateTime<Tz> {
    let utc_dt = Utc.timestamp_millis_opt(timestamp_millis).unwrap();
    utc_dt.with_timezone(tz)
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
        config: &ScaleConfig,
        count: Option<f32>,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get domain bounds
        let domain_type = config.domain.data_type();
        let handler = TemporalHandler::from_data_type(domain_type)?;

        let start_millis = get_temporal_value(&config.domain, 0, &handler)?;
        let end_millis = get_temporal_value(&config.domain, 1, &handler)?;

        // Get timezone for tick generation
        let tz_str = config.option_string("timezone", "UTC");
        let tz = parse_timezone(&tz_str)?;

        // Get target tick count
        let target_count = count.unwrap_or(10.0);

        // Generate ticks
        let tick_millis = generate_temporal_ticks(start_millis, end_millis, target_count, &tz)?;

        // Convert to domain array type
        create_temporal_array_from_millis_vec(&tick_millis, domain_type)
    }

    fn compute_nice_domain(&self, config: &ScaleConfig) -> Result<ArrayRef, AvengerScaleError> {
        // Get domain bounds
        let domain_type = config.domain.data_type();
        let handler = TemporalHandler::from_data_type(domain_type)?;

        let start_millis = get_temporal_value(&config.domain, 0, &handler)?;
        let end_millis = get_temporal_value(&config.domain, 1, &handler)?;

        // Get timezone for nice calculations
        let tz_str = config.option_string("timezone", "UTC");
        let tz = parse_timezone(&tz_str)?;

        // Get nice option
        let nice_option = config.options.get("nice");
        let target_count = match nice_option {
            Some(scalar) => {
                if let Ok(true) = scalar.as_boolean() {
                    10.0
                } else if let Ok(count) = scalar.as_f32() {
                    count
                } else {
                    return Ok(config.domain.clone());
                }
            }
            None => return Ok(config.domain.clone()),
        };

        // Calculate nice bounds
        let (nice_start, nice_end) =
            compute_nice_temporal_bounds(start_millis, end_millis, target_count, &tz)?;

        // Convert back to domain array type
        create_temporal_domain_from_millis(nice_start, nice_end, domain_type)
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

/// Interval hierarchy for time scales
#[derive(Debug, Clone, Copy, PartialEq)]
enum TimeInterval {
    Millisecond(i32),
    Second(i32),
    Minute(i32),
    Hour(i32),
    Day(i32),
    Week(i32),
    Month(i32),
    Year(i32),
}

impl TimeInterval {
    /// Get duration in milliseconds (approximate for months/years)
    fn approx_millis(&self) -> i64 {
        match self {
            TimeInterval::Millisecond(n) => *n as i64,
            TimeInterval::Second(n) => (*n as i64) * 1000,
            TimeInterval::Minute(n) => (*n as i64) * 60 * 1000,
            TimeInterval::Hour(n) => (*n as i64) * 60 * 60 * 1000,
            TimeInterval::Day(n) => (*n as i64) * 24 * 60 * 60 * 1000,
            TimeInterval::Week(n) => (*n as i64) * 7 * 24 * 60 * 60 * 1000,
            TimeInterval::Month(n) => (*n as i64) * 30 * 24 * 60 * 60 * 1000, // Approximate
            TimeInterval::Year(n) => (*n as i64) * 365 * 24 * 60 * 60 * 1000, // Approximate
        }
    }

    /// Floor a datetime to this interval boundary
    fn floor(&self, dt: DateTime<Tz>) -> DateTime<Tz> {
        match self {
            TimeInterval::Millisecond(n) => {
                let millis = dt.timestamp_millis();
                let floored = (millis / *n as i64) * *n as i64;
                dt.timezone().timestamp_millis_opt(floored).unwrap()
            }
            TimeInterval::Second(n) => {
                let base = dt.with_nanosecond(0).unwrap();
                let secs = base.second() as i32;
                base.with_second(((secs / n) * n) as u32).unwrap()
            }
            TimeInterval::Minute(n) => {
                let base = dt.with_second(0).unwrap().with_nanosecond(0).unwrap();
                let mins = base.minute() as i32;
                base.with_minute(((mins / n) * n) as u32).unwrap()
            }
            TimeInterval::Hour(n) => {
                let base = dt
                    .with_minute(0)
                    .unwrap()
                    .with_second(0)
                    .unwrap()
                    .with_nanosecond(0)
                    .unwrap();
                let hours = base.hour() as i32;
                base.with_hour(((hours / n) * n) as u32).unwrap()
            }
            TimeInterval::Day(_) => dt
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(dt.timezone())
                .unwrap(),
            TimeInterval::Week(_) => {
                // Floor to start of week (Sunday)
                let days_since_sunday = dt.weekday().num_days_from_sunday();
                let start_of_week =
                    dt.date_naive() - chrono::Duration::days(days_since_sunday as i64);
                start_of_week
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(dt.timezone())
                    .unwrap()
            }
            TimeInterval::Month(_) => dt
                .with_day(1)
                .unwrap()
                .with_hour(0)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap(),
            TimeInterval::Year(_) => dt
                .with_month(1)
                .unwrap()
                .with_day(1)
                .unwrap()
                .with_hour(0)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap(),
        }
    }

    /// Ceiling - round up to next interval boundary
    fn ceil(&self, dt: DateTime<Tz>) -> DateTime<Tz> {
        let floored = self.floor(dt);
        if floored == dt {
            dt
        } else {
            self.offset(floored, 1)
        }
    }

    /// Offset by n intervals
    fn offset(&self, dt: DateTime<Tz>, count: i32) -> DateTime<Tz> {
        match self {
            TimeInterval::Millisecond(n) => {
                let offset_millis = (*n as i64) * (count as i64);
                dt.timezone()
                    .timestamp_millis_opt(dt.timestamp_millis() + offset_millis)
                    .unwrap()
            }
            TimeInterval::Second(n) => dt + chrono::Duration::seconds((*n as i64) * (count as i64)),
            TimeInterval::Minute(n) => dt + chrono::Duration::minutes((*n as i64) * (count as i64)),
            TimeInterval::Hour(n) => dt + chrono::Duration::hours((*n as i64) * (count as i64)),
            TimeInterval::Day(n) => dt + chrono::Duration::days((*n as i64) * (count as i64)),
            TimeInterval::Week(n) => dt + chrono::Duration::weeks((*n as i64) * (count as i64)),
            TimeInterval::Month(n) => {
                // Handle month arithmetic properly
                let total_months = *n * count;
                let mut result = dt;

                if total_months > 0 {
                    for _ in 0..total_months {
                        result = add_months(result, 1);
                    }
                } else {
                    for _ in 0..(-total_months) {
                        result = subtract_months(result, 1);
                    }
                }
                result
            }
            TimeInterval::Year(n) => {
                // Handle year arithmetic
                let years = *n * count;
                dt.with_year(dt.year() + years).unwrap()
            }
        }
    }
}

/// Add months to a datetime, handling edge cases
fn add_months(dt: DateTime<Tz>, months: i32) -> DateTime<Tz> {
    let target_month = dt.month() as i32 + months;
    let year_offset = (target_month - 1) / 12;
    let new_month = ((target_month - 1) % 12 + 12) % 12 + 1;

    let new_year = dt.year() + year_offset;
    let mut result = dt
        .with_year(new_year)
        .unwrap()
        .with_month(new_month as u32)
        .unwrap();

    // Handle day overflow (e.g., Jan 31 + 1 month = Feb 28/29)
    let max_day = days_in_month(new_year, new_month as u32);
    if dt.day() > max_day {
        result = result.with_day(max_day).unwrap();
    }

    result
}

/// Subtract months from a datetime
fn subtract_months(dt: DateTime<Tz>, months: i32) -> DateTime<Tz> {
    add_months(dt, -months)
}

/// Get number of days in a month
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => panic!("Invalid month: {}", month),
    }
}

/// Check if year is a leap year
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Interval hierarchy for automatic selection
const INTERVAL_HIERARCHY: &[TimeInterval] = &[
    TimeInterval::Millisecond(1),
    TimeInterval::Millisecond(5),
    TimeInterval::Millisecond(10),
    TimeInterval::Millisecond(25),
    TimeInterval::Millisecond(50),
    TimeInterval::Millisecond(100),
    TimeInterval::Millisecond(250),
    TimeInterval::Millisecond(500),
    TimeInterval::Second(1),
    TimeInterval::Second(5),
    TimeInterval::Second(15),
    TimeInterval::Second(30),
    TimeInterval::Minute(1),
    TimeInterval::Minute(5),
    TimeInterval::Minute(15),
    TimeInterval::Minute(30),
    TimeInterval::Hour(1),
    TimeInterval::Hour(3),
    TimeInterval::Hour(6),
    TimeInterval::Hour(12),
    TimeInterval::Day(1),
    TimeInterval::Day(2),
    TimeInterval::Week(1),
    TimeInterval::Month(1),
    TimeInterval::Month(3),
    TimeInterval::Year(1),
    TimeInterval::Year(2),
    TimeInterval::Year(5),
    TimeInterval::Year(10),
    TimeInterval::Year(20),
    TimeInterval::Year(50),
    TimeInterval::Year(100),
];

/// Select appropriate interval based on domain span and target tick count
fn select_time_interval(span_millis: i64, target_count: f32) -> TimeInterval {
    // Find the interval that produces closest to target tick count
    let mut best_interval = INTERVAL_HIERARCHY[0];
    let mut best_diff = f64::INFINITY;

    for &interval in INTERVAL_HIERARCHY {
        let interval_millis = interval.approx_millis() as f64;
        let tick_count = span_millis as f64 / interval_millis;
        let diff = (tick_count - target_count as f64).abs();

        if diff < best_diff {
            best_diff = diff;
            best_interval = interval;
        }
    }

    best_interval
}

/// Compute nice temporal bounds
fn compute_nice_temporal_bounds(
    start_millis: i64,
    end_millis: i64,
    target_count: f32,
    tz: &Tz,
) -> Result<(i64, i64), AvengerScaleError> {
    let span = end_millis - start_millis;
    let interval = select_time_interval(span, target_count);

    let start_dt = convert_to_timezone(start_millis, tz);
    let end_dt = convert_to_timezone(end_millis, tz);

    let nice_start = interval.floor(start_dt);
    let nice_end = interval.ceil(end_dt);

    Ok((nice_start.timestamp_millis(), nice_end.timestamp_millis()))
}

/// Generate temporal ticks
fn generate_temporal_ticks(
    start_millis: i64,
    end_millis: i64,
    target_count: f32,
    tz: &Tz,
) -> Result<Vec<i64>, AvengerScaleError> {
    let span = end_millis - start_millis;
    let interval = select_time_interval(span, target_count);

    let start_dt = convert_to_timezone(start_millis, tz);
    let end_dt = convert_to_timezone(end_millis, tz);

    // Start from floored first tick
    let mut current = interval.ceil(start_dt);
    let mut ticks = Vec::new();

    // Generate ticks within domain
    while current <= end_dt {
        ticks.push(current.timestamp_millis());
        current = interval.offset(current, 1);
    }

    Ok(ticks)
}

/// Create temporal array from vector of millisecond timestamps
fn create_temporal_array_from_millis_vec(
    millis_vec: &[i64],
    data_type: &DataType,
) -> Result<ArrayRef, AvengerScaleError> {
    match data_type {
        DataType::Date32 => {
            // Convert milliseconds to days
            let days: Vec<i32> = millis_vec
                .iter()
                .map(|&ms| (ms / (24 * 60 * 60 * 1000)) as i32)
                .collect();
            Ok(Arc::new(Date32Array::from(days)))
        }
        DataType::Date64 => Ok(Arc::new(Date64Array::from(millis_vec.to_vec()))),
        DataType::Timestamp(TimeUnit::Second, tz) => {
            let secs: Vec<i64> = millis_vec.iter().map(|&ms| ms / 1000).collect();
            Ok(Arc::new(
                TimestampSecondArray::from(secs).with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Millisecond, tz) => Ok(Arc::new(
            TimestampMillisecondArray::from(millis_vec.to_vec()).with_timezone_opt(tz.clone()),
        )),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let micros: Vec<i64> = millis_vec.iter().map(|&ms| ms * 1000).collect();
            Ok(Arc::new(
                TimestampMicrosecondArray::from(micros).with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, tz) => {
            let nanos: Vec<i64> = millis_vec.iter().map(|&ms| ms * 1_000_000).collect();
            Ok(Arc::new(
                TimestampNanosecondArray::from(nanos).with_timezone_opt(tz.clone()),
            ))
        }
        _ => Err(AvengerScaleError::InvalidDataTypeError(
            data_type.clone(),
            "temporal array".to_string(),
        )),
    }
}

/// Create temporal domain array from millisecond timestamps
fn create_temporal_domain_from_millis(
    start_millis: i64,
    end_millis: i64,
    data_type: &DataType,
) -> Result<ArrayRef, AvengerScaleError> {
    create_temporal_array_from_millis_vec(&[start_millis, end_millis], data_type)
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

    #[test]
    fn test_nice_domain() -> Result<(), AvengerScaleError> {
        // Create domain from 2024-01-15 10:30:00 to 2024-02-20 15:45:00
        let start_ts = 1705317000; // 2024-01-15 10:30:00 UTC
        let end_ts = 1708441500; // 2024-02-20 15:45:00 UTC

        let domain_start = Arc::new(TimestampSecondArray::from(vec![start_ts])) as ArrayRef;
        let domain_end = Arc::new(TimestampSecondArray::from(vec![end_ts])) as ArrayRef;

        let mut scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));

        // Apply nice with default count
        scale = scale.with_option("nice", true);
        let nice_domain = scale.scale_impl.compute_nice_domain(&scale.config)?;

        // Should round to nice month boundaries
        let nice_array = nice_domain
            .as_any()
            .downcast_ref::<TimestampSecondArray>()
            .unwrap();

        // Check that domain was extended to nice boundaries
        assert!(nice_array.value(0) <= start_ts);
        assert!(nice_array.value(1) >= end_ts);

        Ok(())
    }

    #[test]
    fn test_tick_generation() -> Result<(), AvengerScaleError> {
        // Create domain for one week
        let start_ts = 1704067200; // 2024-01-01 00:00:00 UTC
        let end_ts = 1704672000; // 2024-01-08 00:00:00 UTC

        let domain_start = Arc::new(TimestampSecondArray::from(vec![start_ts])) as ArrayRef;
        let domain_end = Arc::new(TimestampSecondArray::from(vec![end_ts])) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));

        // Generate approximately 7 ticks (one per day)
        let ticks = scale.ticks(Some(7.0))?;
        let tick_array = ticks
            .as_any()
            .downcast_ref::<TimestampSecondArray>()
            .unwrap();

        // Should have approximately 7-8 ticks
        assert!(tick_array.len() >= 7 && tick_array.len() <= 8);

        // First tick should be at or after start
        assert!(tick_array.value(0) >= start_ts);

        // Last tick should be at or before end
        assert!(tick_array.value(tick_array.len() - 1) <= end_ts);

        Ok(())
    }

    #[test]
    fn test_timezone_conversion() -> Result<(), AvengerScaleError> {
        // Test parsing timezone
        let utc = parse_timezone("UTC")?;
        assert_eq!(utc, Tz::UTC);

        let ny = parse_timezone("America/New_York")?;
        assert_eq!(ny.name(), "America/New_York");

        // Test invalid timezone
        assert!(parse_timezone("Invalid/Timezone").is_err());

        Ok(())
    }

    #[test]
    fn test_interval_selection() {
        // Test second-level selection
        let interval = select_time_interval(10_000, 10.0); // 10 seconds
        assert_eq!(interval, TimeInterval::Second(1));

        // Test minute-level selection
        let interval = select_time_interval(600_000, 10.0); // 10 minutes
        assert_eq!(interval, TimeInterval::Minute(1));

        // Test hour-level selection
        let interval = select_time_interval(36_000_000, 10.0); // 10 hours
        assert_eq!(interval, TimeInterval::Hour(1));

        // Test day-level selection
        let interval = select_time_interval(864_000_000, 10.0); // 10 days
        assert_eq!(interval, TimeInterval::Day(1));
    }
}
