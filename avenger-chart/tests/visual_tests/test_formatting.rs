use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;

use datafusion::arrow::array::{Float64Array, TimestampMillisecondArray};
use datafusion::arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::common::ScalarValue;
use datafusion::prelude::*;
use std::sync::Arc;

fn make_df_xy(x: &[f64], y: &[f64]) -> DataFrame {
    let x_values = Float64Array::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

fn make_df_xyv(x: &[f64], y: &[f64], v: &[f64]) -> DataFrame {
    let x_values = Float64Array::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let v_values = Float64Array::from(v.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("v", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![Arc::new(x_values), Arc::new(y_values), Arc::new(v_values)],
    )
    .unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

fn make_df_time_y(x: &[i64], y: &[f64]) -> DataFrame {
    let x_values = TimestampMillisecondArray::from(x.to_vec());
    let y_values = Float64Array::from(y.to_vec());
    let schema = Arc::new(Schema::new(vec![
        Field::new("x", DataType::Timestamp(TimeUnit::Millisecond, None), false),
        Field::new("y", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(schema, vec![Arc::new(x_values), Arc::new(y_values)]).unwrap();
    let ctx = SessionContext::new();
    ctx.read_batch(batch).unwrap()
}

#[tokio::test]
async fn axis_y_currency_fixed() {
    let ctx = SessionContext::new();
    let df = make_df_xy(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[1200.0, 3400.0, 5600.0, 12345.0, 98765.0],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 100000.0)))
                    .axis(|a| a.title("Revenue").format("$,.2f"))
            })
            .size(80.0)
            .fill_with("#2ca25f", |c| c.no_legend()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "format_axis_y_currency_fixed",
    )
    .await;
}

#[tokio::test]
async fn axis_y_percent() {
    let ctx = SessionContext::new();
    let df = make_df_xy(&[1.0, 2.0, 3.0, 4.0, 5.0], &[0.1, 0.25, 0.5, 0.75, 0.95]);

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 1.0)))
                    .axis(|a| a.title("Completion").format(".0%"))
            })
            .size(80.0)
            .fill_with("#3182bd", |c| c.no_legend()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "format_axis_y_percent").await;
}

#[tokio::test]
async fn axis_y_si_prefix() {
    let ctx = SessionContext::new();
    let df = make_df_xy(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[1.2e3, 4.5e4, 7.8e5, 2.3e6, 9.9e7],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 1.0e8)))
                    .axis(|a| a.title("Population").format(".2~s"))
            })
            .size(80.0)
            .fill_with("#e6550d", |c| c.no_legend()),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "format_axis_y_si_prefix").await;
}

#[tokio::test]
async fn axis_y_numfmt_typst_math_ticks_and_title() {
    let ctx = SessionContext::new();
    let df = make_df_xy(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[1.2e3, 2.4e4, 3.6e5, 7.2e5, 1.2e6],
    );

    let plot = Plot::<Cartesian>::new()
        .add_params([Param::new("peak_force", ScalarValue::Float64(Some(1.2e6)))])
        .configure_title("Peak force #numfmt(peak_force, \".2e\") N", |t| t.typst())
        .data(df)
        .mark(
            Symbol::new()
                .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 1.2e6))).axis(|a| {
                        a.title("Force (N)")
                            .grid(true)
                            .tick_label("#numfmt(value, \".1e\")")
                    })
                })
                .size(90.0)
                .fill_with("#756bb1", |c| c.no_legend()),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile numfmt Typst math axis plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "format_axis_y_numfmt_typst_math_ticks_and_title",
    )
    .await;
}

#[tokio::test]
async fn axis_x_datefmt_ldml_ticks_and_title() {
    let ctx = SessionContext::new();
    let day = 86_400_000_i64;
    let start = 1_704_067_200_000_i64;
    let df = make_df_time_y(
        &[
            start,
            start + day,
            start + 2 * day,
            start + 3 * day,
            start + 4 * day,
        ],
        &[14.0, 18.0, 15.0, 21.0, 19.0],
    );
    let datetime_locale_spec = avenger_text::DateTimeLocaleSpec {
        date_patterns: Some(avenger_text::LengthsSpec {
            long: Some("y'~'MM'~'dd".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let plot = Plot::<Cartesian>::new()
        .formatting_context(
            FormattingContext::new()
                .datetime_locale("visual-datetime")
                .datetime_locale_spec("visual-datetime", datetime_locale_spec),
        )
        .add_params([Param::new("report_date", ScalarValue::Date32(Some(19727)))])
        .configure_title("Report #datefmt(report_date, \"{date:long}\")", |t| {
            t.typst()
        })
        .data(df)
        .mark(
            Line::new()
                .x_with(col("x"), |c| {
                    c.axis(|a| {
                        a.title("Observation date")
                            .tick_count(5.0)
                            .tick_label("#datefmt(value, \"MMM d\")")
                    })
                })
                .y_with(col("y"), |c| {
                    c.scale(|s| s.domain((0.0, 25.0)))
                        .axis(|a| a.title("Temperature"))
                })
                .stroke("#276fbf")
                .stroke_width(3.0),
        );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile datefmt LDML axis plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "format_axis_x_datefmt_ldml_ticks_and_title",
    )
    .await;
}

#[tokio::test]
async fn axis_x_datetime_format_ldml_ticks() {
    let ctx = SessionContext::new();
    let day = 86_400_000_i64;
    let start = 1_704_067_200_000_i64;
    let df = make_df_time_y(
        &[
            start,
            start + day,
            start + 2 * day,
            start + 3 * day,
            start + 4 * day,
        ],
        &[14.0, 18.0, 15.0, 21.0, 19.0],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Line::new()
            .x_with(col("x"), |c| {
                c.axis(|a| {
                    a.title("Observation date")
                        .tick_count(5.0)
                        .datetime_format("MMM d")
                })
            })
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((0.0, 25.0)))
                    .axis(|a| a.title("Temperature"))
            })
            .stroke("#276fbf")
            .stroke_width(3.0),
    );

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile datetime_format LDML axis plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "format_axis_x_datetime_format_ldml_ticks",
    )
    .await;
}

#[tokio::test]
async fn colorbar_percent() {
    let ctx = SessionContext::new();
    let df = make_df_xyv(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[2.0, 4.0, 6.0, 8.0, 10.0],
        &[0.05, 0.12, 0.38, 0.67, 0.91],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
            .size(100.0)
            .fill_with(col("v"), |c| {
                c.scale(|s| s.domain((0.0, 1.0)))
                    .legend(|l| l.title("Percent").format_number(".0%"))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "format_colorbar_percent").await;
}

#[tokio::test]
async fn colorbar_currency_fixed() {
    let ctx = SessionContext::new();
    let df = make_df_xyv(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[2.0, 4.0, 6.0, 8.0, 10.0],
        &[1200.0, 3400.0, 5600.0, 12345.0, 98765.0],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
            .size(100.0)
            .fill_with(col("v"), |c| {
                c.scale(|s| s.domain((0.0, 100000.0)))
                    .legend(|l| l.title("Revenue").format_number("$,.0f"))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "layout",
        "format_colorbar_currency_fixed",
    )
    .await;
}

#[tokio::test]
async fn colorbar_si_prefix() {
    let ctx = SessionContext::new();
    let df = make_df_xyv(
        &[1.0, 2.0, 3.0, 4.0, 5.0],
        &[2.0, 4.0, 6.0, 8.0, 10.0],
        &[1.2e3, 4.5e4, 7.8e5, 2.3e6, 9.9e7],
    );

    let plot = Plot::<Cartesian>::new().data(df).mark(
        Symbol::new()
            .x_with(col("x"), |c| c.scale(|s| s.domain((0.0, 6.0))))
            .y_with(col("y"), |c| c.scale(|s| s.domain((0.0, 12.0))))
            .size(100.0)
            .fill_with(col("v"), |c| {
                c.scale(|s| s.domain((0.0, 1.0e8)))
                    .legend(|l| l.title("Population").format_number(".2~s"))
            }),
    );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "layout", "format_colorbar_si_prefix").await;
}
