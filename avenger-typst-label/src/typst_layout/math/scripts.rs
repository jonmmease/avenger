// Retained script, prime, and limit placement, mapped in UPSTREAM.md to
// upstream `typst-layout/src/math/scripts.rs` and `typst-library/src/math/attach.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathAttachmentMode {
    Scripts,
    DisplayLimits,
    Limits,
}

impl MathAttachmentMode {
    fn from_call_name(name: &str) -> Option<Self> {
        match name {
            "scripts" | "op" => Some(Self::Scripts),
            "limits_display" | "op_limits" => Some(Self::DisplayLimits),
            "limits" => Some(Self::Limits),
            _ => None,
        }
    }

    fn explicit_from_base_node(node: &MathNode) -> Option<Self> {
        if let MathNode::Call(call) = node {
            Self::from_call_name(&call.name).or_else(|| {
                MathSizeCall::from_name(&call.name)
                    .and_then(|_| only_arg_node(call))
                    .and_then(Self::explicit_from_base_node)
            })
        } else {
            None
        }
    }

    fn default_for_base(base: &LaidOutMathAtom, math_size: MathLayoutSize) -> Self {
        if (base.left_class == SimpleMathClass::Relation
            && base.right_class == SimpleMathClass::Relation)
            || (base.left_class == SimpleMathClass::Large
                && base.right_class == SimpleMathClass::Large
                && default_large_operator_uses_display_limits(base)
                && math_size.is_display())
        {
            Self::Limits
        } else {
            Self::Scripts
        }
    }

    fn resolve(self, math_size: MathLayoutSize) -> Self {
        match self {
            Self::DisplayLimits if math_size.is_display() => Self::Limits,
            Self::DisplayLimits => Self::Scripts,
            mode => mode,
        }
    }
}

fn only_arg_node(call: &ast::MathCall) -> Option<&MathNode> {
    let [arg] = &call.args[..] else {
        return None;
    };
    let [node] = &arg.nodes[..] else {
        return None;
    };
    Some(node)
}

fn base_math_size_context(node: &MathNode, current: MathLayoutSize) -> MathLayoutSize {
    let MathNode::Call(call) = node else {
        return current;
    };
    let Some(size) = MathSizeCall::from_name(&call.name) else {
        return current;
    };
    match size {
        MathSizeCall::Display => MathLayoutSize::Display,
        MathSizeCall::Inline => MathLayoutSize::Text,
        MathSizeCall::Script => MathLayoutSize::Script,
        MathSizeCall::ScriptScript => MathLayoutSize::ScriptScript,
    }
}

fn default_large_operator_uses_display_limits(base: &LaidOutMathAtom) -> bool {
    let text = base
        .glyphs
        .iter()
        .map(|glyph| glyph.unicode.as_str())
        .collect::<String>();
    if text.is_empty() || is_integral_operator_text(&text) {
        return false;
    }
    text.chars().count() == 1 || text_operator_uses_display_limits(&text)
}

fn is_integral_operator_text(text: &str) -> bool {
    text.chars()
        .any(|ch| ('∫'..='∳').contains(&ch) || ('⨋'..='⨜').contains(&ch))
}

fn text_operator_uses_display_limits(text: &str) -> bool {
    matches!(
        text,
        "det"
            | "gcd"
            | "lcm"
            | "inf"
            | "lim"
            | "lim\u{2009}inf"
            | "lim\u{2009}sup"
            | "max"
            | "min"
            | "Pr"
            | "sup"
    )
}

fn layout_simple_attachment_mode_call(
    font: &MathFont,
    call: &ast::MathCall,
    _mode: MathAttachmentMode,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
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

fn layout_simple_attach(
    font: &MathFont,
    attach: &ast::MathAttach,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(base) = layout_simple_node_with_mid_target(
        font,
        &attach.base,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let base_math_size = base_math_size_context(&attach.base, math_size);
    let mode = MathAttachmentMode::explicit_from_base_node(&attach.base)
        .unwrap_or_else(|| MathAttachmentMode::default_for_base(&base, base_math_size))
        .resolve(base_math_size);
    let (script_font_size, script_level, script_math_size) =
        math_size.child_context(math_size.script_child(), font, font_size, script_level)?;
    let slots = layout_attach_slots(
        font,
        attach,
        script_font_size,
        script_level,
        script_math_size,
    )?;
    let slots =
        attach_slots_with_primes(font, slots, attach.primes, script_font_size, script_level)?;
    if attach_slots_missing_requested(attach, &slots) {
        return Ok(None);
    }

    layout_simple_attach_parts(font, font_size, base, slots, mode, math_size.cramped)
}

fn layout_simple_attach_with_bottom_continuation(
    font: &MathFont,
    attach: &ast::MathAttach,
    group: &ast::MathGroup,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some([bottom_node]) = attach.bottom.as_deref() else {
        return Ok(None);
    };
    let Some(base) = layout_simple_node_with_mid_target(
        font,
        &attach.base,
        font_size,
        script_level,
        None,
        math_size,
    )?
    else {
        return Ok(None);
    };
    let base_math_size = base_math_size_context(&attach.base, math_size);
    let mode = MathAttachmentMode::explicit_from_base_node(&attach.base)
        .unwrap_or_else(|| MathAttachmentMode::default_for_base(&base, base_math_size))
        .resolve(base_math_size);
    let (script_font_size, script_level, script_math_size) =
        math_size.child_context(math_size.script_child(), font, font_size, script_level)?;
    let mut slots = layout_attach_slots(
        font,
        attach,
        script_font_size,
        script_level,
        script_math_size,
    )?;
    slots = attach_slots_with_primes(font, slots, attach.primes, script_font_size, script_level)?;
    let bottom_nodes = [bottom_node.clone(), MathNode::Group(group.clone())];
    let bottom = layout_simple_nodes_as_atom_with_context(
        font,
        &bottom_nodes,
        script_font_size,
        script_level,
        None,
        script_math_size.cramped(),
    )?;
    if attach_slots_missing_requested(attach, &slots) || bottom.is_none() {
        return Ok(None);
    }
    slots.bottom = bottom;

    layout_simple_attach_parts(font, font_size, base, slots, mode, math_size.cramped)
}

fn is_identifier_subscript_group_continuation(
    attach: &ast::MathAttach,
    group: &ast::MathGroup,
) -> bool {
    // Typst parses `_n(x)` like an identifier subscript expression with an
    // adjacent call-style group, while `_0(x)` leaves `(x)` at the outer level.
    attach.byte_range.end == group.byte_range.start
        && matches!(attach.bottom.as_deref(), Some([MathNode::Identifier(_)]))
}

#[derive(Default)]
struct LaidOutAttachSlots {
    top: Option<LaidOutMathAtom>,
    bottom: Option<LaidOutMathAtom>,
    top_left: Option<LaidOutMathAtom>,
    top_right: Option<LaidOutMathAtom>,
    bottom_left: Option<LaidOutMathAtom>,
    bottom_right: Option<LaidOutMathAtom>,
}

fn layout_attach_slots(
    font: &MathFont,
    attach: &ast::MathAttach,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<LaidOutAttachSlots, LabelError> {
    Ok(LaidOutAttachSlots {
        top: layout_script_nodes(
            font,
            attach.top.as_deref(),
            font_size,
            script_level,
            math_size,
        )?,
        bottom: layout_script_nodes(
            font,
            attach.bottom.as_deref(),
            font_size,
            script_level,
            math_size.cramped(),
        )?,
        top_left: layout_script_nodes(
            font,
            attach.top_left.as_deref(),
            font_size,
            script_level,
            math_size,
        )?,
        top_right: layout_script_nodes(
            font,
            attach.top_right.as_deref(),
            font_size,
            script_level,
            math_size,
        )?,
        bottom_left: layout_script_nodes(
            font,
            attach.bottom_left.as_deref(),
            font_size,
            script_level,
            math_size.cramped(),
        )?,
        bottom_right: layout_script_nodes(
            font,
            attach.bottom_right.as_deref(),
            font_size,
            script_level,
            math_size.cramped(),
        )?,
    })
}

fn attach_slots_with_primes(
    font: &MathFont,
    mut slots: LaidOutAttachSlots,
    primes: usize,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutAttachSlots, LabelError> {
    let Some(prime_atom) = layout_prime_slot(font, primes, font_size, script_level)? else {
        return Ok(slots);
    };
    slots.top_right = combine_script_slots(font_size, slots.top_right, Some(prime_atom))?;
    Ok(slots)
}

fn layout_prime_slot(
    font: &MathFont,
    primes: usize,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if primes == 0 {
        return Ok(None);
    }
    let prime = layout_styled_atom_with_class(
        font,
        &PRIME_CHAR.to_string(),
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Normal,
    )?;
    let advance = prime.metrics.width;
    let mut result = prime.clone();
    result.glyphs.clear();
    result.draw_order.clear();
    for index in 0..primes {
        let mut part = prime.clone();
        offset_atom(&mut part, index as f32 * advance / 2.0, 0.0);
        append_atom_items(
            &mut result.glyphs,
            &mut result.shapes,
            &mut result.draw_order,
            part,
        );
    }
    result.metrics.width = advance * (primes + 1) as f32 / 2.0;
    result.script_kernable = false;
    Ok(Some(result))
}

fn layout_script_nodes(
    font: &MathFont,
    nodes: Option<&[MathNode]>,
    font_size: f32,
    script_level: u8,
    math_size: MathLayoutSize,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(nodes) = nodes else {
        return Ok(None);
    };
    if let [node] = nodes {
        return layout_script_child(font, node, font_size, script_level, math_size);
    }
    layout_simple_nodes_as_atom_with_context(font, nodes, font_size, script_level, None, math_size)
}

fn attach_slots_missing_requested(attach: &ast::MathAttach, slots: &LaidOutAttachSlots) -> bool {
    (attach.top.is_some() && slots.top.is_none())
        || (attach.bottom.is_some() && slots.bottom.is_none())
        || (attach.top_left.is_some() && slots.top_left.is_none())
        || (attach.top_right.is_some() && slots.top_right.is_none())
        || (attach.bottom_left.is_some() && slots.bottom_left.is_none())
        || (attach.bottom_right.is_some() && slots.bottom_right.is_none())
}

fn layout_simple_attach_parts(
    font: &MathFont,
    font_size: f32,
    base: LaidOutMathAtom,
    slots: LaidOutAttachSlots,
    mode: MathAttachmentMode,
    cramped: bool,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    match mode {
        MathAttachmentMode::Scripts | MathAttachmentMode::DisplayLimits => {
            layout_simple_script_attach_parts(font, font_size, base, slots, cramped)
        }
        MathAttachmentMode::Limits => {
            layout_simple_limit_attach_parts(font, font_size, base, slots, cramped)
        }
    }
}

fn layout_simple_script_attach_parts(
    font: &MathFont,
    font_size: f32,
    base: LaidOutMathAtom,
    slots: LaidOutAttachSlots,
    cramped: bool,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if [
        &slots.top,
        &slots.bottom,
        &slots.top_left,
        &slots.top_right,
        &slots.bottom_left,
        &slots.bottom_right,
    ]
    .iter()
    .all(|slot| slot.is_none())
    {
        return Ok(Some(base));
    }
    let post_top = combine_script_slots(font_size, slots.top, slots.top_right)?;
    let post_bottom = combine_script_slots(font_size, slots.bottom, slots.bottom_right)?;
    let top_ref = post_top.as_ref().or(slots.top_left.as_ref());
    let bottom_ref = post_bottom.as_ref().or(slots.bottom_left.as_ref());
    let (shift_up, shift_down) =
        compute_script_shifts(font, font_size, &base, top_ref, bottom_ref, cramped)?;
    let space_after_script = math_constant(font, font_size, |constants| {
        constants.space_after_script().value
    })?;
    let post_top_kern = post_top
        .as_ref()
        .map(|top| math_kern(font, &base, top, shift_up, ScriptCorner::TopRight))
        .transpose()?
        .unwrap_or_default();
    let post_bottom_kern = post_bottom
        .as_ref()
        .map(|bottom| {
            math_kern(font, &base, bottom, shift_down, ScriptCorner::BottomRight)
                .map(|kern| kern - base.italic_correction)
        })
        .transpose()?
        .unwrap_or_default();
    let pre_top_kern = slots
        .top_left
        .as_ref()
        .map(|top| math_kern(font, &base, top, shift_up, ScriptCorner::TopLeft))
        .transpose()?
        .unwrap_or_default();
    let pre_bottom_kern = slots
        .bottom_left
        .as_ref()
        .map(|bottom| math_kern(font, &base, bottom, shift_down, ScriptCorner::BottomLeft))
        .transpose()?
        .unwrap_or_default();

    let top_post_width = post_top
        .as_ref()
        .map(|top| space_after_script + top.metrics.width + post_top_kern)
        .unwrap_or_default();
    let bottom_post_width = post_bottom
        .as_ref()
        .map(|bottom| space_after_script + bottom.metrics.width + post_bottom_kern)
        .unwrap_or_default();
    let top_pre_width = slots
        .top_left
        .as_ref()
        .map(|top| space_after_script + top.metrics.width + pre_top_kern)
        .unwrap_or_default();
    let bottom_pre_width = slots
        .bottom_left
        .as_ref()
        .map(|bottom| space_after_script + bottom.metrics.width + pre_bottom_kern)
        .unwrap_or_default();
    let base_width = base.metrics.width;
    let base_left_class = base.left_class;
    let base_right_class = base.right_class;
    let pre_width = top_pre_width.max(bottom_pre_width);
    let post_width = top_post_width.max(bottom_post_width);
    let width = pre_width + base_width + post_width;
    let baseline = base.metrics.baseline;
    let ink_ascent = base
        .ink_ascent
        .max(
            post_top
                .as_ref()
                .map_or(0.0, |top| shift_up + top.ink_ascent),
        )
        .max(
            slots
                .top_left
                .as_ref()
                .map_or(0.0, |top| shift_up + top.ink_ascent),
        )
        .max(
            post_bottom
                .as_ref()
                .map_or(0.0, |bottom| bottom.ink_ascent - shift_down),
        )
        .max(
            slots
                .bottom_left
                .as_ref()
                .map_or(0.0, |bottom| bottom.ink_ascent - shift_down),
        );
    let ink_descent = base
        .ink_descent
        .max(
            post_top
                .as_ref()
                .map_or(0.0, |top| top.ink_descent - shift_up),
        )
        .max(
            slots
                .top_left
                .as_ref()
                .map_or(0.0, |top| top.ink_descent - shift_up),
        )
        .max(
            post_bottom
                .as_ref()
                .map_or(0.0, |bottom| shift_down + bottom.ink_descent),
        )
        .max(
            slots
                .bottom_left
                .as_ref()
                .map_or(0.0, |bottom| shift_down + bottom.ink_descent),
        );
    let final_baseline = ink_ascent;
    let base_dy = final_baseline - baseline;
    let baseline = final_baseline;
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();

    if let Some(mut top_left) = slots.top_left {
        let dx = pre_width - space_after_script - top_left.metrics.width - pre_top_kern;
        let dy = baseline - shift_up - top_left.metrics.baseline;
        offset_atom(&mut top_left, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, top_left);
    }
    if let Some(mut bottom_left) = slots.bottom_left {
        let dx = pre_width - space_after_script - bottom_left.metrics.width - pre_bottom_kern;
        let dy = baseline + shift_down - bottom_left.metrics.baseline;
        offset_atom(&mut bottom_left, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, bottom_left);
    }
    let mut base = base;
    offset_atom(&mut base, pre_width, base_dy);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, base);

    if let Some(mut top) = post_top {
        let dx = pre_width + base_width + post_top_kern;
        let dy = baseline - shift_up - top.metrics.baseline;
        offset_atom(&mut top, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, top);
    }
    if let Some(mut bottom) = post_bottom {
        let dx = pre_width + base_width + post_bottom_kern;
        let dy = baseline + shift_down - bottom.metrics.baseline;
        offset_atom(&mut bottom, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, bottom);
    }

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height: ink_ascent + ink_descent,
            ascent: ink_ascent,
            baseline: ink_ascent,
            descent: ink_descent,
        },
        ink_ascent,
        ink_descent,
        left_spacing: None,
        right_spacing: None,
        left_class: base_left_class,
        right_class: base_right_class,
        italic_correction: 0.0,
        script_kernable: false,
        base_metrics: Some((ink_ascent, ink_descent)),
        accent_attachment: None,
        spaced: false,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn layout_simple_limit_attach_parts(
    font: &MathFont,
    font_size: f32,
    base: LaidOutMathAtom,
    mut slots: LaidOutAttachSlots,
    cramped: bool,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if slots.top.is_none() && slots.bottom.is_none() {
        return layout_simple_script_attach_parts(font, font_size, base, slots, cramped);
    }

    let top = slots.top.take();
    let bottom = slots.bottom.take();
    let Some(mut base) = layout_simple_script_attach_parts(font, font_size, base, slots, cramped)?
    else {
        return Ok(None);
    };
    let (upper_shift, lower_shift) =
        compute_limit_shifts(font, font_size, &base, top.as_ref(), bottom.as_ref())?;
    let width = base
        .metrics
        .width
        .max(top.as_ref().map_or(0.0, |top| top.metrics.width))
        .max(bottom.as_ref().map_or(0.0, |bottom| bottom.metrics.width));

    let baseline = base.metrics.baseline.max(
        top.as_ref()
            .map_or(0.0, |top| upper_shift + top.metrics.baseline),
    );
    let base_left_class = base.left_class;
    let base_right_class = base.right_class;
    let base_y = baseline - base.metrics.baseline;
    let base_x = (width - base.metrics.width) / 2.0;
    let mut height = base_y + base.metrics.height;
    let mut ink_ascent = base.ink_ascent;
    let mut ink_descent = base.ink_descent;

    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();

    if let Some(mut top) = top {
        let top_baseline = baseline - upper_shift;
        let top_y = top_baseline - top.metrics.baseline;
        let top_x = (width - top.metrics.width) / 2.0;
        height = height.max(top_y + top.metrics.height);
        ink_ascent = ink_ascent.max(baseline - top_y + top.ink_ascent - top.metrics.baseline);
        offset_atom(&mut top, top_x, top_y);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, top);
    }

    offset_atom(&mut base, base_x, base_y);
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, base);

    if let Some(mut bottom) = bottom {
        let bottom_baseline = baseline + lower_shift;
        let bottom_y = bottom_baseline - bottom.metrics.baseline;
        let bottom_x = (width - bottom.metrics.width) / 2.0;
        height = height.max(bottom_y + bottom.metrics.height);
        ink_descent =
            ink_descent.max(bottom_y + bottom.metrics.baseline + bottom.ink_descent - baseline);
        offset_atom(&mut bottom, bottom_x, bottom_y);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, bottom);
    }

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height,
            baseline,
            ascent: baseline,
            descent: height - baseline,
        },
        ink_ascent,
        ink_descent,
        left_spacing: None,
        right_spacing: None,
        left_class: base_left_class,
        right_class: base_right_class,
        italic_correction: 0.0,
        script_kernable: false,
        base_metrics: None,
        accent_attachment: None,
        spaced: false,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn combine_script_slots(
    font_size: f32,
    first: Option<LaidOutMathAtom>,
    second: Option<LaidOutMathAtom>,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    match (first, second) {
        (None, None) => Ok(None),
        (Some(atom), None) | (None, Some(atom)) => Ok(Some(atom)),
        (Some(mut first), Some(mut second)) => {
            let gap = SCRIPT_SLOT_PAIR_GAP_EM * font_size;
            let baseline = first.metrics.baseline.max(second.metrics.baseline);
            let first_dy = baseline - first.metrics.baseline;
            let second_x = first.metrics.width + gap;
            let second_dy = baseline - second.metrics.baseline;
            offset_atom(&mut first, 0.0, first_dy);
            offset_atom(&mut second, second_x, second_dy);

            let width = first.metrics.width + gap + second.metrics.width;
            let ascent = first.metrics.ascent.max(second.metrics.ascent);
            let descent = first.metrics.descent.max(second.metrics.descent);
            let ink_ascent = first.ink_ascent.max(second.ink_ascent);
            let ink_descent = first.ink_descent.max(second.ink_descent);

            let mut glyphs = Vec::new();
            let mut shapes = Vec::new();
            let mut draw_order = Vec::new();
            append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, first);
            append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, second);

            Ok(Some(LaidOutMathAtom {
                metrics: TypesetMetrics {
                    width,
                    height: ascent + descent,
                    baseline,
                    ascent,
                    descent,
                },
                ink_ascent,
                ink_descent,
                left_spacing: None,
                right_spacing: None,
                left_class: SimpleMathClass::Normal,
                right_class: SimpleMathClass::Normal,
                italic_correction: 0.0,
                script_kernable: false,
                base_metrics: None,
                accent_attachment: None,
                spaced: false,
                glyphs,
                shapes,
                draw_order,
            }))
        }
    }
}

fn layout_script_child(
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

fn compute_script_shifts(
    font: &MathFont,
    font_size: f32,
    base: &LaidOutMathAtom,
    top: Option<&LaidOutMathAtom>,
    bottom: Option<&LaidOutMathAtom>,
    cramped: bool,
) -> Result<(f32, f32), LabelError> {
    let sup_shift_up = math_constant(font, font_size, |constants| {
        if cramped {
            constants.superscript_shift_up_cramped().value
        } else {
            constants.superscript_shift_up().value
        }
    })?;
    let sup_bottom_min = math_constant(font, font_size, |constants| {
        constants.superscript_bottom_min().value
    })?;
    let sup_bottom_max_with_sub = math_constant(font, font_size, |constants| {
        constants.superscript_bottom_max_with_subscript().value
    })?;
    let gap_min = math_constant(font, font_size, |constants| {
        constants.sub_superscript_gap_min().value
    })?;
    let sub_shift_down = math_constant(font, font_size, |constants| {
        constants.subscript_shift_down().value
    })?;
    let sub_top_max = math_constant(font, font_size, |constants| {
        constants.subscript_top_max().value
    })?;

    let mut shift_up = 0.0;
    let mut shift_down = 0.0;
    if let Some(top) = top {
        shift_up = f32::max(sup_shift_up, sup_bottom_min + top.ink_descent);
    }
    if let Some(bottom) = bottom {
        shift_down = f32::max(sub_shift_down, bottom.ink_ascent - sub_top_max);
    }

    if let Some((ascent, descent)) = base.base_metrics {
        if top.is_some() {
            let drop = math_constant(font, font_size, |c| c.superscript_baseline_drop_max().value)?;
            shift_up = shift_up.max(ascent - drop);
        }
        if bottom.is_some() {
            let drop = math_constant(font, font_size, |c| c.subscript_baseline_drop_min().value)?;
            shift_down = shift_down.max(descent + drop);
        }
    }

    if let (Some(top), Some(bottom)) = (top, bottom) {
        let sup_bottom = shift_up - top.ink_descent;
        let sub_top = bottom.ink_ascent - shift_down;
        let gap = sup_bottom - sub_top;
        if gap < gap_min {
            let increase = gap_min - gap;
            let sup_only = (sup_bottom_max_with_sub - sup_bottom).clamp(0.0, increase);
            let rest = (increase - sup_only) / 2.0;
            shift_up += sup_only + rest;
            shift_down += rest;
        }
    }

    Ok((shift_up, shift_down))
}

fn compute_limit_shifts(
    font: &MathFont,
    font_size: f32,
    base: &LaidOutMathAtom,
    top: Option<&LaidOutMathAtom>,
    bottom: Option<&LaidOutMathAtom>,
) -> Result<(f32, f32), LabelError> {
    let upper_gap_min = math_constant(font, font_size, |constants| {
        constants.upper_limit_gap_min().value
    })?;
    let upper_rise_min = math_constant(font, font_size, |constants| {
        constants.upper_limit_baseline_rise_min().value
    })?;
    let lower_gap_min = math_constant(font, font_size, |constants| {
        constants.lower_limit_gap_min().value
    })?;
    let lower_drop_min = math_constant(font, font_size, |constants| {
        constants.lower_limit_baseline_drop_min().value
    })?;

    let upper_shift = top.map_or(0.0, |top| {
        base.ink_ascent + upper_rise_min.max(upper_gap_min + top.ink_descent)
    });
    let lower_shift = bottom.map_or(0.0, |bottom| {
        base.ink_descent + lower_drop_min.max(lower_gap_min + bottom.ink_ascent)
    });
    Ok((upper_shift, lower_shift))
}

#[derive(Debug, Clone, Copy)]
enum ScriptCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ScriptCorner {
    fn inverse(self) -> Self {
        match self {
            Self::TopLeft => Self::TopRight,
            Self::TopRight => Self::TopLeft,
            Self::BottomLeft => Self::BottomRight,
            Self::BottomRight => Self::BottomLeft,
        }
    }
}

fn math_kern(
    font: &MathFont,
    base: &LaidOutMathAtom,
    script: &LaidOutMathAtom,
    shift: f32,
    corner: ScriptCorner,
) -> Result<f32, LabelError> {
    if !base.script_kernable || !script.script_kernable {
        return Ok(0.0);
    }

    let (corr_height_top, corr_height_bot) = match corner {
        ScriptCorner::TopLeft | ScriptCorner::TopRight => {
            (base.ink_ascent - shift, shift - script.ink_descent)
        }
        ScriptCorner::BottomLeft | ScriptCorner::BottomRight => {
            (script.ink_ascent - shift, shift - base.ink_descent)
        }
    };

    let summed_kern = |height| {
        Ok(
            kern_at_height(font, edge_glyph(base, corner), corner, height)?
                + kern_at_height(
                    font,
                    edge_glyph(script, corner.inverse()),
                    corner.inverse(),
                    height,
                )?,
        )
    };
    Ok(f32::max(
        summed_kern(corr_height_top)?,
        summed_kern(corr_height_bot)?,
    ))
}

fn edge_glyph(atom: &LaidOutMathAtom, corner: ScriptCorner) -> Option<&LaidOutGlyph> {
    match corner {
        ScriptCorner::TopRight | ScriptCorner::BottomRight => atom.glyphs.last(),
        ScriptCorner::TopLeft | ScriptCorner::BottomLeft => atom.glyphs.first(),
    }
}

fn kern_at_height(
    font: &MathFont,
    glyph: Option<&LaidOutGlyph>,
    corner: ScriptCorner,
    height: f32,
) -> Result<f32, LabelError> {
    let Some(glyph) = glyph else {
        return Ok(0.0);
    };
    let face = parse_math_face(font, "math kern")?;
    let Some(kerns) = face
        .tables()
        .math
        .and_then(|math| math.glyph_info)
        .and_then(|glyph_info| glyph_info.kern_infos)
        .and_then(|kern_infos| kern_infos.get(glyph.glyph_id))
    else {
        return Ok(0.0);
    };
    let Some(kern) = (match corner {
        ScriptCorner::TopLeft => kerns.top_left,
        ScriptCorner::TopRight => kerns.top_right,
        ScriptCorner::BottomRight => kerns.bottom_right,
        ScriptCorner::BottomLeft => kerns.bottom_left,
    }) else {
        return Ok(0.0);
    };

    let units_per_em = face.units_per_em() as f32;
    let height_em = height / glyph.font_size;
    let mut index = 0;
    while index < kern.count()
        && height_em
            > kern
                .height(index)
                .map(|value| value.value as f32 / units_per_em)
                .unwrap_or_default()
    {
        index += 1;
    }
    Ok(kern
        .kern(index)
        .map(|value| value.value as f32 / units_per_em * glyph.font_size)
        .unwrap_or_default())
}

fn math_constant(
    font: &MathFont,
    font_size: f32,
    constant: impl FnOnce(ttf_parser::math::Constants<'_>) -> i16,
) -> Result<f32, LabelError> {
    let face = parse_math_face(font, "math constants")?;
    let value = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(constant)
        .unwrap_or_default();
    Ok(value as f32 * font_size / face.units_per_em() as f32)
}

fn math_unsigned_constant(
    font: &MathFont,
    font_size: f32,
    constant: impl FnOnce(ttf_parser::math::Constants<'_>) -> u16,
) -> Result<f32, LabelError> {
    let face = parse_math_face(font, "math unsigned constants")?;
    let value = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(constant)
        .unwrap_or_default();
    Ok(value as f32 * font_size / face.units_per_em() as f32)
}

fn math_percent(
    font: &MathFont,
    constant: impl FnOnce(ttf_parser::math::Constants<'_>) -> i16,
) -> Result<f32, LabelError> {
    let face = parse_math_face(font, "math percentage constant")?;
    let value = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(constant)
        .unwrap_or_default();
    Ok(value as f32 / 100.0)
}

fn font_cap_height(font: &MathFont, font_size: f32) -> Result<f32, LabelError> {
    let face = parse_math_face(font, "font cap height")?;
    Ok(face
        .capital_height()
        .unwrap_or_else(|| face.ascender())
        .max(0) as f32
        * font_size
        / face.units_per_em() as f32)
}
