use std::sync::Arc;

use crate::error::AvengerScaleError;
use arrow::{
    array::{
        Array, ArrayRef, AsArray, BooleanArray, Float32Array, Float64Array, Int16Array, Int32Array,
        Int64Array, ListArray, StringArray,
    },
    compute::kernels::cast,
    datatypes::{DataType, Float32Type},
};
use css_color_parser::Color;

/// A scalar value wrapper around a single-element Arrow array
#[derive(Debug, Clone)]
pub struct Scalar(pub ArrayRef);

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        if self.0.data_type() != other.0.data_type() {
            return false;
        }

        // Compare based on data type
        match self.0.data_type() {
            DataType::Boolean => match (self.as_boolean(), other.as_boolean()) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
            DataType::Int32 => match (self.as_i32(), other.as_i32()) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
            DataType::Float32 => match (self.as_f32(), other.as_f32()) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
            DataType::Utf8 => match (self.as_string(), other.as_string()) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            },
            _ => {
                // For other types, compare the raw arrays
                format!("{:?}", self.0) == format!("{:?}", other.0)
            }
        }
    }
}

impl Eq for Scalar {}

impl std::hash::Hash for Scalar {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash based on data type and the scalar value
        self.0.data_type().hash(state);

        // Hash the actual value based on type
        match self.0.data_type() {
            DataType::Boolean => {
                if let Ok(value) = self.as_boolean() {
                    value.hash(state);
                }
            }
            DataType::Int32 => {
                if let Ok(value) = self.as_i32() {
                    value.hash(state);
                }
            }
            DataType::Float32 => {
                if let Ok(value) = self.as_f32() {
                    // Use bit representation for float hashing
                    value.to_bits().hash(state);
                }
            }
            DataType::Utf8 => {
                if let Ok(value) = self.as_string() {
                    value.hash(state);
                }
            }
            _ => {
                // For other types, hash the raw bytes if possible
                // This is a fallback - may not be perfect but should work
                format!("{:?}", self.0).hash(state);
            }
        }
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

    /// Convert to f32
    pub fn as_f32(&self) -> Result<f32, AvengerScaleError> {
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
                Ok(array.value(0) as f32)
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
                Ok(array.value(0) as f32)
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
                Ok(array.value(0) as f32)
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
                Ok(array.value(0))
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
                Ok(array.value(0) as f32)
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to f32: {:?}",
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
            DataType::Utf8 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to StringArray".to_string(),
                        )
                    })?;
                Ok(array.value(0).to_string())
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Scalar is not convertable to string: {:?}",
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
            DataType::Utf8 => {
                let array = self
                    .0
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to StringArray".to_string(),
                        )
                    })?;

                let color_str = array.value(0);
                match color_str.parse::<Color>() {
                    Ok(color) => Ok([
                        color.r as f32 / 255.0,
                        color.g as f32 / 255.0,
                        color.b as f32 / 255.0,
                        color.a,
                    ]),
                    Err(e) => Err(AvengerScaleError::InternalError(format!(
                        "Scalar string is not a valid color: {}\n{:?}",
                        color_str, e
                    ))),
                }
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

    /// Create scalar from f32
    pub fn from_f32(value: f32) -> Self {
        Self::new(Arc::new(Float32Array::from(vec![value])))
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

    /// Extract scalar from array at index
    pub fn try_from_array(array: &dyn Array, index: usize) -> Result<Self, AvengerScaleError> {
        if array.is_null(index) {
            return Err(AvengerScaleError::InternalError(
                "Array element is null".to_string(),
            ));
        }

        match array.data_type() {
            DataType::Boolean => {
                let arr = array
                    .as_any()
                    .downcast_ref::<BooleanArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to BooleanArray".to_string(),
                        )
                    })?;
                Ok(Self::from_bool(arr.value(index)))
            }
            DataType::Int32 => {
                let arr = array.as_any().downcast_ref::<Int32Array>().ok_or_else(|| {
                    AvengerScaleError::InternalError("Failed to downcast to Int32Array".to_string())
                })?;
                Ok(Self::from_i32(arr.value(index)))
            }
            DataType::Float32 => {
                let arr = array
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to Float32Array".to_string(),
                        )
                    })?;
                Ok(Self::from_f32(arr.value(index)))
            }
            DataType::Utf8 => {
                let arr = array
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| {
                        AvengerScaleError::InternalError(
                            "Failed to downcast to StringArray".to_string(),
                        )
                    })?;
                Ok(Self::from_string(arr.value(index)))
            }
            _ => Err(AvengerScaleError::InternalError(format!(
                "Unsupported data type: {:?}",
                array.data_type()
            ))),
        }
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
            |e| AvengerScaleError::InternalError(format!("Failed to concatenate arrays: {}", e)),
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
                    AvengerScaleError::InternalError(format!("Failed to cast array: {}", e))
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
impl From<f32> for Scalar {
    fn from(value: f32) -> Self {
        Self::from_f32(value)
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
