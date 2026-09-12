use crate::state::State;
use avenger_app::app::SceneBuild;
use avenger_color::ColorOrGradient;
use avenger_scales::scales::{ConfiguredScale, linear::LinearScale};
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
use avenger_text::types::{TextAlign, TextBaseline, TextSyntaxMode};
use avenger_widgets::{Rect, WidgetTheme};
const INK: [f32; 4] = [0.08, 0.13, 0.20, 1.0];
fn label(value: impl Into<String>, x: f32, y: f32, size: f32) -> SceneTextMark {
    SceneTextMark {
        interactive: false,
        text: value.into().into(),
        x: x.into(),
        y: y.into(),
        font_size: size.into(),
        color: ColorOrGradient::Color(INK).into(),
        ..Default::default()
    }
}
fn box_mark(name: &str, r: Rect, color: [f32; 4]) -> SceneRectMark {
    SceneRectMark {
        name: name.into(),
        interactive: !name.is_empty(),
        x: r.x.into(),
        y: r.y.into(),
        width: Some(r.width.into()),
        height: Some(r.height.into()),
        fill: ColorOrGradient::Color(color).into(),
        ..Default::default()
    }
}
fn line(x: f32, y: f32, x2: f32, y2: f32, color: [f32; 4]) -> SceneRuleMark {
    SceneRuleMark {
        interactive: false,
        clip: true,
        x: x.into(),
        y: y.into(),
        x2: x2.into(),
        y2: y2.into(),
        stroke: ColorOrGradient::Color(color).into(),
        stroke_width: 1.0.into(),
        ..Default::default()
    }
}
fn ticks(scale: &ConfiguredScale) -> Result<Vec<(f32, String)>, String> {
    let values = scale.ticks(Some(5.0)).map_err(|e| e.to_string())?;
    let positions = scale.scale_to_numeric(&values).map_err(|e| e.to_string())?;
    let labels = scale.format(&values).map_err(|e| e.to_string())?;
    Ok(positions
        .as_iter_owned(values.len(), None)
        .zip(labels.as_iter_owned(values.len(), None))
        .collect())
}

pub fn build(state: &mut State) -> Result<SceneBuild, String> {
    let [width, height] = state.size;
    let plot = state.plot();
    let sx = LinearScale::configured((0.0, 100.0), (plot.x, plot.x + plot.width))
        .pan(state.pan[0] / plot.width)
        .map_err(|e| e.to_string())?;
    let sy = LinearScale::configured((0.0, 100.0), (plot.y + plot.height, plot.y))
        .pan(-state.pan[1] / plot.height)
        .map_err(|e| e.to_string())?;
    let mut marks: Vec<SceneMark> = vec![
        box_mark("", Rect::new(0.0, 0.0, width, height), [1.0; 4]).into(),
        label("Plot Style Studio", 32.0, 45.0, 28.0).into(),
        label(
            "Choose a palette, change the points, and edit the annotation.",
            32.0,
            74.0,
            15.0,
        )
        .into(),
        label(&state.title, plot.x, 130.0, 23.0).into(),
        box_mark(
            "",
            Rect::new(width - 376.0, 98.0, 348.0, height - 126.0),
            [0.965, 0.974, 0.985, 1.0],
        )
        .into(),
    ];
    let mut plot_marks: Vec<SceneMark> =
        vec![box_mark("plot", plot, [0.985, 0.991, 1.0, 1.0]).into()];
    for (x, value) in ticks(&sx)? {
        if state.layers.contains(&"grid".into()) {
            plot_marks
                .push(line(x, plot.y, x, plot.y + plot.height, [0.83, 0.87, 0.92, 1.0]).into());
        }
        let mut tick = label(value, x, plot.y + plot.height + 24.0, 12.0);
        tick.align = TextAlign::Center.into();
        marks.push(tick.into());
    }
    for (y, value) in ticks(&sy)? {
        if state.layers.contains(&"grid".into()) {
            plot_marks
                .push(line(plot.x, y, plot.x + plot.width, y, [0.83, 0.87, 0.92, 1.0]).into());
        }
        let mut tick = label(value, plot.x - 12.0, y, 12.0);
        tick.align = TextAlign::Right.into();
        tick.baseline = TextBaseline::Middle.into();
        marks.push(tick.into());
    }
    let palette = match state.palette.as_str() {
        "sunset" => [
            [0.80, 0.28, 0.12, 1.0],
            [0.95, 0.57, 0.13, 1.0],
            [0.58, 0.20, 0.38, 1.0],
        ],
        "mono" => [
            [0.19, 0.24, 0.30, 1.0],
            [0.36, 0.42, 0.49, 1.0],
            [0.54, 0.60, 0.65, 1.0],
        ],
        _ => [
            [0.08, 0.36, 0.63, 1.0],
            [0.05, 0.58, 0.64, 1.0],
            [0.25, 0.38, 0.73, 1.0],
        ],
    };
    if state.layers.contains(&"points".into()) {
        for i in 0..42 {
            let x = 8.0 + (i % 14) as f32 * 6.2;
            let y = 18.0
                + (i % 14) as f32 * 4.2
                + (i / 14) as f32 * 8.0
                + ((i * 17 % 11) as f32 - 5.0) * 2.1;
            let px = sx
                .scale_scalar(&x)
                .and_then(|v| v.as_f32())
                .map_err(|e| e.to_string())?;
            let py = sy
                .scale_scalar(&y)
                .and_then(|v| v.as_f32())
                .map_err(|e| e.to_string())?;
            let mut color = palette[i / 14];
            color[3] = state.opacity as f32;
            plot_marks.push(
                SceneSymbolMark {
                    interactive: false,
                    clip: true,
                    x: px.into(),
                    y: py.into(),
                    size: ((state.marker_size * 2.0).powi(2) as f32).into(),
                    fill: ColorOrGradient::Color(color).into(),
                    stroke: ColorOrGradient::Color([1.0, 1.0, 1.0, 0.85]).into(),
                    stroke_width: Some(1.0),
                    ..Default::default()
                }
                .into(),
            );
        }
    }
    if state.layers.contains(&"annotations".into()) {
        let mut annotation = label(&state.accepted_source, plot.x + 28.0, plot.y + 48.0, 18.0);
        annotation.clip = true;
        annotation.text_syntax = TextSyntaxMode::TypstMarkup;
        annotation.color = ColorOrGradient::Color(palette[0]).into();
        plot_marks.push(annotation.into());
    }
    marks.push(
        SceneGroup {
            interactive: false,
            clip: Clip::Rect {
                x: plot.x,
                y: plot.y,
                width: plot.width,
                height: plot.height,
            },
            marks: plot_marks,
            ..Default::default()
        }
        .into(),
    );
    let mut xlabel = label(
        "Input",
        plot.x + plot.width / 2.0,
        plot.y + plot.height + 50.0,
        14.0,
    );
    xlabel.align = TextAlign::Center.into();
    marks.push(xlabel.into());
    marks.push(
        label(
            "Drag the plot to pan. Grid lines follow the scale ticks.",
            plot.x,
            height - 34.0,
            13.0,
        )
        .into(),
    );
    let x = width - 356.0;
    marks.push(label("Style controls", x, 123.0, 18.0).into());
    let specs = state.controls();
    let theme = WidgetTheme::light();
    let mut prepared = state
        .widgets
        .prepare(&specs, &theme, &state.engine)
        .map_err(|e| e.to_string())?;
    let mut y = 144.0;
    for spec in &specs {
        let id = spec.id().as_str();
        let caption = match id {
            "size" => Some("Marker size"),
            "opacity" => Some("Opacity"),
            "title" => Some("Plot title"),
            "source" => Some("Annotation source · Typst"),
            _ => None,
        };
        if let Some(caption) = caption {
            marks.push(label(caption, x, y + 12.0, 13.0).into());
            y += 22.0;
        }
        let h = prepared
            .metrics(spec.id().clone())
            .unwrap()
            .preferred
            .height;
        prepared
            .place(spec.id().clone(), Rect::new(x, y, 308.0, h), None)
            .map_err(|e| e.to_string())?;
        y += h + 12.0;
        if id == "source" {
            let message = if state.annotation_error.is_some() {
                "Invalid markup · showing the last valid annotation"
            } else if state.source != state.accepted_source {
                "Waiting to typeset…"
            } else {
                "Annotation is up to date"
            };
            let mut status = label(message, x, y + 2.0, 11.0);
            if state.annotation_error.is_some() {
                status.color = ColorOrGradient::Color([0.70, 0.10, 0.08, 1.0]).into();
            }
            marks.push(status.into());
            y += 22.0;
        }
    }
    marks.push(label(&state.last_action, x, height - 44.0, 12.0).into());
    let frame = prepared.finish().map_err(|e| e.to_string())?;
    marks.push(frame.scene.clone().into());
    let scene_graph = SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    };
    let update = state.widgets.install(frame).map_err(|e| e.to_string())?;
    #[cfg(target_arch = "wasm32")]
    crate::web::record(state);
    Ok(SceneBuild {
        scene_graph,
        commands: update.status.commands,
        rebuild_geometry: update.status.rebuild_geometry,
    })
}
