#[cfg(test)]
mod test_image_baselines {
    use dssim::Dssim;
    use rstest::rstest;
    use sg2d::scene_graph::SceneGraph;
    use sg2d_vega::dims::VegaSceneGraphDims;
    use sg2d_vega::scene_graph::VegaSceneGraph;
    use sg2d_wgpu::canvas::{Canvas, PngCanvas};
    use std::fs;
    use std::path::Path;

    #[rstest(
        category,
        spec_name,
        tolerance,
        case("rect", "stacked_bar", 0.001),
        case("rect", "heatmap", 0.006),
        case("symbol", "binned_scatter_diamonds", 0.001),
        case("symbol", "binned_scatter_square", 0.001),
        case("symbol", "binned_scatter_triangle-down", 0.001),
        case("symbol", "binned_scatter_triangle-up", 0.001),
        case("symbol", "binned_scatter_triangle-left", 0.001),
        case("symbol", "binned_scatter_triangle-right", 0.001),
        case("symbol", "binned_scatter_triangle", 0.001),
        case("symbol", "binned_scatter_wedge", 0.001),
        case("symbol", "binned_scatter_arrow", 0.001),
        case("symbol", "binned_scatter_cross", 0.001),
        case("symbol", "binned_scatter_circle", 0.001),
        case("symbol", "binned_scatter_path", 0.001),
        case("symbol", "binned_scatter_path_star", 0.001),
        case("rule", "wide_rule_axes", 0.0001),
        case("text", "bar_axis_labels", 0.01)
    )]
    fn test_image_baseline(category: &str, spec_name: &str, tolerance: f64) {
        let specs_dir = format!(
            "{}/../sg2d-vega-test-data/vega-scenegraphs/{category}",
            env!("CARGO_MANIFEST_DIR")
        );
        let output_dir = format!("{}/tests/output", env!("CARGO_MANIFEST_DIR"));
        fs::create_dir_all(Path::new(&output_dir)).unwrap();

        // Read scene graph spec
        let scene_spec_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.sg.json")).unwrap();
        let scene_spec: VegaSceneGraph = serde_json::from_str(&scene_spec_str).unwrap();

        // Read dims
        let scene_dims_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.dims.json")).unwrap();
        let scene_dims: VegaSceneGraphDims = serde_json::from_str(&scene_dims_str).unwrap();
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
        let scene_graph: SceneGraph = scene_spec
            .to_scene_graph(origin, width, height)
            .expect("Failed to parse scene graph");

        let mut png_canvas = pollster::block_on(PngCanvas::new(width, height)).unwrap();
        png_canvas.set_scene(&scene_graph);
        let img = pollster::block_on(png_canvas.render()).expect("Failed to render PNG image");
        let result_path = format!("{output_dir}/{category}-{spec_name}.png");
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
