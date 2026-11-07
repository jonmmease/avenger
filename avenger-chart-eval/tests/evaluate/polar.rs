//! Polar plots: pies and donuts, rose and radial bar charts, radar charts, sunbursts with curved
//! texts, half circles, the fit of the outer radius to labels outside the circle, plots that share
//! a scale, and the coordinates of polar plots.

use avenger_chart::prelude::*;
use avenger_chart_eval::EvaluatedChart;
use avenger_scenegraph::marks::{arc::SceneArcMark, mark::SceneMark};

use crate::support::*;

/// The arc marks of the scene, in the order that they draw.
fn arcs(evaluated: &EvaluatedChart) -> Vec<&SceneArcMark> {
    fn walk<'a>(marks: &'a [SceneMark], out: &mut Vec<&'a SceneArcMark>) {
        for mark in marks {
            match mark {
                SceneMark::Arc(arc) => out.push(arc),
                SceneMark::Group(group) => walk(&group.marks, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&evaluated.scene.marks, &mut out);
    out
}

/// The largest outer radius of the arcs of `arc`.
fn outer_radius(arc: &SceneArcMark) -> f32 {
    (arc.outer_radius.as_vec(arc.len as usize, None).into_iter()).fold(0.0, f32::max)
}

#[test]
fn a_pie_spans_the_radius_of_its_plot() {
    let wedges = PolarLayer::rect(
        PolarRectMark::builder()
            .theta(Encoding::scaled_column("start"))
            .theta2(Encoding::scaled_column("end"))
            .color(Encoding::scaled_column("category")),
    )
    .table(slices());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(PolarPlot::builder().layers([wedges])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-pie");

    // The pie fills the shorter side of its plot.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let plot = evaluated.plots[0].rect;
    let radius = outer_radius(arcs(&evaluated)[0]);
    assert!(
        (radius - plot[2].min(plot[3]) / 2.0).abs() < 1.0,
        "{radius} {plot:?}"
    );
}

#[test]
fn labels_outside_a_donut_shrink_its_circle() {
    let donut = |labels: bool| {
        let wedges = PolarLayer::rect(
            PolarRectMark::builder()
                .inset(1.0)
                .theta(Encoding::scaled_column("start"))
                .theta2(Encoding::scaled_column("end"))
                .color(Encoding::scaled_column("category")),
        )
        .table(slices());
        let texts = PolarLayer::text(
            PolarTextMark::builder()
                .theta(Encoding::scaled_column("middle"))
                .radius(Encoding::constant(1.0))
                .text(Encoding::column("category")),
        )
        .table(slices());
        let layers = match labels {
            true => vec![wedges, texts],
            false => vec![wedges],
        };
        Chart::builder()
            .number_format_syntax("d3")
            .datetime_format_syntax("d3")
            .root(Plot::polar(
                PolarPlot::builder()
                    .options(Polar::builder().inner_radius(0.5))
                    .layers(layers),
            ))
            .infer_scales()
            .build()
            .unwrap()
    };
    render_both(&donut(true), "polar-donut-labels");

    let plain = evaluate(&donut(false), [320.0, 240.0]);
    let labeled = evaluate(&donut(true), [320.0, 240.0]);
    let [plain, labeled] = [&plain, &labeled].map(|evaluated| outer_radius(arcs(evaluated)[0]));
    assert!(labeled < plain - 5.0, "{labeled} {plain}");
}

#[test]
fn a_rose_chart_draws_its_theta_axis_around_the_rim() {
    let bars = PolarLayer::bar(
        PolarBarMark::builder()
            .theta(Encoding::scaled_column("category").scale(
                Scale::band(BandScale::builder().padding_inner(0.1)).guide(PolarAxis::builder()),
            ))
            .radius(
                Encoding::scaled_column("amount").scale(
                    Scale::builder()
                        .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true))),
                ),
            )
            .color(Encoding::scaled_column("category")),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(PolarPlot::builder().layers([bars])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-rose");

    // Each category labels its spoke once, and the first wedge centers on the start angle.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    for category in ["A", "B", "C", "D", "E", "F"] {
        let count = (scene_texts(&evaluated.scene.marks).iter())
            .filter(|text| *text == category)
            .count();
        assert_eq!(count, 2, "{category}: one label and one legend entry");
    }
    let wedges = arcs(&evaluated)[0];
    let [start, end] =
        [&wedges.start_angle, &wedges.end_angle].map(|angles| angles.as_vec(6, None));
    assert!(
        ((start[0] + end[0]) / 2.0).abs() < 1e-3,
        "{} {}",
        start[0],
        end[0]
    );
}

#[test]
fn a_radial_bar_chart_runs_its_bars_around_part_of_the_circle() {
    let bars = PolarLayer::bar(
        PolarBarMark::builder()
            .orient(Orient::Horizontal)
            .radius(Encoding::scaled_column("category").scale(
                Scale::band(BandScale::builder().padding_inner(0.2)).guide(PolarAxis::builder()),
            ))
            .theta(
                Encoding::scaled_column("amount").scale(
                    Scale::builder()
                        .domain(DataDomain::builder().fit(IntervalFit::builder().zero(true)))
                        .guide(PolarAxis::builder()),
                ),
            ),
    )
    .table(categories());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(
            PolarPlot::builder()
                .options(Polar::builder().end_angle(270.0).inner_radius(0.2))
                .layers([bars]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-radial-bars");

    // Each bar starts at the start angle.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let bars = arcs(&evaluated)[0];
    for angle in bars.start_angle.as_vec(6, None) {
        assert!(angle.abs() < 1e-3, "{angle}");
    }
}

#[test]
fn a_radar_chart_draws_straight_edges_and_a_web_of_polygons() {
    let theta = || {
        Encoding::scaled_column("skill").scale(
            Scale::point(PointScale::builder()).guide(PolarAxis::builder().route(Route::Canvas)),
        )
    };
    let radius = || {
        Encoding::scaled_column("value").scale(
            Scale::builder().domain(Domain::explicit([0.0, 5.0])).guide(
                PolarAxis::builder()
                    .route(Route::Canvas)
                    .ticks(GuideTicks::builder().count(5.0)),
            ),
        )
    };
    let areas = PolarLayer::area(
        PolarAreaMark::builder()
            .route(Route::Canvas)
            .theta(theta())
            .radius(radius())
            .color(Encoding::scaled_column("unit"))
            .fill_opacity(Encoding::constant(0.25)),
    )
    .table(skills());
    let lines = PolarLayer::line(
        PolarLineMark::builder()
            .route(Route::Canvas)
            .theta(theta())
            .radius(radius())
            .color(Encoding::scaled_column("unit")),
    )
    .table(skills());
    let points = PolarLayer::symbol(
        PolarSymbolMark::builder()
            .theta(theta())
            .radius(radius())
            .color(Encoding::scaled_column("unit")),
    )
    .table(skills());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(
            PolarPlot::builder().layers([areas, lines, points]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-radar");
}

#[test]
fn a_sunburst_curves_the_names_of_its_rings() {
    let nodes = PolarLayer::rect(
        PolarRectMark::builder()
            .inset(0.5)
            .theta(Encoding::scaled_column("start"))
            .theta2(Encoding::scaled_column("end"))
            .radius(Encoding::column("inner"))
            .radius2(Encoding::column("outer"))
            .color(Encoding::scaled_column("parent")),
    )
    .table(hierarchy());
    let names = PolarLayer::text(
        PolarTextMark::builder()
            .orient(PolarTextOrient::Tangential)
            .radius_side(RadiusSide::Middle)
            .theta(Encoding::scaled_column("middle"))
            .radius(Encoding::column("ring"))
            .text(Encoding::column("name"))
            .size(Encoding::constant(10.0)),
    )
    .table(hierarchy());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(PolarPlot::builder().layers([nodes, names])))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-sunburst");

    // The names follow paths along their rings.
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let curved = (evaluated.scene.marks.iter())
        .filter_map(|mark| match mark {
            SceneMark::Group(group) => Some(&group.marks),
            _ => None,
        })
        .flatten()
        .any(|mark| matches!(mark, SceneMark::Text(text) if text.text_path.is_some()));
    assert!(curved);
}

#[test]
fn a_half_circle_keeps_the_shape_of_its_box() {
    let band = PolarLayer::rect(
        PolarRectMark::builder()
            .theta(Encoding::scaled_column("start"))
            .theta2(Encoding::scaled_column("end"))
            .color(Encoding::scaled_column("category")),
    )
    .table(slices());
    let needle = PolarLayer::rule(
        PolarRuleMark::builder()
            .theta(Encoding::scaled_constant(0.62))
            .radius(Encoding::constant(0.0))
            .radius2(Encoding::constant(1.0))
            .stroke_width(Encoding::constant(3.0)),
    )
    .table(slices().slice(0, 1));
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(
            PolarPlot::builder()
                .options(
                    Polar::builder()
                        .start_angle(-90.0)
                        .end_angle(90.0)
                        .inner_radius(0.6),
                )
                .layers([band, needle]),
        ))
        .infer_scales()
        .build()
        .unwrap();
    render_both(&chart, "polar-half");

    // The plot is twice as wide as it is tall.
    let evaluated = evaluate(&chart, [640.0, 360.0]);
    let [_, _, width, height] = evaluated.plots[0].rect;
    assert!((width / height - 2.0).abs() < 0.01, "{width} {height}");
}

#[test]
fn pies_that_share_theta_take_one_outer_radius() {
    let pie = |labels: bool| {
        let wedges = PolarLayer::rect(
            PolarRectMark::builder()
                .theta(Encoding::scaled_column("start"))
                .theta2(Encoding::scaled_column("end"))
                .color(Encoding::scaled_column("category")),
        )
        .table(slices());
        let texts = PolarLayer::text(
            PolarTextMark::builder()
                .theta(Encoding::scaled_column("middle"))
                .radius(Encoding::constant(1.0))
                .text(Encoding::column("category")),
        )
        .table(slices());
        let layers = match labels {
            true => vec![wedges, texts],
            false => vec![wedges],
        };
        Plot::polar(PolarPlot::builder().layers(layers))
    };
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(
            Concat::builder()
                .direction(Direction::Row)
                .resolve([Resolve::channel(Channel::Theta).sharing(Sharing::Shared)])
                .children([pie(true), pie(false)]),
        )
        .infer_scales()
        .build()
        .unwrap();
    render(&chart, [640.0, 300.0], "polar-shared-pies");

    let evaluated = evaluate(&chart, [640.0, 300.0]);
    let arcs = arcs(&evaluated);
    assert_eq!(arcs.len(), 2);
    assert!((outer_radius(arcs[0]) - outer_radius(arcs[1])).abs() < 1e-3);
}

#[test]
fn the_coordinates_of_a_polar_plot_map_angles_and_radii_to_pixels() {
    let points = PolarLayer::symbol(
        PolarSymbolMark::builder()
            .theta(Encoding::scaled_column("middle"))
            .radius(Encoding::scaled_column("amount")),
    )
    .table(slices());
    let chart = Chart::builder()
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .root(Plot::polar(PolarPlot::builder().layers([points])))
        .infer_scales()
        .build()
        .unwrap();
    let evaluated = evaluate(&chart, [320.0, 240.0]);
    let coords = &evaluated.plots[0].coords;
    let table = slices();
    let [x, y] = coords
        .pixels([
            table.column_by_name("middle").unwrap(),
            table.column_by_name("amount").unwrap(),
        ])
        .unwrap();
    let symbols = symbol_positions(&evaluated);
    for (row, (symbol_x, symbol_y, _)) in symbols.iter().enumerate() {
        assert!((x.value(row) - symbol_x).abs() < 1e-3, "{row}");
        assert!((y.value(row) - symbol_y).abs() < 1e-3, "{row}");
    }
}
