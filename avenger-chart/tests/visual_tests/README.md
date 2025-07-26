# Visual Regression Tests

This directory contains visual regression tests for avenger-chart, using image comparison to detect unintended rendering changes.

## Running Tests

```bash
# Run all visual tests
cargo test --test visual_regression

# Run a specific test
cargo test --test visual_regression test_simple_bar_chart
```

## Writing New Tests

Tests are incredibly concise with our helper functions:

```rust
#[tokio::test]
async fn test_my_chart() {
    let plot = Plot::new(Cartesian)
        .preferred_size(400.0, 300.0)
        .data(test_data::simple_categories().unwrap())
        // ... configure your plot ...
        .mark(/* ... */);
    
    // Test with default 95% tolerance
    assert_visual_match_default(plot, "my_chart_baseline")
        .await
        .expect("Visual test failed");
}
```

Or with custom tolerance:

```rust
// Use 90% tolerance for tests with more expected variation
assert_visual_match(plot, "my_chart_baseline", 0.90)
    .await
    .expect("Visual test failed");
```

## Updating Baselines

When visual changes are intentional:

1. Run the failing test - it will generate files in `tests/failures/`:
   - `{test_name}_actual.png` - The new rendering
   - `{test_name}_diff.png` - Visual diff showing changes

2. Review the actual image to confirm it's correct

3. Copy the actual image to baselines:
   ```bash
   cp tests/failures/my_test_actual.png tests/baselines/my_test.png
   ```

4. Re-run the test to confirm it passes

5. Commit the updated baseline

## Directory Structure

- `baselines/` - Expected images (committed to git)
- `failures/` - Test failures and diffs (gitignored)
- `helpers.rs` - Rendering and comparison utilities
- `test_data.rs` - Reusable data generation functions
- `*_tests.rs` - Test files organized by chart type

## Tolerance Levels

- `0.98` - Text-heavy visualizations (use `VisualTestConfig::text_heavy()`)
- `0.95` - Default for most tests
- `0.93` - Complex graphics with expected variation
- `0.90` - CI environments or tests with high variation