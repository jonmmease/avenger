use crate::types::{FontStyleSpec, FontWeightSpec, TextAlignSpec, TextBaselineSpec};

extern crate lazy_static;

#[cfg(feature = "cosmic-text")]
pub mod cosmic;

/// Core trait for text measurement functionality
pub trait TextMeasurer: Send + Sync {
    /// Measures the bounding dimensions for a text string with given configuration
    fn measure_text_bounds(
        &self,
        config: &TextMeasurementConfig,
        dimensions: &[f32; 2],
    ) -> TextBounds;
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
    pub font_weight: &'a FontWeightSpec,
    /// Font style (normal or italic)
    pub font_style: &'a FontStyleSpec,
}

/// Results from text measurement
#[derive(Debug, Clone)]
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
        align: &TextAlignSpec,
        baseline: &TextBaselineSpec,
    ) -> [f32; 2] {
        let x = match align {
            TextAlignSpec::Left => position[0],
            TextAlignSpec::Center => position[0] - self.width / 2.0,
            TextAlignSpec::Right => position[0] - self.width,
        };

        let y = match baseline {
            TextBaselineSpec::Alphabetic => position[1] - self.ascent,
            TextBaselineSpec::Top => position[1],
            TextBaselineSpec::Middle => position[1] - self.height / 2.0,
            TextBaselineSpec::Bottom => position[1] - self.height,
            TextBaselineSpec::LineTop => position[1],
            TextBaselineSpec::LineBottom => position[1] - self.line_height,
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
            line_height: 25.0,
        };

        // Test left alignment
        let origin =
            bounds.calculate_origin([10.0, 10.0], &TextAlignSpec::Left, &TextBaselineSpec::Top);
        assert_eq!(origin, [10.0, 10.0]);

        // Test center alignment
        let origin = bounds.calculate_origin(
            [10.0, 10.0],
            &TextAlignSpec::Center,
            &TextBaselineSpec::Middle,
        );
        assert_eq!(origin, [-40.0, 0.0]);

        // Test right alignment
        let origin = bounds.calculate_origin(
            [10.0, 10.0],
            &TextAlignSpec::Right,
            &TextBaselineSpec::Bottom,
        );
        assert_eq!(origin, [-90.0, -10.0]);

        // Test alphabetic baseline
        let origin = bounds.calculate_origin(
            [10.0, 10.0],
            &TextAlignSpec::Left,
            &TextBaselineSpec::Alphabetic,
        );
        assert_eq!(origin, [10.0, 15.0]);
    }
}
