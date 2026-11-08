//! The automatic unit of a partition.

use super::*;

use datafusion::arrow::array::{Array, ArrayRef};
use datafusion::common::Result;

/// What the choice of a unit needs to know about the values of a partition. All of it
/// merges, so it is the state of an aggregate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stats {
    pub n: u64,
    pub min: i64,
    pub max: i64,
    /// The finest calendar field of any value in civil time (`Calendar::rank`).
    pub rank: u8,
    /// The greatest common divisor of the differences of the values in civil time, in ticks.
    pub gcd: u64,
    /// One value in civil time, for the differences.
    pub reference: i64,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            n: 0,
            min: i64::MAX,
            max: i64::MIN,
            rank: 7,
            gcd: 0,
            reference: 0,
        }
    }
}

pub(super) fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Stats {
    pub(super) fn add(&mut self, cal: &mut Calendar, t: i64) {
        let local = cal.local_ticks(t);
        if self.n == 0 {
            self.reference = local;
        } else {
            self.gcd = gcd(self.gcd, local.abs_diff(self.reference));
        }
        self.n += 1;
        self.min = self.min.min(t);
        self.max = self.max.max(t);
        if self.rank > 0 {
            self.rank = self.rank.min(cal.rank(local));
        }
    }

    pub fn merge(&mut self, o: &Stats) {
        if o.n == 0 {
            return;
        }
        if self.n == 0 {
            *self = *o;
            return;
        }
        self.gcd = gcd(gcd(self.gcd, o.gcd), self.reference.abs_diff(o.reference));
        self.n += o.n;
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
        self.rank = self.rank.min(o.rank);
    }

    pub fn of(x: &ArrayRef, cal: &mut Calendar) -> Result<Stats> {
        let mut s = Stats::default();
        s.add_array(x, cal)?;
        Ok(s)
    }

    /// Adds the values of an array.
    pub(super) fn add_array(&mut self, x: &ArrayRef, cal: &mut Calendar) -> Result<()> {
        let ticks = ticks_of(x)?;
        check_range(
            cal,
            x.data_type(),
            &ticks,
            (0..ticks.len()).filter(|i| x.is_valid(*i)),
            None,
        )?;
        prepare(cal, x, &ticks);
        for (i, t) in ticks.into_iter().enumerate() {
            if x.is_valid(i) {
                self.add(cal, t);
            }
        }
        Ok(())
    }
}

/// The automatic units, in order. Years go on as 1, 2, or 5 times a power of ten.
pub(super) fn table(fiscal: bool) -> Vec<Unit> {
    let mut t = Vec::new();
    let mut add = |u: Trunc, ks: &[u32]| t.extend(ks.iter().map(|k| Unit::Trunc(u, *k)));
    add(Trunc::Millisecond, &[1, 2, 5, 10, 20, 50, 100, 200, 500]);
    add(Trunc::Second, &[1, 5, 15, 30]);
    add(Trunc::Minute, &[1, 5, 15, 30]);
    add(Trunc::Hour, &[1, 3, 6, 12]);
    add(Trunc::Day, &[1, 2]);
    add(Trunc::Week, &[1]);
    add(Trunc::Month, &[1]);
    if fiscal {
        add(Trunc::FiscalQuarter, &[1]);
        add(Trunc::FiscalYear, &[1, 2, 5, 10]);
    } else {
        add(Trunc::Quarter, &[1]);
        add(Trunc::Year, &[1, 2, 5, 10]);
    }
    t
}

pub(super) fn first_at_least(target_ms: f64, fiscal: bool) -> Unit {
    if let Some(u) = table(fiscal)
        .into_iter()
        .find(|u| u.approx_ms() >= target_ms)
    {
        return u;
    }
    let year = if fiscal {
        Trunc::FiscalYear
    } else {
        Trunc::Year
    };
    let mut p = 10u32;
    loop {
        for m in [2, 5, 10] {
            let u = Unit::Trunc(year, p * m);
            // Past a billion years, the range check of the unit fails.
            if u.approx_ms() >= target_ms || p >= 100_000_000 {
                return u;
            }
        }
        p *= 10;
    }
}

pub(super) fn longer(a: Unit, b: Unit) -> Unit {
    if b.approx_ms() > a.approx_ms() { b } else { a }
}

/// The shortest unit that the resolution of the data permits: the unit of the finest
/// calendar field of the values, or a unit whose length is a multiple of the common
/// difference of the values, whichever is longer.
pub(crate) fn resolution_floor(s: &Stats, kind: Kind, fiscal: bool) -> Unit {
    use Trunc::*;
    let mut floor = match s.rank {
        0 => Unit::Trunc(Millisecond, 1),
        1 => Unit::Trunc(Second, 1),
        2 => Unit::Trunc(Minute, 1),
        3 => Unit::Trunc(Hour, 1),
        4 => Unit::Trunc(Day, 1),
        5 => Unit::Trunc(Month, 1),
        6 => Unit::Trunc(if fiscal { FiscalQuarter } else { Quarter }, 1),
        _ => Unit::Trunc(if fiscal { FiscalYear } else { Year }, 1),
    };
    if s.rank <= 4 && s.gcd > 0 {
        let g = s.gcd as f64;
        let ticks = |u: &Unit| u.exact_ms().map(|ms| ms as f64 * kind.per_ms());
        let units = table(fiscal);
        // A unit that holds the same number of values, up to 12 times the difference.
        let multiple = units.iter().find(|u| {
            ticks(u).is_some_and(|t| {
                t >= g && t <= 12.0 * g && t.fract() == 0.0 && (t as u64).is_multiple_of(s.gcd)
            })
        });
        let at_least = || {
            units
                .iter()
                .find(|u| u.approx_ms() * kind.per_ms() >= g)
                .copied()
        };
        if let Some(u) = multiple.copied().or_else(at_least) {
            floor = longer(floor, u);
        }
    }
    if kind == Kind::Days {
        floor = longer(floor, Unit::Trunc(Day, 1));
    }
    floor
}

/// The unit of a partition: the first unit of `table` with at least
/// `(max - min) / maxbins`, and no shorter than the floor.
pub(crate) fn choose(s: &Stats, o: &TimeUnitOptions, kind: Kind) -> Option<Unit> {
    if s.n == 0 {
        return None;
    }
    let fiscal = o.fiscal_year_start.is_some();
    let floor = match o.min_unit {
        Some(u) => u,
        None => resolution_floor(s, kind, fiscal),
    };
    let span_ms = s.max.abs_diff(s.min) as f64 / kind.per_ms();
    if span_ms == 0.0 {
        return Some(longer(Unit::Trunc(Trunc::Day, 1), floor));
    }
    let target = span_ms / o.maxbins.unwrap_or(20) as f64;
    if target <= floor.approx_ms() {
        return Some(floor);
    }
    Some(first_at_least(target, fiscal))
}
