// Retained one-line row dispatcher, mapped in UPSTREAM.md to upstream
// `typst-layout/src/math/run.rs`, `mod.rs`, and `typst-library/src/math/ir`.
fn layout_simple_row(
    font: &MathFont,
    math: &MathAst,
    font_size: f32,
) -> Result<Option<SimpleRowLayout>, LabelError> {
    let Some(atom) = layout_simple_nodes_as_atom(font, &math.nodes, font_size, 0)? else {
        return Ok(None);
    };
    Ok(Some(SimpleRowLayout {
        metrics: atom.metrics,
        atoms: vec![atom],
    }))
}

fn layout_simple_nodes_as_atom(
    font: &MathFont,
    nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_nodes_as_atom_with_mid_target(font, nodes, font_size, script_level, None)
}

fn layout_simple_nodes_as_atom_with_mid_target(
    font: &MathFont,
    nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    mid_target_height: Option<f32>,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_nodes_as_atom_with_context(
        font,
        nodes,
        font_size,
        script_level,
        mid_target_height,
        MathLayoutSize::Text,
    )
}

fn layout_simple_nodes_as_atom_with_context(
    font: &MathFont,
    nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    mid_target_height: Option<f32>,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let mut items = Vec::new();
    let mut index = 0usize;
    while index < nodes.len() {
        let node = &nodes[index];
        match node {
            MathNode::Space(_) => {}
            MathNode::Spacing(spacing) => {
                items.push(RowLayoutItem::Spacing(spacing.clone()));
            }
            _ => {
                let (atom, consumed) =
                    if let (MathNode::Attach(attach), Some(MathNode::Group(group))) =
                        (node, nodes.get(index + 1))
                    {
                        if is_identifier_subscript_group_continuation(attach, group) {
                            let Some(atom) = layout_simple_attach_with_bottom_continuation(
                                font,
                                attach,
                                group,
                                font_size,
                                script_level,
                                math_size,
                            )?
                            else {
                                return Ok(None);
                            };
                            (atom, 2)
                        } else {
                            let Some(atom) = layout_simple_node_with_mid_target(
                                font,
                                node,
                                font_size,
                                script_level,
                                mid_target_height,
                                math_size,
                            )?
                            else {
                                return Ok(None);
                            };
                            (atom, 1)
                        }
                    } else {
                        let Some(atom) = layout_simple_node_with_mid_target(
                            font,
                            node,
                            font_size,
                            script_level,
                            mid_target_height,
                            math_size,
                        )?
                        else {
                            return Ok(None);
                        };
                        (atom, 1)
                    };
                items.push(RowLayoutItem::Atom(atom));
                index += consumed;
                continue;
            }
        }
        index += 1;
    }

    if items.is_empty() {
        return Ok(None);
    }

    let mut metrics = TypesetMetrics {
        width: 0.0,
        height: 0.0,
        baseline: 0.0,
        ascent: 0.0,
        descent: 0.0,
    };
    let mut laid_out_atoms = Vec::new();
    let mut previous = None;
    let mut has_material = false;
    let mut row_left_class = None;
    let mut row_right_class = None;

    let mut items = items.into_iter().peekable();
    while let Some(item) = items.next() {
        match item {
            RowLayoutItem::Spacing(spacing) => {
                let next_class = items.peek().and_then(RowLayoutItem::left_class);
                if spacing.weak
                    && (!has_material
                        || previous == Some(SimpleMathClass::Opening)
                        || matches!(
                            next_class,
                            Some(SimpleMathClass::Closing | SimpleMathClass::Fence)
                        ))
                {
                    continue;
                }
                metrics.width += spacing.kind.em_width() * font_size;
                previous = None;
                has_material = true;
                continue;
            }
            RowLayoutItem::Atom(mut atom) => {
                let left_class = resolved_left_class(previous, atom.left_class);
                if let Some(previous) = previous {
                    metrics.width +=
                        math_spacing_for_level(previous, left_class, font_size, script_level);
                }
                if atom.left_class == atom.right_class {
                    atom.right_class = left_class;
                }
                atom.left_class = left_class;
                row_left_class.get_or_insert(atom.left_class);
                offset_atom(&mut atom, metrics.width, 0.0);
                metrics.width += atom.metrics.width;
                metrics.ascent = metrics.ascent.max(atom.metrics.ascent);
                metrics.descent = metrics.descent.max(atom.metrics.descent);
                metrics.height = metrics.ascent + metrics.descent;
                metrics.baseline = metrics.ascent;
                previous = Some(atom.right_class);
                row_right_class = Some(atom.right_class);
                has_material = true;
                laid_out_atoms.push(atom);
            }
        }
    }

    let ink_ascent = laid_out_atoms
        .iter()
        .map(|atom| atom.ink_ascent)
        .fold(0.0, f32::max);
    let ink_descent = laid_out_atoms
        .iter()
        .map(|atom| atom.ink_descent)
        .fold(0.0, f32::max);
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    for atom in laid_out_atoms {
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, atom);
    }

    Ok(Some(LaidOutMathAtom {
        metrics,
        ink_ascent,
        ink_descent,
        left_class: row_left_class.unwrap_or(SimpleMathClass::Normal),
        right_class: row_right_class.unwrap_or(SimpleMathClass::Normal),
        italic_correction: 0.0,
        script_kernable: true,
        glyphs,
        shapes,
        draw_order,
    }))
}

enum RowLayoutItem {
    Atom(LaidOutMathAtom),
    Spacing(MathSpacing),
}

impl RowLayoutItem {
    fn left_class(&self) -> Option<SimpleMathClass> {
        match self {
            Self::Atom(atom) => Some(atom.left_class),
            Self::Spacing(_) => None,
        }
    }
}

fn offset_atom(atom: &mut LaidOutMathAtom, dx: f32, dy: f32) {
    for glyph in &mut atom.glyphs {
        glyph.x += dx;
        glyph.y += dy;
    }
    for shape in &mut atom.shapes {
        shape.x += dx;
        shape.y += dy;
    }
}

fn append_atom_items(
    glyphs: &mut Vec<LaidOutGlyph>,
    shapes: &mut Vec<LaidOutShape>,
    draw_order: &mut Vec<LaidOutDrawItem>,
    mut atom: LaidOutMathAtom,
) {
    let glyph_offset = glyphs.len();
    let shape_offset = shapes.len();
    let pdf_group_offset = glyphs
        .iter()
        .filter_map(|glyph| glyph.pdf_run_group)
        .max()
        .map_or(0, |group| group + 1);
    for glyph in &mut atom.glyphs {
        if let Some(group) = &mut glyph.pdf_run_group {
            *group += pdf_group_offset;
        }
    }
    draw_order.extend(atom.draw_order.iter().map(|item| match *item {
        LaidOutDrawItem::Glyph(index) => LaidOutDrawItem::Glyph(glyph_offset + index),
        LaidOutDrawItem::Shape(index) => LaidOutDrawItem::Shape(shape_offset + index),
    }));
    glyphs.append(&mut atom.glyphs);
    shapes.append(&mut atom.shapes);
}

fn layout_simple_node_with_mid_target(
    font: &MathFont,
    node: &MathNode,
    font_size: f32,
    script_level: u8,
    mid_target_height: Option<f32>,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let Some(atom) = simple_atom(node) {
        let mut layout = if atom.text_operator {
            layout_operator_atom(font, &atom.styled_text, font_size, script_level)?
        } else {
            layout_styled_atom_with_class(
                font,
                &atom.styled_text,
                font_size,
                script_style_feature(script_level),
                atom.class,
            )?
        };
        if atom.class == SimpleMathClass::Large && math_size.is_display() {
            stretch_display_large_operator(font, &mut layout, font_size)?;
        }
        return Ok(Some(layout));
    }

    if let MathNode::Attach(attach) = node {
        return layout_simple_attach(font, attach, font_size, script_level, math_size);
    }

    if let MathNode::Fraction(fraction) = node {
        return layout_simple_fraction(font, fraction, font_size, script_level, math_size);
    }

    if let MathNode::Cancel(cancel) = node {
        return layout_simple_cancel(font, cancel, font_size, script_level, math_size);
    }

    if let MathNode::Accent(accent) = node {
        return layout_simple_accent(font, accent, font_size, script_level, math_size);
    }

    if let MathNode::Group(group) = node {
        return layout_simple_group(font, group, font_size, script_level, math_size);
    }

    if let MathNode::Call(call) = node {
        if let Some(mode) = MathAttachmentMode::from_call_name(&call.name) {
            return layout_simple_attachment_mode_call(
                font,
                call,
                mode,
                font_size,
                script_level,
                math_size,
            );
        }
        if let Some(atom) =
            layout_simple_operator_call(font, call, font_size, script_level, math_size)?
        {
            return Ok(Some(atom));
        }
        if call.name == "frac" {
            return layout_simple_fraction_call(font, call, font_size, script_level, math_size);
        }
        if call.name == "binom" {
            return layout_simple_binom_call(font, call, font_size, script_level, math_size);
        }
        if call.name == "class" {
            return layout_simple_class_call(font, call, font_size, script_level, math_size);
        }
        if let Some(size) = MathSizeCall::from_name(&call.name) {
            return layout_simple_size_call(font, call, size, font_size, script_level);
        }
        if let Some(position) = MathLineCall::from_name(&call.name) {
            return layout_simple_line_call(
                font,
                call,
                position,
                font_size,
                script_level,
                math_size,
            );
        }
        if let Some(kind) = MathUnderOverCall::from_name(&call.name) {
            return layout_simple_under_over_call(
                font,
                call,
                kind,
                font_size,
                script_level,
                math_size,
            );
        }
        if let Some(selection) = MathStyleSelection::from_call_name(&call.name) {
            return layout_simple_variant_call(
                font,
                call,
                selection,
                font_size,
                script_level,
                math_size,
            );
        }
        if call.name == "stretch" {
            return layout_simple_stretch_call(font, call, font_size, script_level, math_size);
        }
        if call.name == "mid" {
            return layout_simple_mid_call(
                font,
                call,
                font_size,
                script_level,
                mid_target_height,
                math_size,
            );
        }
        if let Some((left, right)) = delimiter_call_chars(&call.name) {
            return layout_simple_delimited_call(
                font,
                call,
                left,
                right,
                font_size,
                script_level,
                math_size,
            );
        }
        if call.name == "lr" {
            return layout_simple_lr_call(font, call, font_size, script_level, math_size);
        }
        if call.name == "sqrt" {
            return layout_simple_sqrt(font, call, font_size, script_level, math_size);
        }
        if call.name == "root" {
            return layout_simple_root(font, call, font_size, script_level, math_size);
        }
    }

    Ok(None)
}

fn stretch_display_large_operator(
    font: &MathFont,
    atom: &mut LaidOutMathAtom,
    font_size: f32,
) -> Result<(), LabelError> {
    let target = math_unsigned_constant(font, font_size, |constants| {
        constants.display_operator_min_height()
    })?;
    let _ = stretch_single_glyph_variant(
        font,
        atom,
        MathStretchAxis::Vertical,
        target,
        0.0,
        false,
        "display operator variants",
    )?;
    Ok(())
}

fn layout_simple_class_call(
    font: &MathFont,
    call: &ast::MathCall,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [class_arg, body_arg] = &call.args[..] else {
        return Ok(None);
    };
    let [MathNode::StringLiteral(class)] = &class_arg.nodes[..] else {
        return Ok(None);
    };
    let Some(mut atom) = layout_simple_nodes_as_atom_with_context(
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
    let Some(class) = simple_math_class_from_name(&class.text) else {
        return Ok(None);
    };
    atom.left_class = class;
    atom.right_class = class;
    Ok(Some(atom))
}

fn layout_simple_size_call(
    font: &MathFont,
    call: &ast::MathCall,
    size: MathSizeCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let (font_size, script_level, math_size) = match size {
        MathSizeCall::Display => (font_size, script_level, MathLayoutSize::Display),
        MathSizeCall::Inline => (font_size, script_level, MathLayoutSize::Text),
        MathSizeCall::Script => (
            script_font_size(font, font_size, script_level)?,
            script_level + 1,
            MathLayoutSize::Script,
        ),
        MathSizeCall::ScriptScript => {
            let script_size = script_font_size(font, font_size, script_level)?;
            (
                script_font_size(font, script_size, script_level + 1)?,
                script_level + 2,
                MathLayoutSize::ScriptScript,
            )
        }
    };
    layout_simple_nodes_as_atom_with_context(
        font,
        &arg.nodes,
        font_size,
        script_level,
        None,
        math_size,
    )
}
