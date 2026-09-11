//! Arrange three plots with measured guides and a spanning bottom row.
//! Guide measurement and layout repeat until the allocated plot sizes settle.
//! The same text engine measures guides and renders the final scene.
//!
//! Run `cargo run --release -p avenger-layout --example chart_grid`.
//! An optional first argument sets the PNG path; the default is
//! `target/layout-gallery/chart-grid.png`.

use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::numeric::make_numeric_axis_marks_with_text_engine;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend_with_text_engine};
use avenger_layout::{EdgeDemand, Edges, Layout, LayoutSolution, Rect, Side, Size, SolveOptions};
use avenger_scales::scales::ConfiguredScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_text::TextEngine;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

const CANVAS: Size = Size {
    width: 900.0,
    height: 620.0,
};
/// Minimum plot size, also used for the first guide measurement.
const ESTIMATE: Size = Size {
    width: 320.0,
    height: 220.0,
};

/// One plot's spec: a title per axis, data domains, and points.
struct PlotSpec {
    id: &'static str,
    x_title: &'static str,
    y_title: &'static str,
    x_domain: (f32, f32),
    y_domain: (f32, f32),
    points: Vec<(f32, f32, usize)>, // (x, y, category)
    legend: bool,
}

const CATEGORY_COLORS: [[f32; 4]; 3] = [
    [0.27, 0.51, 0.71, 1.0], // steel blue
    [0.89, 0.47, 0.20, 1.0], // orange
    [0.33, 0.66, 0.41, 1.0], // green
];
const CATEGORY_NAMES: [&str; 3] = ["alpha", "beta", "gamma"];

fn specs() -> Vec<PlotSpec> {
    vec![
        PlotSpec {
            id: "revenue",
            x_title: "week",
            y_title: "revenue",
            x_domain: (0.0, 12.0),
            y_domain: (0.0, 100.0),
            points: vec![
                (1.0, 22.0, 0),
                (2.0, 31.0, 0),
                (4.0, 38.0, 1),
                (5.5, 45.0, 1),
                (7.0, 52.0, 2),
                (8.5, 61.0, 0),
                (10.0, 70.0, 1),
                (11.0, 84.0, 2),
            ],
            legend: false,
        },
        PlotSpec {
            id: "conversion",
            x_title: "week",
            y_title: "rate",
            x_domain: (0.0, 12.0),
            y_domain: (0.0, 8.0),
            points: vec![
                (1.0, 2.2, 0),
                (2.5, 3.1, 1),
                (4.0, 2.8, 2),
                (6.0, 4.5, 0),
                (7.5, 5.2, 1),
                (9.0, 4.1, 2),
                (10.5, 6.4, 0),
                (11.5, 7.0, 1),
            ],
            // The legend rides this plot's right edge, in the legend
            // stratum beyond any guide material there.
            legend: true,
        },
        PlotSpec {
            id: "latency",
            x_title: "hour",
            y_title: "p99 latency (µs)",
            // Wide y labels (up to 12,000): this spanning plot's guide
            // demand is what pushes the shared left track edge.
            x_domain: (0.0, 24.0),
            y_domain: (0.0, 12_000.0),
            points: vec![
                (1.0, 1_800.0, 0),
                (3.0, 2_400.0, 1),
                (6.0, 4_900.0, 2),
                (9.0, 8_200.0, 0),
                (12.0, 11_400.0, 1),
                (15.0, 9_800.0, 2),
                (18.0, 6_300.0, 0),
                (21.0, 3_500.0, 1),
                (23.0, 2_100.0, 2),
            ],
            legend: false,
        },
    ]
}

/// Linear scales ranged to a plot rectangle of `size` (y inverted, the
/// usual screen convention).
fn plot_scales(spec: &PlotSpec, size: Size) -> (ConfiguredScale, ConfiguredScale) {
    let x = LinearScale::configured(spec.x_domain, (0.0, size.width));
    let y = LinearScale::configured(spec.y_domain, (size.height, 0.0));
    (x, y)
}

/// Build the plot's axes at the given size with a local origin, ready to
/// measure or to place.
fn build_axes(
    spec: &PlotSpec,
    size: Size,
    origin: [f32; 2],
    grid: bool,
    text_engine: &TextEngine,
) -> Vec<SceneGroup> {
    let (x_scale, y_scale) = plot_scales(spec, size);
    let dims = [size.width, size.height];
    let x_axis = make_numeric_axis_marks_with_text_engine(
        &x_scale,
        spec.x_title,
        origin,
        &AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: dims,
            grid,
            ..Default::default()
        },
        text_engine,
    )
    .expect("x axis");
    let y_axis = make_numeric_axis_marks_with_text_engine(
        &y_scale,
        spec.y_title,
        origin,
        &AxisConfig {
            orientation: AxisOrientation::Left,
            dimensions: dims,
            grid,
            ..Default::default()
        },
        text_engine,
    )
    .expect("y axis");
    vec![x_axis, y_axis]
}

/// The legend for the `conversion` plot. Built against a zero-width
/// chart area so the whole group is self-contained: placing its origin
/// on the plot's right edge puts every item (and the title) just past
/// it, and the group's bbox width is exactly the clearance to demand.
fn build_legend(plot_height: f32, text_engine: &TextEngine) -> SceneGroup {
    make_symbol_legend_with_text_engine(
        &SymbolLegendConfig {
            title: Some("cohort".to_string()),
            text: ScalarOrArray::new_array(CATEGORY_NAMES.iter().map(|s| s.to_string()).collect()),
            fill: ScalarOrArray::new_array(
                CATEGORY_COLORS
                    .iter()
                    .map(|c| ColorOrGradient::Color(*c))
                    .collect(),
            ),
            inner_width: 0.0,
            inner_height: plot_height,
            outer_margin: 8.0,
            ..Default::default()
        },
        text_engine,
    )
    .expect("legend")
}

/// Measure how far a set of guide groups overflows a `size` plot
/// rectangle whose top-left sits at the groups' shared origin.
fn measure_overflow(groups: &[SceneGroup], size: Size, text_engine: &TextEngine) -> Edges<f32> {
    let mut edges: Edges<f32> = Edges::default();
    for group in groups {
        let bbox = group.bounding_box_with_text_engine(text_engine);
        edges.left = edges.left.max(-bbox.lower()[0]);
        edges.top = edges.top.max(-bbox.lower()[1]);
        edges.right = edges.right.max(bbox.upper()[0] - size.width);
        edges.bottom = edges.bottom.max(bbox.upper()[1] - size.height);
    }
    Edges {
        left: edges.left.max(0.0).ceil(),
        right: edges.right.max(0.0).ceil(),
        top: edges.top.max(0.0).ceil(),
        bottom: edges.bottom.max(0.0).ceil(),
    }
}

/// The scatter for one plot, positioned inside its solved rectangle.
fn build_points(spec: &PlotSpec, content: Rect) -> SceneMark {
    let (x_scale, y_scale) = plot_scales(spec, Size::new(content.width, content.height));
    let xs: Vec<f32> = spec
        .points
        .iter()
        .map(|p| {
            content.x
                + x_scale
                    .scale_scalar(&p.0)
                    .expect("x coordinate")
                    .as_f32()
                    .expect("numeric x")
        })
        .collect();
    let ys: Vec<f32> = spec
        .points
        .iter()
        .map(|p| {
            content.y
                + y_scale
                    .scale_scalar(&p.1)
                    .expect("y coordinate")
                    .as_f32()
                    .expect("numeric y")
        })
        .collect();
    let fills: Vec<ColorOrGradient> = spec
        .points
        .iter()
        .map(|p| ColorOrGradient::Color(CATEGORY_COLORS[p.2]))
        .collect();
    SceneMark::Symbol(SceneSymbolMark {
        len: spec.points.len() as u32,
        x: ScalarOrArray::new_array(xs),
        y: ScalarOrArray::new_array(ys),
        fill: ScalarOrArray::new_array(fills),
        size: 48.0.into(),
        stroke_width: Some(0.0),
        shapes: vec![SymbolShape::Circle],
        shape_index: 0.into(),
        ..Default::default()
    })
}

/// Measure at the current allocations. The leaf sizes stay at the plot
/// minimum so an earlier, larger allocation does not prevent later shrinking.
fn solve_page(
    specs: &[PlotSpec],
    sizes: &[Size],
    text_engine: &TextEngine,
) -> LayoutSolution<&'static str> {
    let leaf = |index: usize| -> Layout<&'static str> {
        let spec = &specs[index];
        let size = sizes[index];
        let edges = measure_overflow(
            &build_axes(spec, size, [0.0, 0.0], false, text_engine),
            size,
            text_engine,
        );
        let legend_width = if spec.legend {
            build_legend(size.height, text_engine)
                .bounding_box_with_text_engine(text_engine)
                .upper()[0]
                .ceil()
        } else {
            0.0
        };
        Layout::leaf(ESTIMATE)
            .id(spec.id)
            .demand(
                Side::Left,
                EdgeDemand {
                    guide: edges.left,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Right,
                EdgeDemand {
                    guide: edges.right,
                    legend: legend_width,
                },
            )
            .demand(
                Side::Top,
                EdgeDemand {
                    guide: edges.top,
                    legend: 0.0,
                },
            )
            .demand(
                Side::Bottom,
                EdgeDemand {
                    guide: edges.bottom,
                    legend: 0.0,
                },
            )
    };
    Layout::grid(2, 2)
        .cell(0, 0, leaf(0))
        .cell(0, 1, leaf(1))
        .cell_span(1, 0, 1, 2, leaf(2))
        .min_gap(28.0)
        .margin(14.0)
        .solve(&SolveOptions {
            width: Some(CANVAS.width),
            height: Some(CANVAS.height),
        })
        .expect("solve")
}

fn settle_layout(specs: &[PlotSpec], text_engine: &TextEngine) -> LayoutSolution<&'static str> {
    let mut sizes = vec![ESTIMATE; specs.len()];
    let mut previous: Option<LayoutSolution<&'static str>> = None;
    for iteration in 1..=8 {
        let solved = solve_page(specs, &sizes, text_engine);
        if previous
            .as_ref()
            .is_some_and(|prev| solved.content_delta(prev) < 0.01)
        {
            println!("Guide measurement settled after {iteration} solves");
            return solved;
        }
        sizes = specs
            .iter()
            .map(|spec| {
                let slot = solved.region(&spec.id).expect("plot region").slot;
                Size::new(slot.width, slot.height)
            })
            .collect();
        previous = Some(solved);
    }
    panic!("guide measurement did not settle after eight solves");
}

#[tokio::main]
async fn main() {
    let specs = specs();
    let text_engine = TextEngine::with_default_config().expect("text engine");
    let solved = settle_layout(&specs, &text_engine);
    let mut marks: Vec<SceneMark> = vec![SceneMark::Rect(SceneRectMark {
        len: 1,
        x: 0.0.into(),
        y: 0.0.into(),
        width: Some(CANVAS.width.into()),
        height: Some(CANVAS.height.into()),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
        ..Default::default()
    })];

    for spec in &specs {
        let region = solved.region(&spec.id).expect("solved region");
        // ADOPT the allocation: the leaf's solved slot IS the final plot
        // rectangle. Edge demands never occupy the slot — interior edges
        // are absorbed by the inter-track gaps (the gap law), and the
        // grid's first/last edges are hoisted to its envelope — so the
        // guides drawn around the slot land in space the solve already
        // reserved. Wherever the canvas had free space the slot is wider
        // than the estimate, and the spanning plot's slot covers both
        // columns plus the gap.
        let granted = &region.granted;
        let content = region.slot;
        println!(
            "adopted   {:>10}: plot rect {:>5.1} x {:>5.1} at ({:>5.1}, {:>5.1}); granted left guide {:>4.1}, right legend {:>4.1}",
            spec.id,
            content.width,
            content.height,
            content.x,
            content.y,
            granted.left.guide,
            granted.right.legend,
        );

        let size = Size::new(content.width, content.height);
        for axis in build_axes(spec, size, [content.x, content.y], true, &text_engine) {
            marks.push(axis.into());
        }
        marks.push(build_points(spec, content));
        if spec.legend {
            // The strata law as placement: legend material stacks beyond
            // guide material on the same edge, so the group's origin is
            // the plot's right edge pushed by the granted guide stratum.
            let mut legend = build_legend(size.height, &text_engine);
            legend.origin = [content.x + content.width + granted.right.guide, content.y];
            marks.push(legend.into());
        }
    }

    // The renderer expects a root group: every mark above becomes a
    // child of one canvas-sized group.
    let root = SceneGroup {
        origin: [0.0, 0.0],
        marks,
        ..Default::default()
    };
    let scene = SceneGraph {
        marks: vec![root.into()],
        width: CANVAS.width,
        height: CANVAS.height,
        origin: [0.0, 0.0],
    };

    // ------------------------------------------------------------------
    // 4. Render to PNG.
    // ------------------------------------------------------------------
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig {
            text_engine: Some(text_engine),
            ..Default::default()
        },
    )
    .await
    .expect("canvas");
    canvas.set_scene(&scene).expect("set scene");
    let image = canvas.render().await.expect("render");
    let out = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("target/layout-gallery/chart-grid.png"));
    if let Some(parent) = out.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).expect("create output directory");
    }
    image.save(&out).expect("save png");
    println!("wrote {}", out.display());
}

#[test]
fn measured_guides_fit_final_allocations() {
    let specs = specs();
    let engine = TextEngine::with_default_config().expect("text engine");
    let solved = settle_layout(&specs, &engine);
    for spec in &specs {
        let region = solved.region(&spec.id).unwrap();
        let size = Size::new(region.slot.width, region.slot.height);
        let measured = measure_overflow(
            &build_axes(spec, size, [0.0, 0.0], false, &engine),
            size,
            &engine,
        );
        for side in [Side::Top, Side::Right, Side::Bottom, Side::Left] {
            assert!(
                *measured.side(side) <= region.granted.side(side).guide + 0.01,
                "{} {:?}: {:?} exceeds {:?}",
                spec.id,
                side,
                measured,
                region.granted
            );
        }
        if spec.legend {
            let bounds = build_legend(size.height, &engine).bounding_box_with_text_engine(&engine);
            assert!(bounds.upper()[0] <= region.granted.right.legend + 0.01);
            assert!(bounds.upper()[1] <= size.height);
        }
    }
    let left = solved.region(&"revenue").unwrap().slot;
    let right = solved.region(&"conversion").unwrap().slot;
    let bottom = solved.region(&"latency").unwrap().slot;
    assert_eq!(left.x, bottom.x);
    assert!((right.x + right.width - bottom.x - bottom.width).abs() < 0.01);
}
