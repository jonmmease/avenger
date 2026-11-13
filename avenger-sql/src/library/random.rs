//! A seeded random number of a row, for `TABLESAMPLE ... REPEATABLE (seed)`: a hash of the seed
//! and the row's number, so the same rows in the same order take the same numbers.

use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::{
        array::{Array, ArrayRef, AsArray, Float64Array},
        datatypes::{DataType, Int64Type},
    },
    common::{Result, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

/// A number in `[0, 1)` of the `INT64`s `seed` and `n`.
pub(crate) fn seeded(seed: Expr, n: Expr) -> Expr {
    SEEDED.call(vec![seed, n])
}

static SEEDED: LazyLock<Arc<ScalarUDF>> = LazyLock::new(|| {
    Arc::new(ScalarUDF::new_from_impl(Seeded {
        signature: Signature::exact(
            vec![DataType::Int64, DataType::Int64],
            Volatility::Immutable,
        ),
    }))
});

#[derive(Debug, PartialEq, Eq, Hash)]
struct Seeded {
    signature: Signature,
}

/// The splitmix64 mix of `value`.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

impl ScalarUDFImpl for Seeded {
    fn name(&self) -> &str {
        "$seeded_random"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _arg_types: &[DataType]) -> Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let arrays = ColumnarValue::values_to_arrays(&args.args)?;
        let (seed, n) = (
            arrays[0].as_primitive::<Int64Type>(),
            arrays[1].as_primitive::<Int64Type>(),
        );
        let values: Float64Array = (0..seed.len())
            .map(|row| {
                if seed.is_null(row) || n.is_null(row) {
                    return None;
                }
                let bits = mix(mix(seed.value(row) as u64) ^ n.value(row) as u64);
                Some((bits >> 11) as f64 / (1u64 << 53) as f64)
            })
            .collect();
        let values: ArrayRef = Arc::new(values);
        match args
            .args
            .iter()
            .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
        {
            true => Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &values, 0,
            )?)),
            false => Ok(ColumnarValue::Array(values)),
        }
    }
}
