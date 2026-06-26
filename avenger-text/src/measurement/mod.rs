use std::convert::Infallible;

use crate::types::{FontStyle, FontWeight, TextAlign, TextBaseline};

/// Core trait for text measurement functionality
pub trait TextMeasurer: Send + Sync {
    /// Measures the bounding dimensions for a text string with given configuration
    fn measure_text_bounds(&self, config: &TextMeasurementConfig) -> TextBounds;

    /// Measures font-level vertical metrics without depending on a specific glyph outline.
    fn measure_font_metrics(&self, config: &FontMetricsConfig) -> FontMetrics;
}

pub fn truncate_text_to_limit_with(
    text: &str,
    limit: f32,
    mut measure_width: impl FnMut(&str) -> f32,
) -> String {
    try_truncate_text_to_limit_with(text, limit, |candidate| {
        Ok::<f32, Infallible>(measure_width(candidate))
    })
    .expect("infallible measurement should not fail")
}

pub fn try_truncate_text_to_limit_with<E>(
    text: &str,
    limit: f32,
    mut measure_width: impl FnMut(&str) -> Result<f32, E>,
) -> Result<String, E> {
    if limit <= 0.0 || text.is_empty() {
        return Ok(text.to_string());
    }

    if measure_width(text)? <= limit {
        return Ok(text.to_string());
    }

    let ellipsis = "\u{2026}";
    if measure_width(ellipsis)? > limit {
        return Ok(String::new());
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut low = 0usize;
    let mut high = chars.len();

    while low < high {
        let mid = (low + high + 1) / 2;
        let candidate = chars[..mid].iter().collect::<String>() + ellipsis;

        if measure_width(&candidate)? <= limit {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    Ok(if low == 0 {
        ellipsis.to_string()
    } else {
        chars[..low].iter().collect::<String>() + ellipsis
    })
}

/// Configuration needed for text measurement
#[derive(Debug, Clone)]
pub struct TextMeasurementConfig<'a> {
    /// The text string to measure
    pub text: &'a str,
    /// Font family name
    pub font: &'a str,
    /// Font size in pixels
    pub font_size: f32,
    /// Font weight (normal, bold, or numeric)
    pub font_weight: &'a FontWeight,
    /// Font style (normal or italic)
    pub font_style: &'a FontStyle,
}

/// Configuration needed for font-level metrics.
#[derive(Debug, Clone)]
pub struct FontMetricsConfig<'a> {
    /// Font family name
    pub font: &'a str,
    /// Font size in pixels
    pub font_size: f32,
    /// Font weight (normal, bold, or numeric)
    pub font_weight: &'a FontWeight,
    /// Font style (normal or italic)
    pub font_style: &'a FontStyle,
}

/// Font-level vertical metrics, independent of any particular glyph.
#[derive(Debug, Clone)]
pub struct FontMetrics {
    /// Distance from top to baseline
    pub ascent: f32,
    /// Distance from baseline to bottom
    pub descent: f32,
    /// Total font height, ascent plus descent
    pub height: f32,
    /// Extra gap included in normal line spacing
    pub line_gap: f32,
    /// Distance from one line top to the next
    pub line_height: f32,
}

impl FontMetrics {
    pub fn fallback(font_size: f32) -> Self {
        let ascent = font_size * 0.8;
        let descent = font_size * 0.2;
        let height = ascent + descent;
        let line_height = font_size * 1.2;

        Self {
            ascent,
            descent,
            height,
            line_gap: line_height - height,
            line_height,
        }
    }
}

/// Results from text measurement
#[derive(Debug, Clone, PartialEq)]
pub struct TextBounds {
    /// Total width of the text
    pub width: f32,
    /// Total height from top to bottom
    pub height: f32,
    /// Distance from top to baseline
    pub ascent: f32,
    /// Distance from bottom to baseline
    pub descent: f32,
    /// Distance from top to where the top of the next line would be
    pub line_height: f32,
}

impl TextBounds {
    /// Calculate the origin (top-left) point of the text box based on alignment and baseline
    pub fn calculate_origin(
        &self,
        position: [f32; 2],
        align: &TextAlign,
        baseline: &TextBaseline,
    ) -> [f32; 2] {
        let x = match align {
            TextAlign::Left => position[0],
            TextAlign::Center => position[0] - self.width / 2.0,
            TextAlign::Right => position[0] - self.width,
        };

        let y = match baseline {
            TextBaseline::Alphabetic => position[1] - self.ascent,
            TextBaseline::Top => position[1],
            TextBaseline::Middle => position[1] - self.height / 2.0,
            TextBaseline::Bottom => position[1] - self.height,
            TextBaseline::LineTop => position[1],
            TextBaseline::LineBottom => position[1] - self.line_height,
        };

        [x, y]
    }

    pub fn empty() -> Self {
        TextBounds {
            width: 0.0,
            height: 10.0,
            ascent: 10.0 * 0.8,
            descent: 10.0 * 0.2,
            line_height: 10.0 * 1.2,
        }
    }
}

pub fn default_text_measurer() -> impl TextMeasurer {
    crate::typst_text::TypstTextMeasurer::with_config(crate::math::TextMathConfig::default())
        .expect("failed to initialize Typst text measurer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_text_to_limit_with_ellipsis() {
        let text = truncate_text_to_limit_with("abcdef", 4.0, |candidate| {
            candidate.chars().count() as f32
        });

        assert_eq!(text, "abc\u{2026}");
    }

    #[test]
    fn returns_empty_text_when_ellipsis_exceeds_limit() {
        let text = truncate_text_to_limit_with("abcdef", 0.5, |candidate| {
            candidate.chars().count() as f32
        });

        assert_eq!(text, "");
    }

    #[test]
    fn propagates_fallible_measurement_errors() {
        let result = try_truncate_text_to_limit_with("abcdef", 4.0, |_candidate| {
            Err::<f32, _>("measurement failed")
        });

        assert_eq!(result, Err("measurement failed"));
    }

    #[test]
    fn test_text_bounds_calculate_origin() {
        let bounds = TextBounds {
            width: 100.0,
            height: 20.0,
            ascent: 15.0,
            descent: 5.0,
            line_height: 25.0,
        };

        // Test left alignment
        let origin = bounds.calculate_origin([10.0, 10.0], &TextAlign::Left, &TextBaseline::Top);
        assert_eq!(origin, [10.0, 10.0]);

        // Test center alignment
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlign::Center, &TextBaseline::Middle);
        assert_eq!(origin, [-40.0, 0.0]);

        // Test right alignment
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlign::Right, &TextBaseline::Bottom);
        assert_eq!(origin, [-90.0, -10.0]);

        // Test alphabetic baseline
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlign::Left, &TextBaseline::Alphabetic);
        assert_eq!(origin, [10.0, -5.0]);
    }
}
