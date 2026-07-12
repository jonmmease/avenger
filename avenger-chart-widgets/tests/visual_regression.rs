use std::{
    fs,
    path::{Path, PathBuf},
};

use avenger_chart::{
    marks::symbol::Symbol,
    plot::{Chart, CompiledPlot},
    zerod::ZeroDCoord,
};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::SessionContext;
use image::RgbaImage;

const BASELINE_DIR: &str = "tests/baselines";
const FAILURE_DIR: &str = "tests/failures";
const BLESS_ENV: &str = "AVENGER_WIDGETS_BLESS_WGPU_BASELINES";
const DEFAULT_SCALE: f32 = 2.0;
const DEFAULT_THRESHOLD: f64 = 0.9999;

#[tokio::test]
async fn scaffold_harness_round_trip() {
    let ctx = SessionContext::new();
    let chart = Chart::<ZeroDCoord>::new()
        .canvas_size(96.0, 64.0)
        .plot_size(64.0, 32.0)
        .mark(
            Symbol::new()
                .size(256.0)
                .shape("square")
                .fill("#0072B2")
                .stroke("#202020")
                .stroke_width(1.0),
        );

    let compiled = chart.compile(&ctx).await.expect("compile smoke chart");
    let encoded = bincode::serialize(&compiled).expect("serialize compiled smoke chart");
    let decoded: CompiledPlot =
        bincode::deserialize(&encoded).expect("deserialize compiled smoke chart");

    let direct = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate direct smoke chart");
    let round_tripped = decoded
        .evaluate(&ctx, None)
        .await
        .expect("evaluate round-tripped smoke chart");
    let direct_image = render_scene_graph_to_wgpu_image(&direct.scene_graph).await;
    let round_trip_image = render_scene_graph_to_wgpu_image(&round_tripped.scene_graph).await;

    let equivalence = image_compare::rgba_hybrid_compare(&direct_image, &round_trip_image)
        .expect("compare direct and round-tripped images");
    assert_eq!(equivalence.score, 1.0, "round trip changed smoke scene");
    assert_visual_match("scaffold/harness_smoke", &direct_image);
}

async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create widget visual-test canvas");
    canvas
        .set_scene(scene_graph)
        .expect("set widget smoke scene");
    canvas.render().await.expect("render widget smoke scene")
}

fn assert_visual_match(name: &str, actual: &RgbaImage) {
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, actual);
        return;
    }

    let actual_path = PathBuf::from(FAILURE_DIR).join(format!("{name}_actual.png"));
    let diff_path = PathBuf::from(FAILURE_DIR).join(format!("{name}_diff.png"));
    if !baseline_path.exists() {
        save_image(&actual_path, actual);
        panic!(
            "No widget baseline at '{}'. Actual saved to '{}'. Bless with {BLESS_ENV}=1.",
            baseline_path.display(),
            actual_path.display()
        );
    }

    let expected = image::open(&baseline_path)
        .unwrap_or_else(|error| panic!("load '{}': {error}", baseline_path.display()))
        .into_rgba8();
    assert_eq!(
        expected.dimensions(),
        actual.dimensions(),
        "widget baseline dimensions differ for {name}"
    );
    let comparison =
        image_compare::rgba_hybrid_compare(&expected, actual).expect("compare widget baseline");
    if comparison.score < DEFAULT_THRESHOLD {
        save_image(&actual_path, actual);
        save_image(&diff_path, &comparison.image.to_color_map().into_rgba8());
        panic!(
            "Widget baseline {name} score {:.6} is below {:.6}. Actual: '{}'; diff: '{}'.",
            comparison.score,
            DEFAULT_THRESHOLD,
            actual_path.display(),
            diff_path.display()
        );
    }
}

fn save_image(path: &Path, image: &RgbaImage) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create '{}': {error}", parent.display()));
    }
    image
        .save(path)
        .unwrap_or_else(|error| panic!("save '{}': {error}", path.display()));
}
