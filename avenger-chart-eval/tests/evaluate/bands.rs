//! Marks in the bands of band and nested band scales: band positions and levels, sizes as
//! fractions of a band, and custom scales of categories.

use std::sync::Arc;

use arrow::array::RecordBatch;
use avenger_chart::prelude::*;
use avenger_chart_eval::{ChartEvaluator, Registry};
use avenger_scenegraph::marks::mark::SceneMark;

use crate::support::*;

/// A bar spans its band whatever its band position, and a point mark stands at its encoding's
/// `band_position` of its band, or at the middle without one.
#[test]
fn a_band_position_places_points_and_keeps_bars_across_their_bands() {
    let bars = |x: EncodingBuilder| {
        let bars = CartesianLayer::bar(
            categories(),
            BarMark::builder()
                .x(x)
                .y(Encoding::scaled_field("amount").scale(Domain::data(
                    DataDomain::builder().fit(IntervalFit::builder().zero(true)),
                ))),
        );
        Chart::builder(Plot::cartesian([bars])).build().unwrap()
    };
    let spanned = bars(Encoding::scaled_field("category").scale(Scale::band(BandScale::builder())));
    let spanned = rect_lefts(&evaluate(&spanned, [320.0, 240.0]));
    let positioned = bars(
        Encoding::scaled_field("category")
            .scale(Scale::band(BandScale::builder()))
            .band_position(0.5),
    );
    let positioned = rect_lefts(&evaluate(&positioned, [320.0, 240.0]));
    for (spanned, positioned) in spanned.iter().zip(&positioned) {
        assert!(
            (spanned - positioned).abs() < 1e-3,
            "{spanned} {positioned}"
        );
    }

    let points = |x: EncodingBuilder| {
        let symbols = CartesianLayer::symbol(categories(), SymbolMark::builder().x(x).y("amount"));
        let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
        symbol_positions(&evaluate(&chart, [320.0, 240.0]))
    };
    let middle =
        points(Encoding::scaled_field("category").scale(Scale::band(BandScale::builder())));
    let start = points(
        Encoding::scaled_field("category")
            .scale(Scale::band(BandScale::builder()))
            .band_position(0.0),
    );
    let end = points(
        Encoding::scaled_field("category")
            .scale(Scale::band(BandScale::builder()))
            .band_position(1.0),
    );
    for ((middle, start), end) in middle.iter().zip(&start).zip(&end) {
        assert!(
            start.0 < middle.0 && middle.0 < end.0,
            "{start:?} {middle:?} {end:?}"
        );
        assert!(((start.0 + end.0) / 2.0 - middle.0).abs() < 1e-3);
    }
}

/// A percentage `size` is a fraction of a band: the thickness of a bar across its band, about the
/// band's middle, and the diameter of a symbol. Another mark warns and draws at the initial size.
#[test]
fn a_percentage_size_is_a_fraction_of_the_band() {
    let styled = |chart: &Chart, css: &str| {
        let mut evaluator =
            ChartEvaluator::new(chart.clone(), cascade_with(css), engine(), Registry::new())
                .unwrap();
        evaluator.evaluate(&host([320.0, 240.0])).unwrap()
    };

    let bars = CartesianLayer::bar(
        categories(),
        BarMark::builder()
            .x("category")
            .y(Encoding::scaled_field("amount").scale(Domain::data(
                DataDomain::builder().fit(IntervalFit::builder().zero(true)),
            ))),
    );
    let bars = Chart::builder(Plot::cartesian([bars])).build().unwrap();
    let full = rect_spans(&styled(&bars, ""));
    let half = rect_spans(&styled(&bars, "mark[type=bar] { size: 50%; }"));
    for ((x, x2), (hx, hx2)) in full.iter().zip(&half) {
        assert!(((hx2 - hx) - (x2 - x) / 2.0).abs() < 1e-3);
        assert!(((hx + hx2) / 2.0 - (x + x2) / 2.0).abs() < 1e-3);
    }

    // Symbols take a point scale of categories, which has no bands, unless they ask for a band
    // scale.
    let symbols = CartesianLayer::symbol(
        categories(),
        SymbolMark::builder()
            .x(Encoding::scaled_field("category").scale(Scale::band(BandScale::builder())))
            .y("amount"),
    );
    let points = Chart::builder(Plot::cartesian([symbols])).build().unwrap();
    let diameter = |css: &str| symbol_positions(&styled(&points, css))[0].2.sqrt();
    let half = diameter("mark[type=symbol] { size: 50%; }");
    let whole = diameter("mark[type=symbol] { size: 100%; }");
    assert!(
        half > 0.0 && (whole - 2.0 * half).abs() < 1e-3,
        "{half} {whole}"
    );

    let lines = CartesianLayer::line(categories(), LineMark::builder().x("category").y("amount"));
    let line = Chart::builder(Plot::cartesian([lines])).build().unwrap();
    let warned = styled(&line, "mark[type=line] { size: 50%; }");
    assert!(warns(&warned, "percentage `size`"), "{:?}", warned.warnings);
}

/// A bar at an outer band level spans its group, and its rows add no leaf to the scale's domain:
/// grouped bars of sales by region and channel, behind which each region's total spans the band of
/// its region, from rows whose channel is null, so the totals differ in width.
#[test]
fn group_totals_span_their_groups_behind_their_members() {
    let sales = RecordBatch::try_from_iter([
        (
            "key",
            region_paths(
                &["East", "East", "East", "West", "West"],
                &[
                    Some("Web"),
                    Some("Store"),
                    Some("Phone"),
                    Some("Web"),
                    Some("Store"),
                ],
            ),
        ),
        ("sales", numbers(&[3.0, 5.0, 2.0, 4.0, 1.0])),
    ])
    .unwrap();
    let totals = RecordBatch::try_from_iter([
        ("key", region_paths(&["East", "West"], &[None, None])),
        ("sales", numbers(&[10.0, 5.0])),
    ])
    .unwrap();

    // A gap of a leaf step between the regions, and a tenth of a step between the channels.
    let key = || {
        Encoding::scaled_field("key").scale(Scale::nested_band(
            NestedBandScale::builder().padding_inner([1.0, 0.1]),
        ))
    };
    let zeroed_sales = || {
        Encoding::scaled_field("sales").scale(Domain::data(
            DataDomain::builder().fit(IntervalFit::builder().zero(true)),
        ))
    };
    let groups = CartesianLayer::bar(
        totals,
        BarMark::builder()
            .x(key().band_level(0))
            .y(zeroed_sales())
            .opacity(0.35),
    );
    let members = CartesianLayer::bar(sales, BarMark::builder().x(key()).y(zeroed_sales()));
    let chart = Chart::builder(Plot::cartesian([groups, members]))
        .build()
        .unwrap();
    render_both(&chart, "grouped-bars-totals");

    let evaluated = evaluate(&chart, [320.0, 240.0]);
    assert!(evaluated.warnings.is_empty(), "{:?}", evaluated.warnings);
    let rects: Vec<(Vec<f32>, Vec<f32>)> = (plot_group(&evaluated).marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Rect(rect) => {
                let len = rect.len as usize;
                let x = rect.x.as_vec(len, None);
                let x2 = rect
                    .x2
                    .as_ref()
                    .map_or_else(Vec::new, |x2| x2.as_vec(len, None));
                Some((x, x2))
            }
            _ => None,
        })
        .collect();
    let [(total_x, total_x2), (member_x, member_x2)] = rects.as_slice() else {
        panic!("two rect marks, not {}", rects.len());
    };
    // East's total spans its three members, and West's its two, so the totals differ in width.
    let spans = |members: std::ops::Range<usize>| {
        let low = member_x[members.clone()]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let high = member_x2[members]
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        (low, high)
    };
    for (index, members) in [(0, 0..3), (1, 3..5)] {
        let (low, high) = spans(members);
        assert!(
            (total_x[index] - low).abs() < 1e-3,
            "{} {low}",
            total_x[index]
        );
        assert!(
            (total_x2[index] - high).abs() < 1e-3,
            "{} {high}",
            total_x2[index]
        );
    }
    assert!(total_x2[0] - total_x[0] > total_x2[1] - total_x[1]);
}

/// A custom scale fits the kind of summary that its implementation states, so a custom scale of
/// categories on `y` runs down its plot, with its first category at the top.
#[test]
fn a_custom_scale_of_categories_runs_down_the_plot() {
    let symbols = CartesianLayer::symbol(
        categories(),
        SymbolMark::builder()
            .x("amount")
            .y(Encoding::scaled_field("category").scale(Scale::custom("dots"))),
    );
    let chart = Chart::builder(Plot::cartesian([symbols])).build().unwrap();

    let registry =
        Registry::new().with_scale("dots", Arc::new(avenger_scales::scales::point::PointScale));
    let mut evaluator = ChartEvaluator::new(chart, cascade(), engine(), registry).unwrap();
    let evaluated = evaluator.evaluate(&host([320.0, 240.0])).unwrap();
    let ys: Vec<f32> = symbol_positions(&evaluated)
        .iter()
        .map(|(_, y, _)| *y)
        .collect();
    assert!(ys.windows(2).all(|pair| pair[0] < pair[1]), "{ys:?}");
}
