//! Transient tooltip layout shared by native and web WGPU canvases.

use avenger_eventstream::runtime::RuntimeTooltipPresentation;
use avenger_text::{
    measurement::TextMeasurementConfig,
    types::{FontStyle, FontWeight, TextSyntaxMode},
    TextEngine,
};

#[derive(Clone, Debug)]
pub(crate) struct TooltipOverlayLayout {
    pub presentation: RuntimeTooltipPresentation,
    pub size: [f32; 2],
    pub lines: Vec<TooltipTextLine>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TooltipTextLine {
    pub text: String,
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl TooltipOverlayLayout {
    pub fn new(presentation: RuntimeTooltipPresentation, text_engine: &TextEngine) -> Self {
        let style = presentation.style.clone();
        let line_height = measure(text_engine, "Mg", &style)
            .line_height
            .max(style.font_size);
        let max_inner_width = (style.max_width - style.padding[0] * 2.0).max(40.0);
        let max_label_width = presentation
            .rows
            .iter()
            .map(|row| measure(text_engine, row.label.as_str(), &style).width)
            .fold(0.0_f32, f32::max)
            .min(max_inner_width * 0.42);
        let max_value_width =
            (max_inner_width - max_label_width - style.column_gap).max(style.font_size * 4.0);

        let mut lines = Vec::new();
        let mut widest_value_width = 0.0_f32;
        let mut y = style.padding[1];
        for row in &presentation.rows {
            let value_lines = wrap_text(text_engine, &row.value, max_value_width, &style);
            let value_width = value_lines
                .iter()
                .map(|line| measure(text_engine, line, &style).width)
                .fold(0.0_f32, f32::max);
            widest_value_width = widest_value_width.max(value_width);
            lines.push(TooltipTextLine {
                text: fit_text(text_engine, row.label.as_str(), max_label_width, &style),
                position: [style.padding[0], y],
                color: style.label_foreground,
            });
            for (index, line) in value_lines.iter().enumerate() {
                lines.push(TooltipTextLine {
                    text: line.clone(),
                    position: [
                        style.padding[0] + max_label_width + style.column_gap,
                        y + index as f32 * line_height,
                    ],
                    color: style.foreground,
                });
            }
            y += line_height * value_lines.len().max(1) as f32 + style.row_gap;
        }
        if !presentation.rows.is_empty() {
            y -= style.row_gap;
        }
        y += style.padding[1];

        Self {
            presentation,
            size: [
                (max_label_width + style.column_gap + widest_value_width + style.padding[0] * 2.0)
                    .min(style.max_width)
                    .max(style.padding[0] * 2.0),
                y.max(style.padding[1] * 2.0),
            ],
            lines,
        }
    }

    pub fn origin(&self, canvas_size: [f32; 2]) -> [f32; 2] {
        place_tooltip(
            self.presentation.anchor,
            self.presentation.offset,
            self.size,
            canvas_size,
        )
    }

    /// Whether two layouts can reuse the same shaped text and local geometry.
    /// Pointer anchor, offset, and owner are presentation state and do not
    /// affect the cached tooltip content renderer.
    pub fn same_rendered_content(&self, other: &Self) -> bool {
        self.size == other.size
            && self.lines == other.lines
            && self.presentation.style == other.presentation.style
    }
}

pub(crate) fn place_tooltip(
    anchor: [f32; 2],
    offset: [f32; 2],
    tooltip_size: [f32; 2],
    canvas_size: [f32; 2],
) -> [f32; 2] {
    let mut x = anchor[0] + offset[0];
    if x + tooltip_size[0] > canvas_size[0] {
        x = anchor[0] - offset[0] - tooltip_size[0];
    }
    let mut y = anchor[1] + offset[1];
    if y + tooltip_size[1] > canvas_size[1] {
        y = anchor[1] - offset[1] - tooltip_size[1];
    }
    [
        x.clamp(0.0, (canvas_size[0] - tooltip_size[0]).max(0.0)),
        y.clamp(0.0, (canvas_size[1] - tooltip_size[1]).max(0.0)),
    ]
}

fn wrap_text(
    text_engine: &TextEngine,
    text: &str,
    max_width: f32,
    style: &avenger_eventstream::runtime::RuntimeTooltipStyle,
) -> Vec<String> {
    let mut output = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        for word in source_line.split_whitespace() {
            if measure(text_engine, word, style).width > max_width {
                if !current.is_empty() {
                    output.push(std::mem::take(&mut current));
                }
                output.extend(split_long_word(text_engine, word, max_width, style));
                continue;
            }
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure(text_engine, &candidate, style).width > max_width {
                output.push(std::mem::take(&mut current));
                current.push_str(word);
            } else {
                current = candidate;
            }
        }
        if current.is_empty() {
            output.push(String::new());
        } else {
            output.push(current);
        }
    }
    if output.is_empty() {
        output.push(String::new());
    }
    output
}

fn split_long_word(
    text_engine: &TextEngine,
    word: &str,
    max_width: f32,
    style: &avenger_eventstream::runtime::RuntimeTooltipStyle,
) -> Vec<String> {
    let mut output = Vec::new();
    let mut current = String::new();
    for character in word.chars() {
        let mut candidate = current.clone();
        candidate.push(character);
        if !current.is_empty() && measure(text_engine, &candidate, style).width > max_width {
            output.push(std::mem::take(&mut current));
        }
        current.push(character);
    }
    if !current.is_empty() {
        output.push(current);
    }
    output
}

fn fit_text(
    text_engine: &TextEngine,
    text: &str,
    max_width: f32,
    style: &avenger_eventstream::runtime::RuntimeTooltipStyle,
) -> String {
    if measure(text_engine, text, style).width <= max_width {
        return text.to_string();
    }
    let mut output = String::new();
    for character in text.chars() {
        let candidate = format!("{output}{character}…");
        if measure(text_engine, &candidate, style).width > max_width {
            break;
        }
        output.push(character);
    }
    output.push('…');
    output
}

fn measure(
    text_engine: &TextEngine,
    text: &str,
    style: &avenger_eventstream::runtime::RuntimeTooltipStyle,
) -> avenger_text::measurement::TextBounds {
    text_engine.measure_bounds_with_plain_fallback_or_approx(&TextMeasurementConfig {
        text,
        font: style.font_family.as_str(),
        font_size: style.font_size,
        font_weight: FontWeight::Number(style.font_weight),
        font_style: FontStyle::Normal,
        syntax_mode: TextSyntaxMode::Plain,
        params: avenger_text::empty_label_params(),
        number_locale: None,
        number_locale_specs: None,
        datetime_locale: None,
        datetime_timezone: None,
        datetime_locale_specs: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(anchor: [f32; 2], value: &str) -> TooltipOverlayLayout {
        let text_engine = TextEngine::with_font_resolution(&Default::default())
            .expect("tooltip cache test text engine");
        TooltipOverlayLayout::new(
            avenger_eventstream::runtime::RuntimeTooltipPresentation {
                owner: "owner".into(),
                anchor,
                offset: [12.0, 12.0],
                rows: vec![avenger_eventstream::runtime::RuntimeTooltipRow {
                    label: "Name".into(),
                    value: value.into(),
                }],
                style: Default::default(),
            },
            &text_engine,
        )
    }

    #[test]
    fn placement_flips_and_clamps_at_canvas_edges() {
        assert_eq!(
            place_tooltip([10.0, 10.0], [5.0, 5.0], [30.0, 20.0], [100.0, 100.0]),
            [15.0, 15.0]
        );
        assert_eq!(
            place_tooltip([95.0, 95.0], [5.0, 5.0], [30.0, 20.0], [100.0, 100.0]),
            [60.0, 70.0]
        );
        assert_eq!(
            place_tooltip([2.0, 2.0], [5.0, 5.0], [130.0, 120.0], [100.0, 100.0]),
            [0.0, 0.0]
        );
        assert_eq!(
            place_tooltip([95.0, 10.0], [5.0, 5.0], [30.0, 20.0], [100.0, 100.0]),
            [60.0, 15.0]
        );
        assert_eq!(
            place_tooltip([10.0, 95.0], [5.0, 5.0], [30.0, 20.0], [100.0, 100.0]),
            [15.0, 70.0]
        );
    }

    #[test]
    fn placement_is_invariant_across_device_scale_factors() {
        let expected = [60.0, 70.0];
        for scale in [0.75_f32, 1.0, 2.0, 4.0] {
            let physical_anchor = [95.0 * scale, 95.0 * scale];
            let physical_offset = [5.0 * scale, 5.0 * scale];
            let physical_tooltip = [30.0 * scale, 20.0 * scale];
            let physical_canvas = [100.0 * scale, 100.0 * scale];
            let logical = |value: [f32; 2]| [value[0] / scale, value[1] / scale];
            assert_eq!(
                place_tooltip(
                    logical(physical_anchor),
                    logical(physical_offset),
                    logical(physical_tooltip),
                    logical(physical_canvas),
                ),
                expected,
                "scale factor {scale}"
            );
        }
    }

    #[test]
    fn pointer_move_reuses_shaped_content_but_value_change_does_not() {
        let initial = layout([10.0, 20.0], "Falcon");
        let moved = layout([220.0, 180.0], "Falcon");
        let changed = layout([220.0, 180.0], "Comet");

        assert!(initial.same_rendered_content(&moved));
        assert!(!initial.same_rendered_content(&changed));
    }

    #[test]
    fn measured_width_reserves_the_fixed_label_column_for_every_value() {
        let text_engine = TextEngine::with_font_resolution(&Default::default())
            .expect("tooltip width test text engine");
        let style = avenger_eventstream::runtime::RuntimeTooltipStyle::default();
        let layout = TooltipOverlayLayout::new(
            avenger_eventstream::runtime::RuntimeTooltipPresentation {
                owner: "owner".into(),
                anchor: [10.0, 20.0],
                offset: [12.0, 12.0],
                rows: vec![
                    avenger_eventstream::runtime::RuntimeTooltipRow {
                        label: "Label".into(),
                        value: "Gamma".into(),
                    },
                    avenger_eventstream::runtime::RuntimeTooltipRow {
                        label: "Horizontal".into(),
                        value: "3.0".into(),
                    },
                ],
                style: style.clone(),
            },
            &text_engine,
        );

        let content_right = layout.size[0] - style.padding[0];
        for line in &layout.lines {
            let line_right = line.position[0] + measure(&text_engine, &line.text, &style).width;
            assert!(
                line_right <= content_right + 0.5,
                "line {:?} ends at {line_right}, outside tooltip content edge {content_right}",
                line.text
            );
        }
        assert!(layout.size[0] < style.max_width);
    }
}
