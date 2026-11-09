//! An argument of a constructor, a mark property, or an option of `scaled`: a constant `ScalarValue`
//! with its class, which the generated code converts to what a setter of the builders takes.

use std::sync::Arc;

use datafusion::{arrow::array::Array, common::ScalarValue};
use prost::{bytes::Bytes, Message};

use crate::{builder::Value, proto, FORMAT_VERSION};

/// The class of an argument, which decides the members that it can set.
pub(crate) enum Class<'a> {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Bytes,
    /// A value of another type, such as a date, which only a `ValueRef` takes.
    Other,
    /// A fragment, the value of a constructor.
    Fragment(&'a proto::Fragment),
}

impl Class<'_> {
    /// The class in the words of an error.
    pub(crate) fn describe(&self) -> String {
        match self {
            Class::Bool => "a Boolean".into(),
            Class::Int => "an integer".into(),
            Class::Float => "a floating-point number".into(),
            Class::Str => "a string".into(),
            Class::Array => "an array".into(),
            Class::Bytes => "bytes".into(),
            Class::Other => "a constant of another type".into(),
            Class::Fragment(fragment) => {
                let name = fragment
                    .r#type
                    .rsplit('.')
                    .next()
                    .unwrap_or(&fragment.r#type);
                match name.starts_with(['A', 'E', 'I', 'O', 'U']) {
                    true => format!("an `{name}`"),
                    false => format!("a `{name}`"),
                }
            }
        }
    }
}

/// A constant argument that is not `NULL`.
pub(crate) struct Arg {
    value: ScalarValue,
    fragment: Option<proto::Fragment>,
}

impl Arg {
    /// The argument of `value`, or `None` for `NULL`, which leaves its parameter unset.
    pub(crate) fn new(value: ScalarValue) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        let fragment = binary(&value).and_then(|bytes| {
            let fragment = proto::Fragment::decode(Bytes::copy_from_slice(bytes)).ok()?;
            let valid = fragment.version == FORMAT_VERSION
                && fragment.r#type.starts_with("avenger.chart.v1.");
            valid.then_some(fragment)
        });
        Some(Self { value, fragment })
    }

    /// The class of the argument.
    pub(crate) fn class(&self) -> Class<'_> {
        if let Some(fragment) = &self.fragment {
            return Class::Fragment(fragment);
        }
        match &self.value {
            ScalarValue::Boolean(_) => Class::Bool,
            ScalarValue::Int8(_)
            | ScalarValue::Int16(_)
            | ScalarValue::Int32(_)
            | ScalarValue::Int64(_)
            | ScalarValue::UInt8(_)
            | ScalarValue::UInt16(_)
            | ScalarValue::UInt32(_)
            | ScalarValue::UInt64(_) => Class::Int,
            ScalarValue::Float16(_) | ScalarValue::Float32(_) | ScalarValue::Float64(_) => {
                Class::Float
            }
            ScalarValue::Decimal128(..) | ScalarValue::Decimal256(..) => Class::Float,
            ScalarValue::Utf8(_) | ScalarValue::LargeUtf8(_) | ScalarValue::Utf8View(_) => {
                Class::Str
            }
            ScalarValue::List(_) | ScalarValue::LargeList(_) | ScalarValue::FixedSizeList(_) => {
                Class::Array
            }
            value if binary(value).is_some() => Class::Bytes,
            _ => Class::Other,
        }
    }

    /// An error that the argument is not what a field takes.
    fn takes(&self, what: &str) -> String {
        format!("takes {what}, not {}", self.class().describe())
    }

    pub(crate) fn boolean(&self) -> Result<bool, String> {
        match &self.value {
            ScalarValue::Boolean(Some(value)) => Ok(*value),
            _ => Err(self.takes("a Boolean")),
        }
    }

    /// The argument as an integer of type `T`, which holds its value.
    pub(crate) fn integer<T: TryFrom<i128>>(&self) -> Result<T, String> {
        let value: i128 = match &self.value {
            ScalarValue::Int8(Some(v)) => (*v).into(),
            ScalarValue::Int16(Some(v)) => (*v).into(),
            ScalarValue::Int32(Some(v)) => (*v).into(),
            ScalarValue::Int64(Some(v)) => (*v).into(),
            ScalarValue::UInt8(Some(v)) => (*v).into(),
            ScalarValue::UInt16(Some(v)) => (*v).into(),
            ScalarValue::UInt32(Some(v)) => (*v).into(),
            ScalarValue::UInt64(Some(v)) => (*v).into(),
            _ => return Err(self.takes("an integer")),
        };
        T::try_from(value).map_err(|_| format!("takes an integer in its range, not {value}"))
    }

    pub(crate) fn float64(&self) -> Result<f64, String> {
        match self.class() {
            Class::Int | Class::Float => {}
            _ => return Err(self.takes("a number")),
        }
        let value = self
            .value
            .cast_to(&datafusion::arrow::datatypes::DataType::Float64)
            .map_err(|error| error.to_string())?;
        match value {
            ScalarValue::Float64(Some(value)) => Ok(value),
            _ => Err(self.takes("a number")),
        }
    }

    pub(crate) fn float32(&self) -> Result<f32, String> {
        self.float64().map(|value| value as f32)
    }

    pub(crate) fn string(&self) -> Result<String, String> {
        match &self.value {
            ScalarValue::Utf8(Some(value))
            | ScalarValue::LargeUtf8(Some(value))
            | ScalarValue::Utf8View(Some(value)) => Ok(value.clone()),
            _ => Err(self.takes("a string")),
        }
    }

    pub(crate) fn bytes(&self) -> Result<Bytes, String> {
        match binary(&self.value) {
            Some(bytes) if self.fragment.is_none() => Ok(Bytes::copy_from_slice(bytes)),
            _ => Err(self.takes("bytes")),
        }
    }

    /// The argument as a value of a `ValueRef`: a scalar, or a list for an array.
    pub(crate) fn value(&self) -> Result<Value, String> {
        if self.fragment.is_some() {
            return Err(self.takes("a constant"));
        }
        let array = self.value.to_array_of_size(1).map_err(|e| e.to_string())?;
        Ok(Value::scalar(array))
    }

    /// The items of an array argument, each an argument of its own. A `NULL` item is an error.
    pub(crate) fn items(&self) -> Result<Vec<Arg>, String> {
        let list: Arc<dyn Array> = match &self.value {
            ScalarValue::List(list) => list.value(0),
            ScalarValue::LargeList(list) => list.value(0),
            ScalarValue::FixedSizeList(list) => list.value(0),
            _ => return Err(self.takes("an array")),
        };
        (0..list.len())
            .map(|index| {
                let item = ScalarValue::try_from_array(&list, index).map_err(|e| e.to_string())?;
                Arg::new(item).ok_or_else(|| "takes an array without NULL items".to_string())
            })
            .collect()
    }
}

/// The bytes of a binary value.
fn binary(value: &ScalarValue) -> Option<&[u8]> {
    match value {
        ScalarValue::Binary(Some(bytes))
        | ScalarValue::LargeBinary(Some(bytes))
        | ScalarValue::BinaryView(Some(bytes))
        | ScalarValue::FixedSizeBinary(_, Some(bytes)) => Some(bytes),
        _ => None,
    }
}
