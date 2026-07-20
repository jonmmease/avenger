use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use avenger_chart::prelude::{
    Cartesian, EvaluationRequest, HConcat, InMemoryNativeWidgetInstanceStore, IntoPlotMark,
    NativeWidgetDocumentId, NativeWidgetPlotId, NativeWidgetRegistry, NativeWidgetRuntimeResources,
    PlotSessionOptions, Subplot,
};
use avenger_chart_external_test::{
    external_compound_mark::ExternalMeanPoint,
    external_coord_system::{Cube, Isometric},
    external_mark::HexBin,
};
use avenger_chart_lang_registry::{
    CoordinatePack, NativeRegistry, NativeRegistryBuilder, RegistryError, builtins,
};
use avenger_chart_schema::{
    BodyMode, ChannelSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    ValueShape,
};
use avenger_chart_widgets::register_native_widgets;
use avenger_common::canvas::CanvasDimensions;
use avenger_lang_compiler::{
    CatalogFactory, CatalogFactoryError, CatalogFactoryRegistry, CompileEnvironment,
    CompiledChartArtifact, Compiler, TableFactory, TableFactoryError, TableFactoryRegistry,
};
use avenger_lang_core::{
    DataCapabilities, MapEnvironmentProvider, SourceFile, SourceId, SourceOrigin, ast::Root,
    syntax::parse_file,
};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use datafusion::{
    arrow::datatypes::{DataType, Field, Schema},
    catalog::{CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider},
    datasource::{TableProvider, empty::EmptyTable},
    logical_expr::lit,
};
use image::RgbaImage;
use serde::Deserialize;

const BLESS_ENV: &str = "AVENGER_LANG_BLESS_FIXTURE_BASELINES";
const CASE_FILTER_ENV: &str = "AVENGER_LANG_FIXTURE_CASE";
const DEFAULT_SCALE: f32 = 2.0;
const BASELINE_THRESHOLD: f64 = 0.9999;
const ROUND_TRIP_THRESHOLD: f64 = 0.99999;
static MOCK_ICEBERG_CREATES: AtomicUsize = AtomicUsize::new(0);
static MOCK_DELTA_CREATES: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    schema_version: u32,
    cases: Vec<FixtureCase>,
}

#[derive(Debug, Deserialize)]
struct FixtureCase {
    root: String,
    expectation: FixtureExpectation,
    review: Option<FixtureReview>,
    #[serde(default)]
    host: FixtureHost,
    sources: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureExpectation {
    Visual,
    Diagnostic,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureReview {
    Reviewed,
    KnownIncorrect,
    Weak,
    IntentionalBlank,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum FixtureHost {
    #[default]
    Stock,
    ComposedRegistry,
    ProviderMocks,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn fixtures_dir() -> PathBuf {
    manifest_dir().join("tests/fixtures/projects")
}

fn baselines_dir() -> PathBuf {
    manifest_dir().join("tests/baselines/visual")
}

fn failures_dir() -> PathBuf {
    manifest_dir().join("../target/tests/avenger-lang-fixture-visual")
}

fn load_manifest() -> FixtureManifest {
    serde_json::from_str(include_str!("fixtures/visual_cases.json"))
        .expect("fixture visual manifest must be valid JSON")
}

fn visit_avenger_files(root: &Path, current: &Path, output: &mut BTreeSet<String>) {
    let mut entries = fs::read_dir(current)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", current.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().unwrap().is_dir() {
            visit_avenger_files(root, &path, output);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("avenger") {
            output.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

fn discovered_sources() -> BTreeSet<String> {
    let root = fixtures_dir();
    let mut sources = BTreeSet::new();
    visit_avenger_files(&root, &root, &mut sources);
    sources
}

fn baseline_path(case: &FixtureCase) -> PathBuf {
    let mut path = baselines_dir().join(&case.root);
    path.set_extension(match case.expectation {
        FixtureExpectation::Visual => "png",
        FixtureExpectation::Diagnostic => "diagnostic.json",
    });
    path
}

fn failure_path(case: &FixtureCase, extension: &str) -> PathBuf {
    let mut path = failures_dir().join(&case.root);
    path.set_extension(extension);
    path
}

fn project_root(case: &FixtureCase) -> PathBuf {
    let project = Path::new(&case.root)
        .components()
        .next()
        .expect("fixture root must include its project directory");
    fixtures_dir().join(project.as_os_str())
}

fn blessing_enabled() -> bool {
    std::env::var_os(BLESS_ENV).is_some()
}

#[test]
fn fixture_visual_manifest_owns_every_avenger_source_exactly_once() {
    let manifest = load_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.cases.len(), 44, "one case per chart root");
    assert_eq!(
        manifest
            .cases
            .iter()
            .filter(|case| case.expectation == FixtureExpectation::Visual)
            .count(),
        42
    );

    let mut owners = BTreeMap::new();
    let mut roots = BTreeSet::new();
    for case in &manifest.cases {
        match case.expectation {
            FixtureExpectation::Visual => assert!(
                case.review.is_some(),
                "visual case {} must declare its review status",
                case.root
            ),
            FixtureExpectation::Diagnostic => assert!(
                case.review.is_none(),
                "diagnostic case {} must not declare a visual review status",
                case.root
            ),
        }
        assert!(
            roots.insert(case.root.clone()),
            "duplicate root {}",
            case.root
        );
        assert!(
            case.sources.contains(&case.root),
            "case {} must own its chart root",
            case.root
        );
        for source in &case.sources {
            assert!(
                owners.insert(source.clone(), case.root.clone()).is_none(),
                "fixture source {source} has more than one manifest owner"
            );
        }

        let source_path = fixtures_dir().join(&case.root);
        let parsed = parse_file(&SourceFile::new(
            SourceId::new(0),
            SourceOrigin::File(source_path.clone()),
            fs::read_to_string(&source_path).unwrap(),
        ))
        .unwrap_or_else(|error| panic!("{} did not parse: {error}", case.root));
        assert!(
            matches!(parsed.ast.root, Root::Chart(_)),
            "case root {} is not a chart",
            case.root
        );
    }

    let owned = owners.into_keys().collect::<BTreeSet<_>>();
    let discovered = discovered_sources();
    assert_eq!(owned.len(), 61, "reviewed fixture inventory changed");
    assert_eq!(
        owned, discovered,
        "update visual_cases.json for fixture drift"
    );
}

#[tokio::test]
async fn fixture_visual_regression() {
    let manifest = load_manifest();
    let case_filter = std::env::var(CASE_FILTER_ENV).ok();
    let mut failures = Vec::new();
    for case in manifest.cases.iter().filter(|case| {
        case_filter
            .as_deref()
            .is_none_or(|filter| case.root.contains(filter))
    }) {
        eprintln!("fixture visual case: {}", case.root);
        if let Err(error) = run_case(case).await {
            failures.push(format!("{}: {error}", case.root));
        }
    }
    assert!(
        failures.is_empty(),
        "{} fixture baseline case(s) failed:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

async fn run_case(case: &FixtureCase) -> Result<(), String> {
    if case.host == FixtureHost::ProviderMocks {
        MOCK_ICEBERG_CREATES.store(0, Ordering::SeqCst);
        MOCK_DELTA_CREATES.store(0, Ordering::SeqCst);
    }
    let compiler = compiler_for(case)?;
    let result = compiler
        .compile_file_generation_attempt(fixtures_dir().join(&case.root), 0)
        .await
        .result;

    match (case.expectation, result) {
        (FixtureExpectation::Diagnostic, Err(failure)) => compare_or_bless_diagnostic(
            case,
            &(serde_json::to_string_pretty(&failure.diagnostics).unwrap() + "\n"),
        ),
        (FixtureExpectation::Diagnostic, Ok(_)) => Err("expected compilation to fail".to_owned()),
        (FixtureExpectation::Visual, Err(failure)) => Err(format!(
            "compilation failed:\n{}",
            serde_json::to_string_pretty(&failure.diagnostics).unwrap()
        )),
        (FixtureExpectation::Visual, Ok(generation)) => {
            if case.host == FixtureHost::ProviderMocks
                && (MOCK_ICEBERG_CREATES.load(Ordering::SeqCst) == 0
                    || MOCK_DELTA_CREATES.load(Ordering::SeqCst) == 0)
            {
                return Err(format!(
                    "provider fixture must instantiate both catalog and table factories; got iceberg={}, delta={}",
                    MOCK_ICEBERG_CREATES.load(Ordering::SeqCst),
                    MOCK_DELTA_CREATES.load(Ordering::SeqCst)
                ));
            }
            let registry = Arc::clone(&compiler.options().native_registry);
            let bytes = generation
                .artifact
                .to_bytes()
                .map_err(|error| format!("artifact serialization failed: {error}"))?;
            let round_tripped = CompiledChartArtifact::from_bytes(&bytes, &registry)
                .map_err(|error| format!("artifact deserialization failed: {error}"))?;

            let direct_environment = generation.environment.fork();
            let serialized_environment = generation.environment.fork();
            let direct = evaluate_and_render(
                case,
                &generation.artifact,
                direct_environment.session_context(),
            )
            .await?;
            let serialized = evaluate_and_render(
                case,
                &round_tripped,
                serialized_environment.session_context(),
            )
            .await?;
            assert_case_image_contract(case, &direct)?;
            compare_round_trip(case, &direct, &serialized)?;
            compare_or_bless_image(case, &direct)
        }
    }
}

fn assert_case_image_contract(case: &FixtureCase, image: &RgbaImage) -> Result<(), String> {
    match case.root.as_str() {
        "08_multi_chart_project/geo.avenger" | "09_native_coordinate_families/geo.avenger" => {
            let (width, height) = image.dimensions();
            let x0 = width * 2 / 5;
            let x1 = width * 3 / 5;
            let y0 = height * 2 / 5;
            let y1 = height * 3 / 5;
            let has_centered_symbol = (y0..y1).any(|y| {
                (x0..x1).any(|x| {
                    let [red, green, blue, alpha] = image.get_pixel(x, y).0;
                    alpha > 0
                        && red.max(green).max(blue) - red.min(green).min(blue) >= 24
                        && u16::from(red) + u16::from(green) + u16::from(blue) < 700
                })
            });
            if !has_centered_symbol {
                return Err("geo station must render within the center fifth of the canvas".into());
            }
        }
        "07_inline_view_raster/chart.avenger" => {
            let (width, height) = image.dimensions();
            let has_plot_raster_pixel = (height / 20..height * 9 / 10).any(|y| {
                (width / 8..width * 4 / 5).any(|x| {
                    let [red, green, blue, alpha] = image.get_pixel(x, y).0;
                    alpha > 0
                        && red.max(green).max(blue) - red.min(green).min(blue) >= 32
                        && u16::from(red) + u16::from(green) + u16::from(blue) < 700
                })
            });
            if !has_plot_raster_pixel {
                return Err(
                    "inline raster must render chromatic pixels inside the plot area".into(),
                );
            }
        }
        "09_native_coordinate_families/facet.avenger" => {
            let (width, height) = image.dimensions();
            for (label, x0, x1) in [
                ("east", width / 10, width / 2),
                ("west", width / 2, width * 9 / 10),
            ] {
                let has_symbol = (height / 8..height * 9 / 10).any(|y| {
                    (x0..x1).any(|x| {
                        let [red, green, blue, alpha] = image.get_pixel(x, y).0;
                        alpha > 0
                            && red.max(green).max(blue) - red.min(green).min(blue) >= 32
                            && u16::from(red) + u16::from(green) + u16::from(blue) < 700
                    })
                });
                if !has_symbol {
                    return Err(format!(
                        "facet {label} cell must render chromatic mark pixels"
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn evaluate_and_render(
    case: &FixtureCase,
    artifact: &CompiledChartArtifact,
    context: &datafusion::prelude::SessionContext,
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
    assert_case_scene_contract(case, &evaluated.scene_graph)?;
    let dimensions = CanvasDimensions {
        size: [evaluated.scene_graph.width, evaluated.scene_graph.height],
        scale: DEFAULT_SCALE,
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
        let initial_epoch = session.evaluation_invalidation_epoch();
        let (evaluated, metrics) = session
            .evaluate_with_metrics(EvaluationRequest::new().exact())
            .await
            .map_err(|error| format!("evaluation failed: {error}"))?;
        let materialization_pending = metrics.pipeline.materialization_queued > 0
            || metrics.pipeline.materialization_running > 0
            || session.has_pending_materializations();
        if !materialization_pending {
            return Ok(evaluated);
        }
        if attempt == 3 {
            return Err(
                "view-local materialization did not stabilize after four evaluations".into(),
            );
        }
        for _ in 0..200 {
            if session.evaluation_invalidation_epoch() > initial_epoch
                || !session.has_pending_materializations()
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    unreachable!("bounded materialization loop always returns")
}

fn assert_case_scene_contract(case: &FixtureCase, scene: &SceneGraph) -> Result<(), String> {
    match case.root.as_str() {
        "07_inline_view_raster/chart.avenger" => {
            let nontransparent_pixels = scene
                .marks
                .iter()
                .map(nontransparent_raster_pixels)
                .sum::<usize>();
            if nontransparent_pixels < 6 {
                return Err(format!(
                    "inline raster scene must contain at least six nontransparent source pixels, got {nontransparent_pixels}"
                ));
            }
        }
        "09_native_coordinate_families/facet.avenger" => {
            let mut counts = Vec::new();
            for mark in &scene.marks {
                collect_facet_cell_symbol_counts(mark, &mut counts);
            }
            counts.sort_unstable();
            if counts != [1, 1] {
                return Err(format!(
                    "facet scene must contain one symbol in each of two cells, got {counts:?}"
                ));
            }
        }
        "09_native_coordinate_families/polar.avenger" => {
            let positions = scene_symbol_positions(scene);
            let center = [scene.width * 0.5, scene.height * 0.5];
            if positions.len() != 1 || distance(positions[0], center) < scene.width * 0.2 {
                return Err(format!(
                    "singleton polar radius must place its symbol away from the origin; got {positions:?} around {center:?}"
                ));
            }
        }
        "09_native_coordinate_families/concat.avenger" => {
            let positions = scene_symbol_positions(scene);
            if positions.len() != 2
                || !positions
                    .iter()
                    .any(|position| position[0] > scene.width * 0.9)
            {
                return Err(format!(
                    "concat polar child must place its singleton radial symbol away from the right-child origin; got {positions:?}"
                ));
            }
        }
        "09_native_coordinate_families/subplot.avenger" => {
            let Some((origin, size)) = positioned_subplot_frame(scene) else {
                return Err("subplot scene must contain a named positioned child frame".into());
            };
            let frame_center = [origin[0] + size[0] * 0.5, origin[1] + size[1] * 0.5];
            let scene_center = [scene.width * 0.5, scene.height * 0.5];
            if (frame_center[0] - scene_center[0]).abs() > 1.0
                || (frame_center[1] - scene_center[1]).abs() > 1.0
                || size != [120.0, 90.0]
            {
                return Err(format!(
                    "120-by-90 subplot must be centered at its evaluated parent position; got origin {origin:?}, size {size:?}, scene center {scene_center:?}"
                ));
            }
            let positions = scene_symbol_positions(scene);
            if positions.len() != 1 || distance(positions[0], frame_center) < size[1] * 0.25 {
                return Err(format!(
                    "subplot polar radius must place its symbol away from the child origin; got {positions:?} around {frame_center:?}"
                ));
            }
        }
        "02_sql_pipeline/chart.avenger" => {
            let mut positions = scene_symbol_positions(scene)
                .into_iter()
                .filter(|position| position[0] < scene.width * 0.8)
                .collect::<Vec<_>>();
            positions.sort_by(|left, right| left[0].total_cmp(&right[0]));
            if positions.len() != 2
                || positions[0][0] <= scene.width * 0.15
                || positions[1][0] >= scene.width * 0.8
            {
                return Err(format!(
                    "SQL categorical point positions must remain inside the plot boundaries; got {positions:?}"
                ));
            }
        }
        "04_composed_extension/external_compound.avenger" => {
            let positions = scene_symbol_positions(scene);
            if positions.len() != 1
                || (positions[0][0] - scene.width * 0.55).abs() > scene.width * 0.1
            {
                return Err(format!(
                    "external compound symbol must use the center of its single category band; got {positions:?}"
                ));
            }
        }
        "03_widget_vertical_slice/chart.avenger" => {
            let positions = scene_symbol_positions(scene);
            let plot_positions = positions
                .iter()
                .filter(|position| position[0] < scene.width * 0.75)
                .collect::<Vec<_>>();
            if plot_positions.len() != 1 {
                return Err(format!(
                    "default widget selection must filter the plot to one symbol; got {positions:?}"
                ));
            }
        }
        "05_definition_widget_adjacency/chart.avenger" => {
            let positions = scene_symbol_positions(scene);
            let plot_positions = positions
                .iter()
                .filter(|position| position[1] < scene.height * 0.8)
                .collect::<Vec<_>>();
            if plot_positions.len() != 1 {
                return Err(format!(
                    "expanded dot definition must render exactly one plot symbol; got {positions:?}"
                ));
            }
        }
        "09_native_coordinate_families/parallel.avenger" => {
            let mut lines = Vec::new();
            for mark in &scene.marks {
                collect_parallel_line_y(mark, &mut lines);
            }
            if lines.len() != 2
                || lines.iter().any(|line| line.len() != 2)
                || (lines[0][0] - lines[0][1]) * (lines[1][0] - lines[1][1]) >= 0.0
            {
                return Err(format!(
                    "parallel fixture must render two contrasting rows that cross across the authored dimension order; got {lines:?}"
                ));
            }
        }
        "04_composed_extension/external_coordinate.avenger"
        | "04_composed_extension/external_primitive.avenger"
        | "relative-import/charts/chart.avenger" => {
            let positions = scene_symbol_positions(scene);
            if positions.len() != 1 {
                return Err(format!(
                    "external/imported fixture must render one deterministic symbol; got {positions:?}"
                ));
            }
        }
        "04_composed_extension/mixed_coordinates.avenger" => {
            let positions = scene_symbol_positions(scene);
            if positions.len() != 2
                || !positions
                    .iter()
                    .any(|position| position[0] < scene.width * 0.5)
                || !positions
                    .iter()
                    .any(|position| position[0] > scene.width * 0.5)
            {
                return Err(format!(
                    "mixed external container must render one symbol in each coordinate child; got {positions:?}"
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn scene_symbol_positions(scene: &SceneGraph) -> Vec<[f32; 2]> {
    let mut positions = Vec::new();
    for mark in &scene.marks {
        collect_symbol_positions(mark, scene.origin, &mut positions);
    }
    positions
}

fn collect_symbol_positions(mark: &SceneMark, origin: [f32; 2], positions: &mut Vec<[f32; 2]>) {
    match mark {
        SceneMark::Symbol(symbol) => {
            if symbol.clip {
                positions.extend(
                    symbol
                        .x_iter()
                        .zip(symbol.y_iter())
                        .map(|(x, y)| [origin[0] + x, origin[1] + y]),
                );
            }
        }
        SceneMark::Group(group) => {
            let child_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
            for child in &group.marks {
                collect_symbol_positions(child, child_origin, positions);
            }
        }
        _ => {}
    }
}

fn positioned_subplot_frame(scene: &SceneGraph) -> Option<([f32; 2], [f32; 2])> {
    scene
        .marks
        .iter()
        .find_map(|mark| find_positioned_subplot_frame(mark, scene.origin))
}

fn find_positioned_subplot_frame(
    mark: &SceneMark,
    origin: [f32; 2],
) -> Option<([f32; 2], [f32; 2])> {
    let SceneMark::Group(group) = mark else {
        return None;
    };
    let group_origin = [origin[0] + group.origin[0], origin[1] + group.origin[1]];
    if group.name.starts_with("cartesian_subplot_") {
        let frame = group
            .marks
            .iter()
            .filter_map(|mark| match mark {
                SceneMark::Group(group) => group.pattern_reference_frame.as_ref(),
                _ => None,
            })
            .next()?;
        return Some((group_origin, [frame.width, frame.height]));
    }
    group
        .marks
        .iter()
        .find_map(|child| find_positioned_subplot_frame(child, group_origin))
}

fn distance(left: [f32; 2], right: [f32; 2]) -> f32 {
    ((left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)).sqrt()
}

fn collect_parallel_line_y(mark: &SceneMark, lines: &mut Vec<Vec<f32>>) {
    match mark {
        SceneMark::Line(line) if line.len == 2 => {
            lines.push(line.y_iter().copied().collect());
        }
        SceneMark::Group(group) => {
            for child in &group.marks {
                collect_parallel_line_y(child, lines);
            }
        }
        _ => {}
    }
}

fn collect_facet_cell_symbol_counts(mark: &SceneMark, counts: &mut Vec<usize>) {
    let SceneMark::Group(group) = mark else {
        return;
    };
    if group.name.starts_with("facet_col_") && !group.name.ends_with("_empty") {
        counts.push(group.marks.iter().map(count_symbols).sum());
        return;
    }
    for child in &group.marks {
        collect_facet_cell_symbol_counts(child, counts);
    }
}

fn count_symbols(mark: &SceneMark) -> usize {
    match mark {
        SceneMark::Symbol(symbol) => symbol.len as usize,
        SceneMark::Group(group) => group.marks.iter().map(count_symbols).sum(),
        _ => 0,
    }
}

fn nontransparent_raster_pixels(mark: &SceneMark) -> usize {
    match mark {
        SceneMark::Image(image) if image.name == "uniform_raster_2d" => image
            .image_source_iter()
            .filter_map(|source| source.inline_image())
            .map(|image| {
                image
                    .data
                    .chunks_exact(4)
                    .filter(|pixel| pixel[3] > 0)
                    .count()
            })
            .sum(),
        SceneMark::Group(group) => group.marks.iter().map(nontransparent_raster_pixels).sum(),
        _ => 0,
    }
}

fn compare_round_trip(
    case: &FixtureCase,
    direct: &RgbaImage,
    serialized: &RgbaImage,
) -> Result<(), String> {
    if direct.dimensions() != serialized.dimensions() {
        save_image(&failure_path(case, "direct.png"), direct)?;
        save_image(&failure_path(case, "serialized.png"), serialized)?;
        return Err(format!(
            "serialized artifact dimensions changed from {:?} to {:?}",
            direct.dimensions(),
            serialized.dimensions()
        ));
    }
    let comparison = image_compare::rgba_hybrid_compare(direct, serialized)
        .map_err(|error| format!("round-trip image comparison failed: {error}"))?;
    if comparison.score < ROUND_TRIP_THRESHOLD {
        save_image(&failure_path(case, "direct.png"), direct)?;
        save_image(&failure_path(case, "serialized.png"), serialized)?;
        save_image(
            &failure_path(case, "round_trip_diff.png"),
            &comparison.image.to_color_map().to_rgba8(),
        )?;
        return Err(format!(
            "serialized artifact similarity {:.6} is below {:.6}",
            comparison.score, ROUND_TRIP_THRESHOLD
        ));
    }
    Ok(())
}

fn compare_or_bless_image(case: &FixtureCase, actual: &RgbaImage) -> Result<(), String> {
    let baseline = baseline_path(case);
    if blessing_enabled() {
        save_image(&baseline, actual)?;
    }
    if !baseline.exists() {
        let actual_path = failure_path(case, "actual.png");
        save_image(&actual_path, actual)?;
        return Err(format!(
            "missing baseline {}; actual saved to {}; run {BLESS_ENV}=1 cargo test --release -p avenger-lang-compiler --test fixture_visual_regression",
            baseline.display(),
            actual_path.display()
        ));
    }
    let expected = image::open(&baseline)
        .map_err(|error| format!("failed to read {}: {error}", baseline.display()))?
        .into_rgba8();
    if expected.dimensions() != actual.dimensions() {
        let actual_path = failure_path(case, "actual.png");
        save_image(&actual_path, actual)?;
        return Err(format!(
            "baseline dimensions {:?} differ from actual {:?}; actual saved to {}",
            expected.dimensions(),
            actual.dimensions(),
            actual_path.display()
        ));
    }
    let comparison = image_compare::rgba_hybrid_compare(&expected, actual)
        .map_err(|error| format!("baseline image comparison failed: {error}"))?;
    if comparison.score < BASELINE_THRESHOLD {
        let actual_path = failure_path(case, "actual.png");
        let diff_path = failure_path(case, "diff.png");
        save_image(&actual_path, actual)?;
        save_image(&diff_path, &comparison.image.to_color_map().to_rgba8())?;
        return Err(format!(
            "similarity {:.6} is below {:.6}; actual saved to {}, diff saved to {}",
            comparison.score,
            BASELINE_THRESHOLD,
            actual_path.display(),
            diff_path.display()
        ));
    }
    Ok(())
}

fn compare_or_bless_diagnostic(case: &FixtureCase, actual: &str) -> Result<(), String> {
    let baseline = baseline_path(case);
    if blessing_enabled() {
        if let Some(parent) = baseline.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&baseline, actual)
            .map_err(|error| format!("failed to write {}: {error}", baseline.display()))?;
    }
    let expected = fs::read_to_string(&baseline)
        .map_err(|error| format!("failed to read {}: {error}", baseline.display()))?;
    if expected != actual {
        let actual_path = failure_path(case, "actual.diagnostic.json");
        if let Some(parent) = actual_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        fs::write(&actual_path, actual)
            .map_err(|error| format!("failed to write {}: {error}", actual_path.display()))?;
        return Err(format!(
            "diagnostic baseline changed; actual saved to {}",
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

fn compiler_for(case: &FixtureCase) -> Result<Compiler, String> {
    let root = project_root(case);
    let mut builder = Compiler::builder().project_root(&root);
    match case.host {
        FixtureHost::Stock => {}
        FixtureHost::ComposedRegistry => {
            builder = builder.native_registry(composed_registry());
        }
        FixtureHost::ProviderMocks => {
            let mut catalogs = CatalogFactoryRegistry::default();
            catalogs
                .register("iceberg", Arc::new(MockIcebergFactory))
                .map_err(|error| error.to_string())?;
            let mut tables = TableFactoryRegistry::default();
            tables
                .register("delta", Arc::new(MockDeltaFactory))
                .map_err(|error| error.to_string())?;
            let mut capabilities = DataCapabilities::project();
            capabilities.allow_environment = true;
            capabilities
                .environment_names
                .insert("ICEBERG_TOKEN".to_owned());
            builder = builder
                .catalog_factories(catalogs)
                .table_factories(tables)
                .data_capabilities(capabilities)
                .environment(Arc::new(MapEnvironmentProvider::new([(
                    "ICEBERG_TOKEN".to_owned(),
                    "fixture-token".to_owned(),
                )])));
        }
    }
    builder.build().map_err(|error| error.to_string())
}

struct MockIcebergFactory;

#[async_trait]
impl CatalogFactory for MockIcebergFactory {
    async fn create(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn CatalogProvider>, CatalogFactoryError> {
        MOCK_ICEBERG_CREATES.fetch_add(1, Ordering::SeqCst);
        let catalog = MemoryCatalogProvider::new();
        let schema = MemorySchemaProvider::new();
        schema
            .register_table("remote_events".to_owned(), provider_table())
            .map_err(|error| CatalogFactoryError::Message(error.to_string()))?;
        catalog
            .register_schema("analytics", Arc::new(schema))
            .map_err(|error| CatalogFactoryError::Message(error.to_string()))?;
        Ok(Arc::new(catalog))
    }

    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, CatalogFactoryError> {
        Ok(Some("fixture-iceberg-snapshot".to_owned()))
    }
}

struct MockDeltaFactory;

#[async_trait]
impl TableFactory for MockDeltaFactory {
    async fn create(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Arc<dyn TableProvider>, TableFactoryError> {
        MOCK_DELTA_CREATES.fetch_add(1, Ordering::SeqCst);
        Ok(provider_table())
    }

    async fn dependency_fingerprint(
        &self,
        _options: &serde_json::Value,
        _environment: &CompileEnvironment,
    ) -> Result<Option<String>, TableFactoryError> {
        Ok(Some("fixture-delta-snapshot".to_owned()))
    }
}

fn provider_table() -> Arc<dyn TableProvider> {
    Arc::new(EmptyTable::new(Arc::new(Schema::new(vec![Field::new(
        "provider_value",
        DataType::Int64,
        false,
    )]))))
}

fn optional_channel(name: &str, docs: &str) -> ChannelSchema {
    ChannelSchema {
        name: name.to_owned(),
        required: false,
        shape: ValueShape::SqlExpression,
        docs: docs.to_owned(),
    }
}

fn composed_registry() -> Arc<NativeRegistry> {
    let mut builder = NativeRegistryBuilder::new(1, "fixture-visual-regression");
    builtins::register_bootstrap_builtins(&mut builder).unwrap();
    builder
        .register_mark::<Cartesian>(
            "cartesian",
            "external_hexbin",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "external_hexbin"),
                "A Cartesian primitive mark implemented by a downstream crate.",
            )
            .channel(optional_channel("x", "Optional horizontal position."))
            .channel(optional_channel("y", "Optional vertical position.")),
            |declaration| {
                let mut mark = HexBin::<Cartesian>::new().x(1.0).y(1.0);
                if let Some(name) = &declaration.source_name {
                    mark = mark.id(name.clone());
                }
                Ok(mark.into_plot_marks())
            },
        )
        .unwrap();
    builder
        .register_mark::<Cartesian>(
            "cartesian",
            "external_mean_point",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "external_mean_point"),
                "An aggregate-backed compound mark implemented by a downstream crate.",
            )
            .property(
                "category",
                PropertySchema::optional(
                    ValueShape::SqlExpression,
                    "Optional category expression.",
                ),
            )
            .property(
                "value",
                PropertySchema::optional(ValueShape::SqlExpression, "Optional value expression."),
            ),
            |_| Ok(ExternalMeanPoint::new(lit("all"), lit(2.0)).into_plot_marks()),
        )
        .unwrap();
    builder
        .register_mark::<Cartesian>(
            "cartesian",
            "failing_external_mark",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "failing_external_mark"),
                "A deterministic downstream lowerer failure fixture.",
            ),
            |_| {
                Err(RegistryError::Lowering {
                    kind: "failing_external_mark".to_owned(),
                    message: "intentional downstream lowerer failure".to_owned(),
                })
            },
        )
        .unwrap();

    let isometric = CoordinatePack::new(
        "external_isometric",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
            "A downstream isometric coordinate system.",
        )
        .body_mode(BodyMode::Mixed)
        .property(
            "angle",
            PropertySchema::optional(ValueShape::Number, "Projection angle in radians."),
        ),
        |_| Ok(Isometric::new()),
    )
    .mark(
        "external_cube",
        KindSchema::new(
            NativeKindKey::mark("external_isometric", "external_cube"),
            "A cube mark implemented by a downstream crate.",
        )
        .channel(optional_channel("iso_x", "Optional isometric x position."))
        .channel(optional_channel("iso_y", "Optional isometric y position."))
        .channel(optional_channel("iso_z", "Optional isometric z position.")),
        |declaration| {
            let mut mark = Cube::<Isometric>::new().iso_x(1.0).iso_y(2.0).iso_z(3.0);
            if let Some(name) = &declaration.source_name {
                mark = mark.id(name.clone());
            }
            Ok(mark.into_plot_marks())
        },
    );
    builder.register_coordinate_pack(isometric).unwrap();

    let container = CoordinatePack::new(
        "external_facet_column",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
            "A downstream container for mixed-coordinate child plots.",
        )
        .body_mode(BodyMode::Mixed),
        |_| Ok(HConcat::new()),
    )
    .child_plots(|plot, child, _placement, _parent| Ok(plot.mark(Subplot::<HConcat>::new(child))));
    builder.register_coordinate_pack(container).unwrap();
    Arc::new(builder.build().unwrap())
}
