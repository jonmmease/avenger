// Retained fenced/delimiter layout, mapped in UPSTREAM.md to upstream
// `typst-layout/src/math/fenced.rs` and glyph stretching helpers.
fn layout_simple_group(
    font: &MathFont,
    group: &ast::MathGroup,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_delimited_nodes(
        font,
        group.left,
        &group.body,
        group.right,
        font_size,
        script_level,
        None,
        math_size,
    )
}

fn layout_simple_delimited_call(
    font: &MathFont,
    call: &ast::MathCall,
    left: char,
    right: char,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_delimited_nodes(
        font,
        left,
        &arg.nodes,
        right,
        font_size,
        script_level,
        call.options.delimiter_size,
        math_size,
    )
}

fn layout_simple_lr_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some((left, body_nodes, right)) = lr_call_delimited_body(&arg.nodes) else {
        return Ok(None);
    };
    layout_simple_delimited_nodes(
        font,
        left,
        body_nodes,
        right,
        font_size,
        script_level,
        call.options.delimiter_size,
        math_size,
    )
}

fn lr_call_delimited_body(nodes: &[MathNode]) -> Option<(char, &[MathNode], char)> {
    let start = nodes
        .iter()
        .position(|node| !matches!(node, MathNode::Space(_)))?;
    let end = nodes
        .iter()
        .rposition(|node| !matches!(node, MathNode::Space(_)))?
        + 1;
    if end <= start + 2 {
        return None;
    }

    let left = delimiter_char_from_node(&nodes[start])?;
    let right = delimiter_char_from_node(&nodes[end - 1])?;
    is_lr_delimiter_pair(left, right).then_some((left, &nodes[start + 1..end - 1], right))
}

fn delimiter_char_from_node(node: &MathNode) -> Option<char> {
    match node {
        MathNode::Operator(operator) => single_char(&operator.operator),
        MathNode::Text(text) => single_char(&text.text),
        MathNode::Identifier(identifier) => identifier.symbol.and_then(single_char),
        MathNode::Shorthand(shorthand) => single_char(shorthand.replacement),
        _ => None,
    }
    .filter(|delimiter| is_lr_delimiter(*delimiter))
}

fn is_lr_delimiter(delimiter: char) -> bool {
    matches!(
        delimiter,
        '|' | '‖' | '(' | ')' | '[' | ']' | '{' | '}' | '⌊' | '⌋' | '⌈' | '⌉' | '⟨' | '⟩'
    )
}

fn is_lr_delimiter_pair(left: char, right: char) -> bool {
    matches!(
        (left, right),
        ('|', '|')
            | ('‖', '‖')
            | ('(', ')')
            | ('[', ']')
            | ('{', '}')
            | ('⌊', '⌋')
            | ('⌈', '⌉')
            | ('⟨', '⟩')
    )
}

fn contains_mid_call(nodes: &[MathNode]) -> bool {
    nodes.iter().any(|node| match node {
        MathNode::Call(call) => {
            call.name == "mid" || call.args.iter().any(|arg| contains_mid_call(&arg.nodes))
        }
        MathNode::Group(group) => contains_mid_call(&group.body),
        MathNode::Attach(attach) => {
            contains_mid_call(std::slice::from_ref(attach.base.as_ref()))
                || attach
                    .top
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
                || attach
                    .bottom
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
                || attach
                    .top_left
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
                || attach
                    .top_right
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
                || attach
                    .bottom_left
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
                || attach
                    .bottom_right
                    .as_ref()
                    .is_some_and(|nodes| contains_mid_call(nodes))
        }
        MathNode::Fraction(fraction) => {
            contains_mid_call(&fraction.numerator) || contains_mid_call(&fraction.denominator)
        }
        MathNode::Cancel(cancel) => contains_mid_call(&cancel.body),
        MathNode::Accent(accent) => contains_mid_call(&accent.base),
        _ => false,
    })
}

#[allow(clippy::too_many_arguments, reason = "Keep the explicit inputs of the existing layout and rendering pipeline.")]
fn layout_simple_delimited_nodes(
    font: &MathFont,
    left: char,
    body_nodes: &[MathNode],
    right: char,
    font_size: f32,
    script_level: u8,
    explicit_size: Option<ast::MathDelimitedSize>,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(body) = layout_simple_nodes_as_atom_with_context(
        font,
        body_nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let delimiter_target_height = delimiter_target_height_for_body(
        font,
        &body,
        DelimiterTarget::Balanced,
        explicit_size,
        font_size,
    )?;
    let body = if contains_mid_call(body_nodes) {
        layout_simple_nodes_as_atom_with_context(
            font,
            body_nodes,
            font_size,
            script_level,
            Some(delimiter_target_height),
            math_size,
        )?
        .unwrap_or(body)
    } else {
        body
    };
    layout_simple_delimited_atom_with_target_height(
        font,
        left,
        body,
        right,
        font_size,
        script_level,
        DelimiterTarget::Balanced,
        delimiter_target_height,
    )
    .map(Some)
}

fn delimiter_target_height_for_body(
    font: &MathFont,
    body: &LaidOutMathAtom,
    target: DelimiterTarget,
    explicit_size: Option<ast::MathDelimitedSize>,
    font_size: f32,
) -> Result<f32, LabelError> {
    let natural_target_height = match target {
        DelimiterTarget::Balanced => {
            let axis = math_constant(font, font_size, |constants| constants.axis_height().value)?;
            2.0 * (body.metrics.ascent - axis).max(body.metrics.descent + axis)
        }
        DelimiterTarget::Frame => body.metrics.height,
    };
    Ok(explicit_size
        .map(|size| resolve_relative_math_size(size, natural_target_height, font_size))
        .unwrap_or(natural_target_height)
        .max(0.0))
}

#[allow(clippy::too_many_arguments, reason = "Keep the explicit inputs of the existing layout and rendering pipeline.")]
fn layout_simple_delimited_atom(
    font: &MathFont,
    left: char,
    body: LaidOutMathAtom,
    right: char,
    font_size: f32,
    script_level: u8,
    target: DelimiterTarget,
    explicit_size: Option<ast::MathDelimitedSize>,
) -> Result<LaidOutMathAtom, LabelError> {
    let delimiter_target_height =
        delimiter_target_height_for_body(font, &body, target, explicit_size, font_size)?;
    layout_simple_delimited_atom_with_target_height(
        font,
        left,
        body,
        right,
        font_size,
        script_level,
        target,
        delimiter_target_height,
    )
}

#[allow(clippy::too_many_arguments, reason = "Keep the explicit inputs of the existing layout and rendering pipeline.")]
fn layout_simple_delimited_atom_with_target_height(
    font: &MathFont,
    left: char,
    mut body: LaidOutMathAtom,
    right: char,
    font_size: f32,
    script_level: u8,
    target: DelimiterTarget,
    delimiter_target_height: f32,
) -> Result<LaidOutMathAtom, LabelError> {
    let mut left = layout_delimiter_atom_with_target(
        font,
        left,
        font_size,
        script_level,
        delimiter_target_height,
        SimpleMathClass::Opening,
        target == DelimiterTarget::Frame,
    )?;
    let mut right = layout_delimiter_atom_with_target(
        font,
        right,
        font_size,
        script_level,
        delimiter_target_height,
        SimpleMathClass::Closing,
        target == DelimiterTarget::Frame,
    )?;

    let width = left.metrics.width + body.metrics.width + right.metrics.width;
    let ascent = left
        .metrics
        .ascent
        .max(body.metrics.ascent)
        .max(right.metrics.ascent);
    let descent = left
        .metrics
        .descent
        .max(body.metrics.descent)
        .max(right.metrics.descent);
    let baseline = ascent;

    let left_dy = baseline - left.metrics.baseline;
    let body_dx = left.metrics.width;
    let body_dy = baseline - body.metrics.baseline;
    let right_dx = left.metrics.width + body.metrics.width;
    let right_dy = baseline - right.metrics.baseline;

    offset_atom(&mut left, 0.0, left_dy);
    offset_atom(&mut body, body_dx, body_dy);
    offset_atom(&mut right, right_dx, right_dy);

    let ink_ascent = (left.ink_ascent + left_dy)
        .max(body.ink_ascent + body_dy)
        .max(right.ink_ascent + right_dy);
    let ink_descent = (left.ink_descent - left_dy)
        .max(body.ink_descent - body_dy)
        .max(right.ink_descent - right_dy);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, left);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, body);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, right);

    Ok(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height: ascent + descent,
            baseline,
            ascent,
            descent,
        },
        ink_ascent,
        ink_descent,
        left_class: SimpleMathClass::Opening,
        right_class: SimpleMathClass::Closing,
        italic_correction: 0.0,
        script_kernable: true,
        glyphs,
        shapes,
        draw_order,
    })
}

fn resolve_relative_math_size(
    size: ast::MathRelativeSize,
    natural_target_height: f32,
    font_size: f32,
) -> f32 {
    size.relative * natural_target_height + size.absolute_em * font_size + size.absolute_pt
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterTarget {
    Balanced,
    Frame,
}

const DELIMITER_SHORT_FALL_EM: f32 = 0.1;
const ACCENT_SHORT_FALL_EM: f32 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStretchAxis {
    Horizontal,
    Vertical,
}

const MATH_GLYPH_ASSEMBLY_MAX_REPEATS: usize = 1024;

fn layout_delimiter_atom_with_target(
    font: &MathFont,
    delimiter: char,
    font_size: f32,
    script_level: u8,
    target_height: f32,
    class: SimpleMathClass,
    force_variant: bool,
) -> Result<LaidOutMathAtom, LabelError> {
    let mut atom = layout_styled_atom_with_class(
        font,
        &delimiter.to_string(),
        font_size,
        script_style_feature(script_level),
        class,
    )?;

    let _ = stretch_single_glyph_variant(
        font,
        &mut atom,
        MathStretchAxis::Vertical,
        target_height,
        DELIMITER_SHORT_FALL_EM * font_size,
        force_variant,
        "delimiter variants",
    )?;
    Ok(atom)
}

fn stretch_single_glyph_variant(
    font: &MathFont,
    atom: &mut LaidOutMathAtom,
    axis: MathStretchAxis,
    target_size: f32,
    short_fall: f32,
    force_variant: bool,
    context: &'static str,
) -> Result<Option<()>, LabelError> {
    let face = parse_math_face(font, context)?;
    let Some(base_glyph) = atom.glyphs.first().cloned() else {
        return Ok(None);
    };
    let Some(construction) =
        face.tables()
            .math
            .and_then(|math| math.variants)
            .and_then(|variants| match axis {
                MathStretchAxis::Horizontal => {
                    variants.horizontal_constructions.get(base_glyph.glyph_id)
                }
                MathStretchAxis::Vertical => {
                    variants.vertical_constructions.get(base_glyph.glyph_id)
                }
            })
    else {
        return Ok(None);
    };

    let short_target_size = (target_size - short_fall).max(0.0);
    let stretch_advance = match axis {
        MathStretchAxis::Horizontal => base_glyph.x_advance,
        MathStretchAxis::Vertical => atom.ink_ascent + atom.ink_descent,
    };
    if !force_variant && short_target_size <= stretch_advance {
        return Ok(Some(()));
    }

    let scale = base_glyph.font_size / face.units_per_em() as f32;
    let target_units = short_target_size / scale;
    let mut variant_glyph = base_glyph.glyph_id;
    let mut variant_advance = match axis {
        MathStretchAxis::Horizontal => Some(base_glyph.x_advance / scale),
        MathStretchAxis::Vertical => Some((atom.ink_ascent + atom.ink_descent) / scale),
    };
    for variant in construction.variants {
        variant_glyph = variant.variant_glyph;
        variant_advance = Some(variant.advance_measurement as f32);
        if variant.advance_measurement as f32 >= target_units {
            break;
        }
    }

    let variant_reaches_target = variant_advance.is_some_and(|advance| target_units <= advance);
    if variant_reaches_target || construction.assembly.is_none() {
        let Some(glyph) = atom.glyphs.first_mut() else {
            return Ok(None);
        };
        glyph.glyph_id = variant_glyph;
        glyph.x_advance = face
            .glyph_hor_advance(variant_glyph)
            .map(|advance| advance as f32 * scale)
            .or_else(|| variant_advance.map(|advance| advance * scale))
            .unwrap_or(glyph.x_advance);
        if let Some(bounds) = face.glyph_bounding_box(variant_glyph) {
            let ascent = bounds.y_max.max(0) as f32 * scale;
            let descent = (-bounds.y_min).max(0) as f32 * scale;
            atom.metrics.height = ascent + descent;
            atom.metrics.baseline = ascent;
            atom.metrics.ascent = ascent;
            atom.metrics.descent = descent;
            atom.ink_ascent = ascent;
            atom.ink_descent = descent;
            glyph.y = ascent;
        }
        atom.metrics.width = glyph.x_advance;
        return Ok(Some(()));
    }

    if axis == MathStretchAxis::Horizontal
        && let Some(assembly) = construction.assembly
        && assemble_horizontal_glyph_from_math_parts(
            &face,
            atom,
            &base_glyph,
            assembly,
            target_units,
        )
    {
        return Ok(Some(()));
    }

    let Some(glyph) = atom.glyphs.first_mut() else {
        return Ok(None);
    };
    glyph.glyph_id = variant_glyph;
    glyph.x_advance = face
        .glyph_hor_advance(variant_glyph)
        .map(|advance| advance as f32 * scale)
        .or_else(|| variant_advance.map(|advance| advance * scale))
        .unwrap_or(glyph.x_advance);
    if let Some(bounds) = face.glyph_bounding_box(variant_glyph) {
        let ascent = bounds.y_max.max(0) as f32 * scale;
        let descent = (-bounds.y_min).max(0) as f32 * scale;
        atom.metrics.height = ascent + descent;
        atom.metrics.baseline = ascent;
        atom.metrics.ascent = ascent;
        atom.metrics.descent = descent;
        atom.ink_ascent = ascent;
        atom.ink_descent = descent;
        glyph.y = ascent;
    }
    atom.metrics.width = glyph.x_advance;
    Ok(Some(()))
}

fn assemble_horizontal_glyph_from_math_parts(
    face: &ttf_parser::Face<'_>,
    atom: &mut LaidOutMathAtom,
    base_glyph: &LaidOutGlyph,
    assembly: ttf_parser::math::GlyphAssembly<'_>,
    target_units: f32,
) -> bool {
    let Some(math) = face.tables().math else {
        return false;
    };
    let Some(variants) = math.variants else {
        return false;
    };
    let scale = base_glyph.font_size / face.units_per_em() as f32;
    let min_overlap = variants.min_connector_overlap as f32;
    let mut repeat = 0usize;
    let (full_units, ratio, repeat) = loop {
        let mut full = 0.0f32;
        let mut growable = 0.0f32;
        let mut parts = repeated_math_parts(assembly, repeat).into_iter().peekable();

        while let Some(part) = parts.next() {
            let mut advance = part.full_advance as f32;
            if let Some(next) = parts.peek() {
                let max_overlap = part.end_connector_length.min(next.start_connector_length) as f32;
                advance -= max_overlap;
                growable += (max_overlap - min_overlap).max(0.0);
            }
            full += advance;
        }

        let mut ratio = 0.0;
        if full < target_units && growable > 0.0 {
            let delta = target_units - full;
            ratio = (delta / growable).min(1.0);
            full += ratio * growable;
        }

        if target_units <= full || repeat >= MATH_GLYPH_ASSEMBLY_MAX_REPEATS {
            break (full, ratio, repeat);
        }
        repeat += 1;
    };
    let mut cursor = 0.0f32;
    let mut glyphs = Vec::new();
    let mut parts = repeated_math_parts(assembly, repeat).into_iter().peekable();
    let mut first_part = true;
    while let Some(part) = parts.next() {
        let mut advance_units = part.full_advance as f32;
        if let Some(next) = parts.peek() {
            let max_overlap = part.end_connector_length.min(next.start_connector_length) as f32;
            advance_units -= max_overlap;
            advance_units += ratio * (max_overlap - min_overlap);
        }

        let mut glyph = base_glyph.clone();
        glyph.glyph_id = part.glyph_id;
        glyph.unicode = if first_part {
            base_glyph.unicode.clone()
        } else {
            String::new()
        };
        glyph.x_advance = advance_units * scale;
        glyph.x = base_glyph.x + cursor * scale;
        glyph.y = base_glyph.y;
        glyphs.push(glyph);
        cursor += advance_units;
        first_part = false;
    }

    if glyphs.is_empty() {
        return false;
    }

    let (ascent, descent) = glyphs
        .iter()
        .filter_map(|glyph| face.glyph_bounding_box(glyph.glyph_id))
        .map(|bounds| {
            (
                bounds.y_max.max(0) as f32 * scale,
                (-bounds.y_min).max(0) as f32 * scale,
            )
        })
        .fold(
            (0.0f32, 0.0f32),
            |(max_ascent, max_descent), (ascent, descent)| {
                (max_ascent.max(ascent), max_descent.max(descent))
            },
        );
    for glyph in &mut glyphs {
        glyph.y = ascent;
    }
    atom.metrics.width = full_units * scale;
    atom.metrics.height = ascent + descent;
    atom.metrics.baseline = ascent;
    atom.metrics.ascent = ascent;
    atom.metrics.descent = descent;
    atom.ink_ascent = ascent;
    atom.ink_descent = descent;
    atom.italic_correction = assembly.italics_correction.value as f32 * scale;
    atom.glyphs = glyphs;
    atom.draw_order = (0..atom.glyphs.len()).map(LaidOutDrawItem::Glyph).collect();
    true
}

fn repeated_math_parts(
    assembly: ttf_parser::math::GlyphAssembly<'_>,
    repeat: usize,
) -> Vec<ttf_parser::math::GlyphPart> {
    assembly
        .parts
        .into_iter()
        .flat_map(|part| {
            let count = if part.part_flags.extender() {
                repeat
            } else {
                1
            };
            std::iter::repeat_n(part, count)
        })
        .collect()
}

fn delimiter_call_chars(name: &str) -> Option<(char, char)> {
    match name {
        "abs" => Some(('|', '|')),
        "norm" => Some(('‖', '‖')),
        "floor" => Some(('⌊', '⌋')),
        "ceil" => Some(('⌈', '⌉')),
        "round" => Some(('⌊', '⌉')),
        "ceil.l" => Some(('⌈', '⌉')),
        "floor.l" => Some(('⌊', '⌋')),
        "paren.l" => Some(('(', ')')),
        "brace.l" => Some(('{', '}')),
        "bracket.l" => Some(('[', ']')),
        "chevron.l" => Some(('⟨', '⟩')),
        "bar" => Some(('|', '|')),
        "bar.double" => Some(('‖', '‖')),
        _ => None,
    }
}
