use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SceneGraphDims {
    width: f32,
    height: f32,
    origin_x: f32,
    origin_y: f32,
}

#[cfg(test)]
mod test_image_baselines {
    use crate::SceneGraphDims;
    use dssim::Dssim;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;
    use vega_wgpu_renderer::renderers::canvas::{Canvas, PngCanvas};
    use vega_wgpu_renderer::scene::scene_graph::SceneGraph;
    use vega_wgpu_renderer::specs::SceneGraphSpec;

    #[rstest(
        spec_name,
        tolerance,
        case("stacked_bar", 0.001),
        case("heatmap", 0.006),
        case("binned_scatter_diamonds", 0.005)
    )]
    fn test_image_baseline(spec_name: &str, tolerance: f64) {
        let specs_dir = format!("{}/tests/specs/rect", env!("CARGO_MANIFEST_DIR"));
        let output_dir = format!("{}/tests/output", env!("CARGO_MANIFEST_DIR"));

        // Read scene graph spec
        let scene_spec_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.sg.json")).unwrap();
        let scene_spec: SceneGraphSpec = serde_json::from_str(&scene_spec_str).unwrap();

        // Read dims
        let scene_dims_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.dims.json")).unwrap();
        let scene_dims: SceneGraphDims = serde_json::from_str(&scene_dims_str).unwrap();
        let width = scene_dims.width;
        let height = scene_dims.height;
        let origin = [scene_dims.origin_x, scene_dims.origin_y];

        // Read expected png
        let expected_dssim = dssim::load_image(
            &Dssim::new(),
            Path::new(&format!("{specs_dir}/{spec_name}.png")),
        )
        .ok()
        .unwrap();

        // Build scene graph
        let scene_graph: SceneGraph = SceneGraph::from_spec(&scene_spec, origin, width, height)
            .expect("Failed to parse scene graph");

        let mut png_canvas = pollster::block_on(PngCanvas::new(width, height, origin)).unwrap();
        png_canvas.set_scene(&scene_graph);
        let img = pollster::block_on(png_canvas.render()).expect("Failed to render PNG image");
        let result_path = format!("{output_dir}/{spec_name}.png");
        img.save(&result_path).unwrap();
        let result_dssim = dssim::load_image(&Dssim::new(), result_path).unwrap();

        // Compare images
        let attr = Dssim::new();
        let (diff, _) = attr.compare(&expected_dssim, result_dssim);
        println!("{diff}");
        assert!(diff < tolerance);
    }

    #[test]
    fn test_marker() {} // Help IDE detect test module
}
