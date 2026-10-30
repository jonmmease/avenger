//! A label's box, which anchors place.

use super::engine::{LabelMetrics, LineMetrics};
use super::frame::Point;

/// The box of a label's text: its lines, from the first line's top to the last line's bottom,
/// with those two padded so that each is at least the font size tall. The box of a label on a
/// path is the bent label's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBounds {
    /// The box's width.
    pub width: f32,
    /// The box's height.
    pub height: f32,
    /// What lies at the label's position.
    pub anchor: BoxAnchor,
}

/// What lies at a label's position, which places its box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoxAnchor {
    /// For straight lines, the point of the box that the label's alignment and baseline name.
    Lines {
        /// The distance from the box's top down to the first line's baseline.
        ascent: f32,
        /// The gap that plain lines leave between their boxes under the label's line height.
        /// A label's line box is its box with half of it above and half below.
        leading: f32,
    },
    /// For a label on a path, its path's origin, from the box's top left. The label's alignment
    /// and baseline have placed its text along the path already.
    Path(Point),
}

impl TextBounds {
    /// A label's box: its lines, with the first line's top and the last line's bottom padded
    /// so that each is at least the font size tall, half the shortfall on either side, and the
    /// gap that plain lines leave between such boxes.
    pub fn new(metrics: &LabelMetrics, font_size: f32) -> Self {
        let lines = LineBox::new(metrics, font_size);
        Self {
            width: metrics.width,
            height: lines.height,
            anchor: BoxAnchor::Lines { ascent: lines.ascent, leading: lines.leading },
        }
    }
}

/// The box of a label's straight lines, as [`TextBounds::new`] pads them.
pub(crate) struct LineBox {
    /// The box's height.
    pub height: f32,
    /// How far below the box's top the top of the label's frame lies.
    pub top: f32,
    /// The distance from the box's top down to the first line's baseline.
    pub ascent: f32,
    /// The gap that plain lines leave between their boxes.
    pub leading: f32,
}

impl LineBox {
    pub fn new(metrics: &LabelMetrics, font_size: f32) -> Self {
        let height =
            |line: Option<&LineMetrics>| line.map_or(0.0, |line| line.bottom - line.top);
        let size = font_size.max(1.0);
        let top = (size - height(metrics.lines.first())).max(0.0) / 2.0;
        let bottom = (size - height(metrics.lines.last())).max(0.0) / 2.0;
        // A label aligns by its first line's baseline.
        let baseline = metrics.lines.first().map_or(0.0, |line| line.baseline);
        Self {
            height: metrics.height + top + bottom,
            top,
            ascent: baseline + top,
            leading: metrics.line_pitch - size,
        }
    }
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
        let lines = |ascent, leading| BoxAnchor::Lines { ascent, leading };

        // A line shorter than the font size gets the rest, half above and half below; the gap
        // between plain lines' boxes is their pitch less the font size.
        let padded = TextBounds::new(&one, 16.0);
        assert_eq!((padded.width, padded.height), (20.0, 16.0));
        assert_eq!(padded.anchor, lines(10.0, 0.5));
        // A line as tall as the font size, as math can be, gets none.
        let tall = TextBounds::new(&one, 10.0);
        assert_eq!(tall.height, 10.0);
        assert_eq!(tall.anchor, lines(7.0, 6.5));

        // Several lines pad the first line's top and the last line's bottom.
        let two = LabelMetrics {
            width: 20.0,
            height: 26.5,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0), line(16.5, 23.5, 26.5)],
        };
        let padded = TextBounds::new(&two, 16.0);
        assert_eq!(padded.height, 32.5);
        assert_eq!(padded.anchor, lines(10.0, 0.5));
    }
}
