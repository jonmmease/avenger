// rustfmt::skip

use avenger_chart::utils::param;
use avenger_chart::{
    runtime::AvengerRuntime,
    types::{
        group::Group,
        mark::{EncodingUtils, Mark},
        scales::{Scale, ScaleDomain, ScaleRange},
    },
};
use datafusion::{
    logical_expr::expr::Placeholder,
    prelude::{lit, Expr, SessionContext},
};
use palette::Srgba;

#[tokio::test]
async fn test_compilation() {
    let ctx = SessionContext::new();
    let runtime = AvengerRuntime::new(ctx);

    let chart = Group::new()
        // .dataset("data_0", dataframe)
        .x(30.0)
        .y(40.0)
        .param("width", lit(300.0))
        .mark(
            Mark::arc()
                // .from("data_0")
                .x(lit(3.0).scale("x_scale"))
                .fill(lit(2.5).scale("color_scale")),
        )
        .scale(
            "x_scale",
            Scale::new()
                .scale_type("linear")
                .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
                .range(ScaleRange::new_numeric(lit(0.0), param("width"))),
        )
        .scale(
            "color_scale",
            Scale::new()
                .scale_type("linear")
                .domain(ScaleDomain::new_interval(lit(0.0), lit(10.0)))
                .range(ScaleRange::new_rgb(vec![
                    Srgba::new(1.0, 0.0, 0.0, 1.0),
                    Srgba::new(0.0, 1.0, 0.0, 1.0),
                ])),
        );

    let scene = runtime.compile_group(&chart, None).await.unwrap();
    println!("{:?}", scene);
}
