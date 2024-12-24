// rustfmt::skip

use std::f32::consts::PI;

use avenger_chart::utils::param;
use avenger_chart::{
    runtime::AvengerRuntime,
    types::{
        group::Group,
        mark::{EncodingUtils, Mark},
        scales::{Scale, ScaleDomain, ScaleRange},
    },
};
use avenger_common::canvas::CanvasDimensions;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use datafusion::common::ParamValues;
use datafusion::{
    logical_expr::expr::Placeholder,
    prelude::{lit, Expr, SessionContext},
};
use palette::Srgba;

#[tokio::test]
async fn test_compilation() -> Result<(), AvengerWgpuError> {
    let ctx = SessionContext::new();
    let runtime = AvengerRuntime::new(ctx);

    let chart = Group::new()
        // .dataset("data_0", dataframe)
        .x(30.0)
        .y(40.0)
        .param("width", lit(300.0))
        .param("stroke_color", lit("red"))
        .mark(
            Mark::arc()
                // .from("data_0")
                .x(lit(3.0).scale("x_scale"))
                .y(lit(150.0))
                .start_angle(lit(0.0))
                .end_angle(lit(PI / 2.0))
                .outer_radius(lit(150.0))
                .inner_radius(lit(20.0))
                .fill(lit(2.5).scale("color_scale"))
                .stroke(param("stroke_color"))
                .stroke_width(lit(3.0)),
        )
        .scale(
            Scale::new("x_scale")
                .kind("linear")
                .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
                .range(ScaleRange::new_numeric(lit(0.0), param("width"))),
        )
        .scale(
            Scale::new("color_scale")
                .kind("linear")
                .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
                .range(ScaleRange::new_color(vec![
                    Srgba::new(1.0, 0.0, 0.0, 1.0),
                    Srgba::new(0.0, 1.0, 0.0, 1.0),
                ])),
        );

    // Compile while overriding params
    let scene_group = runtime
        .compile_group(
            &chart,
            Some(&ParamValues::Map(
                vec![("stroke_color".into(), "cyan".into())]
                    .into_iter()
                    .collect(),
            )),
        )
        .await
        .unwrap();
    println!("{:#?}", scene_group);

    let scene_graph = SceneGraph {
        marks: vec![scene_group.into()],
        width: 300.0,
        height: 400.0,
        origin: [0.0, 0.0],
    };

    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [300.0, 400.0],
            scale: 2.0,
        },
        CanvasConfig::default(),
    )
    .await?;

    canvas.set_scene(&scene_graph)?;
    let png = canvas.render().await?;
    png.save("test.png").unwrap();

    Ok(())
}
