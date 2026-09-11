#[cfg(test)]
mod test_image_baselines {
    use avenger_scenegraph::marks::{
        group::Clip,
        path::ScenePathMark,
        pattern::{PatternAnchor, PatternFill, PatternInk, PatternLayer, StripePatternLayer},
        rect::SceneRectMark,
    };
    use lyon::{geom::point, path::Path as LyonPath};

    use avenger_color::ColorOrGradient;
    use avenger_common::{canvas::CanvasDimensions, types::SymbolShape, value::ScalarOrArray};
    use avenger_scenegraph::marks::{group::SceneGroup, symbol::SceneSymbolMark};
    use avenger_wgpu::canvas::CanvasConfig;

    use avenger_scenegraph::scene_graph::SceneGraph;
    use avenger_vega_scenegraph::scene_graph::VegaSceneGraph;
    use avenger_wgpu::canvas::{Canvas, PngCanvas};
    use dssim::Dssim;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;

    fn vega_font_family(families: &[&str]) -> String {
        use avenger_text::{font_resolver::FontdbFontResolver, FontResolver};
        // Match the generic family used by the original Vega comparison harness.
        let resolver = FontdbFontResolver::new();
        resolver.select_available_font(
            families
                .iter()
                .map(|family| (*family).to_string())
                .collect(),
        )
    }

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
        case("symbol", "scatter_transparent_stroke", 0.005),
        case("symbol", "scatter_transparent_stroke_star", 0.006),
        case("symbol", "circle_fast_path_large_translucent", 0.001),
        case("symbol", "wind_vector", 0.0015),
        case("symbol", "wedge_angle", 0.001),
        case("symbol", "wedge_stroke_angle", 0.001),
        case("symbol", "zindex_circles", 0.001),
        case("symbol", "mixed_symbols", 0.001),
        case("rule", "wide_rule_axes", 0.0001),

        // lyon seems to omit closing square cap, need to investigate
        case("rule", "wide_transparent_caps", 0.08),
        case("rule", "dashed_rules", 0.004),

        case("text", "bar_axis_labels", 0.033),
        case("text", "text_alignment", 0.02),
        case("text", "text_rotation", 0.02),
        case("text", "letter_scatter", 0.055),
        case("text", "lasagna_plot", 0.033),
        case("text", "arc_radial", 0.01),

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
        case("line", "stocks-legend", 0.01),
        case("line", "simple_dashed", 0.0005),
        case("line", "stocks_dashed", 0.003),
        case("line", "line_dashed_round_undefined", 0.0005),

        // lyon's square end cap doesn't seem to work
        case("line", "line_dashed_square_undefined", 0.007),
        case("line", "line_dashed_butt_undefined", 0.0005),

        // case("area", "100_percent_stacked_area", 0.005),
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
        case("gradients", "rules_with_gradients", 0.01), // Lyon square caps issue
        case("gradients", "symbol_cross_gradient", 0.001),
        case("gradients", "symbol_circles_gradient_stroke", 0.001),

        // Our gradient bounding box for arc marks is the full circle, not the bounding box around the arc wedge
        case("gradients", "arc_gradient", 0.1),

        // vl-convert/resvg doesn't handle focus radius properly
        case("gradients", "radial_concentric_gradient_bars", 0.03),
        case("gradients", "radial_offset_gradient_bars", 0.02),
        case("gradients", "symbol_radial_gradient", 0.002),
        case("vl-convert", "bar_chart_trellis_compact", 0.02),
        case("vl-convert", "circle_binned", 0.03),
        case("vl-convert", "circle_binned_base_url", 0.03),
        case("vl-convert", "custom_projection", 0.001),
        case("vl-convert", "float_font_size", 0.015),
        case("vl-convert", "font_with_quotes", 0.02),
        case("vl-convert", "line_with_log_scale", 0.026),
        case("vl-convert", "long_legend_label", 0.012),
        case("vl-convert", "lookup_urls", 0.01),
        case("vl-convert", "numeric_font_weight", 0.02),
        case("vl-convert", "quakes_initial_selection", 0.01),
        case("vl-convert", "remote_images", 0.01),
        case("vl-convert", "seattle-weather", 0.013),
        case("vl-convert", "stocks_locale", 0.012),
        case("vl-convert", "table_heatmap", 0.04),
        case("vl-convert", "stacked_bar_h", 0.02),
        case("vl-convert", "geoScale", 0.01),
        case("vl-convert", "maptile_background", 0.01),
        case("vl-convert", "no_text_in_font_metrics", 0.03),
        case("clip", "clip_mixed_marks", 0.0001),
        case("clip", "text_clip", 0.02),
        case("clip", "clip_rounded", 0.0001),
        case("clip", "text_clip_rounded", 0.02),
        case("clip", "bar_rounded", 0.02),
    )]
    fn test_image_baseline(category: &str, spec_name: &str, tolerance: f64) {
        use avenger_common::canvas::CanvasDimensions;

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

        // println!("{}", serde_json::to_string_pretty(&scene_graph).unwrap());

        let mut png_canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [scene_graph.width, scene_graph.height],
                scale: 2.0,
            },
            avenger_wgpu::canvas::CanvasConfig {
                font_resolution: avenger_text::FontResolutionOptions {
                    extra_font_dirs: vec![Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../avenger-vega-test-data/fonts")],
                    default_sans_serif_family: Some(vega_font_family(&[
                        "Helvetica",
                        "Arial",
                        "Liberation Sans",
                        "sans-serif",
                    ])),
                    default_monospace_family: Some(vega_font_family(&[
                        "Courier New",
                        "Courier",
                        "Liberation Mono",
                        "DejaVu Sans Mono",
                    ])),
                    ..avenger_text::default_font_resolution()
                },
                ..Default::default()
            },
        ))
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

    #[test]
    fn open_circle_edges_do_not_mix_in_transparent_fill_rgb() {
        let mark = SceneSymbolMark {
            len: 128,
            x: ScalarOrArray::new_array((0..128).map(|i| 8.0 + (i % 16) as f32 * 14.0).collect()),
            y: ScalarOrArray::new_array((0..128).map(|i| 8.0 + (i / 16) as f32 * 14.0).collect()),
            size: ScalarOrArray::new_scalar(36.0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
            stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 1.0])),
            stroke_width: Some(2.0),
            ..Default::default()
        };
        let scene = SceneGraph {
            width: 226.0,
            height: 114.0,
            origin: [0.0, 0.0],
            marks: vec![mark.into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [scene.width, scene.height],
                scale: 2.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();
        canvas.set_scene(&scene).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        assert!(
            image.pixels().any(|pixel| pixel.0[1] < 200),
            "expected red strokes"
        );
        assert!(
            image.pixels().all(|pixel| pixel.0[0] >= 254),
            "transparent black contaminated red edges"
        );
    }

    #[test]
    fn reused_instanced_symbol_renderer_respects_changed_group_origin() {
        let scene = |origin: [f32; 2]| SceneGraph {
            width: 160.0,
            height: 160.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                origin,
                marks: vec![SceneSymbolMark {
                    len: 100,
                    shapes: vec![SymbolShape::Circle],
                    x: ScalarOrArray::new_array(
                        (0..100).map(|index| (index % 10) as f32 * 5.0).collect(),
                    ),
                    y: ScalarOrArray::new_array(
                        (0..100).map(|index| (index / 10) as f32 * 5.0).collect(),
                    ),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.1, 0.4, 0.8, 1.0])),
                    size: ScalarOrArray::new_scalar(9.0),
                    shape_index: ScalarOrArray::new_scalar(0),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };
        let dimensions = CanvasDimensions {
            size: [160.0, 160.0],
            scale: 1.0,
        };
        let first = scene([10.0, 10.0]);
        let moved = scene([70.0, 70.0]);

        let mut reused = pollster::block_on(PngCanvas::new(dimensions, CanvasConfig::default()))
            .expect("reused canvas");
        reused.set_scene(&first).expect("install first scene");
        pollster::block_on(reused.render()).expect("render first scene");
        reused.set_scene(&moved).expect("install moved scene");
        let reused_image = pollster::block_on(reused.render()).expect("render moved scene");

        let mut fresh = pollster::block_on(PngCanvas::new(dimensions, CanvasConfig::default()))
            .expect("fresh canvas");
        fresh.set_scene(&moved).expect("install fresh moved scene");
        let fresh_image = pollster::block_on(fresh.render()).expect("render fresh moved scene");

        assert!(
            reused_image.as_raw() == fresh_image.as_raw(),
            "instanced renderer reuse must not retain the previous group origin"
        );
    }

    #[test]
    fn pattern_layer_operations_render_on_every_host_mark() {
        use avenger_scenegraph::marks::{
            arc::SceneArcMark,
            area::SceneAreaMark,
            mark::SceneMark,
            pattern::{PatternLayerOperation, PatternReferenceFrame},
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [40.0, 40.0],
                scale: 1.0,
            },
            CanvasConfig {
                sample_count: Some(1),
                ..Default::default()
            },
        ))
        .unwrap();
        for operation in [
            PatternLayerOperation::Add,
            PatternLayerOperation::Subtract,
            PatternLayerOperation::Xor,
        ] {
            let mut vertical = StripePatternLayer::new(90.0, 8.0, 4.0);
            vertical.operation = operation;
            let pattern = PatternFill {
                anchor: PatternAnchor::Plot,
                ink: PatternInk::Solid {
                    color: [0.0, 0.0, 0.0, 1.0],
                    opacity: 0.5,
                },
                layers: vec![
                    PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 4.0)),
                    PatternLayer::Stripe(vertical),
                ],
            };
            let fill = ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0]);
            let hosts: Vec<(&str, SceneMark)> = vec![
                (
                    "rect",
                    SceneRectMark {
                        width: Some(ScalarOrArray::new_scalar(32.0)),
                        height: Some(ScalarOrArray::new_scalar(32.0)),
                        fill: ScalarOrArray::new_scalar(fill.clone()),
                        fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                        ..Default::default()
                    }
                    .into(),
                ),
                (
                    "arc",
                    SceneArcMark {
                        x: ScalarOrArray::new_scalar(16.0),
                        y: ScalarOrArray::new_scalar(16.0),
                        inner_radius: ScalarOrArray::new_scalar(4.0),
                        outer_radius: ScalarOrArray::new_scalar(16.0),
                        end_angle: ScalarOrArray::new_scalar(std::f32::consts::TAU),
                        fill: ScalarOrArray::new_scalar(fill.clone()),
                        fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                        ..Default::default()
                    }
                    .into(),
                ),
                (
                    "area",
                    SceneAreaMark {
                        len: 2,
                        x: ScalarOrArray::new_array(vec![0.0, 32.0]),
                        y: ScalarOrArray::new_scalar(0.0),
                        y2: ScalarOrArray::new_scalar(32.0),
                        fill: fill.clone(),
                        fill_pattern: Some(pattern.clone()),
                        ..Default::default()
                    }
                    .into(),
                ),
                (
                    "path",
                    ScenePathMark {
                        path: ScalarOrArray::new_scalar(rect_path(0.0, 0.0, 32.0, 32.0)),
                        fill: ScalarOrArray::new_scalar(fill.clone()),
                        fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                        ..Default::default()
                    }
                    .into(),
                ),
                (
                    "symbol",
                    SceneSymbolMark {
                        shapes: vec![SymbolShape::from_vega_str("square").unwrap()],
                        x: ScalarOrArray::new_scalar(16.0),
                        y: ScalarOrArray::new_scalar(16.0),
                        size: ScalarOrArray::new_scalar(1024.0),
                        fill: ScalarOrArray::new_scalar(fill),
                        fill_pattern: ScalarOrArray::new_scalar(Some(pattern.clone())),
                        ..Default::default()
                    }
                    .into(),
                ),
            ];
            for (name, host) in hosts {
                let scene = SceneGraph {
                    width: 40.0,
                    height: 40.0,
                    origin: [0.0, 0.0],
                    marks: vec![SceneGroup {
                        origin: [4.0, 4.0],
                        pattern_reference_frame: Some(PatternReferenceFrame {
                            x: 0.0,
                            y: 0.0,
                            width: 32.0,
                            height: 32.0,
                        }),
                        marks: vec![host],
                        ..Default::default()
                    }
                    .into()],
                };
                canvas.set_scene(&scene).unwrap();
                let image = pollster::block_on(canvas.render()).unwrap();
                let gap = image.get_pixel(16, 16).0;
                for (point, covered) in [
                    ((12, 12), operation == PatternLayerOperation::Add),
                    ((16, 12), true),
                    ((12, 16), operation != PatternLayerOperation::Subtract),
                ] {
                    let pixel = image.get_pixel(point.0, point.1).0;
                    assert_eq!(
                        pixel[0] < gap[0] - 20,
                        covered,
                        "{name} {operation:?} at {point:?}: {pixel:?}, gap {gap:?}"
                    );
                }
                assert_eq!(
                    image.get_pixel(1, 1).0,
                    [255; 4],
                    "{name}: pattern escaped host"
                );
                if name == "arc" {
                    assert_eq!(
                        image.get_pixel(20, 20).0,
                        [255; 4],
                        "pattern filled the donut hole"
                    );
                }
            }
        }
    }
    #[test]
    fn patterned_rect_renders_stripe_overlay() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.25,
            },
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 2.0))],
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(4.0),
                y: ScalarOrArray::new_scalar(4.0),
                width: Some(ScalarOrArray::new_scalar(24.0)),
                height: Some(ScalarOrArray::new_scalar(16.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [40.0, 30.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();

        canvas.set_scene(&scene_graph).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        let stripe = image.get_pixel(10, 12).0;
        let gap = image.get_pixel(10, 8).0;

        assert!(
            stripe[0] < gap[0] && stripe[1] < gap[1] && stripe[2] < gap[2],
            "expected stripe pixel to be darker than gap pixel; stripe={stripe:?}, gap={gap:?}"
        );
    }
    #[test]
    fn patterned_rect_clips_diagonal_stripe_overlay_to_host() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.8,
            },
            layers: vec![PatternLayer::Stripe(StripePatternLayer::new(
                45.0, 8.0, 5.0,
            ))],
        };
        let scene_graph = SceneGraph {
            width: 40.0,
            height: 30.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(12.0),
                y: ScalarOrArray::new_scalar(8.0),
                width: Some(ScalarOrArray::new_scalar(16.0)),
                height: Some(ScalarOrArray::new_scalar(12.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(pattern)),
                stroke_width: ScalarOrArray::new_scalar(0.0),
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [40.0, 30.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();

        canvas.set_scene(&scene_graph).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        let leaked_points: Vec<_> = (0..40)
            .flat_map(|x| (0..30).map(move |y| (x, y)))
            .filter(|(x, y)| *x < 10 || *x > 30 || *y < 6 || *y > 22)
            .filter(|(x, y)| image.get_pixel(*x, *y).0 != [255, 255, 255, 255])
            .collect();

        assert_eq!(
            leaked_points.len(),
            0,
            "expected diagonal pattern overlay to be clipped to the rect host; first leaked points: {:?}",
            &leaked_points[..leaked_points.len().min(12)]
        );
    }
    #[test]
    fn patterned_rect_crosshatch_intersection_does_not_accumulate_opacity() {
        assert_crosshatch_intersection_does_not_accumulate_opacity(CanvasConfig::default(), None);
    }
    #[test]
    fn patterned_rect_crosshatch_intersection_does_not_accumulate_opacity_without_msaa() {
        assert_crosshatch_intersection_does_not_accumulate_opacity(
            CanvasConfig {
                sample_count: Some(1),
                ..Default::default()
            },
            Some(1),
        );
    }
    #[test]
    fn patterned_path_with_path_clip_does_not_accumulate_opacity() {
        let scene_graph = SceneGraph {
            width: 32.0,
            height: 32.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: Clip::Path(rect_path(0.0, 0.0, 24.0, 24.0)),
                marks: vec![ScenePathMark {
                    path: ScalarOrArray::new_scalar(notched_host_path()),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
                    fill_pattern: ScalarOrArray::new_scalar(Some(crosshatch_pattern())),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [32.0, 32.0],
                scale: 1.0,
            },
            CanvasConfig {
                sample_count: Some(1),
                ..Default::default()
            },
        ))
        .unwrap();

        canvas.set_scene(&scene_graph).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        assert_crosshatch_pixels(&image);
        assert_eq!(
            image.get_pixel(28, 8).0,
            [255, 255, 255, 255],
            "expected inherited path clip to suppress host pixels outside the clip"
        );
        assert_eq!(
            image.get_pixel(4, 22).0,
            [255, 255, 255, 255],
            "expected non-rectangular host path to suppress pixels inside the clip but outside the host"
        );
    }
    fn assert_crosshatch_intersection_does_not_accumulate_opacity(
        config: CanvasConfig,
        expected_sample_count: Option<u32>,
    ) {
        let scene_graph = SceneGraph {
            width: 32.0,
            height: 32.0,
            origin: [0.0, 0.0],
            marks: vec![SceneRectMark {
                len: 1,
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: Some(ScalarOrArray::new_scalar(32.0)),
                height: Some(ScalarOrArray::new_scalar(32.0)),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 1.0, 1.0, 1.0])),
                fill_pattern: ScalarOrArray::new_scalar(Some(crosshatch_pattern())),
                stroke_width: ScalarOrArray::new_scalar(0.0),
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [32.0, 32.0],
                scale: 1.0,
            },
            config,
        ))
        .unwrap();
        if let Some(expected_sample_count) = expected_sample_count {
            assert_eq!(canvas.sample_count(), expected_sample_count);
        }

        canvas.set_scene(&scene_graph).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        assert_crosshatch_pixels(&image);
    }
    fn crosshatch_pattern() -> PatternFill {
        PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.5,
            },
            layers: vec![
                PatternLayer::Stripe(StripePatternLayer::new(0.0, 8.0, 4.0)),
                PatternLayer::Stripe(StripePatternLayer::new(90.0, 8.0, 4.0)),
            ],
        }
    }
    fn assert_crosshatch_pixels(image: &image::RgbaImage) {
        let intersection = image.get_pixel(8, 8).0;
        let single_stripe = image.get_pixel(4, 8).0;
        let gap = image.get_pixel(4, 4).0;

        for channel in 0..3 {
            let delta = intersection[channel].abs_diff(single_stripe[channel]);
            assert!(
                delta <= 2,
                "expected crosshatch intersection opacity to match a single stripe; \
                 intersection={intersection:?}, single_stripe={single_stripe:?}, gap={gap:?}"
            );
            assert!(
                single_stripe[channel] < gap[channel],
                "expected stripe to be darker than gap; \
                 intersection={intersection:?}, single_stripe={single_stripe:?}, gap={gap:?}"
            );
        }
    }
    fn rect_path(x: f32, y: f32, width: f32, height: f32) -> LyonPath {
        path_from_points(&[
            [x, y],
            [x + width, y],
            [x + width, y + height],
            [x, y + height],
        ])
    }
    fn notched_host_path() -> LyonPath {
        path_from_points(&[
            [0.0, 0.0],
            [32.0, 0.0],
            [32.0, 32.0],
            [18.0, 32.0],
            [18.0, 18.0],
            [0.0, 18.0],
        ])
    }
    fn path_from_points(points: &[[f32; 2]]) -> LyonPath {
        let mut builder = LyonPath::builder();
        builder.begin(point(points[0][0], points[0][1]));
        for point_value in points.iter().skip(1) {
            builder.line_to(point(point_value[0], point_value[1]));
        }
        builder.close();
        builder.build()
    }
}
