//! Language-owned SQL semantics shared by compiler lowering and analysis.
//!
//! DataFusion remains the expression planner and Arrow remains the cast
//! implementation. This module pins the small amount of source normalization
//! Avenger performs before handing SQL to DataFusion.

use std::{ops::ControlFlow, str::FromStr, sync::Arc};

use arrow::datatypes::{
    DataType, Field, Fields, IntervalUnit as ArrowIntervalUnit, TimeUnit as ArrowTimeUnit, i256,
};
use avenger_lang_core::{
    IntervalUnit, PhysicalField, PhysicalType, TimeUnit, sql::AvengerSqlDialect,
};
use datafusion::common::ScalarValue;
use sqlparser::{
    ast::{Expr as SqlExpr, Value as SqlValue, VisitMut, VisitorMut},
    parser::Parser,
};

/// Bump whenever authored SQL receives different planning or cast semantics.
pub const SQL_SEMANTIC_PROFILE: &str =
    "avenger-sql-df54-arrow58-exact-numeric-v1-strict-boundary-cast-v1";

/// Convert the language's physical type algebra to its exact Arrow type.
pub(crate) fn physical_type_to_arrow(value: &PhysicalType) -> DataType {
    match value {
        PhysicalType::Boolean => DataType::Boolean,
        PhysicalType::Int8 => DataType::Int8,
        PhysicalType::Int16 => DataType::Int16,
        PhysicalType::Int32 => DataType::Int32,
        PhysicalType::Int64 => DataType::Int64,
        PhysicalType::UInt8 => DataType::UInt8,
        PhysicalType::UInt16 => DataType::UInt16,
        PhysicalType::UInt32 => DataType::UInt32,
        PhysicalType::UInt64 => DataType::UInt64,
        PhysicalType::Float16 => DataType::Float16,
        PhysicalType::Float32 => DataType::Float32,
        PhysicalType::Float64 => DataType::Float64,
        PhysicalType::Utf8 => DataType::Utf8,
        PhysicalType::LargeUtf8 => DataType::LargeUtf8,
        PhysicalType::Binary => DataType::Binary,
        PhysicalType::LargeBinary => DataType::LargeBinary,
        PhysicalType::Date32 => DataType::Date32,
        PhysicalType::Date64 => DataType::Date64,
        PhysicalType::Time32(unit) => DataType::Time32(arrow_time_unit(*unit)),
        PhysicalType::Time64(unit) => DataType::Time64(arrow_time_unit(*unit)),
        PhysicalType::Timestamp { unit, timezone } => DataType::Timestamp(
            arrow_time_unit(*unit),
            timezone
                .as_ref()
                .map(|value| Arc::<str>::from(value.as_str())),
        ),
        PhysicalType::Duration(unit) => DataType::Duration(arrow_time_unit(*unit)),
        PhysicalType::Interval(unit) => DataType::Interval(match unit {
            IntervalUnit::YearMonth => ArrowIntervalUnit::YearMonth,
            IntervalUnit::DayTime => ArrowIntervalUnit::DayTime,
            IntervalUnit::MonthDayNano => ArrowIntervalUnit::MonthDayNano,
        }),
        PhysicalType::FixedSizeBinary(size) => DataType::FixedSizeBinary(*size),
        PhysicalType::Decimal128 { precision, scale } => DataType::Decimal128(*precision, *scale),
        PhysicalType::Decimal256 { precision, scale } => DataType::Decimal256(*precision, *scale),
        PhysicalType::List(element) => DataType::List(Arc::new(Field::new_list_field(
            physical_type_to_arrow(element),
            true,
        ))),
        PhysicalType::LargeList(element) => DataType::LargeList(Arc::new(Field::new_list_field(
            physical_type_to_arrow(element),
            true,
        ))),
        PhysicalType::FixedSizeList { element, length } => DataType::FixedSizeList(
            Arc::new(Field::new_list_field(physical_type_to_arrow(element), true)),
            *length,
        ),
        PhysicalType::Struct(fields) => DataType::Struct(Fields::from(
            fields
                .iter()
                .map(physical_field_to_arrow)
                .collect::<Vec<_>>(),
        )),
        PhysicalType::Map { key, value } => DataType::Map(
            Arc::new(Field::new(
                "entries",
                DataType::Struct(Fields::from(vec![
                    Field::new("keys", physical_type_to_arrow(key), false),
                    Field::new("values", physical_type_to_arrow(value), true),
                ])),
                false,
            )),
            false,
        ),
    }
}

pub(crate) fn physical_field_to_arrow(field: &PhysicalField) -> Field {
    Field::new(
        field.name.clone(),
        physical_type_to_arrow(&field.data_type),
        field.nullable,
    )
}

fn arrow_time_unit(value: TimeUnit) -> ArrowTimeUnit {
    match value {
        TimeUnit::Second => ArrowTimeUnit::Second,
        TimeUnit::Millisecond => ArrowTimeUnit::Millisecond,
        TimeUnit::Microsecond => ArrowTimeUnit::Microsecond,
        TimeUnit::Nanosecond => ArrowTimeUnit::Nanosecond,
    }
}

/// Construct an exact source scalar for an Avenger numeric spelling.
///
/// Integers use Int64/UInt64 when possible. Other exact values use the
/// narrowest decimal representation capable of retaining the coefficient and
/// scale. Floating negative zero is the sole floating source special case.
pub fn exact_numeric_scalar(source: &str) -> Result<ScalarValue, String> {
    let source = source.strip_suffix('L').unwrap_or(source);
    let negative = source.starts_with('-');
    let unsigned = source.strip_prefix(['-', '+']).unwrap_or(source);
    let floating_shape = unsigned.contains(['.', 'e', 'E']);
    if negative && floating_shape && numeric_is_zero(unsigned)? {
        return Ok(ScalarValue::Float64(Some(-0.0)));
    }
    if !floating_shape {
        if let Ok(value) = source.parse::<i64>() {
            return Ok(ScalarValue::Int64(Some(value)));
        }
        if !negative && let Ok(value) = source.parse::<u64>() {
            return Ok(ScalarValue::UInt64(Some(value)));
        }
    }

    let (mantissa, exponent) =
        unsigned
            .split_once(['e', 'E'])
            .map_or((unsigned, 0_i32), |(mantissa, exponent)| {
                exponent
                    .parse::<i32>()
                    .map(|exponent| (mantissa, exponent))
                    .unwrap_or((mantissa, i32::MAX))
            });
    if exponent == i32::MAX {
        return Err(format!(
            "numeric exponent is outside the supported range: `{source}`"
        ));
    }
    let (integer, fraction) = mantissa
        .split_once('.')
        .map_or((mantissa, ""), |(integer, fraction)| (integer, fraction));
    let coefficient_digits = format!("{integer}{fraction}");
    if coefficient_digits.is_empty()
        || !coefficient_digits
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err(format!("invalid exact numeric literal `{source}`"));
    }
    let significant = coefficient_digits.trim_start_matches('0');
    let significant = if significant.is_empty() {
        "0"
    } else {
        significant
    };
    let scale = i32::try_from(fraction.len())
        .map_err(|_| format!("numeric scale is outside the supported range: `{source}`"))?
        .checked_sub(exponent)
        .ok_or_else(|| format!("numeric scale is outside the supported range: `{source}`"))?;
    let scale = i8::try_from(scale)
        .map_err(|_| format!("numeric scale is outside Arrow's decimal range: `{source}`"))?;
    let precision = significant
        .len()
        .max(usize::try_from(scale.max(0)).unwrap_or(0));
    let precision = u8::try_from(precision.max(1))
        .map_err(|_| format!("numeric precision is outside Arrow's decimal range: `{source}`"))?;
    if precision > 76 {
        return Err(format!(
            "numeric precision {precision} exceeds Arrow decimal256: `{source}`"
        ));
    }
    let signed_digits = if negative {
        format!("-{significant}")
    } else {
        significant.to_owned()
    };
    if precision <= 38 {
        let coefficient = signed_digits
            .parse::<i128>()
            .map_err(|_| format!("invalid decimal128 coefficient `{source}`"))?;
        Ok(ScalarValue::Decimal128(Some(coefficient), precision, scale))
    } else {
        let coefficient = i256::from_str(&signed_digits)
            .map_err(|_| format!("invalid decimal256 coefficient `{source}`"))?;
        Ok(ScalarValue::Decimal256(Some(coefficient), precision, scale))
    }
}

fn numeric_is_zero(source: &str) -> Result<bool, String> {
    let mantissa = source
        .split_once(['e', 'E'])
        .map_or(source, |(left, _)| left);
    if mantissa
        .chars()
        .all(|character| character == '0' || character == '.')
    {
        Ok(true)
    } else if mantissa
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
    {
        Ok(false)
    } else {
        Err(format!("invalid exact numeric literal `{source}`"))
    }
}

/// Rewrite every numeric literal in one SQL scalar expression to an exact
/// `arrow_cast` source literal before DataFusion's SQL planner sees it.
pub fn normalize_sql_expression(sql: &str) -> Result<String, String> {
    let mut expression = Parser::new(&AvengerSqlDialect::new())
        .try_with_sql(sql)
        .map_err(|error| error.to_string())?
        .parse_expr()
        .map_err(|error| error.to_string())?;
    normalize_sql_ast(&mut expression)?;
    Ok(expression.to_string())
}

/// Apply the same exact numeric policy to a complete SQL statement/query.
pub fn normalize_sql_query(sql: &str) -> Result<String, String> {
    let mut statements =
        Parser::parse_sql(&AvengerSqlDialect::new(), sql).map_err(|error| error.to_string())?;
    if statements.len() != 1 {
        return Err("Avenger SQL requires exactly one statement".to_owned());
    }
    let mut statement = statements.remove(0);
    normalize_sql_ast(&mut statement)?;
    Ok(statement.to_string())
}

fn normalize_sql_ast<T: VisitMut>(ast: &mut T) -> Result<(), String> {
    let mut rewriter = ExactNumericRewriter { error: None };
    let _ = ast.visit(&mut rewriter);
    rewriter.error.map_or(Ok(()), Err)
}

struct ExactNumericRewriter {
    error: Option<String>,
}

impl VisitorMut for ExactNumericRewriter {
    type Break = ();

    fn pre_visit_expr(&mut self, expression: &mut SqlExpr) -> ControlFlow<Self::Break> {
        if self.error.is_some() {
            return ControlFlow::Break(());
        }
        let spelling = match expression {
            SqlExpr::Value(value) => match &value.value {
                SqlValue::Number(value, _) => Some(value.clone()),
                _ => None,
            },
            SqlExpr::UnaryOp { op, expr }
                if matches!(
                    op,
                    sqlparser::ast::UnaryOperator::Plus | sqlparser::ast::UnaryOperator::Minus
                ) && matches!(expr.as_ref(), SqlExpr::Value(value) if matches!(&value.value, SqlValue::Number(_, _))) =>
            {
                let SqlExpr::Value(value) = expr.as_ref() else {
                    unreachable!()
                };
                let SqlValue::Number(value, _) = &value.value else {
                    unreachable!()
                };
                Some(if matches!(op, sqlparser::ast::UnaryOperator::Minus) {
                    format!("-{value}")
                } else {
                    value.clone()
                })
            }
            _ => None,
        };
        let Some(spelling) = spelling else {
            return ControlFlow::Continue(());
        };
        match exact_numeric_cast_expression(&spelling) {
            Ok(replacement) => {
                *expression = replacement;
                ControlFlow::Continue(())
            }
            Err(error) => {
                self.error = Some(error);
                ControlFlow::Break(())
            }
        }
    }
}

fn exact_numeric_cast_expression(source: &str) -> Result<SqlExpr, String> {
    let scalar = exact_numeric_scalar(source)?;
    let source_type = scalar.data_type().to_string();
    let value = match scalar {
        ScalarValue::Int64(Some(value)) => value.to_string(),
        ScalarValue::UInt64(Some(value)) => value.to_string(),
        ScalarValue::Float64(Some(value)) if value == 0.0 && value.is_sign_negative() => {
            "-0.0".to_owned()
        }
        ScalarValue::Decimal128(Some(value), _, scale) => decimal_coefficient_sql(value, scale),
        ScalarValue::Decimal256(Some(value), _, scale) => decimal_coefficient_sql(value, scale),
        other => return Err(format!("unsupported exact numeric scalar {other:?}")),
    };
    Parser::new(&AvengerSqlDialect::new())
        .try_with_sql(&format!(
            "arrow_cast('{}', '{}')",
            value.replace('\'', "''"),
            source_type.replace('\'', "''")
        ))
        .map_err(|error| error.to_string())?
        .parse_expr()
        .map_err(|error| error.to_string())
}

fn decimal_coefficient_sql(value: impl ToString, scale: i8) -> String {
    let value = value.to_string();
    let negative = value.starts_with('-');
    let digits = value.strip_prefix('-').unwrap_or(&value);
    let mut result = if scale <= 0 {
        format!("{digits}{}", "0".repeat(usize::from(scale.unsigned_abs())))
    } else {
        let scale = usize::from(scale.unsigned_abs());
        if digits.len() <= scale {
            format!("0.{}{}", "0".repeat(scale - digits.len()), digits)
        } else {
            format!(
                "{}.{}",
                &digits[..digits.len() - scale],
                &digits[digits.len() - scale..]
            )
        }
    };
    if negative {
        result.insert(0, '-');
    }
    result
}

#[cfg(test)]
mod tests {
    use datafusion::common::ScalarValue;

    use super::{exact_numeric_scalar, normalize_sql_expression, normalize_sql_query};

    #[test]
    fn exact_numeric_sources_preserve_integer_decimal_and_negative_zero() {
        assert_eq!(
            exact_numeric_scalar("18446744073709551615").unwrap(),
            ScalarValue::UInt64(Some(u64::MAX))
        );
        assert_eq!(
            exact_numeric_scalar("2.050").unwrap(),
            ScalarValue::Decimal128(Some(2050), 4, 3)
        );
        assert!(
            normalize_sql_expression("-2.050")
                .unwrap()
                .contains("'-2.050'")
        );
        let ScalarValue::Float64(Some(value)) = exact_numeric_scalar("-0.0").unwrap() else {
            panic!("negative floating zero must remain float64")
        };
        assert!(value.is_sign_negative());
        assert_eq!(
            exact_numeric_scalar("-9223372036854775808").unwrap(),
            ScalarValue::Int64(Some(i64::MIN))
        );
    }

    #[test]
    fn normalization_is_ast_based_for_expressions_and_queries() {
        let expression = normalize_sql_expression("1.25 + 2").unwrap();
        assert!(expression.contains("arrow_cast"));
        let query = normalize_sql_query(
            "SELECT 1.25 AS value, '1.25' AS label FROM values_table WHERE id = 2",
        )
        .unwrap();
        assert!(query.contains("arrow_cast"));
        assert!(query.contains("'1.25' AS label"));
        let signed = normalize_sql_expression("-9223372036854775808 + (+2.50)").unwrap();
        assert!(signed.contains("Int64"), "{signed}");
        assert!(signed.contains("Decimal128"), "{signed}");
    }
}
