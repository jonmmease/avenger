//! Test parameter functionality

use avenger_chart::param::Param;
use avenger_chart::plot::Chart;
use datafusion::arrow::datatypes::DataType;
use datafusion::prelude::*;
use datafusion::scalar::ScalarValue;

#[tokio::test]
async fn test_param_creation() {
    // Create a parameter with a name and default value
    let param = {
        let __avenger_param_name = "threshold";
        let __avenger_param_default: datafusion::common::ScalarValue =
            ScalarValue::Float64(Some(50.0));
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Verify the parameter has the correct name and default
    assert_eq!(param.name, "threshold");
    assert_eq!(param.default, ScalarValue::Float64(Some(50.0)));

    // Verify we can create an expression from the parameter
    let expr = param.expr();
    match expr {
        Expr::Placeholder(placeholder) => {
            assert_eq!(placeholder.id, "$threshold");
            assert!(placeholder.field.is_some());
        }
        _ => panic!("Expected placeholder expression"),
    }
}

#[tokio::test]
async fn test_plot_with_params() {
    use avenger_chart::cartesian::{Cartesian, CartesianRectPositionChannels};
    use avenger_chart::marks::rect::Rect;

    let ctx = SessionContext::new();

    // Create sample data
    let df = ctx
        .sql("SELECT * FROM (VALUES (1, 10), (2, 20), (3, 30)) AS t(x, y)")
        .await
        .unwrap();

    // Create parameters
    let param1 = {
        let __avenger_param_name = "scale_factor";
        let __avenger_param_default: datafusion::common::ScalarValue =
            ScalarValue::Float64(Some(2.0));
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };
    let param2 = {
        let __avenger_param_name = "offset";
        let __avenger_param_default: datafusion::common::ScalarValue = ScalarValue::Int32(Some(5));
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Create plot with parameters
    let plot = Chart::<Cartesian>::new()
        .data(df)
        .param(param1.clone())
        .param(param2.clone())
        .mark(
            Rect::new()
                .x(col("x"))
                .y(col("y") * param1.expr() + param2.expr()),
        );

    // Compile the plot
    let compiled = plot.compile(&ctx).await.unwrap();

    // Verify default parameters were extracted
    let default_params = compiled.get_default_params();
    assert_eq!(default_params.len(), 2);
    assert_eq!(
        default_params.get("scale_factor"),
        Some(&ScalarValue::Float64(Some(2.0)))
    );
    assert_eq!(
        default_params.get("offset"),
        Some(&ScalarValue::Int32(Some(5)))
    );
}

#[tokio::test]
async fn test_param_typed_constructor() {
    let param = Param::typed(
        "my_param",
        DataType::Boolean,
        ScalarValue::Boolean(Some(true)),
    )
    .unwrap();
    assert_eq!(param.name, "my_param");
    assert_eq!(param.data_type, DataType::Boolean);
    assert_eq!(param.default, ScalarValue::Boolean(Some(true)));
}

#[tokio::test]
async fn test_param_into_expr() {
    let param = {
        let __avenger_param_name = "test";
        let __avenger_param_default: datafusion::common::ScalarValue = ScalarValue::Int64(Some(42));
        Param::typed(
            __avenger_param_name,
            __avenger_param_default.data_type(),
            __avenger_param_default,
        )
        .expect("a parameter default must match its selected physical type")
    };

    // Test Into<Expr> for owned Param
    let expr1: Expr = param.clone().into();
    match expr1 {
        Expr::Placeholder(p) => assert_eq!(p.id, "$test"),
        _ => panic!("Expected placeholder"),
    }

    // Test Into<Expr> for borrowed Param
    let expr2: Expr = (&param).into();
    match expr2 {
        Expr::Placeholder(p) => assert_eq!(p.id, "$test"),
        _ => panic!("Expected placeholder"),
    }
}

#[tokio::test]
async fn test_add_params_multiple() {
    use avenger_chart::cartesian::Cartesian;

    let ctx = SessionContext::new();

    let params = vec![
        {
            let __avenger_param_name = "p1";
            let __avenger_param_default: datafusion::common::ScalarValue =
                ScalarValue::Float32(Some(1.0));
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        },
        {
            let __avenger_param_name = "p2";
            let __avenger_param_default: datafusion::common::ScalarValue =
                ScalarValue::Float32(Some(2.0));
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        },
        {
            let __avenger_param_name = "p3";
            let __avenger_param_default: datafusion::common::ScalarValue =
                ScalarValue::Float32(Some(3.0));
            Param::typed(
                __avenger_param_name,
                __avenger_param_default.data_type(),
                __avenger_param_default,
            )
            .expect("a parameter default must match its selected physical type")
        },
    ];

    let plot = Chart::<Cartesian>::new().params(params);

    let compiled = plot.compile(&ctx).await.unwrap();

    // Verify all params were added
    let default_params = compiled.get_default_params();
    assert_eq!(default_params.len(), 3);
    assert!(default_params.contains_key("p1"));
    assert!(default_params.contains_key("p2"));
    assert!(default_params.contains_key("p3"));
}
