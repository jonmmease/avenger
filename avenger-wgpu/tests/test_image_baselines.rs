#[cfg(test)]
mod test_image_baselines {
    use avenger::scene_graph::SceneGraph;
    use avenger_vega::scene_graph::VegaSceneGraph;
    use avenger_wgpu::canvas::{Canvas, CanvasDimensions, PngCanvas};
    use dssim::Dssim;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;

    #[rstest(
        category,
        spec_name,
        tolerance,
        case("rect", "stacked_bar", 0.001),
        case("rect", "stacked_bar_stroke", 0.001),
        case("rect", "stacked_bar_rounded", 0.001),
        case("rect", "stacked_bar_rounded_stroke", 0.001),
        case("rect", "stacked_bar_rounded_stroke_opacity", 0.009),
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
        case("symbol", "binned_scatter_cross_stroke", 0.001),
        case("symbol", "binned_scatter_circle_stroke", 0.001),
        case("symbol", "binned_scatter_circle_stroke_no_fill", 0.001),
        case("symbol", "binned_scatter_path_star_stroke_no_fill", 0.001),
        case("symbol", "scatter_transparent_stroke", 0.001),
        case("symbol", "scatter_transparent_stroke_star", 0.006),
        case("symbol", "wind_vector", 0.0015),
        case("symbol", "wedge_angle", 0.001),
        case("symbol", "wedge_stroke_angle", 0.001),
        case("symbol", "zindex_circles", 0.001),
        case("symbol", "mixed_symbols", 0.001),
        case("rule", "wide_rule_axes", 0.0001),
        case("rule", "wide_transparent_caps", 0.0001),
        case("rule", "dashed_rules", 0.0001),
        case("text", "bar_axis_labels", 0.01),
        case("text", "text_rotation", 0.015),
        case("text", "letter_scatter", 0.012),

        // vl-convert doesn't support emoji at all
        case("text", "emoji", 2.0),
        case("arc", "single_arc_no_inner", 0.0005),
        case("arc", "single_arc_with_inner_radius", 0.0005),
        case("arc", "single_arc_with_inner_radius_wrap", 0.0005),
        case("arc", "single_arc_with_inner_radius_wrap_stroke", 0.0005),
        case("arc", "arcs_with_variable_outer_radius", 0.0005),
        case("arc", "arcs_with_variable_outer_radius_stroke", 0.0005),
        case("arc", "arc_with_stroke", 0.0005),
        case("path", "single_path_no_stroke", 0.0005),
        case("path", "multi_path_no_stroke", 0.0005),

        // vl-convert/resvg messes up the path_with_stroke examples because it scales the path
        // width. The Vega editor renderers don't do this.
        case("path", "single_path_with_stroke", 0.8),
        case("path", "single_path_with_stroke_no_fill", 0.8),
        case("path", "multi_path_with_stroke", 0.8),
        case("path", "multi_path_with_stroke_no_fill", 0.8),

        // us-counties is a bit off due to how anti-aliasing results in light border between
        // adjacent shapes. The wgpu implementation doesn't have this border
        case("shape", "us-counties", 0.003),
        case("shape", "us-map", 0.0006),
        case("shape", "world-natural-earth-projection", 0.0006),
        case("shape", "london_tubes", 0.0002),

        case("line", "simple_line_round_cap", 0.0001),
        case("line", "simple_line_butt_cap_miter_join", 0.0001),
        // lyon seems to omit closing square cap, need to investigate
        case("line", "simple_line_square_cap_bevel_join", 0.002),
        case("line", "connected_scatter", 0.0008),
        case("line", "lines_with_open_symbols", 0.0004),
        case("line", "stocks", 0.0005),
        case("line", "stocks-legend", 0.003),
        case("line", "simple_dashed", 0.0005),
        case("line", "stocks_dashed", 0.001),
        case("line", "line_dashed_round_undefined", 0.0005),

        // lyon's square end cap doesn't seem to work
        case("line", "line_dashed_square_undefined", 0.007),
        case("line", "line_dashed_butt_undefined", 0.0005),

        case("area", "100_percent_stacked_area", 0.005),
        case("area", "simple_unemployment", 0.0005),
        case("area", "simple_unemployment_stroke", 0.0005),
        case("area", "stacked_area", 0.005),
        case("area", "streamgraph_area", 0.005),
        case("area", "with_undefined", 0.0005),
        case("area", "with_undefined_horizontal", 0.0005),

        case("trail", "trail_stocks", 0.0005),
        case("trail", "trail_stocks_opacity", 0.0005),

        case("image", "logos", 0.001),
        case("image", "logos_sized_aspect_false", 0.001),
        case("image", "logos_sized_aspect_false_align_baseline", 0.001),
        case("image", "logos_sized_aspect_true_align_baseline", 0.001),
        case("image", "smooth_false", 0.03),  // vl-convert/resvg doesn't support smooth=false
        case("image", "smooth_true", 0.001),
        case("image", "many_images", 0.001),
        case("image", "large_images", 0.001),

        case("gradients", "heatmap_with_colorbar", 0.001),
        case("gradients", "diagonal_gradient_bars_rounded", 0.001),
        case("gradients", "default_gradient_bars_rounded_stroke", 0.0015),
        case("gradients", "residuals_colorscale", 0.0015),
        case("gradients", "stroke_rect_gradient", 0.002),
        case("gradients", "area_with_gradient", 0.001),
        case("gradients", "area_line_with_gradient", 0.001),
        case("gradients", "trail_gradient", 0.001),

        // vl-convert/resvg messes up scaled paths with strokes
        case("gradients", "path_with_stroke_gradients", 0.5),
        case("gradients", "rules_with_gradients", 0.004), // Slight difference in bounding box for square caps
        case("gradients", "symbol_cross_gradient", 0.001),
        case("gradients", "symbol_circles_gradient_stroke", 0.001),

        // Our gradient bounding box for arc marks is the full circle, not the bounding box around the arc wedge
        case("gradients", "arc_gradient", 0.1),

        // vl-convert/resvg doesn't handle focus radius properly
        case("gradients", "radial_concentric_gradient_bars", 0.03),
        case("gradients", "radial_offset_gradient_bars", 0.02),
        case("gradients", "symbol_radial_gradient", 0.002),
    )]
    fn test_image_baseline(category: &str, spec_name: &str, tolerance: f64) {
        println!("{spec_name}");
        let specs_dir = format!(
            "{}/../avenger-vega-test-data/vega-scenegraphs/{category}",
            env!("CARGO_MANIFEST_DIR")
        );
        let output_dir = format!("{}/tests/output", env!("CARGO_MANIFEST_DIR"));
        fs::create_dir_all(Path::new(&output_dir)).unwrap();

        // Read scene graph spec
        let scene_spec_str =
            fs::read_to_string(format!("{specs_dir}/{spec_name}.sg.json")).unwrap();
        let scene_spec: VegaSceneGraph = serde_json::from_str(&scene_spec_str).unwrap();
        // println!("{scene_spec:#?}");

        // Read expected png
        let expected_dssim = dssim::load_image(
            &Dssim::new(),
            Path::new(&format!("{specs_dir}/{spec_name}.png")),
        )
        .ok()
        .unwrap();

        // Build scene graph
        let scene_graph: SceneGraph = scene_spec
            .to_scene_graph()
            .expect("Failed to parse scene graph");

        let mut png_canvas = pollster::block_on(PngCanvas::new(CanvasDimensions {
            size: [scene_graph.width, scene_graph.height],
            scale: 2.0,
        }))
        .unwrap();
        png_canvas.set_scene(&scene_graph).unwrap();
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
