use avenger_color::ColorOrGradient;
use avenger_common::types::{SceneTextLeaderArrow, SceneTextLeaderShape};
use avenger_scales::{error::AvengerScaleError, scales::ConfiguredScale};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
        rule::SceneRuleMark,
        symbol::SceneSymbolMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_text::{
    text_edit::{cursor_rect_for_offset, selection_rects},
    types::{FontWeight, TextAlign, TextBaseline},
};

use crate::state::{annotation_config, Sample, State};

const INK: [f32; 4] = [0.12, 0.19, 0.26, 1.0];
const BLUE: [f32; 4] = [0.12, 0.45, 0.67, 1.0];

fn text(value: impl Into<String>, x: f32, y: f32, size: f32) -> SceneTextMark {
    SceneTextMark {
        text: value.into().into(),
        x: x.into(),
        y: y.into(),
        font_size: size.into(),
        color: ColorOrGradient::Color(INK).into(),
        interactive: false,
        ..Default::default()
    }
}
fn rect(name: &str, [x, y, width, height]: [f32; 4], fill: [f32; 4]) -> SceneRectMark {
    SceneRectMark {
        name: name.into(),
        interactive: !name.is_empty(),
        x: x.into(),
        y: y.into(),
        width: Some(width.into()),
        height: Some(height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        ..Default::default()
    }
}
fn rule(x: f32, y: f32, x2: f32, y2: f32, color: [f32; 4]) -> SceneRuleMark {
    SceneRuleMark {
        x: x.into(),
        y: y.into(),
        x2: x2.into(),
        y2: y2.into(),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: 1.0.into(),
        interactive: false,
        ..Default::default()
    }
}

fn axis_ticks(scale: &ConfiguredScale) -> Result<Vec<(f32, String)>, AvengerScaleError> {
    let ticks = scale.ticks(Some(5.0))?;
    let positions = scale.scale_to_numeric(&ticks)?;
    let labels = scale.format(&ticks)?;
    Ok(positions
        .as_iter_owned(ticks.len(), None)
        .zip(labels.as_iter_owned(ticks.len(), None))
        .collect())
}

pub fn build(state: &mut State) -> Result<SceneGraph, String> {
    state.scene_builds += 1;
    state.keep_caret_visible();
    let [width, height] = state.size;
    let [px, py, pw, ph] = state.plot();
    let mut marks: Vec<SceneMark> = vec![
        text("Annotation editor", 32.0, 47.0, 28.0).into(),
        text(
            "Edit Typst source to typeset a label, then drag the annotation into place.",
            32.0,
            77.0,
            15.0,
        )
        .into(),
        text(
            format!("Sample {}  ·  12 observations", state.sample.name()),
            60.0,
            121.0,
            16.0,
        )
        .into(),
        rect(
            "",
            [width - 304.0, 108.0, 288.0, height - 192.0],
            [0.95, 0.965, 0.98, 1.0],
        )
        .into(),
    ];
    for (sample, name, x) in [
        (Sample::A, "sample-a", width - 264.0),
        (Sample::B, "sample-b", width - 144.0),
    ] {
        let active = state.sample == sample;
        marks.push(
            rect(
                name,
                [x, 32.0, 112.0, 38.0],
                if active { BLUE } else { [0.9, 0.93, 0.95, 1.0] },
            )
            .into(),
        );
        let mut label = text(format!("Sample {}", sample.name()), x + 20.0, 57.0, 16.0);
        if active {
            label.color = ColorOrGradient::Color([1.0; 4]).into();
        }
        marks.push(label.into());
    }
    let mut plot_marks: Vec<SceneMark> =
        vec![rect("plot", state.plot(), [0.98, 0.986, 0.993, 1.0]).into()];
    let scales = state.scales().map_err(|e| e.to_string())?;
    for (x, value) in axis_ticks(&scales.x).map_err(|e| e.to_string())? {
        let mut grid = rule(x, py, x, py + ph, [0.87, 0.9, 0.93, 1.0]);
        grid.name = "x-grid".into();
        grid.clip = true;
        plot_marks.push(grid.into());
        let mut label = text(value, x, py + ph + 23.0, 12.0);
        label.name = "x-tick-label".into();
        label.align = TextAlign::Center.into();
        marks.push(label.into());
    }
    for (y, value) in axis_ticks(&scales.y).map_err(|e| e.to_string())? {
        let mut grid = rule(px, y, px + pw, y, [0.87, 0.9, 0.93, 1.0]);
        grid.name = "y-grid".into();
        grid.clip = true;
        plot_marks.push(grid.into());
        let mut label = text(value, px - 12.0, y, 12.0);
        label.name = "y-tick-label".into();
        label.align = TextAlign::Right.into();
        label.baseline = TextBaseline::Middle.into();
        marks.push(label.into());
    }
    for (i, point) in state.points.iter().enumerate() {
        let [x, y] = scales.position(point.position).map_err(|e| e.to_string())?;
        plot_marks.push(
            SceneSymbolMark {
                name: format!("point-{i}"),
                x: x.into(),
                y: y.into(),
                size: if i == state.selected { 160.0 } else { 90.0 }.into(),
                fill: ColorOrGradient::Color(if i == state.selected {
                    [0.99, 0.65, 0.2, 1.0]
                } else {
                    BLUE
                })
                .into(),
                stroke: ColorOrGradient::Color([1.0; 4]).into(),
                stroke_width: Some(1.5),
                clip: true,
                // One mark per point keeps the target index stable across scene rebuilds.
                ..Default::default()
            }
            .into(),
        );
        if !point.annotation.trim().is_empty() {
            let config = annotation_config(&point.annotation);
            let mut label = text(point.annotation.clone(), x, y, config.font_size);
            label.text_syntax = config.syntax_mode;
            label.font = config.font.to_string().into();
            label.font_weight = config.font_weight.into();
            label.font_style = config.font_style.into();
            label.name = format!("annotation-{i}");
            label.interactive = true;
            label.clip = true;
            label.dx = point.offset[0].into();
            label.dy = point.offset[1].into();
            label.leader = true.into();
            label.leader_shape = SceneTextLeaderShape::Curved.into();
            label.leader_arrow = SceneTextLeaderArrow::Triangle.into();
            label.leader_stroke = ColorOrGradient::Color(INK).into();
            label.leader_stroke_width = 1.2.into();
            label.leader_target_radius = 8.0.into();
            label.leader_label_padding = 5.0.into();
            plot_marks.push(label.into());
        }
    }
    marks.push(
        SceneGroup {
            clip: Clip::Rect {
                x: px,
                y: py,
                width: pw,
                height: ph,
            },
            marks: plot_marks,
            ..Default::default()
        }
        .into(),
    );
    let ix = width - 284.0;
    marks.push(text("SELECTED POINT", ix, 144.0, 12.0).into());
    let mut title = text(state.points[state.selected].name.clone(), ix, 178.0, 23.0);
    title.font_weight = FontWeight::Number(700.0).into();
    marks.push(title.into());
    let p = state.points[state.selected].position;
    marks.push(
        text(
            format!("x = {:.0}     y = {:.0}", p[0], p[1]),
            ix,
            208.0,
            15.0,
        )
        .into(),
    );
    marks.push(text("Typst source", ix, 240.0, 14.0).into());
    let mut field = rect("field", state.field(), [1.0; 4]);
    field.stroke = ColorOrGradient::Color(if state.focused {
        BLUE
    } else {
        [0.7, 0.76, 0.82, 1.0]
    })
    .into();
    field.stroke_width = if state.focused { 2.0 } else { 1.0 }.into();
    marks.push(field.into());
    let line = state.shaped_line()?;
    let [tx, ty] = state.field_text_origin();
    let mut field_marks: Vec<SceneMark> = Vec::new();
    for r in selection_rects(&line, state.editor.normalized_selection()) {
        field_marks.push(
            rect(
                "",
                [tx + r.x, ty + r.y, r.width, r.height],
                [0.72, 0.84, 0.94, 1.0],
            )
            .into(),
        );
    }
    let mut value = text(state.editor.text(), tx, ty + line.baseline, 17.0);
    value.baseline = TextBaseline::Alphabetic.into();
    value.clip = true;
    field_marks.push(value.into());
    if let Some(range) = state.editor.compose_range() {
        for r in selection_rects(&line, range) {
            field_marks.push(
                rule(
                    tx + r.x,
                    ty + r.y + r.height,
                    tx + r.x + r.width,
                    ty + r.y + r.height,
                    BLUE,
                )
                .into(),
            );
        }
    }
    if state.focused && state.caret_visible && state.editor.show_cursor() {
        let head = state.editor.selection().head;
        let r = cursor_rect_for_offset(&line, head.index, head.affinity);
        field_marks.push(rect("", [tx + r.x, ty + r.y, 1.2, r.height], INK).into());
    }
    // All field drawing shares the horizontal scroll and clip used for hit testing and IME.
    for mark in &mut field_marks {
        match mark {
            SceneMark::Rect(mark) => mark.clip = true,
            SceneMark::Rule(mark) => mark.clip = true,
            _ => {}
        }
    }
    let [fx, fy, fw, fh] = state.field();
    marks.push(
        SceneGroup {
            interactive: false,
            clip: Clip::Rect {
                x: fx + 5.0,
                y: fy + 4.0,
                width: fw - 10.0,
                height: fh - 8.0,
            },
            marks: field_marks,
            ..Default::default()
        }
        .into(),
    );
    let status = if state.editor.compose_range().is_some() {
        "Composing…"
    } else if state.annotation_error.is_some() {
        "Invalid markup · preview unchanged"
    } else if state.pending() {
        "Waiting to apply…"
    } else {
        "Applied"
    };
    let mut status_label = text(status, ix, 325.0, 14.0);
    if let Some(error) = &state.annotation_error {
        let color = ColorOrGradient::Color([0.7, 0.16, 0.12, 1.0]);
        status_label.color = color.clone().into();
        let mut detail = text(error.replace(['\n', '\r'], " "), ix, 348.0, 11.0);
        detail.color = color.into();
        detail.limit = 245.0.into();
        marks.push(detail.into());
    }
    marks.push(status_label.into());
    for (i, line) in [
        "Use *bold*, _italic_, or $sqrt(x)$.",
        "Labels update after you pause.",
        "Enter applies and leaves the field.",
        "Escape restores the applied label.",
        "Use your usual clipboard shortcuts.",
        "Edits last until you change sample.",
    ]
    .iter()
    .enumerate()
    {
        marks.push(text(*line, ix, 372.0 + i as f32 * 25.0, 13.0).into());
    }
    if let Some(sample) = state.loading {
        marks.push(
            text(
                format!("Loading sample {}…", sample.name()),
                ix,
                528.0,
                14.0,
            )
            .into(),
        );
    }
    if let Some(error) = &state.error {
        let mut label = text(error, ix, 555.0, 12.0);
        label.limit = 245.0.into();
        marks.push(label.into());
    }
    marks.push(text("Hover for details  ·  Drag a label to reposition it  ·  Drag the plot background to pan",32.0,height-32.0,14.0).into());
    Ok(SceneGraph {
        width,
        height,
        origin: [0.0; 2],
        marks,
    })
}
