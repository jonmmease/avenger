//! A price explorer, built with avenger-chart and shown by a chart app, which loads the rows of
//! each settled window of dates into a new dashboard on a task.
//!
//! The example generates seeded daily closing prices and volumes of four sectors from 2010 to
//! 2024. The dashboard is a row of the fit modes over a column of three charts:
//!
//! - The modes: `Refit`, `Keep`, and `Extend`, how each load fits the scales of its dashboard.
//! - The detail: an equal-weight index of the sectors, as a line on the left axis, over bars of
//!   the total volume of each period on a second axis on the right.
//! - The sectors: the price of each sector in a facet of its own, with its own `y`.
//! - The overview: the weekly index of every year.
//!
//! The dates of the detail and the sectors read their domain from the selection `window`. A drag
//! on the overview draws a box of dates that writes the window, so the detail and the sectors
//! zoom to it. A drag on the detail or on a sector pans the window,
//! and the wheel zooms it, which moves the box in the overview. A double click on the detail or
//! on a sector, or a click on the overview, clears the window, so the detail and the sectors
//! show every date.
//!
//! The app reports each settled change of the window, and the example requests a new dashboard
//! for it from a task of charts that the app attaches: the rows of the window and of half its
//! span on each side, so a pan shows rows at once, at a resolution for the span of those rows,
//! daily, weekly, or monthly. The task runs each load on the app's runtime, and a newer window
//! cancels a load that has not finished. The app shows the newest finished dashboard at its next
//! build with the window in place. The title of the detail names the resolution of its rows.
//!
//! A click on a mode selects how each load fits the scales of its dashboard: `Refit` fits them
//! to the load's rows, `Keep` keeps the domains that they showed, and `Extend` grows them to hold
//! the load's rows too. The mode moves the `y` scales of the detail's index and of the sectors.
//! The dates read the window, and the volume has an explicit domain, so the mode leaves both
//! unchanged. A click away from the modes means `Refit`.

use std::sync::{Arc, Mutex, PoisonError};

use arrow::array::{Array, ArrayRef, AsArray, Date32Array, Float64Array, RecordBatch, StringArray};
use arrow::compute::cast;
use arrow::datatypes::{DataType, Date32Type};
use avenger_app::task::Cancelled;
use avenger_chart::{
    proto::{self, select, Channel, SelectionResolution},
    Chart, Contribution, Encode, Scale, Selection, TupleTest, Value,
};
use avenger_chart_app::{ChartApp, ChartChange, ChartTask, ChartUpdate, Fit};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use chrono::{Datelike, Days, Months, NaiveDate};

const DATE: &str = "date";
/// The first date after a period.
const END: &str = "end";
const INDEX: &str = "index";
const VOLUME: &str = "volume";
const PRICE: &str = "price";
const SECTOR: &str = "sector";

/// The selection of the dates that the detail and the sectors show.
const WINDOW: &str = "window";

/// The selection of the fit mode of each load's scales, and its column.
const FIT: &str = "fit";
const MODE: &str = "mode";

/// The fit modes, by their names.
const MODES: [(&str, Fit); 3] = [
    ("Refit", Fit::Refit),
    ("Keep", Fit::Keep),
    ("Extend", Fit::Extend),
];

/// Each sector, with the drift and the volatility of its log price in a year, and its mean daily
/// volume in millions of shares.
const SECTORS: [(&str, f64, f64, f64); 4] = [
    ("Technology", 0.14, 0.26, 180.0),
    ("Energy", 0.03, 0.30, 120.0),
    ("Health care", 0.08, 0.17, 90.0),
    ("Utilities", 0.05, 0.13, 60.0),
];

/// The seed of the prices.
const SEED: u64 = 7;

/// The most points that a series of the detail or a sector draws at a resolution.
const MAX_POINTS: i64 = 500;

/// The size of the window.
const SIZE: [f32; 2] = [1200.0, 820.0];

/// A seeded source of uniform and normal numbers, by SplitMix64.
struct Random(u64);

impl Random {
    /// A number in [0, 1).
    fn uniform(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A number from the standard normal distribution, by the Box-Muller transform.
    fn normal(&mut self) -> f64 {
        let u = self.uniform().max(f64::MIN_POSITIVE);
        let v = self.uniform();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }
}

/// The daily closing prices and volumes of the sectors on the weekdays from 2010 to 2024.
struct Prices {
    days: Vec<NaiveDate>,
    /// The closing price of each sector on each day, from 100 before the first day.
    closes: Vec<[f64; 4]>,
    /// The volume of each sector on each day, in millions of shares.
    volumes: Vec<[f64; 4]>,
}

impl Prices {
    /// Generates the prices. A market shock moves every sector, and a shock of its own moves
    /// each sector, and the volume of a sector grows with the size of its move.
    fn generate(seed: u64) -> Self {
        let mut random = Random(seed);
        let first = NaiveDate::from_ymd_opt(2010, 1, 1).unwrap();
        let last = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let mut prices = Prices {
            days: Vec::new(),
            closes: Vec::new(),
            volumes: Vec::new(),
        };
        let mut close = [100.0; 4];
        for day in first.iter_days().take_while(|day| *day <= last) {
            if day.weekday().number_from_monday() > 5 {
                continue;
            }
            let market = random.normal();
            let mut volume = [0.0; 4];
            for (sector, (_, drift, volatility, mean_volume)) in SECTORS.iter().enumerate() {
                let shock = 0.6 * market + 0.8 * random.normal();
                close[sector] *= (drift / 252.0 + volatility / 252f64.sqrt() * shock).exp();
                volume[sector] =
                    mean_volume * (0.25 * random.normal()).exp() * (0.6 + 0.5 * shock.abs());
            }
            prices.days.push(day);
            prices.closes.push(close);
            prices.volumes.push(volume);
        }
        prices
    }

    fn first(&self) -> NaiveDate {
        self.days[0]
    }

    fn last(&self) -> NaiveDate {
        self.days[self.days.len() - 1]
    }
}

/// The length of the period of each point of a series.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Resolution {
    Day,
    Week,
    Month,
}

impl Resolution {
    /// The finest resolution that draws at most `MAX_POINTS` points over the dates from `from`
    /// to `to`, which hold about five weekdays in seven days.
    fn for_dates(from: NaiveDate, to: NaiveDate) -> Self {
        let days = (to - from).num_days() + 1;
        if days * 5 / 7 <= MAX_POINTS {
            Resolution::Day
        } else if days / 7 <= MAX_POINTS {
            Resolution::Week
        } else {
            Resolution::Month
        }
    }

    /// The first date of the period of `day`: the day, its Monday, or the first of its month.
    fn period(self, day: NaiveDate) -> NaiveDate {
        match self {
            Resolution::Day => day,
            Resolution::Week => day - Days::new(day.weekday().num_days_from_monday().into()),
            Resolution::Month => day.with_day(1).unwrap(),
        }
    }

    /// The first date after the period that starts at `start`.
    fn end(self, start: NaiveDate) -> NaiveDate {
        match self {
            Resolution::Day => start + Days::new(1),
            Resolution::Week => start + Days::new(7),
            Resolution::Month => start + Months::new(1),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Resolution::Day => "day",
            Resolution::Week => "week",
            Resolution::Month => "month",
        }
    }
}

/// The closes and volumes of the days from `from` to `to` by period of `resolution`: the first
/// date of each period, the last close of each sector in it, and the sum of each sector's volume.
fn periods(
    prices: &Prices,
    [from, to]: [NaiveDate; 2],
    resolution: Resolution,
) -> Vec<(NaiveDate, [f64; 4], [f64; 4])> {
    let mut periods: Vec<(NaiveDate, [f64; 4], [f64; 4])> = Vec::new();
    let days = prices.days.iter().zip(&prices.closes).zip(&prices.volumes);
    for ((day, close), volume) in days.filter(|((day, _), _)| (from..=to).contains(*day)) {
        let period = resolution.period(*day);
        match periods.last_mut() {
            Some((last, closes, volumes)) if *last == period => {
                *closes = *close;
                for (total, volume) in volumes.iter_mut().zip(volume) {
                    *total += volume;
                }
            }
            _ => periods.push((period, *close, *volume)),
        }
    }
    periods
}

/// A column of `dates`.
fn date_column(dates: impl Iterator<Item = NaiveDate>) -> ArrayRef {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).unwrap();
    Arc::new(Date32Array::from_iter_values(
        dates.map(|date| (date - epoch).num_days() as i32),
    ))
}

fn dates(periods: &[(NaiveDate, [f64; 4], [f64; 4])]) -> ArrayRef {
    date_column(periods.iter().map(|(date, _, _)| *date))
}

/// The equal-weight index of the sectors, the mean of their prices, and their total volume, by
/// period of `resolution`, with the first date after each period.
fn index(periods: &[(NaiveDate, [f64; 4], [f64; 4])], resolution: Resolution) -> RecordBatch {
    let index = (periods.iter()).map(|(_, closes, _)| closes.iter().sum::<f64>() / 4.0);
    let volume = (periods.iter()).map(|(_, _, volumes)| volumes.iter().sum::<f64>());
    let ends = periods.iter().map(|(start, _, _)| resolution.end(*start));
    RecordBatch::try_from_iter([
        (DATE, dates(periods)),
        (END, date_column(ends)),
        (
            INDEX,
            Arc::new(Float64Array::from_iter_values(index)) as ArrayRef,
        ),
        (
            VOLUME,
            Arc::new(Float64Array::from_iter_values(volume)) as ArrayRef,
        ),
    ])
    .unwrap()
}

/// The price of the sector at index `sector` by period.
fn sector_prices(periods: &[(NaiveDate, [f64; 4], [f64; 4])], sector: usize) -> RecordBatch {
    let price = (periods.iter()).map(|(_, closes, _)| closes[sector]);
    RecordBatch::try_from_iter([
        (DATE, dates(periods)),
        (
            PRICE,
            Arc::new(Float64Array::from_iter_values(price)) as ArrayRef,
        ),
    ])
    .unwrap()
}

/// The selection of the dates that the detail and the sectors show. The overview's box and the
/// navigations of the detail and the sectors write it, and the newest gesture replaces it.
fn window() -> Selection {
    Selection::new(WINDOW, SelectionResolution::Global)
}

/// The dates of the detail and the sectors, whose domain is the window, or the dates of their
/// rows without one.
fn windowed_dates() -> Encode {
    Encode::column(DATE)
        .title("Date")
        .scale(Scale::new().selection(WINDOW, Channel::X))
}

/// A navigation of the dates.
fn navigate_dates() -> proto::Navigate {
    proto::Navigate {
        channels: vec![Channel::X as i32],
    }
}

/// The tooltip of a period of the index: its date, its index, and its volume.
fn tooltip() -> proto::Tooltip {
    let field = |column: &str, title: &str, format: &str| proto::TooltipField {
        column: column.into(),
        title: Some(title.into()),
        format: format.into(),
        ..Default::default()
    };
    proto::Tooltip {
        find: Some(proto::tooltip::Find::Nearest(proto::NearestFind {
            max_radius: 16.0,
            ..Default::default()
        })),
        header: Some(field(DATE, "Date", "%b %-d, %Y")),
        fields: vec![
            field(INDEX, "Index", ".1f"),
            field(VOLUME, "Volume (millions)", ",.0f"),
        ],
        ..Default::default()
    }
}

/// The detail: the index on the left axis over bars of the total volume of each period on a
/// right axis, whose domain is three times the most volume so the bars fill the bottom third,
/// with the dates of the window, which a drag pans and the wheel zooms.
fn detail(index: &RecordBatch, resolution: Resolution) -> Chart {
    let volumes = index.column_by_name(VOLUME).unwrap().as_primitive();
    let most = arrow::compute::max::<arrow::datatypes::Float64Type>(volumes).unwrap_or(1.0);
    let axis = |side: proto::Side| proto::Axis {
        side: side as i32,
        ..Default::default()
    };
    // A bar of each period spans its dates, so its width shows the resolution, and an inset
    // parts neighboring bars.
    let bars = proto::RectMark {
        inset: Some(0.5),
        ..Default::default()
    };
    let volume = Chart::unit(index.clone(), bars)
        .x(windowed_dates())
        .x2(Encode::column(END))
        .y(Encode::scaled_constant(0.0)
            .title("Volume (millions of shares)")
            .scale(
                Scale::new()
                    .domain(Arc::new(Float64Array::from(vec![0.0, 3.0 * most])))
                    .guide(proto::Axis {
                        grid: Some(false),
                        ..axis(proto::Side::Right)
                    }),
            ))
        .y2(Encode::column(VOLUME))
        .color(Encode::constant("#9aa5b1"))
        .opacity(Encode::constant(0.6))
        .selection(window())
        .build()
        .unwrap();
    let index_layer = |mark: proto::mark::Mark| {
        Chart::unit(index.clone(), mark)
            .x(windowed_dates())
            .y(Encode::column(INDEX)
                .title("Index")
                .scale(Scale::new().guide(axis(proto::Side::Left))))
            .selection(window())
    };
    // A line is one instance with no row, so invisible points at its vertices are the instances
    // that the tooltip finds.
    let points = index_layer(proto::SymbolMark::default().into())
        .opacity(Encode::constant(0.0))
        .build()
        .unwrap();
    let line = index_layer(proto::LineMark::default().into())
        .build()
        .unwrap();
    let index = Chart::overlay([line, points]).build().unwrap();
    let title = format!(
        "Index and volume by {}: drag to pan, scroll to zoom, double click for every date",
        resolution.name()
    );
    Chart::overlay([volume, index])
        .id("detail")
        .title(title.as_str())
        .tool(navigate_dates())
        .tool(tooltip())
        .independent(Channel::Y)
        .build()
        .unwrap()
}

/// The sectors: the price of each sector in a facet of its own, with its own `y`, and the dates
/// of the window.
fn sectors(periods: &[(NaiveDate, [f64; 4], [f64; 4])]) -> Chart {
    let panels = SECTORS.iter().enumerate().map(|(index, (name, ..))| {
        Chart::unit(sector_prices(periods, index), proto::LineMark::default())
            .id(SECTOR)
            .tool(navigate_dates())
            .x(windowed_dates())
            .y(Encode::column(PRICE).title("Price"))
            .partition(SECTOR, *name)
            .selection(window())
            .build()
            .unwrap()
    });
    Chart::row(panels)
        .facet(SECTOR)
        .facet_header(proto::FacetHeader {
            title: Some(String::new()),
            ..Default::default()
        })
        .independent(Channel::Y)
        .build()
        .unwrap()
}

/// The selection of the fit mode, whose one contribution names a mode, `Refit` at first.
fn fit_selection() -> Selection {
    Selection::new(FIT, SelectionResolution::Global).initial(Contribution::new("modes").tuple([
        TupleTest::Set {
            column: MODE.into(),
            channel: Channel::Unspecified,
            values: Arc::new(StringArray::from(vec![MODES[0].0])),
        },
    ]))
}

/// The row of the fit modes: a symbol for each mode, which its axis names, where a click selects
/// the mode and a condition highlights the selected one.
fn modes() -> Chart {
    let names: Vec<&str> = MODES.iter().map(|(name, _)| *name).collect();
    let table = RecordBatch::try_from_iter([(
        MODE,
        Arc::new(StringArray::from(names.clone())) as ArrayRef,
    )])
    .unwrap();
    let selected = proto::Predicate {
        predicate: Some(proto::predicate::Predicate::Selection(
            proto::SelectionTest {
                selection: FIT.into(),
                empty_selects_none: true,
                ..Default::default()
            },
        )),
    };
    let order = Scale::new().domain(Arc::new(StringArray::from(names)));
    Chart::unit(table, proto::SymbolMark::default())
        .id("modes")
        .title("Fit the scales of each load: click a mode")
        .tool(proto::Select {
            selection: FIT.into(),
            form: Some(select::Form::Points(proto::PointSelect {
                key: MODE.into(),
                toggle: false,
            })),
            scope: Vec::new(),
        })
        .x(Encode::column(MODE).untitled().scale(order))
        .color(Encode::constant("#c4c9d0").when(selected, Value::Constant("#4c78a8".into())))
        .size(Encode::constant(220.0))
        .selection(fit_selection())
        .build()
        .unwrap()
}

/// The overview: the weekly index of every year, where a drag draws a box of dates that writes
/// the window.
fn overview(prices: &Prices) -> Chart {
    let weeks = periods(prices, [prices.first(), prices.last()], Resolution::Week);
    Chart::unit(index(&weeks, Resolution::Week), proto::AreaMark::default())
        .id("overview")
        .title("Drag a box of dates, click to show every date")
        .tool(proto::Select {
            selection: WINDOW.into(),
            form: Some(select::Form::Interval(proto::IntervalSelect {
                channels: vec![Channel::X as i32],
            })),
            scope: Vec::new(),
        })
        .x(Encode::column(DATE).title("Date"))
        .y(Encode::column(INDEX).title("Index"))
        .selection(window())
        .build()
        .unwrap()
}

/// A dashboard of the rows of a window.
struct Load {
    chart: Chart,
    /// The first and the last date of the rows of the detail and the sectors.
    dates: [NaiveDate; 2],
    resolution: Resolution,
}

/// The source of dashboards: the prices, and the overview, which every dashboard shares.
struct Loader {
    prices: Prices,
    overview: Chart,
}

impl Loader {
    fn new(prices: Prices) -> Self {
        let overview = overview(&prices);
        Self { prices, overview }
    }

    /// The dashboard of `window`: the rows of the window and of half its span on each side, or
    /// of every date without a window, at the finest resolution that draws at most `MAX_POINTS`
    /// points.
    fn load(&self, window: Option<[NaiveDate; 2]>) -> Load {
        let (first, last) = (self.prices.first(), self.prices.last());
        let dates = match window {
            Some([from, to]) => {
                let margin = Days::new(((to - from).num_days() / 2).max(0) as u64);
                [(from - margin).max(first), (to + margin).min(last)]
            }
            None => [first, last],
        };
        let resolution = Resolution::for_dates(dates[0], dates[1]);
        let periods = periods(&self.prices, dates, resolution);
        let share = |weight| proto::Track {
            size: Some(proto::track::Size::Share(proto::Share {
                weight,
                max: None,
            })),
        };
        let chart = Chart::column([
            modes(),
            detail(&index(&periods, resolution), resolution),
            sectors(&periods),
            self.overview.clone(),
        ])
        .layout(proto::ConcatLayout {
            tracks: vec![
                proto::Track {
                    size: Some(proto::track::Size::Length(28.0)),
                },
                share(3.0),
                share(2.0),
                share(1.0),
            ],
            ..Default::default()
        })
        .build()
        .unwrap();
        Load {
            chart,
            dates,
            resolution,
        }
    }
}

/// The dates of the window that `contributions` hold, or `None` for an empty window.
fn window_of(contributions: &[Contribution]) -> Option<[NaiveDate; 2]> {
    let ends = (contributions.iter())
        .flat_map(|contribution| contribution.tuples.iter().flatten())
        .find_map(|test| match test {
            TupleTest::Range { column, ends, .. } if column == DATE => Some(ends),
            _ => None,
        })?;
    let days = cast(ends, &DataType::Date32).ok()?;
    let days = days.as_primitive::<Date32Type>();
    if days.len() != 2 || days.null_count() > 0 {
        return None;
    }
    Some([days.value_as_date(0)?, days.value_as_date(1)?])
}

/// What a dashboard depends on: its window of dates, or `None` for every date, and the fit mode
/// of its scales.
type Key = (Option<[NaiveDate; 2]>, Fit);

/// The task of the dashboards, by their keys.
type Dashboards = ChartTask<Key>;

/// The fit mode that `contributions` select, or `Fit::Refit` without one.
fn mode_of(contributions: &[Contribution]) -> Fit {
    (contributions.iter())
        .flat_map(|contribution| contribution.tuples.iter().flatten())
        .find_map(|test| match test {
            TupleTest::Set { column, values, .. } if column == MODE => {
                let names = values.as_string_opt::<i32>()?;
                let name = names.iter().flatten().next()?;
                MODES
                    .iter()
                    .find(|(own, _)| *own == name)
                    .map(|(_, fit)| *fit)
            }
            _ => None,
        })
        .unwrap_or(Fit::Refit)
}

/// The callback of the app, which requests the dashboard of the settled window under the settled
/// fit mode from `dashboards` when either settles. The key of a request holds both, because the
/// task ignores a request for the key of the request that waits or runs, or of the latest
/// dashboard. A newer request cancels the load that waits or runs, and a load that a newer request
/// cancels before it starts loads nothing.
fn on_change(loader: Arc<Loader>, dashboards: Dashboards) -> impl Fn(&ChartChange) + Send + Sync {
    let settled: Mutex<Key> = Mutex::new((None, Fit::Refit));
    move |change| {
        let window = change.selection(WINDOW).map(window_of);
        let mode = change.selection(FIT).map(mode_of);
        if !change.settled || (window.is_none() && mode.is_none()) {
            return;
        }
        let key = {
            let mut settled = settled.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(window) = window {
                settled.0 = window;
            }
            if let Some(mode) = mode {
                settled.1 = mode;
            }
            *settled
        };
        let loader = loader.clone();
        dashboards.request(key, move |cancel| async move {
            cancel.yield_now().await?;
            let (window, fit) = key;
            let load = loader.load(window);
            let [from, to] = load.dates;
            println!(
                "Loaded {from} to {to} by {}, {fit:?}",
                load.resolution.name()
            );
            Ok::<_, Cancelled>(ChartUpdate::new(load.chart).fit(fit))
        });
    }
}

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
        .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()))
}

fn main() {
    let loader = Arc::new(Loader::new(Prices::generate(SEED)));
    let dashboards = Dashboards::new();
    let app = ChartApp::new(loader.load(None).chart, engine())
        .size(SIZE)
        .charts(&dashboards)
        .on_change(on_change(loader, dashboards.clone()));
    let avenger_app = pollster::block_on(app.build()).expect("Failed to create AvengerApp");
    // The task's loads and the host's event handling share one runtime.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .build()
        .expect("the runtime builds");
    let avenger_app = avenger_app
        .with_runtime(runtime.handle().clone())
        .expect("the runtime is multi-thread");
    let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, runtime);
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}

#[cfg(test)]
mod tests;
