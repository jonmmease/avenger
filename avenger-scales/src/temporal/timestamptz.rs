use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};

use crate::numeric::ContinuousNumericScale;

use chrono::{DateTime, TimeZone, Utc};
use chrono::{Datelike, Timelike, Weekday};

/// Define TimestampTzInterval as a trait generic over timezone
pub trait TimestampTzInterval<Tz: TimeZone>: Send + Sync + std::fmt::Debug {
    fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz>;
    fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz>;
    fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64;

    fn ceil(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
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
    use chrono::TimeZone;

    // Define individual interval structs
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
    impl<Tz: TimeZone> TimestampTzInterval<Tz> for MillisecondInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.clone()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::milliseconds(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_milliseconds()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for SecondInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.with_nanosecond(0).unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::seconds(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_seconds()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for MinuteInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.with_second(0).unwrap().with_nanosecond(0).unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::minutes(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_minutes()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for HourInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::hours(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_hours()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for DayInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.with_hour(0)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::days(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_days()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for WeekInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            let days_from_sunday = date.weekday().num_days_from_sunday();
            let target_from_sunday = self.weekday.num_days_from_sunday();
            let days_to_subtract = (days_from_sunday + 7 - target_from_sunday) % 7;

            date.with_hour(0)
                .unwrap()
                .with_minute(0)
                .unwrap()
                .with_second(0)
                .unwrap()
                .with_nanosecond(0)
                .unwrap()
                - chrono::Duration::days(days_to_subtract as i64)
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date + chrono::Duration::weeks(step)
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end - start).num_weeks()
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for MonthInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.timezone()
                .with_ymd_and_hms(date.year(), date.month(), 1, 0, 0, 0)
                .unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            let naive = date.naive_local();
            let year = naive.year() as i32;
            let month = naive.month() as i32;

            let total_months = (year * 12 + month - 1) as i64 + step;
            let new_year = total_months.div_euclid(12);
            let new_month = total_months.rem_euclid(12) + 1;

            date.timezone()
                .with_ymd_and_hms(new_year as i32, new_month as u32, 1, 0, 0, 0)
                .unwrap()
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            let start_months = (start.year() as i64) * 12 + (start.month() as i64);
            let end_months = (end.year() as i64) * 12 + (end.month() as i64);
            end_months - start_months
        }
    }

    impl<Tz: TimeZone> TimestampTzInterval<Tz> for YearInterval {
        fn floor(&self, date: &DateTime<Tz>) -> DateTime<Tz> {
            date.timezone()
                .with_ymd_and_hms(date.year(), 1, 1, 0, 0, 0)
                .unwrap()
        }

        fn offset(&self, date: DateTime<Tz>, step: i64) -> DateTime<Tz> {
            date.timezone()
                .with_ymd_and_hms(date.year() + step as i32, 1, 1, 0, 0, 0)
                .unwrap()
        }

        fn count(&self, start: DateTime<Tz>, end: DateTime<Tz>) -> i64 {
            (end.year() - start.year()) as i64
        }
    }

    // Factory functions now need to specify timezone
    pub fn millisecond<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(MillisecondInterval)
    }

    pub fn second<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(SecondInterval)
    }

    pub fn minute<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(MinuteInterval)
    }

    pub fn hour<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(HourInterval)
    }

    pub fn day<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(DayInterval)
    }

    pub fn week<Tz: TimeZone>(weekday: Weekday) -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(WeekInterval::new(weekday))
    }

    // Helper functions for specific weekdays
    pub fn week_sunday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Sun)
    }

    pub fn week_monday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Mon)
    }

    pub fn week_tuesday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Tue)
    }

    pub fn week_wednesday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Wed)
    }

    pub fn week_thursday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Thu)
    }

    pub fn week_friday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Fri)
    }

    pub fn week_saturday<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        week(Weekday::Sat)
    }

    pub fn month<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(MonthInterval)
    }

    pub fn year<Tz: TimeZone>() -> Box<dyn TimestampTzInterval<Tz>> {
        Box::new(YearInterval)
    }
}

#[derive(Clone, Debug)]
pub struct TimestampTzScaleConfig {
    pub domain: (DateTime<Utc>, DateTime<Utc>),
    pub range: (f32, f32),
    pub clamp: bool,
    pub range_offset: f32,
    pub nice: bool,
}

impl Default for TimestampTzScaleConfig {
    fn default() -> Self {
        let d = DateTime::<Utc>::from_timestamp(0, 0).unwrap();
        Self {
            domain: (d, d),
            range: (0.0, 1.0),
            clamp: false,
            range_offset: 0.0,
            nice: false,
        }
    }
}

/// A scale that maps timestamps to a numeric range.
/// While calculations are done in UTC internally, the scale maintains awareness
/// of a display timezone for operations like tick generation and nice rounding.
#[derive(Clone, Debug)]
pub struct TimestampTzScale<Tz: TimeZone + Copy> {
    domain_start: DateTime<Utc>,
    domain_end: DateTime<Utc>,
    range_start: f32,
    range_end: f32,
    clamp: bool,
    range_offset: f32,
    display_tz: Tz,
}

impl<Tz: TimeZone + Copy> TimestampTzScale<Tz> {
    /// Creates a new timestamp scale with the specified UTC domain and default range [0, 1]
    pub fn new(config: &TimestampTzScaleConfig, display_tz: Tz) -> Self {
        let mut this = Self {
            domain_start: config.domain.0,
            domain_end: config.domain.1,
            range_start: config.range.0,
            range_end: config.range.1,
            clamp: config.clamp,
            range_offset: config.range_offset,
            display_tz,
        };
        if config.nice {
            this = this.nice(None);
        }
        this
    }

    /// Sets the timezone used for display operations (tick formatting, nice rounding)
    pub fn display_timezone(mut self, tz: Tz) -> Self {
        self.display_tz = tz;
        self
    }

    /// Gets the current display timezone
    pub fn get_display_timezone(&self) -> Tz {
        self.display_tz
    }

    /// Sets the input domain of the scale
    pub fn with_domain(mut self, domain: (DateTime<Utc>, DateTime<Utc>)) -> Self {
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

    /// Internal conversion from DateTime to timestamp
    fn to_timestamp(date: &DateTime<Utc>) -> f64 {
        date.timestamp_millis() as f64
    }

    fn from_timestamp(ts: f64) -> Option<DateTime<Utc>> {
        let s = (ts / 1000.0) as i64;
        let ms = ((ts - ((s as f64) * 1000.0)) * 1_000_000.0) as u32;
        DateTime::from_timestamp(s, ms)
    }

    /// Extends the domain to nice round values.
    pub fn nice(mut self, interval: Option<Box<dyn TimestampTzInterval<Tz>>>) -> Self {
        if self.domain_start == self.domain_end {
            return self;
        }

        let start = self.domain_start.with_timezone(&self.display_tz);
        let end = self.domain_end.with_timezone(&self.display_tz);

        let interval = match interval {
            Some(i) => i,
            None => {
                let span = end.clone().signed_duration_since(start.clone());
                if span < chrono::Duration::seconds(1) {
                    interval::millisecond::<Tz>()
                } else if span < chrono::Duration::minutes(1) {
                    interval::second::<Tz>()
                } else if span < chrono::Duration::hours(1) {
                    interval::minute::<Tz>()
                } else if span < chrono::Duration::days(1) {
                    interval::hour::<Tz>()
                } else if span < chrono::Duration::days(30) {
                    interval::day::<Tz>()
                } else if span < chrono::Duration::days(365) {
                    interval::month::<Tz>()
                } else {
                    interval::year::<Tz>()
                }
            }
        };

        let nice_start = interval.floor(&start).with_timezone(&Utc);
        let nice_end = interval.ceil(&end).with_timezone(&Utc);

        self.domain_start = nice_start;
        self.domain_end = nice_end;
        self
    }
}

impl<Tz> ContinuousNumericScale<DateTime<Utc>> for TimestampTzScale<Tz>
where
    Tz: TimeZone + Copy + 'static,
{
    fn domain(&self) -> (DateTime<Utc>, DateTime<Utc>) {
        (self.domain_start, self.domain_end)
    }

    fn range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    fn clamp(&self) -> bool {
        self.clamp
    }

    fn set_domain(&mut self, domain: (DateTime<Utc>, DateTime<Utc>)) {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
    }

    fn set_range(&mut self, range: (f32, f32)) {
        self.range_start = range.0;
        self.range_end = range.1;
    }

    fn set_clamp(&mut self, clamp: bool) {
        self.clamp = clamp;
    }

    fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, DateTime<Utc>>>,
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
    ) -> ScalarOrArray<DateTime<Utc>> {
        // Handle degenerate domain case
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

    fn ticks(&self, count: Option<f32>) -> Vec<DateTime<Utc>> {
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

        // Convert domain to display timezone
        let start = self.domain_start.with_timezone(&self.display_tz);
        let end = self.domain_end.with_timezone(&self.display_tz);

        // Calculate target step size based on domain span and desired tick count
        let span_ms = (end.clone() - start.clone()).num_milliseconds().abs() as f64;
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
        let mut tick = interval.floor(&start);
        let end_display = end;

        while tick <= end_display {
            ticks.push(tick.with_timezone(&Utc));
            tick = interval.offset(tick, step);
        }

        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono_tz::America::New_York;
    use float_cmp::assert_approx_eq;

    // Helper function to check if two DateTimes are approximately equal
    fn assert_datetime_approx_eq(left: DateTime<Utc>, right: DateTime<Utc>) {
        // Compare timestamps in milliseconds, allowing small differences
        let left_ms = left.timestamp_millis();
        let right_ms = right.timestamp_millis();
        assert!(
            (left_ms - right_ms).abs() <= 1,
            "DateTime difference too large: {:?} vs {:?}",
            left,
            right
        );
    }

    #[test]
    fn test_defaults() {
        let now = Utc::now();
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (now, now),
                ..Default::default()
            },
            Utc,
        );
        assert_eq!(scale.domain_start, now);
        assert_eq!(scale.domain_end, now);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
    }

    #[test]
    fn test_scale() {
        let start = Utc::now();
        let end = start + chrono::Duration::days(10);
        let mid = start + chrono::Duration::days(5);

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            Utc,
        )
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
        let start = Utc::now();
        let end = start + chrono::Duration::days(10);
        let mid = start + chrono::Duration::days(5);

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            Utc,
        )
        .with_range((0.0, 100.0))
        .with_range_offset(10.0)
        .with_clamp(true);

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
        let now = Utc::now();
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (now, now),
                ..Default::default()
            },
            Utc,
        )
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
        let start = Utc::now();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            Utc,
        )
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
        let start = Utc::now();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                range_offset: 10.0,
                ..Default::default()
            },
            Utc,
        )
        .with_range((0.0, 100.0))
        .with_clamp(true);

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
        let start = Utc::now();
        let end = start + chrono::Duration::days(10);

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (end, start),
                range: (100.0, 0.0),
                ..Default::default()
            },
            Utc,
        )
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
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 3, 45, 30).unwrap();
        let end = Utc.with_ymd_and_hms(2023, 1, 1, 15, 20, 45).unwrap();

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            New_York,
        )
        .nice(Some(interval::day()));

        // Nice calculation should happen in the display timezone, so when converted back to UTC it should not
        // be round days.
        assert_eq!(
            scale.domain_start,
            Utc.with_ymd_and_hms(2022, 12, 31, 5, 0, 0).unwrap()
        );
        assert_eq!(
            scale.domain_end,
            Utc.with_ymd_and_hms(2023, 1, 2, 5, 0, 0).unwrap()
        );

        // Nice in UTC should be round days
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            Utc,
        )
        .nice(Some(interval::day()));
        assert_eq!(
            scale.domain_start,
            Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap()
        );
        assert_eq!(
            scale.domain_end,
            Utc.with_ymd_and_hms(2023, 1, 2, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_nice_auto_interval() {
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 3, 45, 30).unwrap();
        let end = Utc.with_ymd_and_hms(2023, 1, 1, 9, 20, 45).unwrap();

        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            New_York,
        )
        .nice(None);

        assert_eq!(
            scale.domain_start,
            Utc.with_ymd_and_hms(2023, 1, 1, 3, 0, 0).unwrap()
        );
        assert_eq!(
            scale.domain_end,
            Utc.with_ymd_and_hms(2023, 1, 1, 10, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_ticks_hourly() {
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2023, 1, 1, 6, 0, 0).unwrap();
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            New_York,
        );

        // Should generate hourly ticks
        let ticks = scale.ticks(Some(6.0));
        assert_eq!(ticks.len(), 7); // 0, 1, 2, 3, 4, 5, 6
        assert_eq!(ticks[0], Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(ticks[6], Utc.with_ymd_and_hms(2023, 1, 1, 6, 0, 0).unwrap());
    }

    #[test]
    fn test_ticks_6_hours() {
        // Test ~2 day span
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2023, 1, 3, 0, 0, 0).unwrap();
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            New_York,
        );

        // Should generate 6-hourly ticks with default count
        let ticks = scale.ticks(None);
        assert!(ticks.len() >= 8); // At least start, end, and some intermediate points
        assert_eq!(ticks[0], Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(
            *ticks.last().unwrap(),
            Utc.with_ymd_and_hms(2023, 1, 3, 0, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_ticks_days() {
        // Test ~30 day span
        let start = Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2023, 1, 5, 0, 0, 0).unwrap();
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            New_York,
        );

        // Should place ticks at day boundaries in the display timezone
        let ticks = scale.ticks(Some(5.0));
        assert_eq!(ticks.len(), 5);
        assert_eq!(
            ticks[0],
            Utc.with_ymd_and_hms(2022, 12, 31, 5, 0, 0).unwrap()
        );
        assert_eq!(ticks[1], Utc.with_ymd_and_hms(2023, 1, 1, 5, 0, 0).unwrap());
        assert_eq!(ticks[2], Utc.with_ymd_and_hms(2023, 1, 2, 5, 0, 0).unwrap());
        assert_eq!(ticks[3], Utc.with_ymd_and_hms(2023, 1, 3, 5, 0, 0).unwrap());
        assert_eq!(ticks[4], Utc.with_ymd_and_hms(2023, 1, 4, 5, 0, 0).unwrap());

        // Should place ticks at day boundaries in UTC
        let scale = TimestampTzScale::new(
            &TimestampTzScaleConfig {
                domain: (start, end),
                ..Default::default()
            },
            Utc,
        );
        let ticks = scale.ticks(Some(5.0));
        assert_eq!(ticks.len(), 5);
        assert_eq!(ticks[0], Utc.with_ymd_and_hms(2023, 1, 1, 0, 0, 0).unwrap());
        assert_eq!(ticks[1], Utc.with_ymd_and_hms(2023, 1, 2, 0, 0, 0).unwrap());
        assert_eq!(ticks[2], Utc.with_ymd_and_hms(2023, 1, 3, 0, 0, 0).unwrap());
        assert_eq!(ticks[3], Utc.with_ymd_and_hms(2023, 1, 4, 0, 0, 0).unwrap());
        assert_eq!(ticks[4], Utc.with_ymd_and_hms(2023, 1, 5, 0, 0, 0).unwrap());
    }
}
