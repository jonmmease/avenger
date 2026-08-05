use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use avenger_chart::prelude::{
    EvaluationRequest, InMemoryNativeWidgetInstanceStore, NativeWidgetDocumentId,
    NativeWidgetPlotId, NativeWidgetRegistry, NativeWidgetRuntimeResources, PlotSessionOptions,
};
use avenger_chart_widgets::register_native_widgets;
use avenger_common::canvas::CanvasDimensions;
use avenger_lang_compiler::{CompiledChartArtifact, Compiler};
use avenger_lang_core::{SourceFile, SourceId, SourceOrigin, syntax::parse_file};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use image::RgbaImage;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const CASE_ENV: &str = "AVENGER_VL_GALLERY_CASE";
const BLESS_ENV: &str = "AVENGER_VL_GALLERY_BLESS";
const BASELINE_THRESHOLD: f64 = 0.9999;
const ROUND_TRIP_THRESHOLD: f64 = 0.99999;

#[derive(Debug, Deserialize)]
struct GalleryManifest {
    schema_version: u32,
    counts: GalleryCounts,
    placements: Vec<GalleryPlacement>,
    examples: Vec<GalleryExample>,
}

#[derive(Debug, Deserialize)]
struct GalleryCounts {
    placements: usize,
    examples: usize,
}

#[derive(Debug, Deserialize)]
struct GalleryPlacement {
    position: usize,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GalleryExample {
    name: String,
    upstream_spec: String,
    upstream_spec_sha256: String,
    upstream_reference: String,
    upstream_reference_sha256: String,
    chart: String,
    datasets: Vec<String>,
    implementation: GalleryImplementation,
    blockers: Vec<serde_json::Value>,
    review: GalleryReview,
}

#[derive(Debug, Deserialize)]
struct SourceManifest {
    schema_version: u32,
    sources: Vec<SourceRecord>,
    relations: Vec<RelationRecord>,
}

#[derive(Debug, Deserialize)]
struct SourceRecord {
    path: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct RelationRecord {
    name: String,
    path: String,
    rows: usize,
    sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GalleryImplementation {
    Unported,
    Implemented,
    BlockedData,
    BlockedMark,
    BlockedTransform,
    BlockedLayout,
    BlockedInteraction,
    BlockedLanguage,
    BlockedRuntime,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum GalleryReview {
    Unreviewed,
    Reviewed,
    KnownDifference,
    KnownIncorrect,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixture_dir() -> PathBuf {
    manifest_dir().join("tests/fixtures/vega_lite_gallery")
}

fn baseline_dir() -> PathBuf {
    manifest_dir().join("tests/baselines/vega_lite_gallery")
}

fn failure_dir() -> PathBuf {
    manifest_dir().join("../target/tests/avenger-vega-lite-gallery")
}

fn load_manifest() -> GalleryManifest {
    serde_json::from_str(include_str!("fixtures/vega_lite_gallery/gallery.json"))
        .expect("Vega-Lite gallery manifest must be valid JSON")
}

fn sha256(path: &Path) -> String {
    let mut digest = Sha256::new();
    digest.update(
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display())),
    );
    format!("{:x}", digest.finalize())
}

#[test]
fn gallery_inventory_is_locked_and_complete() {
    let manifest = load_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.counts.placements, 203);
    assert_eq!(manifest.counts.examples, 189);
    assert_eq!(manifest.placements.len(), manifest.counts.placements);
    assert_eq!(manifest.examples.len(), manifest.counts.examples);

    let mut names = BTreeSet::new();
    let mut placements = BTreeMap::<String, usize>::new();
    for (expected_position, placement) in manifest.placements.iter().enumerate() {
        assert_eq!(placement.position, expected_position);
        *placements.entry(placement.name.clone()).or_default() += 1;
    }
    for example in &manifest.examples {
        assert!(
            names.insert(example.name.clone()),
            "duplicate {}",
            example.name
        );
        assert!(placements.contains_key(&example.name));
        let spec = fixture_dir().join(&example.upstream_spec);
        let reference = fixture_dir().join(&example.upstream_reference);
        assert_eq!(
            sha256(&spec),
            example.upstream_spec_sha256,
            "{}",
            example.name
        );
        assert_eq!(
            sha256(&reference),
            example.upstream_reference_sha256,
            "{}",
            example.name
        );

        let chart = fixture_dir().join(&example.chart);
        match example.implementation {
            GalleryImplementation::Implemented => {
                assert!(
                    chart.is_file(),
                    "implemented chart {} is absent",
                    chart.display()
                );
                assert!(
                    example.blockers.is_empty(),
                    "implemented example {} still has blockers",
                    example.name
                );
            }
            GalleryImplementation::Unported => {
                assert!(
                    !chart.exists(),
                    "unported example {} unexpectedly has a chart",
                    example.name
                );
                assert_eq!(example.review, GalleryReview::Unreviewed);
            }
            _ => assert!(
                !example.blockers.is_empty(),
                "blocked example {} must identify a blocker",
                example.name
            ),
        }
        if example.review == GalleryReview::Reviewed {
            assert_eq!(example.implementation, GalleryImplementation::Implemented);
            assert!(
                baseline_dir()
                    .join(format!("{}.png", example.name))
                    .is_file()
            );
        }
    }
    assert_eq!(names, placements.into_keys().collect());
}

#[test]
fn gallery_catalog_is_complete_and_locked() {
    let gallery = load_manifest();
    let source_manifest: SourceManifest = serde_json::from_str(include_str!(
        "fixtures/vega_lite_gallery/provenance/source-manifest.json"
    ))
    .expect("Vega gallery source manifest");
    assert_eq!(source_manifest.schema_version, 1);
    assert_eq!(source_manifest.sources.len(), 46);
    assert_eq!(source_manifest.relations.len(), 48);
    assert!(
        source_manifest
            .sources
            .iter()
            .all(|source| !source.path.is_empty() && source.sha256.len() == 64)
    );

    let catalog_path = fixture_dir().join("catalog.avenger");
    let catalog_source = fs::read_to_string(&catalog_path).unwrap();
    parse_file(&SourceFile::new(
        SourceId::new(0),
        SourceOrigin::File(catalog_path),
        catalog_source.clone(),
    ))
    .expect("generated Vega catalog must parse");
    assert_eq!(catalog_source.matches("table parquet as ").count(), 48);

    let declared = source_manifest
        .relations
        .iter()
        .map(|relation| relation.name.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(declared.len(), source_manifest.relations.len());
    let required = gallery
        .examples
        .iter()
        .flat_map(|example| example.datasets.iter().cloned())
        .collect::<BTreeSet<_>>();
    assert_eq!(required, declared);

    let mut discovered = BTreeSet::new();
    for relation in &source_manifest.relations {
        assert!(relation.rows > 0, "empty relation {}", relation.name);
        let path = fixture_dir().join(&relation.path);
        assert_eq!(sha256(&path), relation.sha256, "{}", relation.name);
        assert!(
            catalog_source.contains(&format!("table parquet as {} {{", relation.name)),
            "catalog does not declare {}",
            relation.name
        );
        assert!(discovered.insert(path.file_name().unwrap().to_owned()));
    }
    let actual_files = fs::read_dir(fixture_dir().join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_files, discovered, "catalog data directory drifted");
}

#[tokio::test]
async fn implemented_gallery_examples_compile_render_and_round_trip() {
    let manifest = load_manifest();
    let filter = std::env::var(CASE_ENV).ok();
    let selected = manifest
        .examples
        .iter()
        .filter(|example| example.implementation == GalleryImplementation::Implemented)
        .filter(|example| {
            filter
                .as_deref()
                .is_none_or(|filter| example.name == filter)
        })
        .collect::<Vec<_>>();
    if let Some(filter) = &filter {
        assert_eq!(
            selected.len(),
            1,
            "{CASE_ENV} must name exactly one implemented gallery example; got {filter:?}"
        );
    }
    assert!(!selected.is_empty(), "gallery has no implemented examples");

    let compiler = Compiler::builder()
        .project_root(fixture_dir())
        .build()
        .expect("gallery compiler");
    let mut failures = Vec::new();
    for example in selected {
        eprintln!("Vega-Lite gallery case: {}", example.name);
        if let Err(error) = run_example(&compiler, example).await {
            failures.push(format!("{}: {error}", example.name));
        }
    }
    assert!(
        failures.is_empty(),
        "{} gallery case(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

async fn run_example(compiler: &Compiler, example: &GalleryExample) -> Result<(), String> {
    let generation = compiler
        .compile_chart_generation_attempt(fixture_dir().join(&example.chart), None, 0)
        .await
        .result
        .map_err(|failure| {
            format!(
                "compilation failed:\n{}",
                serde_json::to_string_pretty(&failure.diagnostics).unwrap()
            )
        })?;
    if std::env::var_os("AVENGER_VL_GALLERY_DEBUG").is_some() {
        eprintln!(
            "{}",
            serde_json::to_string_pretty(generation.artifact.compiled_plot())
                .map_err(|error| format!("compiled plot debug serialization failed: {error}"))?
        );
    }
    let registry = Arc::clone(&compiler.options().native_registry);
    let bytes = generation
        .artifact
        .to_bytes()
        .map_err(|error| format!("artifact serialization failed: {error}"))?;
    let round_tripped = CompiledChartArtifact::from_bytes(&bytes, &registry)
        .map_err(|error| format!("artifact deserialization failed: {error}"))?;

    let direct = evaluate_and_render(
        &generation.artifact,
        generation.environment.fork().session_context(),
        &format!("{}.direct", example.name),
    )
    .await?;
    let serialized = evaluate_and_render(
        &round_tripped,
        generation.environment.fork().session_context(),
        &format!("{}.serialized", example.name),
    )
    .await?;
    compare_round_trip(example, &direct, &serialized)?;
    compare_or_bless(example, &direct)
}

async fn evaluate_and_render(
    artifact: &CompiledChartArtifact,
    context: &datafusion::prelude::SessionContext,
    debug_label: &str,
) -> Result<RgbaImage, String> {
    let mut registry = NativeWidgetRegistry::new();
    register_native_widgets(&mut registry)
        .map_err(|error| format!("native widget registration failed: {error}"))?;
    let resources = NativeWidgetRuntimeResources::new(
        Arc::new(registry),
        Arc::new(InMemoryNativeWidgetInstanceStore::new()),
        NativeWidgetDocumentId::new(),
    );
    let mut session =
        Arc::new(artifact.compiled_plot().clone()).instantiate(Arc::new(context.clone()));
    session.set_options(PlotSessionOptions::from_native_widget_resources(
        &resources,
        NativeWidgetPlotId::chart_root(),
    ));
    let evaluated = evaluate_ready_scene(&mut session).await?;
    if std::env::var_os("AVENGER_VL_GALLERY_DEBUG").is_some() {
        let debug_path = failure_dir().join(format!("{debug_label}.scene.json"));
        if let Some(parent) = debug_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(
            &debug_path,
            serde_json::to_vec_pretty(&evaluated.scene_graph)
                .map_err(|error| format!("scene graph debug serialization failed: {error}"))?,
        )
        .map_err(|error| format!("failed to write {}: {error}", debug_path.display()))?;
        eprintln!("scene graph debug: {}", debug_path.display());
    }
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: 2.0,
    };
    let config = CanvasConfig {
        font_resolution: avenger_chart::fonts::default_font_resolution(),
        ..Default::default()
    };
    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .map_err(|error| format!("canvas creation failed: {error}"))?;
    canvas
        .set_scene(&evaluated.scene_graph)
        .map_err(|error| format!("scene upload failed: {error}"))?;
    canvas
        .render()
        .await
        .map_err(|error| format!("render failed: {error}"))
}

async fn evaluate_ready_scene(
    session: &mut avenger_chart::plot::PlotSession,
) -> Result<avenger_chart::render::EvaluatedPlot, String> {
    for attempt in 0..4 {
        let epoch = session.evaluation_invalidation_epoch();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await
            .map_err(|error| format!("evaluation failed: {error}"))?;
        let pending = metrics.pipeline.materialization_queued > 0
            || metrics.pipeline.materialization_running > 0
            || session.has_pending_materializations();
        if !pending {
            return Ok(evaluated);
        }
        if attempt == 3 {
            return Err("view-local materialization did not stabilize".to_owned());
        }
        for _ in 0..200 {
            if session.evaluation_invalidation_epoch() > epoch
                || !session.has_pending_materializations()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    unreachable!()
}

fn compare_round_trip(
    example: &GalleryExample,
    direct: &RgbaImage,
    serialized: &RgbaImage,
) -> Result<(), String> {
    let comparison = image_compare::rgba_hybrid_compare(direct, serialized)
        .map_err(|error| format!("round-trip comparison failed: {error}"))?;
    if comparison.score < ROUND_TRIP_THRESHOLD {
        save_image(
            &failure_dir().join(format!("{}.direct.png", example.name)),
            direct,
        )?;
        save_image(
            &failure_dir().join(format!("{}.serialized.png", example.name)),
            serialized,
        )?;
        return Err(format!(
            "serialized similarity {:.6} is below {:.6}",
            comparison.score, ROUND_TRIP_THRESHOLD
        ));
    }
    Ok(())
}

fn compare_or_bless(example: &GalleryExample, actual: &RgbaImage) -> Result<(), String> {
    let baseline = baseline_dir().join(format!("{}.png", example.name));
    if std::env::var_os(BLESS_ENV).is_some() {
        let filter = std::env::var(CASE_ENV)
            .map_err(|_| format!("{BLESS_ENV} requires an exact {CASE_ENV}=<example> filter"))?;
        if filter != example.name {
            return Err(format!(
                "refusing to bless {0} through filter {filter:?}",
                example.name
            ));
        }
        save_image(&baseline, actual)?;
    }
    if !baseline.is_file() {
        let actual_path = failure_dir().join(format!("{}.actual.png", example.name));
        save_image(&actual_path, actual)?;
        return Err(format!(
            "missing baseline {}; actual saved to {}",
            baseline.display(),
            actual_path.display()
        ));
    }
    let expected = image::open(&baseline)
        .map_err(|error| format!("failed to read {}: {error}", baseline.display()))?
        .into_rgba8();
    if expected.dimensions() != actual.dimensions() {
        return Err(format!(
            "baseline dimensions {:?} differ from actual {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }
    let comparison = image_compare::rgba_hybrid_compare(&expected, actual)
        .map_err(|error| format!("baseline comparison failed: {error}"))?;
    if comparison.score < BASELINE_THRESHOLD {
        let actual_path = failure_dir().join(format!("{}.actual.png", example.name));
        let diff_path = failure_dir().join(format!("{}.diff.png", example.name));
        save_image(&actual_path, actual)?;
        save_image(&diff_path, &comparison.image.to_color_map().to_rgba8())?;
        return Err(format!(
            "similarity {:.6} is below {:.6}; actual saved to {}",
            comparison.score,
            BASELINE_THRESHOLD,
            actual_path.display()
        ));
    }
    Ok(())
}

fn save_image(path: &Path, image: &RgbaImage) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    image
        .save(path)
        .map_err(|error| format!("failed to save {}: {error}", path.display()))
}
