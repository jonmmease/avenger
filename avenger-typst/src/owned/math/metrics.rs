use crate::api::TypstEngineConfig;
use crate::error::MathTypesetError;
use crate::style::MathFontSpec;
use crate::types::{MathFragmentOptions, MathRunArtifact, TypesetMetrics};

use super::ast::{OwnedMath, OwnedMathNode, OwnedMathTextKind};

pub(crate) fn try_typeset_number_fragment(
    math: &OwnedMath,
    options: &MathFragmentOptions,
    config: &TypstEngineConfig,
) -> Result<Option<MathRunArtifact>, MathTypesetError> {
    if options.outputs.paths || options.outputs.raster.is_some() || options.outputs.pdf_text_layer {
        return Ok(None);
    }
    if !matches!(options.style.font, MathFontSpec::NewComputerModernMath) {
        return Ok(None);
    }
    if !config.font_config.extra_font_families.is_empty() {
        return Ok(None);
    }

    let Some(number) = single_number_token(math) else {
        return Ok(None);
    };
    let Some(font) = load_default_math_font(config) else {
        return Ok(None);
    };
    let metrics = measure_number(&font, number, options.style.font_size.max(1.0))?;

    Ok(Some(MathRunArtifact {
        metrics,
        paths: None,
        raster: None,
        pdf_text: None,
        font_resources: Vec::new(),
        warnings: Vec::new(),
    }))
}

fn single_number_token(math: &OwnedMath) -> Option<&str> {
    let [OwnedMathNode::Text(text)] = &math.nodes[..] else {
        return None;
    };
    (text.kind == OwnedMathTextKind::Number).then_some(text.text.as_str())
}

struct OwnedMathFont {
    data: Vec<u8>,
    face_index: u32,
}

fn load_default_math_font(config: &TypstEngineConfig) -> Option<OwnedMathFont> {
    for path in crate::fonts::candidate_math_font_paths(config) {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let face_count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for face_index in 0..face_count {
            let Ok(face) = ttf_parser::Face::parse(&data, face_index) else {
                continue;
            };
            if face.tables().math.is_some() {
                return Some(OwnedMathFont { data, face_index });
            }
        }
    }
    None
}

fn measure_number(
    font: &OwnedMathFont,
    text: &str,
    font_size: f32,
) -> Result<TypesetMetrics, MathTypesetError> {
    let face = ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| {
        MathTypesetError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to parse owned math font".to_string(),
        }
    })?;
    let Some(rusty) = rustybuzz::Face::from_slice(&font.data, font.face_index) else {
        return Err(MathTypesetError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to shape owned math font".to_string(),
        });
    };

    let scale = font_size / face.units_per_em() as f32;
    let mut width = 0i32;
    let mut glyph_ascent = 0i16;
    let mut descent = 0i16;

    for ch in text.chars() {
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(ch.encode_utf8(&mut [0; 4]));
        if let Some(script) =
            rustybuzz::Script::from_iso15924_tag(ttf_parser::Tag::from_bytes(b"math"))
        {
            buffer.set_script(script);
        }
        buffer.set_direction(rustybuzz::Direction::LeftToRight);
        buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        let glyphs = rustybuzz::shape(&rusty, &[], buffer);
        let Some((info, position)) = glyphs
            .glyph_infos()
            .first()
            .zip(glyphs.glyph_positions().first())
        else {
            continue;
        };
        width += position.x_advance;
        if let Some(bounds) = face.glyph_bounding_box(ttf_parser::GlyphId(info.glyph_id as u16)) {
            glyph_ascent = glyph_ascent.max(bounds.y_max);
            descent = descent.max(-bounds.y_min);
        }
    }

    // Typst wraps math numbers as text-like fragments. The logical line box uses
    // the font cap-height rather than the tighter visible digit outline.
    let ascent = face.capital_height().unwrap_or(glyph_ascent);
    let ascent = ascent.max(0) as f32 * scale;
    let descent = descent.max(0) as f32 * scale;
    Ok(TypesetMetrics {
        width: width as f32 * scale,
        height: ascent + descent,
        baseline: ascent,
        ascent,
        descent,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned::math::syntax::parse_owned_math;
    use crate::types::MathOutputRequest;

    #[test]
    fn number_fragment_declines_heavy_outputs() {
        let math = parse_owned_math("1", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        };

        assert!(
            try_typeset_number_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn number_fragment_declines_non_numbers() {
        let math = parse_owned_math("x", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs.paths = false;

        assert!(
            try_typeset_number_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_none()
        );
    }
}
