use crate::{controller::State, layout::SIZE, selection::PLOTS};
use anyhow::Result;
use arrow::{
    array::{ArrayRef, AsArray, Float32Array},
    datatypes::Float32Type,
};
use avenger_color::ColorOrGradient;
use avenger_guides::axis::{
    numeric::make_numeric_axis_marks_with_text_engine,
    opts::{AxisConfig, AxisOrientation},
};
use avenger_panels::Rect;
use avenger_scales::scales::{ConfiguredScale, linear::LinearScale};
use avenger_scenegraph::{
    marks::{
        group::{Clip, SceneGroup},
        mark::SceneMark,
        rect::SceneRectMark,
        text::SceneTextMark,
    },
    scene_graph::SceneGraph,
};
use std::sync::Arc;

fn text(value: impl Into<String>, x: f32, y: f32, size: f32) -> SceneMark {
    SceneTextMark {
        text: value.into().into(),
        x: x.into(),
        y: y.into(),
        font_size: size.into(),
        interactive: false,
        ..Default::default()
    }
    .into()
}
fn rectangle(name: &str, r: Rect, fill: [f32; 4]) -> SceneRectMark {
    SceneRectMark {
        name: name.into(),
        x: r.x.into(),
        y: r.y.into(),
        width: Some(r.width.into()),
        height: Some(r.height.into()),
        fill: ColorOrGradient::Color(fill).into(),
        interactive: !name.is_empty(),
        ..Default::default()
    }
}
fn scaled(scale: &ConfiguredScale, values: impl Iterator<Item = f32>) -> Result<Vec<f32>> {
    let values: ArrayRef = Arc::new(Float32Array::from_iter_values(values));
    Ok(scale
        .scale(&values)?
        .as_primitive::<Float32Type>()
        .values()
        .to_vec())
}
pub fn build(s: &State) -> Result<SceneGraph> {
    let mut marks = vec![
        rectangle("", Rect::new(0., 0., SIZE[0], SIZE[1]), [1.; 4]).into(),
        text("Cross-Filter Flights", 24., 28., 20.),
        text(
            format!(
                "{} flights · Drag to brush · Click to clear · Esc clears all",
                s.rows
            ),
            24.,
            47.,
            11.,
        ),
    ];
    for (i, p) in PLOTS.iter().enumerate() {
        let r = s.plots[i];
        let bins = &s.result.bins[i];
        let maximum = bins.iter().map(|(_, n)| *n).max().unwrap_or(0).max(5) as f32;
        let x = p.scale(r.width).with_formatting(s.formatting.clone());
        let y = LinearScale::configured((0., maximum), (r.height, 0.))
            .with_formatting(s.formatting.clone());
        marks.push(rectangle(p.name, r, [1.; 4]).into());
        for (scale, orientation, title, format) in [
            (&x, AxisOrientation::Bottom, p.title, ",.0f"),
            (&y, AxisOrientation::Left, "Count", ".2~s"),
        ] {
            marks.push(
                make_numeric_axis_marks_with_text_engine(
                    scale,
                    title,
                    [r.x, r.y],
                    &AxisConfig {
                        dimensions: [r.width, r.height],
                        orientation,
                        tick_count: Some(5.),
                        format_number: Some(format.into()),
                        ..Default::default()
                    },
                    &s.text,
                )?
                .into(),
            );
        }
        let x0 = scaled(
            &x,
            bins.iter()
                .map(|&(bin, _)| p.domain[0] + bin as f32 * p.step),
        )?;
        let x1 = scaled(
            &x,
            bins.iter()
                .map(|&(bin, _)| p.domain[0] + (bin + 1) as f32 * p.step),
        )?;
        let y0 = scaled(&y, bins.iter().map(|&(_, count)| count as f32))?;
        let bars = SceneRectMark {
            len: bins.len() as u32,
            x: x0.iter().map(|x| x + 0.5).collect::<Vec<_>>().into(),
            y: y0.clone().into(),
            width: Some(
                x0.iter()
                    .zip(&x1)
                    .map(|(a, b)| (b - a - 1.).max(0.))
                    .collect::<Vec<_>>()
                    .into(),
            ),
            height: Some(
                y0.iter()
                    .map(|y| (r.height - y).max(0.))
                    .collect::<Vec<_>>()
                    .into(),
            ),
            fill: ColorOrGradient::Color([70. / 255., 130. / 255., 180. / 255., 1.]).into(),
            interactive: false,
            ..Default::default()
        };
        marks.push(
            SceneGroup {
                origin: [r.x, r.y],
                clip: Clip::Rect {
                    x: 0.,
                    y: 0.,
                    width: r.width,
                    height: r.height,
                },
                marks: vec![bars.into()],
                ..Default::default()
            }
            .into(),
        );
        if let Some(bounds) = s
            .preview
            .filter(|(plot, _)| *plot == i)
            .map(|(_, b)| b)
            .or(s.selections.bounds[i])
        {
            let [a, b] = s.selections.pixels(i, bounds)?;
            let mut brush = rectangle(
                "",
                Rect::new(r.x + a, r.y, (b - a).max(0.), r.height),
                [0., 0., 0., 0.12],
            );
            brush.stroke = ColorOrGradient::Color([0.25, 0.25, 0.25, 0.8]).into();
            brush.stroke_width = 1_f32.into();
            marks.push(brush.into());
        } else {
            // A stable slot keeps subsequent panel hit-test paths unchanged during a drag.
            marks.push(SceneGroup::default().into());
        }
    }
    for (value, x, y, font) in [
        (
            format!("{:.0} ms", s.result.elapsed_ms),
            24.,
            SIZE[1] - 29.,
            "monospace",
        ),
        (
            if s.preaggregate {
                format!("auto · {}/3 preaggregated", s.result.preaggregated)
            } else {
                "direct · preaggregation off".into()
            },
            96.,
            SIZE[1] - 29.,
            "sans-serif",
        ),
        (
            s.error.as_ref().unwrap_or(&s.warmup_message).clone(),
            24.,
            SIZE[1] - 12.,
            "sans-serif",
        ),
    ] {
        marks.push(
            SceneTextMark {
                text: value.into(),
                x: x.into(),
                y: y.into(),
                font: font.to_string().into(),
                font_size: 11_f32.into(),
                color: ColorOrGradient::Color([0.42, 0.45, 0.49, 1.]).into(),
                interactive: false,
                ..Default::default()
            }
            .into(),
        );
    }
    Ok(SceneGraph {
        width: SIZE[0],
        height: SIZE[1],
        origin: [0., 0.],
        marks: vec![
            SceneGroup {
                marks,
                ..Default::default()
            }
            .into(),
        ],
    })
}
