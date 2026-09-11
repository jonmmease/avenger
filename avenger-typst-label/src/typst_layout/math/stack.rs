#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackRule {
    Fraction,
    None,
}

// Retained fraction stack formulas, mapped in UPSTREAM.md to upstream
// `typst-layout/src/math/fraction.rs`.
fn layout_simple_stack_nodes(
    font: &MathFont,
    numerator_nodes: &[MathNode],
    denominator_nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    rule: StackRule,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let child_size = math_size.fraction_child();
    let (child_font_size, child_script_level, child_math_size) =
        math_size.child_context(child_size, font, font_size, script_level)?;
    let Some(mut numerator) = layout_fraction_child_nodes(
        font,
        numerator_nodes,
        child_font_size,
        child_script_level,
        child_math_size,
    )?
    else {
        return Ok(None);
    };
    let Some(mut denominator) = layout_fraction_child_nodes(
        font,
        denominator_nodes,
        child_font_size,
        child_script_level,
        child_math_size.cramped(),
    )?
    else {
        return Ok(None);
    };

    if rule == StackRule::None {
        return layout_simple_no_rule_stack(font, numerator, denominator, font_size, math_size);
    }

    let axis = math_constant(font, font_size, |constants| constants.axis_height().value)?;
    let thickness = math_constant(font, font_size, |constants| {
        constants.fraction_rule_thickness().value
    })?;
    let shift_up = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.fraction_numerator_display_style_shift_up().value
        } else {
            constants.fraction_numerator_shift_up().value
        }
    })?;
    let shift_down = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants
                .fraction_denominator_display_style_shift_down()
                .value
        } else {
            constants.fraction_denominator_shift_down().value
        }
    })?;
    let numerator_gap_min = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.fraction_num_display_style_gap_min().value
        } else {
            constants.fraction_numerator_gap_min().value
        }
    })?;
    let denominator_gap_min = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.fraction_denom_display_style_gap_min().value
        } else {
            constants.fraction_denominator_gap_min().value
        }
    })?;
    let padding = FRACTION_PADDING_EM * font_size;

    let numerator_height = numerator.ink_ascent + numerator.ink_descent;
    let denominator_height = denominator.ink_ascent + denominator.ink_descent;
    let numerator_gap =
        (shift_up - (axis + thickness / 2.0) - numerator.ink_descent).max(numerator_gap_min);
    let denominator_gap =
        (shift_down + (axis - thickness / 2.0) - denominator.ink_ascent).max(denominator_gap_min);
    let line_width = numerator.metrics.width.max(denominator.metrics.width);
    let width = line_width + 2.0 * padding;
    let height =
        numerator_height + numerator_gap + thickness + denominator_gap + denominator_height;
    let line_x = (width - line_width) / 2.0;
    let line_y = numerator_height + numerator_gap + thickness / 2.0;
    let baseline = line_y + axis;
    let numerator_x = (width - numerator.metrics.width) / 2.0;
    let numerator_baseline = numerator.ink_ascent;
    let denominator_x = (width - denominator.metrics.width) / 2.0;
    let denominator_y = height - denominator_height;
    let denominator_baseline = denominator_y + denominator.ink_ascent;
    let numerator_dy = numerator_baseline - numerator.metrics.baseline;
    let denominator_dy = denominator_baseline - denominator.metrics.baseline;

    offset_atom(&mut numerator, numerator_x, numerator_dy);
    offset_atom(&mut denominator, denominator_x, denominator_dy);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, numerator);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, denominator);
    if rule == StackRule::Fraction {
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
            x: line_x,
            y: line_y,
            stroke: LaidOutStroke::new(thickness),
        });
        draw_order.push(LaidOutDrawItem::Shape(shapes.len() - 1));
    }

    finalize_inline_frame_atom(
        width, height, baseline, glyphs, shapes, draw_order, font, font_size,
    )
}

fn layout_simple_no_rule_stack(
    font: &MathFont,
    mut numerator: LaidOutMathAtom,
    mut denominator: LaidOutMathAtom,
    font_size: f32,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let shift_up = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.stack_top_display_style_shift_up().value
        } else {
            constants.stack_top_shift_up().value
        }
    })?;
    let shift_down = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.stack_bottom_display_style_shift_down().value
        } else {
            constants.stack_bottom_shift_down().value
        }
    })?;
    let gap_min = math_constant(font, font_size, |constants| {
        if math_size.is_display() {
            constants.stack_display_style_gap_min().value
        } else {
            constants.stack_gap_min().value
        }
    })?;
    let padding = FRACTION_PADDING_EM * font_size;

    let gap = (shift_up - numerator.metrics.descent) + (shift_down - denominator.metrics.ascent);
    let gap = gap.max(gap_min);
    let width = numerator.metrics.width.max(denominator.metrics.width) + 2.0 * padding;
    let height = numerator.metrics.height + gap + denominator.metrics.height;
    let baseline = numerator.metrics.ascent + shift_up + (gap_min - gap).max(0.0) / 2.0;
    let numerator_x = (width - numerator.metrics.width) / 2.0;
    let denominator_x = (width - denominator.metrics.width) / 2.0;
    let denominator_y = height - denominator.metrics.height;

    offset_atom(&mut numerator, numerator_x, 0.0);
    offset_atom(&mut denominator, denominator_x, denominator_y);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, numerator);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, denominator);

    finalize_inline_frame_atom(
        width, height, baseline, glyphs, shapes, draw_order, font, font_size,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn finalize_inline_frame_atom(
    width: f32,
    height: f32,
    baseline: f32,
    glyphs: Vec<LaidOutGlyph>,
    shapes: Vec<LaidOutShape>,
    draw_order: Vec<LaidOutDrawItem>,
    _font: &MathFont,
    _font_size: f32,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height,
            baseline,
            ascent: baseline,
            descent: height - baseline,
        },
        ink_ascent: baseline,
        ink_descent: height - baseline,
        left_spacing: None,
        right_spacing: None,
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: true,
        base_metrics: Some((baseline, height - baseline)),
        accent_attachment: None,
        spaced: false,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn layout_simple_horizontal_fraction_nodes(
    font: &MathFont,
    numerator_nodes: &[MathNode],
    denominator_nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(numerator) = layout_simple_nodes_as_atom_with_context(
        font,
        numerator_nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let slash = layout_styled_atom_with_class(
        font,
        "/",
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Binary,
    )?;
    let Some(denominator) = layout_simple_nodes_as_atom_with_context(
        font,
        denominator_nodes,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let left_class = numerator.left_class;
    let right_class = denominator.right_class;

    Ok(Some(layout_atoms_without_spacing(
        vec![numerator, slash, denominator],
        left_class,
        right_class,
        true,
    )))
}

fn layout_simple_skewed_fraction_nodes(
    font: &MathFont,
    numerator_nodes: &[MathNode],
    denominator_nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let child_size = math_size.fraction_child();
    let (child_font_size, child_script_level, child_math_size) =
        math_size.child_context(child_size, font, font_size, script_level)?;
    let Some(mut numerator) = layout_fraction_child_nodes(
        font,
        numerator_nodes,
        child_font_size,
        child_script_level,
        child_math_size,
    )?
    else {
        return Ok(None);
    };
    let Some(mut denominator) = layout_fraction_child_nodes(
        font,
        denominator_nodes,
        child_font_size,
        child_script_level,
        child_math_size.cramped(),
    )?
    else {
        return Ok(None);
    };

    let vgap = math_constant(font, font_size, |constants| {
        constants.skewed_fraction_vertical_gap().value
    })?;
    let hgap = math_constant(font, font_size, |constants| {
        constants.skewed_fraction_horizontal_gap().value
    })?;
    let axis = math_constant(font, font_size, |constants| constants.axis_height().value)?;

    let mut fraction_height = numerator.metrics.height + denominator.metrics.height + vgap;
    let mut slash = layout_delimiter_atom_with_target(
        font,
        '⁄',
        font_size,
        script_level,
        fraction_height,
        SimpleMathClass::Binary,
        true,
    )?;
    let vertical_offset = ((slash.metrics.height - fraction_height).max(0.0)) / 2.0;
    fraction_height = fraction_height.max(slash.metrics.height);

    let mut slash_x = numerator.metrics.width + hgap / 2.0 - slash.metrics.width / 2.0;
    let slash_y = fraction_height / 2.0 - slash.metrics.height / 2.0;
    let mut numerator_x = 0.0;
    let numerator_y = vertical_offset;
    let mut denominator_x = numerator_x + numerator.metrics.width + hgap;
    let denominator_y = numerator_y + numerator.metrics.height + vgap;
    let horizontal_offset = (-slash_x).max(0.0);
    slash_x += horizontal_offset;
    numerator_x += horizontal_offset;
    denominator_x += horizontal_offset;

    let width = (denominator_x + denominator.metrics.width)
        .max(slash_x + slash.metrics.width)
        .max(numerator_x + numerator.metrics.width);
    let baseline = fraction_height / 2.0 + axis;
    offset_atom_to_top_left(&mut numerator, numerator_x, numerator_y);
    offset_atom_to_top_left(&mut denominator, denominator_x, denominator_y);
    offset_atom_to_top_left(&mut slash, slash_x, slash_y);

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, numerator);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, denominator);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, slash);

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height: fraction_height,
            baseline,
            ascent: baseline,
            descent: fraction_height - baseline,
        },
        ink_ascent: baseline,
        ink_descent: fraction_height - baseline,
        left_spacing: None,
        right_spacing: None,
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: true,
        base_metrics: None,
        accent_attachment: None,
        spaced: false,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn layout_atoms_without_spacing(
    atoms: Vec<LaidOutMathAtom>,
    left_class: SimpleMathClass,
    right_class: SimpleMathClass,
    script_kernable: bool,
) -> LaidOutMathAtom {
    let baseline = atoms
        .iter()
        .map(|atom| atom.metrics.baseline)
        .fold(0.0, f32::max);
    let descent = atoms
        .iter()
        .map(|atom| atom.metrics.descent)
        .fold(0.0, f32::max);
    let height = baseline + descent;
    let mut width = 0.0;
    let mut ink_ascent: f32 = 0.0;
    let mut ink_descent: f32 = 0.0;
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();

    for mut atom in atoms {
        let dy = baseline - atom.metrics.baseline;
        ink_ascent = ink_ascent.max(atom.ink_ascent);
        ink_descent = ink_descent.max(atom.ink_descent);
        offset_atom(&mut atom, width, dy);
        width += atom.metrics.width;
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, atom);
    }

    LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height,
            baseline,
            ascent: baseline,
            descent,
        },
        ink_ascent,
        ink_descent,
        left_spacing: None,
        right_spacing: None,
        left_class,
        right_class,
        italic_correction: 0.0,
        script_kernable,
        base_metrics: Some((baseline, descent)),
        accent_attachment: None,
        spaced: false,
        glyphs,
        shapes,
        draw_order,
    }
}

fn offset_atom_to_top_left(atom: &mut LaidOutMathAtom, x: f32, y: f32) {
    offset_atom(atom, x, y + atom.metrics.ascent - atom.metrics.baseline);
}

fn layout_fraction_child(
    font: &MathFont,
    node: &MathNode,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let MathNode::Group(group) = node {
        return layout_simple_nodes_as_atom_with_context(
            font,
            &group.body,
            font_size,
            script_level,
            None,
            math_size,
        );
    }
    layout_simple_node_with_mid_target(font, node, font_size, script_level, None, math_size)
}

fn layout_fraction_child_nodes(
    font: &MathFont,
    nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let [node] = nodes {
        return layout_fraction_child(font, node, font_size, script_level, math_size);
    }
    layout_simple_nodes_as_atom_with_context(font, nodes, font_size, script_level, None, math_size)
}

const FRACTION_PADDING_EM: f32 = 0.1;
const INLINE_MATH_LEADING_SLACK_EM: f32 = 0.65 * 0.7;
const CANCEL_STROKE_EM: f32 = 0.05;
const SCRIPT_SLOT_PAIR_GAP_EM: f32 = 0.08;
const PRIME_CHAR: char = '′';
