//! Prepared colorbars and legends in a layout, drawn with the diagnostic overlay and compared
//! with baselines.
//!
//! A changed image is written to `tests/output/`; accept it by copying it to `tests/baselines/`.

use std::num::NonZeroUsize;
use std::sync::Arc;

use avenger_color::ColorOrGradient;
use avenger_common::value::ScalarOrArray;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{Axis, AxisConfig, AxisPart};
use avenger_guides::legend::{
    Colorbar, ColorbarConfig, Legend, LegendConfig, LegendLayout, LegendSwatches, LegendWrap,
    SymbolSwatches,
};
use avenger_guides::{GuideSide, PartExtent};
use avenger_layout::diagnostics::diagnostic_layer;
use avenger_layout::{
    Column, Direction, Edges, Gap, Layout, Leaf, Measure, Measurement, Placement, Position, Rect,
    RegionKind, Side, Size, Slab, SlabStack, Solution, SolveOptions, Track, Warning, Wrap,
};
use avenger_render_fixtures::raster::{assert_baseline, svg_to_png};
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark, rect::SceneRectMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::SvgRenderer;
use avenger_typst_label::LabelEngine;

type Id = &'static str;

fn engine() -> LabelEngine {
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
}

/// A guide that a slab holds.
enum Guide<'a> {
    Axis(&'a Axis, AxisPart),
    Colorbar(&'a Colorbar),
    Legend(&'a Legend),
}

/// The guides of a page by their slabs' ids, each on a side.
struct Guides<'a> {
    slabs: Vec<(Id, GuideSide, Guide<'a>)>,
}

impl Guides<'_> {
    fn extent(&self, id: &Id, vertical: bool, length: f32) -> Option<PartExtent> {
        let (_, side, guide) = self.slabs.iter().find(|(slab, ..)| slab == id)?;
        (side.is_vertical() == vertical).then(|| match guide {
            Guide::Axis(axis, part) => axis.extent(*part, length),
            Guide::Colorbar(colorbar) => colorbar.extent(length),
            Guide::Legend(legend) => legend.extent(length),
        })
    }

    /// Each guide that its slab draws, at its content's length and in its content.
    fn draw(&self, solution: &Solution<Id>) -> Vec<SceneMark> {
        let mut marks = Vec::new();
        for (id, side, guide) in &self.slabs {
            let Some(region) = solution.region(id) else {
                continue;
            };
            if !matches!(
                region.kind,
                RegionKind::Slab {
                    represented_by: None,
                    ..
                }
            ) {
                continue;
            }
            let rect = region.content;
            let slot = [rect.x, rect.y, rect.width, rect.height];
            let length = if side.is_vertical() {
                rect.height
            } else {
                rect.width
            };
            let group = match guide {
                Guide::Axis(axis, part) => {
                    let Some(group) = axis.part(*part, length) else {
                        continue;
                    };
                    SceneGroup {
                        origin: axis.origin(slot),
                        ..group
                    }
                }
                Guide::Colorbar(colorbar) => SceneGroup {
                    origin: colorbar.origin(slot),
                    ..colorbar.marks(length).group
                },
                Guide::Legend(legend) => SceneGroup {
                    origin: legend.origin(slot),
                    ..legend.marks(length).group
                },
            };
            marks.push(group.into());
        }
        marks
    }
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

impl Measure<Id> for Guides<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        self.extent(id, false, width).map(answer)
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        self.extent(id, true, height).map(answer)
    }

    /// A legend's length with its entries on as few lines as its wrap allows, along its side.
    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        let (_, side, guide) = self.slabs.iter().find(|(slab, ..)| slab == id)?;
        let along = if side.is_vertical() {
            Direction::Vertical
        } else {
            Direction::Horizontal
        };
        match guide {
            Guide::Legend(legend) if direction == along => Some(legend.max_content()),
            _ => None,
        }
    }
}

/// A plot's area as a light rect.
fn plot(solution: &Solution<Id>, id: Id) -> SceneMark {
    let rect = solution.region(&id).expect("a plot").content;
    SceneMark::Rect(SceneRectMark {
        x: rect.x.into(),
        y: rect.y.into(),
        width: Some(rect.width.into()),
        height: Some(rect.height.into()),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.93, 0.95, 0.98, 1.0])),
        ..Default::default()
    })
}

/// The page's plots and guides with the diagnostic overlay above them, at `origin`.
fn page(
    solution: &Solution<Id>,
    plots: &[Id],
    guides: &Guides,
    origin: [f32; 2],
    engine: &LabelEngine,
) -> SceneMark {
    let mut marks: Vec<SceneMark> = plots.iter().map(|&id| plot(solution, id)).collect();
    marks.extend(guides.draw(solution));
    let layer = diagnostic_layer(solution, &|id: &Id| id.to_string(), engine).unwrap();
    marks.push(layer.into());
    SceneGroup {
        origin,
        marks,
        ..Default::default()
    }
    .into()
}

fn render(name: &str, size: [f32; 2], pages: Vec<SceneMark>, engine: &LabelEngine) {
    let mut marks = vec![SceneMark::Rect(SceneRectMark {
        width: Some(size[0].into()),
        height: Some(size[1].into()),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
        ..Default::default()
    })];
    marks.extend(pages);
    let scene = SceneGraph {
        marks,
        width: size[0],
        height: size[1],
        origin: [0.0, 0.0],
    };
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, engine)
        .unwrap();
    assert_baseline(env!("CARGO_MANIFEST_DIR"), &svg_to_png(&svg, 2.0), name);
}

fn colorbar(domain: (f64, f64), title: &str, engine: &LabelEngine) -> Colorbar {
    let defaults = ColorbarConfig::default();
    Colorbar::new(
        &LinearScale::configured_color(domain, vec!["white", "steelblue"]),
        title,
        &ColorbarConfig {
            axis: AxisConfig {
                tick_format: ",f".into(),
                ..defaults.axis
            },
            ..defaults
        },
        engine,
    )
    .unwrap()
}

fn padded(width: f32, height: Option<f32>) -> SolveOptions {
    SolveOptions {
        width: Some(width),
        height,
        padding: Edges::all(10.0),
        explain: true,
        ..SolveOptions::default()
    }
}

/// A colorbar beside a tall plot stops at 200 pixels, level with the plot's top, and the
/// layout asks it at that length.
#[test]
fn colorbar_beside_a_plot() {
    let engine = engine();
    let colorbar = colorbar((0.0, 100.0), "temperature", &engine);
    let layout: Layout<Id> = Leaf::new()
        .id("plot")
        .min_size(Size::new(240.0, 300.0))
        .slab(
            Side::Right,
            Slab::new()
                .id("colorbar")
                .max_length(200.0)
                .place(Placement::stretch(Position::Start)),
        )
        .into();
    let mut guides = Guides {
        slabs: vec![("colorbar", GuideSide::Right, Guide::Colorbar(&colorbar))],
    };
    let measured = layout
        .measure(&padded(360.0, Some(320.0)), &mut guides)
        .unwrap();
    let solution = measured.solution();
    let slab = solution.region(&"colorbar").unwrap();
    assert_eq!(slab.content.height, 200.0);
    assert_eq!(slab.content.y, solution.region(&"plot").unwrap().content.y);
    let root = solution.at_path(&[]).unwrap().slot;
    assert!(root.bottom() <= 320.0);
    let size = solution.size();
    let pages = vec![page(solution, &["plot"], &guides, [0.0, 0.0], &engine)];
    render(
        "colorbar_beside_a_plot",
        [size.width, size.height],
        pages,
        &engine,
    );
}

fn legend(layout: LegendLayout, count: usize, engine: &LabelEngine) -> Legend {
    titled_legend("cohort", layout, count, engine)
}

fn titled_legend(title: &str, layout: LegendLayout, count: usize, engine: &LabelEngine) -> Legend {
    let names = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa",
    ];
    let palette = [
        [0.27, 0.51, 0.71, 1.0],
        [0.89, 0.47, 0.20, 1.0],
        [0.33, 0.66, 0.41, 1.0],
        [0.78, 0.33, 0.52, 1.0],
        [0.55, 0.45, 0.75, 1.0],
    ];
    Legend::new(
        &LegendConfig {
            title: Some(title.to_string()),
            text: ScalarOrArray::new_array(names[..count].iter().map(|s| s.to_string()).collect()),
            swatches: LegendSwatches::Symbols(SymbolSwatches {
                fill: ScalarOrArray::new_array(
                    (0..count)
                        .map(|i| ColorOrGradient::Color(palette[i % palette.len()]))
                        .collect(),
                ),
                ..Default::default()
            }),
            layout,
            ..Default::default()
        },
        engine,
    )
    .unwrap()
}

/// A legend below the plot breaks its entries into more lines as the canvas narrows, and the
/// layout reflows it when the width leaves the range that its answer held for.
#[test]
fn a_bottom_legend_reflows_as_the_canvas_narrows() {
    let engine = engine();
    let legend = legend(
        LegendLayout {
            side: GuideSide::Bottom,
            wrap: LegendWrap::Flow,
        },
        10,
        &engine,
    );
    let layout: Layout<Id> = Leaf::new()
        .id("plot")
        .min_size(Size::new(160.0, 120.0))
        .slab(Side::Bottom, Slab::new().id("legend"))
        .into();
    let mut guides = Guides {
        slabs: vec![("legend", GuideSide::Bottom, Guide::Legend(&legend))],
    };
    let mut measured = layout.measure(&padded(560.0, None), &mut guides).unwrap();
    let wide = measured.solution().clone();
    let narrow = measured
        .remeasure(Size::new(300.0, 400.0), &mut guides)
        .unwrap()
        .clone();
    let height = |solution: &Solution<Id>| solution.region(&"legend").unwrap().slot.height;
    assert!(height(&narrow) > height(&wide));
    let size = [560.0, wide.size().height + narrow.size().height + 20.0];
    let pages = vec![
        page(&wide, &["plot"], &guides, [0.0, 0.0], &engine),
        page(
            &narrow,
            &["plot"],
            &guides,
            [0.0, wide.size().height + 20.0],
            &engine,
        ),
    ];
    render(
        "a_bottom_legend_reflows_as_the_canvas_narrows",
        size,
        pages,
        &engine,
    );
}

/// A side legend with a fixed column count keeps its columns beside a short plot, and its
/// extra entries drop below the plot, in room that the layout reserves for them.
#[test]
fn a_side_legend_with_fixed_columns_drops_its_extra_entries_below() {
    let engine = engine();
    let legend = legend(
        LegendLayout {
            side: GuideSide::Right,
            wrap: LegendWrap::Grid {
                columns: NonZeroUsize::new(2),
            },
        },
        10,
        &engine,
    );
    let layout: Layout<Id> = Leaf::new()
        .id("plot")
        .min_size(Size::new(220.0, 60.0))
        .slab(Side::Right, Slab::new().id("legend"))
        .into();
    let mut guides = Guides {
        slabs: vec![("legend", GuideSide::Right, Guide::Legend(&legend))],
    };
    let measured = layout.measure(&padded(420.0, None), &mut guides).unwrap();
    let solution = measured.solution();
    let plot_rect: Rect = solution.region(&"plot").unwrap().content;
    let legend_slot = solution.region(&"legend").unwrap().content;
    let extent = legend.extent(legend_slot.height);
    assert!(
        extent.overhang[1] > 0.0,
        "the entries pass the plot's bottom"
    );
    assert!(
        !solution
            .warnings()
            .iter()
            .any(|warning| matches!(warning, Warning::Overlap { .. })),
        "{:?}",
        solution.warnings()
    );
    assert!(solution.size().height >= plot_rect.bottom() + extent.overhang[1]);
    let size = solution.size();
    let pages = vec![page(solution, &["plot"], &guides, [0.0, 0.0], &engine)];
    render(
        "a_side_legend_with_fixed_columns_drops_its_extra_entries_below",
        [size.width, size.height],
        pages,
        &engine,
    );
}

/// A plot's right side: a gap from the plot, then a stack of a legend at its max-content
/// length above a colorbar that fills the rest.
fn legend_stack(plot: Leaf<Id>, legend: Id, colorbar: Id) -> Leaf<Id> {
    plot.slab(Side::Right, Slab::new().min_width(12.0))
        .slab_stack(
            Side::Right,
            SlabStack::new([
                Slab::new().id(legend).tag("legend"),
                Slab::new().id(colorbar).tag("colorbar"),
            ])
            .track(0, Track::auto())
            .gap(Gap::new(12.0)),
        )
}

/// A legend stacked above a colorbar beside a plot. The legend takes its max-content length,
/// and the colorbar takes the rest of the plot's height, down to the plot's bottom edge, below
/// which the x axis lies.
#[test]
fn a_colorbar_below_a_legend_fills_the_rest_of_the_plot_height() {
    let engine = engine();
    let legend = legend(LegendLayout::default(), 4, &engine);
    let colorbar = colorbar((0.0, 100.0), "temperature", &engine);
    let x_axis = Axis::continuous(
        LinearScale::configured((0.0, 10.0), (0.0, 1.0)),
        "time",
        &AxisConfig {
            side: GuideSide::Bottom,
            tick_format: ",f".into(),
            tick_label: None,
            style: Default::default(),
        },
        &engine,
    )
    .unwrap();
    let plot = Leaf::new()
        .id("plot")
        .min_size(Size::new(240.0, 120.0))
        .slab(Side::Bottom, Slab::new().id("x ticks"))
        .slab(Side::Bottom, Slab::new().id("x labels"))
        .slab(Side::Bottom, Slab::new().id("x title"));
    let layout: Layout<Id> = legend_stack(plot, "legend", "colorbar").into();
    let mut guides = Guides {
        slabs: vec![
            (
                "x ticks",
                GuideSide::Bottom,
                Guide::Axis(&x_axis, AxisPart::Ticks),
            ),
            (
                "x labels",
                GuideSide::Bottom,
                Guide::Axis(&x_axis, AxisPart::Labels),
            ),
            (
                "x title",
                GuideSide::Bottom,
                Guide::Axis(&x_axis, AxisPart::Title),
            ),
            ("legend", GuideSide::Right, Guide::Legend(&legend)),
            ("colorbar", GuideSide::Right, Guide::Colorbar(&colorbar)),
        ],
    };
    let measured = layout
        .measure(&padded(440.0, Some(320.0)), &mut guides)
        .unwrap();
    let solution = measured.solution();
    let plot_rect = solution.region(&"plot").unwrap().content;
    let legend_rect = solution.region(&"legend").unwrap().content;
    let colorbar_rect = solution.region(&"colorbar").unwrap().content;
    assert_eq!(legend_rect.y, plot_rect.y);
    assert_eq!(legend_rect.height, legend.max_content());
    assert_eq!(legend.extent(legend_rect.height).overhang, [0.0, 0.0]);
    assert_eq!(colorbar_rect.x, legend_rect.x);
    assert_eq!(colorbar_rect.bottom(), plot_rect.bottom());
    assert_eq!(colorbar_rect.y, legend_rect.bottom() + 12.0);
    assert!(solution.warnings().is_empty(), "{:?}", solution.warnings());
    let size = solution.size();
    let pages = vec![page(solution, &["plot"], &guides, [0.0, 0.0], &engine)];
    render(
        "a_colorbar_below_a_legend_fills_the_rest_of_the_plot_height",
        [size.width, size.height],
        pages,
        &engine,
    );
}

/// Three panels in a Wrap with legends of different widths match their legend and colorbar
/// slabs, so every stack and every plot has one width.
#[test]
fn stacks_match_their_widths_in_a_wrap() {
    let engine = engine();
    let titles = ["cohort", "region", "a longer legend title"];
    let legends = titles.map(|title| titled_legend(title, LegendLayout::default(), 3, &engine));
    let colorbar = colorbar((0.0, 100.0), "temperature", &engine);
    let ids = [
        ["p0", "legend 0", "colorbar 0"],
        ["p1", "legend 1", "colorbar 1"],
        ["p2", "legend 2", "colorbar 2"],
    ];
    let layout: Layout<Id> = Wrap::new(ids.map(|[plot, legend, colorbar]| {
        legend_stack(
            Leaf::new().id(plot).min_size(Size::new(120.0, 150.0)),
            legend,
            colorbar,
        )
    }))
    .gap(Gap::new(16.0))
    .match_tag("legend")
    .match_tag("colorbar")
    .into();
    let mut guides = Guides {
        slabs: ids
            .iter()
            .zip(&legends)
            .flat_map(|([_, legend_id, colorbar_id], legend)| {
                [
                    (*legend_id, GuideSide::Right, Guide::Legend(legend)),
                    (*colorbar_id, GuideSide::Right, Guide::Colorbar(&colorbar)),
                ]
            })
            .collect(),
    };
    let measured = layout.measure(&padded(900.0, None), &mut guides).unwrap();
    let solution = measured.solution();
    let width = |id: Id| solution.region(&id).unwrap().slot.width;
    let plots: Vec<f32> = ids
        .iter()
        .map(|[plot, ..]| solution.region(plot).unwrap().content.width)
        .collect();
    let legend_widths: Vec<f32> = ids.iter().map(|[_, legend, _]| width(legend)).collect();
    let equal = |sizes: &[f32]| sizes.iter().all(|&size| (size - sizes[0]).abs() < 1e-3);
    assert!(equal(&plots), "{plots:?}");
    assert!(equal(&legend_widths), "{legend_widths:?}");
    assert!(legends[0].extent(100.0).thickness < legend_widths[0]);
    assert!(solution.warnings().is_empty(), "{:?}", solution.warnings());
    let size = solution.size();
    let pages = vec![page(
        solution,
        &["p0", "p1", "p2"],
        &guides,
        [0.0, 0.0],
        &engine,
    )];
    render(
        "stacks_match_their_widths_in_a_wrap",
        [size.width, size.height],
        pages,
        &engine,
    );
}

/// Two panels in a Column show one legend and one colorbar, so a merge rule replaces their
/// stacks with one stack along both panels, whose colorbar fills their combined height.
#[test]
fn stacks_merge_along_a_figure() {
    let engine = engine();
    let legend = legend(LegendLayout::default(), 4, &engine);
    let colorbar = colorbar((0.0, 100.0), "temperature", &engine);
    let panel = |plot: Id, legend: Id, colorbar: Id| {
        Leaf::new()
            .id(plot)
            .min_size(Size::new(240.0, 110.0))
            .slab(Side::Right, Slab::new().min_width(12.0))
            .slab_stack(
                Side::Right,
                SlabStack::new([Slab::new().id(legend), Slab::new().id(colorbar)])
                    .track(0, Track::auto())
                    .gap(Gap::new(12.0))
                    .tag("legends")
                    .content_key("cohort, temperature"),
            )
    };
    let layout: Layout<Id> = Column::new([
        panel("top", "legend", "colorbar"),
        panel("bottom", "legend 1", "colorbar 1"),
    ])
    .gap(Gap::new(16.0))
    .merge_tag("legends")
    .into();
    let mut guides = Guides {
        slabs: vec![
            ("legend", GuideSide::Right, Guide::Legend(&legend)),
            ("colorbar", GuideSide::Right, Guide::Colorbar(&colorbar)),
            ("legend 1", GuideSide::Right, Guide::Legend(&legend)),
            ("colorbar 1", GuideSide::Right, Guide::Colorbar(&colorbar)),
        ],
    };
    let measured = layout.measure(&padded(420.0, None), &mut guides).unwrap();
    let solution = measured.solution();
    let top = solution.region(&"top").unwrap().content;
    let bottom = solution.region(&"bottom").unwrap().content;
    let legend_rect = solution.region(&"legend").unwrap().content;
    let colorbar_rect = solution.region(&"colorbar").unwrap().content;
    // The merged stack takes the first stack's ids and runs along both panels.
    assert_eq!(legend_rect.y, top.y);
    assert_eq!(colorbar_rect.bottom(), bottom.bottom());
    assert!(colorbar_rect.y < top.bottom());
    assert_eq!(
        solution.representative(&"colorbar 1").unwrap().id,
        Some("colorbar")
    );
    assert!(solution.warnings().is_empty(), "{:?}", solution.warnings());
    let size = solution.size();
    let pages = vec![page(
        solution,
        &["top", "bottom"],
        &guides,
        [0.0, 0.0],
        &engine,
    )];
    render(
        "stacks_merge_along_a_figure",
        [size.width, size.height],
        pages,
        &engine,
    );
}
