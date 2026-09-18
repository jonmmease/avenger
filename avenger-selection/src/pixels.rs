use std::{
    collections::{BTreeMap, HashMap},
    hash::{Hash, Hasher},
    ops::Bound,
    sync::Arc,
};

use avenger_scales_datafusion::{
    avenger_scales::scalar::Scalar, create_scale_udf, list_literal, options_literal, BuiltinScale,
};
use datafusion::{
    arrow::{
        array::{Array, ArrayRef, BooleanArray, Float32Array, Int64Array},
        compute::{cast, nullif},
        datatypes::{DataType, Field, TimeUnit},
    },
    common::{DataFusionError, Result as DFResult, ScalarValue},
    logical_expr::{
        ColumnarValue, Expr, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
};

use crate::{Error, ProducerDefinition, Result, SelectionValue, ValueTest};

/// A captured scale and grid in chart-local logical pixels.
///
/// Both brush endpoints and rows use the scale adapter's coercion and numeric
/// kernel, followed by Float64 cell arithmetic. Invalid row coordinates become
/// null cells. Device pixel ratio is not part of this configuration.
#[derive(Clone, Debug)]
pub struct PixelGrid(Arc<Grid>);
#[derive(Debug)]
struct Grid {
    key: GridKey,
    domain: ArrayRef,
    range: ArrayRef,
    options: HashMap<String, Scalar>,
    scale: ScalarUDF,
    arguments: Vec<ColumnarValue>,
    decreasing: bool,
    time_start: Option<i64>,
}
#[derive(Debug, PartialEq, Eq, Hash)]
struct GridKey {
    builtin: BuiltinScale,
    domain_type: DataType,
    domain: Vec<ScalarValue>,
    range_type: DataType,
    range: Vec<ScalarValue>,
    options: BTreeMap<String, (DataType, ScalarValue)>,
    origin: u64,
    size: u64,
}
impl PartialEq for PixelGrid {
    fn eq(&self, other: &Self) -> bool {
        self.0.key == other.0.key
    }
}
impl Eq for PixelGrid {}
impl Hash for PixelGrid {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.key.hash(state);
    }
}
impl PixelGrid {
    /// Capture a finite two-element linear or UTC temporal mapping.
    ///
    /// Linear options are `clamp`, finite `range_offset`, and `round: false`.
    /// Time supports only `timezone: "UTC"` (case insensitive). Domains are
    /// already resolved: normalization options such as `nice` are rejected.
    /// Domain/range endpoints must remain distinct in the scale kernel's precision.
    pub fn new(
        builtin: BuiltinScale,
        domain: ArrayRef,
        range: ArrayRef,
        mut options: HashMap<String, Scalar>,
        origin: f64,
        size: f64,
    ) -> Result<Self> {
        if !matches!(builtin, BuiltinScale::Linear | BuiltinScale::Time) {
            return Err(invalid(
                "pixel grids support only linear and UTC time scales",
            ));
        }
        if !origin.is_finite() || !size.is_finite() || size <= 0.0 {
            return Err(invalid(
                "grid origin must be finite and pixel size must be finite and positive",
            ));
        }
        let domain_values = pair_values(&domain)?;
        let range_values = pair_values(&range)?;
        let (r0, r1) = numeric_pair(&range)?;
        let (domain_decreasing, time_start) = if builtin == BuiltinScale::Linear {
            let (d0, d1) = numeric_pair(&domain)?;
            (d0 > d1, None)
        } else {
            match domain.data_type() {
                DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, None) => (),
                DataType::Timestamp(_, Some(tz)) if tz.eq_ignore_ascii_case("UTC") => (),
                _ => {
                    return Err(invalid(
                        "time grids require Date32, Date64, or UTC/zone-free timestamp domains",
                    ))
                }
            }
            let start = temporal_millis(&domain_values[0])
                .ok_or_else(|| invalid("time domain overflows milliseconds"))?;
            let end = temporal_millis(&domain_values[1])
                .ok_or_else(|| invalid("time domain overflows milliseconds"))?;
            if !matches!(end.checked_sub(start), Some(span) if span != 0) {
                return Err(invalid(
                    "time domain must have a nonzero, representable millisecond span",
                ));
            }
            (start > end, Some(start))
        };
        let mut option_key = BTreeMap::new();
        for (name, option) in &mut options {
            if option.0.len() != 1 || option.0.null_count() != 0 {
                return Err(invalid(format!("{name} must be one non-null scalar")));
            }
            let mut value = ScalarValue::try_from_array(&option.0, 0)?;
            let valid = match (builtin, name.as_str(), &value) {
                (BuiltinScale::Linear, "clamp", ScalarValue::Boolean(Some(_))) => true,
                (BuiltinScale::Linear, "round", ScalarValue::Boolean(Some(false))) => true,
                (BuiltinScale::Linear, "range_offset", _) if option.0.data_type().is_numeric() => {
                    let array =
                        cast(&option.0, &DataType::Float32).map_err(DataFusionError::from)?;
                    array.null_count() == 0
                        && array
                            .as_any()
                            .downcast_ref::<Float32Array>()
                            .unwrap()
                            .value(0)
                            .is_finite()
                }
                (BuiltinScale::Time, "timezone", ScalarValue::Utf8(Some(tz))) => {
                    tz.eq_ignore_ascii_case("UTC")
                }
                _ => false,
            };
            if !valid {
                return Err(invalid(format!(
                    "unsupported {builtin:?} pixel-grid option {name}: {value}"
                )));
            }
            if builtin == BuiltinScale::Time && name == "timezone" {
                *option = Scalar::from("UTC");
                value = ScalarValue::from("UTC");
            }
            option_key.insert(name.clone(), (value.data_type(), value));
        }
        let arguments = vec![
            literal(list_literal(domain.clone())?),
            literal(list_literal(range.clone())?),
            literal(options_literal(&options)?),
        ];
        let grid = Self(Arc::new(Grid {
            key: GridKey {
                builtin,
                domain_type: domain.data_type().clone(),
                domain: domain_values,
                range_type: range.data_type().clone(),
                range: range_values,
                options: option_key,
                origin: origin.to_bits(),
                size: size.to_bits(),
            },
            domain,
            range,
            options,
            scale: create_scale_udf(builtin)?,
            arguments,
            decreasing: domain_decreasing ^ (r0 > r1),
            time_start,
        }));
        // Validate the actual kernel, including overflow in its affine arithmetic.
        let mapped = grid.scale(ColumnarValue::Array(grid.0.domain.clone()), 2)?;
        let mapped = mapped.into_array(2)?;
        let mapped = mapped.as_any().downcast_ref::<Float32Array>().unwrap();
        if mapped.iter().any(|v| !v.is_some_and(f32::is_finite)) {
            return Err(invalid(
                "scale maps domain endpoints to non-finite coordinates",
            ));
        }
        Ok(grid)
    }
    /// Return the captured built-in scale descriptor.
    pub fn builtin(&self) -> BuiltinScale {
        self.0.key.builtin
    }
    /// Return the original, already resolved domain.
    pub fn domain(&self) -> &ArrayRef {
        &self.0.domain
    }
    /// Return the original numeric range in chart-local logical pixels.
    pub fn range(&self) -> &ArrayRef {
        &self.0.range
    }
    /// Return the captured checked scale options.
    pub fn options(&self) -> &HashMap<String, Scalar> {
        &self.0.options
    }
    /// Return the grid origin in logical pixels.
    pub fn origin(&self) -> f64 {
        f64::from_bits(self.0.key.origin)
    }
    /// Return logical pixels per cell.
    pub fn size(&self) -> f64 {
        f64::from_bits(self.0.key.size)
    }
    /// Whether increasing data values map toward decreasing coordinates.
    pub fn is_decreasing(&self) -> bool {
        self.0.decreasing
    }
    /// Map a scalar through the same kernel used by cell expressions.
    /// Invalid mapped coordinates return None. Incompatible input types are errors.
    pub fn cell(&self, value: &ScalarValue) -> Result<Option<i64>> {
        let result = self.evaluate(ColumnarValue::Scalar(value.clone()), 1)?;
        let ColumnarValue::Scalar(ScalarValue::Int64(cell)) = result else {
            unreachable!("scalar cell output")
        };
        Ok(cell)
    }
    /// Build a nullable Int64 cell expression for grouping or selection membership.
    /// Null cells retain invalid rows so clearing a selection can recover them.
    pub fn cell_expr(&self, value: Expr) -> Expr {
        ScalarUDF::from(PixelCell {
            grid: self.clone(),
            signature: Signature::user_defined(Volatility::Immutable),
        })
        .call(vec![value])
    }
    pub(crate) fn bounds(
        &self,
        lower: &Bound<ScalarValue>,
        upper: &Bound<ScalarValue>,
    ) -> Result<ValueTest> {
        let lower = self.bound(lower)?;
        let upper = self.bound(upper)?;
        let (lower, upper) = if self.is_decreasing() {
            (upper, lower)
        } else {
            (lower, upper)
        };
        Ok(ValueTest::Range { lower, upper })
    }
    fn bound(&self, bound: &Bound<ScalarValue>) -> Result<Bound<ScalarValue>> {
        let (value, included) = match bound {
            Bound::Unbounded => return Ok(Bound::Unbounded),
            Bound::Included(v) => (v, true),
            Bound::Excluded(v) => (v, false),
        };
        let finite = match value {
            ScalarValue::Float16(Some(v)) => v.is_finite(),
            ScalarValue::Float32(Some(v)) => v.is_finite(),
            ScalarValue::Float64(Some(v)) => v.is_finite(),
            _ => !value.is_null(),
        };
        if !finite {
            return Err(invalid("pixel bounds must be non-null and finite"));
        }
        let cell = self
            .cell(value)?
            .ok_or_else(|| invalid("pixel bound has no representable Int64 cell"))?;
        Ok(if included {
            Bound::Included(cell.into())
        } else {
            Bound::Excluded(cell.into())
        })
    }
    fn input_type(&self, input: DataType) -> DFResult<DataType> {
        let mut types: Vec<_> = self
            .0
            .arguments
            .iter()
            .map(ColumnarValue::data_type)
            .collect();
        types.push(input);
        Ok(self.0.scale.coerce_types(&types)?[3].clone())
    }
    fn scale(&self, value: ColumnarValue, rows: usize) -> DFResult<ColumnarValue> {
        let data_type = self.input_type(value.data_type())?;
        let scalar = matches!(value, ColumnarValue::Scalar(_));
        let array = cast(&value.into_array(rows)?, &data_type)?;
        let array = if let Some(start) = self.0.time_start {
            safe_time_values(array, start)?
        } else {
            array
        };
        let value = if scalar {
            ColumnarValue::Scalar(ScalarValue::try_from_array(&array, 0)?)
        } else {
            ColumnarValue::Array(array)
        };
        let mut args = self.0.arguments.clone();
        args.push(value);
        let fields = args
            .iter()
            .map(|a| Arc::new(Field::new("", a.data_type(), true)))
            .collect();
        self.0.scale.invoke_with_args(ScalarFunctionArgs {
            args,
            arg_fields: fields,
            number_rows: rows,
            return_field: Arc::new(Field::new("", DataType::Float32, true)),
            config_options: Arc::default(),
        })
    }
    fn evaluate(&self, value: ColumnarValue, rows: usize) -> DFResult<ColumnarValue> {
        let scalar = matches!(value, ColumnarValue::Scalar(_));
        let rows = if scalar { 1 } else { rows };
        let mapped = self.scale(value, rows)?.into_array(rows)?;
        let mapped = mapped.as_any().downcast_ref::<Float32Array>().unwrap();
        let cells = Int64Array::from_iter(
            mapped
                .iter()
                .map(|v| v.and_then(|v| cell_index(v, self.origin(), self.size()))),
        );
        if scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &cells, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(Arc::new(cells)))
        }
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidValue(message.into())
}
fn literal(expr: Expr) -> ColumnarValue {
    let Expr::Literal(v, _) = expr else {
        unreachable!("scale configuration literal")
    };
    ColumnarValue::Scalar(v)
}
fn pair_values(array: &ArrayRef) -> Result<Vec<ScalarValue>> {
    if array.len() != 2 || array.null_count() != 0 {
        return Err(invalid(
            "pixel domain and range must each have two non-null values",
        ));
    }
    Ok(vec![
        ScalarValue::try_from_array(array, 0)?,
        ScalarValue::try_from_array(array, 1)?,
    ])
}
fn numeric_pair(array: &ArrayRef) -> Result<(f32, f32)> {
    if !array.data_type().is_numeric() {
        return Err(invalid("linear domains and pixel ranges must be numeric"));
    }
    let array = cast(array, &DataType::Float32).map_err(DataFusionError::from)?;
    let array = array.as_any().downcast_ref::<Float32Array>().unwrap();
    let (a, b) = (array.value(0), array.value(1));
    if array.null_count() != 0 || !a.is_finite() || !b.is_finite() || a == b || !(b - a).is_finite()
    {
        return Err(invalid(
            "pixel domain and range must have finite, nonzero spans in scale precision",
        ));
    }
    Ok((a, b))
}
fn cell_index(value: f32, origin: f64, size: f64) -> Option<i64> {
    let cell = ((f64::from(value) - origin) / size).floor();
    // i64::MAX rounds up to 2^63 in Float64, so the upper check is exclusive.
    (cell.is_finite() && cell >= i64::MIN as f64 && cell < -(i64::MIN as f64))
        .then_some(cell as i64)
}
fn millis(raw: i64, t: &DataType) -> Option<i64> {
    match t {
        DataType::Date32 => raw.checked_mul(86_400_000),
        DataType::Date64 | DataType::Timestamp(TimeUnit::Millisecond, _) => Some(raw),
        DataType::Timestamp(TimeUnit::Second, _) => raw.checked_mul(1000),
        DataType::Timestamp(TimeUnit::Microsecond, _) => Some(raw / 1000),
        DataType::Timestamp(TimeUnit::Nanosecond, _) => Some(raw / 1_000_000),
        _ => None,
    }
}
fn temporal_millis(value: &ScalarValue) -> Option<i64> {
    let raw = match value {
        ScalarValue::Date32(Some(v)) => i64::from(*v),
        ScalarValue::Date64(Some(v))
        | ScalarValue::TimestampSecond(Some(v), _)
        | ScalarValue::TimestampMillisecond(Some(v), _)
        | ScalarValue::TimestampMicrosecond(Some(v), _)
        | ScalarValue::TimestampNanosecond(Some(v), _) => *v,
        _ => return None,
    };
    millis(raw, &value.data_type())
}
fn safe_time_values(array: ArrayRef, start: i64) -> DFResult<ArrayRef> {
    // The existing time kernel converts to milliseconds and subtracts in i64.
    // Mask overflow before invoking it, preserving identical scalar/batch behavior.
    let raw = cast(&array, &DataType::Int64)?;
    let raw = raw.as_any().downcast_ref::<Int64Array>().unwrap();
    let invalid = BooleanArray::from_iter(raw.iter().map(|v| {
        Some(v.is_some_and(|v| {
            millis(v, array.data_type())
                .and_then(|v| v.checked_sub(start))
                .is_none()
        }))
    }));
    nullif(&array, &invalid).map_err(Into::into)
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct PixelCell {
    grid: PixelGrid,
    signature: Signature,
}
impl ScalarUDFImpl for PixelCell {
    fn name(&self) -> &str {
        "avenger_selection_pixel_cell"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn coerce_types(&self, types: &[DataType]) -> DFResult<Vec<DataType>> {
        if types.len() != 1 {
            return Err(DataFusionError::Plan(
                "pixel cell requires one row value".into(),
            ));
        }
        Ok(vec![self.grid.input_type(types[0].clone())?])
    }
    fn return_type(&self, types: &[DataType]) -> DFResult<DataType> {
        self.coerce_types(types)?;
        Ok(DataType::Int64)
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        self.grid.evaluate(args.args[0].clone(), args.number_rows)
    }
}

pub(crate) fn effective_value(
    producer: &ProducerDefinition,
    value: &SelectionValue,
) -> Result<Option<SelectionValue>> {
    if producer.pixel_grids().next().is_none() {
        return Ok(None);
    }
    let SelectionValue::Tuples(mut tuples) = value.clone() else {
        unreachable!("pixel interval producer")
    };
    for tuple in &mut tuples {
        for term in &mut tuple.terms {
            match (producer.pixel_grid(&term.projection), &term.test) {
                (Some(grid), ValueTest::Range { lower, upper }) => {
                    term.test = grid.bounds(lower, upper)?
                }
                (Some(_), _) => {
                    return Err(invalid("pixel-mapped projections require range terms"))
                }
                (None, ValueTest::Range { .. }) => {
                    return Err(invalid(format!(
                        "pixel interval needs a grid for range projection {}",
                        term.projection
                    )))
                }
                (None, _) => (),
            }
        }
    }
    Ok(Some(SelectionValue::Tuples(tuples)))
}
