// rustfmt::skip

use std::f32::consts::PI;
use std::sync::Arc;

use arrow::array::{ArrayRef, Float32Array, RecordBatch};
use arrow::datatypes::{DataType, Field, Schema};
use avenger_chart2::param::Param;
use avenger_chart2::runtime::scale::scale_expr;
use avenger_chart2::utils::param;
use avenger_chart2::{
    runtime::AvengerRuntime,
    types::{
        group::Group,
        mark::{EncodingUtils, Mark},
        scales::{Scale, ScaleDomain, ScaleRange},
    },
};
use avenger_common::canvas::CanvasDimensions;
use avenger_geometry::geo_types::Line;
use avenger_scales2::scales::linear::LinearScale;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use avenger_wgpu::error::AvengerWgpuError;
use datafusion::common::ParamValues;
use datafusion::prelude::{col, placeholder};
use datafusion::scalar::ScalarValue;
use datafusion::{
    logical_expr::expr::Placeholder,
    prelude::{lit, Expr, SessionContext},
};
use palette::Srgba;

#[tokio::test]
async fn test_compilation() -> Result<(), AvengerWgpuError> {
    // runtime
    let runtime = AvengerRuntime::new(SessionContext::new());

    // params
    let stroke_color = Param::new("stroke_color", "cyan");
    let width = Param::new("width", 300.0);

    // Load dataframe
    let schema = Schema::new(vec![Field::new("a", DataType::Float32, true)]);
    let columns = vec![Arc::new(Float32Array::from(vec![1.0, 2.0, 3.0, 4.0, 5.0])) as ArrayRef];
    let batch = RecordBatch::try_new(Arc::new(schema), columns).unwrap();
    let data_0 = runtime.ctx().read_batch(batch).unwrap();

    // scales
    let x_scale = Scale::new(LinearScale)
        .domain_data_field(Arc::new(data_0.clone()), "a")
        .range(ScaleRange::new_interval(lit(0.0), &width));

    let y_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_interval(lit(0.0), lit(400.0)));

    let color_scale = Scale::new(LinearScale)
        .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
        .range(ScaleRange::new_color(vec![
            Srgba::new(1.0, 0.0, 0.0, 1.0),
            Srgba::new(0.0, 1.0, 0.0, 1.0),
        ]));

    let chart = Group::new().x(10.0).y(10.0).mark(
        Mark::arc()
            .from(data_0)
            .x(scale_expr(&x_scale, col("a")).unwrap())
            .y(scale_expr(&y_scale, lit(5.0)).unwrap())
            .start_angle(lit(0.0))
            .end_angle(lit(PI / 2.0))
            .outer_radius(lit(50.0))
            .inner_radius(lit(20.0))
            .fill(scale_expr(&color_scale, col("a")).unwrap())
            .stroke(&stroke_color)
            .stroke_width(lit(3.0)),
    );

    // Compile while overriding params
    let scene_group = runtime
        .compile_group(&chart, vec![stroke_color, width])
        .await
        .unwrap();
    println!("{:#?}", scene_group);

    let scene_graph = SceneGraph {
        marks: vec![scene_group.into()],
        width: 400.0,
        height: 400.0,
        origin: [0.0, 0.0],
    };

    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [scene_graph.width, scene_graph.height],
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
