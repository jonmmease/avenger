use std::sync::Arc;

use crate::error::MathTypesetError;
use crate::fonts::{EmbeddedFontFace, ATKINSON_FACES};
use crate::pdf::{MathFontResource, MathFontResourceId};
use crate::style::{FontStyle, FontWeight, PlainTextStyle};

pub(crate) struct OwnedTextFace<'a> {
    pub(crate) face: ttf_parser::Face<'a>,
    data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OwnedShapedMetrics {
    pub(crate) width: f32,
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedShapedGlyph {
    pub(crate) glyph_id: ttf_parser::GlyphId,
    pub(crate) unicode: String,
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) x_advance: f32,
    pub(crate) y_advance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OwnedShapedText {
    pub(crate) metrics: OwnedShapedMetrics,
    pub(crate) glyphs: Vec<OwnedShapedGlyph>,
    pub(crate) has_missing_glyph: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct OwnedFontMetrics {
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) height: f32,
}

impl<'a> OwnedTextFace<'a> {
    pub(crate) fn for_plain_style(
        style: &PlainTextStyle,
    ) -> Result<Option<Self>, MathTypesetError> {
        if !resolves_to_embedded_atkinson(&style.font_family) {
            return Ok(None);
        }

        let face = select_atkinson_face(&style.font_weight, style.font_style).ok_or(
            MathTypesetError::UnsupportedOutput(
                "owned plain text requires an embedded Atkinson face",
            ),
        )?;
        let parsed =
            ttf_parser::Face::parse(face.data, 0).map_err(|_| MathTypesetError::Engine {
                start: 0,
                end: 0,
                message: format!("failed to parse embedded font {}", face.name),
            })?;

        Ok(Some(Self {
            face: parsed,
            data: face.data,
        }))
    }

    pub(crate) fn default_text_edge_metrics(&self, font_size: f32) -> OwnedFontMetrics {
        let scale = font_scale(&self.face, font_size);
        let cap_height = self
            .face
            .capital_height()
            .filter(|height| *height > 0)
            .unwrap_or_else(|| {
                self.face
                    .typographic_ascender()
                    .unwrap_or_else(|| self.face.ascender())
            })
            .max(0) as f32
            * scale;

        OwnedFontMetrics {
            ascent: cap_height,
            descent: 0.0,
            height: cap_height,
        }
    }

    pub(crate) fn shaped_text(&self, text: &str, font_size: f32) -> OwnedShapedText {
        self.shaped_text_with_features(text, font_size, &[])
    }

    pub(crate) fn shaped_text_with_features(
        &self,
        text: &str,
        font_size: f32,
        features: &[rustybuzz::Feature],
    ) -> OwnedShapedText {
        let edge_metrics = self.default_text_edge_metrics(font_size);
        let Some(face) = rustybuzz::Face::from_slice(self.data, 0) else {
            let fallback = fallback_width(text, font_size);
            return OwnedShapedText {
                metrics: OwnedShapedMetrics {
                    width: fallback,
                    ascent: edge_metrics.ascent,
                    descent: edge_metrics.descent,
                    height: edge_metrics.height,
                },
                glyphs: Vec::new(),
                has_missing_glyph: true,
            };
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let glyphs = rustybuzz::shape(&face, features, buffer);
        let scale = font_scale(&self.face, font_size);
        let mut cursor_x = 0i32;
        let mut cursor_y = 0i32;
        let mut advance_width = 0i32;
        let mut shaped_glyphs = Vec::new();
        let mut has_missing_glyph = false;

        for (info, position) in glyphs.glyph_infos().iter().zip(glyphs.glyph_positions()) {
            has_missing_glyph |= info.glyph_id == 0;
            let x = cursor_x + position.x_offset;
            let y = cursor_y + position.y_offset;
            cursor_x += position.x_advance;
            cursor_y += position.y_advance;
            advance_width += position.x_advance;
            shaped_glyphs.push(OwnedShapedGlyph {
                glyph_id: ttf_parser::GlyphId(info.glyph_id as u16),
                unicode: glyph_unicode_for_cluster(text, info.cluster),
                x: x as f32 * scale,
                y: -(y as f32) * scale,
                x_advance: position.x_advance as f32 * scale,
                y_advance: -(position.y_advance as f32) * scale,
            });
        }

        OwnedShapedText {
            metrics: OwnedShapedMetrics {
                width: advance_width as f32 * scale,
                ascent: edge_metrics.ascent,
                descent: edge_metrics.descent,
                height: edge_metrics.height,
            },
            glyphs: shaped_glyphs,
            has_missing_glyph,
        }
    }

    pub(crate) fn script_style(
        &self,
        parent_style: &PlainTextStyle,
        script: OwnedTextScript,
    ) -> PlainTextStyle {
        let scale = font_scale(&self.face, parent_style.font_size.max(1.0));
        let metrics = match script {
            OwnedTextScript::Subscript => self.face.subscript_metrics(),
            OwnedTextScript::Superscript => self.face.superscript_metrics(),
        };
        let font_size = metrics
            .and_then(|metrics| (metrics.y_size > 0).then_some(metrics.y_size as f32 * scale))
            .unwrap_or_else(|| parent_style.font_size.max(1.0) * 0.7)
            .max(1.0);

        PlainTextStyle {
            font_size,
            ..parent_style.clone()
        }
    }

    pub(crate) fn script_baseline_shift(
        &self,
        parent_font_size: f32,
        script: OwnedTextScript,
    ) -> f32 {
        let scale = font_scale(&self.face, parent_font_size.max(1.0));
        let metrics = match script {
            OwnedTextScript::Subscript => self.face.subscript_metrics(),
            OwnedTextScript::Superscript => self.face.superscript_metrics(),
        };
        metrics
            .map(|metrics| {
                let offset = metrics.y_offset as f32 * scale;
                match script {
                    OwnedTextScript::Subscript => offset.abs(),
                    OwnedTextScript::Superscript => -offset.abs(),
                }
            })
            .unwrap_or_else(|| match script {
                OwnedTextScript::Subscript => parent_font_size.max(1.0) * 0.2,
                OwnedTextScript::Superscript => -parent_font_size.max(1.0) * 0.35,
            })
    }

    pub(crate) fn font_resource(&self, id: MathFontResourceId) -> MathFontResource {
        MathFontResource {
            id,
            family: font_name(&self.face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
                .or_else(|| font_name(&self.face, ttf_parser::name_id::FAMILY))
                .unwrap_or_else(|| "Unknown".to_string()),
            postscript_name: font_name(&self.face, ttf_parser::name_id::POST_SCRIPT_NAME),
            face_index: 0,
            units_per_em: self.face.units_per_em() as f32,
            data: Arc::<[u8]>::from(self.data),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OwnedTextScript {
    Subscript,
    Superscript,
}

fn glyph_unicode_for_cluster(text: &str, cluster: u32) -> String {
    let cluster = cluster as usize;
    let Some((start, _)) = text.char_indices().find(|(start, _)| *start == cluster) else {
        return String::new();
    };
    let end = text[start..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(next, _)| start + next);
    text[start..end].to_string()
}

fn font_name(face: &ttf_parser::Face<'_>, name_id: u16) -> Option<String> {
    face.names().into_iter().find_map(|name| {
        (name.name_id == name_id)
            .then(|| name.to_string())
            .flatten()
    })
}

fn select_atkinson_face(
    weight: &FontWeight,
    style: FontStyle,
) -> Option<&'static EmbeddedFontFace> {
    let target_weight = font_weight_number(weight);
    ATKINSON_FACES
        .iter()
        .filter(|face| face.style == style)
        .min_by_key(|face| face.weight.abs_diff(target_weight))
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn resolves_to_embedded_atkinson(font_family: &str) -> bool {
    font_family.split(',').any(|family| {
        let family = family.trim().trim_matches('"').trim_matches('\'');
        family.eq_ignore_ascii_case("sans-serif")
            || family.eq_ignore_ascii_case("Atkinson Hyperlegible Next")
    })
}

fn font_scale(face: &ttf_parser::Face<'_>, font_size: f32) -> f32 {
    font_size.max(1.0) / face.units_per_em() as f32
}

fn fallback_width(text: &str, font_size: f32) -> f32 {
    text.chars().count() as f32 * font_size.max(1.0) * 0.6
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_nearest_embedded_weight() {
        let style = PlainTextStyle {
            font_weight: FontWeight::Number(575),
            ..PlainTextStyle::default()
        };

        let face = OwnedTextFace::for_plain_style(&style)
            .unwrap()
            .expect("default sans-serif should resolve");

        assert!(face.shaped_text("Hello", 12.0).metrics.width > 0.0);
        assert!(face.default_text_edge_metrics(12.0).height > 0.0);
    }

    #[test]
    fn declines_non_atkinson_fonts_for_fast_path() {
        let style = PlainTextStyle {
            font_family: "serif".to_string(),
            ..PlainTextStyle::default()
        };

        assert!(OwnedTextFace::for_plain_style(&style).unwrap().is_none());
    }
}
