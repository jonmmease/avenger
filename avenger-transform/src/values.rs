use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, AsArray, BooleanArray},
        compute::{cast, nullif},
        datatypes::DataType,
    },
    common::{plan_err, Result},
};

use crate::udf::Function;

pub(crate) fn numeric(t: &DataType) -> bool {
    matches!(
        t,
        DataType::Null
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float32
            | DataType::Float64
    )
}

fn string(t: &DataType) -> bool {
    matches!(t, DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View)
}

pub(crate) fn output_type(function: Function, t: &DataType) -> Result<DataType> {
    use Function::*;
    let accepted = numeric(t)
        || function != Numeric && string(t)
        || matches!(function, Truthy | Valid | Missing) && *t == DataType::Boolean;
    if !accepted {
        return plan_err!("{} does not support {t}", function.name());
    }
    Ok(match function {
        Truthy | Valid | Missing => DataType::Boolean,
        Numeric => DataType::Float64,
        _ => t.clone(),
    })
}

pub(crate) fn evaluate(function: Function, input: &ArrayRef) -> Result<ArrayRef> {
    use Function::*;
    let t = input.data_type();
    output_type(function, t)?;
    if function == Numeric {
        let input = cast(input, &DataType::Float64)?;
        let nan = BooleanArray::from_iter(
            input
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .iter()
                .map(|v| v.is_some_and(f64::is_nan)),
        );
        return Ok(nullif(input.as_ref(), &nan)?);
    }
    let flags = if numeric(t) {
        let numbers = cast(input, &DataType::Float64)?;
        BooleanArray::from_iter(
            numbers
                .as_primitive::<datafusion::arrow::datatypes::Float64Type>()
                .iter()
                .map(|v| match function {
                    Truthy => v.is_some_and(|v| v != 0.0 && !v.is_nan()),
                    Missing => v.is_none(),
                    Valid => v.is_some_and(|v| !v.is_nan()),
                    Clean => v.is_none_or(f64::is_nan),
                    _ => unreachable!(),
                }),
        )
    } else if string(t) {
        let strings = cast(input, &DataType::Utf8View)?;
        BooleanArray::from_iter(strings.as_string_view().iter().map(|v| {
            let present = v.is_some_and(|s| !s.is_empty());
            if matches!(function, Missing | Clean) {
                !present
            } else {
                present
            }
        }))
    } else {
        BooleanArray::from_iter(input.as_boolean().iter().map(|v| match function {
            Truthy => v.unwrap_or(false),
            Missing => v.is_none(),
            Valid => v.is_some(),
            _ => unreachable!(),
        }))
    };
    if function == Clean {
        Ok(nullif(input.as_ref(), &flags)?)
    } else {
        Ok(Arc::new(flags))
    }
}
