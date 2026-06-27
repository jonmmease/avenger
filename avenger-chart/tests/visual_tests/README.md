# Visual Regression Tests

This directory contains visual regression tests for avenger-chart, using image comparison to detect unintended rendering changes.

## Running Tests

```bash
# Run all visual tests
cargo test --release -p avenger-chart --test visual_regression

# Run a specific test
cargo test --release -p avenger-chart --test visual_regression test_simple_bar_chart

# Run the SVG/resvg parity layer in addition to WGPU visual tests
AVENGER_CHART_SVG_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture

# Run only the SVG/resvg parity layer, reusing committed WGPU PNG baselines
AVENGER_CHART_SVG_BASELINES=only \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture

# Run the PDF/PDFium parity layer in addition to WGPU visual tests
AVENGER_CHART_PDF_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture

# Run only the PDF/PDFium parity layer, reusing committed WGPU PNG baselines
AVENGER_CHART_PDF_BASELINES=only \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture

# Rewrite all committed WGPU PNG baselines from the current renderer output
AVENGER_CHART_BLESS_WGPU_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

## Writing New Tests

Tests are incredibly concise with our helper functions:

```rust
#[tokio::test]
async fn test_my_chart() {
    let df = test_data::simple_categories();
    
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(df)
        // ... configure your plot ...
        .mark(/* ... */);
    
    // Test with default 99.99% tolerance
    // "bar" is the category subdirectory
    assert_visual_match_default(plot, "bar", "my_chart_baseline").await;
}
```

Or with custom tolerance:

```rust
// Use 90% tolerance for tests with more expected variation
assert_visual_match(plot, "bar", "my_chart_baseline", 0.90).await;
```

The assertion functions will panic with a descriptive message if the test fails.

## Updating Baselines

When visual changes are intentional:

To rewrite every committed WGPU PNG baseline from the current renderer output:

```bash
AVENGER_CHART_BLESS_WGPU_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

This replaces each `tests/baselines/{category}/{name}.png` as the visual test
runs, including images that would already have passed against the old baseline.
Review the changed PNGs, then commit them in focused chunks.

For one-off failures:

1. Run the failing test - it will generate files in `tests/failures/{category}/`:
   - `{test_name}_actual.png` - The new rendering
   - `{test_name}_diff.png` - Visual diff showing changes

2. Review the actual image to confirm it's correct

3. Copy the actual image to baselines:
   ```bash
   cp tests/failures/bar/my_test_actual.png tests/baselines/bar/my_test.png
   ```

4. Re-run the test to confirm it passes

5. Commit the updated baseline

## Updating SVG Baselines

SVG baselines live beside the WGPU PNG baselines under `tests/baselines_svg/`.
Each visual baseline has two SVG artifacts:

- `tests/baselines_svg/{category}/{name}.svg` - the generated SVG string
- `tests/baselines_svg/{category}/{name}.png` - the `resvg` rasterization

To generate or refresh all SVG artifacts:

```bash
AVENGER_CHART_SVG_BASELINES=only AVENGER_CHART_BLESS_SVG_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

To validate both WGPU and SVG outputs:

```bash
AVENGER_CHART_SVG_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

Failures are written to `tests/failures_svg/{category}/` with the generated
SVG, the `resvg` PNG, and diffs against the SVG PNG baseline and WGPU PNG
baseline. The SVG-to-WGPU comparison is intentionally a hard failure at 95%
global similarity. Treat failures as parity bugs before relaxing any test.

## Updating PDF Baselines

PDF baselines live beside the WGPU PNG baselines under `tests/baselines_pdf/`.
Each visual baseline has two PDF artifacts:

- `tests/baselines_pdf/{category}/{name}.pdf` - the generated PDF bytes
- `tests/baselines_pdf/{category}/{name}.png` - the PDFium rasterization

PDF tests use the Rust `pdfium-render` bindings. They do not download PDFium
automatically. Install a PDFium dynamic library and either place it where the
system loader can find it or pass its full path with
`AVENGER_CHART_PDFIUM_LIBRARY_PATH`.

For local macOS arm64 testing, one option is:

```bash
mkdir -p target/pdfium
curl -L --fail --show-error \
  https://github.com/bblanchon/pdfium-binaries/releases/latest/download/pdfium-mac-arm64.tgz \
  -o target/pdfium/pdfium-mac-arm64.tgz
tar -xzf target/pdfium/pdfium-mac-arm64.tgz -C target/pdfium
export AVENGER_CHART_PDFIUM_LIBRARY_PATH="$PWD/target/pdfium/lib/libpdfium.dylib"
```

To generate or refresh all PDF artifacts:

```bash
AVENGER_CHART_PDF_BASELINES=only AVENGER_CHART_BLESS_PDF_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

To validate both WGPU and PDF outputs:

```bash
AVENGER_CHART_PDF_BASELINES=1 \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

PDF visual baselines use the direct `krilla` renderer by default. To make the
choice explicit while debugging:

```bash
AVENGER_CHART_PDF_RENDERER=krilla \
AVENGER_CHART_PDF_BASELINES=only \
AVENGER_CHART_PDFIUM_LIBRARY_PATH="$PWD/target/pdfium/lib/libpdfium.dylib" \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

To measure PDFium-vs-WGPU scores while auditing renderer parity:

```bash
AVENGER_CHART_PDF_BASELINES=only \
AVENGER_CHART_PDF_SCORE_REPORT=target/tests/pdf-wgpu-scores.csv \
  cargo test --release -p avenger-chart --test visual_regression -- --nocapture
```

Failures are written to `tests/failures_pdf/{category}/` with the generated
PDF, the PDFium PNG, and diffs against the PDFium PNG baseline. PDF-to-WGPU
scores are useful for audits, but the direct `krilla` renderer has different
text/vector rasterization behavior from WGPU, so those scores are reported
rather than used as a global pass/fail gate.

The committed PDF is kept as a first-class export artifact for review, but the
suite does not compare PDF bytes directly. Embedded font resources and subsets
can be ordered differently across processes while producing the same PDFium
raster. Deterministic validation therefore comes from the PDFium PNG baseline.
The PDFium PNG baseline threshold is `0.998`, which is tight enough to catch
visible PDF output changes while allowing tiny PDFium raster variance observed
in a few text-heavy charts.

## Directory Structure

```
visual_tests/
├── baselines/           # Expected images (committed to git)
│   ├── bar/            # Bar chart baselines
│   ├── line/           # Line chart baselines (future)
│   └── scatter/        # Scatter plot baselines (future)
├── failures/           # Test failures and diffs (gitignored)
│   ├── bar/
│   ├── line/
│   └── scatter/
├── helpers.rs          # Rendering and comparison utilities
├── test_data.rs        # Reusable data generation functions
├── bar_charts.rs       # Bar chart tests
├── line_charts.rs      # Line chart tests (future)
└── scatter_plots.rs    # Scatter plot tests (future)
```

## Tolerance Levels

- `0.98` - Text-heavy visualizations (use `VisualTestConfig::text_heavy()`)
- `0.95` - Default for most tests
- `0.93` - Complex graphics with expected variation
- `0.90` - CI environments or tests with high variation
