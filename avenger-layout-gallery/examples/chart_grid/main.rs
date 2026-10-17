//! Lays out three plots with measured axes and a legend, with the third plot spanning the bottom
//! row, and renders the page through SVG.
//!
//! Run `cargo run --release -p avenger-layout-gallery --example chart_grid`. An optional first argument
//! sets the PNG path; the default is `target/layout-gallery/chart-grid.png`.

mod chart;

use avenger_render_fixtures::raster::svg_to_png;
use avenger_svg::SvgRenderer;

fn main() {
    let chart = chart::Chart::new();
    let measured = chart.solve(false);
    let solution = measured.solution();
    for index in 0..chart.specs.len() {
        let rect = chart::plot_rect(solution, index);
        println!(
            "plot {index}: {:.1} x {:.1} at ({:.1}, {:.1})",
            rect.width, rect.height, rect.x, rect.y
        );
    }
    let scene = chart.scene(solution);
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &chart.engine)
        .expect("the scene renders to SVG");
    let image = svg_to_png(&svg, 2.0);
    let out = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("target/layout-gallery/chart-grid.png"));
    if let Some(parent) = out.parent().filter(|path| !path.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).expect("the output directory exists");
    }
    image.save(&out).expect("the PNG saves");
    println!("wrote {}", out.display());
}

#[cfg(test)]
mod tests {
    use avenger_geometry::marks::TextGeometryUtils;
    use avenger_layout::{Rect, Side};

    use super::chart::{self, Chart, ChartId, PARTS};

    /// How far text and rules reach past the shapes that the layout measures, which bounding
    /// boxes for hit testing add.
    const HIT_MARGIN: f32 = 1.0;

    #[test]
    fn measured_guides_fit_their_slabs() {
        let chart = Chart::new();
        let measured = chart.solve(false);
        let solution = measured.solution();
        assert_guides_fit(&chart, solution);
        let first = chart::plot_rect(solution, 0);
        let second = chart::plot_rect(solution, 1);
        let spanning = chart::plot_rect(solution, 2);
        assert_eq!(first.x, spanning.x);
        assert!((second.right() - spanning.right()).abs() < 0.01);
        assert_eq!(first.width, second.width);
    }

    /// Checks that each axis part, measured again at its slab's final length, fits the slab
    /// that the layout gave it, that the drawn parts stay inside the root and off the other
    /// plots, and that the legend fits its slab.
    fn assert_guides_fit(chart: &Chart, solution: &avenger_layout::Solution<ChartId>) {
        let engine = &chart.engine;
        let root = solution.at_path(&[]).expect("the root has a region").slot;
        let plots: Vec<Rect> = (0..chart.specs.len())
            .map(|index| chart::plot_rect(solution, index))
            .collect();
        for (index, [x, y]) in chart.axes.iter().enumerate() {
            for part in PARTS {
                for (axis, id, side) in [
                    (x, ChartId::XAxis(index, part), Side::Bottom),
                    (y, ChartId::YAxis(index, part), Side::Left),
                ] {
                    let slot = solution.region(&id).expect("every slab has a region").slot;
                    let (length, thickness) = match side {
                        Side::Bottom => (slot.width, slot.height),
                        _ => (slot.height, slot.width),
                    };
                    let extent = axis.extent(part, length);
                    assert!(
                        extent.thickness <= thickness + 0.01,
                        "plot {index} {id:?}: {} in {thickness}",
                        extent.thickness
                    );
                    let Some(group) = chart::drawn_part(solution, axis, id, part) else {
                        continue;
                    };
                    let bounds = group.bounding_box(engine);
                    let [left, top] = bounds.lower();
                    let [right, bottom] = bounds.upper();
                    let drawn = Rect::new(
                        left + HIT_MARGIN,
                        top + HIT_MARGIN,
                        right - left - 2.0 * HIT_MARGIN,
                        bottom - top - 2.0 * HIT_MARGIN,
                    );
                    assert!(
                        root.x <= drawn.x + 0.01
                            && root.y <= drawn.y + 0.01
                            && drawn.right() <= root.right() + 0.01
                            && drawn.bottom() <= root.bottom() + 0.01,
                        "plot {index} {id:?}: {drawn:?} outside {root:?}"
                    );
                    for (other, plot) in plots
                        .iter()
                        .enumerate()
                        .filter(|(other, _)| *other != index)
                    {
                        let apart = drawn.right() <= plot.x + 0.01
                            || plot.right() <= drawn.x + 0.01
                            || drawn.bottom() <= plot.y + 0.01
                            || plot.bottom() <= drawn.y + 0.01;
                        assert!(apart, "plot {index}'s {id:?} reaches plot {other}");
                    }
                }
            }
        }
        for (index, spec) in chart.specs.iter().enumerate() {
            if spec.legend {
                let slab = solution.region(&ChartId::Legend(index)).unwrap().slot;
                let extent = chart.legend.extent(slab.height);
                assert!(extent.thickness <= slab.width + 0.01);
                assert_eq!(
                    extent.overhang,
                    [0.0, 0.0],
                    "the legend fits beside its plot"
                );
            }
        }
    }
}
