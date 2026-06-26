use crate::api::TypstEngineConfig;
use crate::error::MathTypesetError;
use crate::style::MathFontSpec;
use crate::types::{MathFragmentOptions, MathRunArtifact, TypesetMetrics};

use super::ast::{
    OwnedMath, OwnedMathNode, OwnedMathOperator, OwnedMathShorthand, OwnedMathText,
    OwnedMathTextKind,
};

pub(crate) fn try_typeset_single_atom_fragment(
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

    let Some(styled_text) = single_atom_text(math) else {
        return Ok(None);
    };
    let Some(font) = load_default_math_font(config) else {
        return Ok(None);
    };
    let metrics = measure_styled_atom(&font, &styled_text, options.style.font_size.max(1.0))?;

    Ok(Some(MathRunArtifact {
        metrics,
        paths: None,
        raster: None,
        pdf_text: None,
        font_resources: Vec::new(),
        warnings: Vec::new(),
    }))
}

fn single_atom_text(math: &OwnedMath) -> Option<String> {
    let [node] = &math.nodes[..] else {
        return None;
    };
    match node {
        OwnedMathNode::Text(text) => Some(style_text_atom(text)),
        OwnedMathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            if identifier.symbol.is_some() || text.chars().count() == 1 {
                Some(style_default_math_text(text))
            } else {
                None
            }
        }
        OwnedMathNode::Operator(operator) => Some(operator_text(operator)),
        OwnedMathNode::Shorthand(shorthand) => Some(shorthand_text(shorthand)),
        _ => None,
    }
}

fn style_text_atom(text: &OwnedMathText) -> String {
    match text.kind {
        OwnedMathTextKind::Grapheme => style_default_math_text(&text.text),
        OwnedMathTextKind::Number => text.text.clone(),
    }
}

fn style_default_math_text(text: &str) -> String {
    text.chars().map(style_default_math_char).collect()
}

fn operator_text(operator: &OwnedMathOperator) -> String {
    operator.operator.clone()
}

fn shorthand_text(shorthand: &OwnedMathShorthand) -> String {
    shorthand.replacement.to_string()
}

fn style_default_math_char(ch: char) -> char {
    if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) || matches!(ch, 'ı' | 'ȷ' | 'ħ')
    {
        to_math_italic(ch)
    } else {
        ch
    }
}

fn is_lower_greek_math_char(ch: char) -> bool {
    matches!(ch, 'α'..='ω' | '∂' | 'ϵ' | 'ϑ' | 'ϰ' | 'ϕ' | 'ϱ' | 'ϖ')
}

fn to_math_italic(ch: char) -> char {
    let delta = match ch {
        'h' => 0x20A6,
        'ħ' => 0x1FE8,
        'A'..='Z' => 0x1D3F3,
        'a'..='z' => 0x1D3ED,
        'ı' => 0x1D573,
        'ȷ' => 0x1D46E,
        'Α'..='Ρ' => 0x1D351,
        'ϴ' => 0x1D2FF,
        'Σ'..='Ω' => 0x1D351,
        '∇' => 0x1B4F4,
        'α'..='ω' => 0x1D34B,
        '∂' => 0x1B513,
        'ϵ' => 0x1D321,
        'ϑ' => 0x1D346,
        'ϰ' => 0x1D328,
        'ϕ' => 0x1D344,
        'ϱ' => 0x1D329,
        'ϖ' => 0x1D345,
        _ => return ch,
    };
    std::char::from_u32((ch as u32) + delta).unwrap_or(ch)
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

fn measure_styled_atom(
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
        let glyph_id = ttf_parser::GlyphId(info.glyph_id as u16);
        width += position.x_advance;
        if !is_extended_shape(&face, glyph_id) {
            width += italic_correction(&face, glyph_id).unwrap_or_default() as i32;
        }
        if let Some(bounds) = face.glyph_bounding_box(glyph_id) {
            glyph_ascent = glyph_ascent.max(bounds.y_max);
        }
    }

    // Typst wraps standalone math atoms as text-like fragments. The logical
    // line box uses cap-height and ignores descenders from the tighter outline.
    let ascent = face.capital_height().unwrap_or(glyph_ascent);
    let ascent = ascent.max(0) as f32 * scale;
    let descent = 0.0;
    Ok(TypesetMetrics {
        width: width as f32 * scale,
        height: ascent + descent,
        baseline: ascent,
        ascent,
        descent,
    })
}

fn italic_correction(face: &ttf_parser::Face<'_>, glyph_id: ttf_parser::GlyphId) -> Option<i16> {
    face.tables()
        .math?
        .glyph_info?
        .italic_corrections?
        .get(glyph_id)
        .map(|value| value.value)
}

fn is_extended_shape(face: &ttf_parser::Face<'_>, glyph_id: ttf_parser::GlyphId) -> bool {
    face.tables()
        .math
        .and_then(|math| math.glyph_info)
        .and_then(|glyph_info| glyph_info.extended_shapes)
        .and_then(|coverage| coverage.get(glyph_id))
        .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned::math::syntax::parse_owned_math;
    use crate::types::MathOutputRequest;

    #[test]
    fn atom_fragment_declines_heavy_outputs() {
        let math = parse_owned_math("1", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        };

        assert!(
            try_typeset_single_atom_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn atom_fragment_declines_multi_atom_rows() {
        let math = parse_owned_math("x + y", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs.paths = false;

        assert!(
            try_typeset_single_atom_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn atom_fragment_styles_latin_and_greek_as_math_italic() {
        let x = parse_owned_math("x", 0).unwrap();
        let alpha = parse_owned_math("alpha", 0).unwrap();

        assert_eq!(single_atom_text(&x).as_deref(), Some("𝑥"));
        assert_eq!(single_atom_text(&alpha).as_deref(), Some("𝛼"));
    }

    #[test]
    fn atom_fragment_keeps_numbers_and_operators_plain() {
        let number = parse_owned_math("0.94", 0).unwrap();
        let plus = parse_owned_math("+", 0).unwrap();
        let arrow = parse_owned_math("->", 0).unwrap();

        assert_eq!(single_atom_text(&number).as_deref(), Some("0.94"));
        assert_eq!(single_atom_text(&plus).as_deref(), Some("+"));
        assert_eq!(single_atom_text(&arrow).as_deref(), Some("→"));
    }
}
