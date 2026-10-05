//! Render nested categorical bands and a currency-formatted numeric guide.
use std::sync::Arc;

use arrow::array::{create_array, ArrayRef, Float32Array, StructArray};
use avenger_color::ColorOrGradient;
use avenger_common::canvas::CanvasDimensions;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_guides::axis::{
    continuous::make_continuous_axis_marks,
    nested_band::make_nested_band_axis_marks,
    opts::{AxisConfig, AxisOrientation, AxisStyle},
};
use avenger_scales::scales::{
    linear::LinearScale,
    nested_band::{NestedBandLayout, NestedBandScale},
};
use avenger_scenegraph::{
    marks::{group::SceneGroup, rect::SceneRectMark, text::SceneTextMark},
    scene_graph::SceneGraph,
};
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "nested-bands.png".into());
    let columns: Vec<(&str, ArrayRef)> = vec![
        (
            "Region",
            create_array!(Utf8, ["East", "East", "Central", "West", "West", "West"]),
        ),
        (
            "Channel",
            create_array!(
                Utf8,
                ["Online", "Retail", "Online", "Online", "Retail", "Partners"]
            ),
        ),
    ];
    let domain: ArrayRef = Arc::new(StructArray::try_from(columns)?);
    let x = NestedBandScale::configured(domain.clone(), (0.0, 600.0))
        .with_option("padding_inner", 0.2)
        .with_option("padding_outer", 0.1);
    let layout = NestedBandLayout::from_config(&x.config)?;
    let y = LinearScale::configured((0.0, 100_000.0), (220.0, 0.0));
    let values: ArrayRef = Arc::new(Float32Array::from(vec![
        82_000.0, 54_000.0, 68_000.0, 94_000.0, 62_000.0, 41_000.0,
    ]));
    let x_positions = x.scale_to_numeric(&domain)?;
    let y_positions = y.scale_to_numeric(&values)?;
    let bars = SceneRectMark {
        len: 6,
        x: x_positions,
        y: y_positions,
        y2: Some(220.0.into()),
        width: Some((layout.leaf_bandwidth() as f32).into()),
        fill: vec![
            ColorOrGradient::Color([0.15, 0.46, 0.67, 1.0]),
            ColorOrGradient::Color([0.15, 0.46, 0.67, 1.0]),
            ColorOrGradient::Color([0.18, 0.58, 0.47, 1.0]),
            ColorOrGradient::Color([0.53, 0.38, 0.67, 1.0]),
            ColorOrGradient::Color([0.53, 0.38, 0.67, 1.0]),
            ColorOrGradient::Color([0.53, 0.38, 0.67, 1.0]),
        ]
        .into(),
        ..Default::default()
    };
    // The engine that formats and measures the axes' labels draws them too.
    let text_engine = avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
    let scene = SceneGraph {
        width: 770.0,
        height: 420.0,
        origin: [0.0, 0.0],
        marks: vec![
            SceneTextMark {
                text: "Revenue by region and channel".to_string().into(),
                x: 28.0.into(),
                y: 44.0.into(),
                font_size: 24.0.into(),
                ..Default::default()
            }
            .into(),
            SceneTextMark {
                text: "Equal bar widths, with space allocated to each region's channels"
                    .to_string()
                    .into(),
                x: 28.0.into(),
                y: 72.0.into(),
                font_size: 14.0.into(),
                ..Default::default()
            }
            .into(),
            SceneGroup {
                origin: [100.0, 108.0],
                marks: vec![
                    make_continuous_axis_marks(
                        &y,
                        "",
                        [0.0; 2],
                        &AxisConfig {
                            orientation: AxisOrientation::Left,
                            dimensions: [600.0, 220.0],
                            grid: true,
                            tick_format: "$,.0f".into(),
                            tick_label: None,
                            style: AxisStyle {
                                tick_count: Some(5.0),
                                ..Default::default()
                            },
                        },
                        &text_engine,
                    )?
                    .into(),
                    bars.into(),
                    make_nested_band_axis_marks(
                        &x,
                        "",
                        [0.0; 2],
                        &AxisConfig {
                            orientation: AxisOrientation::Bottom,
                            dimensions: [600.0, 220.0],
                            grid: false,
                            tick_format: String::new(),
                            tick_label: None,
                            style: AxisStyle {
                                title_visible: false,
                                ..Default::default()
                            },
                        },
                        &[],
                        &text_engine,
                    )?
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
        ],
    };
    let mut canvas = pollster::block_on(PngCanvas::new(
        CanvasDimensions {
            size: [scene.width, scene.height],
            scale: 2.0,
        },
        CanvasConfig::default(),
    ))?;
    canvas.set_scene(&scene, &text_engine)?;
    pollster::block_on(canvas.render())?.save(&output)?;
    println!("Saved {output}");
    Ok(())
}
