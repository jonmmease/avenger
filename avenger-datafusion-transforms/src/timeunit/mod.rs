//! `timeunit`: the calendar unit, such as the month or the quarter, that contains each date
//! or timestamp.
//!
//! A call is a [`TimeUnitCall`], which [`plan`] adds to its input, or among the windows of a
//! projection, the placeholder window of [`window`], which [`crate::plan_windows`] plans. The
//! options, [`TimeUnitOptions`], are:
//!
//! - `unit`: a truncating unit, such as `'month'`, `'15 minutes'`, `'fiscal_quarter'`, or a
//!   collapsing unit, such as `'month_of_year'`. Without it, `timeunit` chooses a unit for
//!   each window partition. With it, the partitions do not matter, so `OVER ()` serves.
//! - `maxbins` (20): the largest number of units that the chosen unit permits.
//! - `min_unit`: the shortest unit that `timeunit` may choose. Without it, the floor is
//!   the resolution of the data: the finest calendar field that is not at its start in some
//!   value (`month` for values that are all first days of months), and a multiple of the
//!   greatest common divisor of the differences of the values (`30 minutes` for values
//!   every 10 minutes).
//! - `week_start` (1): the first day of a week, as GoogleSQL's `DAYOFWEEK` numbers it, from 1
//!   for Sunday to 7 for Saturday, such as 2 for Monday.
//! - `fiscal_year_start`: the number of the month, from 1 to 12, in which a fiscal year
//!   starts, such as 10 for October. It sets the start of the units `fiscal_year` and
//!   `fiscal_quarter`, which need it.
//!
//! A unit is a name, with an optional count and plural before a truncating unit: `'hour'`,
//! `'3 hours'`, `'2 fiscal_quarters'`. A unit whose bounds depend on a calendar names it in
//! parentheses after its name: `week`, `week_of_year`, and `day_of_week` the first day of their
//! weeks, as `'week(2)'` names Monday, and the fiscal units the first month of their fiscal
//! years, as `'fiscal_quarter(10)'` names October. A unit without it takes the calendar of the
//! options, and a unit that names one sets the option, so `'week(2)'` with `week_start => 1` is
//! an error.
//!
//! A count steps evenly across the longer units, as `date_bin` does, without restarting at each
//! day, month, or year. Days and the units shorter than a day count from January 1, 2012, and
//! weeks from the first day of the reference year of their first day, so `'2 weeks(2)'` count
//! from Monday, January 1, 2024. Months, quarters, and years count from January of year 0, and
//! fiscal units from the start of fiscal year 0. A count that divides a longer unit, such as
//! `'15 minutes'` or `'3 months'`, starts at each start of it.
//!
//! The result is `struct<start, end, unit>`: the start of the unit, the start of the next
//! unit, both with the type of the value (a `DATE` stays a `DATE`, a time zone stays), and
//! the text of the unit with its calendar, such as `'month'`, `'3 months'`, or `'week(1)'`, so
//! `impute` steps through the units of a key in the key's calendar.
//!
//! Time zones come from the type. A timestamp with a time zone is binned in civil time in
//! that zone. A `DATE` and a timestamp without a time zone hold civil time. The edges of a
//! unit shorter than a day are the instants whose local time is a whole number of units from
//! the origin: a local time that occurs two times gives two edges, and one that the zone
//! skips moves forward by the length of the skip. A unit of a day or longer starts at the
//! first instant of its first date. Collapsing units map each value into a reference year, the
//! leap year 2012, or for `week_of_year` and `day_of_week`, a leap year that starts on the
//! first day of their weeks: 2012 for Sunday, 2024 for Monday, 2008 for Tuesday, 2020 for
//! Wednesday, 2004 for Thursday, 2016 for Friday, and 2000 for Saturday.
//!
//! For a named time zone, a table of the offsets over the range of the values, which the
//! batches of a process share, holds each change of offset. It samples the offset once a day,
//! so it misses two changes within one day that cancel out.
//!
//! The plan of a call with a `unit` is the internal function `__timeunit` of each row. Without
//! a `unit`, the internal aggregate `__timeunit_choose` chooses the unit of each partition, from
//! a join of an aggregate by the partition, or without a partition, a scalar subquery, and the
//! internal function `__timeunit_apply` gives the unit of each row.

mod apply;
mod calendar;
mod choose;
mod functions;
mod options;
mod units;

pub use apply::*;
pub(crate) use calendar::*;
pub(crate) use choose::*;
pub use functions::*;
pub use options::*;
pub use units::*;
