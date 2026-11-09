use std::sync::Arc;

use crate::error::AvengerScaleError;
use arrow::{
    array::{
        Array, ArrayRef, AsArray, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array,
        Int64Array, ListArray, ListBuilder, StringArray, StringBuilder, UInt32Array,
    },
    compute::{kernels::cast, take},
    datatypes::{DataType, Float32Type, Float64Type},
    row::{RowConverter, SortField},
};
use avenger_color::parse_color_string;

/// A scalar value wrapper around a single-element Arrow array
#[derive(Debug, Clone)]
pub struct Scalar(pub ArrayRef);

/// Scalars are equal when they have one type and one value, which Arrow's row format compares for
/// every type: a NaN equals a NaN, and `0.0` and `-0.0` differ, so equal scalars hash equally.
impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        self.0.data_type() == other.0.data_type() && self.row() == other.row()
    }
}

impl Eq for Scalar {}

impl std::hash::Hash for Scalar {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.data_type().hash(state);
        self.row().hash(state);
    }
}

impl Scalar {
    /// Create a new scalar from a single-element array
    pub fn new(array: ArrayRef) -> Self {
        assert_eq!(array.len(), 1, "Scalar array must have exactly one element");
        Self(array)
    }

    /// Get the underlying array
    pub fn array(&self) -> &ArrayRef {
        &self.0
    }

    /// The value's bytes in Arrow's row format, which compare as the value does.
    fn row(&self) -> Option<Box<[u8]>> {
        let converter = RowConverter::new(vec![SortField::new(self.0.data_type().clone())]).ok()?;
        let rows = converter
            .convert_columns(std::slice::from_ref(&self.0))
            .ok()?;
        Some(rows.row(0).as_ref().into())
    }

    /// Convert to f64
    pub fn as_f64(&self) -> Result<f64, AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::Int16 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int16Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int16Array".to_string(),
                        )
                    })?;
                Ok(f64::from(array.value(0)))
            }
            DataType::Int32 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int32Array".to_string(),
                        )
                    })?;
                Ok(f64::from(array.value(0)))
            }
            DataType::Int64 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int64Array".to_string(),
                        )
                    })?;
                Ok(array.value(0) as f64)
            }
            DataType::Float32 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Float32Array".to_string(),
                        )
                    })?;
                Ok(f64::from(array.value(0)))
            }
            DataType::Float64 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Float64Array".to_string(),
                        )
                    })?;
                Ok(array.value(0))
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to f64: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to boolean
    pub fn as_boolean(&self) -> Result<bool, AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::Boolean => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to BooleanArray".to_string(),
                        )
                    })?;
                Ok(array.value(0))
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to boolean: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to i32
    pub fn as_i32(&self) -> Result<i32, AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::Int16 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int16Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int16Array".to_string(),
                        )
                    })?;
                Ok(array.value(0) as i32)
            }
            DataType::Int32 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int32Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int32Array".to_string(),
                        )
                    })?;
                Ok(array.value(0))
            }
            DataType::Int64 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Int64Array".to_string(),
                        )
                    })?;
                Ok(array.value(0) as i32)
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to i32: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to string
    pub fn as_string(&self) -> Result<String, AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                // Normalize all string-like Arrow types to Utf8 for extraction.
                let casted = cast::cast(&self.0, &DataType::Utf8).map_err(|e| {
                    AvengerScaleError::InternalError(format!(
                        "Failed to cast string-like scalar to Utf8: {e}"
                    ))
                })?;
                let array = casted.as_string::<i32>();
                Ok(array.value(0).to_string())
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to string: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// The numbers in a list scalar.
    pub fn as_f64_list(&self) -> Result<Vec<f64>, AvengerScaleError> {
        let element = self.list_element()?;
        if !element.data_type().is_numeric() {
            return Err(AvengerScaleError::InternalError(format!(
                "List scalar of {:?} is not a list of numbers",
                element.data_type()
            )));
        }
        let element = cast::cast(&element, &DataType::Float64)?;
        let values = element.as_primitive::<Float64Type>();
        if values.null_count() > 0 {
            return Err(AvengerScaleError::InternalError(
                "List scalar contains null values".to_string(),
            ));
        }
        Ok(values.values().to_vec())
    }

    /// The strings in a list scalar.
    pub fn as_string_list(&self) -> Result<Vec<String>, AvengerScaleError> {
        let element = self.list_element()?;
        if !matches!(
            element.data_type(),
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
        ) {
            return Err(AvengerScaleError::InternalError(format!(
                "List scalar of {:?} is not a list of strings",
                element.data_type()
            )));
        }
        let element = cast::cast(&element, &DataType::Utf8)?;
        let values = element.as_string::<i32>();
        if values.null_count() > 0 {
            return Err(AvengerScaleError::InternalError(
                "List scalar contains null values".to_string(),
            ));
        }
        Ok(values.iter().flatten().map(str::to_string).collect())
    }

    /// The single list of a list scalar.
    fn list_element(&self) -> Result<ArrayRef, AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }
        match self.0.as_list_opt::<i32>() {
            Some(list) => Ok(list.value(0)),
            None => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not a list: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to [f32; 2] from list
    pub fn as_f32_2(&self) -> Result<[f32; 2], AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::List(_) => {
                let list_array = self.0.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
                    AvengerScaleError::InternalError("Failed to downcast to ListArray".to_string())
                })?;

                let element = list_array.value(0);
                let element = cast(&element, &DataType::Float32)?;
                let array = element.as_primitive::<Float32Type>();

                if array.len() != 2 {
                    return Err(AvengerScaleError::InternalError(format!(
                        "List array length {} is not 2 for [f32; 2] conversion",
                        array.len()
                    )));
                }

                Ok([array.value(0), array.value(1)])
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to [f32; 2]: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to [f32; 4] from list
    pub fn as_f32_4(&self) -> Result<[f32; 4], AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        match self.0.data_type() {
            DataType::List(_) => {
                let list_array = self.0.as_any().downcast_ref::<ListArray>().ok_or_else(|| {
                    AvengerScaleError::InternalError("Failed to downcast to ListArray".to_string())
                })?;

                let element = list_array.value(0);
                let element = cast(&element, &DataType::Float32)?;
                let array = element.as_primitive::<Float32Type>();

                if array.len() != 4 {
                    return Err(AvengerScaleError::InternalError(format!(
                        "List array length {} is not 4 for [f32; 4] conversion",
                        array.len()
                    )));
                }

                Ok([
                    array.value(0),
                    array.value(1),
                    array.value(2),
                    array.value(3),
                ])
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to [f32; 4]: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to RGBA color array [f32; 4]
    pub fn as_rgba(&self) -> Result<[f32; 4], AvengerScaleError> {
        if self.0.is_null(0) {
            return Err(AvengerScaleError::InternalError(
                "Scalar contains null value".to_string(),
            ));
        }

        // First try as f32_4 (assuming it's already RGBA)
        if let Ok(rgba) = self.as_f32_4() {
            return Ok(rgba);
        }

        // Try as string color
        match self.0.data_type() {
            DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => {
                let color_str = self.as_string()?;
                parse_color_string(&color_str).ok_or_else(|| {
                    AvengerScaleError::InternalError(format!(
                        "Scalar string is not a valid color: {color_str}"
                    ))
                })
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to RGBA color: {:?}",
                self.0.data_type()
            ))),
        }
    }

    /// Convert to ArrayRef (returns the underlying array)
    pub fn to_array(&self) -> ArrayRef {
        self.0.clone()
    }

    /// Get the data type of the underlying array
    pub fn data_type(&self) -> &DataType {
        self.0.data_type()
    }

    /// Create scalar from f64
    pub fn from_f64(value: f64) -> Self {
        Self::new(Arc::new(Float64Array::from(vec![value])))
    }

    /// Create scalar from bool
    pub fn from_bool(value: bool) -> Self {
        Self::new(Arc::new(BooleanArray::from(vec![value])))
    }

    /// Create scalar from i32
    pub fn from_i32(value: i32) -> Self {
        Self::new(Arc::new(Int32Array::from(vec![value])))
    }

    /// Create scalar from string
    pub fn from_string(value: &str) -> Self {
        Self::new(Arc::new(StringArray::from(vec![value])))
    }

    /// Create a list scalar of numbers.
    pub fn from_f64_list(values: &[f64]) -> Self {
        Self::new(Arc::new(
            ListArray::from_iter_primitive::<Float64Type, _, _>(vec![Some(
                values.iter().copied().map(Some),
            )]),
        ))
    }

    /// Create a list scalar of strings.
    pub fn from_string_list(values: &[&str]) -> Self {
        let mut builder = ListBuilder::new(StringBuilder::new());
        for value in values {
            builder.values().append_value(value);
        }
        builder.append(true);
        Self::new(Arc::new(builder.finish()))
    }

    /// Create RGBA scalar from components
    pub fn make_rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        let list_array = ListArray::from_iter_primitive::<Float32Type, _, _>(vec![Some(vec![
            Some(r),
            Some(g),
            Some(b),
            Some(a),
        ])]);

        Self::new(Arc::new(list_array))
    }

    /// The value at `index` of `array`, or an error for a null element. A Float32 value is a
    /// Float64 scalar and a LargeUtf8 or a Utf8View value is a Utf8 scalar, as the scalars that
    /// `from_f64` and `from_string` make are, and a value of any other type keeps its type.
    pub fn try_from_array(array: &dyn Array, index: usize) -> Result<Self, AvengerScaleError> {
        if array.is_null(index) {
            return Err(AvengerScaleError::InternalError(
                "Array element is null".to_string(),
            ));
        }
        // A copy of the one row, which does not keep the rest of the array.
        let value = take(array, &UInt32Array::from(vec![index as u32]), None)?;
        let normal = match array.data_type() {
            DataType::Float32 => Some(DataType::Float64),
            DataType::LargeUtf8 | DataType::Utf8View => Some(DataType::Utf8),
            _ => None,
        };
        Ok(Self::new(match normal {
            Some(data_type) => cast::cast(&value, &data_type)?,
            None => value,
        }))
    }

    /// Create array from iterator of values
    pub fn iter_to_array<T>(values: Vec<T>) -> Result<ArrayRef, AvengerScaleError>
    where
        T: Into<Scalar>,
    {
        if values.is_empty() {
            return Err(AvengerScaleError::InternalError(
                "Cannot create array from empty values".to_string(),
            ));
        }

        let scalars: Vec<Scalar> = values.into_iter().map(|v| v.into()).collect();
        let arrays: Vec<ArrayRef> = scalars.into_iter().map(|s| s.to_array()).collect();

        arrow::compute::concat(&arrays.iter().map(|a| a.as_ref()).collect::<Vec<_>>()).map_err(
            |e| AvengerScaleError::InternalError(format!("Failed to concatenate arrays: {e}")),
        )
    }

    /// Create a list array from vector of arrays (replacement for arrays_into_list_array)
    pub fn arrays_into_list_array(arrays: Vec<ArrayRef>) -> Result<ArrayRef, AvengerScaleError> {
        use arrow::array::ListArray;

        if arrays.is_empty() {
            return Err(AvengerScaleError::InternalError(
                "Cannot create list array from empty arrays".to_string(),
            ));
        }

        // Convert each array to Float32 values and create list array
        let values: Result<Vec<_>, AvengerScaleError> = arrays
            .into_iter()
            .map(|arr| {
                // Convert array to Float32Array and extract values
                let cast_arr = arrow::compute::cast(&arr, &DataType::Float32).map_err(|e| {
                    AvengerScaleError::InternalError(format!("Failed to cast array: {e}"))
                })?;
                let float_arr = cast_arr.as_primitive::<Float32Type>();
                let values: Vec<Option<f32>> = (0..float_arr.len())
                    .map(|i| {
                        if float_arr.is_null(i) {
                            None
                        } else {
                            Some(float_arr.value(i))
                        }
                    })
                    .collect();
                Ok(values)
            })
            .collect();

        let list_array =
            ListArray::from_iter_primitive::<Float32Type, _, _>(values?.into_iter().map(Some));

        Ok(Arc::new(list_array))
    }
}

// From trait implementations for easy conversion
impl From<f64> for Scalar {
    fn from(value: f64) -> Self {
        Self::from_f64(value)
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::from_bool(value)
    }
}

impl From<i32> for Scalar {
    fn from(value: i32) -> Self {
        Self::from_i32(value)
    }
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::from_string(value)
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::from_string(&value)
    }
}

impl From<Vec<f64>> for Scalar {
    fn from(values: Vec<f64>) -> Self {
        Self::from_f64_list(&values)
    }
}

impl From<Vec<&str>> for Scalar {
    fn from(values: Vec<&str>) -> Self {
        Self::from_string_list(&values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::StringArray;

    #[test]
    fn list_scalars_hold_numbers_or_strings() {
        assert_eq!(
            Scalar::from(vec![0.5, 2.0]).as_f64_list().unwrap(),
            vec![0.5, 2.0]
        );
        assert_eq!(
            Scalar::from(vec!["free", "shared"])
                .as_string_list()
                .unwrap(),
            vec!["free", "shared"]
        );
        assert!(Scalar::from(vec!["free"]).as_f64_list().is_err());
        assert!(Scalar::from(0.5).as_f64_list().is_err());
    }

    #[test]
    fn scalar_try_from_utf8_view() -> Result<(), AvengerScaleError> {
        let utf8 = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
        let utf8_view = cast::cast(&utf8, &DataType::Utf8View).map_err(|e| {
            AvengerScaleError::InternalError(format!("Failed to cast to Utf8View: {e}"))
        })?;

        let scalar = Scalar::try_from_array(utf8_view.as_ref(), 1)?;
        assert_eq!(scalar.as_string()?, "b");
        assert_eq!(scalar.data_type(), &DataType::Utf8);

        Ok(())
    }

    #[test]
    fn scalars_of_other_types_keep_their_type_and_compare_by_value() -> Result<(), AvengerScaleError>
    {
        use arrow::array::{Int64Array, StructArray};
        use arrow::datatypes::Field;
        use std::hash::{BuildHasher, RandomState};

        let lumps = |values: Vec<Option<i64>>, others: Vec<Option<&str>>| -> ArrayRef {
            Arc::new(StructArray::from(vec![
                (
                    Arc::new(Field::new("value", DataType::Int64, true)),
                    Arc::new(Int64Array::from(values)) as ArrayRef,
                ),
                (
                    Arc::new(Field::new("other", DataType::Utf8, true)),
                    Arc::new(StringArray::from(others)) as ArrayRef,
                ),
            ]))
        };
        let a = lumps(vec![Some(4), None], vec![None, Some("Other")]);
        let b = lumps(vec![None, Some(8)], vec![Some("Other"), None]);
        let other = Scalar::try_from_array(a.as_ref(), 1)?;
        assert_eq!(other.data_type(), a.data_type());
        assert_eq!(other.0.len(), 1);
        assert_eq!(other, Scalar::try_from_array(b.as_ref(), 0)?);
        assert_ne!(other, Scalar::try_from_array(a.as_ref(), 0)?);
        let hasher = RandomState::new();
        assert_eq!(
            hasher.hash_one(&other),
            hasher.hash_one(Scalar::try_from_array(b.as_ref(), 0)?)
        );

        // Floats compare by value as their rows do.
        assert_eq!(Scalar::from_f64(f64::NAN), Scalar::from_f64(f64::NAN));
        assert_ne!(Scalar::from_f64(0.0), Scalar::from_f64(-0.0));
        let float32: ArrayRef = Arc::new(Float32Array::from(vec![0.5]));
        assert_eq!(
            Scalar::try_from_array(float32.as_ref(), 0)?,
            Scalar::from_f64(0.5)
        );
        Ok(())
    }
}
