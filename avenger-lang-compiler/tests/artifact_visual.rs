use std::{path::PathBuf, sync::Arc};

use avenger_chart::prelude::*;
use avenger_common::canvas::CanvasDimensions;
use avenger_lang_compiler::{
    CompiledChartArtifact, Compiler, DependencyFingerprint, ProjectChartId,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{
    arrow::{
        array::Float64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    logical_expr::col,
    prelude::SessionContext,
};

#[tokio::test]
async fn vertical_slice_hello_scatter_matches_direct_rust_and_existing_baseline() {
    let x_values = Float64Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0]);
    let y_values = Float64Array::from(vec![2.5, 3.2, 4.8, 3.1, 5.9, 7.2, 6.5, 8.1, 7.8, 9.5]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
    let data_context = SessionContext::new();
    let data = data_context.read_batch(batch).unwrap();
    let context = SessionContext::new();
    let plot = Chart::<Cartesian>::new().data(data).mark(
        MarkGroup::new().id("points").mark(
            Symbol::new()
                .id("dots")
                .x_with(col("x"), |channel| {
                    channel
                        .scale_with::<Linear>(|scale| scale)
                        .axis(|axis| axis.title("X Value"))
                })
                .y_with(col("y"), |channel| {
                    channel
                        .scale_with::<Linear>(|scale| scale)
                        .axis(|axis| axis.title("Y Value"))
                })
                .size(100.0)
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(1.0),
        ),
    );
    let compiled = Arc::new(plot.compile(&context).await.unwrap());
    let registry = avenger_chart_lang_registry::builtins::bootstrap_registry().unwrap();
    let mut artifact = CompiledChartArtifact::new(
        ProjectChartId::new("existing-simple-scatter"),
        Some("Existing simple scatter visual fixture".to_string()),
        avenger_lang_core::SourceId::new(0),
        compiled,
        registry.profile_id().clone(),
        DependencyFingerprint::new("existing-simple-scatter"),
    );
    artifact.interface.public_targets.insert(
        "chart.points.dots".to_string(),
        artifact.compiled_plot().marks()[0]
            .state()
            .identity
            .runtime_id
            .as_opaque_str()
            .to_string(),
    );

    let direct_evaluated = artifact
        .compiled_plot()
        .evaluate(&context, None)
        .await
        .unwrap();
    let fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/projects/00_hello_scatter");
    let dsl_artifact = Compiler::builder()
        .project_root(&fixture)
        .build()
        .unwrap()
        .compile_file(fixture.join("chart.avenger"))
        .await
        .unwrap();
    let dsl_evaluated = dsl_artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    assert_eq!(dsl_artifact.interface, artifact.interface);

    let direct = render_scene(&direct_evaluated.scene_graph, 2.0).await;
    let dsl = render_scene(&dsl_evaluated.scene_graph, 2.0).await;

    let equivalence = image_compare::rgba_hybrid_compare(&direct, &dsl).unwrap();
    assert!(
        equivalence.score >= 0.9999,
        "DSL render differs from direct Rust render (similarity {})",
        equivalence.score
    );

    let baseline_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../avenger-chart/tests/baselines/symbol/simple_scatter_plot.png");
    let expected = image::open(&baseline_path).unwrap().to_rgba8();
    let comparison = image_compare::rgba_hybrid_compare(&expected, &dsl).unwrap();
    assert!(
        comparison.score >= 0.9999,
        "DSL artifact render differs from {} (similarity {})",
        baseline_path.display(),
        comparison.score
    );
}

async fn render_scene(
    scene_graph: &avenger_scenegraph::scene_graph::SceneGraph,
    scale: f32,
) -> image::RgbaImage {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale,
    };
    let config = CanvasConfig {
        font_resolution: avenger_chart::fonts::default_font_resolution(),
        ..Default::default()
    };
    let mut canvas = PngCanvas::new(dimensions, config).await.unwrap();
    canvas.set_scene(scene_graph).unwrap();
    canvas.render().await.unwrap()
}

#[tokio::test]
async fn registry_driven_phase0_artifact_renders_through_wgpu() {
    let compiler = Compiler::builder()
        .project_root("/project")
        .build()
        .unwrap();
    let artifact = compiler.compile_phase0_example().await.unwrap();
    let context = SessionContext::new();
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&context, None)
        .await
        .unwrap();
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: 1.0,
    };
    let mut canvas = PngCanvas::new(dimensions, CanvasConfig::default())
        .await
        .unwrap();
    canvas.set_scene(&evaluated.scene_graph).unwrap();
    let image = canvas.render().await.unwrap();

    assert!(image.width() > 0 && image.height() > 0);
    assert!(image.pixels().any(|pixel| pixel.0[3] != 0));
}
