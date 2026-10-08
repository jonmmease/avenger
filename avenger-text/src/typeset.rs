use avenger_common::types::{FontStyle, FontWeight, TextSyntaxMode};
use avenger_typst_label::{
    CompiledLabel, LabelEngine, LabelError, LabelLimits, LabelLineHeight, LabelMetrics,
    LabelOptions, LabelWidth, Lang, LineMetrics, MathStyle, Region, TextDir,
};

use crate::{measurement::TextBounds, types::TextConfig};

/// Settings that an engine's labels share.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LabelSettings {
    /// The style of math, over each label's text style.
    pub math_style: MathStyle,
    pub limits: LabelLimits,
    /// The labels' language, which selects smart quotes and the text direction.
    pub lang: Lang,
    /// The labels' region, which refines the language.
    pub region: Option<Region>,
    /// The labels' base direction. `Auto` takes it from the language.
    pub dir: TextDir,
}

impl Default for LabelSettings {
    fn default() -> Self {
        let text = avenger_typst_label::TextStyle::default();
        Self {
            math_style: MathStyle::default(),
            limits: LabelLimits::default(),
            lang: text.lang,
            region: text.region,
            dir: text.dir,
        }
    }
}

/// Typesets a label: its source as literal text in the plain syntaxes, and as markup otherwise.
pub(crate) fn typeset(
    typst: &LabelEngine,
    settings: &LabelSettings,
    config: &TextConfig,
) -> Result<CompiledLabel, LabelError> {
    let options = label_options(settings, config);
    let label = match config.syntax_mode {
        TextSyntaxMode::Plain | TextSyntaxMode::PlainLines => {
            typst.compile_text(config.text, &options)?
        }
        TextSyntaxMode::TypstMarkup => typst
            .with_params(config.params.clone())
            .compile(config.text, &options)?,
    };
    for warning in &label.warnings {
        tracing::warn!(?warning, "label typesetting warning");
    }
    Ok(label)
}

/// The label crate's options for a label. A width or line height that the label crate would
/// reject means none, since that error has no plain-text fallback.
pub(crate) fn label_options(settings: &LabelSettings, config: &TextConfig) -> LabelOptions {
    let valid = |value: f32| value.is_finite() && value >= 0.0;
    let layout = &config.layout;
    let width = match layout.width {
        LabelWidth::Max(width) | LabelWidth::Fixed(width) if !valid(width) => LabelWidth::Auto,
        width => width,
    };
    let line_height = match layout.line_height {
        LabelLineHeight::Fixed(value) | LabelLineHeight::Relative(value) if !valid(value) => {
            LabelLineHeight::Auto
        }
        line_height => line_height,
    };
    LabelOptions {
        width,
        wrap: layout.wrap,
        align: layout.align,
        line_height,
        max_lines: layout.max_lines,
        ellipsis: layout.ellipsis,
        newline_breaks: config.syntax_mode == TextSyntaxMode::PlainLines,
        text: avenger_typst_label::TextStyle {
            font_family: config.font.to_string(),
            font_size: config.font_size,
            fill: avenger_color::AbsoluteColor::from_rgba(config.color),
            font_weight: typst_font_weight(config.font_weight),
            font_style: typst_font_style(config.font_style),
            lang: settings.lang,
            region: settings.region,
            dir: settings.dir,
        },
        math: settings.math_style.clone(),
        limits: settings.limits,
        ..Default::default()
    }
}

/// The first line's baseline, from the label's top. A label aligns by it.
pub(crate) fn first_baseline(metrics: &LabelMetrics) -> f32 {
    metrics.lines.first().map_or(0.0, |line| line.baseline)
}

/// A label's text box: its lines, with the first line's top and the last line's bottom padded
/// so that each is at least the font size tall, half the shortfall on either side, and the gap
/// that plain lines leave between such boxes.
pub(crate) fn bounds_from_metrics(metrics: &LabelMetrics, font_size: f32) -> TextBounds {
    let height = |line: Option<&LineMetrics>| line.map_or(0.0, |line| line.bottom - line.top);
    let size = font_size.max(1.0);
    let top = (size - height(metrics.lines.first())).max(0.0) / 2.0;
    let bottom = (size - height(metrics.lines.last())).max(0.0) / 2.0;
    let ascent = first_baseline(metrics);
    TextBounds {
        width: metrics.width,
        height: metrics.height + top + bottom,
        ascent: ascent + top,
        descent: metrics.height - ascent + bottom,
        leading: metrics.line_pitch - size,
    }
}

pub(crate) fn typst_font_weight(weight: FontWeight) -> avenger_typst_label::FontWeight {
    avenger_typst_label::FontWeight::from_number(weight.0)
}

pub(crate) fn typst_font_style(style: FontStyle) -> avenger_typst_label::FontStyle {
    match style {
        FontStyle::Normal => avenger_typst_label::FontStyle::Normal,
        FontStyle::Italic => avenger_typst_label::FontStyle::Italic,
        FontStyle::Oblique => avenger_typst_label::FontStyle::Oblique,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::TextLayout;

    #[test]
    fn label_options_take_the_layout() {
        use avenger_typst_label::LabelAlign;
        let options = |syntax_mode, layout| {
            label_options(
                &LabelSettings::default(),
                &TextConfig {
                    syntax_mode,
                    layout,
                    ..Default::default()
                },
            )
        };
        let layout = TextLayout {
            width: LabelWidth::Fixed(80.0),
            wrap: false,
            max_lines: std::num::NonZeroUsize::new(2),
            ellipsis: true,
            line_height: LabelLineHeight::Relative(1.1),
            align: LabelAlign::Center,
        };
        let label = options(TextSyntaxMode::Plain, layout);
        assert_eq!(
            (label.width, label.wrap, label.max_lines, label.ellipsis),
            (layout.width, false, layout.max_lines, true)
        );
        assert_eq!(
            (label.line_height, label.align),
            (layout.line_height, layout.align)
        );
        assert!(!label.newline_breaks);
        assert!(options(TextSyntaxMode::PlainLines, layout).newline_breaks);
        assert!(!options(TextSyntaxMode::TypstMarkup, layout).newline_breaks);

        // Distances the label crate would reject mean none.
        let invalid = TextLayout {
            width: LabelWidth::Max(f32::NAN),
            line_height: LabelLineHeight::Fixed(-1.0),
            ..layout
        };
        let label = options(TextSyntaxMode::Plain, invalid);
        assert_eq!(
            (label.width, label.line_height),
            (LabelWidth::Auto, LabelLineHeight::Auto)
        );
    }

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
        let padded = bounds_from_metrics(&one, 16.0);
        assert_eq!(
            (padded.width, padded.height, padded.ascent, padded.descent),
            (20.0, 16.0, 10.0, 6.0)
        );
        assert_eq!(padded.leading, 0.5);
        // A line as tall as the font size, as math can be, gets none.
        let tall = bounds_from_metrics(&one, 10.0);
        assert_eq!((tall.height, tall.ascent, tall.descent), (10.0, 7.0, 3.0));
        assert_eq!(tall.leading, 6.5);

        // Several lines pad the first line's top and the last line's bottom.
        let two = LabelMetrics {
            width: 20.0,
            height: 26.5,
            line_pitch: 16.5,
            lines: vec![line(0.0, 7.0, 10.0), line(16.5, 23.5, 26.5)],
        };
        let padded = bounds_from_metrics(&two, 16.0);
        assert_eq!(
            (padded.height, padded.ascent, padded.descent),
            (32.5, 10.0, 22.5)
        );
    }
}
