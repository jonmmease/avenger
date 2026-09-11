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
        math_size,
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
    let annotation_arg = match annotation_args {
        [] => None,
        [arg] => Some(arg),
        _ => return Ok(None),
    };
    let Some(mut body) = layout_simple_nodes_as_atom_with_context(
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
    let annotation = annotation_arg
        .map(|arg| {
            let script_size = script_font_size(font, font_size, script_level)?;
            layout_simple_nodes_as_atom(font, &arg.nodes, script_size, script_level + 1)
        })
        .transpose()?
        .flatten();
    if annotation_arg.is_some() && annotation.is_none() {
        return Ok(None);
    };

    let target_width = body.metrics.width.max(font_size * 0.25);
    let (gap, thickness) = match kind.position {
        MathUnderOverPosition::Below => (
            math_constant(font, font_size, |constants| {
                constants.underbar_vertical_gap().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.underbar_rule_thickness().value
            })?,
        ),
        MathUnderOverPosition::Above => (
            math_constant(font, font_size, |constants| {
                constants.overbar_vertical_gap().value
            })?,
            math_constant(font, font_size, |constants| {
                constants.overbar_rule_thickness().value
            })?,
        ),
    };
    let gap = gap.min(font_size * 0.05).max(font_size * 0.02);
    let thickness = thickness.max(font_size * 0.04);
    let mut ornament = layout_under_over_ornament_atom(kind, target_width, font_size, thickness);

    let width = body.metrics.width.max(ornament.metrics.width);
    let body_x = (width - body.metrics.width) / 2.0;
    let ornament_x = (width - ornament.metrics.width) / 2.0;
    let body_height = body.metrics.height;
    let ornament_height = ornament.metrics.height;
    let baseline = match kind.position {
        MathUnderOverPosition::Below => body.metrics.baseline,
        MathUnderOverPosition::Above => {
            offset_atom(&mut body, body_x, ornament_height + gap);
            offset_atom(&mut ornament, ornament_x, 0.0);
            body.metrics.baseline + ornament_height + gap
        }
    };
    if kind.position == MathUnderOverPosition::Below {
        offset_atom(&mut body, body_x, 0.0);
        offset_atom(&mut ornament, ornament_x, body_height + gap);
    }

    let height = body_height + gap + ornament_height;
    let left_class = body.left_class;
    let right_class = body.right_class;
    let italic_correction = body.italic_correction;
    let script_kernable = body.script_kernable;
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, body);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, ornament);

    let Some(mut atom) = finalize_inline_frame_atom(
        width, height, baseline, glyphs, shapes, draw_order, font, font_size,
    )?
    else {
        return Ok(None);
    };
    atom.left_class = left_class;
    atom.right_class = right_class;
    atom.italic_correction = italic_correction;
    atom.script_kernable = script_kernable;
    if let Some(annotation) = annotation {
        let mut slots = LaidOutAttachSlots::default();
        match kind.position {
            MathUnderOverPosition::Below => slots.bottom = Some(annotation),
            MathUnderOverPosition::Above => slots.top = Some(annotation),
        }
        layout_simple_limit_attach_parts(font, font_size, atom, slots)
    } else {
        Ok(Some(atom))
    }
}

fn layout_under_over_ornament_atom(
    kind: MathUnderOverCall,
    width: f32,
    font_size: f32,
    thickness: f32,
) -> LaidOutMathAtom {
    // Keep the visible ornament compact while reserving enough outer line-box
    // room to survive the inline math leading slack in `finalize_inline_frame_atom`.
    let visible_height = (font_size * 0.18).max(thickness * 2.0);
    let height = (font_size * 0.52).max(visible_height);
    let path = under_over_ornament_path(kind, width, visible_height);
    let shape_y = match kind.position {
        MathUnderOverPosition::Below => 0.0,
        MathUnderOverPosition::Above => height - visible_height,
    };
    LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height,
            baseline: height,
            ascent: height,
            descent: 0.0,
        },
        ink_ascent: height,
        ink_descent: 0.0,
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: false,
        glyphs: Vec::new(),
        shapes: vec![LaidOutShape {
            path,
            x: 0.0,
            y: shape_y,
            stroke: LaidOutStroke::new(thickness),
        }],
        draw_order: vec![LaidOutDrawItem::Shape(0)],
    }
}

fn under_over_ornament_path(kind: MathUnderOverCall, width: f32, height: f32) -> PathData {
    let w = width.max(0.0);
    let h = height.max(0.0);
    let mid = w / 2.0;
    let quarter = w / 4.0;
    let three_quarter = 3.0 * w / 4.0;
    let top = 0.0;
    let bottom = h;

    let commands = match (kind.position, kind.shape) {
        (MathUnderOverPosition::Above, MathUnderOverShape::Bracket) => vec![
            PathCommand::MoveTo { x: 0.0, y: bottom },
            PathCommand::LineTo { x: 0.0, y: top },
            PathCommand::LineTo { x: w, y: top },
            PathCommand::LineTo { x: w, y: bottom },
        ],
        (MathUnderOverPosition::Below, MathUnderOverShape::Bracket) => vec![
            PathCommand::MoveTo { x: 0.0, y: top },
            PathCommand::LineTo { x: 0.0, y: bottom },
            PathCommand::LineTo { x: w, y: bottom },
            PathCommand::LineTo { x: w, y: top },
        ],
        (MathUnderOverPosition::Above, MathUnderOverShape::Paren) => vec![
            PathCommand::MoveTo { x: 0.0, y: bottom },
            PathCommand::CubicTo {
                x1: quarter,
                y1: top,
                x2: three_quarter,
                y2: top,
                x: w,
                y: bottom,
            },
        ],
        (MathUnderOverPosition::Below, MathUnderOverShape::Paren) => vec![
            PathCommand::MoveTo { x: 0.0, y: top },
            PathCommand::CubicTo {
                x1: quarter,
                y1: bottom,
                x2: three_quarter,
                y2: bottom,
                x: w,
                y: top,
            },
        ],
        (MathUnderOverPosition::Above, MathUnderOverShape::Shell) => vec![
            PathCommand::MoveTo { x: 0.0, y: bottom },
            PathCommand::QuadTo {
                x1: mid,
                y1: top,
                x: w,
                y: bottom,
            },
        ],
        (MathUnderOverPosition::Below, MathUnderOverShape::Shell) => vec![
            PathCommand::MoveTo { x: 0.0, y: top },
            PathCommand::QuadTo {
                x1: mid,
                y1: bottom,
                x: w,
                y: top,
            },
        ],
        (MathUnderOverPosition::Above, MathUnderOverShape::Brace) => vec![
            PathCommand::MoveTo { x: 0.0, y: bottom },
            PathCommand::CubicTo {
                x1: quarter * 0.5,
                y1: bottom,
                x2: quarter * 0.7,
                y2: top,
                x: quarter,
                y: top,
            },
            PathCommand::CubicTo {
                x1: mid * 0.85,
                y1: top,
                x2: mid * 0.75,
                y2: bottom * 0.55,
                x: mid,
                y: bottom * 0.55,
            },
            PathCommand::CubicTo {
                x1: mid * 1.25,
                y1: bottom * 0.55,
                x2: mid * 1.15,
                y2: top,
                x: three_quarter,
                y: top,
            },
            PathCommand::CubicTo {
                x1: w - quarter * 0.7,
                y1: top,
                x2: w - quarter * 0.5,
                y2: bottom,
                x: w,
                y: bottom,
            },
        ],
        (MathUnderOverPosition::Below, MathUnderOverShape::Brace) => vec![
            PathCommand::MoveTo { x: 0.0, y: top },
            PathCommand::CubicTo {
                x1: quarter * 0.5,
                y1: top,
                x2: quarter * 0.7,
                y2: bottom,
                x: quarter,
                y: bottom,
            },
            PathCommand::CubicTo {
                x1: mid * 0.85,
                y1: bottom,
                x2: mid * 0.75,
                y2: bottom * 0.45,
                x: mid,
                y: bottom * 0.45,
            },
            PathCommand::CubicTo {
                x1: mid * 1.25,
                y1: bottom * 0.45,
                x2: mid * 1.15,
                y2: bottom,
                x: three_quarter,
                y: bottom,
            },
            PathCommand::CubicTo {
                x1: w - quarter * 0.7,
                y1: bottom,
                x2: w - quarter * 0.5,
                y2: top,
                x: w,
                y: top,
            },
        ],
    };

    PathData { commands }
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
