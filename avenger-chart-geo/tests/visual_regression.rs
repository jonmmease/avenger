//! Visual baselines for the Geo coordinate system (scratch/geo phase 2+).
//!
//! Bless with `AVENGER_GEO_BLESS_BASELINES=1 cargo test --release -p
//! avenger-chart-geo --test visual_regression`. Failures write
//! `tests/failures/<name>_{actual,diff}.png`.

use std::path::{Path, PathBuf};

use avenger_chart::plot::Plot;
use avenger_chart_core::CoordinateSystem;
use avenger_chart_geo::{Geo, GraticuleStyle, SphereStyle};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::prelude::SessionContext;
use image::RgbaImage;

const BASELINE_DIR: &str = "tests/baselines";
const FAILURE_DIR: &str = "tests/failures";
const BLESS_ENV: &str = "AVENGER_GEO_BLESS_BASELINES";
const DEFAULT_SCALE: f32 = 2.0;
const DEFAULT_THRESHOLD: f64 = 0.9999;

fn furnished(geo: Geo) -> Geo {
    geo.sphere(SphereStyle::default())
        .graticule(GraticuleStyle::default())
}

fn world_plot(geo: Geo) -> Plot<Geo> {
    Plot::with_coord(furnished(geo)).plot_size(520.0, 320.0)
}

#[tokio::test]
async fn graticule_sphere_equal_earth() {
    assert_visual_match(
        world_plot(Geo::equal_earth()),
        "graticule_sphere_equal_earth",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_natural_earth1() {
    assert_visual_match(
        world_plot(Geo::natural_earth()),
        "graticule_sphere_natural_earth1",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_winkel_tripel() {
    assert_visual_match(
        world_plot(Geo::winkel_tripel()),
        "graticule_sphere_winkel_tripel",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_equirectangular() {
    assert_visual_match(
        world_plot(Geo::equirectangular()),
        "graticule_sphere_equirectangular",
    )
    .await;
}

#[tokio::test]
async fn graticule_sphere_mercator() {
    // Mercator world: the auto world-square clip bounds the poles.
    assert_visual_match(
        Plot::with_coord(furnished(Geo::mercator())).plot_size(360.0, 360.0),
        "graticule_sphere_mercator",
    )
    .await;
}

#[tokio::test]
async fn graticule_albers_conus() {
    // CONUS Albers aspect: centered on Kansas, zoomed so the lower 48 fill
    // the frame. Graticule curvature under the conic is the visual proof
    // the resampler works.
    let geo = Geo::albers_usa_conus()
        .center_projected(0.0031, 0.6410)
        .zoom(3.4);
    assert_visual_match(
        Plot::with_coord(furnished(geo)).plot_size(520.0, 360.0),
        "graticule_albers_conus",
    )
    .await;
}

#[tokio::test]
async fn graticule_conic_conformal_europe() {
    let geo = Geo::conic_conformal((35.0, 65.0))
        .rotate([-15.0, 0.0, 0.0])
        .center_projected(-0.0408, 1.0053)
        .zoom(6.5);
    assert_visual_match(
        Plot::with_coord(furnished(geo)).plot_size(460.0, 400.0),
        "graticule_conic_conformal_europe",
    )
    .await;
}

#[tokio::test]
async fn graticule_rotated_equal_earth() {
    // Oblique aspect: rotation exercises the antimeridian cut and the
    // three-axis rotation together.
    assert_visual_match(
        world_plot(Geo::equal_earth().rotate([15.0, -30.0, 12.0])),
        "graticule_rotated_equal_earth",
    )
    .await;
}

// ---------------------------------------------------------------------------
// Harness (treemap pattern; research/baseline-harness.md §5)
// ---------------------------------------------------------------------------

async fn assert_visual_match<C>(plot: Plot<C>, baseline_name: &str)
where
    C: CoordinateSystem,
{
    let ctx = SessionContext::new();
    let compiled = plot.compile(&ctx).await.expect("compile geo plot");
    let evaluated = compiled
        .evaluate(&ctx, None)
        .await
        .expect("evaluate geo plot");
    let image = render_scene_graph_to_wgpu_image(&evaluated.scene_graph).await;
    let baseline_path = PathBuf::from(BASELINE_DIR).join(format!("{baseline_name}.png"));
    if std::env::var_os(BLESS_ENV).is_some() {
        save_image(&baseline_path, &image);
        return;
    }
    compare_image(&baseline_path, baseline_name, &image, DEFAULT_THRESHOLD);
}

async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .expect("create visual test canvas");
    canvas.set_scene(scene_graph).expect("set scene graph");
    canvas.render().await.expect("render scene graph")
}

fn compare_image(baseline_path: &Path, baseline_name: &str, actual: &RgbaImage, threshold: f64) {
    let actual_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_actual.png"));
    let diff_path = PathBuf::from(FAILURE_DIR).join(format!("{baseline_name}_diff.png"));

    if !baseline_path.exists() {
        save_image(&actual_path, actual);
        panic!(
            "No baseline at '{}'. Actual saved to '{}'. Bless with {}=1.",
            baseline_path.display(),
            actual_path.display(),
            BLESS_ENV
        );
    }

    let expected = image::open(baseline_path)
        .unwrap_or_else(|err| panic!("failed to load baseline: {err}"))
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_image(&actual_path, actual);
        panic!(
            "Dimension mismatch for {baseline_name}: baseline {:?}, actual {:?} (actual saved to {})",
            expected.dimensions(),
            actual.dimensions(),
            actual_path.display()
        );
    }

    let result = image_compare::rgba_hybrid_compare(&expected, actual).expect("image comparison");
    if result.score < threshold {
        save_image(&actual_path, actual);
        save_image(&diff_path, &result.image.to_color_map().into_rgba8());
        panic!(
            "Similarity {:.6} below threshold {threshold:.6} for {baseline_name}. Actual: {}, diff: {}",
            result.score,
            actual_path.display(),
            diff_path.display()
        );
    }
}

fn save_image(path: &Path, image: &RgbaImage) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create baseline directory");
    }
    image.save(path).expect("save image");
}
