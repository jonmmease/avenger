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

#[allow(clippy::too_many_arguments, reason = "Keep the explicit inputs of the existing layout and rendering pipeline.")]
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
    let base_nodes = if accent.dotless {
        dotless_accent_base_nodes(&accent.base)
    } else {
        accent.base.clone()
    };
    let Some(mut base) = layout_simple_nodes_as_atom_with_context(
        font,
        &base_nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let width = base.metrics.width;
    let height = base.metrics.height;
    let baseline = base.metrics.baseline;
    let mut accent_atom = layout_accent_atom(font, accent.accent, font_size, script_level)?;
    let accent_target_width = resolve_relative_math_size(accent.size, width, font_size).max(0.0);
    let _ = stretch_single_glyph_variant(
        font,
        &mut accent_atom,
        MathStretchAxis::Horizontal,
        accent_target_width,
        ACCENT_SHORT_FALL_EM * font_size,
        false,
        "math accent stretch variants",
    )?;
    let base_attach = atom_top_accent_attachment(font, &base)?;
    let accent_attach = atom_top_accent_attachment(font, &accent_atom)?;
    let accent_x = base_attach - accent_attach;
    let accent_y = baseline - accent_atom.metrics.baseline;
    let output_height = if is_bottom_math_accent(accent.accent) {
        height.max(accent_y + accent_atom.metrics.height)
    } else {
        height
    };

    offset_atom(&mut base, 0.0, 0.0);
    offset_atom(&mut accent_atom, accent_x, accent_y);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, base);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, accent_atom);

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height: output_height,
            baseline,
            ascent: baseline,
            descent: output_height - baseline,
        },
        ink_ascent: baseline,
        ink_descent: output_height - baseline,
        left_class: SimpleMathClass::Alphabetic,
        right_class: SimpleMathClass::Alphabetic,
        italic_correction: 0.0,
        script_kernable: false,
        glyphs,
        shapes,
        draw_order,
    }))
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

fn dotless_accent_base_nodes(nodes: &[MathNode]) -> Vec<MathNode> {
    if nodes.len() != 1 {
        return nodes.to_vec();
    }
    match &nodes[0] {
        MathNode::Identifier(identifier) if identifier.symbol.is_none() => {
            dotless_char(&identifier.name).map_or_else(
                || nodes.to_vec(),
                |text| {
                    vec![MathNode::Identifier(ast::MathIdentifier {
                        name: text.to_string(),
                        symbol: None,
                        byte_range: identifier.byte_range.clone(),
                    })]
                },
            )
        }
        MathNode::Text(text) if matches!(text.kind, MathTextKind::Grapheme) => {
            dotless_char(&text.text).map_or_else(
                || nodes.to_vec(),
                |dotless| {
                    vec![MathNode::Text(ast::MathText {
                        text: dotless.to_string(),
                        kind: text.kind,
                        byte_range: text.byte_range.clone(),
                    })]
                },
            )
        }
        _ => nodes.to_vec(),
    }
}

fn dotless_char(text: &str) -> Option<char> {
    match text {
        "i" => Some('ı'),
        "j" => Some('ȷ'),
        _ => None,
    }
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
        math_size,
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
            let script_size = script_font_size(font, font_size, script_level)?;
            let script_script_size = script_font_size(font, script_size, script_level + 1)?;
            layout_simple_nodes_as_atom(font, nodes, script_script_size, script_level + 2)
        })
        .transpose()?
        .flatten();
    if index_nodes.is_some() && index.is_none() {
        return Ok(None);
    }

    let radicand_height = radicand.ink_ascent + radicand.ink_descent;
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
