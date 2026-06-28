use std::sync::Arc;

use crate::engine::ast::DecorationStroke;
use crate::engine::glyph_path::outline_glyph_path;
use crate::error::LabelError;
use crate::label::EngineOptions;
use crate::types::{MathLayoutOptions, MathRunArtifact, TypesetMetrics};
use crate::typst_library::{Color, FontWeight, MathFontSpec};
use crate::typst_pdf::{FontResource, FontResourceId, PdfGlyph, PdfGlyphRun, PdfTextLayer};
#[cfg(feature = "raster")]
use crate::typst_render::rasterize_path_artifact;
use crate::typst_svg::{
    PathArtifact, PathCommand, PathData, PathItem, PathKind, Stroke, Transform,
};

use super::ast::{
    MathAccent, MathAst, MathCancel, MathCancelAngle, MathFractionStyle, MathNode, MathOperator,
    MathShorthand, MathText, MathTextKind,
};
use super::syntax::predefined_operator_text;

pub(crate) fn try_typeset_simple_row_fragment(
    math: &MathAst,
    options: &MathLayoutOptions,
    config: &EngineOptions,
) -> Result<Option<MathRunArtifact>, LabelError> {
    #[cfg(not(feature = "raster"))]
    if options.outputs.raster.is_some() {
        return Ok(None);
    }
    if !matches!(
        options.style.font,
        MathFontSpec::LeteSansMath | MathFontSpec::NewComputerModernMath
    ) {
        return Ok(None);
    }
    if !config.fonts.extra_font_families.is_empty() {
        return Ok(None);
    }

    let Some(font) =
        load_default_math_font(config, &options.style.font, &options.style.font_weight)
    else {
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
    text_operator: bool,
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
    left_class: SimpleMathClass,
    right_class: SimpleMathClass,
    italic_correction: f32,
    script_kernable: bool,
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
    pdf_run_group: Option<usize>,
}

#[derive(Debug, Clone)]
struct LaidOutShape {
    path: PathData,
    x: f32,
    y: f32,
    stroke: LaidOutStroke,
}

#[derive(Debug, Clone)]
struct LaidOutStroke {
    paint: Option<Color>,
    width: f32,
    line_cap: crate::typst_svg::StrokeCap,
    line_join: crate::typst_svg::StrokeJoin,
    dash: Option<Vec<f32>>,
}

impl LaidOutStroke {
    fn new(width: f32) -> Self {
        Self {
            paint: None,
            width,
            line_cap: crate::typst_svg::StrokeCap::Butt,
            line_join: crate::typst_svg::StrokeJoin::Miter,
            dash: None,
        }
    }

    fn from_decoration(stroke: &DecorationStroke, default_width: f32, font_size: f32) -> Self {
        let width = stroke
            .thickness
            .map(|thickness| thickness.resolve(font_size))
            .unwrap_or(default_width);
        Self {
            paint: stroke.paint,
            width,
            line_cap: stroke.line_cap.unwrap_or_default(),
            line_join: stroke.line_join.unwrap_or_default(),
            dash: stroke
                .dash
                .as_ref()
                .and_then(|dash| dash.resolve(width, font_size)),
        }
    }
}

#[derive(Debug, Clone)]
enum LaidOutDrawItem {
    Glyph(usize),
    Shape(usize),
}

struct PdfArtifact {
    text_layer: PdfTextLayer,
    font_resources: Vec<FontResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimpleMathClass {
    Normal,
    Alphabetic,
    Binary,
    Unary,
    Vary,
    Relation,
    Opening,
    Closing,
    Fence,
    Punctuation,
    Large,
}

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
    let mut atoms = Vec::new();
    let mut index = 0usize;
    while index < nodes.len() {
        let node = &nodes[index];
        match node {
            MathNode::Space(_) => {}
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
                        )?
                        else {
                            return Ok(None);
                        };
                        (atom, 1)
                    };
                atoms.push(atom);
                index += consumed;
                continue;
            }
        }
        index += 1;
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
        let left_class = resolved_left_class(previous, atom.left_class);
        if let Some(previous) = previous {
            metrics.width += math_spacing_for_level(previous, left_class, font_size, script_level);
        }
        if atom.left_class == atom.right_class {
            atom.right_class = left_class;
        }
        atom.left_class = left_class;
        offset_atom(&mut atom, metrics.width, 0.0);
        metrics.width += atom.metrics.width;
        metrics.ascent = metrics.ascent.max(atom.metrics.ascent);
        metrics.descent = metrics.descent.max(atom.metrics.descent);
        metrics.height = metrics.ascent + metrics.descent;
        metrics.baseline = metrics.ascent;
        previous = Some(atom.right_class);
        laid_out_atoms.push(atom);
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
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: true,
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

fn layout_simple_node(
    font: &MathFont,
    node: &MathNode,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_node_with_mid_target(font, node, font_size, script_level, None)
}

fn layout_simple_node_with_mid_target(
    font: &MathFont,
    node: &MathNode,
    font_size: f32,
    script_level: u8,
    mid_target_height: Option<f32>,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let Some(atom) = simple_atom(node) {
        let layout = if atom.text_operator {
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
        return Ok(Some(layout));
    }

    if let MathNode::Attach(attach) = node {
        return layout_simple_attach(font, attach, font_size, script_level);
    }

    if let MathNode::Fraction(fraction) = node {
        return layout_simple_fraction(font, fraction, font_size, script_level);
    }

    if let MathNode::Cancel(cancel) = node {
        return layout_simple_cancel(font, cancel, font_size, script_level);
    }

    if let MathNode::Accent(accent) = node {
        return layout_simple_accent(font, accent, font_size, script_level);
    }

    if let MathNode::Group(group) = node {
        return layout_simple_group(font, group, font_size, script_level);
    }

    if let MathNode::Call(call) = node {
        if let Some(mode) = MathAttachmentMode::from_call_name(&call.name) {
            return layout_simple_attachment_mode_call(font, call, mode, font_size, script_level);
        }
        if let Some(atom) = layout_simple_operator_call(font, call, font_size, script_level)? {
            return Ok(Some(atom));
        }
        if call.name == "frac" {
            return layout_simple_fraction_call(font, call, font_size, script_level);
        }
        if call.name == "binom" {
            return layout_simple_binom_call(font, call, font_size, script_level);
        }
        if call.name == "class" {
            return layout_simple_class_call(font, call, font_size, script_level);
        }
        if let Some(size) = MathSizeCall::from_name(&call.name) {
            return layout_simple_size_call(font, call, size, font_size, script_level);
        }
        if let Some(position) = MathLineCall::from_name(&call.name) {
            return layout_simple_line_call(font, call, position, font_size, script_level);
        }
        if let Some(selection) = MathStyleSelection::from_call_name(&call.name) {
            return layout_simple_variant_call(font, call, selection, font_size, script_level);
        }
        if call.name == "stretch" {
            return layout_simple_stretch_call(font, call, font_size, script_level);
        }
        if call.name == "mid" {
            return layout_simple_mid_call(font, call, font_size, script_level, mid_target_height);
        }
        if let Some((left, right)) = delimiter_call_chars(&call.name) {
            return layout_simple_delimited_call(font, call, left, right, font_size, script_level);
        }
        if call.name == "lr" {
            return layout_simple_lr_call(font, call, font_size, script_level);
        }
        if call.name == "sqrt" {
            return layout_simple_sqrt(font, call, font_size, script_level);
        }
        if call.name == "root" {
            return layout_simple_root(font, call, font_size, script_level);
        }
    }

    Ok(None)
}

fn layout_simple_class_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [class_arg, body_arg] = &call.args[..] else {
        return Ok(None);
    };
    let [MathNode::StringLiteral(class)] = &class_arg.nodes[..] else {
        return Ok(None);
    };
    let Some(mut atom) =
        layout_simple_nodes_as_atom(font, &body_arg.nodes, font_size, script_level)?
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
    call: &super::ast::MathCall,
    size: MathSizeCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let (font_size, script_level) = match size {
        MathSizeCall::Display | MathSizeCall::Inline => (font_size, script_level),
        MathSizeCall::Script => (
            script_font_size(font, font_size, script_level)?,
            script_level + 1,
        ),
        MathSizeCall::ScriptScript => {
            let script_size = script_font_size(font, font_size, script_level)?;
            (
                script_font_size(font, script_size, script_level + 1)?,
                script_level + 2,
            )
        }
    };
    layout_simple_nodes_as_atom(font, &arg.nodes, font_size, script_level)
}

fn layout_simple_line_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    position: MathLineCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut body) = layout_simple_nodes_as_atom(font, &arg.nodes, font_size, script_level)?
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

fn layout_simple_variant_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    selection: MathStyleSelection,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let styled_nodes = style_math_nodes(&arg.nodes, selection);
    layout_simple_nodes_as_atom(font, &styled_nodes, font_size, script_level)
}

fn layout_simple_stretch_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut atom) = layout_simple_nodes_as_atom(font, &arg.nodes, font_size, script_level)?
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
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
    target_height: Option<f32>,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    let Some(mut atom) = layout_simple_nodes_as_atom(font, &arg.nodes, font_size, script_level)?
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
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
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
        MathNode::Identifier(super::ast::MathIdentifier {
            name: text.to_string(),
            symbol: None,
            byte_range: call.byte_range.start..call.byte_range.start + call.name.len(),
        }),
        MathNode::Group(super::ast::MathGroup {
            left: '(',
            right: ')',
            body: arg.nodes.clone(),
            byte_range: arg.byte_range.clone(),
        }),
    ];
    layout_simple_nodes_as_atom(font, &nodes, font_size, script_level)
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

fn layout_simple_group(
    font: &MathFont,
    group: &super::ast::MathGroup,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_delimited_nodes(
        font,
        group.left,
        &group.body,
        group.right,
        font_size,
        script_level,
        None,
    )
}

fn layout_simple_delimited_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    left: char,
    right: char,
    font_size: f32,
    script_level: u8,
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
    )
}

fn layout_simple_lr_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
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

fn layout_simple_delimited_nodes(
    font: &MathFont,
    left: char,
    body_nodes: &[MathNode],
    right: char,
    font_size: f32,
    script_level: u8,
    explicit_size: Option<super::ast::MathDelimitedSize>,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(body) = layout_simple_nodes_as_atom(font, body_nodes, font_size, script_level)? else {
        return Ok(None);
    };
    let delimiter_target_height =
        delimiter_target_height_for_body(&body, DelimiterTarget::Ink, explicit_size, font_size);
    let body = if contains_mid_call(body_nodes) {
        layout_simple_nodes_as_atom_with_mid_target(
            font,
            body_nodes,
            font_size,
            script_level,
            Some(delimiter_target_height),
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
        DelimiterTarget::Ink,
        delimiter_target_height,
    )
    .map(Some)
}

fn delimiter_target_height_for_body(
    body: &LaidOutMathAtom,
    target: DelimiterTarget,
    explicit_size: Option<super::ast::MathDelimitedSize>,
    font_size: f32,
) -> f32 {
    let natural_target_height = match target {
        DelimiterTarget::Ink => body.ink_ascent + body.ink_descent,
        DelimiterTarget::Frame => body.metrics.height,
    };
    explicit_size
        .map(|size| resolve_relative_math_size(size, natural_target_height, font_size))
        .unwrap_or(natural_target_height)
        .max(0.0)
}

fn layout_simple_delimited_atom(
    font: &MathFont,
    left: char,
    body: LaidOutMathAtom,
    right: char,
    font_size: f32,
    script_level: u8,
    target: DelimiterTarget,
    explicit_size: Option<super::ast::MathDelimitedSize>,
) -> Result<LaidOutMathAtom, LabelError> {
    let delimiter_target_height =
        delimiter_target_height_for_body(&body, target, explicit_size, font_size);
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
    size: super::ast::MathRelativeSize,
    natural_target_height: f32,
    font_size: f32,
) -> f32 {
    size.relative * natural_target_height + size.absolute_em * font_size + size.absolute_pt
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterTarget {
    Ink,
    Frame,
}

const DELIMITER_SHORT_FALL_EM: f32 = 0.1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathStretchAxis {
    Horizontal,
    Vertical,
}

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
    let Some(glyph) = atom.glyphs.first_mut() else {
        return Ok(None);
    };
    let Some(construction) =
        face.tables()
            .math
            .and_then(|math| math.variants)
            .and_then(|variants| match axis {
                MathStretchAxis::Horizontal => {
                    variants.horizontal_constructions.get(glyph.glyph_id)
                }
                MathStretchAxis::Vertical => variants.vertical_constructions.get(glyph.glyph_id),
            })
    else {
        return Ok(None);
    };

    let short_target_size = (target_size - short_fall).max(0.0);
    let stretch_advance = match axis {
        MathStretchAxis::Horizontal => glyph.x_advance,
        MathStretchAxis::Vertical => atom.ink_ascent + atom.ink_descent,
    };
    if !force_variant && short_target_size <= stretch_advance {
        return Ok(Some(()));
    }

    let scale = glyph.font_size / face.units_per_em() as f32;
    let target_units = short_target_size / scale;
    let base_glyph = glyph.glyph_id;
    let mut variant_glyph = glyph.glyph_id;
    let mut variant_advance = None;
    for variant in construction.variants {
        if variant.variant_glyph == base_glyph {
            continue;
        }
        variant_glyph = variant.variant_glyph;
        variant_advance = Some(variant.advance_measurement);
        if variant.advance_measurement as f32 >= target_units {
            break;
        }
    }

    glyph.glyph_id = variant_glyph;
    glyph.x_advance = face
        .glyph_hor_advance(variant_glyph)
        .map(|advance| advance as f32 * scale)
        .or_else(|| variant_advance.map(|advance| advance as f32 * scale))
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
        "bar.double" => Some(('‖', '‖')),
        _ => None,
    }
}

fn layout_simple_sqrt(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_radical(font, &arg.nodes, None, font_size, script_level)
}

fn layout_simple_root(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
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
    )
}

fn layout_simple_cancel(
    font: &MathFont,
    cancel: &MathCancel,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(mut body) = layout_simple_nodes_as_atom(font, &cancel.body, font_size, script_level)?
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let base_nodes = if accent.dotless {
        dotless_accent_base_nodes(&accent.base)
    } else {
        accent.base.clone()
    };
    let Some(mut base) = layout_simple_nodes_as_atom(font, &base_nodes, font_size, script_level)?
    else {
        return Ok(None);
    };
    let width = base.metrics.width;
    let height = base.metrics.height;
    let baseline = base.metrics.baseline;
    let mut accent_atom = layout_accent_atom(font, accent.accent, font_size, script_level)?;
    let base_attach = atom_top_accent_attachment(font, &base)?;
    let accent_attach = atom_top_accent_attachment(font, &accent_atom)?;
    let accent_x = base_attach - accent_attach;
    let accent_y = baseline - accent_atom.metrics.baseline;

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
            height,
            baseline,
            ascent: baseline,
            descent: height - baseline,
        },
        ink_ascent: baseline,
        ink_descent: height - baseline,
        left_class: SimpleMathClass::Alphabetic,
        right_class: SimpleMathClass::Alphabetic,
        italic_correction: 0.0,
        script_kernable: false,
        glyphs,
        shapes,
        draw_order,
    }))
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
                    vec![MathNode::Identifier(super::ast::MathIdentifier {
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
                    vec![MathNode::Text(super::ast::MathText {
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
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
    fraction: &super::ast::MathFraction,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    layout_simple_fraction_nodes(
        font,
        &fraction.numerator,
        &fraction.denominator,
        fraction.style,
        font_size,
        script_level,
    )
}

fn layout_simple_fraction_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
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
    )
}

fn layout_simple_binom_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    font_size: f32,
    script_level: u8,
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

fn binom_lower_nodes(lower: &[super::ast::MathArg]) -> Vec<MathNode> {
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    match style {
        MathFractionStyle::Vertical => layout_simple_stack_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
            StackRule::Fraction,
        ),
        MathFractionStyle::Skewed => layout_simple_skewed_fraction_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
        ),
        MathFractionStyle::Horizontal => layout_simple_horizontal_fraction_nodes(
            font,
            numerator_nodes,
            denominator_nodes,
            font_size,
            script_level,
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StackRule {
    Fraction,
    None,
}

fn layout_simple_stack_nodes(
    font: &MathFont,
    numerator_nodes: &[MathNode],
    denominator_nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
    rule: StackRule,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let child_font_size = script_font_size(font, font_size, script_level)?;
    let Some(mut numerator) =
        layout_fraction_child_nodes(font, numerator_nodes, child_font_size, script_level + 1)?
    else {
        return Ok(None);
    };
    let Some(mut denominator) =
        layout_fraction_child_nodes(font, denominator_nodes, child_font_size, script_level + 1)?
    else {
        return Ok(None);
    };

    if rule == StackRule::None {
        return layout_simple_no_rule_stack(font, numerator, denominator, font_size);
    }

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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let shift_up = math_constant(font, font_size, |constants| {
        constants.stack_top_shift_up().value
    })?;
    let shift_down = math_constant(font, font_size, |constants| {
        constants.stack_bottom_shift_down().value
    })?;
    let gap_min = math_constant(font, font_size, |constants| constants.stack_gap_min().value)?;
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

fn finalize_inline_frame_atom(
    width: f32,
    height: f32,
    baseline: f32,
    mut glyphs: Vec<LaidOutGlyph>,
    mut shapes: Vec<LaidOutShape>,
    draw_order: Vec<LaidOutDrawItem>,
    font: &MathFont,
    font_size: f32,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
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
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: true,
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(numerator) =
        layout_simple_nodes_as_atom(font, numerator_nodes, font_size, script_level)?
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
    let Some(denominator) =
        layout_simple_nodes_as_atom(font, denominator_nodes, font_size, script_level)?
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let child_font_size = script_font_size(font, font_size, script_level)?;
    let Some(mut numerator) =
        layout_fraction_child_nodes(font, numerator_nodes, child_font_size, script_level + 1)?
    else {
        return Ok(None);
    };
    let Some(mut denominator) =
        layout_fraction_child_nodes(font, denominator_nodes, child_font_size, script_level + 1)?
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
        left_class: SimpleMathClass::Normal,
        right_class: SimpleMathClass::Normal,
        italic_correction: 0.0,
        script_kernable: true,
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
        ink_ascent = ink_ascent.max(atom.ink_ascent + dy);
        ink_descent = ink_descent.max((atom.ink_descent - dy).max(0.0));
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
        left_class,
        right_class,
        italic_correction: 0.0,
        script_kernable,
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let MathNode::Group(group) = node {
        return layout_simple_nodes_as_atom(font, &group.body, font_size, script_level);
    }
    layout_simple_node(font, node, font_size, script_level)
}

fn layout_fraction_child_nodes(
    font: &MathFont,
    nodes: &[MathNode],
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let [node] = nodes {
        return layout_fraction_child(font, node, font_size, script_level);
    }
    layout_simple_nodes_as_atom(font, nodes, font_size, script_level)
}

const FRACTION_PADDING_EM: f32 = 0.1;
const INLINE_MATH_LEADING_SLACK_EM: f32 = 0.65 * 0.7;
const CANCEL_STROKE_EM: f32 = 0.05;
const SCRIPT_SLOT_PAIR_GAP_EM: f32 = 0.08;
const PRIME_CHAR: char = '′';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathAttachmentMode {
    Scripts,
    Limits,
}

impl MathAttachmentMode {
    fn from_call_name(name: &str) -> Option<Self> {
        match name {
            "scripts" | "limits_display" => Some(Self::Scripts),
            "limits" => Some(Self::Limits),
            _ => None,
        }
    }

    fn explicit_from_base_node(node: &MathNode) -> Option<Self> {
        if let MathNode::Call(call) = node {
            Self::from_call_name(&call.name)
        } else {
            None
        }
    }

    fn default_for_base(base: &LaidOutMathAtom) -> Self {
        if base.left_class == SimpleMathClass::Relation
            && base.right_class == SimpleMathClass::Relation
        {
            Self::Limits
        } else {
            Self::Scripts
        }
    }
}

fn layout_simple_attachment_mode_call(
    font: &MathFont,
    call: &super::ast::MathCall,
    _mode: MathAttachmentMode,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let [arg] = &call.args[..] else {
        return Ok(None);
    };
    layout_simple_nodes_as_atom(font, &arg.nodes, font_size, script_level)
}

fn layout_simple_attach(
    font: &MathFont,
    attach: &super::ast::MathAttach,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(base) = layout_simple_node(font, &attach.base, font_size, script_level)? else {
        return Ok(None);
    };
    let mode = MathAttachmentMode::explicit_from_base_node(&attach.base)
        .unwrap_or_else(|| MathAttachmentMode::default_for_base(&base));
    let script_font_size = script_font_size(font, font_size, script_level)?;
    let slots = layout_attach_slots(font, attach, script_font_size, script_level + 1)?;
    let slots = attach_slots_with_primes(
        font,
        slots,
        attach.primes,
        script_font_size,
        script_level + 1,
    )?;
    if attach_slots_missing_requested(attach, &slots) {
        return Ok(None);
    }

    layout_simple_attach_parts(font, font_size, base, slots, mode)
}

fn layout_simple_attach_with_bottom_continuation(
    font: &MathFont,
    attach: &super::ast::MathAttach,
    group: &super::ast::MathGroup,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some([bottom_node]) = attach.bottom.as_deref() else {
        return Ok(None);
    };
    let Some(base) = layout_simple_node(font, &attach.base, font_size, script_level)? else {
        return Ok(None);
    };
    let mode = MathAttachmentMode::explicit_from_base_node(&attach.base)
        .unwrap_or_else(|| MathAttachmentMode::default_for_base(&base));
    let script_font_size = script_font_size(font, font_size, script_level)?;
    let mut slots = layout_attach_slots(font, attach, script_font_size, script_level + 1)?;
    slots = attach_slots_with_primes(
        font,
        slots,
        attach.primes,
        script_font_size,
        script_level + 1,
    )?;
    let bottom_nodes = [bottom_node.clone(), MathNode::Group(group.clone())];
    let bottom =
        layout_simple_nodes_as_atom(font, &bottom_nodes, script_font_size, script_level + 1)?;
    if attach_slots_missing_requested(attach, &slots) || bottom.is_none() {
        return Ok(None);
    }
    slots.bottom = bottom;

    layout_simple_attach_parts(font, font_size, base, slots, mode)
}

fn is_identifier_subscript_group_continuation(
    attach: &super::ast::MathAttach,
    group: &super::ast::MathGroup,
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
    attach: &super::ast::MathAttach,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutAttachSlots, LabelError> {
    Ok(LaidOutAttachSlots {
        top: layout_script_nodes(font, attach.top.as_deref(), font_size, script_level)?,
        bottom: layout_script_nodes(font, attach.bottom.as_deref(), font_size, script_level)?,
        top_left: layout_script_nodes(font, attach.top_left.as_deref(), font_size, script_level)?,
        top_right: layout_script_nodes(font, attach.top_right.as_deref(), font_size, script_level)?,
        bottom_left: layout_script_nodes(
            font,
            attach.bottom_left.as_deref(),
            font_size,
            script_level,
        )?,
        bottom_right: layout_script_nodes(
            font,
            attach.bottom_right.as_deref(),
            font_size,
            script_level,
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
    let text = PRIME_CHAR.to_string().repeat(primes);
    layout_styled_atom_with_class(
        font,
        &text,
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Normal,
    )
    .map(Some)
}

fn layout_script_nodes(
    font: &MathFont,
    nodes: Option<&[MathNode]>,
    font_size: f32,
    script_level: u8,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let Some(nodes) = nodes else {
        return Ok(None);
    };
    if let [node] = nodes {
        return layout_script_child(font, node, font_size, script_level);
    }
    layout_simple_nodes_as_atom(font, nodes, font_size, script_level)
}

fn attach_slots_missing_requested(
    attach: &super::ast::MathAttach,
    slots: &LaidOutAttachSlots,
) -> bool {
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    match mode {
        MathAttachmentMode::Scripts => {
            layout_simple_script_attach_parts(font, font_size, base, slots)
        }
        MathAttachmentMode::Limits => {
            layout_simple_limit_attach_parts(font, font_size, base, slots)
        }
    }
}

fn layout_simple_script_attach_parts(
    font: &MathFont,
    font_size: f32,
    base: LaidOutMathAtom,
    slots: LaidOutAttachSlots,
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    let post_top = combine_script_slots(font_size, slots.top, slots.top_right)?;
    let post_bottom = combine_script_slots(font_size, slots.bottom, slots.bottom_right)?;
    let top_ref = post_top.as_ref().or(slots.top_left.as_ref());
    let bottom_ref = post_bottom.as_ref().or(slots.bottom_left.as_ref());
    let (shift_up, shift_down) =
        compute_script_shifts(font, font_size, &base, top_ref, bottom_ref)?;
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
    let base_metrics = base.metrics;
    let base_left_class = base.left_class;
    let base_right_class = base.right_class;
    let base_italic_correction = base.italic_correction;
    let base_script_kernable = base.script_kernable;
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
    offset_atom(&mut base, pre_width, 0.0);
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
            ..base_metrics
        },
        ink_ascent,
        ink_descent,
        left_class: base_left_class,
        right_class: base_right_class,
        italic_correction: base_italic_correction,
        script_kernable: base_script_kernable,
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if slots.top.is_none() && slots.bottom.is_none() {
        return layout_simple_script_attach_parts(font, font_size, base, slots);
    }

    let top = slots.top.take();
    let bottom = slots.bottom.take();
    let Some(mut base) = layout_simple_script_attach_parts(font, font_size, base, slots)? else {
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
    let base_italic_correction = base.italic_correction;
    let base_script_kernable = base.script_kernable;
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
        left_class: base_left_class,
        right_class: base_right_class,
        italic_correction: base_italic_correction,
        script_kernable: base_script_kernable,
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
            let ascent = (first.metrics.ascent + first_dy).max(second.metrics.ascent + second_dy);
            let descent =
                (first.metrics.descent - first_dy).max(second.metrics.descent - second_dy);
            let ink_ascent = (first.ink_ascent + first_dy).max(second.ink_ascent + second_dy);
            let ink_descent = (first.ink_descent - first_dy).max(second.ink_descent - second_dy);

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
                left_class: SimpleMathClass::Normal,
                right_class: SimpleMathClass::Normal,
                italic_correction: 0.0,
                script_kernable: false,
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
) -> Result<Option<LaidOutMathAtom>, LabelError> {
    if let MathNode::Group(group) = node {
        return layout_simple_nodes_as_atom(font, &group.body, font_size, script_level);
    }
    layout_simple_node(font, node, font_size, script_level)
}

fn script_font_size(font: &MathFont, font_size: f32, script_level: u8) -> Result<f32, LabelError> {
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
    font: &MathFont,
    font_size: f32,
    base: &LaidOutMathAtom,
    top: Option<&LaidOutMathAtom>,
    bottom: Option<&LaidOutMathAtom>,
) -> Result<(f32, f32), LabelError> {
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
    // need those fields when fractions and radicals become supported.
    let _ = base;

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
    let lower_gap_min = math_constant(font, font_size, |constants| {
        constants.lower_limit_gap_min().value
    })?;

    let upper_shift = top.map_or(0.0, |top| base.ink_ascent + upper_gap_min + top.ink_descent);
    let lower_shift = bottom.map_or(0.0, |_| base.ink_descent + lower_gap_min);
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

#[cfg(test)]
fn single_atom_text(math: &MathAst) -> Option<String> {
    let [node] = &math.nodes[..] else {
        return None;
    };
    simple_atom(node).map(|atom| atom.styled_text)
}

fn simple_atom(node: &MathNode) -> Option<SimpleMathAtom> {
    match node {
        MathNode::Text(text) => Some(SimpleMathAtom {
            styled_text: style_text_atom(text),
            class: match text.kind {
                MathTextKind::Grapheme => SimpleMathClass::Alphabetic,
                MathTextKind::Number => SimpleMathClass::Normal,
            },
            text_operator: false,
        }),
        MathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            if let Some(operator) = operator_identifier_text(text) {
                Some(SimpleMathAtom {
                    styled_text: operator.to_string(),
                    class: SimpleMathClass::Large,
                    text_operator: true,
                })
            } else if identifier.symbol.is_some() || text.chars().count() == 1 {
                Some(SimpleMathAtom {
                    styled_text: style_default_math_text(text),
                    class: identifier_class(text),
                    text_operator: false,
                })
            } else {
                None
            }
        }
        MathNode::Operator(operator) => Some(SimpleMathAtom {
            styled_text: operator_text(operator),
            class: operator_class(&operator.operator),
            text_operator: false,
        }),
        MathNode::Shorthand(shorthand) => Some(SimpleMathAtom {
            styled_text: shorthand_text(shorthand),
            class: symbol_class(shorthand.replacement),
            text_operator: false,
        }),
        _ => None,
    }
}

fn layout_operator_atom(
    font: &MathFont,
    text: &str,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutMathAtom, LabelError> {
    let face =
        ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to parse Typst math font".to_string(),
        })?;
    let Some(rusty) = rustybuzz::Face::from_slice(&font.data, font.face_index) else {
        return Err(LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to shape Typst math font".to_string(),
        });
    };

    let features = if let Some(script_style) = script_style_feature(script_level) {
        vec![rustybuzz::Feature::new(
            rustybuzz::ttf_parser::Tag::from_bytes(b"ssty"),
            script_style,
            ..,
        )]
    } else {
        Vec::new()
    };
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(rustybuzz::Direction::LeftToRight);
    buffer.set_flags(rustybuzz::BufferFlags::REMOVE_DEFAULT_IGNORABLES);

    let scale = font_size / face.units_per_em() as f32;
    let shaped = rustybuzz::shape(&rusty, &features, buffer);
    let mut cursor_x = 0i32;
    let mut cursor_y = 0i32;
    let mut width = 0i32;
    let mut glyph_ascent = 0i16;
    let mut glyph_descent = 0i16;
    let mut glyphs = Vec::new();
    for (info, position) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let glyph_id = ttf_parser::GlyphId(info.glyph_id as u16);
        let x = cursor_x + position.x_offset;
        let y = cursor_y + position.y_offset;
        cursor_x += position.x_advance;
        cursor_y += position.y_advance;
        width += position.x_advance;
        glyphs.push(LaidOutGlyph {
            glyph_id,
            unicode: text
                .get(glyph_cluster_range(text, info.cluster))
                .unwrap_or_default()
                .to_string(),
            x: x as f32 * scale,
            y: -(y as f32) * scale,
            x_advance: position.x_advance as f32 * scale,
            font_size,
            pdf_run_group: Some(0),
        });
        if let Some(bounds) = face.glyph_bounding_box(glyph_id) {
            glyph_ascent = glyph_ascent.max(bounds.y_max);
            glyph_descent = glyph_descent.max(-bounds.y_min);
        }
    }

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
        left_class: SimpleMathClass::Large,
        right_class: SimpleMathClass::Large,
        italic_correction: 0.0,
        script_kernable: false,
        glyphs,
        shapes: Vec::new(),
        draw_order: Vec::new(),
    };
    for glyph in &mut atom.glyphs {
        glyph.y += atom.metrics.baseline;
    }
    atom.draw_order
        .extend((0..atom.glyphs.len()).map(LaidOutDrawItem::Glyph));
    Ok(atom)
}

fn operator_identifier_text(name: &str) -> Option<&'static str> {
    predefined_operator_text(name)
}

fn glyph_cluster_range(text: &str, cluster: u32) -> std::ops::Range<usize> {
    let cluster = cluster as usize;
    let Some((start, _)) = text.char_indices().find(|(start, _)| *start == cluster) else {
        return 0..0;
    };
    let end = text[start..]
        .char_indices()
        .nth(1)
        .map_or(text.len(), |(next, _)| start + next);
    start..end
}

fn style_text_atom(text: &MathText) -> String {
    match text.kind {
        MathTextKind::Grapheme => style_default_math_text(&text.text),
        MathTextKind::Number => text.text.clone(),
    }
}

fn style_default_math_text(text: &str) -> String {
    text.chars().map(style_default_math_char).collect()
}

fn style_math_nodes(nodes: &[MathNode], selection: MathStyleSelection) -> Vec<MathNode> {
    nodes
        .iter()
        .flat_map(|node| style_math_node(node, selection))
        .collect()
}

fn style_math_node(node: &MathNode, selection: MathStyleSelection) -> Vec<MathNode> {
    match node {
        MathNode::Space(_)
        | MathNode::Operator(_)
        | MathNode::Shorthand(_)
        | MathNode::StringLiteral(_) => vec![node.clone()],
        MathNode::Text(text) => vec![MathNode::Text(super::ast::MathText {
            text: style_math_text_with_selection(&text.text, selection),
            kind: MathTextKind::Number,
            byte_range: text.byte_range.clone(),
        })],
        MathNode::Identifier(identifier) => {
            let text = identifier.symbol.unwrap_or(&identifier.name);
            vec![MathNode::Text(super::ast::MathText {
                text: style_math_text_with_selection(text, selection),
                kind: MathTextKind::Number,
                byte_range: identifier.byte_range.clone(),
            })]
        }
        MathNode::Group(group) => vec![MathNode::Group(super::ast::MathGroup {
            left: group.left,
            right: group.right,
            body: style_math_nodes(&group.body, selection),
            byte_range: group.byte_range.clone(),
        })],
        MathNode::Attach(attach) => {
            let base = style_single_math_node(&attach.base, selection);
            vec![MathNode::Attach(super::ast::MathAttach {
                base: Box::new(base),
                top: style_optional_math_nodes(&attach.top, selection),
                bottom: style_optional_math_nodes(&attach.bottom, selection),
                top_left: style_optional_math_nodes(&attach.top_left, selection),
                top_right: style_optional_math_nodes(&attach.top_right, selection),
                bottom_left: style_optional_math_nodes(&attach.bottom_left, selection),
                bottom_right: style_optional_math_nodes(&attach.bottom_right, selection),
                primes: attach.primes,
                byte_range: attach.byte_range.clone(),
            })]
        }
        MathNode::Fraction(fraction) => {
            vec![MathNode::Fraction(super::ast::MathFraction {
                numerator: style_math_nodes(&fraction.numerator, selection),
                denominator: style_math_nodes(&fraction.denominator, selection),
                style: fraction.style,
                slash_range: fraction.slash_range.clone(),
                byte_range: fraction.byte_range.clone(),
            })]
        }
        MathNode::Cancel(cancel) => vec![MathNode::Cancel(super::ast::MathCancel {
            body: style_math_nodes(&cancel.body, selection),
            options: cancel.options.clone(),
            byte_range: cancel.byte_range.clone(),
        })],
        MathNode::Accent(accent) => vec![MathNode::Accent(super::ast::MathAccent {
            base: style_math_nodes(&accent.base, selection),
            accent: accent.accent,
            dotless: accent.dotless,
            byte_range: accent.byte_range.clone(),
        })],
        MathNode::Call(call) => {
            if let Some(nested) = MathStyleSelection::from_call_name(&call.name) {
                let combined = selection.compose(nested);
                return call
                    .args
                    .iter()
                    .flat_map(|arg| style_math_nodes(&arg.nodes, combined))
                    .collect();
            }

            vec![MathNode::Call(super::ast::MathCall {
                name: call.name.clone(),
                args: call
                    .args
                    .iter()
                    .map(|arg| super::ast::MathArg {
                        nodes: style_math_nodes(&arg.nodes, selection),
                        byte_range: arg.byte_range.clone(),
                    })
                    .collect(),
                options: call.options,
                byte_range: call.byte_range.clone(),
            })]
        }
    }
}

fn style_optional_math_nodes(
    nodes: &Option<Vec<MathNode>>,
    selection: MathStyleSelection,
) -> Option<Vec<MathNode>> {
    nodes
        .as_ref()
        .map(|nodes| style_math_nodes(nodes, selection))
}

fn style_single_math_node(node: &MathNode, selection: MathStyleSelection) -> MathNode {
    let mut styled = style_math_node(node, selection);
    if styled.len() == 1 {
        styled.remove(0)
    } else {
        MathNode::Group(super::ast::MathGroup {
            left: '(',
            right: ')',
            body: styled,
            byte_range: node.byte_range(),
        })
    }
}

fn style_math_text_with_selection(text: &str, selection: MathStyleSelection) -> String {
    text.chars()
        .flat_map(|ch| {
            let style = MathAlphabetStyle::select(ch, selection);
            style_math_char(ch, style)
                .into_iter()
                .filter(|styled| *styled != '\0')
        })
        .collect()
}

fn operator_text(operator: &MathOperator) -> String {
    match operator.operator.as_str() {
        "-" => "−".to_string(),
        _ => operator.operator.clone(),
    }
}

fn shorthand_text(shorthand: &MathShorthand) -> String {
    shorthand.replacement.to_string()
}

fn identifier_class(text: &str) -> SimpleMathClass {
    if matches!(text, "∑" | "∏" | "∫") {
        SimpleMathClass::Large
    } else if let Some(ch) = single_char(text) {
        simple_math_class_for_char(ch)
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
    single_char(text)
        .map(simple_math_class_for_char)
        .unwrap_or(SimpleMathClass::Normal)
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

fn simple_math_class_for_char(ch: char) -> SimpleMathClass {
    use unicode_math_class::MathClass;

    let class = match ch {
        ':' => Some(MathClass::Relation),
        '⋯' | '⋱' | '⋰' | '⋮' => Some(MathClass::Normal),
        '.' | '/' => Some(MathClass::Normal),
        '\u{22A5}' => Some(MathClass::Normal),
        '⅋' => Some(MathClass::Binary),
        '⎰' | '⟅' => Some(MathClass::Opening),
        '⎱' | '⟆' => Some(MathClass::Closing),
        '⟇' => Some(MathClass::Binary),
        '،' => Some(MathClass::Punctuation),
        c => unicode_math_class::class(c),
    };

    match class {
        Some(MathClass::Alphabetic) => SimpleMathClass::Alphabetic,
        Some(MathClass::Binary) => SimpleMathClass::Binary,
        Some(MathClass::Closing) => SimpleMathClass::Closing,
        Some(MathClass::Fence) => SimpleMathClass::Fence,
        Some(MathClass::Large) => SimpleMathClass::Large,
        Some(MathClass::Opening) => SimpleMathClass::Opening,
        Some(MathClass::Punctuation) => SimpleMathClass::Punctuation,
        Some(MathClass::Relation) => SimpleMathClass::Relation,
        Some(MathClass::Unary) => SimpleMathClass::Unary,
        Some(MathClass::Vary) => SimpleMathClass::Vary,
        _ => SimpleMathClass::Normal,
    }
}

fn simple_math_class_from_name(name: &str) -> Option<SimpleMathClass> {
    match name {
        "normal" => Some(SimpleMathClass::Normal),
        "alphabetic" => Some(SimpleMathClass::Alphabetic),
        "binary" => Some(SimpleMathClass::Binary),
        "unary" => Some(SimpleMathClass::Unary),
        "vary" => Some(SimpleMathClass::Vary),
        "relation" => Some(SimpleMathClass::Relation),
        "opening" => Some(SimpleMathClass::Opening),
        "closing" => Some(SimpleMathClass::Closing),
        "fence" => Some(SimpleMathClass::Fence),
        "punctuation" => Some(SimpleMathClass::Punctuation),
        "large" => Some(SimpleMathClass::Large),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathSizeCall {
    Display,
    Inline,
    Script,
    ScriptScript,
}

impl MathSizeCall {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "display" => Some(Self::Display),
            "inline" => Some(Self::Inline),
            "script" => Some(Self::Script),
            "sscript" => Some(Self::ScriptScript),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathLineCall {
    Below,
    Above,
}

impl MathLineCall {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "underline" => Some(Self::Below),
            "overline" => Some(Self::Above),
            _ => None,
        }
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct MathStyleSelection {
    variant: Option<MathVariant>,
    bold: bool,
    italic: Option<bool>,
}

impl MathStyleSelection {
    fn from_call_name(name: &str) -> Option<Self> {
        let mut selection = Self::default();
        match name {
            "bold" => selection.bold = true,
            "upright" => selection.italic = Some(false),
            "italic" => selection.italic = Some(true),
            "serif" => selection.variant = Some(MathVariant::Plain),
            "sans" => selection.variant = Some(MathVariant::SansSerif),
            "cal" => selection.variant = Some(MathVariant::Chancery),
            "scr" => selection.variant = Some(MathVariant::Roundhand),
            "frak" => selection.variant = Some(MathVariant::Fraktur),
            "mono" => selection.variant = Some(MathVariant::Monospace),
            "bb" => selection.variant = Some(MathVariant::DoubleStruck),
            _ => return None,
        }
        Some(selection)
    }

    fn compose(self, nested: Self) -> Self {
        Self {
            variant: nested.variant.or(self.variant),
            bold: self.bold || nested.bold,
            italic: nested.italic.or(self.italic),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathVariant {
    Plain,
    Fraktur,
    SansSerif,
    Monospace,
    DoubleStruck,
    Chancery,
    Roundhand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathAlphabetStyle {
    Plain,
    Bold,
    Italic,
    BoldItalic,
    Fraktur,
    BoldFraktur,
    SansSerif,
    SansSerifBold,
    SansSerifItalic,
    SansSerifBoldItalic,
    Monospace,
    DoubleStruck,
    DoubleStruckItalic,
    Chancery,
    BoldChancery,
    Roundhand,
    BoldRoundhand,
    Hebrew,
}

impl MathAlphabetStyle {
    fn select(ch: char, selection: MathStyleSelection) -> Self {
        use MathAlphabetStyle::*;

        match (
            selection.variant.unwrap_or(MathVariant::Plain),
            selection.bold,
            selection.italic,
        ) {
            (MathVariant::SansSerif, false, Some(false)) if ch.is_ascii_alphabetic() => SansSerif,
            (MathVariant::SansSerif, false, _) if ch.is_ascii_alphabetic() => SansSerifItalic,
            (MathVariant::SansSerif, true, Some(false)) if ch.is_ascii_alphabetic() => {
                SansSerifBold
            }
            (MathVariant::SansSerif, true, _) if ch.is_ascii_alphabetic() => SansSerifBoldItalic,
            (MathVariant::SansSerif, false, _) if ch.is_ascii_digit() => SansSerif,
            (MathVariant::SansSerif, true, _) if ch.is_ascii_digit() => SansSerifBold,
            (MathVariant::SansSerif, _, Some(false)) if is_greek_math_char(ch) => SansSerifBold,
            (MathVariant::SansSerif, _, Some(true)) if is_greek_math_char(ch) => {
                SansSerifBoldItalic
            }
            (MathVariant::SansSerif, _, None) if is_upper_greek_math_char(ch) => SansSerifBold,
            (MathVariant::SansSerif, _, None) if is_lower_greek_math_char(ch) => {
                SansSerifBoldItalic
            }
            (MathVariant::Fraktur, false, _) if ch.is_ascii_alphabetic() => Fraktur,
            (MathVariant::Fraktur, true, _) if ch.is_ascii_alphabetic() => BoldFraktur,
            (MathVariant::Monospace, _, _) if ch.is_ascii_digit() || ch.is_ascii_alphabetic() => {
                Monospace
            }
            (MathVariant::DoubleStruck, _, Some(true))
                if matches!(ch, 'D' | 'd' | 'e' | 'i' | 'j') =>
            {
                DoubleStruckItalic
            }
            (MathVariant::DoubleStruck, _, _)
                if ch.is_ascii_digit()
                    || ch.is_ascii_alphabetic()
                    || matches!(ch, '∑' | 'Γ' | 'Π' | 'γ' | 'π') =>
            {
                DoubleStruck
            }
            (MathVariant::Chancery, false, _) if ch.is_ascii_alphabetic() => Chancery,
            (MathVariant::Chancery, true, _) if ch.is_ascii_alphabetic() => BoldChancery,
            (MathVariant::Roundhand, false, _) if ch.is_ascii_alphabetic() => Roundhand,
            (MathVariant::Roundhand, true, _) if ch.is_ascii_alphabetic() => BoldRoundhand,
            (_, false, Some(true)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => Italic,
            (_, false, None) if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) => Italic,
            (_, true, Some(false)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => Bold,
            (_, true, Some(true)) if ch.is_ascii_alphabetic() || is_greek_math_char(ch) => {
                BoldItalic
            }
            (_, true, None) if ch.is_ascii_alphabetic() || is_lower_greek_math_char(ch) => {
                BoldItalic
            }
            (_, true, None) if is_upper_greek_math_char(ch) => Bold,
            (_, true, _) if ch.is_ascii_digit() || matches!(ch, 'Ϝ' | 'ϝ') => Bold,
            (_, _, Some(true) | None) if matches!(ch, 'ı' | 'ȷ' | 'ħ') => Italic,
            (_, _, Some(true) | None) if is_hebrew_math_char(ch) => Hebrew,
            _ => Plain,
        }
    }
}

fn style_math_char(ch: char, style: MathAlphabetStyle) -> [char; 2] {
    use MathAlphabetStyle::*;
    match style {
        Plain => [ch, '\0'],
        Bold => [to_math_bold(ch), '\0'],
        Italic => [to_math_italic(ch), '\0'],
        BoldItalic => [to_math_bold_italic(ch), '\0'],
        Fraktur => [to_math_fraktur(ch), '\0'],
        BoldFraktur => [to_math_bold_fraktur(ch), '\0'],
        SansSerif => [to_math_sans_serif(ch), '\0'],
        SansSerifBold => [to_math_sans_serif_bold(ch), '\0'],
        SansSerifItalic => [to_math_sans_serif_italic(ch), '\0'],
        SansSerifBoldItalic => [to_math_sans_serif_bold_italic(ch), '\0'],
        Monospace => [to_math_monospace(ch), '\0'],
        DoubleStruck => [to_math_double_struck(ch), '\0'],
        DoubleStruckItalic => [to_math_double_struck_italic(ch), '\0'],
        Chancery => with_variation_selector(to_math_script(ch), '\u{fe00}', ch),
        BoldChancery => with_variation_selector(to_math_bold_script(ch), '\u{fe00}', ch),
        Roundhand => with_variation_selector(to_math_script(ch), '\u{fe01}', ch),
        BoldRoundhand => with_variation_selector(to_math_bold_script(ch), '\u{fe01}', ch),
        Hebrew => [to_math_hebrew(ch), '\0'],
    }
}

fn with_variation_selector(styled: char, selector: char, original: char) -> [char; 2] {
    if styled == original && !original.is_ascii_alphabetic() {
        [styled, '\0']
    } else {
        [styled, selector]
    }
}

fn is_greek_math_char(ch: char) -> bool {
    is_upper_greek_math_char(ch) || is_lower_greek_math_char(ch)
}

fn is_upper_greek_math_char(ch: char) -> bool {
    matches!(ch, 'Α'..='Ω' | '∇' | 'ϴ')
}

fn is_hebrew_math_char(ch: char) -> bool {
    matches!(ch, 'א'..='ד')
}

fn apply_math_delta(ch: char, delta: u32) -> char {
    std::char::from_u32((ch as u32) + delta).unwrap_or(ch)
}

fn to_math_bold(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D3BF,
        'a'..='z' => 0x1D3B9,
        'Α'..='Ρ' => 0x1D317,
        'ϴ' => 0x1D2C5,
        'Σ'..='Ω' => 0x1D317,
        '∇' => 0x1B4BA,
        'α'..='ω' => 0x1D311,
        '∂' => 0x1B4D9,
        'ϵ' => 0x1D2E7,
        'ϑ' => 0x1D30C,
        'ϰ' => 0x1D2EE,
        'ϕ' => 0x1D30A,
        'ϱ' => 0x1D2EF,
        'ϖ' => 0x1D30B,
        'Ϝ'..='ϝ' => 0x1D3EE,
        '0'..='9' => 0x1D79E,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D427,
        'a'..='z' => 0x1D421,
        'Α'..='Ρ' => 0x1D38B,
        'ϴ' => 0x1D339,
        'Σ'..='Ω' => 0x1D38B,
        '∇' => 0x1B52E,
        'α'..='ω' => 0x1D385,
        '∂' => 0x1B54D,
        'ϵ' => 0x1D35B,
        'ϑ' => 0x1D380,
        'ϰ' => 0x1D362,
        'ϕ' => 0x1D37E,
        'ϱ' => 0x1D363,
        'ϖ' => 0x1D37F,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_script(ch: char) -> char {
    let delta = match ch {
        'g' => 0x20A3,
        'H' => 0x20C3,
        'I' => 0x20C7,
        'L' => 0x20C6,
        'R' => 0x20C9,
        'B' => 0x20EA,
        'e' => 0x20CA,
        'E'..='F' => 0x20EB,
        'M' => 0x20E6,
        'o' => 0x20C5,
        'A'..='Z' => 0x1D45B,
        'a'..='z' => 0x1D455,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_script(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D48F,
        'a'..='z' => 0x1D489,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_fraktur(ch: char) -> char {
    let delta = match ch {
        'H' => 0x20C4,
        'I' => 0x20C8,
        'R' => 0x20CA,
        'Z' => 0x20CE,
        'C' => 0x20EA,
        'A'..='Z' => 0x1D4C3,
        'a'..='z' => 0x1D4BD,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_bold_fraktur(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D52B,
        'a'..='z' => 0x1D525,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D55F,
        'a'..='z' => 0x1D559,
        '0'..='9' => 0x1D7B2,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_bold(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D593,
        'a'..='z' => 0x1D58D,
        'Α'..='Ρ' => 0x1D3C5,
        'ϴ' => 0x1D373,
        'Σ'..='Ω' => 0x1D3C5,
        '∇' => 0x1B568,
        'α'..='ω' => 0x1D3BF,
        '∂' => 0x1B587,
        'ϵ' => 0x1D395,
        'ϑ' => 0x1D3BA,
        'ϰ' => 0x1D39C,
        'ϕ' => 0x1D3B8,
        'ϱ' => 0x1D39D,
        'ϖ' => 0x1D3B9,
        '0'..='9' => 0x1D7BC,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D5C7,
        'a'..='z' => 0x1D5C1,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_sans_serif_bold_italic(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D5FB,
        'a'..='z' => 0x1D5F5,
        'Α'..='Ρ' => 0x1D3FF,
        'ϴ' => 0x1D3AD,
        'Σ'..='Ω' => 0x1D3FF,
        '∇' => 0x1B5A2,
        'α'..='ω' => 0x1D3F9,
        '∂' => 0x1B5C1,
        'ϵ' => 0x1D3CF,
        'ϑ' => 0x1D3F4,
        'ϰ' => 0x1D3D6,
        'ϕ' => 0x1D3F2,
        'ϱ' => 0x1D3D7,
        'ϖ' => 0x1D3F3,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_monospace(ch: char) -> char {
    let delta = match ch {
        'A'..='Z' => 0x1D62F,
        'a'..='z' => 0x1D629,
        '0'..='9' => 0x1D7C6,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_double_struck(ch: char) -> char {
    let delta = match ch {
        'C' => 0x20BF,
        'H' => 0x20C5,
        'N' => 0x20C7,
        'P'..='Q' => 0x20C9,
        'R' => 0x20CB,
        'Z' => 0x20CA,
        'π' => 0x1D7C,
        'γ' => 0x1D8A,
        'Γ' => 0x1DAB,
        'Π' => 0x1D9F,
        '∑' => return '⅀',
        'A'..='Z' => 0x1D4F7,
        'a'..='z' => 0x1D4F1,
        '0'..='9' => 0x1D7A8,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_double_struck_italic(ch: char) -> char {
    let delta = match ch {
        'D' => 0x2101,
        'd'..='e' => 0x20E2,
        'i'..='j' => 0x20DF,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

fn to_math_hebrew(ch: char) -> char {
    let delta = match ch {
        'א'..='ד' => 0x1B65,
        _ => return ch,
    };
    apply_math_delta(ch, delta)
}

struct MathFont {
    data: Vec<u8>,
    face_index: u32,
}

fn load_default_math_font(
    config: &EngineOptions,
    spec: &MathFontSpec,
    weight: &FontWeight,
) -> Option<MathFont> {
    if matches!(spec, MathFontSpec::LeteSansMath) {
        let target_weight = font_weight_number(weight);
        let mut bundled = crate::fonts::bundled_math_fonts()
            .iter()
            .collect::<Vec<_>>();
        bundled.sort_by_key(|face| {
            (
                face.weight.abs_diff(target_weight),
                face.weight < target_weight,
            )
        });
        for face in bundled {
            if let Some(font) = math_font_from_data(face.decompressed_data().to_vec()) {
                return Some(font);
            }
        }
    }

    for path in crate::fonts::candidate_math_font_paths(config) {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };
        if let Some(font) = math_font_from_data(data) {
            return Some(font);
        }
    }
    None
}

fn font_weight_number(weight: &FontWeight) -> u16 {
    match weight {
        FontWeight::Normal => 400,
        FontWeight::Bold => 700,
        FontWeight::Number(value) => (*value).clamp(1, 1000),
    }
}

fn math_font_from_data(data: Vec<u8>) -> Option<MathFont> {
    let face_count = ttf_parser::fonts_in_collection(&data).unwrap_or(1);
    for face_index in 0..face_count {
        let Ok(face) = ttf_parser::Face::parse(&data, face_index) else {
            continue;
        };
        if face.tables().math.is_some() {
            return Some(MathFont { data, face_index });
        }
    }
    None
}

fn parse_math_face<'a>(
    font: &'a MathFont,
    context: &str,
) -> Result<ttf_parser::Face<'a>, LabelError> {
    ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
        start: 0,
        end: 0,
        message: format!("failed to parse Typst math font for {context}"),
    })
}

fn layout_styled_atom_with_class(
    font: &MathFont,
    text: &str,
    font_size: f32,
    script_style: Option<u32>,
    class: SimpleMathClass,
) -> Result<LaidOutMathAtom, LabelError> {
    let face =
        ttf_parser::Face::parse(&font.data, font.face_index).map_err(|_| LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to parse Typst math font".to_string(),
        })?;
    let Some(rusty) = rustybuzz::Face::from_slice(&font.data, font.face_index) else {
        return Err(LabelError::Engine {
            start: 0,
            end: text.len(),
            message: "failed to shape Typst math font".to_string(),
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
            pdf_run_group: None,
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
        left_class: class,
        right_class: class,
        italic_correction: atom_italic_correction,
        script_kernable: true,
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

fn layout_accent_atom(
    font: &MathFont,
    accent: char,
    font_size: f32,
    script_level: u8,
) -> Result<LaidOutMathAtom, LabelError> {
    layout_styled_atom_with_class(
        font,
        &accent.to_string(),
        font_size,
        script_style_feature(script_level),
        SimpleMathClass::Normal,
    )
}

fn atom_top_accent_attachment(font: &MathFont, atom: &LaidOutMathAtom) -> Result<f32, LabelError> {
    if atom.glyphs.len() == 1 && atom.shapes.is_empty() {
        let glyph = &atom.glyphs[0];
        if let Some(attachment) = top_accent_attachment(font, glyph)? {
            return Ok(attachment);
        }
    }
    Ok((atom.metrics.width + atom.italic_correction) / 2.0)
}

fn top_accent_attachment(font: &MathFont, glyph: &LaidOutGlyph) -> Result<Option<f32>, LabelError> {
    let face = parse_math_face(font, "top accent attachment")?;
    let Some(value) = face
        .tables()
        .math
        .and_then(|math| math.glyph_info)
        .and_then(|glyph_info| glyph_info.top_accent_attachments)
        .and_then(|attachments| attachments.get(glyph.glyph_id))
    else {
        return Ok(None);
    };

    Ok(Some(
        value.value as f32 * glyph.font_size / face.units_per_em() as f32,
    ))
}

fn script_style_feature(script_level: u8) -> Option<u32> {
    (script_level > 0).then_some(u32::from(script_level.min(2)))
}

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
        variations: Vec::new(),
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
                    dx: glyph.x,
                    dy: glyph.y,
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

fn path_artifact_from_simple_row(
    font: &MathFont,
    layout: &SimpleRowLayout,
    fill: crate::typst_library::Color,
) -> PathArtifact {
    let Ok(face) = ttf_parser::Face::parse(&font.data, font.face_index) else {
        return PathArtifact {
            logical_width: layout.metrics.width,
            logical_height: layout.metrics.height,
            items: Vec::new(),
            images: Vec::new(),
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
                    let path = outline_glyph_path(
                        &face,
                        glyph.glyph_id,
                        glyph.font_size,
                        glyph.x,
                        glyph.y,
                    );
                    if !path.commands.is_empty() {
                        items.push(PathItem {
                            path,
                            kind: PathKind::GlyphOutline {
                                glyph_run,
                                glyph_index: 0,
                            },
                            fill: Some(fill),
                            stroke: None,
                            transform: Transform::IDENTITY,
                            clip: None,
                        });
                    }
                    glyph_run += 1;
                }
                LaidOutDrawItem::Shape(index) => {
                    let Some(shape) = atom.shapes.get(index) else {
                        continue;
                    };
                    items.push(PathItem {
                        path: shape.path.clone(),
                        kind: PathKind::MathShape,
                        fill: None,
                        stroke: Some(Stroke {
                            color: shape.stroke.paint.unwrap_or(fill),
                            width: shape.stroke.width,
                            line_cap: shape.stroke.line_cap,
                            line_join: shape.stroke.line_join,
                            dash: shape.stroke.dash.clone(),
                        }),
                        transform: Transform {
                            dx: shape.x,
                            dy: shape.y,
                            ..Transform::IDENTITY
                        },
                        clip: None,
                    });
                }
            }
        }
    }

    PathArtifact {
        logical_width: layout.metrics.width,
        logical_height: layout.metrics.height,
        items,
        images: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::math::syntax::parse_math;
    use crate::types::MathOutputOptions;

    type LineSegment = ((f32, f32), (f32, f32));

    fn path_y_extent(item: &PathItem) -> f32 {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for command in &item.path.commands {
            let mut include = |y: f32| {
                let y = y + item.transform.dy;
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            };
            match *command {
                PathCommand::MoveTo { y, .. } | PathCommand::LineTo { y, .. } => include(y),
                PathCommand::QuadTo { y1, y, .. } => {
                    include(y1);
                    include(y);
                }
                PathCommand::CubicTo { y1, y2, y, .. } => {
                    include(y1);
                    include(y2);
                    include(y);
                }
                PathCommand::Close => {}
            }
        }
        if min_y.is_finite() && max_y.is_finite() {
            max_y - min_y
        } else {
            0.0
        }
    }

    fn cancel_shape_lines(source: &str, options: &MathLayoutOptions) -> Vec<LineSegment> {
        let math = parse_math(source, 0).unwrap();
        let artifact = try_typeset_simple_row_fragment(&math, options, &EngineOptions::default())
            .unwrap()
            .unwrap_or_else(|| panic!("cancel call should be handled: {source}"));
        artifact
            .paths
            .expect("cancel paths should exist")
            .items
            .into_iter()
            .filter(|item| matches!(item.kind, PathKind::MathShape))
            .filter_map(|item| match &item.path.commands[..] {
                [
                    PathCommand::MoveTo { x: x0, y: y0 },
                    PathCommand::LineTo { x: x1, y: y1 },
                ] => Some((
                    (x0 + item.transform.dx, y0 + item.transform.dy),
                    (x1 + item.transform.dx, y1 + item.transform.dy),
                )),
                _ => None,
            })
            .collect()
    }

    fn line_delta(line: LineSegment) -> (f32, f32) {
        (line.1.0 - line.0.0, line.1.1 - line.0.1)
    }

    fn line_length(line: LineSegment) -> f32 {
        let (dx, dy) = line_delta(line);
        dx.hypot(dy)
    }

    #[test]
    #[cfg(not(feature = "raster"))]
    fn atom_fragment_declines_raster_without_raster_feature() {
        let math = parse_math("1", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: false,
            raster: Some(crate::typst_render::RasterRequest::default()),
            pdf_text_layer: false,
        };

        assert!(
            try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn atom_fragment_can_emit_pdf_glyph_metadata() {
        let math = parse_math("1", 0).unwrap();
        let mut options = MathLayoutOptions::default();

        options.outputs = MathOutputOptions {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        assert!(
            try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
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
    fn atom_fragment_can_rasterize_from_typst_paths() {
        let math = parse_math("alpha + beta -> gamma", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: false,
            raster: Some(crate::typst_render::RasterRequest { scale: 2.0 }),
            pdf_text_layer: false,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple row should rasterize through Typst paths");

        assert!(artifact.paths.is_none());
        assert!(
            artifact
                .raster
                .as_ref()
                .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0)
        );
    }

    #[test]
    fn simple_row_can_emit_script_glyph_metadata() {
        let math = parse_math("x^2", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple superscript should be handled by Typst row path");
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 2);
        assert!(pdf.glyph_runs[1].font_size < pdf.glyph_runs[0].font_size);
    }

    #[test]
    fn simple_row_can_force_limits_and_scripts() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let limits = parse_math("limits(A)_1^2", 0).unwrap();
        let scripts = parse_math("scripts(A)_1^2", 0).unwrap();
        let display_limits = parse_math("limits(A, inline: #false)_1^2", 0).unwrap();

        let limits_metrics = layout_simple_row(&font, &limits, font_size)
            .unwrap()
            .expect("limits row should layout")
            .metrics;
        let scripts_metrics = layout_simple_row(&font, &scripts, font_size)
            .unwrap()
            .expect("scripts row should layout")
            .metrics;
        let display_limits_metrics = layout_simple_row(&font, &display_limits, font_size)
            .unwrap()
            .expect("display-only limits row should layout as scripts in labels")
            .metrics;

        assert!(
            limits_metrics.height > scripts_metrics.height + font_size * 0.4,
            "forced limits should create a taller inline label box"
        );
        assert!(
            limits_metrics.width < scripts_metrics.width,
            "forced limits should center top/bottom attachments instead of widening side scripts"
        );
        assert!(
            (display_limits_metrics.width - scripts_metrics.width).abs() < font_size * 0.05,
            "display-only limits should use side scripts in Avenger's inline label context"
        );
    }

    #[test]
    fn simple_row_relation_class_defaults_to_limits() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let relation_limits = parse_math("class(\"relation\", x)_a^b", 0).unwrap();
        let forced_scripts = parse_math("scripts(class(\"relation\", x))_a^b", 0).unwrap();
        let normal_scripts = parse_math("class(\"normal\", x)_a^b", 0).unwrap();

        let relation_metrics = layout_simple_row(&font, &relation_limits, font_size)
            .unwrap()
            .expect("relation class row should layout")
            .metrics;
        let forced_scripts_metrics = layout_simple_row(&font, &forced_scripts, font_size)
            .unwrap()
            .expect("forced scripts row should layout")
            .metrics;
        let normal_metrics = layout_simple_row(&font, &normal_scripts, font_size)
            .unwrap()
            .expect("normal class row should layout")
            .metrics;

        assert!(
            relation_metrics.height > forced_scripts_metrics.height + font_size * 0.25,
            "relation class should place top/bottom attachments as centered limits by default"
        );
        assert!(
            relation_metrics.width < forced_scripts_metrics.width,
            "relation class limits should avoid widening the row with a side script"
        );
        assert!(
            (normal_metrics.width - forced_scripts_metrics.width).abs() < font_size * 0.05,
            "normal class should keep ordinary side script placement"
        );
    }

    #[test]
    fn simple_row_can_emit_prime_glyphs_as_scripts() {
        let math = parse_math("a'''_b", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("prime attachment should be handled by Typst row path");
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text = pdf
            .glyph_runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text.matches(PRIME_CHAR).count(), 3);
        let max_font_size = pdf
            .glyph_runs
            .iter()
            .map(|run| run.font_size)
            .fold(0.0, f32::max);
        assert!(
            pdf.glyph_runs
                .iter()
                .any(|run| run.text.contains(PRIME_CHAR) && run.font_size < max_font_size)
        );
        let paths = artifact.paths.expect("prime paths should exist");
        assert!(paths.items.len() >= 5);
    }

    #[test]
    fn simple_row_can_emit_attach_call_with_corner_slots() {
        let base_math = parse_math("Pi", 0).unwrap();
        let attach_math = parse_math(
            "attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)",
            0,
        )
        .unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let base = try_typeset_simple_row_fragment(&base_math, &options, &EngineOptions::default())
            .unwrap()
            .expect("base should be handled by Typst row path");
        let artifact =
            try_typeset_simple_row_fragment(&attach_math, &options, &EngineOptions::default())
                .unwrap()
                .expect("attach call should be handled by Typst row path");

        assert!(artifact.metrics.width > base.metrics.width);
        let paths = artifact.paths.expect("attach paths should exist");
        assert!(paths.items.len() > base.paths.expect("base paths should exist").items.len());
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let glyph_count = pdf
            .glyph_runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>();
        assert!(glyph_count >= 10);
    }

    #[test]
    fn simple_row_can_emit_fraction_rule_paths() {
        let math = parse_math("a / (b + c)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple fraction should be handled by Typst row path");
        let paths = artifact.paths.expect("fraction paths should exist");
        assert!(
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        );
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_row_can_emit_frac_call_rule_paths() {
        let math = parse_math("frac(x + y, z)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple frac call should be handled by Typst row path");
        let paths = artifact.paths.expect("fraction paths should exist");
        assert!(
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        );
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_row_can_emit_frac_style_variants() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        for source in [
            "frac(x + y, z, style: \"skewed\")",
            "frac(x + y, z, style: \"horizontal\")",
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("{source} should be handled by Typst row path"));
            let paths = artifact.paths.expect("fraction paths should exist");
            assert!(
                paths
                    .items
                    .iter()
                    .all(|item| !matches!(item.kind, PathKind::MathShape)),
                "{source} should not emit a vertical fraction rule"
            );
            let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
            assert!(
                pdf.glyph_runs.len() >= 3,
                "{source} should emit numerator, slash, and denominator glyphs"
            );
        }
    }

    #[test]
    fn simple_row_can_emit_binom_paths_without_fraction_rule() {
        let math = parse_math("binom(n, k)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple binom call should be handled by Typst row path");
        let paths = artifact.paths.expect("binom paths should exist");
        assert_eq!(paths.items.len(), 4);
        assert!(
            paths
                .items
                .iter()
                .all(|item| !matches!(item.kind, PathKind::MathShape))
        );
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "(𝑛𝑘)");
    }

    #[test]
    fn simple_row_can_emit_variadic_binom_lower_terms() {
        let math = parse_math("binom(n, k_1, k_2, k_3)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("variadic binom call should be handled by Typst row path");
        let paths = artifact.paths.expect("binom paths should exist");
        assert!(
            paths
                .items
                .iter()
                .all(|item| !matches!(item.kind, PathKind::MathShape))
        );
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.contains("𝑘"));
        assert_eq!(text.matches(',').count(), 2);
    }

    #[test]
    fn simple_row_can_emit_cancel_overlay_path() {
        let math = parse_math("cancel(x)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple cancel call should be handled by Typst row path");
        let paths = artifact.paths.expect("cancel paths should exist");
        assert_eq!(paths.items.len(), 2);
        assert!(matches!(paths.items[0].kind, PathKind::GlyphOutline { .. }));
        assert!(matches!(paths.items[1].kind, PathKind::MathShape));
        assert!(paths.items[1].stroke.is_some());
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 1);
    }

    #[test]
    fn simple_row_cancel_honors_literal_geometry_options() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let default_line = cancel_shape_lines("cancel(x)", &options);
        let long_line = cancel_shape_lines("cancel(x, length: #200%)", &options);
        let horizontal_line = cancel_shape_lines("cancel(x, angle: #90deg)", &options);
        let vertical_line = cancel_shape_lines("cancel(x, angle: #0deg)", &options);
        let inverted_line = cancel_shape_lines("cancel(x, inverted: #true)", &options);
        let cross_lines = cancel_shape_lines("cancel(x, cross: #true)", &options);

        assert_eq!(default_line.len(), 1);
        assert_eq!(long_line.len(), 1);
        assert_eq!(horizontal_line.len(), 1);
        assert_eq!(vertical_line.len(), 1);
        assert_eq!(inverted_line.len(), 1);
        assert_eq!(cross_lines.len(), 2);

        assert!(
            line_length(long_line[0]) > line_length(default_line[0]) * 1.4,
            "length option should lengthen the cancel line"
        );
        assert!(
            line_delta(horizontal_line[0]).1.abs() < line_delta(horizontal_line[0]).0.abs() * 0.05,
            "90deg should make the cancel line nearly horizontal"
        );
        assert!(
            line_delta(vertical_line[0]).0.abs() < line_delta(vertical_line[0]).1.abs() * 0.05,
            "0deg should make the cancel line nearly vertical"
        );
        assert!(
            line_delta(default_line[0]).0.signum() != line_delta(inverted_line[0]).0.signum(),
            "inverted cancel should flip the horizontal direction"
        );
        assert!(
            line_delta(cross_lines[0]).0.signum() != line_delta(cross_lines[1]).0.signum(),
            "cross cancel should draw opposing lines"
        );
    }

    #[test]
    fn simple_row_cancel_honors_literal_stroke_options() {
        let math = parse_math(
            "cancel(x, stroke: #(thickness: 0.25em, paint: maroon, cap: \"round\", dash: \"dotted\"))",
            0,
        )
        .unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: false,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple cancel call should be handled by Typst row path");
        let paths = artifact.paths.expect("cancel paths should exist");
        let stroke = paths
            .items
            .iter()
            .find(|item| matches!(item.kind, PathKind::MathShape))
            .and_then(|item| item.stroke.as_ref())
            .expect("cancel stroke should exist");

        assert_eq!(stroke.color, Color::rgba(0.5, 0.0, 0.0, 1.0));
        assert!((stroke.width - options.style.font_size * 0.25).abs() < 1e-4);
        assert_eq!(stroke.line_cap, crate::typst_svg::StrokeCap::Round);
        assert_eq!(stroke.line_join, crate::typst_svg::StrokeJoin::Miter);
        let dash = stroke.dash.as_ref().expect("dash should be resolved");
        assert_eq!(dash.len(), 2);
        assert!((dash[0] - stroke.width).abs() < 1e-4);
        assert!((dash[1] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn simple_row_can_emit_sqrt_overbar_paths() {
        let math = parse_math("sqrt(x)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple sqrt should be handled by Typst row path");
        let paths = artifact.paths.expect("sqrt paths should exist");
        assert_eq!(paths.items.len(), 3);
        assert!(matches!(paths.items[1].kind, PathKind::MathShape));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 2);
    }

    #[test]
    fn simple_row_can_emit_math_underline_overline_paths() {
        let math = parse_math("overline(underline(x + y))", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple math underline/overline should be handled by Typst row path");
        let paths = artifact.paths.expect("line paths should exist");
        let shape_count = paths
            .items
            .iter()
            .filter(|item| matches!(item.kind, PathKind::MathShape))
            .count();
        assert_eq!(shape_count, 2);
        assert!(paths.items.iter().any(|item| item.stroke.is_some()));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 3);
    }

    #[test]
    fn simple_row_can_emit_indexed_root_paths() {
        let math = parse_math("root(3, x)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple indexed root should be handled by Typst row path");
        let paths = artifact.paths.expect("root paths should exist");
        assert_eq!(paths.items.len(), 4);
        assert!(matches!(paths.items[2].kind, PathKind::MathShape));
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 3);
        assert!(pdf.glyph_runs[0].font_size < pdf.glyph_runs[1].font_size);
    }

    #[test]
    fn simple_row_can_emit_visible_group_paths() {
        let math = parse_math("x(t)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple visible group should be handled by Typst row path");
        let paths = artifact.paths.expect("group paths should exist");
        assert_eq!(paths.items.len(), 4);
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "𝑥(𝑡)");
    }

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_can_rasterize_visible_group() {
        let math = parse_math("x(t)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: false,
            raster: Some(crate::typst_render::RasterRequest { scale: 2.0 }),
            pdf_text_layer: false,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple visible group should rasterize through Typst paths");
        assert!(
            artifact
                .raster
                .as_ref()
                .is_some_and(|raster| raster.image.width > 0 && raster.image.height > 0)
        );
    }

    #[test]
    fn simple_row_extends_identifier_subscript_with_adjacent_group() {
        let math = parse_math("J_n(x)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("identifier subscript group should be handled by Typst row path");
        let paths = artifact.paths.expect("group paths should exist");
        assert_eq!(paths.items.len(), 5);
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        assert_eq!(pdf.glyph_runs.len(), 5);
        assert!(
            pdf.glyph_runs[1..]
                .iter()
                .all(|run| run.font_size < pdf.glyph_runs[0].font_size)
        );
    }

    #[test]
    fn simple_row_can_emit_delimiter_helper_calls() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        for (source, expected) in [
            ("abs(x)", "|𝑥|"),
            ("norm(v)", "‖𝑣‖"),
            ("floor(x)", "⌊𝑥⌋"),
            ("ceil(x)", "⌈𝑥⌉"),
            ("round(x)", "⌊𝑥⌉"),
            ("ceil.l(x)", "⌈𝑥⌉"),
            ("floor.l(x)", "⌊𝑥⌋"),
            ("paren.l(x)", "(𝑥)"),
            ("brace.l(x)", "{𝑥}"),
            ("bracket.l(x)", "[𝑥]"),
            ("chevron.l(x)", "⟨𝑥⟩"),
            ("bar.double(x)", "‖𝑥‖"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("simple delimiter call should be handled: {source}"));
            let paths = artifact.paths.expect("delimiter call paths should exist");
            assert_eq!(paths.items.len(), 3, "{source}");
            let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_lr_delimited_call() {
        let math = parse_math("lr(|x + y|)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple lr call should be handled by Typst row path");
        let paths = artifact.paths.expect("lr paths should exist");
        assert_eq!(paths.items.len(), 5);
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "|𝑥+𝑦|");
    }

    #[test]
    fn simple_row_stretches_mid_delimiter_inside_lr() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let raw = parse_math("lr(| A | frac(1, 2) |)", 0).unwrap();
        let mid = parse_math("lr(| A mid(|) frac(1, 2) |)", 0).unwrap();
        let raw_artifact =
            try_typeset_simple_row_fragment(&raw, &options, &EngineOptions::default())
                .unwrap()
                .expect("raw middle delimiter row should be handled");
        let mid_artifact =
            try_typeset_simple_row_fragment(&mid, &options, &EngineOptions::default())
                .unwrap()
                .expect("mid delimiter row should be handled");

        let raw_paths = raw_artifact
            .paths
            .expect("raw delimiter paths should exist");
        let mid_paths = mid_artifact
            .paths
            .expect("mid delimiter paths should exist");
        assert!(raw_paths.items.len() >= 5);
        assert!(mid_paths.items.len() >= 5);

        let raw_middle_height = path_y_extent(&raw_paths.items[2]);
        let mid_middle_height = path_y_extent(&mid_paths.items[2]);
        assert!(
            mid_middle_height > raw_middle_height + 3.0,
            "mid delimiter should stretch to surrounding lr height: raw={raw_middle_height}, mid={mid_middle_height}"
        );

        let pdf = mid_artifact
            .pdf_text
            .expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.starts_with("|𝐴|1"));
        assert!(text.ends_with("2|"));
    }

    #[test]
    fn simple_row_applies_lr_delimiter_size_option() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let plain = parse_math("lr(|x|)", 0).unwrap();
        let sized = parse_math("lr(size: #240%, |x|)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain lr call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized lr call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 3.0,
            "explicit delimiter size should increase line height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(
            sized_artifact
                .paths
                .as_ref()
                .expect("sized lr paths should exist")
                .items
                .len(),
            3
        );
    }

    #[test]
    fn simple_row_applies_delimiter_helper_size_option() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let plain = parse_math("abs(x)", 0).unwrap();
        let sized = parse_math("abs(x, size: #2em)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain abs call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized abs call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 5.0,
            "explicit delimiter size should increase helper height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(
            sized_artifact
                .paths
                .as_ref()
                .expect("sized abs paths should exist")
                .items
                .len(),
            3
        );
    }

    #[test]
    fn simple_row_applies_callable_delimiter_symbol_size_option() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let plain = parse_math("bracket.l(x)", 0).unwrap();
        let sized = parse_math("bracket.l(x, size: #240%)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain bracket.l call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized bracket.l call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 3.0,
            "explicit delimiter size should increase callable symbol height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(
            sized_artifact
                .paths
                .as_ref()
                .expect("sized bracket.l paths should exist")
                .items
                .len(),
            3
        );
    }

    #[test]
    fn simple_row_can_emit_operator_calls() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        for (source, expected) in [
            ("sin(x)", "sin(𝑥)"),
            ("cos(theta)", "cos(𝜃)"),
            ("sech(x)", "sech(𝑥)"),
            ("liminf_(n -> oo)", "lim inf𝑛→∞"),
            ("Pr(X)", "Pr(𝑋)"),
            ("op(\"custom\")", "custom"),
            ("op(\"myop\", limits: #true)_n", "myop𝑛"),
            ("op(lt, limits: #false)", "<"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("operator call should be handled: {source}"));
            let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_operator_identifier_with_script() {
        let math = parse_math("lim_(x -> oo) f(x)", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("operator identifier with script should be handled by Typst row path");
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.starts_with("lim"));
        assert!(text.contains("∞"));
    }

    #[test]
    fn simple_row_can_emit_math_variant_calls() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        for (source, expected) in [
            ("bb(R)", "ℝ"),
            ("cal(P)", "𝒫"),
            ("scr(L)", "ℒ"),
            ("frak(g)", "𝔤"),
            ("sans(x)", "𝘹"),
            ("mono(123)", "𝟷𝟸𝟹"),
            ("bold(alpha + 2)", "𝜶+𝟐"),
            ("upright(R)", "R"),
            ("italic(R)", "𝑅"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("math variant call should be handled: {source}"));
            let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_vertical_stretch_call() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        let plain = parse_math("stretch(|)", 0).unwrap();
        let stretched = parse_math("stretch(|, size: #2em)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain vertical stretch call should be handled by Typst row path");
        let stretched_artifact =
            try_typeset_simple_row_fragment(&stretched, &options, &EngineOptions::default())
                .unwrap()
                .expect("stretched bar call should be handled by Typst row path");

        assert!(
            stretched_artifact.metrics.height > plain_artifact.metrics.height + 5.0,
            "vertical stretch should increase height: plain={:?}, stretched={:?}",
            plain_artifact.metrics,
            stretched_artifact.metrics
        );
        let paths = stretched_artifact
            .paths
            .expect("stretched bar paths should exist");
        assert_eq!(paths.items.len(), 1);
    }

    #[test]
    fn simple_row_can_emit_math_accent_calls() {
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: true,
            raster: None,
            pdf_text_layer: true,
        };

        for (source, expected) in [
            ("grave(a)", "𝑎\u{0300}"),
            ("acute(b)", "𝑏\u{0301}"),
            ("hat(x)", "𝑥\u{0302}"),
            ("hat(i)", "𝚤\u{0302}"),
            ("hat(dotless: #false, i)", "𝑖\u{0302}"),
            ("tilde(x)", "𝑥\u{0303}"),
            ("macron(x)", "𝑥\u{0304}"),
            ("dash(x)", "𝑥\u{0305}"),
            ("breve(x)", "𝑥\u{0306}"),
            ("dot(x)", "𝑥\u{0307}"),
            ("dot.double(x)", "𝑥\u{0308}"),
            ("ddot(x)", "𝑥\u{0308}"),
            ("dot.triple(x)", "𝑥\u{20db}"),
            ("dot.quad(x)", "𝑥\u{20dc}"),
            ("circle(x)", "𝑥\u{030a}"),
            ("acute.double(x)", "𝑥\u{030b}"),
            ("caron(x)", "𝑥\u{030c}"),
            ("arrow(v)", "𝑣\u{20d7}"),
            ("arrow.l(v)", "𝑣\u{20d6}"),
            ("arrow.l.r(v)", "𝑣\u{20e1}"),
            ("harpoon(v)", "𝑣\u{20d1}"),
            ("harpoon.lt(v)", "𝑣\u{20d0}"),
            ("accent(v, <-)", "𝑣\u{20d6}"),
            ("accent(v, \".\")", "𝑣\u{0307}"),
            ("accent(v, arrow.l.r)", "𝑣\u{20e1}"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("math accent call should be handled: {source}"));
            let paths = artifact.paths.expect("accent paths should exist");
            assert_eq!(paths.items.len(), 2, "{source}");
            let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_omits_script_group_delimiters() {
        let math = parse_math("sum_(i=0)^n i", 0).unwrap();
        let mut options = MathLayoutOptions::default();
        options.outputs = MathOutputOptions {
            paths: false,
            raster: None,
            pdf_text_layer: true,
        };

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple grouped script should be handled by Typst row path");
        let pdf = artifact.pdf_text.expect("PDF glyph metadata should exist");
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "∑𝑛𝑖=0𝑖");
    }

    #[test]
    fn atom_fragment_styles_latin_and_greek_as_math_italic() {
        let x = parse_math("x", 0).unwrap();
        let alpha = parse_math("alpha", 0).unwrap();

        assert_eq!(single_atom_text(&x).as_deref(), Some("𝑥"));
        assert_eq!(single_atom_text(&alpha).as_deref(), Some("𝛼"));
    }

    #[test]
    fn atom_fragment_keeps_numbers_and_operators_plain() {
        let number = parse_math("0.94", 0).unwrap();
        let plus = parse_math("+", 0).unwrap();
        let arrow = parse_math("->", 0).unwrap();

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

    #[test]
    fn simple_row_class_call_overrides_spacing_class() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let normal = parse_math("a ! b", 0).unwrap();
        let relation = parse_math("a class(\"relation\", !) b", 0).unwrap();

        let normal_width = layout_simple_row(&font, &normal, font_size)
            .unwrap()
            .expect("normal row should layout")
            .metrics
            .width;
        let relation_width = layout_simple_row(&font, &relation, font_size)
            .unwrap()
            .expect("class row should layout")
            .metrics
            .width;

        assert!(
            relation_width > normal_width + font_size * 0.1,
            "relation class should add more spacing than the default binary-vary operator"
        );
    }

    #[test]
    fn simple_row_math_size_calls_scale_body() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let inline = parse_math("inline(x)", 0).unwrap();
        let script = parse_math("script(x)", 0).unwrap();
        let sscript = parse_math("sscript(x)", 0).unwrap();

        let inline_width = layout_simple_row(&font, &inline, font_size)
            .unwrap()
            .expect("inline row should layout")
            .metrics
            .width;
        let script_width = layout_simple_row(&font, &script, font_size)
            .unwrap()
            .expect("script row should layout")
            .metrics
            .width;
        let sscript_width = layout_simple_row(&font, &sscript, font_size)
            .unwrap()
            .expect("sscript row should layout")
            .metrics
            .width;

        assert!(script_width < inline_width);
        assert!(sscript_width < script_width);
    }
}
