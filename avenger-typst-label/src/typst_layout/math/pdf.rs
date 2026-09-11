fn pdf_text_from_simple_row(
    font: &MathFont,
    layout: &SimpleRowLayout,
    source: &str,
    fill: crate::typst_library::Color,
) -> Result<PdfArtifact, LabelError> {
    let face =
        ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
            start: 0,
            end: source.len(),
            message: "failed to parse Typst math font for PDF glyph output".to_string(),
        })?;
    let font_id = FontResourceId(0);
    let font_resources = vec![FontResource {
        id: font_id,
        family: font_name(&face, ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
            .or_else(|| font_name(&face, ttf_parser::name_id::FAMILY))
            .unwrap_or_else(|| "Unknown".to_string()),
        postscript_name: font_name(&face, ttf_parser::name_id::POST_SCRIPT_NAME),
        face_index: font.face_index,
        units_per_em: face.units_per_em() as f32,
        data: Arc::<[u8]>::from(font.data.clone()),
    }];

    let mut glyph_runs = Vec::new();
    for atom in &layout.atoms {
        let mut index = 0;
        while index < atom.glyphs.len() {
            let glyph = &atom.glyphs[index];
            if let Some(group) = glyph.pdf_run_group {
                let start = index;
                index += 1;
                while index < atom.glyphs.len()
                    && atom.glyphs[index].pdf_run_group == Some(group)
                    && atom.glyphs[index].font_size == glyph.font_size
                {
                    index += 1;
                }
                push_pdf_glyph_run(
                    &mut glyph_runs,
                    font_id,
                    glyph.font_size,
                    fill,
                    &atom.glyphs[start..index],
                );
            } else {
                push_pdf_glyph_run(
                    &mut glyph_runs,
                    font_id,
                    glyph.font_size,
                    fill,
                    std::slice::from_ref(glyph),
                );
                index += 1;
            }
        }
    }

    Ok(PdfArtifact {
        text_layer: PdfTextLayer {
            logical_width: layout.metrics.width,
            logical_height: layout.metrics.height,
            semantic_text: source.to_string(),
            glyph_runs,
        },
        font_resources,
    })
}

fn push_pdf_glyph_run(
    glyph_runs: &mut Vec<PdfGlyphRun>,
    font: FontResourceId,
    font_size: f32,
    fill: crate::typst_library::Color,
    glyphs: &[LaidOutGlyph],
) {
    let mut text = String::new();
    let mut glyph_text_ranges = Vec::with_capacity(glyphs.len());
    for glyph in glyphs {
        let start = text.len();
        text.push_str(&glyph.unicode);
        glyph_text_ranges.push(start..text.len());
    }

    glyph_runs.push(PdfGlyphRun {
        font,
        font_size,
        fill,
        stroke: None,
        text,
        glyphs: glyphs
            .iter()
            .zip(glyph_text_ranges)
            .map(|(glyph, text_range)| PdfGlyph {
                glyph_id: glyph.glyph_id.0,
                unicode: glyph.unicode.clone(),
                text_range,
                x: 0.0,
                y: 0.0,
                x_advance: glyph.x_advance,
                y_advance: 0.0,
                transform: Transform {
                    tx: glyph.x,
                    ty: glyph.y,
                    ..Transform::IDENTITY
                },
            })
            .collect(),
    });
}

fn font_name(face: &ttf_parser::Face<'_>, name_id: u16) -> Option<String> {
    face.names()
        .into_iter()
        .find(|name| name.name_id == name_id && name.is_unicode())
        .and_then(|name| name.to_string())
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
