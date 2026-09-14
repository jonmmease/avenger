#[cfg(test)]
mod test_image_baselines {
    use avenger_color::ColorOrGradient;
    use avenger_common::{
        canvas::CanvasDimensions,
        types::{ImageAlign, ImageBaseline, SymbolShape},
        value::ScalarOrArray,
    };
    use avenger_image::{ImageResourceResolver, ImageResourceState, RgbaImage};
    use avenger_resource::ResourceKey;
    use avenger_scenegraph::{
        marks::group::{Clip, SceneGroup},
        marks::image::{
            SceneImageMark, SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy,
        },
        marks::path::ScenePathMark,
        marks::pattern::{PatternAnchor, PatternFill, PatternInk, PatternLayer, StripeLayer},
        marks::rect::SceneRectMark,
        marks::symbol::SceneSymbolMark,
        scene_graph::SceneGraph,
    };
    use avenger_vega_scenegraph::scene_graph::VegaSceneGraph;
    use avenger_wgpu::{
        canvas::{Canvas, CanvasConfig, PngCanvas},
        error::AvengerWgpuError,
        image_resources::{WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuMissingImagePolicy},
    };
    use dssim::Dssim;
    use lyon::{geom::point, path::Path as LyonPath};
    use rstest::rstest;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// The first of the families that is installed, as the original Vega comparison harness
    /// chose them, or none.
    fn installed_family(families: &[&str]) -> Option<String> {
        use avenger_typst_label::{
            bundled_font_options, EngineOptions, FontOptions, LabelEngine, MissingFontPolicy,
            TextStyle,
        };
        static ENGINE: std::sync::OnceLock<LabelEngine> = std::sync::OnceLock::new();
        let engine = ENGINE.get_or_init(|| {
            LabelEngine::new(EngineOptions {
                fonts: FontOptions {
                    missing_font: MissingFontPolicy::Error,
                    ..bundled_font_options()
                },
            })
        });
        families
            .iter()
            .find(|family| {
                let style = TextStyle {
                    font_family: family.to_string(),
                    ..Default::default()
                };
                engine.font_metrics(&style).is_ok()
            })
            .map(|family| family.to_string())
    }

    /// The engine with the fonts the original Vega comparison harness chose: the first installed
    /// family of each list, else the bundled one, and the Vega test fonts.
    fn vega_text_engine() -> avenger_typst_label::LabelEngine {
        static ENGINE: std::sync::OnceLock<avenger_typst_label::LabelEngine> =
            std::sync::OnceLock::new();
        ENGINE
            .get_or_init(|| {
                let defaults = avenger_typst_label::bundled_font_options();
                let fonts = avenger_typst_label::FontOptions {
                    extra_font_dirs: vec![Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("../avenger-vega-test-data/fonts")],
                    default_sans_serif_family: installed_family(&[
                        "Helvetica",
                        "Arial",
                        "Liberation Sans",
                    ])
                    .or(defaults.default_sans_serif_family.clone()),
                    default_monospace_family: installed_family(&[
                        "Courier New",
                        "Courier",
                        "Liberation Mono",
                    ])
                    .or(defaults.default_monospace_family.clone()),
                    ..defaults
                };
                avenger_typst_label::LabelEngine::new(avenger_typst_label::EngineOptions { fonts })
            })
            .clone()
    }

    // Reference updates and text/subpixel-rectangle tolerance adjustments are
    // documented in docs/rendering-validation.md.
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
        case("line", "stocks-legend", 0.003),
        case("line", "simple_dashed", 0.0005),
        case("line", "stocks_dashed", 0.002),
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
        case("vl-convert", "float_font_size", 0.01),
        case("vl-convert", "font_with_quotes", 0.02),
        case("vl-convert", "line_with_log_scale", 0.02),
        case("vl-convert", "long_legend_label", 0.01),
        case("vl-convert", "lookup_urls", 0.01),
        case("vl-convert", "numeric_font_weight", 0.02),
        case("vl-convert", "quakes_initial_selection", 0.01),
        case("vl-convert", "remote_images", 0.01),
        case("vl-convert", "seattle-weather", 0.01),
        case("vl-convert", "stocks_locale", 0.01),
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
            CanvasConfig::default(),
        ))
        .unwrap();
        png_canvas
            .set_scene(&scene_graph, &vega_text_engine())
            .unwrap();
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
        canvas
            .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
            .unwrap();
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

    /// A symbol mark draws the same below and at the instancing threshold: a thick translucent
    /// stroke blends once wherever it covers, on the multi-mark path (99 symbols) and the
    /// instanced path (100).
    #[test]
    fn thick_translucent_strokes_match_across_instancing_threshold() {
        let render = |len: u32| {
            let mark = SceneSymbolMark {
                len,
                shapes: vec![SymbolShape::from_vega_str("triangle-up").unwrap()],
                x: ScalarOrArray::new_array(
                    (0..len).map(|i| 20.0 + (i % 10) as f32 * 40.0).collect(),
                ),
                y: ScalarOrArray::new_array(
                    (0..len).map(|i| 20.0 + (i / 10) as f32 * 40.0).collect(),
                ),
                size: ScalarOrArray::new_scalar(64.0),
                fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 0.0])),
                stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0, 0.0, 0.0, 0.5])),
                stroke_width: Some(8.0),
                ..Default::default()
            };
            let scene = SceneGraph {
                width: 400.0,
                height: 400.0,
                origin: [0.0, 0.0],
                marks: vec![mark.into()],
            };
            let mut canvas = pollster::block_on(PngCanvas::new(
                CanvasDimensions {
                    size: [scene.width, scene.height],
                    scale: 1.0,
                },
                CanvasConfig::default(),
            ))
            .unwrap();
            canvas
                .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
                .unwrap();
            pollster::block_on(canvas.render()).unwrap()
        };
        let multi = render(99);
        let instanced = render(100);
        for (x, y, pixel) in multi.enumerate_pixels() {
            // The cells of the first 99 symbols, which both marks draw.
            if x / 40 + y / 40 * 10 < 99 {
                assert_eq!(pixel, instanced.get_pixel(x, y), "pixel ({x}, {y})");
            }
        }
    }

    fn render_scene(scene: &SceneGraph) -> image::RgbaImage {
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [scene.width, scene.height],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();
        canvas
            .set_scene(scene, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        pollster::block_on(canvas.render()).unwrap()
    }

    fn blue_rect() -> SceneRectMark {
        SceneRectMark {
            x: ScalarOrArray::new_scalar(0.0),
            y: ScalarOrArray::new_scalar(0.0),
            width: Some(ScalarOrArray::new_scalar(100.0)),
            height: Some(ScalarOrArray::new_scalar(50.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
            clip: true,
            ..Default::default()
        }
    }

    /// A clip path with no area hides its marks.
    #[test]
    fn zero_area_clip_path_hides_its_marks() {
        let mut clip = lyon::path::Path::builder();
        clip.begin(lyon::math::point(20.0, 20.0));
        clip.line_to(lyon::math::point(20.0, 20.0));
        clip.line_to(lyon::math::point(20.0, 20.0));
        clip.close();
        let image = render_scene(&SceneGraph {
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip: Clip::Path(clip.build()),
                marks: vec![blue_rect().into()],
                ..Default::default()
            }
            .into()],
        });
        assert!(image.pixels().all(|pixel| pixel.0 == [255, 255, 255, 255]));
    }

    /// A clip rectangle that starts left of the canvas clips at its right edge, for marks drawn
    /// together (a rect) and instanced (100 symbols).
    #[test]
    fn clip_rect_left_of_the_canvas_clips_at_its_right_edge() {
        let clip = Clip::Rect {
            x: -50.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
        };
        let symbols = SceneSymbolMark {
            len: 100,
            x: ScalarOrArray::new_array((0..100).map(|i| i as f32).collect()),
            y: ScalarOrArray::new_scalar(75.0),
            size: ScalarOrArray::new_scalar(100.0),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 1.0, 1.0])),
            clip: true,
            ..Default::default()
        };
        let image = render_scene(&SceneGraph {
            width: 100.0,
            height: 100.0,
            origin: [0.0, 0.0],
            marks: vec![SceneGroup {
                clip,
                marks: vec![blue_rect().into(), symbols.into()],
                ..Default::default()
            }
            .into()],
        });
        for (x, y, pixel) in image.enumerate_pixels() {
            let blue = pixel.0[0] < 128;
            if x >= 50 {
                assert!(!blue, "pixel ({x}, {y}) is past the clip");
            } else if y < 50 || (70..80).contains(&y) {
                assert!(blue, "pixel ({x}, {y}) is inside the clip");
            }
        }
    }

    fn render_pattern_mark(
        mark: avenger_scenegraph::marks::mark::SceneMark,
        samples: u32,
    ) -> image::RgbaImage {
        let scene = SceneGraph {
            width: 200.0,
            height: 100.0,
            origin: [0.0; 2],
            marks: vec![mark],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [200.0, 100.0],
                scale: 1.0,
            },
            CanvasConfig {
                sample_count: Some(samples),
                ..Default::default()
            },
        ))
        .unwrap();
        canvas
            .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        pollster::block_on(canvas.render()).unwrap()
    }

    #[test]
    fn pattern_does_not_change_compound_host_fill() {
        use avenger_common::types::PathTransform;
        let mut builder = LyonPath::builder();
        for (min, max) in [(10.0, 90.0), (30.0, 70.0)] {
            builder.begin(point(min, min));
            builder.line_to(point(max, min));
            builder.line_to(point(max, max));
            builder.line_to(point(min, max));
            builder.close();
        }
        let mark = ScenePathMark {
            len: 2,
            path: builder.build().into(),
            transform: vec![
                PathTransform::identity(),
                PathTransform::translation(100.0, 0.0),
            ]
            .into(),
            fill: ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0]).into(),
            ..Default::default()
        };
        let reference = render_pattern_mark(mark.clone().into(), 1);
        assert_eq!(reference.get_pixel(50, 50).0, [255; 4]);
        assert_eq!(reference.get_pixel(20, 50).0, [0, 0, 0, 255]);
        let empty = PatternFill {
            anchor: PatternAnchor::Mark,
            ..Default::default()
        };
        let transparent = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.0,
            },
            layers: vec![PatternLayer::from(StripeLayer::new(0.0, 8.0, 2.0))],
        };
        for fill_pattern in [
            Some(empty.clone()).into(),
            Some(transparent).into(),
            vec![None, Some(empty)].into(),
        ] {
            let patterned = ScenePathMark {
                fill_pattern,
                ..mark.clone()
            };
            assert_eq!(
                render_pattern_mark(patterned.into(), 1).as_raw(),
                reference.as_raw()
            );
        }
    }

    #[test]
    fn pattern_symbols_union_instances_before_operations() {
        use avenger_scenegraph::marks::pattern::{
            PatternLayerOperation, PatternSymbol, PatternSymbolShape, SymbolLattice2d, SymbolLayer,
            SymbolPaint,
        };
        for samples in [1, 4] {
            for (paint, spacing, ink_pixel, gap_pixel) in [
                (SymbolPaint::Filled, 30.0, [65, 50], [50, 20]),
                (
                    SymbolPaint::Open { stroke_width: 8.0 },
                    38.0,
                    [69, 50],
                    [50, 50],
                ),
            ] {
                for operation in [
                    PatternLayerOperation::Add,
                    PatternLayerOperation::Subtract,
                    PatternLayerOperation::Xor,
                ] {
                    let mut layers = Vec::new();
                    if operation != PatternLayerOperation::Add {
                        layers.push(StripeLayer::new(0.0, 1000.0, 1000.0).into());
                    }
                    let symbols = SymbolLayer {
                        lattice: SymbolLattice2d {
                            u_spacing: spacing,
                            v_spacing: 100.0,
                            u_angle: 0.0,
                            v_angle: 90.0,
                            u_phase: 50.0,
                            v_phase: 50.0,
                        },
                        // Squares 40 px wide.
                        symbol: PatternSymbol {
                            shape: PatternSymbolShape::Square,
                            size: 1600.0,
                            rotation: 0.0,
                        },
                        paint: paint.clone(),
                    };
                    layers.push(PatternLayer::from(symbols).with_operation(operation));
                    let mark = SceneRectMark {
                        width: Some(100.0.into()),
                        height: Some(100.0.into()),
                        fill: ColorOrGradient::Color([1.0; 4]).into(),
                        fill_pattern: Some(PatternFill {
                            anchor: PatternAnchor::Mark,
                            ink: PatternInk::Solid {
                                color: [0.0, 0.0, 0.0, 1.0],
                                opacity: 0.5,
                            },
                            layers,
                        })
                        .into(),
                        ..Default::default()
                    };
                    let image = render_pattern_mark(mark.into(), samples);
                    for (pixel, covered) in [(ink_pixel, true), (gap_pixel, false)] {
                        let covered = covered == (operation == PatternLayerOperation::Add);
                        let value = image.get_pixel(pixel[0], pixel[1]).0[0];
                        let expected = if covered { 127 } else { 255 };
                        assert!(value.abs_diff(expected) <= 1,
                            "{operation:?}, {paint:?}, spacing={spacing}, samples={samples}, pixel={pixel:?}: {value} != {expected}");
                    }
                }
            }
        }
    }

    #[test]
    fn pattern_layer_operations_render_on_every_host_mark() {
        use avenger_scenegraph::marks::{
            arc::SceneArcMark, area::SceneAreaMark, mark::SceneMark, pattern::PatternLayerOperation,
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
            let vertical =
                PatternLayer::from(StripeLayer::new(90.0, 8.0, 4.0)).with_operation(operation);
            let pattern = PatternFill {
                anchor: PatternAnchor::Group,
                ink: PatternInk::Solid {
                    color: [0.0, 0.0, 0.0, 1.0],
                    opacity: 0.5,
                },
                layers: vec![StripeLayer::new(0.0, 8.0, 4.0).into(), vertical],
            };
            let fill = ColorOrGradient::Color([0.8, 0.8, 1.0, 1.0]);
            let hosts: Vec<(&str, SceneMark)> = vec![
                (
                    "rect",
                    SceneRectMark {
                        width: Some(ScalarOrArray::new_scalar(32.0)),
                        height: Some(ScalarOrArray::new_scalar(32.0)),
                        fill: ScalarOrArray::new_scalar(fill.clone()),
                        fill_pattern: pattern.clone().into(),
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
                        fill_pattern: pattern.clone().into(),
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
                        fill_pattern: pattern.clone().into(),
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
                        fill_pattern: pattern.clone().into(),
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
                        pattern_origin: Some([0.0, 0.0]),
                        marks: vec![host],
                        ..Default::default()
                    }
                    .into()],
                };
                canvas
                    .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
                    .unwrap();
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
    /// Patterned items share a render pass, so each overlay must start from a clear stencil: the
    /// second rect covers the first one's stripes and shows only its own.
    #[test]
    fn overlapping_patterned_items_show_only_their_own_patterns() {
        let stripes = |spacing: f32| PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 1.0,
            },
            layers: vec![PatternLayer::from(StripeLayer::new(0.0, spacing, 2.0))],
        };
        let mark = SceneRectMark {
            len: 2,
            x: ScalarOrArray::new_array(vec![10.0, 50.0]),
            y: ScalarOrArray::new_scalar(10.0),
            width: Some(ScalarOrArray::new_scalar(80.0)),
            height: Some(ScalarOrArray::new_scalar(80.0)),
            fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([1.0; 4])),
            fill_pattern: vec![Some(stripes(4.0)), Some(stripes(40.0))].into(),
            ..Default::default()
        };
        for samples in [1, 4] {
            let image = render_pattern_mark(mark.clone().into(), samples);
            // The first rect's stripe at y = 14, outside the second rect and under it.
            assert_eq!(image.get_pixel(30, 14).0[0], 0, "samples={samples}");
            assert_eq!(image.get_pixel(70, 14).0[0], 255, "samples={samples}");
            // The second rect's stripe at y = 50.
            assert_eq!(image.get_pixel(70, 50).0[0], 0, "samples={samples}");
        }
    }

    /// A symbol legend's patterns anchor to each swatch, so the legend draws the same at a second,
    /// fractional origin. The image is compared with `tests/baselines/legend-patterns.png` and
    /// written to the ignored `tests/output`, and copying it over the baseline accepts a change.
    #[test]
    fn symbol_legend_patterns_anchor_to_their_swatches() {
        use avenger_guides::legend::symbol::{make_symbol_legend, SymbolLegendConfig};
        use avenger_scenegraph::marks::pattern::{
            PatternSymbol, PatternSymbolShape, StripeDash, SymbolLattice2d, SymbolLayer,
            SymbolPaint,
        };
        use avenger_typst_label::{bundled_font_options, EngineOptions, FontOptions, LabelEngine};

        let text_engine = LabelEngine::new(EngineOptions {
            fonts: FontOptions {
                load_system_fonts: false,
                ..bundled_font_options()
            },
        });
        let pattern = |layers: Vec<PatternLayer>| {
            Some(PatternFill {
                layers,
                ..Default::default()
            })
        };
        let dots = SymbolLayer {
            lattice: SymbolLattice2d {
                u_spacing: 6.0,
                u_angle: 0.0,
                v_spacing: 6.0,
                v_angle: 60.0,
                u_phase: 0.0,
                v_phase: 0.0,
            },
            symbol: PatternSymbol {
                shape: PatternSymbolShape::Circle,
                size: 9.0,
                rotation: 0.0,
            },
            paint: SymbolPaint::Filled,
        };
        let mut dashes = StripeLayer::new(0.0, 5.0, 1.5);
        dashes.dash = Some(StripeDash {
            length: 4.0,
            gap: 2.0,
            phase: 0.0,
        });
        let legend = make_symbol_legend(
            &SymbolLegendConfig {
                title: Some("Patterns".to_string()),
                text: ScalarOrArray::new_array(
                    ["Stripes", "Dots", "Crosshatch", "Dashes"]
                        .map(String::from)
                        .to_vec(),
                ),
                shape: ScalarOrArray::new_array(
                    ["square", "circle", "triangle-up", "diamond"]
                        .map(|shape| SymbolShape::from_vega_str(shape).unwrap())
                        .to_vec(),
                ),
                size: ScalarOrArray::new_scalar(400.0),
                fill: ScalarOrArray::new_array(
                    [
                        [0.27, 0.51, 0.71, 1.0],
                        [0.99, 0.55, 0.24, 1.0],
                        [0.17, 0.63, 0.17, 1.0],
                        [0.92, 0.92, 0.92, 1.0],
                    ]
                    .map(ColorOrGradient::Color)
                    .to_vec(),
                ),
                fill_pattern: vec![
                    pattern(vec![StripeLayer::new(45.0, 5.0, 1.5).into()]),
                    pattern(vec![dots.into()]),
                    pattern(vec![
                        StripeLayer::new(45.0, 5.0, 1.0).into(),
                        StripeLayer::new(-45.0, 5.0, 1.0).into(),
                    ]),
                    pattern(vec![dashes.into()]),
                ]
                .into(),
                ..Default::default()
            },
            &text_engine,
        )
        .unwrap()
        .group;
        // The second legend is 301 by 13 device pixels from the first.
        let at = |origin| -> avenger_scenegraph::marks::mark::SceneMark {
            SceneGroup {
                origin,
                ..legend.clone()
            }
            .into()
        };
        let scene = SceneGraph {
            width: 310.0,
            height: 150.0,
            origin: [0.0; 2],
            marks: vec![at([8.0, 8.0]), at([158.5, 14.5])],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [scene.width, scene.height],
                scale: 2.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();
        canvas.set_scene(&scene, &text_engine).unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        for y in 0..287 {
            for x in 0..300 {
                let first = image.get_pixel(x, y).0;
                let second = image.get_pixel(x + 301, y + 13).0;
                assert!(
                    first.iter().zip(second).all(|(a, b)| a.abs_diff(b) <= 2),
                    "pixel ({x}, {y}): {first:?} != {second:?}"
                );
            }
        }

        let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let output = tests.join("output/legend-patterns.png");
        fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(&output).unwrap();
        let baseline = tests.join("baselines/legend-patterns.png");
        let attr = Dssim::new();
        let expected = dssim::load_image(&attr, &baseline).expect("the baseline exists");
        let actual = dssim::load_image(&attr, &output).unwrap();
        let (diff, _) = attr.compare(&expected, actual);
        assert!(
            diff < 0.0001,
            "DSSIM {diff}; to accept {}, copy it to {}",
            output.display(),
            baseline.display()
        );
    }

    /// The shader paints the pattern regions that the CPU pattern geometry describes: pixels more
    /// than 1.5 device pixels from every edge of the CPU geometry match its coverage exactly.
    #[test]
    fn patterns_match_cpu_geometry_away_from_edges() {
        use avenger_scenegraph::{
            marks::pattern::{
                PatternLayerOperation, PatternSymbol, PatternSymbolShape, StripeDash,
                SymbolLattice2d, SymbolLayer, SymbolPaint,
            },
            pattern_geometry::{
                build_layered_pattern_geometry, PatternCoveragePrimitive, PatternRect,
                PatternRenderContext,
            },
        };
        use lyon::{
            algorithms::{aabb::bounding_box, hit_test::hit_test_path},
            lyon_tessellation::{
                geometry_builder::simple_builder, StrokeOptions, StrokeTessellator, VertexBuffers,
            },
            path::iterator::PathIterator,
            path::FillRule,
        };

        let symbols = |shape, size: f32, spacing: f32, paint, anchor| PatternFill {
            anchor,
            layers: vec![SymbolLayer {
                lattice: SymbolLattice2d {
                    u_spacing: spacing,
                    u_angle: 10.0,
                    v_spacing: spacing * 1.15,
                    v_angle: 75.0,
                    u_phase: 3.0,
                    v_phase: 1.0,
                },
                symbol: PatternSymbol {
                    shape,
                    size,
                    rotation: 20.0,
                },
                paint,
            }
            .into()],
            ..Default::default()
        };
        let mut patterns = Vec::new();
        for shape in PatternSymbolShape::ALL {
            patterns.push(symbols(
                shape,
                100.0,
                15.0,
                SymbolPaint::Filled,
                PatternAnchor::Mark,
            ));
            // Open symbols are large enough that each keeps pixels clear of its corners' miters.
            patterns.push(symbols(
                shape,
                900.0,
                40.0,
                SymbolPaint::Open { stroke_width: 3.0 },
                PatternAnchor::Group,
            ));
        }
        let mut dashed = StripeLayer::new(-20.0, 11.0, 4.0);
        dashed.phase = 2.0;
        dashed.dash = Some(StripeDash {
            length: 7.0,
            gap: 5.0,
            phase: 2.0,
        });
        patterns.push(PatternFill {
            anchor: PatternAnchor::Scene,
            layers: vec![dashed.into()],
            ..Default::default()
        });
        // A lattice whose steps are 5 degrees apart, which the shader reduces.
        patterns.push(PatternFill {
            layers: vec![SymbolLayer {
                lattice: SymbolLattice2d {
                    u_spacing: 13.0,
                    u_angle: 10.0,
                    v_spacing: 70.0,
                    v_angle: 15.0,
                    u_phase: 0.0,
                    v_phase: 0.0,
                },
                symbol: PatternSymbol {
                    shape: PatternSymbolShape::Circle,
                    size: 36.0,
                    rotation: 0.0,
                },
                paint: SymbolPaint::Filled,
            }
            .into()],
            ..Default::default()
        });
        // Symbols that overlap their neighbors, and every operation.
        patterns.push(symbols(
            PatternSymbolShape::Square,
            900.0,
            15.0,
            SymbolPaint::Open { stroke_width: 4.0 },
            PatternAnchor::Mark,
        ));
        patterns.push(PatternFill {
            layers: vec![
                StripeLayer::new(30.0, 12.0, 5.0).into(),
                PatternLayer::from(StripeLayer::new(-60.0, 14.0, 5.0))
                    .with_operation(PatternLayerOperation::Xor),
                symbols(
                    PatternSymbolShape::Circle,
                    100.0,
                    15.0,
                    SymbolPaint::Filled,
                    PatternAnchor::Mark,
                )
                .layers
                .remove(0)
                .with_operation(PatternLayerOperation::Subtract),
            ],
            ..Default::default()
        });

        let ink = [0.1, 0.05, 0.0, 1.0];
        for pattern in &mut patterns {
            pattern.ink = PatternInk::Solid {
                color: ink,
                opacity: 0.8,
            };
        }
        let fill = [0.3, 0.6, 0.8, 1.0];
        let (columns, cell) = (6, [70.0, 60.0]);
        let rows = patterns.len().div_ceil(columns);
        let hosts: Vec<PatternRect> = (0..patterns.len())
            .map(|i| {
                let (column, row) = ((i % columns) as f32, (i / columns) as f32);
                PatternRect::new(4.0 + column * cell[0], 4.0 + row * cell[1], 64.0, 54.0)
            })
            .collect();
        let size = [8.0 + columns as f32 * cell[0], 8.0 + rows as f32 * cell[1]];
        let group_origin = [5.0, 7.0];
        let scene = SceneGraph {
            width: size[0],
            height: size[1],
            origin: [0.0; 2],
            marks: vec![SceneGroup {
                pattern_origin: Some(group_origin),
                marks: vec![SceneRectMark {
                    len: hosts.len() as u32,
                    x: ScalarOrArray::new_array(hosts.iter().map(|host| host.x).collect()),
                    y: ScalarOrArray::new_array(hosts.iter().map(|host| host.y).collect()),
                    width: Some(ScalarOrArray::new_scalar(64.0)),
                    height: Some(ScalarOrArray::new_scalar(54.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(fill)),
                    fill_pattern: patterns
                        .iter()
                        .cloned()
                        .map(Some)
                        .collect::<Vec<_>>()
                        .into(),
                    ..Default::default()
                }
                .into()],
                ..Default::default()
            }
            .into()],
        };

        // Each host's layers, as their primitives' bounds, outlines, and stroke triangles.
        struct Primitive {
            bounds: lyon::math::Box2D,
            path: LyonPath,
            /// The outline's segments: a filled path's edges, or a stroke's centerline.
            segments: Vec<(lyon::math::Point, lyon::math::Point)>,
            /// The outline's vertices where it turns by more than 15 degrees, with the sine of
            /// half the angle between their edges.
            corners: Vec<(lyon::math::Point, f32)>,
            /// A stroke's half width and its outline as triangles.
            stroke: Option<(f32, Vec<[lyon::math::Point; 3]>)>,
        }
        let host_fill = ColorOrGradient::Color(fill);
        let regions: Vec<Vec<(PatternLayerOperation, Vec<Primitive>)>> = patterns
            .iter()
            .zip(&hosts)
            .map(|(pattern, host)| {
                let context = PatternRenderContext {
                    group_origin,
                    host_bounds: *host,
                    host_fill: &host_fill,
                    gradients: &[],
                };
                let geometry = build_layered_pattern_geometry(pattern, &context)
                    .unwrap()
                    .unwrap();
                geometry
                    .layers
                    .into_iter()
                    .map(|layer| {
                        let primitives = layer
                            .primitives
                            .into_iter()
                            .map(|primitive| {
                                let (path, stroke_width) = match primitive {
                                    PatternCoveragePrimitive::Filled(path) => (path, None),
                                    PatternCoveragePrimitive::Stroked { path, stroke_width } => {
                                        (path, Some(stroke_width))
                                    }
                                };
                                // Closed contours, as their vertices.
                                let mut contours: Vec<Vec<lyon::math::Point>> = Vec::new();
                                for event in path.iter().flattened(0.01) {
                                    match event {
                                        lyon::path::Event::Begin { at } => contours.push(vec![at]),
                                        lyon::path::Event::Line { to, .. } => {
                                            contours.last_mut().unwrap().push(to)
                                        }
                                        _ => {}
                                    }
                                }
                                let mut segments = Vec::new();
                                let mut corners = Vec::new();
                                for mut contour in contours {
                                    if contour.len() > 1 && contour.first() == contour.last() {
                                        contour.pop();
                                    }
                                    let n = contour.len();
                                    for k in 0..n {
                                        let (before, at, after) = (
                                            contour[(k + n - 1) % n],
                                            contour[k],
                                            contour[(k + 1) % n],
                                        );
                                        segments.push((at, after));
                                        let turn = (at - before).angle_to(after - at).radians.abs();
                                        if turn > 15f32.to_radians() {
                                            let angle =
                                                (before - at).angle_to(after - at).radians.abs();
                                            corners.push((at, (angle / 2.0).sin()));
                                        }
                                    }
                                }
                                let stroke = stroke_width.map(|width| {
                                    let mut buffers: VertexBuffers<lyon::math::Point, u16> =
                                        VertexBuffers::new();
                                    StrokeTessellator::new()
                                        .tessellate_path(
                                            &path,
                                            &StrokeOptions::default()
                                                .with_line_width(width)
                                                .with_tolerance(0.01),
                                            &mut simple_builder(&mut buffers),
                                        )
                                        .unwrap();
                                    let triangles = buffers
                                        .indices
                                        .chunks(3)
                                        .map(|t| {
                                            [t[0], t[1], t[2]].map(|i| buffers.vertices[i as usize])
                                        })
                                        .collect();
                                    (width / 2.0, triangles)
                                });
                                let pad = stroke_width.unwrap_or(0.0) * 2.0 + 2.0;
                                Primitive {
                                    bounds: bounding_box(&path).inflate(pad, pad),
                                    path,
                                    segments,
                                    corners,
                                    stroke,
                                }
                            })
                            .collect();
                        (layer.operation, primitives)
                    })
                    .collect()
            })
            .collect();
        let segment_distance =
            |p: lyon::math::Point, (a, b): &(lyon::math::Point, lyon::math::Point)| {
                let ab = *b - *a;
                let t = ((p - *a).dot(ab) / ab.square_length().max(1e-12)).clamp(0.0, 1.0);
                (p - (*a + ab * t)).length()
            };
        let in_triangle = |p: lyon::math::Point, t: &[lyon::math::Point; 3]| {
            let side = |a: lyon::math::Point, b: lyon::math::Point| (b - a).cross(p - a);
            let sides = [side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0])];
            sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
        };
        // Whether a point is in the pattern region of a host, or `None` when an edge of a
        // primitive is within `margin` of it. A stroke's edge near a corner of its centerline can
        // be a miter, so the corners count as edges as far as their miters reach.
        let covered = |cell: usize, p: lyon::math::Point, margin: f32| -> Option<bool> {
            let mut covered = false;
            for (operation, primitives) in &regions[cell] {
                let mut inside = false;
                for primitive in primitives
                    .iter()
                    .filter(|primitive| primitive.bounds.contains(p))
                {
                    let distance = primitive
                        .segments
                        .iter()
                        .map(|segment| segment_distance(p, segment))
                        .fold(f32::INFINITY, f32::min);
                    let hit = match &primitive.stroke {
                        None => {
                            if distance < margin {
                                return None;
                            }
                            hit_test_path(&p, primitive.path.iter(), FillRule::EvenOdd, 0.01)
                        }
                        Some((half_width, triangles)) => {
                            let near_corner = primitive.corners.iter().any(|(corner, sine)| {
                                (p - *corner).length() < half_width / sine + margin
                            });
                            if (distance - half_width).abs() < margin || near_corner {
                                return None;
                            }
                            triangles.iter().any(|t| in_triangle(p, t))
                        }
                    };
                    inside |= hit;
                }
                covered = match operation {
                    PatternLayerOperation::Add => covered || inside,
                    PatternLayerOperation::Subtract => covered && !inside,
                    PatternLayerOperation::Xor => covered != inside,
                };
            }
            Some(covered)
        };

        let to_u8 = |value: f32| (value * 255.0).round() as i32;
        let painted = (0..3)
            .map(|c| to_u8(ink[c] * 0.8 + fill[c] * 0.2))
            .collect::<Vec<_>>();
        let unpainted = (0..3).map(|c| to_u8(fill[c])).collect::<Vec<_>>();
        // The pixels checked in each host's pattern region and outside it.
        let mut checked = vec![[0; 2]; hosts.len()];
        for scale in [1.0_f32, 2.0] {
            for samples in [1, 4] {
                let mut canvas = pollster::block_on(PngCanvas::new(
                    CanvasDimensions { size, scale },
                    CanvasConfig {
                        sample_count: Some(samples),
                        ..Default::default()
                    },
                ))
                .unwrap();
                canvas
                    .set_scene(&scene, &avenger_typst_label::bundled_label_engine())
                    .unwrap();
                let image = pollster::block_on(canvas.render()).unwrap();
                let margin = 1.5 / scale;
                for (cell, host) in hosts.iter().enumerate() {
                    let x_range = ((host.x + 1.0) * scale) as u32
                        ..((host.x + host.width - 1.0) * scale) as u32;
                    for px in x_range {
                        for py in ((host.y + 1.0) * scale) as u32
                            ..((host.y + host.height - 1.0) * scale) as u32
                        {
                            let center =
                                point((px as f32 + 0.5) / scale, (py as f32 + 0.5) / scale);
                            let Some(inside) = covered(cell, center, margin) else {
                                continue;
                            };
                            checked[cell][usize::from(inside)] += 1;
                            let expected = if inside { &painted } else { &unpainted };
                            let actual = image.get_pixel(px, py).0;
                            assert!(
                                (0..3).all(|c| (actual[c] as i32 - expected[c]).abs() <= 2),
                                "pattern {cell} at scale {scale} with {samples} samples: pixel \
                                 ({px}, {py}) is {actual:?}, expected {expected:?}"
                            );
                        }
                    }
                }
            }
        }
        for (cell, [outside, inside]) in checked.into_iter().enumerate() {
            assert!(
                outside >= 30 && inside >= 30,
                "pattern {cell} checked {inside} pixels inside and {outside} outside"
            );
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
            layers: vec![PatternLayer::from(StripeLayer::new(0.0, 8.0, 2.0))],
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
                fill_pattern: pattern.into(),
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

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();
        let stripe = image.get_pixel(10, 12).0;
        let gap = image.get_pixel(10, 8).0;

        assert!(
            stripe[0] < gap[0] && stripe[1] < gap[1] && stripe[2] < gap[2],
            "expected stripe pixel to be darker than gap pixel; stripe={stripe:?}, gap={gap:?}"
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
        reused
            .set_scene(&first, &avenger_typst_label::bundled_label_engine())
            .expect("install first scene");
        pollster::block_on(reused.render()).expect("render first scene");
        reused
            .set_scene(&moved, &avenger_typst_label::bundled_label_engine())
            .expect("install moved scene");
        let reused_image = pollster::block_on(reused.render()).expect("render moved scene");

        let mut fresh = pollster::block_on(PngCanvas::new(dimensions, CanvasConfig::default()))
            .expect("fresh canvas");
        fresh
            .set_scene(&moved, &avenger_typst_label::bundled_label_engine())
            .expect("install fresh moved scene");
        let fresh_image = pollster::block_on(fresh.render()).expect("render fresh moved scene");

        assert!(
            reused_image.as_raw() == fresh_image.as_raw(),
            "instanced renderer reuse must not retain the previous group origin"
        );
    }

    #[test]
    fn image_smooth_false_uses_nearest_sampling() {
        let scene_graph = SceneGraph {
            width: 20.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(SceneImageSource::inline(RgbaImage {
                    width: 2,
                    height: 2,
                    data: vec![
                        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                    ],
                })),
                x: ScalarOrArray::new_scalar(2.0),
                y: ScalarOrArray::new_scalar(2.0),
                width: ScalarOrArray::new_scalar(16.0),
                height: ScalarOrArray::new_scalar(16.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [20.0, 20.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        assert_eq!(image.get_pixel(5, 5).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(14, 5).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(5, 14).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(14, 14).0, [255, 255, 255, 255]);
    }

    #[test]
    fn warped_image_mark_renders_sheared_mesh() {
        use avenger_scenegraph::marks::warped_image::SceneWarpedImageMark;

        // 2x2 checker (red, green / blue, white) mapped onto a
        // parallelogram: quad vertices sheared +4px in x from top to bottom.
        let scene_graph = SceneGraph {
            width: 24.0,
            height: 20.0,
            origin: [0.0, 0.0],
            marks: vec![SceneWarpedImageMark {
                smooth: false,
                image: SceneImageSource::inline(RgbaImage {
                    width: 2,
                    height: 2,
                    data: vec![
                        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
                    ],
                }),
                positions: vec![[2.0, 2.0], [18.0, 2.0], [22.0, 18.0], [6.0, 18.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                indices: vec![0, 1, 2, 0, 2, 3],
                ..Default::default()
            }
            .into()],
        };
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [24.0, 20.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        // For a parallelogram the triangle interpolation is affine:
        // p(u, v) = a + u * (b - a) + v * (d - a).
        assert_eq!(image.get_pixel(7, 6).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(15, 6).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(9, 14).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(17, 14).0, [255, 255, 255, 255]);
        // Outside the sheared quad (upper-right corner region) shows the
        // canvas background, not the nearest texture quadrant (green).
        assert_ne!(image.get_pixel(21, 4).0, [0, 255, 0, 255]);
    }

    #[test]
    fn patterned_rect_clips_diagonal_stripe_overlay_to_host() {
        let pattern = PatternFill {
            anchor: PatternAnchor::Mark,
            ink: PatternInk::Solid {
                color: [0.0, 0.0, 0.0, 1.0],
                opacity: 0.8,
            },
            layers: vec![PatternLayer::from(StripeLayer::new(45.0, 8.0, 5.0))],
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
                fill_pattern: pattern.into(),
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

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
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
                    fill_pattern: crosshatch_pattern().into(),
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

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
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
                fill_pattern: crosshatch_pattern().into(),
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

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
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
                PatternLayer::from(StripeLayer::new(0.0, 8.0, 4.0)),
                PatternLayer::from(StripeLayer::new(90.0, 8.0, 4.0)),
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

    #[test]
    fn resource_image_pending_draws_placeholder() {
        let key = ResourceKey::new("tile/0/0/0");
        let resolver = Arc::new(FakeImageResolver::new(ImageResourceState::Pending));
        let mut canvas = resource_image_canvas(resolver.clone());
        let scene_graph = resource_image_scene_graph_with_policy(
            key.clone(),
            SceneImageUnavailablePolicy::RendererDefault,
        );

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        assert_eq!(canvas.image_resource_status().pending, vec![key]);
        assert_eq!(image.get_pixel(4, 4).0, [10, 20, 30, 255]);
    }

    #[test]
    fn resource_image_pending_skip_policy_suppresses_placeholder() {
        let key = ResourceKey::new("tile/0/0/0");
        let resolver = Arc::new(FakeImageResolver::new(ImageResourceState::Pending));
        let mut canvas = resource_image_canvas(resolver.clone());
        let scene_graph =
            resource_image_scene_graph_with_policy(key.clone(), SceneImageUnavailablePolicy::Skip);

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        assert_eq!(canvas.image_resource_status().pending, vec![key]);
        assert_eq!(image.get_pixel(4, 4).0, [255, 255, 255, 255]);
    }

    #[test]
    fn resource_image_ready_after_pending_redraws_without_set_scene() {
        let key = ResourceKey::new("tile/0/0/0");
        let resolver = Arc::new(FakeImageResolver::new(ImageResourceState::Pending));
        let mut canvas = resource_image_canvas(resolver.clone());
        let scene_graph = resource_image_scene_graph(key.clone());

        canvas
            .set_scene(&scene_graph, &avenger_typst_label::bundled_label_engine())
            .unwrap();
        let pending = pollster::block_on(canvas.render()).unwrap();
        assert_eq!(pending.get_pixel(4, 4).0, [10, 20, 30, 255]);
        assert_eq!(canvas.image_resource_status().pending, vec![key]);

        resolver.set_state(ImageResourceState::Ready(Arc::new(solid_image([
            0, 200, 60, 255,
        ]))));
        let ready = pollster::block_on(canvas.render()).unwrap();

        assert!(canvas.image_resource_status().pending.is_empty());
        assert!(canvas.image_resource_status().missing.is_empty());
        assert!(canvas.image_resource_status().failed.is_empty());
        assert_eq!(ready.get_pixel(4, 4).0, [0, 200, 60, 255]);
    }

    #[test]
    fn resource_image_missing_errors_by_default() {
        let key = ResourceKey::new("tile/0/0/0");
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [8.0, 8.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();
        canvas
            .set_scene(
                &resource_image_scene_graph(key),
                &avenger_typst_label::bundled_label_engine(),
            )
            .unwrap();

        let error = pollster::block_on(canvas.render()).unwrap_err();
        assert!(matches!(
            error,
            AvengerWgpuError::ImageResourceError(message)
                if message.contains("No WGPU image resource resolver configured")
        ));
    }

    #[test]
    fn adjacent_linear_images_do_not_show_atlas_seam() {
        let mut canvas = pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [32.0, 16.0],
                scale: 1.0,
            },
            CanvasConfig::default(),
        ))
        .unwrap();
        canvas
            .set_scene(
                &adjacent_image_seam_scene_graph(),
                &avenger_typst_label::bundled_label_engine(),
            )
            .unwrap();
        let image = pollster::block_on(canvas.render()).unwrap();

        for y in 1..15 {
            assert_red_pixel(image.get_pixel(15, y).0, 15, y);
            assert_red_pixel(image.get_pixel(16, y).0, 16, y);
        }
    }

    fn resource_image_canvas(resolver: Arc<FakeImageResolver>) -> PngCanvas {
        pollster::block_on(PngCanvas::new(
            CanvasDimensions {
                size: [8.0, 8.0],
                scale: 1.0,
            },
            CanvasConfig {
                image_resource_config: WgpuImageResourceConfig {
                    resolver: Some(resolver),
                    missing_policy: WgpuMissingImagePolicy::DrawPlaceholder,
                    placeholder: WgpuImagePlaceholder::Solid([10, 20, 30, 255]),
                },
                ..Default::default()
            },
        ))
        .unwrap()
    }

    fn resource_image_scene_graph(key: ResourceKey) -> SceneGraph {
        resource_image_scene_graph_with_policy(key, SceneImageUnavailablePolicy::RendererDefault)
    }

    fn resource_image_scene_graph_with_policy(
        key: ResourceKey,
        unavailable_policy: SceneImageUnavailablePolicy,
    ) -> SceneGraph {
        SceneGraph {
            width: 8.0,
            height: 8.0,
            origin: [0.0, 0.0],
            marks: vec![SceneImageMark {
                len: 1,
                aspect: false,
                smooth: false,
                image: ScalarOrArray::new_scalar(SceneImageSource::Resource(SceneImageResource {
                    key,
                    intrinsic_width: 2,
                    intrinsic_height: 2,
                    fallback_key: None,
                })),
                x: ScalarOrArray::new_scalar(0.0),
                y: ScalarOrArray::new_scalar(0.0),
                width: ScalarOrArray::new_scalar(8.0),
                height: ScalarOrArray::new_scalar(8.0),
                align: ScalarOrArray::new_scalar(ImageAlign::Left),
                baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
                unavailable_policy,
                ..Default::default()
            }
            .into()],
        }
    }

    fn solid_image(color: [u8; 4]) -> RgbaImage {
        solid_image_with_size(color, 2, 2)
    }

    fn solid_image_with_size(color: [u8; 4], width: u32, height: u32) -> RgbaImage {
        RgbaImage {
            width,
            height,
            data: color.repeat((width * height) as usize),
        }
    }

    fn adjacent_image_seam_scene_graph() -> SceneGraph {
        SceneGraph {
            width: 32.0,
            height: 16.0,
            origin: [0.0, 0.0],
            marks: vec![
                SceneRectMark {
                    len: 1,
                    x: ScalarOrArray::new_scalar(0.0),
                    y: ScalarOrArray::new_scalar(0.0),
                    width: Some(ScalarOrArray::new_scalar(32.0)),
                    height: Some(ScalarOrArray::new_scalar(16.0)),
                    fill: ScalarOrArray::new_scalar(ColorOrGradient::Color([0.0, 0.0, 0.0, 1.0])),
                    zindex: Some(0),
                    ..Default::default()
                }
                .into(),
                inline_image_mark(0.0, 0.0, 16.0, 16.0).into(),
                inline_image_mark(16.0, 0.0, 16.0, 16.0).into(),
            ],
        }
    }

    fn inline_image_mark(x: f32, y: f32, width: f32, height: f32) -> SceneImageMark {
        SceneImageMark {
            len: 1,
            aspect: false,
            smooth: true,
            image: ScalarOrArray::new_scalar(SceneImageSource::inline(solid_image_with_size(
                [220, 20, 20, 255],
                4,
                4,
            ))),
            x: ScalarOrArray::new_scalar(x),
            y: ScalarOrArray::new_scalar(y),
            width: ScalarOrArray::new_scalar(width),
            height: ScalarOrArray::new_scalar(height),
            align: ScalarOrArray::new_scalar(ImageAlign::Left),
            baseline: ScalarOrArray::new_scalar(ImageBaseline::Top),
            zindex: Some(1),
            ..Default::default()
        }
    }

    fn assert_red_pixel(pixel: [u8; 4], x: u32, y: u32) {
        assert!(
            pixel[0] >= 200 && pixel[1] <= 35 && pixel[2] <= 35 && pixel[3] == 255,
            "expected red image pixel at ({x}, {y}), got {pixel:?}"
        );
    }

    struct FakeImageResolver {
        state: Mutex<ImageResourceState>,
    }

    impl FakeImageResolver {
        fn new(state: ImageResourceState) -> Self {
            Self {
                state: Mutex::new(state),
            }
        }

        fn set_state(&self, state: ImageResourceState) {
            *self.state.lock().unwrap() = state;
        }
    }

    impl ImageResourceResolver for FakeImageResolver {
        fn image_state(&self, _key: &ResourceKey) -> ImageResourceState {
            self.state.lock().unwrap().clone()
        }
    }
}
