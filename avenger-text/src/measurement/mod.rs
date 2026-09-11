use unicode_segmentation::UnicodeSegmentation;

use crate::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextSyntaxMode};

pub fn truncate_text_to_limit_with<E>(
    text: &str,
    limit: f32,
    mut measure_width: impl FnMut(&str) -> Result<f32, E>,
) -> Result<String, E> {
    if !limit.is_finite() || limit <= 0.0 || text.is_empty() {
        return Ok(text.to_string());
    }

    if measure_width(text)? <= limit {
        return Ok(text.to_string());
    }

    let ellipsis = "\u{2026}";
    if measure_width(ellipsis)? > limit {
        return Ok(String::new());
    }

    let chars = text.graphemes(true).collect::<Vec<_>>();
    let mut low = 0usize;
    let mut high = chars.len();

    while low < high {
        let mid = (low + high).div_ceil(2);
        let candidate = chars[..mid].concat() + ellipsis;

        if measure_width(&candidate)? <= limit {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    Ok(if low == 0 {
        ellipsis.to_string()
    } else {
        chars[..low].concat() + ellipsis
    })
}

/// Prepare source for the width-limit policy. Plain text is ellipsized at
/// grapheme boundaries. Typst markup is compiled intact and clipped after layout.
/// Nonpositive or nonfinite limits mean unconstrained text.
pub fn prepare_text_to_limit_with<E>(
    text: &str,
    syntax: TextSyntaxMode,
    limit: f32,
    measure_width: impl FnMut(&str) -> Result<f32, E>,
) -> Result<String, E> {
    if syntax == TextSyntaxMode::Plain {
        truncate_text_to_limit_with(text, limit, measure_width)
    } else {
        Ok(text.to_string())
    }
}

/// Apply the rich-label width limit to layout metrics. The returned cutoff is
/// in label coordinates: paint at x greater than this value must be clipped.
/// Left glyph overhang and vertical ink are preserved.
pub fn apply_text_limit(
    bounds: &mut TextBounds,
    syntax: TextSyntaxMode,
    limit: f32,
) -> Option<f32> {
    if syntax == TextSyntaxMode::TypstMarkup
        && limit.is_finite()
        && limit > 0.0
        && bounds.width > limit
    {
        bounds.width = limit;
        Some(limit)
    } else {
        None
    }
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
    pub font_weight: FontWeight,
    /// Font style (normal or italic)
    pub font_style: FontStyle,
    /// Whether to interpret the source string as plain text or Typst markup.
    pub syntax_mode: TextSyntaxMode,
    /// Read-only Typst label parameters available to markup labels.
    pub params: &'a avenger_typst_label::LabelParams,
    /// Optional number locale id available to Typst markup functions such as `#numfmt`.
    pub number_locale: Option<&'a str>,
    /// Optional custom number locale specs available to Typst markup functions.
    pub number_locale_specs: Option<&'a crate::NumberLocaleSpecs>,
    /// Optional datetime locale id available to Typst markup functions such as `#datefmt`.
    pub datetime_locale: Option<&'a str>,
    /// Optional default timezone available to Typst markup functions such as `#datefmt`.
    pub datetime_timezone: Option<&'a str>,
    /// Optional custom datetime locale specs available to Typst markup functions.
    pub datetime_locale_specs: Option<&'a crate::DateTimeLocaleSpecs>,
}

/// Configuration needed for font-level metrics.
#[derive(Debug, Clone)]
pub struct FontMetricsConfig<'a> {
    /// Font family name
    pub font: &'a str,
    /// Font size in pixels
    pub font_size: f32,
    /// Font weight (normal, bold, or numeric)
    pub font_weight: FontWeight,
    /// Font style (normal or italic)
    pub font_style: FontStyle,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ellipsis_preserves_combining_sequences_and_emoji_clusters() {
        for cluster in ["e\u{301}", "👩‍👩‍👧‍👦", "🇺🇸"] {
            let text = format!("{cluster}abc");
            let result = truncate_text_to_limit_with(&text, 2.0, |s| {
                Ok::<_, ()>(s.graphemes(true).count() as f32)
            })
            .unwrap();
            assert_eq!(result, format!("{cluster}…"));
        }
    }

    #[test]
    fn truncates_text_to_limit_with_ellipsis() {
        let text = truncate_text_to_limit_with("abcdef", 4.0, |candidate| {
            Ok::<f32, ()>(candidate.chars().count() as f32)
        })
        .unwrap();

        assert_eq!(text, "abc\u{2026}");
    }

    #[test]
    fn returns_empty_text_when_ellipsis_exceeds_limit() {
        let text = truncate_text_to_limit_with("abcdef", 0.5, |candidate| {
            Ok::<f32, ()>(candidate.chars().count() as f32)
        })
        .unwrap();

        assert_eq!(text, "");
    }

    #[test]
    fn propagates_fallible_measurement_errors() {
        let result = truncate_text_to_limit_with("abcdef", 4.0, |_candidate| {
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
