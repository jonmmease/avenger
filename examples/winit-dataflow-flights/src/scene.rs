use crate::{
    controller::State,
    dataflow::{Evaluation, Metadata, batch, number, strings},
    layout::DashboardLayout,
    picking::{PALETTE, Points},
};
use anyhow::{Context, Result};
use avenger_color::ColorOrGradient;
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::{
    numeric::make_numeric_axis_marks_with_text_engine,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_panels::Rect;
use avenger_scales::scales::linear::LinearScale;
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use avenger_text::TextEngine;
use avenger_widgets::prelude::*;
use std::{collections::BTreeMap, sync::Arc};

const INK: [f32; 4] = [0.12, 0.19, 0.25, 1.];
const MUTED: [f32; 4] = [0.40, 0.46, 0.51, 1.];
const BLUE: [f32; 4] = [0.12, 0.43, 0.62, 1.];
#[derive(Clone)]
pub struct Bar {
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub count: usize,
}
pub struct Rendered {
    pub selections: crate::selection::Selections,
    pub layout: DashboardLayout,
    pub points: Arc<Points>,
    pub airlines: Vec<(String, usize)>,
    pub airline_positions: Vec<(f32, f32, usize)>,
    pub histograms: BTreeMap<String, Vec<Bar>>,
    pub selected: usize,
    pub mean: Option<f64>,
    pub elapsed_ms: f64,
    pub strategy: String,
}
impl Rendered {
    pub fn new(
        evaluation: &Evaluation,
        metadata: &Metadata,
        layout: DashboardLayout,
        previous: Option<&Rendered>,
        selections: crate::selection::Selections,
    ) -> Result<Self> {
        let t = evaluation.tables_for(metadata)?;
        let points = if let Some(old) = previous.filter(|p| p.points.id == t.scatter.id()) {
            old.points.clone()
        } else {
            Points::new(&t.scatter, metadata)?
        };
        let airline_batch = batch(&t.airlines)?;
        let airlines = strings(&t.airlines, "carrier")?
            .into_iter()
            .enumerate()
            .map(|(i, c)| Ok((c, number(&airline_batch, "count", i)?.unwrap() as usize)))
            .collect::<Result<Vec<_>>>()?;
        let airline_positions = (0..airline_batch.num_rows())
            .map(|i| {
                Ok((
                    number(&airline_batch, "width", i)?.unwrap() as f32,
                    number(&airline_batch, "y", i)?.unwrap() as f32,
                    number(&airline_batch, "color", i)?.unwrap() as usize,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut histograms = BTreeMap::new();
        for (dest, t) in &t.panels {
            let b = batch(t)?;
            let bars = (0..b.num_rows())
                .map(|i| {
                    Ok(Bar {
                        x0: number(&b, "x0", i)?.unwrap() as f32,
                        x1: number(&b, "x1", i)?.unwrap() as f32,
                        y: number(&b, "y", i)?.unwrap() as f32,
                        count: number(&b, "count", i)?.unwrap() as usize,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            histograms.insert(dest.clone(), bars);
        }
        let (selected, mean) = evaluation.summary()?;
        Ok(Self {
            selections,
            layout,
            points,
            airlines,
            airline_positions,
            histograms,
            selected,
            mean,
            elapsed_ms: evaluation.elapsed.as_secs_f64() * 1000.,
            strategy: evaluation.strategy.clone(),
        })
    }
}
pub fn text(value: impl Into<String>, x: f32, y: f32, size: f32, color: [f32; 4]) -> SceneMark {
    SceneTextMark {
        text: value.into().into(),
        x: x.into(),
        y: y.into(),
        font_size: size.into(),
        color: ColorOrGradient::Color(color).into(),
        interactive: false,
        ..Default::default()
    }
    .into()
}
fn rect(name: &str, r: Rect, fill: [f32; 4], stroke: Option<[f32; 4]>) -> SceneMark {
    SceneRectMark {
        name: name.into(),
        x: r.x.into(),
        y: r.y.into(),
        width: Some(r.width.into()),
        height: Some(r.height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        stroke: ColorOrGradient::Color(stroke.unwrap_or([0.; 4])).into(),
        stroke_width: if stroke.is_some() { 1_f32 } else { 0. }.into(),
        interactive: !name.is_empty(),
        ..Default::default()
    }
    .into()
}
fn axes(
    engine: &TextEngine,
    r: Rect,
    domains: [[f32; 2]; 2],
    x_title: &str,
    y_title: &str,
    x_labels: bool,
) -> Result<Vec<SceneMark>> {
    let mut groups = vec![];
    for (domain, range, orientation, title) in [
        (domains[0], [0., r.width], AxisOrientation::Bottom, x_title),
        (domains[1], [r.height, 0.], AxisOrientation::Left, y_title),
    ] {
        let scale = LinearScale::configured((domain[0], domain[1]), (range[0], range[1]));
        let group = make_numeric_axis_marks_with_text_engine(
            &scale,
            title,
            [r.x, r.y],
            &AxisConfig {
                labels_visible: Some(!matches!(orientation, AxisOrientation::Bottom) || x_labels),
                orientation,
                dimensions: [r.width, r.height],
                tick_count: Some(
                    ((range[1] - range[0]).abs()
                        / if matches!(orientation, AxisOrientation::Left) {
                            35.
                        } else {
                            100.
                        })
                    .clamp(1., 6.)
                    .min((domain[1] - domain[0]).abs().max(1.)),
                ),
                format_number: Some(",.0f".into()),
                grid: true,
                grid_color: Some([0.90, 0.93, 0.95, 1.]),
                label_color: Some(MUTED),
                label_font_size: Some(11.),
                title_font_size: Some(12.),
                title_color: Some(MUTED),
                ..Default::default()
            },
            engine,
        )?;
        let bounds = group.bounding_box_with_text_engine(engine);
        anyhow::ensure!(
            bounds
                .lower()
                .iter()
                .chain(bounds.upper().iter())
                .all(|x| x.is_finite()),
            "non-finite guide bounds"
        );
        groups.push(group.into());
    }
    Ok(groups)
}
pub fn build(state: &mut State) -> Result<avenger_app::app::SceneBuild> {
    let r = &state.rendered;
    let layout = &r.layout;
    let mut marks = vec![rect(
        "",
        Rect::new(0., 0., state.size[0], state.size[1]),
        [0.98, 0.985, 0.99, 1.],
        None,
    )];
    marks.push(text("FLIGHT DELAY EXPLORER", 24., 31., 23., INK));
    marks.push(text(
        format!(
            "NYC · 2013    {} flights drawn · {} missing-delay rows excluded",
            comma(r.points.count),
            comma(state.metadata.source_rows - state.metadata.eligible_rows)
        ),
        24.,
        54.,
        12.,
        MUTED,
    ));
    marks.push(text(
        format!(
            "{} selected    Mean arrival delay: {}",
            comma(r.selected),
            r.mean.map_or("—".into(), |n| format!("{n:.1} min"))
        ),
        24.,
        83.,
        17.,
        BLUE,
    ));
    marks.push(text(
        format!(
            "Preaggregation: {} · {}",
            state.config.mode(),
            if state.config.exact {
                "exact bounds"
            } else {
                "2 px cells"
            }
        ),
        state.size[0] - 330.,
        31.,
        12.,
        MUTED,
    ));
    marks.push(text(
        if let Some(error) = &state.error {
            error.clone()
        } else if state.pending {
            "Updating…".into()
        } else {
            format!("{:.0} ms · {}", r.elapsed_ms, r.strategy)
        },
        24.,
        state.size[1] - 15.,
        11.,
        MUTED,
    ));
    let scatter = layout.scatter;
    marks.push(rect("", scatter, [1.; 4], Some([0.85, 0.89, 0.92, 1.])));
    marks.extend(axes(
        &state.engine,
        scatter,
        state.metadata.domains,
        "Departure delay (min)",
        "Arrival delay (min)",
        true,
    )?);
    marks.push(text(
        "All eligible flights · drag to brush",
        scatter.x,
        scatter.y - 16.,
        13.,
        INK,
    ));
    marks.push(
        SceneGroup {
            origin: [scatter.x, scatter.y],
            clip: Clip::Rect {
                x: 0.,
                y: 0.,
                width: scatter.width,
                height: scatter.height,
            },
            marks: r.points.marks.iter().cloned().map(Into::into).collect(),
            ..Default::default()
        }
        .into(),
    );
    marks.push(rect("scatter-hit", scatter, [0.; 4], None));
    if let Some(bounds) = state.preview.or(r.selections.raw_brush) {
        let extent = state.brush_rect(bounds);
        marks.push(rect("", extent, [0.12, 0.43, 0.62, 0.13], Some(BLUE)));
    }
    let airline = layout.airline;
    marks.push(text(
        "Airlines · click to include / exclude",
        airline.x - 25.,
        airline.y - 16.,
        13.,
        INK,
    ));
    let row_height = airline.height / r.airlines.len().max(1) as f32;
    for (i, (carrier, count)) in r.airlines.iter().enumerate() {
        let (width, offset, color_index) = r.airline_positions[i];
        let y = airline.y + offset;
        let mut color = PALETTE[color_index % PALETTE.len()];
        color[3] = if r.selections.carriers.contains(carrier) {
            0.90
        } else {
            0.20
        };
        marks.push(rect(
            &format!("airline-{carrier}"),
            Rect::new(airline.x - 30., y, airline.width + 30., row_height),
            [0.; 4],
            None,
        ));
        marks.push(rect(
            "",
            Rect::new(airline.x, y + 2., width, (row_height - 4.).max(1.)),
            color,
            None,
        ));
        marks.push(text(
            carrier,
            airline.x - 28.,
            y + row_height * 0.72,
            11_f32.min(row_height * 0.8),
            INK,
        ));
        marks.push(text(
            comma(*count),
            airline.x + airline.width - 48.,
            y + row_height * 0.72,
            10_f32.min(row_height * 0.8),
            MUTED,
        ));
    }
    for (dest, plot) in &layout.panels {
        let bars = r.histograms.get(dest).context("destination histogram")?;
        let max = bars.iter().map(|b| b.count).max().unwrap_or(1).max(1);
        marks.push(text(
            format!(
                "{dest}  ·  {} selected",
                comma(bars.iter().map(|b| b.count).sum())
            ),
            plot.x,
            plot.y - 21.,
            13.,
            INK,
        ));
        marks.extend(axes(
            &state.engine,
            *plot,
            [[0., 24.], [0., max as f32]],
            if layout
                .guides
                .decision(&"departure-labels".into(), &dest.clone().into())
                .and_then(|d| d.instance())
                .is_some_and(|i| i.anchor() == &avenger_panels::NodeId::Panel(dest.clone().into()))
            {
                "Scheduled departure hour"
            } else {
                ""
            },
            "",
            layout
                .guides
                .decision(&"departure-labels".into(), &dest.clone().into())
                .and_then(|d| d.instance())
                .is_some_and(|i| i.anchor() == &avenger_panels::NodeId::Panel(dest.clone().into())),
        )?);
        marks.push(
            SceneGroup {
                origin: [plot.x, plot.y],
                clip: Clip::Rect {
                    x: 0.,
                    y: 0.,
                    width: plot.width,
                    height: plot.height,
                },
                marks: bars
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        let mut mark = SceneRectMark {
                            name: format!("histogram-{dest}-{i}"),
                            x: (b.x0 + 0.5).into(),
                            y: b.y.into(),
                            width: Some((b.x1 - b.x0 - 1.).max(0.5).into()),
                            height: Some((plot.height - b.y).max(0.).into()),
                            fill: ColorOrGradient::Color([0.18, 0.52, 0.66, 0.90]).into(),
                            clip: true,
                            ..Default::default()
                        };
                        mark.interactive = true;
                        mark.into()
                    })
                    .collect(),
                ..Default::default()
            }
            .into(),
        );
    }
    let mut specs: Vec<WidgetSpec> = vec![
        Button::new("reset", "Reset").into(),
        Button::new("all", "All airlines").into(),
        Button::new("none", "No airlines").into(),
    ];
    for dest in &state.metadata.destinations {
        specs.push(
            Button::new(
                format!("bin-{dest}"),
                format!("{} min", state.bins.get(dest).copied().unwrap_or(30)),
            )
            .into(),
        );
    }
    let mut theme = WidgetTheme::light();
    theme.button.text.size = 11.;
    let mut prepared = state.widgets.prepare(&specs, &theme, &state.engine)?;
    let mut x = state.size[0] - 330.;
    for id in ["reset", "all", "none"] {
        let m = prepared.metrics(id).unwrap().preferred;
        prepared.place(id, Rect::new(x, 62., m.width, m.height), None)?;
        x += m.width + 8.;
    }
    for (dest, r) in &layout.panels {
        let id = format!("bin-{dest}");
        let m = prepared.metrics(id.clone()).unwrap().preferred;
        prepared.place(
            id,
            Rect::new(r.x + r.width - m.width, r.y - 39., m.width, m.height),
            None,
        )?;
    }
    let frame = prepared.finish()?;
    marks.push(frame.scene.clone().into());
    let update = state.widgets.install(frame)?;
    Ok(avenger_app::app::SceneBuild {
        scene_graph: SceneGraph {
            width: state.size[0],
            height: state.size[1],
            origin: [0., 0.],
            marks: vec![
                SceneGroup {
                    marks,
                    ..Default::default()
                }
                .into(),
            ],
        },
        commands: update.status.commands,
        rebuild_geometry: update.status.rebuild_geometry,
    })
}
pub fn comma(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}
