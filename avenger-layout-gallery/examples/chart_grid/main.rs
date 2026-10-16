//! Lays out three plots with measured axes and a legend, with the third plot spanning the bottom
//! row, and renders the page through SVG.
//!
//! Run `cargo run --release -p avenger-layout-gallery --example chart_grid`. An optional first argument
//! sets the PNG path; the default is `target/layout-gallery/chart-grid.png`.

mod chart;

use avenger_render_fixtures::raster::svg_to_png;
use avenger_svg::SvgRenderer;

fn main() {
    let specs = chart::specs();
    let engine = chart::text_engine();
    let measured = chart::solve(&specs, &engine, false);
    let solution = measured.solution();
    for index in 0..specs.len() {
        let rect = chart::plot_rect(solution, index);
        println!(
            "plot {index}: {:.1} x {:.1} at ({:.1}, {:.1})",
            rect.width, rect.height, rect.x, rect.y
        );
    }
    let scene = chart::scene(&specs, solution, &engine);
    let svg = SvgRenderer::new()
        .render_scene_graph(&scene, &engine)
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
    use avenger_layout::{Rect, Size, Solution};
    use avenger_typst_label::LabelEngine;

    use super::chart::{self, ChartId, PlotSpec};

    #[test]
    fn measured_guides_fit_their_slabs() {
        let specs = chart::specs();
        let engine = chart::text_engine();
        let measured = chart::solve(&specs, &engine, false);
        let solution = measured.solution();
        assert_guides_fit(&specs, solution, &engine);
        let first = chart::plot_rect(solution, 0);
        let second = chart::plot_rect(solution, 1);
        let spanning = chart::plot_rect(solution, 2);
        assert_eq!(first.x, spanning.x);
        assert!((second.right() - spanning.right()).abs() < 0.01);
        assert_eq!(first.width, second.width);
    }

    /// Checks that each plot's axes and legend, measured again at the plot's final size, fit the
    /// slabs that the layout gave them, and that the labels past each plot's ends stay inside
    /// the root and off the other plots.
    fn assert_guides_fit(specs: &[PlotSpec], solution: &Solution<ChartId>, engine: &LabelEngine) {
        let slab = |id| solution.region(&id).expect("every slab has a region").slot;
        let root = solution.at_path(&[]).expect("the root has a region").slot;
        let plots: Vec<Rect> = (0..specs.len())
            .map(|index| chart::plot_rect(solution, index))
            .collect();
        for (index, spec) in specs.iter().enumerate() {
            let rect = chart::plot_rect(solution, index);
            let size = Size::new(rect.width, rect.height);
            let overflow = chart::measure_overflow(
                &chart::build_axes(spec, size, [0.0, 0.0], false, engine),
                size,
                engine,
            );
            assert!(
                overflow.left <= slab(ChartId::YAxis(index)).width,
                "plot {index} y axis"
            );
            assert!(
                overflow.bottom <= slab(ChartId::XAxis(index)).height,
                "plot {index} x axis"
            );
            for axis in chart::build_axes(spec, size, [rect.x, rect.y], false, engine) {
                let bounds = axis.bounding_box(engine);
                let [x, y] = bounds.lower();
                let [right, bottom] = bounds.upper();
                let drawn = Rect::new(x, y, right - x, bottom - y);
                assert!(
                    root.x <= drawn.x + 0.01
                        && root.y <= drawn.y + 0.01
                        && drawn.right() <= root.right() + 0.01
                        && drawn.bottom() <= root.bottom() + 0.01,
                    "plot {index}: {drawn:?} outside {root:?}"
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
                    assert!(apart, "plot {index}'s axis reaches plot {other}");
                }
            }
            if spec.legend {
                let bounds = chart::build_legend(engine).bounding_box(engine);
                assert!(bounds.upper()[0] <= slab(ChartId::Legend(index)).width + 0.01);
                assert!(bounds.upper()[1] <= rect.height);
            }
        }
    }
}
