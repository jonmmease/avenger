//! `timeunit`: the calendar unit, such as the month or the quarter, that contains each date
//! or timestamp.
//!
//! ```sql
//! SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
//! SELECT *, timeunit(t, 'unit', 'month') OVER () AS u FROM events
//! ```
//!
//! The options are pairs of a name and a constant value, after the value, as for `bin`:
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
//! - `week_start` (`'monday'`): `'monday'` or `'sunday'`.
//! - `fiscal_year_start`: the number of the month, from 1 to 12, in which a fiscal year
//!   starts, such as 10 for October. It sets the start of the units `fiscal_year` and
//!   `fiscal_quarter`, which need it.
//!
//! A unit is a name, with an optional count and plural before a truncating unit: `'hour'`,
//! `'3 hours'`, `'2 fiscal_quarters'`. A count steps evenly across the longer units, as
//! `date_bin` does, without restarting at each day, month, or year. Units of a week or shorter
//! count from the first day of the reference year below, Monday, January 1, 2024, or Sunday,
//! January 1, 2012, so `'7 days'` are weeks. Months, quarters, and years count from January of
//! year 0, and fiscal units from the start of fiscal year 0. A count that divides a longer
//! unit, such as `'15 minutes'` or `'3 months'`, starts at each start of it.
//!
//! The result is `struct<start, end, unit>`: the start of the unit, the start of the next
//! unit, both with the type of the value (a `DATE` stays a `DATE`, a time zone stays), and
//! the name of the unit, such as `'month'` or `'3 months'`. The field has the metadata key
//! `avenger.time_unit`.
//!
//! Time zones come from the type. A timestamp with a time zone is binned in civil time in
//! that zone. A `DATE` and a timestamp without a time zone hold civil time. The edges of a
//! unit shorter than a day are the instants whose local time is a whole number of units from
//! the origin: a local time that occurs two times gives two edges, and one that the zone
//! skips moves forward by the length of the skip. A unit of a day or longer starts at the
//! first instant of its first date. Collapsing units map each value into a reference year, a
//! leap year that starts on the first day of the week: 2024 for Monday, 2012 for Sunday.
//!
//! For a named time zone, a table of the offsets over the range of the values, which the
//! batches of a process share, holds each change of offset. It samples the offset once a day,
//! so it misses two changes within one day that cancel out.
//!
//! The functions:
//!
//! - `timeunit(x, ..options) OVER (..)`: the window function that users call.
//! - `__timeunit(x, ..options)`: internal; the unit of each value for a call with a `unit`.
//!   It has a name of its own because DataFusion 55.2 plans a scalar function before a
//!   window function of the same name and drops the `OVER` clause.
//! - `__timeunit_choose(x, ..options)`: internal aggregate; the unit of a partition.
//! - `__timeunit_apply(x, unit, ..options)`: internal; the unit of each value.
//!
//! The rule [`TimeUnitWindowRewrite`] plans `timeunit(..) OVER (..)` as `__timeunit` (with a
//! `unit`) or as `__timeunit_apply` with the unit from a scalar subquery (no `PARTITION BY`)
//! or from a join with the unit of each partition.

/// The metadata key of a result's field, whose value is JSON of the calendar options of its
/// units: `week_start` and `fiscal_year_start`.
pub const TAG: &str = "avenger.time_unit";

mod apply;
mod calendar;
mod choose;
mod functions;
mod options;
mod rule;
mod units;

pub use apply::*;
pub(crate) use calendar::*;
pub(crate) use choose::*;
pub use functions::*;
pub use options::*;
pub use rule::*;
pub use units::*;
