//! Small DataFusion expression helpers for calendar-oriented chart data.

use datafusion::{
    arrow::datatypes::DataType,
    functions::{
        datetime::expr_fn::{date_part, make_date as df_make_date, to_char},
        string::expr_fn::concat,
    },
    logical_expr::{Expr, expr_fn::cast, lit},
};

use crate::IntoExpr;

fn part(name: &'static str, expr: impl IntoExpr) -> Expr {
    date_part(lit(name), expr.into_expr())
}

/// Extract the calendar year from a date or timestamp expression.
pub fn year(expr: impl IntoExpr) -> Expr {
    part("year", expr)
}

/// Extract the calendar quarter from a date or timestamp expression.
pub fn quarter(expr: impl IntoExpr) -> Expr {
    part("quarter", expr)
}

/// Extract the calendar month from a date or timestamp expression.
pub fn month(expr: impl IntoExpr) -> Expr {
    part("month", expr)
}

/// Extract the ISO week number from a date or timestamp expression.
///
/// This is a low-level DataFusion helper. `TimeLevels` v1 intentionally does
/// not expose week-number hierarchies because week-year semantics need a
/// separate design.
pub fn week(expr: impl IntoExpr) -> Expr {
    part("week", expr)
}

/// Extract the day of month from a date or timestamp expression.
pub fn day_of_month(expr: impl IntoExpr) -> Expr {
    part("day", expr)
}

/// Extract the day of week from a date or timestamp expression.
///
/// DataFusion's `dow` convention is Sunday = 0.
pub fn day_of_week(expr: impl IntoExpr) -> Expr {
    part("dow", expr)
}

/// Extract the hour from a date, time, or timestamp expression.
pub fn hour(expr: impl IntoExpr) -> Expr {
    part("hour", expr)
}

/// Extract the minute from a date, time, or timestamp expression.
pub fn minute(expr: impl IntoExpr) -> Expr {
    part("minute", expr)
}

/// Format a date, time, or timestamp expression with DataFusion `to_char`.
pub fn format(expr: impl IntoExpr, fmt: impl IntoExpr) -> Expr {
    to_char(expr.into_expr(), fmt.into_expr())
}

/// Build a date from year, month, and day expressions.
pub fn make_date_expr(year: impl IntoExpr, month: impl IntoExpr, day: impl IntoExpr) -> Expr {
    df_make_date(year.into_expr(), month.into_expr(), day.into_expr())
}

/// Alias for [`make_date_expr`] with the intended public name.
pub use make_date_expr as make_date;

/// Convert a year key expression to a display label.
pub fn year_label(expr: impl IntoExpr) -> Expr {
    cast(expr.into_expr(), DataType::Utf8)
}

/// Convert a quarter key expression to labels like `Q1`.
pub fn quarter_label(expr: impl IntoExpr) -> Expr {
    concat(vec![lit("Q"), cast(expr.into_expr(), DataType::Utf8)])
}

/// Convert a 1-based month key to the full month name.
pub fn month_name_from_number(expr: impl IntoExpr) -> Expr {
    format(make_date(lit(2000_i32), expr, lit(1_i32)), lit("%B"))
}

/// Convert a 1-based month key to the abbreviated month name.
pub fn month_abbrev_from_number(expr: impl IntoExpr) -> Expr {
    format(make_date(lit(2000_i32), expr, lit(1_i32)), lit("%b"))
}

/// Convert DataFusion's Sunday=0 day-of-week key to the full day name.
pub fn day_name_from_number(expr: impl IntoExpr) -> Expr {
    let day = cast(expr.into_expr(), DataType::Int32) + lit(7_i32);
    format(make_date(lit(2024_i32), lit(1_i32), day), lit("%A"))
}

/// Convert DataFusion's Sunday=0 day-of-week key to the abbreviated day name.
pub fn day_abbrev_from_number(expr: impl IntoExpr) -> Expr {
    let day = cast(expr.into_expr(), DataType::Int32) + lit(7_i32);
    format(make_date(lit(2024_i32), lit(1_i32), day), lit("%a"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::{
        arrow::{
            array::{StringArray, TimestampMillisecondArray},
            datatypes::{DataType, Field, Schema, TimeUnit},
            record_batch::RecordBatch,
        },
        prelude::{SessionContext, col},
    };
    use std::sync::Arc;

    #[test]
    fn time_expr_year_quarter_month_lower_to_date_part() {
        assert_eq!(
            year(col("date")).to_string(),
            "date_part(Utf8(\"year\"), date)"
        );
        assert_eq!(
            quarter(col("date")).to_string(),
            "date_part(Utf8(\"quarter\"), date)"
        );
        assert_eq!(
            month(col("date")).to_string(),
            "date_part(Utf8(\"month\"), date)"
        );
    }

    #[test]
    fn time_expr_make_date_and_format_build_expected_exprs() {
        assert_eq!(
            make_date(lit(2024_i32), lit(1_i32), lit(31_i32)).to_string(),
            "make_date(Int32(2024), Int32(1), Int32(31))"
        );
        assert_eq!(
            format(col("date"), lit("%b")).to_string(),
            "to_char(date, Utf8(\"%b\"))"
        );
    }

    #[test]
    fn time_expr_quarter_label_formats_q_prefix() {
        assert_eq!(
            quarter_label(col("quarter")).to_string(),
            "concat(Utf8(\"Q\"), CAST(quarter AS Utf8))"
        );
    }

    #[test]
    fn time_expr_month_name_from_number_uses_synthetic_date() {
        assert_eq!(
            month_name_from_number(col("month")).to_string(),
            "to_char(make_date(Int32(2000), month, Int32(1)), Utf8(\"%B\"))"
        );
        assert_eq!(
            month_abbrev_from_number(col("month")).to_string(),
            "to_char(make_date(Int32(2000), month, Int32(1)), Utf8(\"%b\"))"
        );
    }

    #[tokio::test]
    async fn time_expr_helpers_work_in_dataframe_projection() {
        let ctx = SessionContext::new();
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new(
                "date",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                false,
            )])),
            vec![Arc::new(TimestampMillisecondArray::from(vec![1_704_067_200_000_i64])) as _],
        )
        .expect("batch");
        let df = ctx
            .read_batch(batch)
            .expect("dataframe")
            .select(vec![
                quarter_label(cast(quarter(col("date")), DataType::Int32)).alias("quarter_label"),
                month_abbrev_from_number(cast(month(col("date")), DataType::Int32))
                    .alias("month_label"),
                day_name_from_number(lit(0_i32)).alias("day_label"),
                day_abbrev_from_number(lit(0_i32)).alias("day_abbrev"),
            ])
            .expect("select");
        let batches = df.collect().await.expect("collect");
        let batch = &batches[0];
        let quarter = batch
            .column_by_name("quarter_label")
            .expect("quarter label")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("quarter strings");
        let month = batch
            .column_by_name("month_label")
            .expect("month label")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("month strings");
        let day = batch
            .column_by_name("day_label")
            .expect("day label")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("day strings");
        let day_abbrev = batch
            .column_by_name("day_abbrev")
            .expect("day abbrev")
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("day abbrev strings");
        assert_eq!(quarter.value(0), "Q1");
        assert_eq!(month.value(0), "Jan");
        assert_eq!(day.value(0), "Sunday");
        assert_eq!(day_abbrev.value(0), "Sun");
    }
}
