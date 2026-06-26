use std::sync::Arc;

use crate::api::TypstEngineConfig;
use crate::error::MathTypesetError;
use crate::paths::{
    MathPathArtifact, MathPathCommand, MathPathData, MathPathItem, MathPathKind, MathStroke,
    MathTransform,
};
use crate::pdf::{
    MathFontResource, MathFontResourceId, MathPdfGlyph, MathPdfGlyphRun, MathPdfTextLayer,
};
#[cfg(feature = "raster")]
use crate::raster::rasterize_path_artifact;
use crate::style::MathFontSpec;
use crate::types::{MathFragmentOptions, MathRunArtifact, TypesetMetrics};

use super::ast::{
    OwnedMath, OwnedMathNode, OwnedMathOperator, OwnedMathShorthand, OwnedMathText,
    OwnedMathTextKind,
};

pub(crate) fn try_typeset_simple_row_fragment(
    math: &OwnedMath,
    options: &MathFragmentOptions,
    config: &TypstEngineConfig,
) -> Result<Option<MathRunArtifact>, MathTypesetError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }
    if !matches!(options.style.font, MathFontSpec::NewComputerModernMath) {
        return Ok(None);
    }
    if !config.font_config.extra_font_families.is_empty() {
        return Ok(None);
    }

    let Some(font) = load_default_math_font(config) else {
        return Ok(None);
    };
    let Some(layout) = layout_simple_row(&font, math, options.style.font_size.max(1.0))? else {
        return Ok(None);
    };
    let path_artifact = (options.outputs.paths || options.outputs.raster.is_some())
        .then(|| path_artifact_from_simple_row(&font, &layout, options.style.fill));
    #[cfg(feature = "raster")]
    let raster = options
        .outputs
        .raster
        .map(|request| {
            rasterize_path_artifact(
                path_artifact
                    .as_ref()
                    .expect("path artifact should be available for raster requests"),
                request,
            )
        })
        .transpose()?;
    #[cfg(not(feature = "raster"))]
    let raster = None;
    let paths = options.outputs.paths.then(|| {
        path_artifact
            .clone()
            .expect("path artifact should be available for path requests")
    });
    let (pdf_text, font_resources) = if options.outputs.pdf_text_layer {
        let artifact = pdf_text_from_simple_row(&font, &layout, &math.source, options.style.fill)?;
        (Some(artifact.text_layer), artifact.font_resources)
    } else {
        (None, Vec::new())
    };

    Ok(Some(MathRunArtifact {
        metrics: layout.metrics,
        paths,
        raster,
        pdf_text,
        font_resources,
        warnings: Vec::new(),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleMathAtom {
    styled_text: String,
    class: SimpleMathClass,
}

#[derive(Debug, Clone)]
struct SimpleRowLayout {
    metrics: TypesetMetrics,
    atoms: Vec<LaidOutMathAtom>,
}

#[derive(Debug, Clone)]
struct LaidOutMathAtom {
    metrics: TypesetMetrics,
    ink_ascent: f32,
    ink_descent: f32,
    class: SimpleMathClass,
    italic_correction: f32,
    glyphs: Vec<LaidOutGlyph>,
    shapes: Vec<LaidOutShape>,
    draw_order: Vec<LaidOutDrawItem>,
}

#[derive(Debug, Clone)]
struct LaidOutGlyph {
    glyph_id: ttf_parser::GlyphId,
    unicode: String,
    x: f32,
    y: f32,
    x_advance: f32,
    font_size: f32,
}

#[derive(Debug, Clone)]
struct LaidOutShape {
    path: MathPathData,
    x: f32,
    y: f32,
    stroke_width: f32,
}

#[derive(Debug, Clone)]
enum LaidOutDrawItem {
    Glyph(usize),
    Shape(usize),
}

struct OwnedPdfArtifact {
    text_layer: MathPdfTextLayer,
    font_resources: Vec<MathFontResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleMathClass {
    Normal,
    Alphabetic,
    Binary,
    Vary,
    Relation,
    Opening,
    Closing,
    Fence,
    Punctuation,
    Large,
}

fn layout_simple_row(
    font: &OwnedMathFont,
    math: &OwnedMath,
    font_size: f32,
) -> Result<Option<SimpleRowLayout>, MathTypesetError> {
    let Some(atom) = layout_simple_nodes_as_atom(font, &math.nodes, font_size, 0)? else {
        return Ok(None);
    };
    Ok(Some(SimpleRowLayout {
        metrics: atom.metrics,
        atoms: vec![atom],
    }))
}

fn layout_simple_nodes_as_atom(
    font: &OwnedMathFont,
    nodes: &[OwnedMathNode],
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let mut atoms = Vec::new();
    for node in nodes {
        match node {
            OwnedMathNode::Space(_) => {}
            _ => {
                let Some(atom) = layout_simple_node(font, node, font_size, script_level)? else {
                    return Ok(None);
                };
                atoms.push(atom);
            }
        }
    }

    if atoms.is_empty() {
        return Ok(None);
    }

    let mut metrics = TypesetMetrics {
        width: 0.0,
        height: 0.0,
        baseline: 0.0,
        ascent: 0.0,
        descent: 0.0,
    };
    let mut laid_out_atoms = Vec::with_capacity(atoms.len());
    let mut previous = None;

    for mut atom in atoms {
        let class = resolved_left_class(previous, atom.class);
        if let Some(previous) = previous {
            metrics.width += math_spacing_for_level(previous, class, font_size, script_level);
        }
        atom.class = class;
        offset_atom(&mut atom, metrics.width, 0.0);
        metrics.width += atom.metrics.width;
        metrics.ascent = metrics.ascent.max(atom.metrics.ascent);
        metrics.descent = metrics.descent.max(atom.metrics.descent);
        metrics.height = metrics.ascent + metrics.descent;
        metrics.baseline = metrics.ascent;
        laid_out_atoms.push(atom);
        previous = Some(class);
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
        class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        glyphs,
        shapes,
        draw_order,
    }))
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
    draw_order.extend(atom.draw_order.iter().map(|item| match *item {
        LaidOutDrawItem::Glyph(index) => LaidOutDrawItem::Glyph(glyph_offset + index),
        LaidOutDrawItem::Shape(index) => LaidOutDrawItem::Shape(shape_offset + index),
    }));
    glyphs.append(&mut atom.glyphs);
    shapes.append(&mut atom.shapes);
}

fn layout_simple_node(
    font: &OwnedMathFont,
    node: &OwnedMathNode,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    if let Some(atom) = simple_atom(node) {
        let layout = layout_styled_atom_with_class(
            font,
            &atom.styled_text,
            font_size,
            script_style_feature(script_level),
            atom.class,
        )?;
        return Ok(Some(layout));
    }

    if let OwnedMathNode::Attach(attach) = node {
        return layout_simple_attach(font, attach, font_size, script_level);
    }

    if let OwnedMathNode::Fraction(fraction) = node {
        return layout_simple_fraction(font, fraction, font_size, script_level);
    }

    if let OwnedMathNode::Call(call) = node {
        if call.name == "sqrt" {
            return layout_simple_sqrt(font, call, font_size, script_level);
        }
        if call.name == "root" {
            return layout_simple_root(font, call, font_size, script_level);
        }
    }

    Ok(None)
}

fn layout_simple_sqrt(
    font: &OwnedMathFont,
    call: &super::ast::OwnedMathCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_radical(font, &arg.nodes, None, font_size, script_level)
}

fn layout_simple_root(
    font: &OwnedMathFont,
    call: &super::ast::OwnedMathCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let [index, radicand] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_radical(
        font,
        &radicand.nodes,
        Some(index.nodes.as_slice()),
        font_size,
        script_level,
    )
}

fn layout_simple_radical(
    font: &OwnedMathFont,
    radicand_nodes: &[OwnedMathNode],
    index_nodes: Option<&[OwnedMathNode]>,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let Some(mut radicand) =
        layout_simple_nodes_as_atom(font, radicand_nodes, font_size, script_level)?
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
        constants.radical_vertical_gap().value
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
        path: MathPathData {
            commands: vec![
                MathPathCommand::MoveTo { x: 0.0, y: 0.0 },
                MathPathCommand::LineTo {
                    x: line_width,
                    y: 0.0,
                },
            ],
        },
        x: radicand_x,
        y: line_y,
        stroke_width: thickness,
    });
    draw_order.push(LaidOutDrawItem::Shape(shapes.len() - 1));
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, radicand);

    finalize_inline_frame_atom(
        width, height, ascent, glyphs, shapes, draw_order, font, font_size,
    )
}

fn layout_simple_fraction(
    font: &OwnedMathFont,
    fraction: &super::ast::OwnedMathFraction,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let child_font_size = script_font_size(font, font_size, script_level)?;
    let Some(mut numerator) =
        layout_fraction_child(font, &fraction.numerator, child_font_size, script_level + 1)?
    else {
        return Ok(None);
    };
    let Some(mut denominator) = layout_fraction_child(
        font,
        &fraction.denominator,
        child_font_size,
        script_level + 1,
    )?
    else {
        return Ok(None);
    };

    let axis = math_constant(font, font_size, |constants| constants.axis_height().value)?;
    let thickness = math_constant(font, font_size, |constants| {
        constants.fraction_rule_thickness().value
    })?;
    let shift_up = math_constant(font, font_size, |constants| {
        constants.fraction_numerator_shift_up().value
    })?;
    let shift_down = math_constant(font, font_size, |constants| {
        constants.fraction_denominator_shift_down().value
    })?;
    let numerator_gap_min = math_constant(font, font_size, |constants| {
        constants.fraction_numerator_gap_min().value
    })?;
    let denominator_gap_min = math_constant(font, font_size, |constants| {
        constants.fraction_denominator_gap_min().value
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
    shapes.push(LaidOutShape {
        path: MathPathData {
            commands: vec![
                MathPathCommand::MoveTo { x: 0.0, y: 0.0 },
                MathPathCommand::LineTo {
                    x: line_width,
                    y: 0.0,
                },
            ],
        },
        x: line_x,
        y: line_y,
        stroke_width: thickness,
    });
    draw_order.push(LaidOutDrawItem::Shape(shapes.len() - 1));

    finalize_inline_frame_atom(
        width, height, baseline, glyphs, shapes, draw_order, font, font_size,
    )
}

fn finalize_inline_frame_atom(
    width: f32,
    height: f32,
    baseline: f32,
    mut glyphs: Vec<LaidOutGlyph>,
    mut shapes: Vec<LaidOutShape>,
    draw_order: Vec<LaidOutDrawItem>,
    font: &OwnedMathFont,
    font_size: f32,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    let final_ascent =
        font_cap_height(font, font_size)?.max(baseline - INLINE_MATH_LEADING_SLACK_EM * font_size);
    let final_descent = (height - baseline - INLINE_MATH_LEADING_SLACK_EM * font_size).max(0.0);
    let final_dy = final_ascent - baseline;
    for glyph in &mut glyphs {
        glyph.y += final_dy;
    }
    for shape in &mut shapes {
        shape.y += final_dy;
    }

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            height: final_ascent + final_descent,
            baseline: final_ascent,
            ascent: final_ascent,
            descent: final_descent,
        },
        ink_ascent: baseline,
        ink_descent: height - baseline,
        class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn layout_fraction_child(
    font: &OwnedMathFont,
    node: &OwnedMathNode,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    if let OwnedMathNode::Group(group) = node {
        return layout_simple_nodes_as_atom(font, &group.body, font_size, script_level);
    }
    layout_simple_node(font, node, font_size, script_level)
}

const FRACTION_PADDING_EM: f32 = 0.1;
const INLINE_MATH_LEADING_SLACK_EM: f32 = 0.65 * 0.7;

fn layout_simple_attach(
    font: &OwnedMathFont,
    attach: &super::ast::OwnedMathAttach,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, MathTypesetError> {
    if attach.primes > 0 {
        return Ok(None);
    }

    let Some(base) = layout_simple_node(font, &attach.base, font_size, script_level)? else {
        return Ok(None);
    };
    let script_font_size = script_font_size(font, font_size, script_level)?;
    let top = attach
        .top
        .as_deref()
        .map(|node| layout_simple_node(font, node, script_font_size, script_level + 1))
        .transpose()?
        .flatten();
    let bottom = attach
        .bottom
        .as_deref()
        .map(|node| layout_simple_node(font, node, script_font_size, script_level + 1))
        .transpose()?
        .flatten();
    if (attach.top.is_some() && top.is_none()) || (attach.bottom.is_some() && bottom.is_none()) {
        return Ok(None);
    }

    let (shift_up, shift_down) =
        compute_script_shifts(font, font_size, &base, top.as_ref(), bottom.as_ref())?;
    let space_after_script = math_constant(font, font_size, |constants| {
        constants.space_after_script().value
    })?;
    let top_kern = top
        .as_ref()
        .map(|top| math_kern(font, &base, top, shift_up, ScriptCorner::TopRight))
        .transpose()?
        .unwrap_or_default();
    let bottom_kern = bottom
        .as_ref()
        .map(|bottom| {
            math_kern(font, &base, bottom, shift_down, ScriptCorner::BottomRight)
                .map(|kern| kern - base.italic_correction)
        })
        .transpose()?
        .unwrap_or_default();

    let top_post_width = top
        .as_ref()
        .map(|top| space_after_script + top.metrics.width + top_kern)
        .unwrap_or_default();
    let bottom_post_width = bottom
        .as_ref()
        .map(|bottom| space_after_script + bottom.metrics.width + bottom_kern)
        .unwrap_or_default();
    let base_width = base.metrics.width;
    let base_metrics = base.metrics;
    let base_class = base.class;
    let base_italic_correction = base.italic_correction;
    let width = base_width + top_post_width.max(bottom_post_width);
    let baseline = base.metrics.baseline;
    let ink_ascent = base
        .ink_ascent
        .max(top.as_ref().map_or(0.0, |top| shift_up + top.ink_ascent))
        .max(
            bottom
                .as_ref()
                .map_or(0.0, |bottom| bottom.ink_ascent - shift_down),
        );
    let ink_descent = base
        .ink_descent
        .max(top.as_ref().map_or(0.0, |top| top.ink_descent - shift_up))
        .max(
            bottom
                .as_ref()
                .map_or(0.0, |bottom| shift_down + bottom.ink_descent),
        );
    let mut glyphs = Vec::new();
    let mut shapes = Vec::new();
    let mut draw_order = Vec::new();
    append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, base);

    if let Some(mut top) = top {
        let dx = base_width + top_kern;
        let dy = baseline - shift_up - top.metrics.baseline;
        offset_atom(&mut top, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, top);
    }
    if let Some(mut bottom) = bottom {
        let dx = base_width + bottom_kern;
        let dy = baseline + shift_down - bottom.metrics.baseline;
        offset_atom(&mut bottom, dx, dy);
        append_atom_items(&mut glyphs, &mut shapes, &mut draw_order, bottom);
    }

    Ok(Some(LaidOutMathAtom {
        metrics: TypesetMetrics {
            width,
            ..base_metrics
        },
        ink_ascent,
        ink_descent,
        class: base_class,
        italic_correction: base_italic_correction,
        glyphs,
        shapes,
        draw_order,
    }))
}

fn script_font_size(
    font: &OwnedMathFont,
    font_size: f32,
    script_level: u8,
) -> Result<f32, MathTypesetError> {
    let face = parse_math_face(font, "math script constants")?;
    let (script_percent, script_script_percent) = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(|constants| {
            (
                constants.script_percent_scale_down(),
                constants.script_script_percent_scale_down(),
            )
        })
        .unwrap_or((70, 50));
    let script_percent = script_percent.max(1) as f32;
    let script_script_percent = script_script_percent.max(1) as f32;
    if script_level == 0 {
        Ok(font_size * script_percent / 100.0)
    } else {
        Ok(font_size * script_script_percent / script_percent)
    }
}

fn compute_script_shifts(
    font: &OwnedMathFont,
    font_size: f32,
    base: &LaidOutMathAtom,
    top: Option<&LaidOutMathAtom>,
    bottom: Option<&LaidOutMathAtom>,
) -> Result<(f32, f32), MathTypesetError> {
    let sup_shift_up = math_constant(font, font_size, |constants| {
        constants.superscript_shift_up().value
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

    // Text-like bases intentionally do not apply Typst's base ascent/descent
    // drop rules. Keep `base` in the signature because non-text-like boxes will
    // need those fields when fractions and radicals become owned.
    let _ = base;

    Ok((shift_up, shift_down))
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
    font: &OwnedMathFont,
    base: &LaidOutMathAtom,
    script: &LaidOutMathAtom,
    shift: f32,
    corner: ScriptCorner,
) -> Result<f32, MathTypesetError> {
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
    font: &OwnedMathFont,
    glyph: Option<&LaidOutGlyph>,
    corner: ScriptCorner,
    height: f32,
) -> Result<f32, MathTypesetError> {
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
    font: &OwnedMathFont,
    font_size: f32,
    constant: impl FnOnce(ttf_parser::math::Constants<'_>) -> i16,
) -> Result<f32, MathTypesetError> {
    let face = parse_math_face(font, "math constants")?;
    let value = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(constant)
        .unwrap_or_default();
    Ok(value as f32 * font_size / face.units_per_em() as f32)
}

fn math_percent(
    font: &OwnedMathFont,
    constant: impl FnOnce(ttf_parser::math::Constants<'_>) -> i16,
) -> Result<f32, MathTypesetError> {
    let face = parse_math_face(font, "math percentage constant")?;
    let value = face
        .tables()
        .math
        .and_then(|math| math.constants)
        .map(constant)
        .unwrap_or_default();
    Ok(value as f32 / 100.0)
}

fn font_cap_height(font: &OwnedMathFont, font_size: f32) -> Result<f32, MathTypesetError> {
    let face = parse_math_face(font, "font cap height")?;
    Ok(face
        .capital_height()
        .unwrap_or_else(|| face.ascender())
        .max(0) as f32
        * font_size
        / face.units_per_em() as f32)
}

#[cfg(test)]
fn single_atom_text(math: &OwnedMath) -> Option<String> {
    let [node] = &math.nodes[..] else {
        return None;
    };
    simple_atom(node).map(|atom| atom.styled_text)
}

fn simple_atom(node: &OwnedMathNode) -> Option<SimpleMathAtom> {
    match node {
        OwnedMathNode::Text(text) => Some(SimpleMathAtom {
            styled_text: style_text_atom(text),
            class: match text.kind {
                OwnedMathTextKind::Grapheme => SimpleMathClass::Alphabetic,
                OwnedMathTextKind::Number => SimpleMathClass::Normal,
            },
        }),
        OwnedMathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            if identifier.symbol.is_some() || text.chars().count() == 1 {
                Some(SimpleMathAtom {
                    styled_text: style_default_math_text(text),
                    class: identifier_class(text),
                })
            } else {
                None
            }
        }
        OwnedMathNode::Operator(operator) => Some(SimpleMathAtom {
            styled_text: operator_text(operator),
            class: operator_class(&operator.operator),
        }),
        OwnedMathNode::Shorthand(shorthand) => Some(SimpleMathAtom {
            styled_text: shorthand_text(shorthand),
            class: symbol_class(shorthand.replacement),
        }),
        _ => None,
    }
}

fn style_text_atom(text: &OwnedMathText) -> String {
    match text.kind {
        OwnedMathTextKind::Grapheme => style_default_math_text(&text.text),
        OwnedMathTextKind::Number => text.text.clone(),
    }
}

fn style_default_math_text(text: &str) -> String {
    text.chars().map(style_default_math_char).collect()
}

fn operator_text(operator: &OwnedMathOperator) -> String {
    operator.operator.clone()
}

fn shorthand_text(shorthand: &OwnedMathShorthand) -> String {
    shorthand.replacement.to_string()
}

fn identifier_class(text: &str) -> SimpleMathClass {
    if matches!(text, "∑" | "∏" | "∫") {
        SimpleMathClass::Large
    } else {
        SimpleMathClass::Alphabetic
    }
}

fn operator_class(text: &str) -> SimpleMathClass {
    match text {
        "=" | "<" | ">" | ":" => SimpleMathClass::Relation,
        "," => SimpleMathClass::Punctuation,
        "(" | "[" | "{" => SimpleMathClass::Opening,
        ")" | "]" | "}" => SimpleMathClass::Closing,
        "|" => SimpleMathClass::Fence,
        "+" | "-" | "*" | "!" | "&" => SimpleMathClass::Vary,
        _ => SimpleMathClass::Normal,
    }
}

fn symbol_class(text: &str) -> SimpleMathClass {
    match text {
        "≤" | "≥" | "≠" | "⇒" | "→" | "←" | "≔" => SimpleMathClass::Relation,
        "∑" | "∏" | "∫" => SimpleMathClass::Large,
        _ => SimpleMathClass::Normal,
    }
}

fn resolved_left_class(
    previous: Option<SimpleMathClass>,
    class: SimpleMathClass,
) -> SimpleMathClass {
    if class == SimpleMathClass::Vary
        && previous.is_some_and(|prev| {
            matches!(
                prev,
                SimpleMathClass::Normal
                    | SimpleMathClass::Alphabetic
                    | SimpleMathClass::Closing
                    | SimpleMathClass::Fence
            )
        })
    {
        SimpleMathClass::Binary
    } else {
        class
    }
}

#[cfg(test)]
fn math_spacing(left: SimpleMathClass, right: SimpleMathClass, font_size: f32) -> f32 {
    math_spacing_for_level(left, right, font_size, 0)
}

fn math_spacing_for_level(
    left: SimpleMathClass,
    right: SimpleMathClass,
    font_size: f32,
    script_level: u8,
) -> f32 {
    if script_level > 0 {
        return 0.0;
    }

    use SimpleMathClass::*;

    match (left, right) {
        (_, Punctuation) => 0.0,
        (Punctuation, _) => THIN_EM * font_size,
        (Opening, _) | (_, Closing) => 0.0,
        (Relation, Relation) => 0.0,
        (Relation, _) | (_, Relation) => THICK_EM * font_size,
        (Binary, _) | (_, Binary) => MEDIUM_EM * font_size,
        (Large, Opening | Fence) => 0.0,
        (Large, _) | (_, Large) => THIN_EM * font_size,
        _ => 0.0,
    }
}

const THIN_EM: f32 = 1.0 / 6.0;
const MEDIUM_EM: f32 = 2.0 / 9.0;
const THICK_EM: f32 = 5.0 / 18.0;

fn style_default_math_char(ch: char) -> char {
    if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) || matches!(ch, 'ı' | 'ȷ' | 'ħ')
    {
        to_math_italic(ch)
    } else {
        ch
    }
}

fn is_lower_greek_math_char(ch: char) -> bool {
    matches!(ch, 'α'..='ω' | '∂' | 'ϵ' | 'ϑ' | 'ϰ' | 'ϕ' | 'ϱ' | 'ϖ')
}

fn to_math_italic(ch: char) -> char {
    let delta = match ch {
        'h' => 0x20A6,
        'ħ' => 0x1FE8,
        'A'..='Z' => 0x1D3F3,
        'a'..='z' => 0x1D3ED,
        'ı' => 0x1D573,
        'ȷ' => 0x1D46E,
        'Α'..='Ρ' => 0x1D351,
        'ϴ' => 0x1D2FF,
        'Σ'..='Ω' => 0x1D351,
        '∇' => 0x1B4F4,
        'α'..='ω' => 0x1D34B,
        '∂' => 0x1B513,
        'ϵ' => 0x1D321,
        'ϑ' => 0x1D346,
        'ϰ' => 0x1D328,
        'ϕ' => 0x1D344,
        'ϱ' => 0x1D329,
        'ϖ' => 0x1D345,
        _ => return ch,
    };
    std::char::from_u32((ch as u32) + delta).unwrap_or(ch)
}

struct OwnedMathFont {
    data: Vec<u8>,
    face_index: u32,
}

fn load_default_math_font(config: &TypstEngineConfig) -> Option<OwnedMathFont> {
    for path in crate::fonts::candidate_math_font_paths(config) {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        let face_count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
        for face_index in 0..face_count {
            let Ok(face) = ttf_parser::Face::parse(&data, face_index) else {
                continue;
            };
            if face.tables().math.is_some() {
                return Some(OwnedMathFont { data, face_index });
            }
        }
    }
    None
}

fn parse_math_face<'a>(
    font: &'a OwnedMathFont,
    context: &str,
) -> Result<ttf_parser::Face<'a>, MathTypesetError> {
    ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| MathTypesetError::Engine {
        start: 0,
        end: 0,
        message: format!("failed to parse owned math font for {context}"),
    })
}

fn layout_styled_atom_with_class(
    font: &OwnedMathFont,
    text: &str,
    font_size: f32,
    script_style: Option<u32>,
    class: SimpleMathClass,
) -> Result<LaidOutMathAtom, MathTypesetError> {
    let face = ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| {
        MathTypesetError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to parse owned math font".to_string(),
        }
    })?;
    let Some(rusty) = rustybuzz::Face::from_slice(&font.data, font.face_index) else {
        return Err(MathTypesetError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to shape owned math font".to_string(),
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
        class,
        italic_correction: atom_italic_correction,
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

fn script_style_feature(script_level: u8) -> Option<u32> {
    (script_level > 0).then_some(u32::from(script_level.min(2)))
}

fn pdf_text_from_simple_row(
    font: &OwnedMathFont,
    layout: &SimpleRowLayout,
    source: &str,
    fill: crate::style::Color,
) -> Result<OwnedPdfArtifact, MathTypesetError> {
    let face = ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| {
        MathTypesetError::Engine {
            start: 0,
            end: source.len(),
            message: "failed to parse owned math font for PDF glyph output".to_string(),
        }
    })?;
    let font_id = MathFontResourceId(0);
    let font_resources = vec![MathFontResource {
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
        for glyph in &atom.glyphs {
            glyph_runs.push(MathPdfGlyphRun {
                font: font_id,
                font_size: glyph.font_size,
                fill,
                stroke: None,
                glyphs: vec![MathPdfGlyph {
                    glyph_id: glyph.glyph_id.0,
                    unicode: glyph.unicode.clone(),
                    x: 0.0,
                    y: 0.0,
                    x_advance: glyph.x_advance,
                    y_advance: 0.0,
                    transform: MathTransform {
                        dx: glyph.x,
                        dy: glyph.y,
                        ..MathTransform::IDENTITY
                    },
                }],
            });
        }
    }

    Ok(OwnedPdfArtifact {
        text_layer: MathPdfTextLayer {
            logical_width: layout.metrics.width,
            logical_height: layout.metrics.height,
            semantic_text: source.to_string(),
            glyph_runs,
        },
        font_resources,
    })
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

fn path_artifact_from_simple_row(
    font: &OwnedMathFont,
    layout: &SimpleRowLayout,
    fill: crate::style::Color,
) -> MathPathArtifact {
    let Ok(face) = ttf_parser::Face::parse(&font.data, font.face_index) else {
        return MathPathArtifact {
            logical_width: layout.metrics.width,
            logical_height: layout.metrics.height,
            items: Vec::new(),
        };
    };
    let mut items = Vec::new();
    let mut glyph_run = 0usize;

    for atom in &layout.atoms {
        for item in &atom.draw_order {
            match *item {
                LaidOutDrawItem::Glyph(index) => {
                    let Some(glyph) = atom.glyphs.get(index) else {
                        continue;
                    };
                    let mut builder = OwnedGlyphPathBuilder {
                        path: MathPathData::default(),
                        scale: glyph.font_size / face.units_per_em() as f32,
                        x_offset: glyph.x,
                        y_offset: glyph.y,
                    };
                    face.outline_glyph(glyph.glyph_id, &mut builder);
                    if !builder.path.commands.is_empty() {
                        items.push(MathPathItem {
                            path: builder.path,
                            kind: MathPathKind::GlyphOutline {
                                glyph_run,
                                glyph_index: 0,
                            },
                            fill: Some(fill),
                            stroke: None,
                            transform: MathTransform::IDENTITY,
                            clip: None,
                        });
                    }
                    glyph_run += 1;
                }
                LaidOutDrawItem::Shape(index) => {
                    let Some(shape) = atom.shapes.get(index) else {
                        continue;
                    };
                    items.push(MathPathItem {
                        path: shape.path.clone(),
                        kind: MathPathKind::MathShape,
                        fill: None,
                        stroke: Some(MathStroke {
                            color: fill,
                            width: shape.stroke_width,
                        }),
                        transform: MathTransform {
                            dx: shape.x,
                            dy: shape.y,
                            ..MathTransform::IDENTITY
                        },
                        clip: None,
                    });
                }
            }
        }
    }

    MathPathArtifact {
        logical_width: layout.metrics.width,
        logical_height: layout.metrics.height,
        items,
    }
}

struct OwnedGlyphPathBuilder {
    path: MathPathData,
    scale: f32,
    x_offset: f32,
    y_offset: f32,
}

impl OwnedGlyphPathBuilder {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.x_offset + x * self.scale,
            self.y_offset - y * self.scale,
        )
    }
}

impl ttf_parser::OutlineBuilder for OwnedGlyphPathBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::MoveTo { x, y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::LineTo { x, y });
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        self.path
            .commands
            .push(MathPathCommand::QuadTo { x1, y1, x, y });
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x1, y1) = self.point(x1, y1);
        let (x2, y2) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.path.commands.push(MathPathCommand::CubicTo {
            x1,
            y1,
            x2,
            y2,
            x,
            y,
        });
    }

    fn close(&mut self) {
        self.path.commands.push(MathPathCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::owned::math::syntax::parse_owned_math;
    use crate::types::MathOutputRequest;

    #[test]
    #[cfg(not(feature = "raster"))]
    fn atom_fragment_declines_raster_without_raster_feature() {
        let math = parse_owned_math("1", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: false,
            raster: Some(crate::raster::RasterRequest::default()),
            pdf_text_layer: false,
        };

        assert!(
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn atom_fragment_can_emit_pdf_glyph_metadata() {
        let math = parse_owned_math("1", 0).unwrap();
        let mut options = MathFragmentOptions::default();

        options.outputs = MathOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        assert!(
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .is_some_and(|artifact| artifact
                    .pdf_text
                    .as_ref()
                    .is_some_and(|pdf| pdf.glyph_runs.len() == 1)
                    && artifact.font_resources.len() == 1)
        );
    }

    #[cfg(feature = "raster")]
    #[test]
    fn atom_fragment_can_rasterize_from_owned_paths() {
        let math = parse_owned_math("alpha + beta -> gamma", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: false,
            raster: Some(crate::raster::RasterRequest { scale: 2.0 }),
            pdf_text_layer: false,
        };

        let artifact =
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .expect("simple row should rasterize through owned paths");

        assert!(artifact.paths.is_none());
        assert!(artifact
            .raster
            .as_ref()
            .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0));
    }

    #[test]
    fn simple_row_can_emit_script_glyph_metadata() {
        let math = parse_owned_math("x^2", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact =
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .expect("simple superscript should be handled by owned row path");
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 2);
        assert!(pdf.glyph_runs[1].font_size < pdf.glyph_runs[0].font_size);
    }

    #[test]
    fn simple_row_can_emit_fraction_rule_paths() {
        let math = parse_owned_math("a / (b + c)", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact =
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .expect("simple fraction should be handled by owned row path");
        let paths = artifact.paths.expect("fraction paths should exist");
        assert!(paths
            .items
            .iter()
            .any(|item| matches!(item.kind, MathPathKind::MathShape) && item.stroke.is_some()));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_row_can_emit_sqrt_overbar_paths() {
        let math = parse_owned_math("sqrt(x)", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact =
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .expect("simple sqrt should be handled by owned row path");
        let paths = artifact.paths.expect("sqrt paths should exist");
        assert_eq!(paths.items.len(), 3);
        assert!(matches!(paths.items[1].kind, MathPathKind::MathShape));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 2);
    }

    #[test]
    fn simple_row_can_emit_indexed_root_paths() {
        let math = parse_owned_math("root(3, x)", 0).unwrap();
        let mut options = MathFragmentOptions::default();
        options.outputs = MathOutputRequest {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact =
            try_typeset_simple_row_fragment(&math, &options, &TypstEngineConfig::default())
                .unwrap()
                .expect("simple indexed root should be handled by owned row path");
        let paths = artifact.paths.expect("root paths should exist");
        assert_eq!(paths.items.len(), 4);
        assert!(matches!(paths.items[2].kind, MathPathKind::MathShape));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 3);
        assert!(pdf.glyph_runs[0].font_size < pdf.glyph_runs[1].font_size);
    }

    #[test]
    fn atom_fragment_styles_latin_and_greek_as_math_italic() {
        let x = parse_owned_math("x", 0).unwrap();
        let alpha = parse_owned_math("alpha", 0).unwrap();

        assert_eq!(single_atom_text(&x).as_deref(), Some("𝑥"));
        assert_eq!(single_atom_text(&alpha).as_deref(), Some("𝛼"));
    }

    #[test]
    fn atom_fragment_keeps_numbers_and_operators_plain() {
        let number = parse_owned_math("0.94", 0).unwrap();
        let plus = parse_owned_math("+", 0).unwrap();
        let arrow = parse_owned_math("->", 0).unwrap();

        assert_eq!(single_atom_text(&number).as_deref(), Some("0.94"));
        assert_eq!(single_atom_text(&plus).as_deref(), Some("+"));
        assert_eq!(single_atom_text(&arrow).as_deref(), Some("→"));
    }

    #[test]
    fn simple_row_inserts_binary_and_relation_spacing() {
        let font_size = 12.0;

        assert_eq!(
            math_spacing(
                SimpleMathClass::Alphabetic,
                SimpleMathClass::Binary,
                font_size
            ),
            MEDIUM_EM * font_size
        );
        assert_eq!(
            math_spacing(
                SimpleMathClass::Relation,
                SimpleMathClass::Alphabetic,
                font_size
            ),
            THICK_EM * font_size
        );
    }
}
