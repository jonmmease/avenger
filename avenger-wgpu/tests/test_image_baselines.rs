use std::{path::Path, sync::Once};

use avenger_text::measurement::cosmic::register_font_directory;

static INIT: Once = Once::new();

pub fn initialize() {
    INIT.call_once(|| {
        let root_path = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fonts_dir = root_path
            .join("..")
            .join("avenger-vega-test-data")
            .join("fonts");
        register_font_directory(fonts_dir.to_str().unwrap());
    });
}

#[cfg(test)]
mod test_image_baselines {
    use crate::initialize;
    use avenger_color::ColorOrGradient;
    use avenger_common::{
        canvas::CanvasDimensions,
        types::{ImageAlign, ImageBaseline},
        value::ScalarOrArray,
    };
    use avenger_image::{ImageResourceResolver, ImageResourceState, RgbaImage};
    use avenger_resource::ResourceKey;
    use avenger_scenegraph::{
        marks::image::{
            SceneImageMark, SceneImageResource, SceneImageSource, SceneImageUnavailablePolicy,
        },
        marks::rect::SceneRectMark,
        scene_graph::SceneGraph,
    };
    use avenger_vega_scenegraph::scene_graph::VegaSceneGraph;
    use avenger_wgpu::{
        canvas::{Canvas, CanvasConfig, PngCanvas},
        error::AvengerWgpuError,
        image_resources::{WgpuImagePlaceholder, WgpuImageResourceConfig, WgpuMissingImagePolicy},
    };
    use dssim::Dssim;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

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
        case("text", "text_alignment", 0.015),
        case("text", "text_rotation", 0.015),
        case("text", "letter_scatter", 0.03),
        case("text", "lasagna_plot", 0.02),
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
        case("clip", "text_clip", 0.025),
        case("clip", "clip_rounded", 0.0001),
        case("clip", "text_clip_rounded", 0.025),
        case("clip", "bar_rounded", 0.02),
    )]
    fn test_image_baseline(category: &str, spec_name: &str, tolerance: f64) {
        use avenger_common::canvas::CanvasDimensions;

        initialize();

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
            Default::default(),
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
    fn resource_image_pending_draws_placeholder() {
        let key = ResourceKey::new("tile/0/0/0");
        let resolver = Arc::new(FakeImageResolver::new(ImageResourceState::Pending));
        let mut canvas = resource_image_canvas(resolver.clone());
        let scene_graph = resource_image_scene_graph_with_policy(
            key.clone(),
            SceneImageUnavailablePolicy::RendererDefault,
        );

        canvas.set_scene(&scene_graph).unwrap();
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

        canvas.set_scene(&scene_graph).unwrap();
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

        canvas.set_scene(&scene_graph).unwrap();
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
        canvas.set_scene(&resource_image_scene_graph(key)).unwrap();

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
            .set_scene(&adjacent_image_seam_scene_graph())
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
            image: ScalarOrArray::new_scalar(SceneImageSource::Inline(solid_image_with_size(
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
