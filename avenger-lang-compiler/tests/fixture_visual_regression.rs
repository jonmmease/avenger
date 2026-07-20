use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use avenger_chart::prelude::{
    Cartesian, EvaluationRequest, FacetColumn, FacetColumnSubplotChannels,
    InMemoryNativeWidgetInstanceStore, IntoPlotMark, NativeWidgetDocumentId, NativeWidgetPlotId,
    NativeWidgetRegistry, NativeWidgetRuntimeResources, PlotSessionOptions, Subplot,
};
use avenger_chart_external_test::{
    external_compound_mark::ExternalMeanPoint,
    external_coord_system::{Cube, Isometric},
    external_mark::HexBin,
};
use avenger_chart_lang_registry::{
    CoordinatePack, NativeRegistry, NativeRegistryBuilder, RegistryError, ResolvedValue, builtins,
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
            let registry = Arc::clone(&compiler.options().native_registry);
            let bytes = generation
                .artifact
                .to_bytes()
                .map_err(|error| format!("artifact serialization failed: {error}"))?;
            let round_tripped = CompiledChartArtifact::from_bytes(&bytes, &registry)
                .map_err(|error| format!("artifact deserialization failed: {error}"))?;

            let direct_environment = generation.environment.fork();
            let serialized_environment = generation.environment.fork();
            let direct =
                evaluate_and_render(&generation.artifact, direct_environment.session_context())
                    .await?;
            let serialized =
                evaluate_and_render(&round_tripped, serialized_environment.session_context())
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
        _ => {}
    }
    Ok(())
}

async fn evaluate_and_render(
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
    let evaluated = session
        .evaluate(EvaluationRequest::new())
        .await
        .map_err(|error| format!("evaluation failed: {error}"))?;
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
        |_| Ok(FacetColumn),
    )
    .child_plots(|plot, child, placement, _parent| {
        let column = match placement.properties.get("column") {
            Some(ResolvedValue::Expr(expr)) => expr.clone(),
            Some(ResolvedValue::String(value)) => lit(value.clone()),
            _ => {
                return Err(RegistryError::InvalidPropertyType {
                    property: "column".to_owned(),
                    expected: "a scalar expression".to_owned(),
                });
            }
        };
        Ok(plot.mark(Subplot::<FacetColumn>::new(child).column(column)))
    });
    builder.register_coordinate_pack(container).unwrap();
    Arc::new(builder.build().unwrap())
}
