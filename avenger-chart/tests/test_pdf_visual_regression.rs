use std::{
    path::{Path, PathBuf},
    process::Command,
};

use avenger_chart::{prelude::*, render::PdfRenderer};
use datafusion::prelude::SessionContext;
use image::RgbaImage;

const ENABLE_PDF_VISUAL_TESTS: &str = "AVENGER_CHART_PDF_VISUAL_TESTS";
const PDF_RASTER_DPI: &str = "144";
const SIMPLE_BAR_BASELINE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/baselines/bar/simple_bar_chart.png"
);
const SIMPLE_BAR_FAILURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/failures_pdf/bar");

#[tokio::test]
async fn pdf_raster_matches_simple_bar_chart_baseline() {
    if std::env::var_os(ENABLE_PDF_VISUAL_TESTS).is_none() {
        eprintln!("skipping PDF visual test; set {ENABLE_PDF_VISUAL_TESTS}=1 to enable");
        return;
    }

    let Some(pdftoppm) = find_pdftoppm() else {
        eprintln!("skipping PDF visual test; pdftoppm was not found on PATH");
        return;
    };

    let ctx = SessionContext::new();
    let compiled = simple_bar_chart(&ctx)
        .await
        .compile(&ctx)
        .await
        .expect("Failed to compile plot");

    let pdf = PdfRenderer::new()
        .render(&compiled, &ctx, None)
        .await
        .expect("Failed to render PDF");
    let actual = rasterize_pdf_with_pdftoppm(&pdf, &pdftoppm);

    compare_to_baseline("simple_bar_chart", SIMPLE_BAR_BASELINE, &pdf, actual, 0.95)
        .expect("PDF raster did not match the simple bar chart baseline");
}

async fn simple_bar_chart(ctx: &SessionContext) -> Plot<Cartesian> {
    let df = ctx
        .sql(
            "SELECT column1 AS category, column2 AS value FROM (VALUES
                ('A', 28.0),
                ('B', 55.0),
                ('C', 43.0),
                ('D', 91.0),
                ('E', 81.0),
                ('F', 53.0),
                ('G', 19.0),
                ('H', 87.0),
                ('I', 52.0)
            )",
        )
        .await
        .expect("Failed to create simple category data");

    Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Rect::new()
                .x_with(col("category"), |c| {
                    c.scale_with::<Band>(|s| {
                        s.domain_discrete(vec![
                            lit("A"),
                            lit("B"),
                            lit("C"),
                            lit("D"),
                            lit("E"),
                            lit("F"),
                            lit("G"),
                            lit("H"),
                            lit("I"),
                        ])
                    })
                    .axis(|a| a.title("Category").grid(false))
                })
                .x2_with(col(":x"), |c| c.band(1.0))
                .y_with(lit(0.0), |c| {
                    c.scale(|s| s.domain((0.0, 100.0)))
                        .axis(|a| a.title("Value").grid(true))
                })
                .y2(col("value"))
                .fill("#4682b4")
                .stroke("#000000")
                .stroke_width(1.0),
        )
        .mark(
            Rect::new()
                .x(0.0)
                .x2(1.0)
                .y(50.0)
                .y2(50.0)
                .stroke("#ff0000")
                .stroke_width(2.0)
                .opacity(0.7),
        )
}

fn find_pdftoppm() -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|entry| entry.join("pdftoppm"))
        .find(|candidate| candidate.is_file())
}

fn rasterize_pdf_with_pdftoppm(pdf: &[u8], pdftoppm: &Path) -> RgbaImage {
    let temp_dir = tempfile::tempdir().expect("Failed to create temp directory");
    let input_pdf = temp_dir.path().join("chart.pdf");
    let output_prefix = temp_dir.path().join("chart");
    let output_png = output_prefix.with_extension("png");
    std::fs::write(&input_pdf, pdf).expect("Failed to write temporary PDF");

    let output = Command::new(pdftoppm)
        .arg("-png")
        .arg("-r")
        .arg(PDF_RASTER_DPI)
        .arg("-singlefile")
        .arg(&input_pdf)
        .arg(&output_prefix)
        .output()
        .expect("Failed to run pdftoppm");

    assert!(
        output.status.success(),
        "pdftoppm failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    image::open(&output_png)
        .unwrap_or_else(|err| {
            panic!(
                "Failed to load PDF raster '{}': {err}",
                output_png.display()
            )
        })
        .into_rgba8()
}

fn compare_to_baseline(
    name: &str,
    baseline_path: &str,
    pdf: &[u8],
    actual: RgbaImage,
    threshold: f64,
) -> Result<(), String> {
    let expected = image::open(baseline_path)
        .map_err(|e| format!("Failed to load baseline image '{baseline_path}': {e}"))?
        .into_rgba8();

    if expected.dimensions() != actual.dimensions() {
        save_failures(name, pdf, &actual, None)?;
        return Err(format!(
            "Image dimensions differ. Expected {:?}, actual {:?}. Failures saved under {}",
            expected.dimensions(),
            actual.dimensions(),
            SIMPLE_BAR_FAILURE_DIR
        ));
    }

    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {e}"))?;

    if result.score < threshold {
        let diff = result.image.to_color_map().into_rgba8();
        save_failures(name, pdf, &actual, Some(&diff))?;
        Err(format!(
            "Image similarity {:.4} is below threshold {:.4}. Failures saved under {}",
            result.score, threshold, SIMPLE_BAR_FAILURE_DIR
        ))
    } else {
        Ok(())
    }
}

fn save_failures(
    name: &str,
    pdf: &[u8],
    actual: &RgbaImage,
    diff: Option<&RgbaImage>,
) -> Result<(), String> {
    let failure_dir = Path::new(SIMPLE_BAR_FAILURE_DIR);
    std::fs::create_dir_all(failure_dir)
        .map_err(|e| format!("Failed to create failure directory: {e}"))?;
    std::fs::write(failure_dir.join(format!("{name}.pdf")), pdf)
        .map_err(|e| format!("Failed to save failure PDF: {e}"))?;
    actual
        .save(failure_dir.join(format!("{name}.png")))
        .map_err(|e| format!("Failed to save failure PNG: {e}"))?;
    if let Some(diff) = diff {
        diff.save(failure_dir.join(format!("{name}_diff.png")))
            .map_err(|e| format!("Failed to save failure diff: {e}"))?;
    }
    Ok(())
}
