use crate::types::{FontStyle, FontWeight, TextAlign, TextBaseline, TextLayout, TextSyntaxMode};

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
    /// How the label lays out its lines.
    pub layout: TextLayout,
    /// Read-only Typst label parameters available to markup labels.
    pub params: &'a avenger_typst_label::LabelParams,
    /// Provider selection and locale data for numeric Typst functions such as `#numfmt`.
    pub number_format: Option<&'a std::sync::Arc<dyn crate::NumberFormatProvider>>,
    /// Provider selection, locale data, and timezone for `#datetimefmt`.
    pub datetime_format: Option<&'a std::sync::Arc<dyn crate::DateTimeFormatProvider>>,
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

/// The box of a label's text: its lines, from the first line's top to the last line's bottom,
/// each padded to at least the font size.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBounds {
    /// The box's width.
    pub width: f32,
    /// The box's height.
    pub height: f32,
    /// The distance from the box's top down to the first line's baseline.
    pub ascent: f32,
    /// The distance from the first line's baseline down to the box's bottom.
    pub descent: f32,
    /// The gap that plain lines leave between their boxes under the label's line height. A
    /// label's line box is its box with half of it above and half below.
    pub leading: f32,
}

impl TextBounds {
    /// The top left of the box, for a position that the alignment and baseline anchor.
    ///
    /// Top, Middle and Bottom place the box, LineTop and LineBottom its line box, and
    /// Alphabetic the first line's baseline.
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
            TextBaseline::LineTop => position[1] + self.leading / 2.0,
            TextBaseline::LineBottom => position[1] - self.height - self.leading / 2.0,
        };

        [x, y]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_bounds_calculate_origin() {
        let bounds = TextBounds {
            width: 100.0,
            height: 20.0,
            ascent: 15.0,
            descent: 5.0,
            leading: 4.0,
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

        // The line box has half the leading above the box and half below.
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlign::Left, &TextBaseline::LineTop);
        assert_eq!(origin, [10.0, 12.0]);
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlign::Left, &TextBaseline::LineBottom);
        assert_eq!(origin, [10.0, -12.0]);
    }
}
