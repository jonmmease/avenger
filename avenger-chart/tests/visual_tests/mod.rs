//! Visual regression testing infrastructure for avenger-chart
//!
//! This module provides utilities for comparing rendered charts against baseline images,
//! with support for fuzzy matching to handle cross-platform rendering differences.

pub mod bar_charts;
pub mod helpers;
pub mod test_data;

use image::RgbaImage;
use std::path::Path;

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
            threshold: 0.95, // 95% similarity required by default
            save_diff_on_failure: true,
        }
    }
}

impl VisualTestConfig {
    /// Create config for text-heavy visualizations (requires higher similarity)
    pub fn text_heavy() -> Self {
        Self {
            threshold: 0.98,
            ..Default::default()
        }
    }

    /// Create config for complex graphics (allows more variation)
    pub fn graphics() -> Self {
        Self {
            threshold: 0.93,
            ..Default::default()
        }
    }

    /// Create config for CI environments (most lenient)
    pub fn ci() -> Self {
        Self {
            threshold: 0.90,
            ..Default::default()
        }
    }
}

/// Compare a rendered image against a baseline
pub fn compare_images(
    baseline_path: &str,
    actual: RgbaImage,
    config: &VisualTestConfig,
) -> Result<(), String> {
    // Load baseline image
    let expected = image::open(baseline_path)
        .map_err(|e| format!("Failed to load baseline image '{}': {}", baseline_path, e))?
        .into_rgba8();

    // Ensure dimensions match
    if expected.dimensions() != actual.dimensions() {
        return Err(format!(
            "Image dimensions don't match. Expected: {:?}, Actual: {:?}",
            expected.dimensions(),
            actual.dimensions()
        ));
    }

    // Compare images using hybrid algorithm (best for visualization)
    let result = image_compare::rgba_hybrid_compare(&expected, &actual)
        .map_err(|e| format!("Image comparison failed: {}", e))?;

    // Check if similarity meets threshold
    if result.score < config.threshold {
        // Save difference image if requested
        if config.save_diff_on_failure {
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
            
            let diff_path = format!("{}/{}_diff.png", failures_dir, test_name);
            let actual_path = format!("{}/{}_actual.png", failures_dir, test_name);

            // Save the actual image
            actual
                .save(&actual_path)
                .map_err(|e| format!("Failed to save actual image: {}", e))?;

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

/// Helper to get platform-specific baseline path
pub fn get_baseline_path(category: &str, base_name: &str) -> String {
    // For now, use the same baseline for all platforms
    // Can be extended to use platform-specific baselines if needed
    format!("tests/baselines/{}/{}.png", category, base_name)
}

/// Helper to update baselines when visual changes are intentional
/// This should only be run manually, not in CI
#[allow(dead_code)]
pub fn update_baseline(category: &str, baseline_name: &str, image: &RgbaImage) -> Result<(), String> {
    let baseline_path = get_baseline_path(category, baseline_name);
    image
        .save(&baseline_path)
        .map_err(|e| format!("Failed to update baseline '{}': {}", baseline_path, e))?;
    println!("Updated baseline: {}", baseline_path);
    Ok(())
}

/// Macro to simplify visual test creation
#[macro_export]
macro_rules! visual_test {
    ($name:ident, $render_fn:expr) => {
        #[test]
        fn $name() {
            visual_test!($name, $render_fn, Default::default());
        }
    };
    ($name:ident, $render_fn:expr, $config:expr) => {
        #[test]
        fn $name() {
            use $crate::visual_tests::{compare_images, get_baseline_path};

            let baseline_path = get_baseline_path(stringify!($name));
            let rendered = $render_fn;
            let config = $config;

            compare_images(&baseline_path, rendered, &config)
                .expect(&format!("Visual test '{}' failed", stringify!($name)));
        }
    };
}