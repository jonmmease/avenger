// Retained under/over, line-decoration, operator, variant, stretch, and `mid`
// helpers, mapped in UPSTREAM.md to upstream math scripts/fenced/text/glyph code.
fn layout_simple_line_call(
    font: &MathFont,
    call: &ast::MathCall,
    position: MathLineCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut body) = layout_simple_nodes_as_atom_with_context(
        font,
        &arg.nodes,
        font_size,
        script_level,
        None,
        if matches!(position, MathLineCall::Above) {
            math_size.cramped()
        } else {
            math_size
        },
    )?
    else {
        return Ok(None);
    };

    let (sep, thickness, gap) = match position {
        MathLineCall::Below => (
            math_constant(font, font_size, |constants| {
                constants.underbar_extra_descender().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.underbar_rule_thickness().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.underbar_vertical_gap().value
            })?,
        ),
        MathLineCall::Above => (
            math_constant(font, font_size, |constants| {
                constants.overbar_extra_ascender().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.overbar_rule_thickness().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.overbar_vertical_gap().value
            })?,
        ),
    };

    let body_width = body.metrics.width;
    let body_height = body.metrics.height;
    let extra_height = sep + thickness + gap;
    let (line_y, baseline) = match position {
        MathLineCall::Below => (body_height + gap + thickness / 2.0, body.metrics.baseline),
        MathLineCall::Above => {
            offset_atom(&mut body, 0.0, extra_height);
            (sep + thickness / 2.0, body.metrics.baseline + extra_height)
        }
    };
    let line_width = match position {
        MathLineCall::Below => (body_width - body.italic_correction).max(0.0),
        MathLineCall::Above => body_width,
    };

    let left_class = body.left_class;
    let right_class = body.right_class;
    let italic_correction = body.italic_correction;
    let script_kernable = body.script_kernable;
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, body);
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
        x: 0.0,
        y: line_y,
        stroke: LaidOutStroke::new(thickness),
    });
    draw_order.push(LaidOutDrawItem::Shape(shapes.len() - 1));

    let Some(mut atom) = finalize_inline_frame_atom(
        body_width,
        body_height + extra_height,
        baseline,
        glyphs,
        shapes,
        draw_order,
        font,
        font_size,
    )?
    else {
        return Ok(None);
    };
    atom.left_class = left_class;
    atom.right_class = right_class;
    atom.italic_correction = italic_correction;
    atom.script_kernable = script_kernable;
    Ok(Some(atom))
}

fn layout_simple_under_over_call(
    font: &MathFont,
    call: &ast::MathCall,
    kind: MathUnderOverCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [body_arg, annotation_args @ ..] = &call.args[..] else {
        return Ok(None);
    };
    if annotation_args.len() > 1 {
        return Ok(None);
    }
    let Some(body) = layout_simple_nodes_as_atom_with_context(
        font,
        &body_arg.nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let top = kind.position == MathUnderOverPosition::Above;
    let ornament = match (top, kind.shape) {
        (true, MathUnderOverShape::Brace) => '⏞',
        (false, MathUnderOverShape::Brace) => '⏟',
        (true, MathUnderOverShape::Bracket) => '⎴',
        (false, MathUnderOverShape::Bracket) => '⎵',
        (true, MathUnderOverShape::Paren) => '⏜',
        (false, MathUnderOverShape::Paren) => '⏝',
        (true, MathUnderOverShape::Shell) => '⏠',
        (false, MathUnderOverShape::Shell) => '⏡',
    };
    let target = body.metrics.width;
    let atom = compose_math_accent(
        font,
        body,
        ornament,
        top,
        target,
        0.0,
        true,
        font_size,
        script_level,
    )?;
    let Some(annotation) = annotation_args.first() else {
        return Ok(Some(atom));
    };
    let context = if top {
        math_size.script_child()
    } else {
        math_size.script_child().cramped()
    };
    let (size, level, context) = math_size.child_context(context, font, font_size, script_level)?;
    let Some(annotation) = layout_simple_nodes_as_atom_with_context(
        font,
        &annotation.nodes,
        size,
        level,
        None,
        context,
    )?
    else {
        return Ok(None);
    };
    let mut slots = LaidOutAttachSlots::default();
    if top {
        slots.top = Some(annotation);
    } else {
        slots.bottom = Some(annotation);
    }
    layout_simple_limit_attach_parts(font, font_size, atom, slots, math_size.cramped)
}

fn layout_simple_variant_call(
    font: &MathFont,
    call: &ast::MathCall,
    selection: MathStyleSelection,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let styled_nodes = style_math_nodes(&arg.nodes, selection);
    layout_simple_nodes_as_atom_with_context(
        font,
        &styled_nodes,
        font_size,
        script_level,
        None,
        math_size,
    )
}

fn layout_simple_stretch_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut atom) = layout_simple_nodes_as_atom_with_context(
        font,
        &arg.nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };

    if atom.glyphs.len() != 1 || !atom.shapes.is_empty() {
        return Ok(Some(atom));
    }

    let size = call.options.stretch_size.unwrap_or_default();
    let horizontal_target =
        resolve_relative_math_size(size, atom.metrics.width, font_size).max(0.0);
    if stretch_single_glyph_variant(
        font,
        &mut atom,
        MathStretchAxis::Horizontal,
        horizontal_target,
        0.0,
        false,
        "math stretch variants",
    )?
    .is_some()
    {
        return Ok(Some(atom));
    }

    let vertical_target =
        resolve_relative_math_size(size, atom.ink_ascent + atom.ink_descent, font_size).max(0.0);
    let _ = stretch_single_glyph_variant(
        font,
        &mut atom,
        MathStretchAxis::Vertical,
        vertical_target,
        0.0,
        false,
        "math stretch variants",
    )?;
    Ok(Some(atom))
}

fn layout_simple_mid_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    target_height: Option<f32>,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut atom) = layout_simple_nodes_as_atom_with_context(
        font,
        &arg.nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };

    atom.left_class = SimpleMathClass::Relation;
    atom.right_class = SimpleMathClass::Relation;

    if let Some(target_height) = target_height
        && atom.glyphs.len() == 1
        && atom.shapes.is_empty()
    {
        let _ = stretch_single_glyph_variant(
            font,
            &mut atom,
            MathStretchAxis::Vertical,
            target_height,
            DELIMITER_SHORT_FALL_EM * font_size,
            false,
            "mid delimiter variants",
        )?;
    }

    Ok(Some(atom))
}

fn layout_simple_operator_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if call.name == "op" || call.name == "op_limits" {
        let [arg] = &call.args[..] else {
            return Ok(None);
        };
        let Some(text) = operator_arg_text(&arg.nodes) else {
            return Ok(None);
        };
        return layout_operator_atom(font, &text, font_size, script_level).map(Some);
    }

    let Some(text) = operator_identifier_text(&call.name) else {
        return Ok(None);
    };
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let nodes = [
        MathNode::Identifier(ast::MathIdentifier {
            name: text.to_string(),
            symbol: None,
            byte_range: call.byte_range.start..call.byte_range.start + call.name.len(),
        }),
        MathNode::Group(ast::MathGroup {
            left: '(',
            right: ')',
            body: arg.nodes.clone(),
            byte_range: arg.byte_range.clone(),
        }),
    ];
    layout_simple_nodes_as_atom_with_context(font, &nodes, font_size, script_level, None, math_size)
}

fn operator_arg_text(nodes: &[MathNode]) -> Option<String> {
    let mut text = String::new();
    for node in nodes {
        match node {
            MathNode::StringLiteral(string) => text.push_str(&string.text),
            MathNode::Identifier(identifier) => {
                text.push_str(identifier.symbol.unwrap_or(&identifier.name));
            }
            MathNode::Operator(operator) => text.push_str(&operator.operator),
            MathNode::Shorthand(shorthand) => text.push_str(shorthand.replacement),
            MathNode::Text(text_node) => text.push_str(&text_node.text),
            _ => return None,
        }
    }
    (!text.is_empty()).then_some(text)
}
