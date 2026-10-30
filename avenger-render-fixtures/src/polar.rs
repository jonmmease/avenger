//! Polar axes on radar charts, rose charts, and parts of circles, in panels of a sheet. The polar
//! axes example renders sheets that compare their options, and the guides' tests compare
//! [`polar_axes`] with a baseline.

use std::sync::Arc;

use arrow::array::{ArrayRef, Float64Array, StringArray};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{FontWeight, TextAlign, TextBaseline},
    value::ScalarOrArray,
};
use avenger_guides::axis::{
    AxisStyle, PolarAxis, PolarAxisConfig, PolarAxisKind, PolarFrame, PolarLabelOrient, PolarShape,
};
use avenger_scales::scales::{band::BandScale, linear::LinearScale, ConfiguredScale};
use avenger_scenegraph::{
    marks::{
        arc::SceneArcMark,
        group::SceneGroup,
        mark::SceneMark,
        path::ScenePathMark,
        text::{SceneTextMark, TextHalo},
    },
    scene_graph::SceneGraph,
};
use avenger_typst_label::LabelEngine;
use lyon_path::{geom::euclid::Point2D, Path};

/// The size of a panel.
pub const PANEL: [f32; 2] = [300.0, 300.0];
const TITLE_HEIGHT: f32 = 28.0;
const GRAY: [f32; 4] = [0.35, 0.35, 0.35, 1.0];

/// A polar chart in one panel: its axes, its angles, and how it draws its data.
pub struct Panel {
    pub title: String,
    pub theta: PolarAxisConfig,
    pub radius: PolarAxisConfig,
    pub theta_scale: ConfiguredScale,
    pub radius_scale: ConfiguredScale,
    /// The hole, as a fraction of the outer radius.
    pub hole: f32,
    pub data: Data,
}

/// What a panel draws inside its axes.
pub enum Data {
    /// A closed polygon through a value at each category.
    Radar(Vec<f64>),
    /// A wedge for each category, out to its value.
    Rose(Vec<f64>),
    /// Nothing but the axes.
    None,
}

/// A white halo behind each label, `width` pixels wide, that lets the grid and the data show
/// through.
pub fn halo(width: f32) -> TextHalo {
    TextHalo {
        color: [1.0, 1.0, 1.0, 0.75].into(),
        width,
    }
}

/// A radar chart of six skills, with its radius axis along the spoke at `angle`, or the start
/// spoke for `None`.
pub fn radar(title: &str, angle: Option<f32>, halo: Option<TextHalo>) -> Panel {
    let skills = strings(&["Speed", "Power", "Range", "Armor", "Agility", "Stealth"]);
    Panel {
        title: title.to_string(),
        theta: theta_config(
            0.0,
            360.0,
            PolarShape::Polygon,
            PolarLabelOrient::Horizontal,
        ),
        radius: radius_config(0.0, 360.0, angle, PolarShape::Polygon, halo),
        theta_scale: BandScale::configured(skills, (0.0, 1.0)),
        radius_scale: LinearScale::configured((0.0, 100.0), (0.0, 1.0)),
        hole: 0.0,
        data: Data::Radar(vec![80.0, 65.0, 90.0, 40.0, 70.0, 55.0]),
    }
}

/// A rose chart of the rainfall of each month, with its month labels turned by `orient`.
pub fn rose(title: &str, orient: PolarLabelOrient, halo: Option<TextHalo>) -> Panel {
    let months = strings(&[
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ]);
    Panel {
        title: title.to_string(),
        theta: theta_config(0.0, 360.0, PolarShape::Circle, orient),
        radius: radius_config(0.0, 360.0, None, PolarShape::Circle, halo),
        theta_scale: BandScale::configured(months, (0.0, 1.0)),
        radius_scale: LinearScale::configured((0.0, 100.0), (0.0, 1.0)),
        hole: 0.0,
        data: Data::Rose(vec![
            78.0, 60.0, 55.0, 48.0, 50.0, 42.0, 35.0, 40.0, 52.0, 70.0, 85.0, 90.0,
        ]),
    }
}

/// The axes of the part of a circle from `start` to `end` degrees, with a hole of `hole` times
/// its radius.
pub fn sector(title: &str, start: f32, end: f32, hole: f32) -> Panel {
    Panel {
        title: title.to_string(),
        theta: theta_config(start, end, PolarShape::Circle, PolarLabelOrient::Horizontal),
        radius: radius_config(start, end, None, PolarShape::Circle, None),
        theta_scale: LinearScale::configured((0.0, 100.0), (0.0, 1.0)),
        radius_scale: LinearScale::configured((0.0, 50.0), (0.0, 1.0)),
        hole,
        data: Data::None,
    }
}

/// The panels in rows of `columns`, with their labels measured by `engine`, which formats
/// numbers with d3 patterns.
pub fn sheet(panels: &[Panel], columns: usize, engine: &LabelEngine) -> SceneGraph {
    let rows = panels.len().div_ceil(columns);
    let marks = panels
        .iter()
        .enumerate()
        .map(|(index, panel)| {
            let origin = [
                (index % columns) as f32 * PANEL[0],
                (index / columns) as f32 * PANEL[1],
            ];
            panel_group(panel, engine, origin).into()
        })
        .collect();
    SceneGraph {
        marks,
        width: PANEL[0] * columns as f32,
        height: PANEL[1] * rows as f32,
        origin: [0.0, 0.0],
    }
}

/// A radar chart and two rose charts with halos behind their radius labels, one with month labels
/// that curve along their circle, and a half donut and a sector with only their axes.
pub fn polar_axes(engine: &LabelEngine) -> SceneGraph {
    let panels = [
        radar("Radar, halo", None, Some(halo(2.0))),
        rose(
            "Rose, radial labels",
            PolarLabelOrient::Radial,
            Some(halo(2.0)),
        ),
        rose(
            "Rose, curved labels",
            PolarLabelOrient::Tangential,
            Some(halo(2.0)),
        ),
        sector("Half donut, -90° to 90°", -90.0, 90.0, 0.5),
        sector("Sector, 30° to 150°", 30.0, 150.0, 0.0),
    ];
    sheet(&panels, panels.len(), engine)
}

fn style() -> AxisStyle {
    AxisStyle {
        tick_length: 4.0,
        tick_color: [0.53, 0.53, 0.53, 1.0],
        domain_color: [0.53, 0.53, 0.53, 1.0],
        label_color: GRAY,
        label_font_size: 11.0,
        grid_color: [0.0, 0.0, 0.0, 0.12],
        grid_width: 1.0,
        ..AxisStyle::default()
    }
}

fn theta_config(
    start: f32,
    end: f32,
    shape: PolarShape,
    orient: PolarLabelOrient,
) -> PolarAxisConfig {
    PolarAxisConfig {
        kind: PolarAxisKind::Theta,
        start_angle: start,
        end_angle: end,
        shape,
        label_orient: orient,
        tick_format: ",.0f".to_string(),
        style: style(),
        ..Default::default()
    }
}

fn radius_config(
    start: f32,
    end: f32,
    angle: Option<f32>,
    shape: PolarShape,
    halo: Option<TextHalo>,
) -> PolarAxisConfig {
    PolarAxisConfig {
        kind: PolarAxisKind::Radius,
        start_angle: start,
        end_angle: end,
        angle,
        shape,
        label_halo: halo,
        tick_format: ",.0f".to_string(),
        style: AxisStyle {
            tick_count: Some(4.0),
            ..style()
        },
        ..Default::default()
    }
}

fn strings(values: &[&str]) -> ArrayRef {
    Arc::new(StringArray::from(values.to_vec()))
}

/// The box of the part of a unit circle between two angles, with its hole, as the axis sizes it.
fn unit_box(start: f32, end: f32, hole: f32) -> [[f32; 2]; 2] {
    PolarFrame {
        center: [0.0, 0.0],
        inner_radius: hole,
        outer_radius: 1.0,
        corners: Vec::new(),
    }
    .sector_box(start, end)
}

fn panel_group(panel: &Panel, engine: &LabelEngine, origin: [f32; 2]) -> SceneGroup {
    let theta = match panel.theta_scale.scale_impl.scale_type() {
        "band" => PolarAxis::band(panel.theta_scale.clone(), &panel.theta, engine),
        _ => PolarAxis::continuous(panel.theta_scale.clone(), &panel.theta, engine),
    }
    .unwrap();
    let radius = PolarAxis::continuous(panel.radius_scale.clone(), &panel.radius, engine).unwrap();
    let [width, height] = [PANEL[0] - 16.0, PANEL[1] - TITLE_HEIGHT - 16.0];
    let unit = unit_box(panel.theta.start_angle, panel.theta.end_angle, panel.hole);
    let [unit_width, unit_height] = [unit[1][0] - unit[0][0], unit[1][1] - unit[0][1]];
    // The largest circle whose box, with the reach of the axes past it, fits the panel.
    let mut reach = [0.0_f32; 4];
    let mut frame = PolarFrame {
        center: [0.0, 0.0],
        inner_radius: 0.0,
        outer_radius: 1.0,
        corners: Vec::new(),
    };
    for _ in 0..4 {
        let outer = ((width - reach[0] - reach[1]) / unit_width)
            .min((height - reach[2] - reach[3]) / unit_height)
            .max(1.0);
        let center = [
            8.0 + reach[0] - unit[0][0] * outer
                + (width - reach[0] - reach[1] - unit_width * outer) / 2.0,
            TITLE_HEIGHT + 8.0 + reach[2] - unit[0][1] * outer,
        ];
        frame = PolarFrame {
            center,
            inner_radius: outer * panel.hole,
            outer_radius: outer,
            corners: Vec::new(),
        };
        frame.corners = theta.tick_angles(&frame);
        let [a, b] = [theta.reach(&frame), radius.reach(&frame)];
        reach = [0, 1, 2, 3].map(|side| a[side].max(b[side]));
    }

    let mut marks: Vec<SceneMark> = vec![theta.grid(&frame).into(), radius.grid(&frame).into()];
    let radii = radius.scale(&frame);
    let positions = |scale: &ConfiguredScale, values: ArrayRef| -> Vec<f32> {
        let len = values.len();
        scale.scale_to_numeric(&values).unwrap().as_vec(len, None)
    };
    let categories = panel.theta_scale.domain().clone();
    match &panel.data {
        Data::Radar(values) => {
            let angles = positions(&theta.scale(&frame).with_option("band", 0.5), categories);
            let radii = positions(&radii, Arc::new(Float64Array::from(values.clone())));
            let mut builder = Path::builder();
            for (index, (angle, radius)) in angles.iter().zip(&radii).enumerate() {
                let [x, y] = frame.point(*angle, *radius);
                if index == 0 {
                    builder.begin(Point2D::new(x, y));
                } else {
                    builder.line_to(Point2D::new(x, y));
                }
            }
            builder.end(true);
            marks.push(
                ScenePathMark {
                    len: 1,
                    path: ScalarOrArray::new_scalar(builder.build()),
                    fill: ColorOrGradient::Color([0.298, 0.471, 0.659, 0.35]).into(),
                    stroke: ColorOrGradient::Color([0.298, 0.471, 0.659, 1.0]).into(),
                    stroke_width: Some(2.0),
                    ..Default::default()
                }
                .into(),
            );
        }
        Data::Rose(values) => {
            // The theta scale places the start of each band.
            let starts = positions(&theta.scale(&frame), categories);
            let step = 360.0 / values.len() as f32;
            let outer = positions(&radii, Arc::new(Float64Array::from(values.clone())));
            marks.push(
                SceneArcMark {
                    len: values.len() as u32,
                    x: frame.center[0].into(),
                    y: frame.center[1].into(),
                    start_angle: ScalarOrArray::new_array(
                        starts
                            .iter()
                            .map(|start| (start + 1.0).to_radians())
                            .collect(),
                    ),
                    end_angle: ScalarOrArray::new_array(
                        starts
                            .iter()
                            .map(|start| (start + step - 1.0).to_radians())
                            .collect(),
                    ),
                    inner_radius: frame.inner_radius.into(),
                    outer_radius: ScalarOrArray::new_array(outer),
                    fill: ColorOrGradient::Color([0.96, 0.52, 0.09, 0.8]).into(),
                    ..Default::default()
                }
                .into(),
            );
        }
        Data::None => {}
    }
    marks.push(theta.marks(&frame).into());
    marks.push(radius.marks_avoiding(&frame, &[&theta]).into());
    marks.push(
        SceneTextMark {
            len: 1,
            text: panel.title.clone().into(),
            x: (PANEL[0] / 2.0).into(),
            y: 6.0.into(),
            align: TextAlign::Center.into(),
            baseline: TextBaseline::Top.into(),
            font_size: 12.0.into(),
            font_weight: FontWeight::BOLD.into(),
            ..Default::default()
        }
        .into(),
    );
    SceneGroup {
        origin,
        marks,
        ..Default::default()
    }
}
