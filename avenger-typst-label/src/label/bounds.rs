//! A label's box, which anchors place.

use super::engine::{LabelMetrics, LineMetrics};

/// The box of a label's text: its lines, from the first line's top to the last line's bottom,
/// each padded to at least the font size.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// A label's box: its lines, with the first line's top and the last line's bottom padded
    /// so that each is at least the font size tall, half the shortfall on either side, and the
    /// gap that plain lines leave between such boxes.
    pub fn new(metrics: &LabelMetrics, font_size: f32) -> Self {
        let height =
            |line: Option<&LineMetrics>| line.map_or(0.0, |line| line.bottom - line.top);
        let size = font_size.max(1.0);
        let top = (size - height(metrics.lines.first())).max(0.0) / 2.0;
        let bottom = (size - height(metrics.lines.last())).max(0.0) / 2.0;
        let ascent = first_baseline(metrics);
        Self {
            width: metrics.width,
            height: metrics.height + top + bottom,
            ascent: ascent + top,
            descent: metrics.height - ascent + bottom,
            leading: metrics.line_pitch - size,
        }
    }
}

/// The first line's baseline, from the label's top. A label aligns by it.
pub(crate) fn first_baseline(metrics: &LabelMetrics) -> f32 {
    metrics.lines.first().map_or(0.0, |line| line.baseline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_boxes_pad_lines_to_the_font_size() {
        let line = |top: f32, baseline: f32, bottom: f32| LineMetrics {
            left: 0.0,
            right: 20.0,
            top,
            baseline,
            bottom,
        };
        let one = LabelMetrics {
            width: 20.0,
            height: 10.0,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0)],
        };

        // A line shorter than the font size gets the rest, half above and half below; the gap
        // between plain lines' boxes is their pitch less the font size.
        let padded = TextBounds::new(&one, 16.0);
        assert_eq!(
            (padded.width, padded.height, padded.ascent, padded.descent),
            (20.0, 16.0, 10.0, 6.0)
        );
        assert_eq!(padded.leading, 0.5);
        // A line as tall as the font size, as math can be, gets none.
        let tall = TextBounds::new(&one, 10.0);
        assert_eq!((tall.height, tall.ascent, tall.descent), (10.0, 7.0, 3.0));
        assert_eq!(tall.leading, 6.5);

        // Several lines pad the first line's top and the last line's bottom.
        let two = LabelMetrics {
            width: 20.0,
            height: 26.5,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0), line(16.5, 23.5, 26.5)],
        };
        let padded = TextBounds::new(&two, 16.0);
        assert_eq!((padded.height, padded.ascent, padded.descent), (32.5, 10.0, 22.5));
    }
}
