use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use chrono::{DateTime, Datelike, NaiveDateTime, Timelike, Weekday};

use crate::numeric::ContinuousNumericScale;

/// Define TimestampInterval as a trait for naive timestamps
pub trait TimestampInterval: Send + Sync + std::fmt::Debug {
    fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime;
    fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime;
    fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64;

    fn ceil(&self, date: &NaiveDateTime) -> NaiveDateTime {
        let floored = self.floor(date);
        if &floored == date {
            date.clone()
        } else {
            self.offset(floored, 1)
        }
    }
}

pub mod interval {
    use super::*;

    // Define individual interval structs (same as TimestampTz but without Tz parameter)
    #[derive(Debug, Clone)]
    pub struct MillisecondInterval;
    #[derive(Debug, Clone)]
    pub struct SecondInterval;
    #[derive(Debug, Clone)]
    pub struct MinuteInterval;
    #[derive(Debug, Clone)]
    pub struct HourInterval;
    #[derive(Debug, Clone)]
    pub struct DayInterval;
    #[derive(Debug, Clone)]
    pub struct WeekInterval {
        weekday: Weekday,
    }
    #[derive(Debug, Clone)]
    pub struct MonthInterval;
    #[derive(Debug, Clone)]
    pub struct YearInterval;

    impl WeekInterval {
        pub fn new(weekday: Weekday) -> Self {
            Self { weekday }
        }
    }

    // Implement the trait for each interval type
    impl TimestampInterval for MillisecondInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            date.clone()
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::milliseconds(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_milliseconds()
        }
    }

    impl TimestampInterval for DayInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            date.date().and_hms_opt(0, 0, 0).unwrap()
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::days(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_days()
        }
    }

    impl TimestampInterval for WeekInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            let days_from_sunday = date.weekday().num_days_from_sunday();
            let target_from_sunday = self.weekday.num_days_from_sunday();
            let days_to_subtract = (days_from_sunday + 7 - target_from_sunday) % 7;

            date.date().and_hms_opt(0, 0, 0).unwrap()
                - chrono::Duration::days(days_to_subtract as i64)
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::weeks(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_weeks()
        }
    }

    impl TimestampInterval for SecondInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            date.with_nanosecond(0).unwrap()
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::seconds(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_seconds()
        }
    }

    impl TimestampInterval for MinuteInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            date.with_second(0).unwrap().with_nanosecond(0).unwrap()
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::minutes(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_minutes()
        }
    }

    impl TimestampInterval for HourInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            date.with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            date + chrono::Duration::hours(step)
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end - start).num_hours()
        }
    }

    impl TimestampInterval for MonthInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            NaiveDateTime::new(
                date.date().with_day(1).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            let naive = date;
            let year = naive.year() as i32;
            let month = naive.month() as i32;

            let total_months = (year * 12 + month - 1) as i64 + step;
            let new_year = total_months.div_euclid(12);
            let new_month = total_months.rem_euclid(12) + 1;

            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(new_year as i32, new_month as u32, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            let start_months = (start.year() as i64) * 12 + (start.month() as i64);
            let end_months = (end.year() as i64) * 12 + (end.month() as i64);
            end_months - start_months
        }
    }

    impl TimestampInterval for YearInterval {
        fn floor(&self, date: &NaiveDateTime) -> NaiveDateTime {
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        }

        fn offset(&self, date: NaiveDateTime, step: i64) -> NaiveDateTime {
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(date.year() + step as i32, 1, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        }

        fn count(&self, start: NaiveDateTime, end: NaiveDateTime) -> i64 {
            (end.year() - start.year()) as i64
        }
    }

    // Factory functions (simpler than TimestampTz as they don't need timezone parameter)
    pub fn millisecond() -> Box<dyn TimestampInterval> {
        Box::new(MillisecondInterval)
    }

    pub fn day() -> Box<dyn TimestampInterval> {
        Box::new(DayInterval)
    }

    pub fn week(weekday: Weekday) -> Box<dyn TimestampInterval> {
        Box::new(WeekInterval::new(weekday))
    }

    // Helper functions for specific weekdays
    pub fn week_sunday() -> Box<dyn TimestampInterval> {
        week(Weekday::Sun)
    }

    pub fn week_monday() -> Box<dyn TimestampInterval> {
        week(Weekday::Mon)
    }

    pub fn week_tuesday() -> Box<dyn TimestampInterval> {
        week(Weekday::Tue)
    }

    pub fn week_wednesday() -> Box<dyn TimestampInterval> {
        week(Weekday::Wed)
    }

    pub fn week_thursday() -> Box<dyn TimestampInterval> {
        week(Weekday::Thu)
    }

    pub fn week_friday() -> Box<dyn TimestampInterval> {
        week(Weekday::Fri)
    }

    pub fn week_saturday() -> Box<dyn TimestampInterval> {
        week(Weekday::Sat)
    }
    pub fn second() -> Box<dyn TimestampInterval> {
        Box::new(SecondInterval)
    }

    pub fn minute() -> Box<dyn TimestampInterval> {
        Box::new(MinuteInterval)
    }

    pub fn hour() -> Box<dyn TimestampInterval> {
        Box::new(HourInterval)
    }

    pub fn month() -> Box<dyn TimestampInterval> {
        Box::new(MonthInterval)
    }

    pub fn year() -> Box<dyn TimestampInterval> {
        Box::new(YearInterval)
    }
}

pub struct TimestampScaleConfig {
    pub domain: (NaiveDateTime, NaiveDateTime),
    pub range: (f32, f32),
    pub clamp: bool,
    pub range_offset: f32,
    pub nice: bool,
}

impl Default for TimestampScaleConfig {
    fn default() -> Self {
        Self {
            domain: (NaiveDateTime::MIN, NaiveDateTime::MAX),
            range: (0.0, 1.0),
            clamp: false,
            range_offset: 0.0,
            nice: false,
        }
    }
}

/// A scale that maps naive timestamps to a numeric range
#[derive(Clone, Debug)]
pub struct TimestampScale {
    domain_start: NaiveDateTime,
    domain_end: NaiveDateTime,
    range_start: f32,
    range_end: f32,
    clamp: bool,
    range_offset: f32,
}

impl TimestampScale {
    /// Creates a new timestamp scale with the specified domain and default range [0, 1]
    pub fn new(config: &TimestampScaleConfig) -> Self {
        let mut this = Self {
            domain_start: config.domain.0,
            domain_end: config.domain.1,
            range_start: config.range.0,
            range_end: config.range.1,
            clamp: config.clamp,
            range_offset: config.range_offset,
        };
        if config.nice {
            this = this.nice(None);
        }
        this
    }

    /// Sets the input domain of the scale
    pub fn with_domain(mut self, domain: (NaiveDateTime, NaiveDateTime)) -> Self {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
        self
    }

    /// Sets the output range of the scale
    pub fn with_range(mut self, range: (f32, f32)) -> Self {
        self.range_start = range.0;
        self.range_end = range.1;
        self
    }

    /// Enables or disables clamping of output values to the range
    pub fn with_clamp(mut self, clamp: bool) -> Self {
        self.clamp = clamp;
        self
    }

    /// Sets the range offset
    pub fn with_range_offset(mut self, range_offset: f32) -> Self {
        self.range_offset = range_offset;
        self
    }

    /// Internal conversion from NaiveDateTime to timestamp
    fn to_timestamp(date: &NaiveDateTime) -> f64 {
        date.and_utc().timestamp_millis() as f64
    }

    fn from_timestamp(ts: f64) -> Option<NaiveDateTime> {
        let s = (ts / 1000.0) as i64;
        let ms = ((ts - ((s as f64) * 1000.0)) * 1_000_000.0) as u32;
        Some(DateTime::from_timestamp(s, ms)?.naive_utc())
    }

    /// Extends the domain to nice round values
    pub fn nice(mut self, interval: Option<Box<dyn TimestampInterval>>) -> Self {
        if self.domain_start == self.domain_end {
            return self;
        }

        let interval = match interval {
            Some(i) => i,
            None => {
                let span = self.domain_end - self.domain_start;
                if span < chrono::Duration::seconds(1) {
                    interval::millisecond()
                } else if span < chrono::Duration::minutes(1) {
                    interval::second()
                } else if span < chrono::Duration::hours(1) {
                    interval::minute()
                } else if span < chrono::Duration::days(1) {
                    interval::hour()
                } else if span < chrono::Duration::days(30) {
                    interval::day()
                } else if span < chrono::Duration::days(365) {
                    interval::month()
                } else {
                    interval::year()
                }
            }
        };

        let nice_start = interval.floor(&self.domain_start);
        let nice_end = interval.ceil(&self.domain_end);

        self.domain_start = nice_start;
        self.domain_end = nice_end;
        self
    }
}

impl ContinuousNumericScale<NaiveDateTime> for TimestampScale {
    fn domain(&self) -> (NaiveDateTime, NaiveDateTime) {
        (self.domain_start, self.domain_end)
    }

    fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    fn clamp(&self) -> bool {
        self.clamp
    }

    fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, NaiveDateTime>>,
    ) -> ScalarOrArray<f32> {
        if self.domain_start == self.domain_end || self.range_start == self.range_end {
            return values.into().map(|_| self.range_start);
        }

        let range_start = self.range_start as f64;
        let range_end = self.range_end as f64;

        let domain_start_ts = Self::to_timestamp(&self.domain_start);
        let domain_end_ts = Self::to_timestamp(&self.domain_end);
        let domain_span = domain_end_ts - domain_start_ts;
        let scale = (range_end - range_start) / domain_span;
        let range_offset = self.range_offset as f64;
        let offset = (range_start - scale * domain_start_ts + range_offset) as f32;

        if self.clamp {
            let (range_min, range_max) = if self.range_start <= self.range_end {
                (self.range_start, self.range_end)
            } else {
                (self.range_end, self.range_start)
            };

            values.into().map(|v| {
                let v_ts = Self::to_timestamp(&v);
                ((scale * v_ts) as f32 + offset).clamp(range_min, range_max)
            })
        } else {
            values.into().map(|v| {
                let v_ts = Self::to_timestamp(&v);
                (scale * v_ts) as f32 + offset
            })
        }
    }

    fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
    ) -> ScalarOrArray<NaiveDateTime> {
        if self.domain_start == self.domain_end
            || self.range_start == self.range_end
            || self.range_start.is_nan()
            || self.range_end.is_nan()
        {
            return values.into().map(|_| self.domain_start);
        }

        let domain_start_ts = Self::to_timestamp(&self.domain_start);
        let domain_end_ts = Self::to_timestamp(&self.domain_end);

        let scale =
            (domain_end_ts - domain_start_ts) / (self.range_end as f64 - self.range_start as f64);

        let range_offset = self.range_offset as f64;
        let offset = domain_start_ts - scale * (self.range_start as f64);

        if self.clamp {
            let range_lower = f32::min(self.range_start, self.range_end) as f64;
            let range_upper = f32::max(self.range_start, self.range_end) as f64;

            values.into().map(|v| {
                let v = (*v as f64 - range_offset).clamp(range_lower, range_upper);
                let millis = scale * v + offset;
                Self::from_timestamp(millis).unwrap_or(self.domain_start)
            })
        } else {
            values.into().map(|v| {
                let v = *v as f64 - range_offset;
                let millis = scale * v + offset;
                Self::from_timestamp(millis).unwrap_or(self.domain_start)
            })
        }
    }

    fn ticks(&self, count: Option<f32>) -> Vec<NaiveDateTime> {
        let count = count.unwrap_or(10.0);

        // Define standard tick intervals
        let tick_intervals = [
            (interval::second(), 1, 1000i64),        // 1 second
            (interval::second(), 5, 5000),           // 5 seconds
            (interval::second(), 15, 15000),         // 15 seconds
            (interval::second(), 30, 30000),         // 30 seconds
            (interval::minute(), 1, 60000),          // 1 minute
            (interval::minute(), 5, 300000),         // 5 minutes
            (interval::minute(), 15, 900000),        // 15 minutes
            (interval::minute(), 30, 1800000),       // 30 minutes
            (interval::hour(), 1, 3600000),          // 1 hour
            (interval::hour(), 3, 10800000),         // 3 hours
            (interval::hour(), 6, 21600000),         // 6 hours
            (interval::hour(), 12, 43200000),        // 12 hours
            (interval::day(), 1, 86400000),          // 1 day
            (interval::day(), 2, 172800000),         // 2 days
            (interval::week_sunday(), 1, 604800000), // 1 week
            (interval::month(), 1, 2592000000),      // 1 month
            (interval::month(), 3, 7776000000),      // 3 months
            (interval::year(), 1, 31536000000),      // 1 year
        ];

        // Calculate target step size based on domain span and desired tick count
        let span_ms = (self.domain_end - self.domain_start)
            .num_milliseconds()
            .abs() as f64;
        let target_step = span_ms / count as f64;

        // Find the most appropriate interval
        let (interval, step) = tick_intervals
            .into_iter()
            .find(|(_, _, step_ms)| *step_ms as f64 >= target_step)
            .map(|(interval, step, _)| (interval, step))
            .unwrap_or((
                interval::year(),
                (target_step / 31536000000.0).ceil() as i64,
            ));

        // Generate ticks using the selected interval
        let mut ticks = Vec::new();
        let mut tick = interval.floor(&self.domain_start);

        while tick <= self.domain_end {
            ticks.push(tick);
            tick = interval.offset(tick, step);
        }

        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use float_cmp::assert_approx_eq;

    // Helper function to check if two NaiveDateTimes are approximately equal
    fn assert_datetime_approx_eq(left: NaiveDateTime, right: NaiveDateTime) {
        // Compare timestamps in milliseconds, allowing small differences
        let left_ms = left.and_utc().timestamp_millis();
        let right_ms = right.and_utc().timestamp_millis();
        assert!(
            (left_ms - right_ms).abs() <= 1,
            "DateTime difference too large: {:?} vs {:?}",
            left,
            right
        );
    }

    #[test]
    fn test_defaults() {
        let now = chrono::Utc::now().naive_utc();
        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (now, now),
            ..Default::default()
        });
        assert_eq!(scale.domain_start, now);
        assert_eq!(scale.domain_end, now);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
    }

    #[test]
    fn test_scale() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);
        let mid = start + chrono::Duration::days(5);

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .with_range((0.0, 100.0))
        .with_clamp(true);

        let values = vec![
            start - chrono::Duration::days(1), // < domain
            start,                             // domain start
            mid,                               // middle
            end,                               // domain end
            end + chrono::Duration::days(1),   // > domain
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 0.0); // domain start
        assert_approx_eq!(f32, result[2], 50.0); // middle
        assert_approx_eq!(f32, result[3], 100.0); // domain end
        assert_approx_eq!(f32, result[4], 100.0); // clamped
    }

    #[test]
    fn test_scale_with_range_offset() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);
        let mid = start + chrono::Duration::days(5);

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .with_range((0.0, 100.0))
        .with_clamp(true)
        .with_range_offset(10.0);

        let values = vec![
            start - chrono::Duration::days(1),
            start,
            mid,
            end,
            end + chrono::Duration::days(1),
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 10.0); // domain start + offset
        assert_approx_eq!(f32, result[2], 60.0); // middle + offset
        assert_approx_eq!(f32, result[3], 100.0); // domain end (clamped)
        assert_approx_eq!(f32, result[4], 100.0); // clamped
    }

    #[test]
    fn test_scale_degenerate() {
        let now = chrono::Utc::now().naive_utc();
        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (now, now),
            ..Default::default()
        })
        .with_range((0.0, 100.0))
        .with_clamp(true);

        let values = vec![
            now - chrono::Duration::days(1),
            now,
            now + chrono::Duration::days(1),
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);

        // All values should map to range_start
        for r in result {
            assert_approx_eq!(f32, r, 0.0);
        }
    }

    #[test]
    fn test_invert() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .with_range((0.0, 100.0))
        .with_clamp(true);

        let values = vec![-10.0, 0.0, 50.0, 100.0, 110.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_datetime_approx_eq(result[0], start); // clamped below
        assert_datetime_approx_eq(result[1], start); // range start
        assert_datetime_approx_eq(result[2], start + chrono::Duration::days(5)); // middle
        assert_datetime_approx_eq(result[3], end); // range end
        assert_datetime_approx_eq(result[4], end); // clamped above
    }

    #[test]
    fn test_invert_with_range_offset() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .with_range((0.0, 100.0))
        .with_clamp(true)
        .with_range_offset(10.0);

        let values = vec![-10.0, 10.0, 60.0, 110.0, 120.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_datetime_approx_eq(result[0], start); // clamped below
        assert_datetime_approx_eq(result[1], start); // range start
        assert_datetime_approx_eq(result[2], start + chrono::Duration::days(5)); // middle
        assert_datetime_approx_eq(result[3], end); // range end
        assert_datetime_approx_eq(result[4], end); // clamped above
    }

    #[test]
    fn test_invert_reversed_range() {
        let start = DateTime::from_timestamp(0, 0).unwrap().naive_utc();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (end, start),
            ..Default::default()
        })
        .with_range((100.0, 0.0)) // Reversed range
        .with_clamp(true);

        let values = vec![110.0, 100.0, 50.0, 0.0, -10.0];
        let result = scale.invert(&values).as_vec(values.len(), None);

        assert_datetime_approx_eq(result[0], end); // clamped to end (> 100)
        assert_datetime_approx_eq(result[1], end); // range start (100.0)
        assert_datetime_approx_eq(result[2], start + chrono::Duration::days(5)); // middle
        assert_datetime_approx_eq(result[3], start); // range end (0.0)
        assert_datetime_approx_eq(result[4], start); // clamped to start (< 0)
    }

    #[test]
    fn test_nice_with_interval() {
        let start = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(3, 45, 30).unwrap(),
        );
        let end = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(15, 20, 45).unwrap(),
        );

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .nice(Some(interval::day()));

        assert_eq!(
            scale.domain_start,
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        );
        assert_eq!(
            scale.domain_end,
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(2023, 1, 2).unwrap(),
                chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            )
        );
    }

    #[test]
    fn test_nice_auto_interval() {
        let start = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(3, 45, 30).unwrap(),
        );
        let end = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(9, 20, 45).unwrap(),
        );

        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        })
        .nice(None);

        assert_eq!(
            scale.domain_start,
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(3, 0, 0).unwrap(),
            )
        );
        assert_eq!(
            scale.domain_end,
            NaiveDateTime::new(
                chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
                chrono::NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            )
        );
    }

    #[test]
    fn test_ticks_hourly() {
        let start = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        let end = start + chrono::Duration::hours(6);
        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        });

        // Should generate hourly ticks
        let ticks = scale.ticks(Some(6.0));
        assert_eq!(ticks.len(), 7); // 0, 1, 2, 3, 4, 5, 6
        assert_eq!(ticks[0], start);
        assert_eq!(ticks[6], end);
    }

    #[test]
    fn test_ticks_days() {
        let start = NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        );
        let end = start + chrono::Duration::days(5);
        let scale = TimestampScale::new(&TimestampScaleConfig {
            domain: (start, end),
            ..Default::default()
        });

        let ticks = scale.ticks(Some(5.0));
        assert_eq!(ticks.len(), 6);
        assert_eq!(ticks[0], start);
        assert_eq!(ticks[1], start + chrono::Duration::days(1));
        assert_eq!(ticks[2], start + chrono::Duration::days(2));
        assert_eq!(ticks[3], start + chrono::Duration::days(3));
        assert_eq!(ticks[4], start + chrono::Duration::days(4));
        assert_eq!(ticks[5], end);
    }
}
