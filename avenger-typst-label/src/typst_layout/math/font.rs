// Retained math font, glyph, variant, and assembly helpers, mapped in
// UPSTREAM.md to upstream `math/text.rs`, `math/shaping.rs`, and
// `math/fragment/glyph.rs`.
struct MathFont {
    data: Vec<u8>,
    face_index: u32,
}

#[cfg(test)]
fn load_default_math_font(
    config: &EngineOptions,
    spec: &MathFontSpec,
    weight: &FontWeight,
) -> Option<MathFont> {
    let fontdb = crate::typst_layout::inline::font::build_text_fontdb(config);
    load_default_math_font_with_fontdb(config, &fontdb, spec, weight)
}

fn load_default_math_font_with_fontdb(
    config: &EngineOptions,
    fontdb: &fontdb::Database,
    spec: &MathFontSpec,
    weight: &FontWeight,
) -> Option<MathFont> {
    if let MathFontSpec::FontBytes(id) = spec
        && let Some((data, face_index)) = crate::label::fonts::registered_font_data(config, *id) {
            return math_font_from_face_data(data.to_vec(), face_index);
        }

    for family in math_font_family_candidates(config, spec) {
        if let Some(font) = math_font_from_fontdb(fontdb, &family, weight) {
            return Some(font);
        }
    }

    for path in crate::label::fonts::candidate_math_font_paths(config) {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        if let Some(font) = math_font_from_data(data) {
            return Some(font);
        }
    }
    None
}

fn math_font_family_candidates(config: &EngineOptions, spec: &MathFontSpec) -> Vec<String> {
    let mut families = Vec::new();
    let mut push = |family: &str| {
        if !families
            .iter()
            .any(|existing: &String| existing.eq_ignore_ascii_case(family))
        {
            families.push(family.to_string());
        }
    };

    match spec {
        MathFontSpec::LeteSansMath => {
            if let Some(family) = &config.fonts.default_math_family {
                push(family);
            }
            push("Lete Sans Math");
            push("LeteSansMath");
        }
        MathFontSpec::NewComputerModernMath => {
            push("New Computer Modern Math");
            push("NewCMMath");
        }
        MathFontSpec::Family(family) => push(family),
        MathFontSpec::FontBytes(_) => {}
    }

    families
}

fn math_font_from_fontdb(
    fontdb: &fontdb::Database,
    family: &str,
    weight: &FontWeight,
) -> Option<MathFont> {
    let families = [fontdb::Family::Name(family)];
    let query = fontdb::Query {
        families: &families,
        weight: fontdb::Weight(font_weight_number(weight)),
        stretch: fontdb::Stretch::Normal,
        style: fontdb::Style::Normal,
    };
    if let Some(font) = fontdb
        .query(&query)
        .and_then(|id| math_font_from_fontdb_id(fontdb, id))
    {
        return Some(font);
    }

    fontdb
        .faces()
        .filter(|face| {
            face.families
                .iter()
                .any(|(candidate, _)| candidate.eq_ignore_ascii_case(family))
        })
        .filter_map(|face| math_font_from_fontdb_id(fontdb, face.id))
        .find(|font| {
            ttf_parser::Face::parse(&font.data, font.face_index)
                .ok()
                .and_then(|face| face.tables().math)
                .is_some()
        })
}

fn math_font_from_fontdb_id(fontdb: &fontdb::Database, id: fontdb::ID) -> Option<MathFont> {
    fontdb.with_face_data(id, |data, face_index| {
        math_font_from_face_data(data.to_vec(), face_index)
    })?
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn math_font_from_data(data: Vec<u8>) -> Option<MathFont> {
    let face_count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    for face_index in 0..face_count {
        if let Some(font) = math_font_from_face_data(data.clone(), face_index) {
            return Some(font);
        }
    }
    None
}

fn math_font_from_face_data(data: Vec<u8>, face_index: u32) -> Option<MathFont> {
    let face = ttf_parser::Face::parse(&data, face_index).ok()?;
    face.tables().math?;
    Some(MathFont { data, face_index })
}

fn parse_math_face<'a>(
    font: &'a MathFont,
    context: &str,
) -> Result<ttf_parser::Face<'a>, LabelError> {
    ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
        start: 0,
        end: 0,
        message: format!("failed to parse Typst math font for {context}"),
    })
}

fn layout_styled_atom_with_class(
    font: &MathFont,
    text: &str,
    font_size: f32,
    script_style: Option<u32>,
    class: SimpleMathClass,
) -> Result<LaidOutMathAtom, LabelError> {
    let face =
        ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to parse Typst math font".to_string(),
        })?;
    let Some(rusty) = rustybuzz::Face::from_slice(&font.data, font.face_index) else {
        return Err(LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to shape Typst math font".to_string(),
        });
    };

    let scale = font_size / face.units_per_em() as f32;
    let mut width = 0i32;
    let mut glyph_ascent = 0i16;
    let mut glyph_descent = 0i16;
    let mut atom_italic_correction = 0.0;
    let mut glyphs = Vec::new();
    let features = if let Some(script_style) = script_style {
        vec![rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"ssty"),
            script_style,
            ..,
        )]
    } else {
        Vec::new()
    };

    for ch in text.chars() {
        let glyph_x = width as f32 * scale;
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(ch.encode_utf8(&mut [0; 4]));
        if let Some(script) =
            rustybuzz::Script::from_iso15924_tag(ttf_parser::Tag::from_bytes(b"math"))
        {
            buffer.set_script(script);
        }
        buffer.set_direction(rustybuzz::Direction::LeftToRight);
        buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);
        let shaped = rustybuzz::shape(&rusty, &features, buffer);
        let Some((info, position)) = shaped
            .glyph_infos()
            .first()
            .zip(shaped.glyph_positions().first())
        else {
            continue;
        };
        let glyph_id = ttf_parser::GlyphId(info.glyph_id as u16);
        let mut advance = position.x_advance;
        let mut glyph_italic_correction = 0;
        if !is_extended_shape(&face, glyph_id) {
            glyph_italic_correction = italic_correction(&face, glyph_id).unwrap_or_default();
            advance += glyph_italic_correction as i32;
        }
        width += advance;
        glyphs.push(LaidOutGlyph {
            glyph_id,
            unicode: ch.to_string(),
            x: glyph_x,
            y: 0.0,
            x_advance: advance as f32 * scale,
            font_size,
            pdf_run_group: None,
        });
        if let Some(bounds) = face.glyph_bounding_box(glyph_id) {
            glyph_ascent = glyph_ascent.max(bounds.y_max);
            glyph_descent = glyph_descent.max(-bounds.y_min);
        }
        atom_italic_correction = glyph_italic_correction as f32 * scale;
    }

    // Typst wraps standalone math atoms as text-like fragments. The logical
    // line box uses cap-height and ignores descenders from the tighter outline.
    let ascent = face.capital_height().unwrap_or(glyph_ascent);
    let ascent = ascent.max(0) as f32 * scale;
    let descent = 0.0;
    let mut atom = LaidOutMathAtom {
        metrics: TypesetMetrics {
            width: width as f32 * scale,
            height: ascent + descent,
            baseline: ascent,
            ascent,
            descent,
        },
        ink_ascent: glyph_ascent.max(0) as f32 * scale,
        ink_descent: glyph_descent.max(0) as f32 * scale,
        left_class: class,
        right_class: class,
        italic_correction: atom_italic_correction,
        script_kernable: true,
        glyphs,
        shapes: Vec::new(),
        draw_order: Vec::new(),
    };
    for glyph in &mut atom.glyphs {
        glyph.y = atom.metrics.baseline;
    }
    atom.draw_order
        .extend((0..atom.glyphs.len()).map(LaidOutDrawItem::Glyph));
    Ok(atom)
}

fn layout_accent_atom(
    font: &MathFont,
    accent: char,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutMathAtom, LabelError> {
    layout_styled_atom_with_class(
        font,
        &accent.to_string(),
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Normal,
    )
}

fn atom_top_accent_attachment(font: &MathFont, atom: &LaidOutMathAtom) -> Result<f32, LabelError> {
    if atom.glyphs.len() == 1 && atom.shapes.is_empty() {
        let glyph = &atom.glyphs[0];
        if let Some(attachment) = top_accent_attachment(font, glyph)? {
            return Ok(attachment);
        }
    }
    Ok((atom.metrics.width + atom.italic_correction) / 2.0)
}

fn top_accent_attachment(font: &MathFont, glyph: &LaidOutGlyph) -> Result<Option<f32>, LabelError> {
    let face = parse_math_face(font, "top accent attachment")?;
    let Some(value) = face
        .tables()
        .math
        .and_then(|math| math.glyph_info)
        .and_then(|glyph_info| glyph_info.top_accent_attachments)
        .and_then(|attachments| attachments.get(glyph.glyph_id))
    else {
        return Ok(None);
    };

    Ok(Some(
        value.value as f32 * glyph.font_size / face.units_per_em() as f32,
    ))
}

fn script_style_feature(script_level: u8) -> Option<u32> {
    (script_level > 0).then_some(u32::from(script_level.min(2)))
}
