use std::sync::Arc;

use arrow::array::{
    Array, ArrayRef, Date32Array, Date64Array, Float32Array, StringArray,
    TimestampMicrosecondArray, TimestampMillisecondArray, TimestampNanosecondArray,
    TimestampSecondArray,
};
use arrow::compute::kernels::cast;
use arrow::datatypes::{DataType, TimeUnit};
use avenger_common::types::LinearScaleAdjustment;
use avenger_common::value::ScalarOrArray;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use lazy_static::lazy_static;

use crate::error::AvengerScaleError;
use crate::formatter::{DateFormatter, TimestampFormatter, TimestamptzFormatter};

use super::{
    ConfiguredScale, InferDomainFromDataMethod, OptionConstraint, OptionDefinition, ScaleConfig,
    ScaleContext, ScaleImpl,
};

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

/// Temporal tick formatter that adapts format based on interval
#[derive(Debug, Clone)]
struct TemporalTickFormatter {
    interval: TimeInterval,
    timezone: Tz,
}

impl TemporalTickFormatter {
    fn new(interval: TimeInterval, timezone: Tz) -> Self {
        Self { interval, timezone }
    }

    /// Get format string based on interval type
    fn get_format_string(&self) -> &'static str {
        match &self.interval {
            TimeInterval::Millisecond(_) => "%H:%M:%S%.3f",
            TimeInterval::Second(_) => "%H:%M:%S",
            TimeInterval::Minute(n) if *n < 60 => "%H:%M",
            TimeInterval::Hour(n) if *n < 24 => "%H:%M",
            TimeInterval::Day(_) => "%b %d",
            TimeInterval::Week(_) => "%b %d",
            TimeInterval::Month(n) if *n < 12 => "%B",
            TimeInterval::Year(_) => "%Y",
            _ => "%Y-%m-%d %H:%M:%S",
        }
    }
}

impl DateFormatter for TemporalTickFormatter {
    fn format(&self, values: &[Option<NaiveDate>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        let format_str = self.get_format_string();

        values
            .iter()
            .map(|v| {
                v.map(|date| {
                    // Convert to datetime at midnight in the target timezone
                    match self
                        .timezone
                        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
                    {
                        chrono::LocalResult::Single(dt) => dt.format(format_str).to_string(),
                        chrono::LocalResult::None => {
                            // Handle DST gap by trying next hours
                            self.timezone
                                .from_local_datetime(&date.and_hms_opt(3, 0, 0).unwrap())
                                .single()
                                .unwrap()
                                .format(format_str)
                                .to_string()
                        }
                        chrono::LocalResult::Ambiguous(dt, _) => dt.format(format_str).to_string(),
                    }
                })
                .unwrap_or_else(|| default.to_string())
            })
            .collect()
    }
}

impl TimestampFormatter for TemporalTickFormatter {
    fn format(
        &self,
        values: &[Option<chrono::NaiveDateTime>],
        default: Option<&str>,
    ) -> Vec<String> {
        let default = default.unwrap_or("");
        let format_str = self.get_format_string();

        values
            .iter()
            .map(|v| {
                v.map(|naive_dt| {
                    // Convert to timezone-aware datetime
                    let local_dt = self.timezone.from_utc_datetime(&naive_dt);
                    local_dt.format(format_str).to_string()
                })
                .unwrap_or_else(|| default.to_string())
            })
            .collect()
    }
}

impl TimestamptzFormatter for TemporalTickFormatter {
    fn format(&self, values: &[Option<DateTime<Utc>>], default: Option<&str>) -> Vec<String> {
        let default = default.unwrap_or("");
        let format_str = self.get_format_string();

        values
            .iter()
            .map(|v| {
                v.map(|utc_dt| {
                    let local_dt = utc_dt.with_timezone(&self.timezone);
                    local_dt.format(format_str).to_string()
                })
                .unwrap_or_else(|| default.to_string())
            })
            .collect()
    }
}

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

/// DST transition types
#[derive(Debug, Clone, PartialEq)]
enum DstTransition {
    None,
    SpringForward {
        missing_start: u32,
        missing_end: u32,
    },
    FallBack {
        repeated_start: u32,
        repeated_end: u32,
    },
}

/// DST resolution strategy for ambiguous times
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[allow(dead_code)]
enum DstStrategy {
    #[default]
    Earliest, // For fall-back, use first occurrence
    Latest,         // For fall-back, use second occurrence
    PreferStandard, // Prefer standard time over DST
    PreferDaylight, // Prefer DST over standard time
}

/// Safe time construction that handles DST transitions
mod safe_time {
    use super::*;

    /// Safely set hour on a DateTime, handling DST transitions
    pub fn safe_with_hour(dt: DateTime<Tz>, hour: u32) -> Result<DateTime<Tz>, AvengerScaleError> {
        match dt.with_hour(hour) {
            Some(new_dt) => Ok(new_dt),
            None => {
                // Hour doesn't exist (spring forward gap)
                // Try the next hour
                if hour < 23 {
                    dt.with_hour(hour + 1).ok_or_else(|| {
                        AvengerScaleError::DstTransitionError(format!(
                            "Cannot set hour {} during DST transition",
                            hour
                        ))
                    })
                } else {
                    // If we're at hour 23 and it doesn't exist, go to next day
                    let next_day = dt.date_naive() + chrono::Duration::days(1);
                    next_day
                        .and_hms_opt(0, dt.minute(), dt.second())
                        .and_then(|naive_dt| dt.timezone().from_local_datetime(&naive_dt).single())
                        .ok_or_else(|| {
                            AvengerScaleError::DstTransitionError(format!(
                                "Cannot set hour {} during DST transition",
                                hour
                            ))
                        })
                }
            }
        }
    }

    /// Safely set time components, handling DST transitions
    #[allow(dead_code)]
    pub fn safe_with_time(
        dt: DateTime<Tz>,
        hour: u32,
        min: u32,
        sec: u32,
        strategy: DstStrategy,
    ) -> Result<DateTime<Tz>, AvengerScaleError> {
        let date = dt.date_naive();
        safe_and_hms(date, &dt.timezone(), hour, min, sec, strategy)
    }

    /// Safely create DateTime from date and time components
    pub fn safe_and_hms(
        date: NaiveDate,
        tz: &Tz,
        hour: u32,
        min: u32,
        sec: u32,
        strategy: DstStrategy,
    ) -> Result<DateTime<Tz>, AvengerScaleError> {
        match date.and_hms_opt(hour, min, sec) {
            None => Err(AvengerScaleError::DstTransitionError(format!(
                "Invalid time components: {}:{}:{}",
                hour, min, sec
            ))),
            Some(naive_dt) => {
                match tz.from_local_datetime(&naive_dt) {
                    chrono::LocalResult::Single(dt) => Ok(dt),
                    chrono::LocalResult::None => {
                        // Time doesn't exist (spring forward)
                        // Jump forward an hour
                        if hour < 23 {
                            safe_and_hms(date, tz, hour + 1, min, sec, strategy)
                        } else {
                            // Jump to next day
                            let next_date = date + chrono::Duration::days(1);
                            safe_and_hms(next_date, tz, 0, min, sec, strategy)
                        }
                    }
                    chrono::LocalResult::Ambiguous(early, late) => {
                        // Time is ambiguous (fall back)
                        match strategy {
                            DstStrategy::Earliest => Ok(early),
                            DstStrategy::Latest => Ok(late),
                            DstStrategy::PreferStandard => {
                                // Standard time has the larger UTC offset in fall-back
                                // Compare using timestamps since we can't compare offsets directly
                                if early.timestamp() > late.timestamp() {
                                    Ok(late) // Later timestamp has standard time
                                } else {
                                    Ok(early)
                                }
                            }
                            DstStrategy::PreferDaylight => {
                                // DST has the smaller UTC offset in fall-back
                                // Earlier timestamp has DST
                                if early.timestamp() < late.timestamp() {
                                    Ok(early)
                                } else {
                                    Ok(late)
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Detect DST transitions for a given date
    pub fn find_dst_transition(date: NaiveDate, tz: &Tz) -> DstTransition {
        let mut spring_forward_start = None;
        let mut spring_forward_end = None;
        let mut fall_back_start = None;
        let mut fall_back_end = None;

        // Check each hour of the day
        for hour in 0..24 {
            match date.and_hms_opt(hour, 0, 0) {
                None => continue,
                Some(naive_dt) => {
                    match tz.from_local_datetime(&naive_dt) {
                        chrono::LocalResult::None => {
                            // Spring forward gap
                            if spring_forward_start.is_none() {
                                spring_forward_start = Some(hour);
                            }
                            spring_forward_end = Some(hour + 1);
                        }
                        chrono::LocalResult::Ambiguous(_, _) => {
                            // Fall back overlap
                            if fall_back_start.is_none() {
                                fall_back_start = Some(hour);
                            }
                            fall_back_end = Some(hour + 1);
                        }
                        chrono::LocalResult::Single(_) => {}
                    }
                }
            }
        }

        if let (Some(start), Some(end)) = (spring_forward_start, spring_forward_end) {
            DstTransition::SpringForward {
                missing_start: start,
                missing_end: end,
            }
        } else if let (Some(start), Some(end)) = (fall_back_start, fall_back_end) {
            DstTransition::FallBack {
                repeated_start: start,
                repeated_end: end,
            }
        } else {
            DstTransition::None
        }
    }

    /// Check if a date has a DST transition
    #[allow(dead_code)]
    pub fn is_dst_transition_date(date: NaiveDate, tz: &Tz) -> bool {
        !matches!(find_dst_transition(date, tz), DstTransition::None)
    }
}

/// Compute actual duration between two timestamps considering DST transitions
fn compute_actual_duration_millis(
    start_millis: i64,
    end_millis: i64,
    tz: &Tz,
) -> Result<i64, AvengerScaleError> {
    // If both timestamps are in UTC or the same timezone, just subtract
    if tz == &chrono_tz::UTC {
        return Ok(end_millis - start_millis);
    }

    // Convert to timezone-aware datetimes
    let start = tz
        .timestamp_opt(
            start_millis / 1000,
            ((start_millis % 1000) * 1_000_000) as u32,
        )
        .single()
        .ok_or_else(|| {
            AvengerScaleError::DstTransitionError(format!(
                "Ambiguous start timestamp: {}",
                start_millis
            ))
        })?;

    let end = tz
        .timestamp_opt(end_millis / 1000, ((end_millis % 1000) * 1_000_000) as u32)
        .single()
        .ok_or_else(|| {
            AvengerScaleError::DstTransitionError(format!(
                "Ambiguous end timestamp: {}",
                end_millis
            ))
        })?;

    // Compute actual duration
    let duration = end - start;
    Ok(duration.num_milliseconds())
}

impl ScaleImpl for TimeScale {
    fn scale_type(&self) -> &'static str {
        "time"
    }

    fn infer_domain_from_data_method(&self) -> InferDomainFromDataMethod {
        InferDomainFromDataMethod::Interval
    }

    fn option_definitions(&self) -> &[OptionDefinition] {
        lazy_static! {
            static ref DEFINITIONS: Vec<OptionDefinition> = vec![
                OptionDefinition::optional("timezone", OptionConstraint::String),
                OptionDefinition::optional("nice", OptionConstraint::nice()),
                OptionDefinition::optional("interval", OptionConstraint::String),
                OptionDefinition::optional(
                    "week_start",
                    OptionConstraint::StringEnum {
                        values: vec![
                            "sunday".to_string(),
                            "monday".to_string(),
                            "tuesday".to_string(),
                            "wednesday".to_string(),
                            "thursday".to_string(),
                            "friday".to_string(),
                            "saturday".to_string()
                        ]
                    }
                ),
                OptionDefinition::optional("locale", OptionConstraint::String),
                OptionDefinition::optional("default", OptionConstraint::String),
            ];
        }

        &DEFINITIONS
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

        // Get timezone for DST-aware duration calculations
        let tz_str = config.option_string("timezone", "UTC");
        let tz = parse_timezone(&tz_str)?;

        // Compute actual duration (accounting for DST)
        let actual_duration = compute_actual_duration_millis(domain_start, domain_end, &tz)?;
        let use_actual_duration = actual_duration != (domain_end - domain_start);

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
                        let normalized = if use_actual_duration {
                            // Use actual duration for DST-aware scaling
                            let value_duration =
                                compute_actual_duration_millis(domain_start, value, &tz)?;
                            value_duration as f32 / actual_duration as f32
                        } else {
                            // Use nominal duration for performance when no DST
                            (value - domain_start) as f32 / (domain_end - domain_start) as f32
                        };
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
                        let normalized = if use_actual_duration {
                            // Use actual duration for DST-aware scaling
                            let value_duration =
                                compute_actual_duration_millis(domain_start, value, &tz)?;
                            value_duration as f32 / actual_duration as f32
                        } else {
                            // Use nominal duration for performance when no DST
                            (value - domain_start) as f32 / (domain_end - domain_start) as f32
                        };
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
                &tz,
                use_actual_duration,
                actual_duration,
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
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ArrayRef, AvengerScaleError> {
        // Get domain and range bounds
        let domain_type = config.domain.data_type();
        let handler = TemporalHandler::from_data_type(domain_type)?;

        let domain_start = get_temporal_value(&config.domain, 0, &handler)?;
        let domain_end = get_temporal_value(&config.domain, 1, &handler)?;

        let (range_start, range_end) = config.numeric_interval_range()?;

        // Get timezone for DST-aware duration calculations
        let tz_str = config.option_string("timezone", "UTC");
        let tz = parse_timezone(&tz_str)?;

        // Compute actual duration (accounting for DST)
        let actual_duration = compute_actual_duration_millis(domain_start, domain_end, &tz)?;
        let use_actual_duration = actual_duration != (domain_end - domain_start);

        // Ensure values are numeric
        let numeric_values = match values.data_type() {
            DataType::Float32 => values.clone(),
            _ => Arc::new(cast::cast(values, &DataType::Float32)?),
        };

        let float_array = numeric_values
            .as_any()
            .downcast_ref::<Float32Array>()
            .unwrap();
        let mut result_millis = Vec::with_capacity(float_array.len());

        // Invert each value
        for i in 0..float_array.len() {
            if float_array.is_null(i) {
                result_millis.push(None);
            } else {
                let value = float_array.value(i);
                // Reverse the scaling operation
                let normalized = (value - range_start) / (range_end - range_start);

                let millis = if use_actual_duration {
                    // For DST-aware inversion, we need to find the timestamp
                    // whose actual duration from domain_start equals the target
                    let target_duration = (actual_duration as f32 * normalized) as i64;

                    // Start with a nominal estimate
                    let mut estimate = domain_start + target_duration;

                    // Refine the estimate by checking actual duration
                    // This handles DST transitions correctly
                    for _ in 0..3 {
                        // Usually converges in 1-2 iterations
                        let actual = compute_actual_duration_millis(domain_start, estimate, &tz)?;
                        let error = target_duration - actual;
                        if error.abs() < 1000 {
                            // Within 1 second is good enough
                            break;
                        }
                        estimate += error;
                    }
                    estimate
                } else {
                    // No DST transitions, use simple linear interpolation
                    domain_start + ((domain_end - domain_start) as f32 * normalized) as i64
                };

                result_millis.push(Some(millis));
            }
        }

        // Convert milliseconds back to temporal array
        create_temporal_array_from_optional_millis(&result_millis, domain_type)
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

    fn scale_to_string(
        &self,
        config: &ScaleConfig,
        values: &ArrayRef,
    ) -> Result<ScalarOrArray<String>, AvengerScaleError> {
        // Get timezone for formatting
        let tz_str = config.option_string("timezone", "UTC");
        let tz = parse_timezone(&tz_str)?;

        // Determine the tick interval based on domain span and nice settings
        let domain_type = config.domain.data_type();
        let handler = TemporalHandler::from_data_type(domain_type)?;
        let domain_start = get_temporal_value(&config.domain, 0, &handler)?;
        let domain_end = get_temporal_value(&config.domain, 1, &handler)?;
        let span = domain_end - domain_start;

        // Get target tick count from nice option or default
        let target_count = match config.options.get("nice") {
            Some(scalar) => {
                if let Ok(true) = scalar.as_boolean() {
                    10.0
                } else {
                    scalar.as_f32().unwrap_or(10.0)
                }
            }
            None => 10.0,
        };

        // Select appropriate interval for formatting
        let interval = select_time_interval(span, target_count);

        // Create custom formatter
        let formatter = TemporalTickFormatter::new(interval, tz);
        let default = config.option_string("default", "");

        // Format based on the input data type
        match values.data_type() {
            DataType::Date32 => {
                let values = values.as_any().downcast_ref::<Date32Array>().unwrap();
                let dates: Vec<_> = (0..values.len())
                    .map(|i| {
                        if values.is_null(i) {
                            None
                        } else {
                            let days = values.value(i);
                            NaiveDate::from_num_days_from_ce_opt(days + 719_163)
                        }
                    })
                    .collect();
                Ok(ScalarOrArray::new_array(DateFormatter::format(
                    &formatter,
                    &dates,
                    Some(&default),
                )))
            }
            DataType::Date64 => {
                let values = values.as_any().downcast_ref::<Date64Array>().unwrap();
                let dates: Vec<_> = (0..values.len())
                    .map(|i| {
                        if values.is_null(i) {
                            None
                        } else {
                            let millis = values.value(i);
                            DateTime::from_timestamp(
                                millis / 1000,
                                ((millis % 1000) * 1_000_000) as u32,
                            )
                            .map(|dt| dt.naive_utc())
                        }
                    })
                    .collect();
                Ok(ScalarOrArray::new_array(TimestampFormatter::format(
                    &formatter,
                    &dates,
                    Some(&default),
                )))
            }
            DataType::Timestamp(unit, tz_opt) => {
                // Convert to NaiveDateTime first
                let timestamps = match unit {
                    TimeUnit::Second => {
                        let array = values
                            .as_any()
                            .downcast_ref::<TimestampSecondArray>()
                            .unwrap();
                        (0..array.len())
                            .map(|i| {
                                if array.is_null(i) {
                                    None
                                } else {
                                    DateTime::from_timestamp(array.value(i), 0)
                                        .map(|dt| dt.naive_utc())
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                    TimeUnit::Millisecond => {
                        let array = values
                            .as_any()
                            .downcast_ref::<TimestampMillisecondArray>()
                            .unwrap();
                        (0..array.len())
                            .map(|i| {
                                if array.is_null(i) {
                                    None
                                } else {
                                    let millis = array.value(i);
                                    DateTime::from_timestamp(
                                        millis / 1000,
                                        ((millis % 1000) * 1_000_000) as u32,
                                    )
                                    .map(|dt| dt.naive_utc())
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                    TimeUnit::Microsecond => {
                        let array = values
                            .as_any()
                            .downcast_ref::<TimestampMicrosecondArray>()
                            .unwrap();
                        (0..array.len())
                            .map(|i| {
                                if array.is_null(i) {
                                    None
                                } else {
                                    let micros = array.value(i);
                                    DateTime::from_timestamp(
                                        micros / 1_000_000,
                                        ((micros % 1_000_000) * 1_000) as u32,
                                    )
                                    .map(|dt| dt.naive_utc())
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                    TimeUnit::Nanosecond => {
                        let array = values
                            .as_any()
                            .downcast_ref::<TimestampNanosecondArray>()
                            .unwrap();
                        (0..array.len())
                            .map(|i| {
                                if array.is_null(i) {
                                    None
                                } else {
                                    let nanos = array.value(i);
                                    DateTime::from_timestamp(
                                        nanos / 1_000_000_000,
                                        (nanos % 1_000_000_000) as u32,
                                    )
                                    .map(|dt| dt.naive_utc())
                                }
                            })
                            .collect::<Vec<_>>()
                    }
                };

                if tz_opt.is_some() {
                    // Convert to UTC DateTime for timestamptz
                    let utc_timestamps: Vec<_> = timestamps
                        .iter()
                        .map(|opt| opt.map(|naive| Utc.from_utc_datetime(&naive)))
                        .collect();
                    Ok(ScalarOrArray::new_array(TimestamptzFormatter::format(
                        &formatter,
                        &utc_timestamps,
                        Some(&default),
                    )))
                } else {
                    // Use NaiveDateTime for timestamp without timezone
                    Ok(ScalarOrArray::new_array(TimestampFormatter::format(
                        &formatter,
                        &timestamps,
                        Some(&default),
                    )))
                }
            }
            _ => {
                // Fallback to default formatting
                let scaled = self.scale(config, values)?;
                config.context.formatters.format(&scaled, Some(&default))
            }
        }
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

#[allow(clippy::too_many_arguments)]
fn scale_timestamp_values(
    values: &ArrayRef,
    unit: &TimeUnit,
    handler: &TemporalHandler,
    domain_start: i64,
    domain_end: i64,
    range_start: f32,
    range_end: f32,
    tz: &Tz,
    use_actual_duration: bool,
    actual_duration: i64,
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
                    let normalized = if use_actual_duration {
                        // Use actual duration for DST-aware scaling
                        let value_duration =
                            compute_actual_duration_millis(domain_start, value, tz)?;
                        value_duration as f32 / actual_duration as f32
                    } else {
                        // Use nominal duration for performance when no DST
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32
                    };
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
                    let normalized = if use_actual_duration {
                        // Use actual duration for DST-aware scaling
                        let value_duration =
                            compute_actual_duration_millis(domain_start, value, tz)?;
                        value_duration as f32 / actual_duration as f32
                    } else {
                        // Use nominal duration for performance when no DST
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32
                    };
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
                    let normalized = if use_actual_duration {
                        // Use actual duration for DST-aware scaling
                        let value_duration =
                            compute_actual_duration_millis(domain_start, value, tz)?;
                        value_duration as f32 / actual_duration as f32
                    } else {
                        // Use nominal duration for performance when no DST
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32
                    };
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
                    let normalized = if use_actual_duration {
                        // Use actual duration for DST-aware scaling
                        let value_duration =
                            compute_actual_duration_millis(domain_start, value, tz)?;
                        value_duration as f32 / actual_duration as f32
                    } else {
                        // Use nominal duration for performance when no DST
                        (value - domain_start) as f32 / (domain_end - domain_start) as f32
                    };
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
                let base = dt.with_nanosecond(0).unwrap_or(dt);
                let secs = base.second() as i32;
                let target_sec = ((secs / n) * n) as u32;
                base.with_second(target_sec).unwrap_or(base)
            }
            TimeInterval::Minute(n) => {
                let base = dt
                    .with_second(0)
                    .unwrap_or(dt)
                    .with_nanosecond(0)
                    .unwrap_or(dt);
                let mins = base.minute() as i32;
                let target_min = ((mins / n) * n) as u32;
                base.with_minute(target_min).unwrap_or(base)
            }
            TimeInterval::Hour(n) => {
                let base = dt
                    .with_minute(0)
                    .unwrap_or(dt)
                    .with_second(0)
                    .unwrap_or(dt)
                    .with_nanosecond(0)
                    .unwrap_or(dt);
                let hours = base.hour() as i32;
                let target_hour = ((hours / n) * n) as u32;

                // Use safe hour setting for DST
                safe_time::safe_with_hour(base, target_hour).unwrap_or_else(|_| {
                    // If we can't set the hour due to DST, try the next valid hour
                    if target_hour < 23 {
                        safe_time::safe_with_hour(base, target_hour + 1).unwrap_or(base)
                    } else {
                        base
                    }
                })
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
            TimeInterval::Hour(n) => {
                // For hours, use calendar arithmetic to handle DST properly
                let total_hours = *n * count;
                let current_hour = dt.hour() as i32;
                let new_hour = current_hour + total_hours;

                if (0..24).contains(&new_hour) {
                    // Same day
                    safe_time::safe_with_hour(dt, new_hour as u32)
                        .unwrap_or_else(|_| dt + chrono::Duration::hours(total_hours as i64))
                } else {
                    // Spans days, use duration addition
                    dt + chrono::Duration::hours(total_hours as i64)
                }
            }
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

    /// Get actual duration of interval at a specific time (accounting for DST)
    #[allow(dead_code)]
    fn actual_duration(&self, start: DateTime<Tz>) -> chrono::Duration {
        let end = self.offset(start, 1);
        end - start
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

    // Validate that we didn't create an invalid range
    if nice_end <= nice_start {
        // This shouldn't happen, but if it does, fall back to original bounds
        return Ok((start_millis, end_millis));
    }

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
    let mut prev_millis = None;

    // Generate ticks within domain
    while current <= end_dt {
        let current_millis = current.timestamp_millis();

        // Ensure we're not generating duplicate ticks (can happen during fall-back)
        if prev_millis.is_none_or(|prev| current_millis > prev) {
            ticks.push(current_millis);
            prev_millis = Some(current_millis);
        }

        // Safe offset that handles DST
        let next = interval.offset(current, 1);

        // Safety check: ensure we're making progress
        if next <= current {
            break;
        }

        current = next;
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

/// Create temporal array from optional millisecond timestamps (for invert)
fn create_temporal_array_from_optional_millis(
    millis_vec: &[Option<i64>],
    data_type: &DataType,
) -> Result<ArrayRef, AvengerScaleError> {
    match data_type {
        DataType::Date32 => {
            // Convert milliseconds to days
            let days: Vec<Option<i32>> = millis_vec
                .iter()
                .map(|opt_ms| opt_ms.map(|ms| (ms / (24 * 60 * 60 * 1000)) as i32))
                .collect();
            Ok(Arc::new(Date32Array::from(days)))
        }
        DataType::Date64 => Ok(Arc::new(Date64Array::from(millis_vec.to_vec()))),
        DataType::Timestamp(TimeUnit::Second, tz) => {
            let secs: Vec<Option<i64>> = millis_vec
                .iter()
                .map(|opt_ms| opt_ms.map(|ms| ms / 1000))
                .collect();
            Ok(Arc::new(
                TimestampSecondArray::from(secs).with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Millisecond, tz) => Ok(Arc::new(
            TimestampMillisecondArray::from(millis_vec.to_vec()).with_timezone_opt(tz.clone()),
        )),
        DataType::Timestamp(TimeUnit::Microsecond, tz) => {
            let micros: Vec<Option<i64>> = millis_vec
                .iter()
                .map(|opt_ms| opt_ms.map(|ms| ms * 1000))
                .collect();
            Ok(Arc::new(
                TimestampMicrosecondArray::from(micros).with_timezone_opt(tz.clone()),
            ))
        }
        DataType::Timestamp(TimeUnit::Nanosecond, tz) => {
            let nanos: Vec<Option<i64>> = millis_vec
                .iter()
                .map(|opt_ms| opt_ms.map(|ms| ms * 1_000_000))
                .collect();
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::TimestampSecondArray;
    use avenger_common::value::ScalarOrArrayValue;

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

    #[test]
    fn test_time_scale_invert() -> Result<(), AvengerScaleError> {
        // Create domain from 2024-01-01 to 2024-12-31
        let start_date = 19723; // 2024-01-01 in days since epoch
        let end_date = 20088; // 2024-12-31 in days since epoch

        let domain_start = Arc::new(Date32Array::from(vec![start_date])) as ArrayRef;
        let domain_end = Arc::new(Date32Array::from(vec![end_date])) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));

        // Test inverting some range values
        let range_values = Arc::new(Float32Array::from(vec![0.0, 50.0, 100.0, 25.0])) as ArrayRef;
        let inverted = scale.invert(&range_values)?;
        let inverted_array = inverted.as_any().downcast_ref::<Date32Array>().unwrap();

        // Check results
        assert_eq!(inverted_array.value(0), start_date); // 0.0 -> start
        assert!((inverted_array.value(1) - 19905).abs() <= 1); // 50.0 -> mid-year (approximately)
        assert_eq!(inverted_array.value(2), end_date); // 100.0 -> end
        assert!(
            inverted_array.value(3) > start_date
                && inverted_array.value(3) < inverted_array.value(1)
        ); // 25.0 -> Q1

        Ok(())
    }

    #[test]
    fn test_dst_spring_forward() -> Result<(), AvengerScaleError> {
        // Test spring forward DST transition (2024-03-10 in US Eastern)
        let et = parse_timezone("America/New_York")?;

        // Test safe hour construction during spring forward
        let date = NaiveDate::from_ymd_opt(2024, 3, 10).unwrap();
        let _dt_before = safe_time::safe_and_hms(date, &et, 1, 30, 0, DstStrategy::Earliest)?;

        // Try to create 2:30 AM which doesn't exist (should jump to 3:30 AM)
        let dt_gap = safe_time::safe_and_hms(date, &et, 2, 30, 0, DstStrategy::Earliest)?;
        assert_eq!(dt_gap.hour(), 3); // Should have jumped forward
        assert_eq!(dt_gap.minute(), 30);

        // Check DST detection
        let transition = safe_time::find_dst_transition(date, &et);
        match transition {
            DstTransition::SpringForward {
                missing_start,
                missing_end,
            } => {
                assert_eq!(missing_start, 2);
                assert_eq!(missing_end, 3);
            }
            _ => panic!("Expected spring forward transition"),
        }

        Ok(())
    }

    #[test]
    fn test_dst_fall_back() -> Result<(), AvengerScaleError> {
        // Test fall back DST transition (2024-11-03 in US Eastern)
        let et = parse_timezone("America/New_York")?;

        let date = NaiveDate::from_ymd_opt(2024, 11, 3).unwrap();

        // Test both occurrences of 1:30 AM
        let dt_early = safe_time::safe_and_hms(date, &et, 1, 30, 0, DstStrategy::Earliest)?;
        let dt_late = safe_time::safe_and_hms(date, &et, 1, 30, 0, DstStrategy::Latest)?;

        // Both should be valid but different instants
        assert_eq!(dt_early.hour(), 1);
        assert_eq!(dt_late.hour(), 1);
        assert!(dt_early < dt_late); // Early occurrence comes before late

        // Check DST detection
        let transition = safe_time::find_dst_transition(date, &et);
        match transition {
            DstTransition::FallBack {
                repeated_start,
                repeated_end,
            } => {
                assert_eq!(repeated_start, 1);
                assert_eq!(repeated_end, 2);
            }
            _ => panic!("Expected fall back transition"),
        }

        Ok(())
    }

    #[test]
    fn test_dst_tick_generation() -> Result<(), AvengerScaleError> {
        // Test tick generation across DST boundary
        let et = parse_timezone("America/New_York")?;

        // Domain spans spring forward (2024-03-10 00:00 to 06:00 ET)
        let start_ts = et
            .with_ymd_and_hms(2024, 3, 10, 0, 0, 0)
            .unwrap()
            .timestamp();
        let end_ts = et
            .with_ymd_and_hms(2024, 3, 10, 6, 0, 0)
            .unwrap()
            .timestamp();

        let domain_start = Arc::new(TimestampSecondArray::from(vec![start_ts])) as ArrayRef;
        let domain_end = Arc::new(TimestampSecondArray::from(vec![end_ts])) as ArrayRef;

        let mut scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0));
        scale = scale.with_option("timezone", "America/New_York");

        // Generate hourly ticks
        let ticks = scale.ticks(Some(6.0))?;
        let tick_array = ticks
            .as_any()
            .downcast_ref::<TimestampSecondArray>()
            .unwrap();

        // Should have ticks at 0:00, 1:00, 3:00, 4:00, 5:00, 6:00 (skipping 2:00)
        let mut tick_hours = Vec::new();
        for i in 0..tick_array.len() {
            let ts = tick_array.value(i);
            let dt = et.timestamp_opt(ts, 0).unwrap();
            tick_hours.push(dt.hour());
        }

        // Verify 2:00 AM is skipped
        assert!(!tick_hours.contains(&2));

        Ok(())
    }

    #[test]
    fn test_no_dst_timezone() -> Result<(), AvengerScaleError> {
        // Test timezone without DST (UTC)
        let utc = parse_timezone("UTC")?;

        let date = NaiveDate::from_ymd_opt(2024, 3, 10).unwrap();

        // All hours should be valid in UTC
        for hour in 0..24 {
            let dt = safe_time::safe_and_hms(date, &utc, hour, 0, 0, DstStrategy::Earliest)?;
            assert_eq!(dt.hour(), hour);
        }

        // No transitions
        let transition = safe_time::find_dst_transition(date, &utc);
        assert_eq!(transition, DstTransition::None);

        Ok(())
    }

    #[test]
    fn test_dst_scale_operations() -> Result<(), AvengerScaleError> {
        // Test scaling across DST transitions
        let tz_str = "America/New_York";
        let et = parse_timezone(tz_str)?;

        // Create domain spanning spring-forward DST transition
        // 2024-03-10 00:00 to 04:00 ET (2:00 AM doesn't exist)
        let start = et.with_ymd_and_hms(2024, 3, 10, 0, 0, 0).unwrap();
        let end = et.with_ymd_and_hms(2024, 3, 10, 4, 0, 0).unwrap();

        let domain_start = Arc::new(TimestampMillisecondArray::from(vec![
            start.timestamp_millis()
        ])) as ArrayRef;
        let domain_end = Arc::new(TimestampMillisecondArray::from(
            vec![end.timestamp_millis()],
        )) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0))
            .with_option("timezone", tz_str);

        // Test scaling values across the DST gap
        let test_times = [
            et.with_ymd_and_hms(2024, 3, 10, 0, 0, 0).unwrap(), // Start
            et.with_ymd_and_hms(2024, 3, 10, 1, 0, 0).unwrap(), // Before gap
            et.with_ymd_and_hms(2024, 3, 10, 3, 0, 0).unwrap(), // After gap (2AM skipped)
            et.with_ymd_and_hms(2024, 3, 10, 4, 0, 0).unwrap(), // End
        ];

        let values = Arc::new(TimestampMillisecondArray::from(
            test_times
                .iter()
                .map(|dt| dt.timestamp_millis())
                .collect::<Vec<_>>(),
        )) as ArrayRef;

        let scaled = scale.scale(&values)?;
        let scaled_array = scaled.as_any().downcast_ref::<Float32Array>().unwrap();

        // The actual duration is 3 hours (not 4) due to DST
        // So scaling should reflect this
        assert_eq!(scaled_array.value(0), 0.0); // Start
        assert!((scaled_array.value(1) - 33.33).abs() < 1.0); // 1 hour / 3 hours ≈ 33.33%
        assert!((scaled_array.value(2) - 66.67).abs() < 1.0); // 2 hours / 3 hours ≈ 66.67%
        assert_eq!(scaled_array.value(3), 100.0); // End

        // Test inversion
        let range_values = Arc::new(Float32Array::from(vec![0.0, 33.33, 66.67, 100.0])) as ArrayRef;
        let inverted = scale.invert(&range_values)?;
        let inverted_array = inverted
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();

        // Check that inverted values match original times (approximately)
        for (i, test_time) in test_times.iter().enumerate() {
            let diff = (inverted_array.value(i) - test_time.timestamp_millis()).abs();
            assert!(diff < 60000, "Inverted time differs by more than 1 minute");
            // Within 1 minute
        }

        Ok(())
    }

    #[test]
    fn test_dst_fall_back_scale() -> Result<(), AvengerScaleError> {
        // Test scaling across fall-back DST transition
        let tz_str = "America/New_York";
        let et = parse_timezone(tz_str)?;

        // Create domain spanning fall-back DST transition
        // 2024-11-03 00:00 to 04:00 ET (1:00 AM happens twice)
        let start = et.with_ymd_and_hms(2024, 11, 3, 0, 0, 0).unwrap();
        let end = et.with_ymd_and_hms(2024, 11, 3, 4, 0, 0).unwrap();

        let domain_start = Arc::new(TimestampMillisecondArray::from(vec![
            start.timestamp_millis()
        ])) as ArrayRef;
        let domain_end = Arc::new(TimestampMillisecondArray::from(
            vec![end.timestamp_millis()],
        )) as ArrayRef;

        let scale = TimeScale::configured((domain_start, domain_end), (0.0, 100.0))
            .with_option("timezone", tz_str);

        // The actual duration is 5 hours (not 4) due to DST fall-back
        let values = Arc::new(TimestampMillisecondArray::from(vec![
            start.timestamp_millis(),
            start.timestamp_millis() + 3600 * 1000, // 1 hour later
            start.timestamp_millis() + 2 * 3600 * 1000, // 2 hours later (first 1 AM)
            start.timestamp_millis() + 3 * 3600 * 1000, // 3 hours later (second 1 AM)
            start.timestamp_millis() + 4 * 3600 * 1000, // 4 hours later
            end.timestamp_millis(),
        ])) as ArrayRef;

        let scaled = scale.scale(&values)?;
        let scaled_array = scaled.as_any().downcast_ref::<Float32Array>().unwrap();

        // Check scaling accounts for the extra hour
        assert_eq!(scaled_array.value(0), 0.0); // Start
        assert!((scaled_array.value(1) - 20.0).abs() < 1.0); // 1/5 = 20%
        assert!((scaled_array.value(2) - 40.0).abs() < 1.0); // 2/5 = 40%
        assert!((scaled_array.value(3) - 60.0).abs() < 1.0); // 3/5 = 60%
        assert!((scaled_array.value(4) - 80.0).abs() < 1.0); // 4/5 = 80%
        assert_eq!(scaled_array.value(5), 100.0); // End

        Ok(())
    }

    #[test]
    fn test_time_scale_tick_formatting() -> Result<(), AvengerScaleError> {
        // Test formatting with different intervals
        let tz_str = "America/New_York";

        // Test daily ticks
        let start = Arc::new(Date32Array::from(vec![19723])) as ArrayRef; // 2024-01-01
        let end = Arc::new(Date32Array::from(vec![19730])) as ArrayRef; // 2024-01-08

        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", tz_str)
            .with_option("nice", 7.0); // ~7 daily ticks

        let ticks = scale.ticks(Some(7.0))?;
        let formatted = scale.scale_to_string(&ticks)?;

        match formatted.value() {
            ScalarOrArrayValue::Array(strings) => {
                // Should show month and day for daily ticks
                assert!(strings[0].contains("Jan"));
                assert!(strings[0].contains("01") || strings[0].contains(" 1"));
            }
            _ => panic!("Expected array of strings"),
        }

        // Test hourly ticks with timestamps
        let start_ts = chrono_tz::America::New_York
            .with_ymd_and_hms(2024, 1, 1, 0, 0, 0)
            .unwrap()
            .timestamp_millis();
        let end_ts = chrono_tz::America::New_York
            .with_ymd_and_hms(2024, 1, 1, 12, 0, 0)
            .unwrap()
            .timestamp_millis();

        let start = Arc::new(TimestampMillisecondArray::from(vec![start_ts])) as ArrayRef;
        let end = Arc::new(TimestampMillisecondArray::from(vec![end_ts])) as ArrayRef;

        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", tz_str)
            .with_option("nice", 6.0); // ~6 hourly ticks

        let ticks = scale.ticks(Some(6.0))?;
        let formatted = scale.scale_to_string(&ticks)?;

        match formatted.value() {
            ScalarOrArrayValue::Array(strings) => {
                // Should show hours for hourly ticks
                assert!(strings[0].contains(":"));
                // Should not show seconds for hourly ticks
                assert!(!strings[0].contains(":00:00"));
            }
            _ => panic!("Expected array of strings"),
        }

        // Test yearly ticks
        let start = Arc::new(Date32Array::from(vec![18262])) as ArrayRef; // 2020-01-01
        let end = Arc::new(Date32Array::from(vec![20088])) as ArrayRef; // 2024-12-31

        let scale = TimeScale::configured((start, end), (0.0, 100.0))
            .with_option("timezone", tz_str)
            .with_option("nice", 5.0); // ~5 yearly ticks

        let ticks = scale.ticks(Some(5.0))?;
        let formatted = scale.scale_to_string(&ticks)?;

        match formatted.value() {
            ScalarOrArrayValue::Array(strings) => {
                // Should show only years for yearly ticks
                assert!(strings[0].starts_with("20")); // Year 20XX
                assert!(!strings[0].contains("Jan")); // No month
            }
            _ => panic!("Expected array of strings"),
        }

        Ok(())
    }
}
