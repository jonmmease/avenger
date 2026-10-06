// Retained construct layouts, mapped in UPSTREAM.md to upstream
// `accent.rs`, `cancel.rs`, `fraction.rs`, and `radical.rs`.
fn layout_simple_sqrt(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_radical(font, &arg.nodes, None, font_size, script_level, math_size)
}

fn layout_simple_root(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [index, radicand] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_radical(
        font,
        &radicand.nodes,
        Some(index.nodes.as_slice()),
        font_size,
        script_level,
        math_size,
    )
}

fn layout_simple_cancel(
    font: &MathFont,
    cancel: &MathCancel,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(mut body) = layout_simple_nodes_as_atom_with_context(
        font,
        &cancel.body,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };

    let width = body.metrics.width;
    let height = body.metrics.height;
    let diagonal = width.hypot(height);
    if diagonal > 0.0 {
        let default_length = diagonal;
        let length = cancel.options.length.relative * default_length
            + cancel.options.length.absolute_em * font_size;
        let length = length.max(0.0);
        let angle = match cancel.options.angle {
            MathCancelAngle::Auto => width.atan2(height),
            MathCancelAngle::Degrees(degrees) => degrees.to_radians(),
        };
        let invert_first_line = !cancel.options.cross && cancel.options.inverted;
        push_cancel_line(
            &mut body,
            width,
            height,
            length,
            angle,
            invert_first_line,
            font_size,
            &cancel.options.stroke,
        );
        if cancel.options.cross {
            push_cancel_line(
                &mut body,
                width,
                height,
                length,
                angle,
                true,
                font_size,
                &cancel.options.stroke,
            );
        }
    }

    Ok(Some(body))
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn push_cancel_line(
    body: &mut LaidOutMathAtom,
    width: f32,
    height: f32,
    length: f32,
    angle: f32,
    inverted: bool,
    font_size: f32,
    stroke: &DecorationStroke,
) {
    let angle = if inverted { -angle } else { angle };
    let center_x = width / 2.0;
    let center_y = height / 2.0;
    let half_length = length / 2.0;
    let delta_x = angle.sin() * half_length;
    let delta_y = angle.cos() * half_length;
    body.shapes.push(LaidOutShape {
        path: PathData {
            commands: vec![
                PathCommand::MoveTo {
                    x: center_x - delta_x,
                    y: center_y + delta_y,
                },
                PathCommand::LineTo {
                    x: center_x + delta_x,
                    y: center_y - delta_y,
                },
            ],
        },
        x: 0.0,
        y: 0.0,
        stroke: LaidOutStroke::from_decoration(stroke, CANCEL_STROKE_EM * font_size, font_size),
    });
    body.draw_order
        .push(LaidOutDrawItem::Shape(body.shapes.len() - 1));
}

fn layout_simple_accent(
    font: &MathFont,
    accent: &MathAccent,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let top = !is_bottom_math_accent(accent.accent);
    let base_font = if top && accent.dotless {
        font.with_feature(b"dtls")
    } else {
        font.clone()
    };
    let Some(base) = layout_simple_nodes_as_atom_with_context(
        &base_font,
        &accent.base,
        font_size,
        script_level,
        None,
        if top { math_size.cramped() } else { math_size },
    )?
    else {
        return Ok(None);
    };
    let target = resolve_relative_math_size(accent.size, base.metrics.width, font_size).max(0.0);
    compose_math_accent(
        font,
        base,
        accent.accent,
        top,
        target,
        ACCENT_SHORT_FALL_EM * font_size,
        false,
        font_size,
        script_level,
    )
    .map(Some)
}

#[allow(clippy::too_many_arguments)]
fn compose_math_accent(
    font: &MathFont,
    mut base: LaidOutMathAtom,
    accent: char,
    top: bool,
    target: f32,
    short_fall: f32,
    exact_width: bool,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutMathAtom, LabelError> {
    let flatten_height =
        math_constant(font, font_size, |c| c.flattened_accent_base_height().value)?;
    let accent_font = if top && base.metrics.ascent > flatten_height {
        font.with_feature(b"flac")
    } else {
        font.clone()
    };
    let mut ornament = layout_accent_atom(&accent_font, accent, font_size, script_level)?;
    // Some flattened/script alternates lack constructions. Retry the original
    // glyph before accepting an accent too narrow for its base.
    if stretch_single_glyph_variant(
        font,
        &mut ornament,
        MathStretchAxis::Horizontal,
        target,
        short_fall,
        false,
        "math accent variants",
    )?
    .is_none()
        && ornament.metrics.width < target - short_fall
    {
        ornament = layout_accent_atom(font, accent, font_size, 0)?;
        stretch_single_glyph_variant(
            font,
            &mut ornament,
            MathStretchAxis::Horizontal,
            target,
            short_fall,
            false,
            "math accent variants",
        )?;
    }
    let base_attach = (
        atom_top_accent_attachment(font, &base)?,
        base.accent_attachment.map_or(
            (base.metrics.width - base.italic_correction) / 2.0,
            |(_, bottom)| bottom,
        ),
    );
    let attach = if top { base_attach.0 } else { base_attach.1 };
    let ornament_attach = atom_top_accent_attachment(font, &ornament)?;
    let base_x = if exact_width {
        (ornament_attach - attach).max(0.0)
    } else {
        0.0
    };
    let ornament_x = base_x + attach - ornament_attach;
    let width = if exact_width {
        (base_x + base.metrics.width).max(ornament_x + ornament.metrics.width)
    } else {
        base.metrics.width
    };
    let (base_y, ornament_y, height) = if top {
        let accent_base = math_constant(font, font_size, |c| c.accent_base_height().value)?;
        let gap = -ornament.metrics.descent - base.metrics.ascent.min(accent_base);
        let base_y = ornament.metrics.height + gap;
        (base_y, 0.0, base_y + base.metrics.height)
    } else {
        let ornament_y = base.metrics.height - ornament.metrics.ascent;
        (0.0, ornament_y, ornament_y + ornament.metrics.height)
    };
    let baseline = base_y + base.metrics.ascent;
    let mut result = base.clone();
    result.metrics = TypesetMetrics {
        width,
        height,
        baseline,
        ascent: baseline,
        descent: height - baseline,
    };
    result.ink_ascent = baseline;
    result.ink_descent = height - baseline;
    if exact_width {
        result.base_metrics = Some(
            base.base_metrics
                .unwrap_or((base.metrics.ascent, base.metrics.descent)),
        );
    }
    result.accent_attachment = Some((base_attach.0 + base_x, base_attach.1 + base_x));
    result.script_kernable = false;
    result.glyphs.clear();
    result.shapes.clear();
    result.draw_order.clear();
    offset_atom(&mut base, base_x, base_y);
    offset_atom(&mut ornament, ornament_x, ornament_y);
    append_atom_items(
        &mut result.glyphs,
        &mut result.shapes,
        &mut result.draw_order,
        base,
    );
    append_atom_items(
        &mut result.glyphs,
        &mut result.shapes,
        &mut result.draw_order,
        ornament,
    );
    Ok(result)
}

fn is_bottom_math_accent(accent: char) -> bool {
    matches!(
        accent,
        '\u{0323}'
            | '\u{032c}'
            | '\u{032d}'
            | '\u{032e}'
            | '\u{032f}'
            | '\u{0330}'
            | '\u{0331}'
            | '\u{0332}'
            | '\u{0333}'
            | '\u{20e8}'
            | '\u{20ec}'
            | '\u{20ed}'
            | '\u{20ee}'
            | '\u{20ef}'
    )
}

fn layout_simple_radical(
    font: &MathFont,
    radicand_nodes: &[MathNode],
    index_nodes: Option<&[MathNode]>,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(mut radicand) = layout_simple_nodes_as_atom_with_context(
        font,
        radicand_nodes,
        font_size,
        script_level,
        None,
        math_size.cramped(),
    )?
    else {
        return Ok(None);
    };
    let mut sqrt = layout_styled_atom_with_class(
        font,
        "√",
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Opening,
    )?;

    let thickness = math_constant(font, font_size, |constants| {
        constants.radical_rule_thickness().value
    })?;
    let extra_ascender = math_constant(font, font_size, |constants| {
        constants.radical_extra_ascender().value
    })?;
    let mut gap = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.radical_display_style_vertical_gap().value
        } else {
            constants.radical_vertical_gap().value
        }
    })?;
    let kern_before = math_constant(font, font_size, |constants| {
        constants.radical_kern_before_degree().value
    })?;
    let kern_after = math_constant(font, font_size, |constants| {
        constants.radical_kern_after_degree().value
    })?;
    let raise_factor = math_percent(font, |constants| {
        constants.radical_degree_bottom_raise_percent()
    })?;
    let index = index_nodes
        .map(|nodes| {
            let (size, level, context) = math_size.child_context(
                MathLayoutSize::ScriptScript.cramped(),
                font,
                font_size,
                script_level,
            )?;
            layout_simple_nodes_as_atom_with_context(font, nodes, size, level, None, context)
        })
        .transpose()?
        .flatten();
    if index_nodes.is_some() && index.is_none() {
        return Ok(None);
    }

    let radicand_height = radicand.ink_ascent + radicand.ink_descent;
    stretch_single_glyph_variant(
        font,
        &mut sqrt,
        MathStretchAxis::Vertical,
        radicand_height + thickness + gap,
        0.0,
        false,
        "radical variants",
    )?;
    let sqrt_height = sqrt.ink_ascent + sqrt.ink_descent;
    gap = gap.max((sqrt_height - thickness - radicand_height + gap) / 2.0);

    let sqrt_ascent = radicand.ink_ascent + gap + thickness;
    let descent = sqrt_height - sqrt_ascent;
    let inner_ascent = sqrt_ascent + extra_ascender;
    let mut sqrt_offset = 0.0;
    let mut shift_up = 0.0;
    let mut ascent = inner_ascent;
    if let Some(index) = &index {
        sqrt_offset = kern_before + index.metrics.width + kern_after;
        shift_up = raise_factor * (inner_ascent - descent) + index.metrics.descent;
        ascent = ascent.max(shift_up + index.metrics.ascent);
    }
    let sqrt_width = sqrt.metrics.width;
    let line_width = radicand.metrics.width;
    let sqrt_x = sqrt_offset.max(0.0);
    let radicand_x = sqrt_x + sqrt_width;
    let radicand_y = ascent - radicand.ink_ascent;
    let sqrt_y = radicand_y - gap - thickness;
    let line_y = radicand_y - gap - thickness / 2.0;
    let width = radicand_x + line_width;
    let height = ascent + descent;

    let sqrt_dy = sqrt_y + sqrt.ink_ascent - sqrt.metrics.baseline;
    let radicand_dy = radicand_y + radicand.ink_ascent - radicand.metrics.baseline;
    offset_atom(&mut sqrt, sqrt_x, sqrt_dy);
    offset_atom(&mut radicand, radicand_x, radicand_dy);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    if let Some(mut index) = index {
        let index_x = -sqrt_offset.min(0.0) + kern_before;
        let index_y = ascent - index.metrics.ascent - shift_up;
        offset_atom(&mut index, index_x, index_y);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, index);
    }
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, sqrt);
    shapes.push(LaidOutShape {
        path: PathData {
            commands: vec![
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo {
                    x: line_width,
                    y: 0.0,
                },
            ],
        },
        x: radicand_x,
        y: line_y,
        stroke: LaidOutStroke::new(thickness),
    });
    draw_order.push(LaidOutDrawItem::Shape(shapes.len() - 1));
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, radicand);

    finalize_inline_frame_atom(
        width, height, ascent, glyphs, shapes, draw_order, font, font_size,
    )
}

fn layout_simple_fraction(
    font: &MathFont,
    fraction: &ast::MathFraction,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_fraction_nodes(
        font,
        &fraction.numerator,
        &fraction.denominator,
        fraction.style,
        font_size,
        script_level,
        math_size,
    )
}

fn layout_simple_fraction_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [numerator, denominator] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_fraction_nodes(
        font,
        &numerator.nodes,
        &denominator.nodes,
        MathFractionStyle::Vertical,
        font_size,
        script_level,
        math_size,
    )
}

fn layout_simple_binom_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [top, lower @ ..] = &call.args[..] else {
        return Ok(None);
    };
    if lower.is_empty() {
        return Ok(None);
    }
    let bottom_nodes = binom_lower_nodes(lower);
    let Some(stack) = layout_simple_stack_nodes(
        font,
        &top.nodes,
        &bottom_nodes,
        font_size,
        script_level,
        StackRule::None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    layout_simple_delimited_atom(
        font,
        '(',
        stack,
        ')',
        font_size,
        script_level,
        DelimiterTarget::Frame,
        None,
    )
    .map(Some)
}

fn binom_lower_nodes(lower: &[ast::MathArg]) -> Vec<MathNode> {
    let mut nodes = Vec::new();
    for (index, arg) in lower.iter().enumerate() {
        if index > 0 {
            let previous = &lower[index - 1];
            nodes.push(MathNode::Operator(MathOperator {
                operator: ",".to_string(),
                byte_range: previous.byte_range.end..arg.byte_range.start,
            }));
        }
        nodes.extend(arg.nodes.clone());
    }
    nodes
}

fn layout_simple_fraction_nodes(
    font: &MathFont,
    numerator_nodes: &[MathNode],
    denominator_nodes: &[MathNode],
    style: MathFractionStyle,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    match style {
        MathFractionStyle::Vertical => layout_simple_stack_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
            StackRule::Fraction,
            math_size,
        ),
        MathFractionStyle::Skewed => layout_simple_skewed_fraction_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
            math_size,
        ),
        MathFractionStyle::Horizontal => layout_simple_horizontal_fraction_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
            math_size,
        ),
    }
}
