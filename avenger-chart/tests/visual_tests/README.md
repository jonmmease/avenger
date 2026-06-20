# Visual Regression Tests

This directory contains visual regression tests for avenger-chart, using image comparison to detect unintended rendering changes.

## Running Tests

```bash
# Run all visual tests
cargo test --test visual_regression

# Run a specific test
cargo test --test visual_regression test_simple_bar_chart

# Run the SVG/resvg parity layer in addition to WGPU visual tests
AVENGER_CHART_SVG_BASELINES=1 cargo test --test visual_regression -- --nocapture

# Run only the SVG/resvg parity layer, reusing committed WGPU PNG baselines
AVENGER_CHART_SVG_BASELINES=only cargo test --test visual_regression -- --nocapture
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
    
    // Test with default 95% tolerance
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
  cargo test --test visual_regression -- --nocapture
```

To validate both WGPU and SVG outputs:

```bash
AVENGER_CHART_SVG_BASELINES=1 cargo test --test visual_regression -- --nocapture
```

Failures are written to `tests/failures_svg/{category}/` with the generated
SVG, the `resvg` PNG, and diffs against the SVG PNG baseline and WGPU PNG
baseline. The SVG-to-WGPU comparison is intentionally a hard failure at 90%
global similarity and also applies a local tile-difference guard so narrow
localized regressions, such as flat colorbar gradients, cannot hide inside a
large otherwise-matching image. Treat failures as parity bugs before relaxing
any test.

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
