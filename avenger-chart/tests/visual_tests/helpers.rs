// Helper functions for visual tests

use crate::tracing::try_init_tracing;
use avenger_chart::plot::{CompiledPlot, EvaluationRequest, PlotSessionOptions};
use avenger_chart::render::{EvaluatedPlot, EvaluationOptions};
use avenger_common::canvas::CanvasDimensions;
use avenger_image::{
    ImageResourceCache, ImageResourceLoadOptions, ImageResourceResolver,
    load_image_resource_requests_blocking,
};
use avenger_scenegraph::image_resources::resolve_ready_image_resources;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_svg::{SvgRenderOptions, SvgRenderer};
use avenger_text::{FontResolutionOptions, MissingFontPolicy};
use avenger_wgpu::{
    canvas::{Canvas, CanvasConfig, PngCanvas},
    image_resources::WgpuImageResourceStatus,
};
use datafusion::common::ScalarValue;
use image::RgbaImage;
use indexmap::IndexMap;
use pdfium_render::prelude::{PdfRenderConfig, Pdfium, PdfiumError};
use std::{
    collections::HashSet,
    fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

/// Default dimensions for test charts
pub const DEFAULT_SCALE: f32 = 2.0;

const SVG_BASELINES_ENV: &str = "AVENGER_CHART_SVG_BASELINES";
const PDF_BASELINES_ENV: &str = "AVENGER_CHART_PDF_BASELINES";
const BLESS_WGPU_BASELINES_ENV: &str = "AVENGER_CHART_BLESS_WGPU_BASELINES";
const BLESS_SVG_BASELINES_ENV: &str = "AVENGER_CHART_BLESS_SVG_BASELINES";
const BLESS_PDF_BASELINES_ENV: &str = "AVENGER_CHART_BLESS_PDF_BASELINES";
const PDF_SCORE_REPORT_ENV: &str = "AVENGER_CHART_PDF_SCORE_REPORT";
const PDF_RENDERER_ENV: &str = "AVENGER_CHART_PDF_RENDERER";
const PDFIUM_LIBRARY_PATH_ENV: &str = "AVENGER_CHART_PDFIUM_LIBRARY_PATH";
const SVG_BASELINE_RESVG_THRESHOLD: f64 = 0.99999;
const SVG_WGPU_BASELINE_THRESHOLD: f64 = 0.95;
const PDF_BASELINE_PDFIUM_THRESHOLD: f64 = 0.998;
const PDF_WGPU_BASELINE_THRESHOLD: f64 = 0.95;

static PDFIUM_RENDER_LOCK: Mutex<()> = Mutex::new(());
static PDF_SCORE_REPORT_LOCK: Mutex<()> = Mutex::new(());
static PDF_SCORE_REPORT_KEYS: OnceLock<Mutex<HashSet<(String, String)>>> = OnceLock::new();

/// Configuration for visual tests
pub struct VisualTestConfig {
    /// Similarity threshold (0.0 to 1.0, where 1.0 is identical)
    pub threshold: f64,
    /// Whether to save difference images on failure
    pub save_diff_on_failure: bool,
}

impl Default for VisualTestConfig {
    fn default() -> Self {
        Self {
            threshold: 0.9999, // 99.99% similarity required by default
            save_diff_on_failure: true,
        }
    }
}

fn failure_image_paths(baseline_path: &str) -> (String, String) {
    let path = Path::new(baseline_path);
    let test_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    // Extract category from path (e.g., "tests/baselines/bar/simple_bar_chart.png" -> "bar")
    let category = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let failures_dir = if category.is_empty() {
        "tests/failures".to_string()
    } else {
        format!("tests/failures/{}", category)
    };

    (failures_dir, test_name.to_string())
}

fn save_actual_failure_image(actual: &RgbaImage, baseline_path: &str) -> Result<String, String> {
    let (failures_dir, test_name) = failure_image_paths(baseline_path);

    std::fs::create_dir_all(&failures_dir)
        .map_err(|e| format!("Failed to create failures directory: {}", e))?;

    let actual_path = format!("{}/{}.png", failures_dir, test_name);
    actual
        .save(&actual_path)
        .map_err(|e| format!("Failed to save actual image: {}", e))?;

    Ok(actual_path)
}

fn serialized_compiled_plot_copy(compiled: &CompiledPlot) -> CompiledPlot {
    let serialized =
        bincode::serialize(compiled).expect("Failed to serialize CompiledPlot with bincode");
    bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode")
}

/// Evaluate a CompiledPlot directly and after a bincode round-trip.
async fn evaluate_compiled_plot_with_serialization(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> (EvaluatedPlot, EvaluatedPlot) {
    let direct_program = serialized_compiled_plot_copy(compiled);
    let mut direct_session = Arc::new(direct_program)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    let direct_result = direct_session
        .evaluate(evaluation_request(params.clone()))
        .await
        .expect("Failed to evaluate plot directly");

    let deserialized = serialized_compiled_plot_copy(compiled);
    let mut serialized_session = Arc::new(deserialized)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    let bincode_result = serialized_session
        .evaluate(evaluation_request(params))
        .await
        .expect("Failed to evaluate plot after bincode deserialization");

    // Log if dimensions differ (but don't panic - serialization might change some aspects)
    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        tracing::warn!(
            direct_width = direct_result.scene_graph.width,
            direct_height = direct_result.scene_graph.height,
            bincode_width = bincode_result.scene_graph.width,
            bincode_height = bincode_result.scene_graph.height,
            "Serialization round-trip changed scene dimensions"
        );
    }

    (direct_result, bincode_result)
}

/// Evaluate a CompiledPlot with explicit options directly and after a bincode round-trip.
async fn evaluate_compiled_plot_with_serialization_and_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
) -> (EvaluatedPlot, EvaluatedPlot) {
    let direct_program = serialized_compiled_plot_copy(compiled);
    let mut direct_session = Arc::new(direct_program)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    let direct_result = direct_session
        .evaluate(evaluation_request(params.clone()).options(options.clone()))
        .await
        .expect("Failed to evaluate plot directly with options");

    let deserialized = serialized_compiled_plot_copy(compiled);
    let mut serialized_session = Arc::new(deserialized)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    let bincode_result = serialized_session
        .evaluate(evaluation_request(params).options(options))
        .await
        .expect("Failed to evaluate plot after bincode deserialization with options");

    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        tracing::warn!(
            direct_width = direct_result.scene_graph.width,
            direct_height = direct_result.scene_graph.height,
            bincode_width = bincode_result.scene_graph.width,
            bincode_height = bincode_result.scene_graph.height,
            "Serialization round-trip changed scene dimensions with options"
        );
    }

    (direct_result, bincode_result)
}

async fn evaluate_compiled_plot(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
) -> EvaluatedPlot {
    let program = serialized_compiled_plot_copy(compiled);
    let mut session = Arc::new(program)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    session
        .evaluate(evaluation_request(params))
        .await
        .expect("Failed to evaluate plot")
}

fn evaluation_request(params: Option<IndexMap<String, ScalarValue>>) -> EvaluationRequest {
    let request = EvaluationRequest::new().exact();
    if let Some(params) = params {
        request.params(params)
    } else {
        request
    }
}

async fn evaluate_compiled_plot_with_serialization_and_session_options(
    compiled: Arc<CompiledPlot>,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: PlotSessionOptions,
) -> (EvaluatedPlot, EvaluatedPlot) {
    let mut direct_session = compiled
        .clone()
        .instantiate(Arc::new(ctx.clone()))
        .with_options(options.clone());
    let direct_result = direct_session
        .evaluate(evaluation_request(params.clone()))
        .await
        .expect("Failed to evaluate plot directly with session options");

    let serialized = bincode::serialize(compiled.as_ref())
        .expect("Failed to serialize CompiledPlot with bincode");
    let deserialized: CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");
    let mut serialized_session = Arc::new(deserialized)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(options);
    let bincode_result = serialized_session
        .evaluate(evaluation_request(params))
        .await
        .expect("Failed to evaluate plot after bincode deserialization with session options");

    if direct_result.scene_graph.width != bincode_result.scene_graph.width
        || direct_result.scene_graph.height != bincode_result.scene_graph.height
    {
        tracing::warn!(
            direct_width = direct_result.scene_graph.width,
            direct_height = direct_result.scene_graph.height,
            bincode_width = bincode_result.scene_graph.width,
            bincode_height = bincode_result.scene_graph.height,
            "Serialization round-trip changed scene dimensions with session options"
        );
    }

    (direct_result, bincode_result)
}

fn scene_graph_with_resolved_image_resources(
    evaluated: &EvaluatedPlot,
    resolver: Option<&dyn ImageResourceResolver>,
) -> SceneGraph {
    if evaluated.resource_requests.is_empty() {
        return evaluated.scene_graph.clone();
    }

    if let Some(resolver) = resolver {
        load_image_resource_requests_blocking(
            resolver,
            &evaluated.resource_requests,
            ImageResourceLoadOptions::default(),
        )
        .expect("Failed to load visual test image resources");
        return resolve_ready_image_resources(&evaluated.scene_graph, resolver)
            .expect("Failed to inline visual test image resources");
    }

    let cache = ImageResourceCache::new();
    load_image_resource_requests_blocking(
        &cache,
        &evaluated.resource_requests,
        ImageResourceLoadOptions::default(),
    )
    .expect("Failed to load visual test image resources");
    resolve_ready_image_resources(&evaluated.scene_graph, &cache)
        .expect("Failed to inline visual test image resources")
}

async fn evaluate_compiled_plot_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
) -> EvaluatedPlot {
    let program = serialized_compiled_plot_copy(compiled);
    let mut session = Arc::new(program)
        .instantiate(Arc::new(ctx.clone()))
        .with_options(visual_session_options());
    session
        .evaluate(evaluation_request(params).options(options))
        .await
        .expect("Failed to evaluate plot with options")
}

pub async fn render_scene_graph_to_wgpu_image(scene_graph: &SceneGraph) -> RgbaImage {
    render_scene_graph_to_wgpu_image_with_config(scene_graph, visual_canvas_config())
        .await
        .0
}

pub async fn render_scene_graph_to_wgpu_image_with_config(
    scene_graph: &SceneGraph,
    config: CanvasConfig,
) -> (RgbaImage, WgpuImageResourceStatus) {
    let dimensions = CanvasDimensions {
        size: [scene_graph.width, scene_graph.height],
        scale: DEFAULT_SCALE,
    };

    let mut canvas = PngCanvas::new(dimensions, config)
        .await
        .expect("Failed to create visual test canvas");
    canvas
        .set_scene(scene_graph)
        .expect("Failed to set visual test scene");
    let image = canvas
        .render()
        .await
        .expect("Failed to render visual test scene");
    let status = canvas.image_resource_status().clone();
    (image, status)
}

/// Helper to get platform-specific baseline path
pub fn get_baseline_path(category: &str, base_name: &str) -> String {
    format!("tests/baselines/{}/{}.png", category, base_name)
}

fn svg_baselines_enabled() -> bool {
    std::env::var_os(SVG_BASELINES_ENV).is_some()
}

fn svg_baselines_only_enabled() -> bool {
    std::env::var_os(SVG_BASELINES_ENV)
        .as_deref()
        .is_some_and(|value| value == "only")
}

fn bless_svg_baselines_enabled() -> bool {
    std::env::var_os(BLESS_SVG_BASELINES_ENV).is_some()
}

fn pdf_baselines_enabled() -> bool {
    std::env::var_os(PDF_BASELINES_ENV).is_some()
}

fn pdf_baselines_only_enabled() -> bool {
    std::env::var_os(PDF_BASELINES_ENV)
        .as_deref()
        .is_some_and(|value| value == "only")
}

fn bless_pdf_baselines_enabled() -> bool {
    std::env::var_os(BLESS_PDF_BASELINES_ENV).is_some()
}

fn pdf_score_report_path() -> Option<PathBuf> {
    std::env::var_os(PDF_SCORE_REPORT_ENV).map(PathBuf::from)
}

fn sidecar_baselines_only_enabled() -> bool {
    svg_baselines_only_enabled() || pdf_baselines_only_enabled()
}

fn bless_wgpu_baselines_enabled() -> bool {
    std::env::var_os(BLESS_WGPU_BASELINES_ENV).is_some()
}

fn write_wgpu_baseline(baseline_path: &str, image: &RgbaImage) {
    save_image_to_path(image, Path::new(baseline_path)).unwrap_or_else(|e| {
        panic!(
            "Failed to write WGPU visual baseline '{}': {e}",
            baseline_path
        )
    });
}

fn svg_baseline_path(category: &str, baseline_name: &str, extension: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("baselines_svg")
        .join(category)
        .join(format!("{baseline_name}.{extension}"))
}

fn svg_failure_path(category: &str, baseline_name: &str, suffix: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("failures_svg")
        .join(category)
        .join(format!("{baseline_name}{suffix}"))
}

fn pdf_baseline_path(category: &str, baseline_name: &str, extension: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("baselines_pdf")
        .join(category)
        .join(format!("{baseline_name}.{extension}"))
}

fn pdf_failure_path(category: &str, baseline_name: &str, suffix: &str) -> PathBuf {
    PathBuf::from("tests")
        .join("failures_pdf")
        .join(category)
        .join(format!("{baseline_name}{suffix}"))
}

fn write_file(path: &Path, bytes: impl AsRef<[u8]>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    fs::write(path, bytes).map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

fn save_image_to_path(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }
    image
        .save(path)
        .map_err(|e| format!("Failed to save {}: {e}", path.display()))
}

fn rasterize_svg(svg: &str) -> Result<RgbaImage, String> {
    let mut options = usvg::Options::default();
    options.fontdb = std::sync::Arc::new(avenger_text::fonts::build_fontdb(
        &svg_visual_font_resolution(),
    ));
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|e| format!("Failed to parse generated SVG: {e}"))?;
    let width = (tree.size().width() * DEFAULT_SCALE).ceil() as u32;
    let height = (tree.size().height() * DEFAULT_SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| format!("Failed to allocate SVG pixmap {width}x{height}"))?;

    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(DEFAULT_SCALE, DEFAULT_SCALE),
        &mut pixmap.as_mut(),
    );

    RgbaImage::from_raw(width, height, pixmap.data().to_vec())
        .ok_or_else(|| "Failed to convert SVG pixmap into an image".to_string())
}

fn bind_pdfium() -> Result<Pdfium, String> {
    if let Some(path) = std::env::var_os(PDFIUM_LIBRARY_PATH_ENV) {
        let path = absolute_pdfium_path(PathBuf::from(path));
        return match Pdfium::bind_to_library(&path) {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(err) => Err(pdfium_setup_error(format!(
                "failed to bind PDFium from {}: {err}",
                path.display()
            ))),
        };
    }

    match Pdfium::bind_to_library(Pdfium::pdfium_platform_library_name_at_path("./")) {
        Ok(bindings) => Ok(Pdfium::new(bindings)),
        Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
        Err(local_err) => match Pdfium::bind_to_system_library() {
            Ok(bindings) => Ok(Pdfium::new(bindings)),
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => Ok(Pdfium::default()),
            Err(system_err) => Err(pdfium_setup_error(format!(
                "failed to bind PDFium beside the test binary: {local_err}; failed to bind system PDFium: {system_err}"
            ))),
        },
    }
}

fn absolute_pdfium_path(path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        return path;
    }

    let workspace_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&path);
    if workspace_path.exists() {
        return workspace_path;
    }

    std::env::current_dir()
        .map(|cwd| cwd.join(&path))
        .unwrap_or(path)
}

fn pdfium_setup_error(detail: String) -> String {
    format!(
        "{detail}. PDF baselines require a PDFium dynamic library. Install PDFium and either make it visible to the system loader or set {PDFIUM_LIBRARY_PATH_ENV}=/path/to/libpdfium.dylib."
    )
}

fn scaled_pdf_dimension(value: f32, axis: &str) -> Result<i32, String> {
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("Invalid PDF {axis} dimension: {value}"));
    }

    let pixels = (value * DEFAULT_SCALE).ceil();
    if pixels > i32::MAX as f32 {
        return Err(format!(
            "PDF {axis} dimension {pixels} exceeds PDFium pixel limits"
        ));
    }

    Ok(pixels as i32)
}

fn rasterize_pdf_with_pdfium(
    pdf: &[u8],
    scene_width: f32,
    scene_height: f32,
) -> Result<RgbaImage, String> {
    let width = scaled_pdf_dimension(scene_width, "width")?;
    let height = scaled_pdf_dimension(scene_height, "height")?;
    let _guard = PDFIUM_RENDER_LOCK
        .lock()
        .map_err(|_| "PDFium render lock was poisoned".to_string())?;
    let pdfium = bind_pdfium()?;
    let document = pdfium
        .load_pdf_from_byte_vec(pdf.to_vec(), None)
        .map_err(|err| format!("Failed to load generated PDF with PDFium: {err}"))?;

    let page_count = document.pages().len();
    if page_count != 1 {
        return Err(format!(
            "Expected generated PDF to contain exactly one page, found {page_count}"
        ));
    }

    let page = document
        .pages()
        .get(0)
        .map_err(|err| format!("Failed to access generated PDF page: {err}"))?;
    let bitmap = page
        .render_with_config(&PdfRenderConfig::new().set_fixed_size(width, height))
        .map_err(|err| format!("Failed to rasterize generated PDF with PDFium: {err}"))?;

    if bitmap.width() != width || bitmap.height() != height {
        return Err(format!(
            "PDFium raster dimensions differ. Expected ({width}, {height}), got ({}, {}).",
            bitmap.width(),
            bitmap.height()
        ));
    }

    let image = bitmap
        .as_image()
        .map_err(|err| format!("Failed to convert PDFium bitmap to image: {err}"))?
        .into_rgba8();

    if image.dimensions() != (width as u32, height as u32) {
        return Err(format!(
            "PDFium image dimensions differ. Expected ({}, {}), got {:?}.",
            width,
            height,
            image.dimensions()
        ));
    }

    Ok(image)
}

fn save_svg_failures(
    category: &str,
    baseline_name: &str,
    svg: &str,
    image: &RgbaImage,
) -> Result<(), String> {
    write_file(
        &svg_failure_path(category, baseline_name, ".svg"),
        svg.as_bytes(),
    )?;
    save_image_to_path(image, &svg_failure_path(category, baseline_name, ".png"))
}

fn save_pdf_failures(
    category: &str,
    baseline_name: &str,
    pdf: &[u8],
    image: &RgbaImage,
) -> Result<(), String> {
    write_file(&pdf_failure_path(category, baseline_name, ".pdf"), pdf)?;
    save_image_to_path(image, &pdf_failure_path(category, baseline_name, ".png"))
}

fn compare_image_with_named_failures(
    baseline_path: &Path,
    actual: &RgbaImage,
    actual_failure_path: &Path,
    diff_failure_path: &Path,
    threshold: f64,
    label: &str,
) -> Result<f64, String> {
    if !baseline_path.exists() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "No {label} baseline found at '{}'. Actual saved to '{}'.",
            baseline_path.display(),
            actual_failure_path.display()
        ));
    }

    let expected = image::open(baseline_path)
        .map_err(|e| {
            format!(
                "Failed to load {label} baseline '{}': {e}",
                baseline_path.display()
            )
        })?
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "{label} dimensions differ. Expected {:?}, actual {:?}. Actual saved to '{}'.",
            expected.dimensions(),
            actual.dimensions(),
            actual_failure_path.display()
        ));
    }

    let result = image_compare::rgba_hybrid_compare(&expected, actual)
        .map_err(|e| format!("{label} image comparison failed: {e}"))?;
    if result.score < threshold {
        save_image_to_path(actual, actual_failure_path)?;
        save_image_to_path(&result.image.to_color_map().into_rgba8(), diff_failure_path)?;
        return Err(format!(
            "{label} image similarity {:.6} is below threshold {:.6}. Actual saved to '{}', diff saved to '{}'.",
            result.score,
            threshold,
            actual_failure_path.display(),
            diff_failure_path.display()
        ));
    }

    Ok(result.score)
}

fn image_similarity_score_with_named_failures(
    baseline_path: &Path,
    actual: &RgbaImage,
    actual_failure_path: &Path,
    label: &str,
) -> Result<f64, String> {
    if !baseline_path.exists() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "No {label} baseline found at '{}'. Actual saved to '{}'.",
            baseline_path.display(),
            actual_failure_path.display()
        ));
    }

    let expected = image::open(baseline_path)
        .map_err(|e| {
            format!(
                "Failed to load {label} baseline '{}': {e}",
                baseline_path.display()
            )
        })?
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_image_to_path(actual, actual_failure_path)?;
        return Err(format!(
            "{label} dimensions differ. Expected {:?}, actual {:?}. Actual saved to '{}'.",
            expected.dimensions(),
            actual.dimensions(),
            actual_failure_path.display()
        ));
    }

    image_compare::rgba_hybrid_compare(&expected, actual)
        .map(|result| result.score)
        .map_err(|e| format!("{label} image comparison failed: {e}"))
}

fn append_pdf_score_report(
    category: &str,
    baseline_name: &str,
    pdf_baseline_score: f64,
    pdf_wgpu_score: f64,
) -> Result<(), String> {
    let Some(path) = pdf_score_report_path() else {
        return Ok(());
    };

    let keys = PDF_SCORE_REPORT_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
    {
        let mut keys = keys
            .lock()
            .map_err(|_| "PDF score report key lock was poisoned".to_string())?;
        let key = (category.to_string(), baseline_name.to_string());
        if !keys.insert(key) {
            return Ok(());
        }
    }

    let _guard = PDF_SCORE_REPORT_LOCK
        .lock()
        .map_err(|_| "PDF score report lock was poisoned".to_string())?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    }

    let write_header = fs::metadata(&path).map(|m| m.len() == 0).unwrap_or(true);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Failed to open PDF score report '{}': {e}", path.display()))?;

    if write_header {
        writeln!(
            file,
            "category,baseline_name,pdf_baseline_score,pdf_wgpu_score"
        )
        .map_err(|e| format!("Failed to write PDF score report header: {e}"))?;
    }

    writeln!(
        file,
        "{},{},{:.8},{:.8}",
        csv_field(category),
        csv_field(baseline_name),
        pdf_baseline_score,
        pdf_wgpu_score
    )
    .map_err(|e| format!("Failed to append PDF score report row: {e}"))
}

fn csv_field(value: &str) -> String {
    if value.contains(|c| matches!(c, ',' | '"' | '\n' | '\r')) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn assert_svg_scene_graph_match(scene_graph: &SceneGraph, category: &str, baseline_name: &str) {
    if !svg_baselines_enabled() {
        return;
    }

    let svg = SvgRenderer::new()
        .with_options(SvgRenderOptions {
            font_resolution: svg_visual_font_resolution(),
            ..Default::default()
        })
        .render_scene_graph(scene_graph)
        .expect("Failed to render SVG visual baseline");
    let svg_image = rasterize_svg(&svg).expect("Failed to rasterize SVG visual baseline");

    let svg_path = svg_baseline_path(category, baseline_name, "svg");
    let svg_png_path = svg_baseline_path(category, baseline_name, "png");
    let svg_failure = svg_failure_path(category, baseline_name, ".svg");
    let svg_png_failure = svg_failure_path(category, baseline_name, ".png");
    let svg_diff_failure = svg_failure_path(category, baseline_name, "_vs_svg_baseline_diff.png");
    let wgpu_diff_failure = svg_failure_path(category, baseline_name, "_vs_wgpu_baseline_diff.png");
    let wgpu_baseline_path = PathBuf::from(get_baseline_path(category, baseline_name));

    if bless_svg_baselines_enabled() {
        write_file(&svg_path, svg.as_bytes()).expect("Failed to write SVG baseline");
        save_image_to_path(&svg_image, &svg_png_path).expect("Failed to write SVG PNG baseline");
    } else {
        if !svg_path.exists() {
            save_svg_failures(category, baseline_name, &svg, &svg_image)
                .expect("Failed to save missing SVG baseline failure");
            panic!(
                "No SVG baseline found at '{}'. Generated SVG saved to '{}'. Generate baselines with {SVG_BASELINES_ENV}=only {BLESS_SVG_BASELINES_ENV}=1 cargo test --release -p avenger-chart --test visual_regression -- --nocapture",
                svg_path.display(),
                svg_failure.display()
            );
        }

        let expected_svg = fs::read_to_string(&svg_path).unwrap_or_else(|e| {
            panic!("Failed to read SVG baseline '{}': {e}", svg_path.display())
        });
        if expected_svg != svg {
            save_svg_failures(category, baseline_name, &svg, &svg_image)
                .expect("Failed to save SVG mismatch failure");
            panic!(
                "SVG baseline '{}' differs from generated SVG. Generated SVG saved to '{}'.",
                svg_path.display(),
                svg_failure.display()
            );
        }

        if let Err(msg) = compare_image_with_named_failures(
            &svg_png_path,
            &svg_image,
            &svg_png_failure,
            &svg_diff_failure,
            SVG_BASELINE_RESVG_THRESHOLD,
            "SVG/resvg",
        ) {
            panic!("SVG baseline '{}' failed: {msg}", baseline_name);
        }
    }

    if let Err(msg) = compare_image_with_named_failures(
        &wgpu_baseline_path,
        &svg_image,
        &svg_png_failure,
        &wgpu_diff_failure,
        SVG_WGPU_BASELINE_THRESHOLD,
        "SVG/WGPU",
    ) {
        panic!("SVG/WGPU baseline '{}' failed: {msg}", baseline_name);
    }
}

fn assert_pdf_scene_graph_match(scene_graph: &SceneGraph, category: &str, baseline_name: &str) {
    if !pdf_baselines_enabled() {
        return;
    }

    let renderer = selected_pdf_renderer();
    let pdf = render_scene_graph_pdf(scene_graph, renderer)
        .unwrap_or_else(|err| panic!("Failed to render {renderer} PDF visual baseline: {err}"));
    let pdf_image = rasterize_pdf_with_pdfium(&pdf, scene_graph.width, scene_graph.height)
        .unwrap_or_else(|err| {
            panic!("Failed to rasterize {renderer} PDF visual baseline with PDFium: {err}")
        });

    let pdf_path = pdf_baseline_path(category, baseline_name, "pdf");
    let pdf_png_path = pdf_baseline_path(category, baseline_name, "png");
    let pdf_failure = pdf_failure_path(category, baseline_name, ".pdf");
    let pdf_png_failure = pdf_failure_path(category, baseline_name, ".png");
    let pdf_diff_failure = pdf_failure_path(category, baseline_name, "_vs_pdf_baseline_diff.png");
    let wgpu_diff_failure = pdf_failure_path(category, baseline_name, "_vs_wgpu_baseline_diff.png");
    let wgpu_baseline_path = PathBuf::from(get_baseline_path(category, baseline_name));

    let mut pdf_baseline_score = 1.0;
    if bless_pdf_baselines_enabled() {
        write_file(&pdf_path, &pdf).expect("Failed to write PDF baseline");
        save_image_to_path(&pdf_image, &pdf_png_path).expect("Failed to write PDF PNG baseline");
    } else {
        if !pdf_path.exists() {
            save_pdf_failures(category, baseline_name, &pdf, &pdf_image)
                .expect("Failed to save missing PDF baseline failure");
            panic!(
                "No PDF baseline found at '{}'. Generated PDF saved to '{}'. Generate baselines with {PDF_BASELINES_ENV}=only {BLESS_PDF_BASELINES_ENV}=1 cargo test --release -p avenger-chart --test visual_regression -- --nocapture",
                pdf_path.display(),
                pdf_failure.display()
            );
        }

        let expected_pdf = fs::read(&pdf_path).unwrap_or_else(|e| {
            panic!("Failed to read PDF baseline '{}': {e}", pdf_path.display())
        });
        if !expected_pdf.starts_with(b"%PDF-") {
            save_pdf_failures(category, baseline_name, &pdf, &pdf_image)
                .expect("Failed to save invalid PDF baseline failure");
            panic!(
                "PDF baseline '{}' does not start with a PDF header. Generated PDF saved to '{}'.",
                pdf_path.display(),
                pdf_failure.display()
            );
        }
        if !pdf.starts_with(b"%PDF-") {
            save_pdf_failures(category, baseline_name, &pdf, &pdf_image)
                .expect("Failed to save invalid generated PDF failure");
            panic!(
                "Generated {renderer} PDF for '{}' does not start with a PDF header. Generated PDF saved to '{}'.",
                baseline_name,
                pdf_failure.display()
            );
        }

        pdf_baseline_score = compare_image_with_named_failures(
            &pdf_png_path,
            &pdf_image,
            &pdf_png_failure,
            &pdf_diff_failure,
            PDF_BASELINE_PDFIUM_THRESHOLD,
            "PDF/PDFium",
        )
        .unwrap_or_else(|msg| panic!("PDF baseline '{}' failed: {msg}", baseline_name));
    }

    let pdf_wgpu_score = if pdf_score_report_path().is_some() || bless_pdf_baselines_enabled() {
        image_similarity_score_with_named_failures(
            &wgpu_baseline_path,
            &pdf_image,
            &pdf_png_failure,
            "PDF/WGPU",
        )
        .unwrap_or_else(|msg| panic!("PDF/WGPU baseline '{}' failed: {msg}", baseline_name))
    } else {
        compare_image_with_named_failures(
            &wgpu_baseline_path,
            &pdf_image,
            &pdf_png_failure,
            &wgpu_diff_failure,
            PDF_WGPU_BASELINE_THRESHOLD,
            "PDF/WGPU",
        )
        .unwrap_or_else(|msg| panic!("PDF/WGPU baseline '{}' failed: {msg}", baseline_name))
    };

    append_pdf_score_report(category, baseline_name, pdf_baseline_score, pdf_wgpu_score)
        .expect("Failed to append PDF score report");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisualPdfRenderer {
    LegacySvg2Pdf,
    #[cfg(feature = "pdf-krilla-visual-tests")]
    KrillaDirect,
}

impl std::fmt::Display for VisualPdfRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LegacySvg2Pdf => f.write_str("svg2pdf"),
            #[cfg(feature = "pdf-krilla-visual-tests")]
            Self::KrillaDirect => f.write_str("krilla"),
        }
    }
}

fn selected_pdf_renderer() -> VisualPdfRenderer {
    match std::env::var(PDF_RENDERER_ENV) {
        Ok(value) if value.eq_ignore_ascii_case("krilla") => {
            #[cfg(feature = "pdf-krilla-visual-tests")]
            {
                VisualPdfRenderer::KrillaDirect
            }
            #[cfg(not(feature = "pdf-krilla-visual-tests"))]
            {
                panic!(
                    "{PDF_RENDERER_ENV}=krilla requires cargo feature `pdf-krilla-visual-tests`"
                );
            }
        }
        Ok(value)
            if value.eq_ignore_ascii_case("svg2pdf") || value.eq_ignore_ascii_case("legacy") =>
        {
            VisualPdfRenderer::LegacySvg2Pdf
        }
        Ok(value) if !value.is_empty() => {
            panic!(
                "unsupported {PDF_RENDERER_ENV}={value:?}; expected `svg2pdf`, `legacy`, or `krilla`"
            );
        }
        _ => VisualPdfRenderer::LegacySvg2Pdf,
    }
}

fn render_scene_graph_pdf(
    scene_graph: &SceneGraph,
    renderer: VisualPdfRenderer,
) -> Result<Vec<u8>, String> {
    match renderer {
        VisualPdfRenderer::LegacySvg2Pdf => avenger_pdf::PdfRenderer::new()
            .with_options(avenger_pdf::PdfRenderOptions {
                font_resolution: pdf_visual_font_resolution(),
                ..Default::default()
            })
            .render_scene_graph(scene_graph)
            .map_err(|err| err.to_string()),
        #[cfg(feature = "pdf-krilla-visual-tests")]
        VisualPdfRenderer::KrillaDirect => render_scene_graph_pdf_krilla(scene_graph),
    }
}

#[cfg(feature = "pdf-krilla-visual-tests")]
fn render_scene_graph_pdf_krilla(scene_graph: &SceneGraph) -> Result<Vec<u8>, String> {
    avenger_pdf_krilla::PdfRenderer::new()
        .with_options(avenger_pdf_krilla::PdfRenderOptions {
            font_resolution: pdf_visual_font_resolution(),
            ..Default::default()
        })
        .render_scene_graph(scene_graph)
        .map_err(|err| err.to_string())
}

fn svg_visual_font_resolution() -> FontResolutionOptions {
    FontResolutionOptions {
        load_system_fonts: true,
        missing_font: MissingFontPolicy::Fallback,
        ..Default::default()
    }
}

fn pdf_visual_font_resolution() -> FontResolutionOptions {
    FontResolutionOptions {
        missing_font: MissingFontPolicy::Fallback,
        ..Default::default()
    }
}

fn visual_session_options() -> PlotSessionOptions {
    PlotSessionOptions::default()
}

fn visual_canvas_config() -> CanvasConfig {
    visual_canvas_config_from(CanvasConfig::default())
}

fn visual_canvas_config_from(config: CanvasConfig) -> CanvasConfig {
    config
}

fn sidecar_session_options(_category: &str) -> PlotSessionOptions {
    PlotSessionOptions::default()
}

const DERIVED_PLOT_SIZE_BASELINES_ENV: &str = "AVENGER_CHART_DERIVED_PLOT_SIZE_BASELINES";

fn canvas_derived_plot_size_enabled() -> bool {
    std::env::var_os(DERIVED_PLOT_SIZE_BASELINES_ENV).is_some()
}

fn canvas_derived_plot_size_only_enabled() -> bool {
    std::env::var_os(DERIVED_PLOT_SIZE_BASELINES_ENV)
        .as_deref()
        .is_some_and(|value| value == "only")
}

fn canvas_derived_plot_size_category(category: &str) -> Option<String> {
    if !canvas_derived_plot_size_enabled() {
        return None;
    }

    match category {
        "facet"
        | "facet_legend_sharing"
        | "facet_legends"
        | "nested_grid"
        | "nested_grid_empty_subplot" => Some(format!("{category}_plot_size_from_canvas")),
        _ => None,
    }
}

/// Compare a rendered image against a baseline
pub fn compare_images(
    baseline_path: &str,
    actual: RgbaImage,
    config: &VisualTestConfig,
) -> Result<(), String> {
    // Check if baseline exists
    if !std::path::Path::new(baseline_path).exists() {
        // Save the actual image to failures directory for review
        let actual_path = save_actual_failure_image(&actual, baseline_path)?;

        // Ensure baseline directory exists for the copy command
        if let Some(baseline_dir) = Path::new(baseline_path).parent() {
            let mkdir_cmd = format!("mkdir -p {}", baseline_dir.display());
            return Err(format!(
                "No baseline image found at '{}'. Generated image saved to '{}'. \
                To accept this as the baseline, run:\n  {}\n  cp {} {}",
                baseline_path, actual_path, mkdir_cmd, actual_path, baseline_path
            ));
        } else {
            return Err(format!(
                "No baseline image found at '{}'. Generated image saved to '{}'. \
                To accept this as the baseline, run: cp {} {}",
                baseline_path, actual_path, actual_path, baseline_path
            ));
        }
    }

    // Load baseline image
    let expected = image::open(baseline_path)
        .map_err(|e| format!("Failed to load baseline image '{}': {}", baseline_path, e))?
        .into_rgba8();

    // Ensure dimensions match
    if expected.dimensions() != actual.dimensions() {
        if config.save_diff_on_failure {
            let actual_path = save_actual_failure_image(&actual, baseline_path)?;
            return Err(format!(
                "Image dimensions don't match. Expected: {:?}, Actual: {:?}. Actual saved to: {}",
                expected.dimensions(),
                actual.dimensions(),
                actual_path
            ));
        } else {
            return Err(format!(
                "Image dimensions don't match. Expected: {:?}, Actual: {:?}",
                expected.dimensions(),
                actual.dimensions()
            ));
        }
    }

    // Compare images using hybrid algorithm (best for visualization)
    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {}", e))?;

    // Check if similarity meets threshold
    if result.score < config.threshold {
        // Save difference image if requested
        if config.save_diff_on_failure {
            let (failures_dir, test_name) = failure_image_paths(baseline_path);
            let diff_path = format!("{}/{}_diff.png", failures_dir, test_name);

            // Save the actual image
            let actual_path = save_actual_failure_image(&actual, baseline_path)?;

            // Save the difference map
            result
                .image
                .to_color_map()
                .save(&diff_path)
                .map_err(|e| format!("Failed to save diff image: {}", e))?;

            Err(format!(
                "Image similarity {:.4} is below threshold {:.4}. Diff saved to: {}, Actual saved to: {}",
                result.score, config.threshold, diff_path, actual_path
            ))
        } else {
            Err(format!(
                "Image similarity {:.4} is below threshold {:.4}",
                result.score, config.threshold
            ))
        }
    } else {
        Ok(())
    }
}

/// Test a CompiledPlot against its baseline with params and tolerance
pub async fn assert_visual_match(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    let params_for_derived_plot_size = params.clone();
    let derived_category = canvas_derived_plot_size_category(category);
    if canvas_derived_plot_size_only_enabled() {
        if let Some(derived_category) = derived_category {
            assert_canvas_derived_plot_size_visual_match(
                compiled,
                ctx,
                params_for_derived_plot_size,
                &derived_category,
                baseline_name,
                tolerance,
            )
            .await;
        }
        return;
    }

    assert_visual_match_baseline_only(compiled, ctx, params, category, baseline_name, tolerance)
        .await;

    if let Some(derived_category) = derived_category {
        assert_canvas_derived_plot_size_visual_match(
            compiled,
            ctx,
            params_for_derived_plot_size,
            &derived_category,
            baseline_name,
            tolerance,
        )
        .await;
    }
}

async fn assert_visual_match_baseline_only(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    assert_visual_match_baseline_only_with_image_resolver(
        compiled,
        ctx,
        params,
        category,
        baseline_name,
        tolerance,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn assert_visual_match_with_image_resolver(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
    resolver: &dyn ImageResourceResolver,
) {
    assert_visual_match_baseline_only_with_image_resolver(
        compiled,
        ctx,
        params,
        category,
        baseline_name,
        tolerance,
        Some(resolver),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn assert_visual_match_wgpu_only_with_canvas_config(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
    canvas_config: CanvasConfig,
) -> (WgpuImageResourceStatus, WgpuImageResourceStatus) {
    try_init_tracing();

    let baseline_path = get_baseline_path(category, baseline_name);
    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization(compiled, ctx, params).await;
    let (direct_image, direct_status) = render_scene_graph_to_wgpu_image_with_config(
        &direct_result.scene_graph,
        visual_canvas_config_from(canvas_config.clone()),
    )
    .await;
    let (serialized_image, serialized_status) = render_scene_graph_to_wgpu_image_with_config(
        &serialized_result.scene_graph,
        visual_canvas_config_from(canvas_config),
    )
    .await;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if bless_wgpu_baselines_enabled() {
        write_wgpu_baseline(&baseline_path, &direct_image);
    }

    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct WGPU-only rendering): {}",
            baseline_name, msg
        );
    }

    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (serialized WGPU-only rendering): {}",
            baseline_name, msg
        );
    }

    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized WGPU-only renders");
    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed WGPU-only rendering"
        );
    }

    (direct_status, serialized_status)
}

#[allow(clippy::too_many_arguments)]
pub async fn assert_visual_match_with_canvas_config_and_sidecars(
    compiled: Arc<CompiledPlot>,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
    canvas_config: CanvasConfig,
) {
    try_init_tracing();
    let session_options = sidecar_session_options(category);

    if sidecar_baselines_only_enabled() {
        let mut session = compiled
            .clone()
            .instantiate(Arc::new(ctx.clone()))
            .with_options(session_options);
        let direct_result = session
            .evaluate(evaluation_request(params))
            .await
            .expect("Failed to evaluate plot with sidecar session options");
        assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        assert_pdf_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        return;
    }

    let baseline_path = get_baseline_path(category, baseline_name);
    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization_and_session_options(
            compiled,
            ctx,
            params,
            session_options,
        )
        .await;
    let direct_image = render_scene_graph_to_wgpu_image_with_config(
        &direct_result.scene_graph,
        visual_canvas_config_from(canvas_config.clone()),
    )
    .await
    .0;
    let serialized_image = render_scene_graph_to_wgpu_image_with_config(
        &serialized_result.scene_graph,
        visual_canvas_config_from(canvas_config),
    )
    .await
    .0;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if bless_wgpu_baselines_enabled() {
        write_wgpu_baseline(&baseline_path, &direct_image);
    }

    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct configured WGPU rendering): {}",
            baseline_name, msg
        );
    }

    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (serialized configured WGPU rendering): {}",
            baseline_name, msg
        );
    }

    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized configured WGPU renders");
    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed configured WGPU rendering"
        );
    }

    assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
    assert_pdf_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
}

#[allow(clippy::too_many_arguments)]
async fn assert_visual_match_baseline_only_with_image_resolver(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
    resolver: Option<&dyn ImageResourceResolver>,
) {
    try_init_tracing();

    let baseline_path = get_baseline_path(category, baseline_name);

    if sidecar_baselines_only_enabled() {
        let direct_result = evaluate_compiled_plot(compiled, ctx, params).await;
        let direct_scene_graph =
            scene_graph_with_resolved_image_resources(&direct_result, resolver);
        assert_svg_scene_graph_match(&direct_scene_graph, category, baseline_name);
        assert_pdf_scene_graph_match(&direct_scene_graph, category, baseline_name);
        return;
    }

    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization(compiled, ctx, params).await;
    let direct_scene_graph = scene_graph_with_resolved_image_resources(&direct_result, resolver);
    let serialized_scene_graph =
        scene_graph_with_resolved_image_resources(&serialized_result, resolver);
    let direct_image = render_scene_graph_to_wgpu_image(&direct_scene_graph).await;
    let serialized_image = render_scene_graph_to_wgpu_image(&serialized_scene_graph).await;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if bless_wgpu_baselines_enabled() {
        write_wgpu_baseline(&baseline_path, &direct_image);
    }

    // Test direct rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct rendering): {}",
            baseline_name, msg
        );
    }

    // Test serialized rendering against baseline
    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (after serialization): {}",
            baseline_name, msg
        );
    }

    // Also verify that direct and serialized produce identical results
    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized renders");

    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed rendering"
        );
    }

    assert_svg_scene_graph_match(&direct_scene_graph, category, baseline_name);
    assert_pdf_scene_graph_match(&direct_scene_graph, category, baseline_name);
}

/// Test a CompiledPlot against its baseline with default tolerance (99.99%)
pub async fn assert_visual_match_default(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match(compiled, ctx, params, category, baseline_name, 0.9999).await
}

pub async fn assert_scene_graph_visual_match(
    scene_graph: &SceneGraph,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    try_init_tracing();
    let baseline_path = get_baseline_path(category, baseline_name);

    if !sidecar_baselines_only_enabled() {
        let image = render_scene_graph_to_wgpu_image(scene_graph).await;
        let config = VisualTestConfig {
            threshold: tolerance,
            save_diff_on_failure: true,
        };
        if bless_wgpu_baselines_enabled() {
            write_wgpu_baseline(&baseline_path, &image);
        }
        if let Err(msg) = compare_images(&baseline_path, image, &config) {
            panic!(
                "Visual test '{}' failed (scene graph rendering): {}",
                baseline_name, msg
            );
        }
    }

    assert_svg_scene_graph_match(scene_graph, category, baseline_name);
    assert_pdf_scene_graph_match(scene_graph, category, baseline_name);
}

pub async fn assert_scene_graph_visual_match_default(
    scene_graph: &SceneGraph,
    category: &str,
    baseline_name: &str,
) {
    assert_scene_graph_visual_match(scene_graph, category, baseline_name, 0.9999).await
}

/// Test a CompiledPlot against its baseline with explicit evaluation options.
pub async fn assert_visual_match_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    if canvas_derived_plot_size_only_enabled() {
        return;
    }

    try_init_tracing();

    let baseline_path = get_baseline_path(category, baseline_name);

    if sidecar_baselines_only_enabled() {
        let direct_result =
            evaluate_compiled_plot_with_options(compiled, ctx, params, options).await;
        assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        assert_pdf_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
        return;
    }

    let (direct_result, serialized_result) =
        evaluate_compiled_plot_with_serialization_and_options(compiled, ctx, params, options).await;
    let direct_image = render_scene_graph_to_wgpu_image(&direct_result.scene_graph).await;
    let serialized_image = render_scene_graph_to_wgpu_image(&serialized_result.scene_graph).await;

    let config = VisualTestConfig {
        threshold: tolerance,
        save_diff_on_failure: true,
    };

    if bless_wgpu_baselines_enabled() {
        write_wgpu_baseline(&baseline_path, &direct_image);
    }

    if let Err(msg) = compare_images(&baseline_path, direct_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (direct rendering): {}",
            baseline_name, msg
        );
    }

    if let Err(msg) = compare_images(&baseline_path, serialized_image.clone(), &config) {
        panic!(
            "Visual test '{}' failed (after serialization): {}",
            baseline_name, msg
        );
    }

    let comparison = image_compare::rgba_hybrid_compare(&direct_image, &serialized_image)
        .expect("Failed to compare direct and serialized renders");

    if comparison.score < 0.99999 {
        tracing::warn!(
            baseline_name = baseline_name,
            similarity = comparison.score,
            "Serialization round-trip changed rendering with options"
        );
    }

    assert_svg_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
    assert_pdf_scene_graph_match(&direct_result.scene_graph, category, baseline_name);
}

/// Test a CompiledPlot against its baseline with options and default tolerance (99.99%).
pub async fn assert_visual_match_default_with_options(
    compiled: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    options: EvaluationOptions,
    category: &str,
    baseline_name: &str,
) {
    assert_visual_match_with_options(
        compiled,
        ctx,
        params,
        options,
        category,
        baseline_name,
        0.9999,
    )
    .await
}

/// Render a canvas-sized faceted chart as a plot-area-sized chart.
///
/// The leaf plot-area size is derived from the final canvas solution. This is useful
/// for checking whether plot-size mode can reproduce the canvas-fit layout when
/// both paths start from the same leaf subplot dimensions.
pub async fn assert_canvas_derived_plot_size_visual_match(
    compiled_canvas: &CompiledPlot,
    ctx: &datafusion::prelude::SessionContext,
    params: Option<IndexMap<String, ScalarValue>>,
    category: &str,
    baseline_name: &str,
    tolerance: f64,
) {
    try_init_tracing();

    let (leaf_plot_width, leaf_plot_height) = compiled_canvas
        .uniform_facet_leaf_plot_area_from_canvas_for_testing(
            ctx,
            params.clone(),
            EvaluationOptions::default(),
        )
        .await
        .expect("Failed to derive uniform leaf plot-area size from canvas solution");

    tracing::info!(
        baseline_name,
        leaf_plot_width,
        leaf_plot_height,
        "Derived leaf plot-area size from canvas solution"
    );

    let serialized =
        bincode::serialize(compiled_canvas).expect("Failed to serialize CompiledPlot with bincode");
    let plot_size_compiled: CompiledPlot =
        bincode::deserialize(&serialized).expect("Failed to deserialize CompiledPlot from bincode");
    let plot_size_compiled =
        plot_size_compiled.with_facet_leaf_plot_area_for_testing(leaf_plot_width, leaf_plot_height);

    assert_visual_match_baseline_only(
        &plot_size_compiled,
        ctx,
        params,
        category,
        baseline_name,
        tolerance,
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    use std::fs;

    #[test]
    fn rasterizes_pdf_with_pdfium_when_pdf_baselines_enabled() {
        if !pdf_baselines_enabled() {
            return;
        }

        let scene_graph = SceneGraph {
            marks: vec![],
            width: 16.0,
            height: 12.0,
            origin: [0.0, 0.0],
        };
        let renderer = selected_pdf_renderer();
        let pdf = render_scene_graph_pdf(&scene_graph, renderer).unwrap_or_else(|err| {
            panic!("empty scene graph should render to {renderer} PDF: {err}")
        });

        let image = rasterize_pdf_with_pdfium(&pdf, scene_graph.width, scene_graph.height)
            .expect("generated PDF should rasterize with PDFium");

        assert_eq!(image.dimensions(), (32, 24));
    }

    #[test]
    fn compare_images_saves_actual_on_dimension_mismatch() {
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let baseline_dir = temp_dir.path().join("baselines/dimension_mismatch");
        fs::create_dir_all(&baseline_dir).expect("failed to create test baseline dir");
        let baseline_path = baseline_dir.join("size_case.png");
        let failure_path = Path::new("tests/failures/dimension_mismatch/size_case.png");

        if let Some(parent) = failure_path.parent() {
            fs::create_dir_all(parent).expect("failed to create failure parent dir");
        }
        let _ = fs::remove_file(failure_path);

        RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 255]))
            .save(&baseline_path)
            .expect("failed to save test baseline");
        let actual = RgbaImage::from_pixel(3, 2, Rgba([255, 0, 0, 255]));

        let err = compare_images(
            baseline_path.to_str().expect("non-utf8 test baseline path"),
            actual,
            &VisualTestConfig::default(),
        )
        .expect_err("dimension mismatch should fail");

        assert!(err.contains("Image dimensions don't match"));
        assert!(err.contains("Actual saved to"));
        assert!(failure_path.exists());

        let saved = image::open(failure_path)
            .expect("saved actual image should be readable")
            .into_rgba8();
        assert_eq!(saved.dimensions(), (3, 2));

        fs::remove_file(failure_path).expect("failed to remove test failure image");
    }
}
