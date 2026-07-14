use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    error::AvengerTextError,
    math::TextMarkupConfig,
    measurement::{TextBounds, TextMeasurementConfig},
    text_line::{bounds_from_metrics, typeset_line},
    types::TextSyntaxMode,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    #[default]
    Downstream,
    Upstream,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedGlyph {
    pub text_range: Range<usize>,
    pub left: f32,
    pub x_advance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedRun {
    pub byte_range: Range<usize>,
    pub is_rtl: bool,
    pub left: f32,
    pub width: f32,
    pub glyphs: Vec<ShapedGlyph>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShapedLine {
    pub text: String,
    pub bounds: TextBounds,
    pub baseline: f32,
    /// Runs in visual order, with monotonically increasing visual positions.
    pub runs: Vec<ShapedRun>,
}

pub(crate) fn shape_line(
    typst: &avenger_typst_label::LabelEngine,
    math: &TextMarkupConfig,
    config: &TextMeasurementConfig,
) -> Result<ShapedLine, AvengerTextError> {
    if config.syntax_mode != TextSyntaxMode::Plain {
        return Err(AvengerTextError::InternalError(
            "editable shaped lines require plain text syntax".to_string(),
        ));
    }
    let result = typeset_line(
        typst,
        &math.plain_text(),
        config.text,
        config.font,
        config.font_size,
        config.font_weight,
        config.font_style,
        [0.0, 0.0, 0.0, 1.0],
        config.params,
        config.number_locale,
        config.number_locale_specs,
        config.datetime_locale,
        config.datetime_timezone,
        config.datetime_locale_specs,
    )?;
    let bounds = bounds_from_metrics(result.label.metrics, config.font_size, false);
    let mut runs = Vec::new();
    collect_plain_runs(&result.label.frame.items, config.text, &mut runs)?;
    runs.sort_by(|a, b| a.left.total_cmp(&b.left));

    Ok(ShapedLine {
        text: config.text.to_string(),
        baseline: bounds.ascent,
        bounds,
        runs,
    })
}

fn collect_plain_runs(
    items: &[(
        avenger_typst_label::Point,
        avenger_typst_label::LabelFrameItem,
    )],
    source: &str,
    runs: &mut Vec<ShapedRun>,
) -> Result<(), AvengerTextError> {
    for (point, item) in items {
        match item {
            avenger_typst_label::LabelFrameItem::Text(item)
                if item.kind == avenger_typst_label::TextItemKind::Plain =>
            {
                if !point.x.is_finite() || !item.metrics.width.is_finite() {
                    return Err(invalid_shape("run geometry is not finite"));
                }
                let mut glyphs = item
                    .glyphs
                    .iter()
                    .map(|glyph| {
                        let text_range = valid_range(source, glyph.text_range.clone())
                            .ok_or_else(|| invalid_shape("glyph range is not source-relative"))?;
                        let right = glyph.transform.tx + glyph.x_advance;
                        if !glyph.transform.tx.is_finite() || !right.is_finite() {
                            return Err(invalid_shape("glyph geometry is not finite"));
                        }
                        Ok(ShapedGlyph {
                            text_range,
                            left: glyph.transform.tx.min(right),
                            x_advance: (right - glyph.transform.tx).abs(),
                        })
                    })
                    .collect::<Result<Vec<_>, AvengerTextError>>()?;
                glyphs.sort_by(|a, b| a.left.total_cmp(&b.left));
                let glyphs = merge_cluster_glyphs(glyphs);
                runs.push(ShapedRun {
                    byte_range: valid_range(source, item.byte_range.clone())
                        .ok_or_else(|| invalid_shape("run range is not source-relative"))?,
                    is_rtl: item.is_rtl,
                    left: point.x,
                    width: item.metrics.width.max(0.0),
                    glyphs,
                });
            }
            avenger_typst_label::LabelFrameItem::Group(group) => {
                collect_plain_runs(&group.items, source, runs)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn invalid_shape(message: &str) -> AvengerTextError {
    AvengerTextError::InternalError(format!("invalid editable shaped line: {message}"))
}

fn merge_cluster_glyphs(glyphs: Vec<ShapedGlyph>) -> Vec<ShapedGlyph> {
    let mut merged: Vec<ShapedGlyph> = Vec::new();
    for glyph in glyphs {
        if let Some(existing) = merged
            .iter_mut()
            .find(|existing| existing.text_range == glyph.text_range)
        {
            let left = existing.left.min(glyph.left);
            let right = (existing.left + existing.x_advance).max(glyph.left + glyph.x_advance);
            existing.left = left;
            existing.x_advance = right - left;
        } else {
            merged.push(glyph);
        }
    }
    merged.sort_by(|a, b| a.left.total_cmp(&b.left));
    merged
}

fn valid_range(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    (range.start <= range.end
        && range.end <= text.len()
        && text.is_char_boundary(range.start)
        && text.is_char_boundary(range.end))
    .then_some(range)
}

pub fn byte_offset_for_x(line: &ShapedLine, x: f32) -> (usize, Affinity) {
    let Some(first) = line.runs.first() else {
        return (0, Affinity::Downstream);
    };
    let Some(last) = line.runs.last() else {
        return (0, Affinity::Downstream);
    };
    if x <= first.left {
        return visual_run_edge(first, true);
    }
    if x >= last.left + last.width {
        return visual_run_edge(last, false);
    }

    let run = line
        .runs
        .iter()
        .find(|run| x <= run.left + run.width)
        .unwrap_or(last);
    if run.glyphs.is_empty() {
        return visual_run_edge(run, x <= run.left + run.width * 0.5);
    }
    let glyph = run
        .glyphs
        .iter()
        .find(|glyph| x <= glyph.left + glyph.x_advance)
        .unwrap_or_else(|| run.glyphs.last().expect("non-empty checked above"));
    offset_in_cluster(&line.text, glyph, run.is_rtl, x)
}

fn visual_run_edge(run: &ShapedRun, left: bool) -> (usize, Affinity) {
    let offset = match (run.is_rtl, left) {
        (false, true) | (true, false) => run.byte_range.start,
        (false, false) | (true, true) => run.byte_range.end,
    };
    (offset, affinity_for_offset(run, offset))
}

fn affinity_for_offset(run: &ShapedRun, offset: usize) -> Affinity {
    if offset == run.byte_range.end {
        Affinity::Upstream
    } else {
        Affinity::Downstream
    }
}

fn offset_in_cluster(text: &str, glyph: &ShapedGlyph, is_rtl: bool, x: f32) -> (usize, Affinity) {
    let boundaries = grapheme_boundaries_in(text, glyph.text_range.clone());
    if boundaries.len() < 2 || glyph.x_advance <= f32::EPSILON {
        return (
            glyph.text_range.start,
            if is_rtl {
                Affinity::Upstream
            } else {
                Affinity::Downstream
            },
        );
    }
    let slots = boundaries.len() - 1;
    let fraction = ((x - glyph.left) / glyph.x_advance).clamp(0.0, 1.0);
    let visual_index = (fraction * slots as f32 + 0.5).floor() as usize;
    let logical_index = if is_rtl {
        slots.saturating_sub(visual_index.min(slots))
    } else {
        visual_index.min(slots)
    };
    let offset = boundaries[logical_index];
    (
        offset,
        if offset == glyph.text_range.end {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        },
    )
}

pub fn cursor_rect_for_offset(line: &ShapedLine, offset: usize, affinity: Affinity) -> TextRect {
    let offset = safe_grapheme_offset(&line.text, offset);
    let run = select_run_for_offset(line, offset, affinity);
    let x = run.map_or(0.0, |run| x_for_offset_in_run(line, run, offset));
    TextRect {
        x,
        y: line.baseline - line.bounds.ascent,
        width: 1.0,
        height: line.bounds.ascent + line.bounds.descent,
    }
}

fn select_run_for_offset(
    line: &ShapedLine,
    offset: usize,
    affinity: Affinity,
) -> Option<&ShapedRun> {
    let exact = match affinity {
        Affinity::Downstream => line.runs.iter().find(|run| run.byte_range.start == offset),
        Affinity::Upstream => line.runs.iter().find(|run| run.byte_range.end == offset),
    };
    exact
        .or_else(|| {
            line.runs.iter().find(|run| match affinity {
                Affinity::Downstream => {
                    run.byte_range.start <= offset && offset < run.byte_range.end
                }
                Affinity::Upstream => run.byte_range.start < offset && offset <= run.byte_range.end,
            })
        })
        .or_else(|| line.runs.first())
}

fn x_for_offset_in_run(line: &ShapedLine, run: &ShapedRun, offset: usize) -> f32 {
    for glyph in &run.glyphs {
        if glyph.text_range.start <= offset && offset <= glyph.text_range.end {
            let boundaries = grapheme_boundaries_in(&line.text, glyph.text_range.clone());
            let logical_index = boundaries
                .iter()
                .position(|boundary| *boundary == offset)
                .unwrap_or_else(|| {
                    boundaries
                        .iter()
                        .rposition(|boundary| *boundary < offset)
                        .unwrap_or(0)
                });
            let slots = boundaries.len().saturating_sub(1).max(1);
            let fraction = logical_index as f32 / slots as f32;
            return if run.is_rtl {
                glyph.left + glyph.x_advance * (1.0 - fraction)
            } else {
                glyph.left + glyph.x_advance * fraction
            };
        }
    }
    match (run.is_rtl, offset <= run.byte_range.start) {
        (false, true) | (true, false) => run.left,
        (false, false) | (true, true) => run.left + run.width,
    }
}

pub fn selection_rects(line: &ShapedLine, range: Range<usize>) -> Vec<TextRect> {
    let start = safe_grapheme_offset(&line.text, range.start.min(range.end));
    let end = safe_grapheme_offset(&line.text, range.start.max(range.end));
    if start == end {
        return Vec::new();
    }
    let y = line.baseline - line.bounds.ascent;
    let height = line.bounds.ascent + line.bounds.descent;
    line.runs
        .iter()
        .filter_map(|run| {
            let mut left = f32::INFINITY;
            let mut right = f32::NEG_INFINITY;
            for glyph in &run.glyphs {
                let selected_start = start.max(glyph.text_range.start);
                let selected_end = end.min(glyph.text_range.end);
                if selected_start >= selected_end {
                    continue;
                }
                let x1 = x_for_offset_in_run(line, run, selected_start);
                let x2 = x_for_offset_in_run(line, run, selected_end);
                left = left.min(x1.min(x2));
                right = right.max(x1.max(x2));
            }
            (left.is_finite() && right > left).then_some(TextRect {
                x: left,
                y,
                width: right - left,
                height,
            })
        })
        .collect()
}

fn grapheme_boundaries_in(text: &str, range: Range<usize>) -> Vec<usize> {
    let Some(slice) = text.get(range.clone()) else {
        return vec![range.start, range.end];
    };
    let mut boundaries = slice
        .grapheme_indices(true)
        .map(|(offset, _)| range.start + offset)
        .collect::<Vec<_>>();
    if boundaries.first().copied() != Some(range.start) {
        boundaries.insert(0, range.start);
    }
    if boundaries.last().copied() != Some(range.end) {
        boundaries.push(range.end);
    }
    boundaries
}

pub fn prev_grapheme(text: &str, offset: usize) -> usize {
    let offset = safe_grapheme_offset(text, offset);
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index < offset)
        .last()
        .unwrap_or(0)
}

pub fn next_grapheme(text: &str, offset: usize) -> usize {
    let offset = safe_grapheme_offset(text, offset);
    text.grapheme_indices(true)
        .map(|(index, grapheme)| index + grapheme.len())
        .find(|end| *end > offset)
        .unwrap_or(text.len())
}

pub fn prev_word_boundary(text: &str, offset: usize) -> usize {
    let offset = safe_grapheme_offset(text, offset);
    text.unicode_word_indices()
        .map(|(start, _)| start)
        .take_while(|start| *start < offset)
        .last()
        .unwrap_or(0)
}

pub fn next_word_boundary(text: &str, offset: usize) -> usize {
    let offset = safe_grapheme_offset(text, offset);
    text.unicode_word_indices()
        .map(|(start, word)| start + word.len())
        .find(|end| *end > offset)
        .unwrap_or(text.len())
}

pub fn word_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = safe_grapheme_offset(text, offset);
    if let Some((start, word)) = text.unicode_word_indices().find(|(start, word)| {
        *start <= offset && offset < start + word.len()
            || offset == text.len() && start + word.len() == offset
    }) {
        return start..start + word.len();
    }
    let end = next_grapheme(text, offset);
    let start = if end == offset {
        prev_grapheme(text, offset)
    } else {
        offset
    };
    start..end.max(start)
}

pub(crate) fn safe_grapheme_offset(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    if offset == text.len() {
        return text.len();
    }
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .take_while(|index| *index <= offset)
        .last()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        empty_label_params,
        measurement::TextMeasurementConfig,
        types::{FontStyle, FontWeight, TextSyntaxMode},
        TextEngine,
    };

    fn config<'a>(text: &'a str) -> TextMeasurementConfig<'a> {
        TextMeasurementConfig {
            text,
            font: "Lato",
            font_size: 18.0,
            font_weight: FontWeight::default(),
            font_style: FontStyle::default(),
            syntax_mode: TextSyntaxMode::Plain,
            params: empty_label_params(),
            number_locale: None,
            number_locale_specs: None,
            datetime_locale: None,
            datetime_timezone: None,
            datetime_locale_specs: None,
        }
    }

    #[test]
    fn mixed_script_runs_keep_source_ranges_and_bidi_direction() {
        let engine = TextEngine::with_default_config().unwrap();
        let text = "abc אבג xyz";
        let line = engine.shape_line(&config(text)).unwrap();
        assert!(line.runs.iter().any(|run| run.is_rtl), "{line:#?}");
        assert!(line.runs.iter().any(|run| !run.is_rtl), "{line:#?}");
        assert!(
            line.runs
                .windows(2)
                .all(|runs| runs[0].left <= runs[1].left),
            "{line:#?}"
        );
        for run in &line.runs {
            assert!(!run_text(&line, run).is_empty());
            for glyph in &run.glyphs {
                assert!(run.byte_range.start <= glyph.text_range.start);
                assert!(glyph.text_range.end <= run.byte_range.end);
                assert!(!&text[glyph.text_range.clone()].is_empty());
            }
        }
    }

    fn run_text<'a>(line: &'a ShapedLine, run: &ShapedRun) -> &'a str {
        &line.text[run.byte_range.clone()]
    }

    #[test]
    fn bidi_selection_produces_per_run_rectangles_and_affinity_positions() {
        let engine = TextEngine::with_default_config().unwrap();
        let text = "abc אבג xyz";
        let line = engine.shape_line(&config(text)).unwrap();
        let rects = selection_rects(&line, 0..text.len());
        assert!(rects.len() >= 2, "{line:#?}");

        let rtl = line.runs.iter().find(|run| run.is_rtl).unwrap();
        let boundary_run = line
            .runs
            .iter()
            .find(|run| run.byte_range.end == rtl.byte_range.start)
            .expect("mixed line should have an affinity boundary");
        assert!(!boundary_run.is_rtl);
        let downstream = cursor_rect_for_offset(&line, rtl.byte_range.start, Affinity::Downstream);
        let upstream = cursor_rect_for_offset(&line, rtl.byte_range.start, Affinity::Upstream);
        assert_ne!(downstream.x, upstream.x, "{line:#?}");
    }

    #[test]
    fn cluster_hit_testing_subdivides_ligatures_and_graphemes() {
        let engine = TextEngine::with_default_config().unwrap();
        let line = engine.shape_line(&config("office")).unwrap();
        let cluster = line
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter().map(move |glyph| (run, glyph)))
            .find(|(_, glyph)| glyph.text_range.end - glyph.text_range.start > 1)
            .expect("Lato should shape an office ligature cluster");
        let (run, glyph) = cluster;
        let quarter = byte_offset_for_x(&line, glyph.left + glyph.x_advance * 0.25).0;
        let three_quarters = byte_offset_for_x(&line, glyph.left + glyph.x_advance * 0.75).0;
        assert!(glyph.text_range.contains(&quarter) || quarter == glyph.text_range.end);
        assert!(
            glyph.text_range.contains(&three_quarters) || three_quarters == glyph.text_range.end
        );
        assert_ne!(quarter, three_quarters, "{run:#?} {glyph:#?}");

        let family = "👩‍👩‍👧‍👦x";
        assert_eq!(next_grapheme(family, 0), "👩‍👩‍👧‍👦".len());
        assert_eq!(prev_grapheme(family, family.len()), "👩‍👩‍👧‍👦".len());
        let family_line = engine.shape_line(&config(family)).unwrap();
        let family_cluster = family_line
            .runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .find(|glyph| glyph.text_range == (0.."👩‍👩‍👧‍👦".len()))
            .expect("ZWJ family should remain one source cluster");
        assert_eq!(
            byte_offset_for_x(
                &family_line,
                family_cluster.left + family_cluster.x_advance * 0.25,
            )
            .0,
            0
        );
        assert_eq!(
            byte_offset_for_x(
                &family_line,
                family_cluster.left + family_cluster.x_advance * 0.75,
            )
            .0,
            "👩‍👩‍👧‍👦".len()
        );
    }

    #[test]
    fn empty_line_uses_real_shaped_caret_metrics() {
        let engine = TextEngine::with_default_config().unwrap();
        let line = engine.shape_line(&config("")).unwrap();
        let caret = cursor_rect_for_offset(&line, 0, Affinity::Downstream);
        assert_eq!(line.bounds.width, 0.0);
        assert!(caret.height > 0.0);
        assert_eq!(caret.height, line.bounds.ascent + line.bounds.descent);
    }

    #[test]
    fn grapheme_and_word_boundaries_are_byte_based() {
        let text = "a e\u{301} world";
        let world = text.find("world").unwrap();
        assert_eq!(prev_word_boundary(text, text.len()), world);
        assert_eq!(next_word_boundary(text, world), text.len());
        assert_eq!(word_range_at(text, world + 2), world..text.len());
        assert_eq!(next_grapheme(text, 2), 2 + "e\u{301}".len());
    }
}
