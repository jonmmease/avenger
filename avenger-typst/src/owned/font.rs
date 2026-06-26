use crate::error::MathTypesetError;
use crate::fonts::{EmbeddedFontFace, ATKINSON_FACES};
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

    pub(crate) fn shaped_metrics(&self, text: &str, font_size: f32) -> OwnedShapedMetrics {
        let edge_metrics = self.default_text_edge_metrics(font_size);
        let Some(face) = rustybuzz::Face::from_slice(self.data, 0) else {
            let fallback = fallback_width(text, font_size);
            return OwnedShapedMetrics {
                width: fallback,
                ascent: edge_metrics.ascent,
                descent: edge_metrics.descent,
                height: edge_metrics.height,
            };
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        let glyphs = rustybuzz::shape(&face, &[], buffer);
        let scale = font_scale(&self.face, font_size);
        let mut advance = 0i32;

        for position in glyphs.glyph_positions() {
            advance += position.x_advance;
        }

        OwnedShapedMetrics {
            width: advance as f32 * scale,
            ascent: edge_metrics.ascent,
            descent: edge_metrics.descent,
            height: edge_metrics.height,
        }
    }
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

        assert!(face.shaped_metrics("Hello", 12.0).width > 0.0);
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
