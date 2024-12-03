use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use chrono::{Datelike, NaiveDate, Weekday};

use super::opts::TemporalScaleOptions;

/// Define DateInterval as a trait for dates
pub trait DateInterval: Send + Sync + std::fmt::Debug {
    fn floor(&self, date: &NaiveDate) -> NaiveDate;
    fn offset(&self, date: NaiveDate, step: i64) -> NaiveDate;
    fn count(&self, start: NaiveDate, end: NaiveDate) -> i64;

    fn ceil(&self, date: &NaiveDate) -> NaiveDate {
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

    // Define individual interval structs
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

    impl DateInterval for DayInterval {
        fn floor(&self, date: &NaiveDate) -> NaiveDate {
            date.clone()
        }

        fn offset(&self, date: NaiveDate, step: i64) -> NaiveDate {
            date + chrono::Duration::days(step)
        }

        fn count(&self, start: NaiveDate, end: NaiveDate) -> i64 {
            (end - start).num_days()
        }
    }

    impl DateInterval for WeekInterval {
        fn floor(&self, date: &NaiveDate) -> NaiveDate {
            let days_from_sunday = date.weekday().num_days_from_sunday();
            let target_from_sunday = self.weekday.num_days_from_sunday();
            let days_to_subtract = (days_from_sunday + 7 - target_from_sunday) % 7;

            *date - chrono::Duration::days(days_to_subtract as i64)
        }

        fn offset(&self, date: NaiveDate, step: i64) -> NaiveDate {
            date + chrono::Duration::weeks(step)
        }

        fn count(&self, start: NaiveDate, end: NaiveDate) -> i64 {
            (end - start).num_weeks()
        }
    }

    impl DateInterval for MonthInterval {
        fn floor(&self, date: &NaiveDate) -> NaiveDate {
            NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap()
        }

        fn offset(&self, date: NaiveDate, step: i64) -> NaiveDate {
            let year = date.year() as i32;
            let month = date.month() as i32;

            let total_months = (year * 12 + month - 1) as i64 + step;
            let new_year = total_months.div_euclid(12);
            let new_month = total_months.rem_euclid(12) + 1;

            NaiveDate::from_ymd_opt(new_year as i32, new_month as u32, 1).unwrap()
        }

        fn count(&self, start: NaiveDate, end: NaiveDate) -> i64 {
            let start_months = (start.year() as i64) * 12 + (start.month() as i64);
            let end_months = (end.year() as i64) * 12 + (end.month() as i64);
            end_months - start_months
        }
    }

    impl DateInterval for YearInterval {
        fn floor(&self, date: &NaiveDate) -> NaiveDate {
            NaiveDate::from_ymd_opt(date.year(), 1, 1).unwrap()
        }

        fn offset(&self, date: NaiveDate, step: i64) -> NaiveDate {
            NaiveDate::from_ymd_opt(date.year() + step as i32, 1, 1).unwrap()
        }

        fn count(&self, start: NaiveDate, end: NaiveDate) -> i64 {
            (end.year() - start.year()) as i64
        }
    }

    // Factory functions
    pub fn day() -> Box<dyn DateInterval> {
        Box::new(DayInterval)
    }

    pub fn week(weekday: Weekday) -> Box<dyn DateInterval> {
        Box::new(WeekInterval::new(weekday))
    }

    pub fn month() -> Box<dyn DateInterval> {
        Box::new(MonthInterval)
    }

    pub fn year() -> Box<dyn DateInterval> {
        Box::new(YearInterval)
    }

    // Helper functions for specific weekdays
    pub fn week_sunday() -> Box<dyn DateInterval> {
        week(Weekday::Sun)
    }

    pub fn week_monday() -> Box<dyn DateInterval> {
        week(Weekday::Mon)
    }

    pub fn week_tuesday() -> Box<dyn DateInterval> {
        week(Weekday::Tue)
    }

    pub fn week_wednesday() -> Box<dyn DateInterval> {
        week(Weekday::Wed)
    }

    pub fn week_thursday() -> Box<dyn DateInterval> {
        week(Weekday::Thu)
    }

    pub fn week_friday() -> Box<dyn DateInterval> {
        week(Weekday::Fri)
    }

    pub fn week_saturday() -> Box<dyn DateInterval> {
        week(Weekday::Sat)
    }
}

/// A scale that maps dates to a numeric range
#[derive(Clone, Debug)]
pub struct DateScale {
    domain_start: NaiveDate,
    domain_end: NaiveDate,
    range_start: f32,
    range_end: f32,
    clamp: bool,
}

impl DateScale {
    /// Creates a new date scale with the specified domain and default range [0, 1]
    pub fn new(domain: (NaiveDate, NaiveDate)) -> Self {
        Self {
            domain_start: domain.0,
            domain_end: domain.1,
            range_start: 0.0,
            range_end: 1.0,
            clamp: false,
        }
    }

    /// Sets the input domain of the scale
    pub fn domain(mut self, domain: (NaiveDate, NaiveDate)) -> Self {
        self.domain_start = domain.0;
        self.domain_end = domain.1;
        self
    }

    /// Returns the current domain as (start, end)
    pub fn get_domain(&self) -> (NaiveDate, NaiveDate) {
        (self.domain_start, self.domain_end)
    }

    /// Sets the output range of the scale
    pub fn range(mut self, range: (f32, f32)) -> Self {
        self.range_start = range.0;
        self.range_end = range.1;
        self
    }

    /// Returns the current range as (start, end)
    pub fn get_range(&self) -> (f32, f32) {
        (self.range_start, self.range_end)
    }

    /// Enables or disables clamping of output values to the range
    pub fn clamp(mut self, clamp: bool) -> Self {
        self.clamp = clamp;
        self
    }

    /// Returns whether output clamping is enabled
    pub fn get_clamp(&self) -> bool {
        self.clamp
    }

    /// Internal conversion from NaiveDate to timestamp (days since epoch)
    fn to_timestamp(date: &NaiveDate) -> f64 {
        date.signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as f64
    }

    fn from_timestamp(ts: f64) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(1970, 1, 1).map(|epoch| epoch + chrono::Duration::days(ts as i64))
    }

    /// Maps input values from domain to range
    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, NaiveDate>>,
        opts: &TemporalScaleOptions,
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
        let range_offset = opts.range_offset.unwrap_or(0.0) as f64;
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

    /// Maps output values from range back to domain
    pub fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &TemporalScaleOptions,
    ) -> ScalarOrArray<NaiveDate> {
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

        let range_offset = opts.range_offset.unwrap_or(0.0) as f64;
        let offset = domain_start_ts - scale * (self.range_start as f64);

        if self.clamp {
            let range_lower = f32::min(self.range_start, self.range_end) as f64;
            let range_upper = f32::max(self.range_start, self.range_end) as f64;

            values.into().map(|v| {
                let v = (*v as f64 - range_offset).clamp(range_lower, range_upper);
                let days = scale * v + offset;
                Self::from_timestamp(days).unwrap_or(self.domain_start)
            })
        } else {
            values.into().map(|v| {
                let v = *v as f64 - range_offset;
                let days = scale * v + offset;
                Self::from_timestamp(days).unwrap_or(self.domain_start)
            })
        }
    }

    /// Extends the domain to nice round values
    pub fn nice(&mut self, interval: Option<Box<dyn DateInterval>>) -> &mut Self {
        if self.domain_start == self.domain_end {
            return self;
        }

        let interval = match interval {
            Some(i) => i,
            None => {
                let span = self.domain_end - self.domain_start;
                if span < chrono::Duration::days(30) {
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

    /// Generate date ticks for the scale
    pub fn ticks(&self, count: Option<usize>) -> Vec<NaiveDate> {
        let count = count.unwrap_or(10);

        // Define standard tick intervals
        let tick_intervals = [
            (interval::day(), 1, 1i64),      // 1 day
            (interval::day(), 2, 2),         // 2 days
            (interval::week_sunday(), 1, 7), // 1 week
            (interval::month(), 1, 30),      // 1 month
            (interval::month(), 3, 90),      // 3 months
            (interval::year(), 1, 365),      // 1 year
        ];

        // Calculate target step size based on domain span and desired tick count
        let span_days = (self.domain_end - self.domain_start).num_days().abs() as f64;
        let target_step = span_days / count as f64;

        // Find the most appropriate interval
        let (interval, step) = tick_intervals
            .into_iter()
            .find(|(_, _, step_days)| *step_days as f64 >= target_step)
            .map(|(interval, step, _)| (interval, step))
            .unwrap_or((interval::year(), (target_step / 365.0).ceil() as i64));

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

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn test_defaults() {
        let today = chrono::Local::now().date_naive();
        let scale = DateScale::new((today, today));
        assert_eq!(scale.domain_start, today);
        assert_eq!(scale.domain_end, today);
        assert_eq!(scale.range_start, 0.0);
        assert_eq!(scale.range_end, 1.0);
        assert_eq!(scale.clamp, false);
    }

    #[test]
    fn test_scale() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 11);
        let mid = date(2023, 1, 6);

        let scale = DateScale::new((start, end)).range((0.0, 100.0)).clamp(true);

        let values = vec![
            date(2022, 12, 31), // < domain
            start,              // domain start
            mid,                // middle
            end,                // domain end
            date(2023, 1, 12),  // > domain
        ];

        let result = scale
            .scale(&values, &Default::default())
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 0.0); // domain start
        assert_approx_eq!(f32, result[2], 50.0); // middle
        assert_approx_eq!(f32, result[3], 100.0); // domain end
        assert_approx_eq!(f32, result[4], 100.0); // clamped
    }

    #[test]
    fn test_scale_with_range_offset() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 11);
        let mid = date(2023, 1, 6);

        let scale = DateScale::new((start, end)).range((0.0, 100.0)).clamp(true);

        let values = vec![date(2022, 12, 31), start, mid, end, date(2023, 1, 12)];

        let result = scale
            .scale(
                &values,
                &TemporalScaleOptions {
                    range_offset: Some(10.0),
                },
            )
            .as_vec(values.len(), None);

        assert_approx_eq!(f32, result[0], 0.0); // clamped
        assert_approx_eq!(f32, result[1], 10.0); // domain start + offset
        assert_approx_eq!(f32, result[2], 60.0); // middle + offset
        assert_approx_eq!(f32, result[3], 100.0); // domain end (clamped)
        assert_approx_eq!(f32, result[4], 100.0); // clamped
    }

    #[test]
    fn test_scale_degenerate() {
        let today = chrono::Local::now().date_naive();
        let scale = DateScale::new((today, today))
            .range((0.0, 100.0))
            .clamp(true);

        let values = vec![
            today - chrono::Duration::days(1),
            today,
            today + chrono::Duration::days(1),
        ];

        let result = scale
            .scale(&values, &Default::default())
            .as_vec(values.len(), None);

        // All values should map to range_start
        for r in result {
            assert_approx_eq!(f32, r, 0.0);
        }
    }

    #[test]
    fn test_invert() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 11);

        let scale = DateScale::new((start, end)).range((0.0, 100.0)).clamp(true);

        let values = vec![-10.0, 0.0, 50.0, 100.0, 110.0];
        let result = scale
            .invert(&values, &Default::default())
            .as_vec(values.len(), None);

        assert_eq!(result[0], start); // clamped below
        assert_eq!(result[1], start); // range start
        assert_eq!(result[2], date(2023, 1, 6)); // middle
        assert_eq!(result[3], end); // range end
        assert_eq!(result[4], end); // clamped above
    }

    #[test]
    fn test_invert_with_range_offset() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 11);

        let scale = DateScale::new((start, end)).range((0.0, 100.0)).clamp(true);

        let values = vec![-10.0, 10.0, 60.0, 110.0, 120.0];
        let result = scale
            .invert(
                &values,
                &TemporalScaleOptions {
                    range_offset: Some(10.0),
                },
            )
            .as_vec(values.len(), None);

        assert_eq!(result[0], start); // clamped below
        assert_eq!(result[1], start); // range start
        assert_eq!(result[2], date(2023, 1, 6)); // middle
        assert_eq!(result[3], end); // range end
        assert_eq!(result[4], end); // clamped above
    }

    #[test]
    fn test_invert_reversed_range() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 11);

        let scale = DateScale::new((end, start))
            .range((100.0, 0.0)) // Reversed range
            .clamp(true);

        let values = vec![110.0, 100.0, 50.0, 0.0, -10.0];
        let result = scale
            .invert(&values, &Default::default())
            .as_vec(values.len(), None);

        assert_eq!(result[0], end); // clamped to end (> 100)
        assert_eq!(result[1], end); // range start (100.0)
        assert_eq!(result[2], date(2023, 1, 6)); // middle
        assert_eq!(result[3], start); // range end (0.0)
        assert_eq!(result[4], start); // clamped to start (< 0)
    }

    #[test]
    fn test_nice_with_interval() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 15);

        let mut scale = DateScale::new((start, end));
        scale.nice(Some(interval::month()));

        assert_eq!(scale.domain_start, date(2023, 1, 1));
        assert_eq!(scale.domain_end, date(2023, 2, 1));
    }

    #[test]
    fn test_nice_auto_interval() {
        // Test with ~15 day span (should use day interval)
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 15);

        let mut scale = DateScale::new((start, end));
        scale.nice(None);

        assert_eq!(scale.domain_start, date(2023, 1, 1));
        assert_eq!(scale.domain_end, date(2023, 1, 15));

        // Test with ~2 month span (should use month interval)
        let start = date(2023, 1, 15);
        let end = date(2023, 3, 15);

        let mut scale = DateScale::new((start, end));
        scale.nice(None);

        assert_eq!(scale.domain_start, date(2023, 1, 1));
        assert_eq!(scale.domain_end, date(2023, 4, 1));
    }

    #[test]
    fn test_ticks_days() {
        let start = date(2023, 1, 1);
        let end = date(2023, 1, 5);
        let scale = DateScale::new((start, end));

        let ticks = scale.ticks(Some(5));
        assert_eq!(ticks.len(), 5);
        assert_eq!(ticks[0], date(2023, 1, 1));
        assert_eq!(ticks[1], date(2023, 1, 2));
        assert_eq!(ticks[2], date(2023, 1, 3));
        assert_eq!(ticks[3], date(2023, 1, 4));
        assert_eq!(ticks[4], date(2023, 1, 5));
    }

    #[test]
    fn test_ticks_months() {
        let start = date(2023, 1, 1);
        let end = date(2023, 6, 1);
        let scale = DateScale::new((start, end));

        let ticks = scale.ticks(Some(6));
        assert_eq!(ticks.len(), 6);
        assert_eq!(ticks[0], date(2023, 1, 1));
        assert_eq!(ticks[1], date(2023, 2, 1));
        assert_eq!(ticks[2], date(2023, 3, 1));
        assert_eq!(ticks[3], date(2023, 4, 1));
        assert_eq!(ticks[4], date(2023, 5, 1));
        assert_eq!(ticks[5], date(2023, 6, 1));
    }

    #[test]
    fn test_ticks_years() {
        let start = date(2020, 1, 1);
        let end = date(2025, 1, 1);
        let scale = DateScale::new((start, end));

        let ticks = scale.ticks(Some(6));
        assert_eq!(ticks.len(), 6);
        assert_eq!(ticks[0], date(2020, 1, 1));
        assert_eq!(ticks[1], date(2021, 1, 1));
        assert_eq!(ticks[2], date(2022, 1, 1));
        assert_eq!(ticks[3], date(2023, 1, 1));
        assert_eq!(ticks[4], date(2024, 1, 1));
        assert_eq!(ticks[5], date(2025, 1, 1));
    }
}
