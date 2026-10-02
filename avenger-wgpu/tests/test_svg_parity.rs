#[path = "../../tests/render_fixtures/parity.rs"]
mod parity;

use avenger_common::canvas::CanvasDimensions;
use avenger_svg::SvgRenderer;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

#[test]
fn svg_parity_scenes_render_expected_coverage() {
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [420.0, 240.0],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))
    .unwrap();
    let output = std::env::var_os("AVENGER_PARITY_OUTPUT").map(std::path::PathBuf::from);
    if let Some(output) = &output {
        std::fs::create_dir_all(output).unwrap();
    }
    for case in parity::cases() {
        eprintln!(
            "render {} (browser reference: {})",
            case.name, case.browser_only
        );
        let size = [case.scene.width, case.scene.height];
        if canvas.dimensions().size != size {
            canvas = pollster::block_on(PngCanvas::new(
                CanvasDimensions { size, scale: 2.0 },
                CanvasConfig::default(),
            ))
            .unwrap();
        }
        canvas.set_scene(&case.scene).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        let svg = SvgRenderer::new().render_scene_graph(&case.scene).unwrap();
        if let Some(output) = &output {
            std::fs::write(output.join(format!("{}.svg", case.name)), &svg).unwrap();
            image
                .save(output.join(format!("{}-wgpu.png", case.name)))
                .unwrap();
        }
        if !case.browser_only {
            let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).unwrap();
            let mut pixels = resvg::tiny_skia::Pixmap::new(image.width(), image.height()).unwrap();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(2.0, 2.0),
                &mut pixels.as_mut(),
            );
            let reference = image::load_from_memory(&pixels.encode_png().unwrap())
                .unwrap()
                .to_rgba8();
            let large = image
                .pixels()
                .zip(reference.pixels())
                .filter(|(a, b)| a.0.into_iter().zip(b.0).any(|(a, b)| a.abs_diff(b) > 20))
                .count();
            assert!(
                large < image.pixels().len() / 100,
                "{}: {large} pixels differ beyond edge tolerance",
                case.name
            );
        }
        for (point, expected) in case.samples {
            let actual = image.get_pixel(point[0] * 2, point[1] * 2).0;
            assert!(
                actual
                    .into_iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(b) <= 3),
                "{} {point:?}: {actual:?} != {expected:?}",
                case.name
            );
        }
    }
}
