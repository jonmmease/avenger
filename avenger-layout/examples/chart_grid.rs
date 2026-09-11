//! Charts at a lower level: avenger-guides + avenger-layout, no
//! avenger-chart.
//!
//! The estimate → solve → final loop that any chart library needs:
//!
//! 1. ESTIMATE — build each plot's axes (and legend) with
//!    `avenger-guides` at a guessed plot size, and measure how far they
//!    overflow the plot rectangle. Those measurements become per-side
//!    [`EdgeDemand`]s: axis material in the `guide` stratum, legend
//!    material in the `legend` stratum.
//! 2. SOLVE — declare the page as a [`Layout`] grid (three plots, one
//!    spanning the full bottom row), attach the measured demands to each
//!    leaf, and solve once at the canvas size. The solver aligns the
//!    plot rectangles into shared tracks and reserves every track edge
//!    for the worst demand along it — the spanning plot's wide tick
//!    labels push the whole left column, the legend pushes the right
//!    edge for the column above it.
//! 3. FINAL — read each plot's solved content rectangle, rebuild the
//!    guides at their exact final sizes, position the legend just past
//!    the solved guide stratum, scatter the data with scales ranged to
//!    the final rectangle, and render the assembled scene graph to a
//!    PNG.
//!
//! Run with: `cargo run --release -p avenger-layout --example chart_grid`

use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_common::types::SymbolShape;
use avenger_common::value::ScalarOrArray;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::numeric::make_numeric_axis_marks;
use avenger_guides::axis::opts::{AxisConfig, AxisOrientation};
use avenger_guides::legend::symbol::{SymbolLegendConfig, make_symbol_legend};
use avenger_layout::{EdgeDemand, Edges, Layout, LayoutSolution, Rect, Side, Size, SolveOptions};
use avenger_scales::scales::ConfiguredScale;
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::symbol::SceneSymbolMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

const CANVAS: Size = Size {
    width: 900.0,
    height: 620.0,
};
/// The size we GUESS each plot will get, before the solve. Guides built
/// at this size give honest overflow measurements; the solver then hands
/// every plot its real rectangle.
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
fn build_axes(spec: &PlotSpec, size: Size, origin: [f32; 2], grid: bool) -> Vec<SceneGroup> {
    let (x_scale, y_scale) = plot_scales(spec, size);
    let dims = [size.width, size.height];
    let x_axis = make_numeric_axis_marks(
        &x_scale,
        spec.x_title,
        origin,
        &AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: dims,
            grid,
            ..Default::default()
        },
    )
    .expect("x axis");
    let y_axis = make_numeric_axis_marks(
        &y_scale,
        spec.y_title,
        origin,
        &AxisConfig {
            orientation: AxisOrientation::Left,
            dimensions: dims,
            grid,
            ..Default::default()
        },
    )
    .expect("y axis");
    vec![x_axis, y_axis]
}

/// The legend for the `conversion` plot. Built against a zero-width
/// chart area so the whole group is self-contained: placing its origin
/// on the plot's right edge puts every item (and the title) just past
/// it, and the group's bbox width is exactly the clearance to demand.
fn build_legend(plot_height: f32) -> SceneGroup {
    make_symbol_legend(&SymbolLegendConfig {
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
    })
    .expect("legend")
}

/// Measure how far a set of guide groups overflows a `size` plot
/// rectangle whose top-left sits at the groups' shared origin.
fn measure_overflow(groups: &[SceneGroup], size: Size) -> Edges<f32> {
    let mut edges: Edges<f32> = Edges::default();
    for group in groups {
        let bbox = group.bounding_box();
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
    let map_x = |v: f32| {
        content.x + (v - spec.x_domain.0) / (spec.x_domain.1 - spec.x_domain.0) * content.width
    };
    let map_y = |v: f32| {
        content.y + content.height
            - (v - spec.y_domain.0) / (spec.y_domain.1 - spec.y_domain.0) * content.height
    };
    let xs: Vec<f32> = spec.points.iter().map(|p| map_x(p.0)).collect();
    let ys: Vec<f32> = spec.points.iter().map(|p| map_y(p.1)).collect();
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

#[tokio::main]
async fn main() {
    // ------------------------------------------------------------------
    // 1. ESTIMATE: guides at guessed sizes -> per-side demands.
    // ------------------------------------------------------------------
    let specs = specs();
    let mut demands: Vec<Edges<f32>> = Vec::new();
    let mut legend_width = 0.0f32;
    for spec in &specs {
        let axes = build_axes(spec, ESTIMATE, [0.0, 0.0], false);
        let mut edges = measure_overflow(&axes, ESTIMATE);
        if spec.legend {
            // The legend group is self-contained (zero-width chart area):
            // its bbox width, outer margin included, is the clearance to
            // ask for in the `legend` stratum.
            let legend = build_legend(ESTIMATE.height);
            legend_width = legend.bounding_box().upper()[0].ceil();
            edges.right += legend_width;
        }
        demands.push(edges);
        println!(
            "estimated {:>10}: guide overflow l={:>3} r={:>3} t={:>2} b={:>2}{}",
            spec.id,
            edges.left,
            edges.right,
            edges.top,
            edges.bottom,
            if spec.legend {
                format!("  (right includes {legend_width}px legend)")
            } else {
                String::new()
            }
        );
    }

    // ------------------------------------------------------------------
    // 2. SOLVE: a 2x2 grid; `latency` spans the full bottom row. Axis
    //    overflow is a `guide` demand; the legend is a `legend` demand
    //    (it stacks beyond any guide material on the same edge).
    // ------------------------------------------------------------------
    let leaf = |index: usize, legend_right: f32| -> Layout<&'static str> {
        let spec = &specs[index];
        let edges = demands[index];
        Layout::leaf(Size::new(ESTIMATE.width, ESTIMATE.height))
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
                    guide: edges.right - legend_right,
                    legend: legend_right,
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
    let page: Layout<&'static str> = Layout::grid(2, 2)
        .cell(0, 0, leaf(0, 0.0))
        .cell(0, 1, leaf(1, legend_width))
        .cell_span(1, 0, 1, 2, leaf(2, 0.0))
        .min_gap(28.0)
        .margin(14.0);
    let solved: LayoutSolution<&'static str> = page
        .solve(&SolveOptions {
            width: Some(CANVAS.width),
            height: Some(CANVAS.height),
        })
        .expect("solve");

    // ------------------------------------------------------------------
    // 3. FINAL: rebuild guides at the solved rectangles and assemble the
    //    scene graph.
    // ------------------------------------------------------------------
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
        for axis in build_axes(spec, size, [content.x, content.y], true) {
            marks.push(axis.into());
        }
        marks.push(build_points(spec, content));
        if spec.legend {
            // The strata law as placement: legend material stacks beyond
            // guide material on the same edge, so the group's origin is
            // the plot's right edge pushed by the granted guide stratum.
            let mut legend = build_legend(size.height);
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
        CanvasConfig::default(),
    )
    .await
    .expect("canvas");
    canvas.set_scene(&scene).expect("set scene");
    let image = canvas.render().await.expect("render");
    let out = std::env::temp_dir().join("avenger_layout_chart_grid.png");
    image.save(&out).expect("save png");
    println!("wrote {}", out.display());
}
