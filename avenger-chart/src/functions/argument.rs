//! An argument of a function of the format: a constant that is not null, with its class, which the
//! generated code converts to what a setter of the builders takes.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{
        new_null_array, Array, ArrayRef, AsArray, BinaryArray, BooleanArray, Float64Array,
        Int64Array, StringArray, StructArray,
    },
    compute::{cast, concat},
    datatypes::{self, DataType, Field},
};
use prost::{bytes::Bytes, Message};

use super::{
    error::{ArgError, Expected},
    Shape,
};
use crate::{builder::Value, proto, FORMAT_VERSION};

/// A constant argument that is not null. `None` in the arguments of a function leaves its
/// parameter unset.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Bool(bool),
    Int(i128),
    Float(f64),
    Str(String),
    Bytes(Bytes),
    /// An array, whose items can be null where a `ValueRef` stores it, such as the null category
    /// of an explicit domain.
    List(Vec<Option<Arg>>),
    /// A struct: its fields in order, each with its name and its value, where a null field is
    /// `None`, as the named options of a custom scale or projection take it.
    Struct(Vec<(String, Option<Arg>)>),
    /// A fragment, the value of a constructor.
    Fragment(proto::Fragment),
    /// A value of another type, such as a date, as an array of one element, which only a
    /// `ValueRef` takes.
    Value(ArrayRef),
}

/// The class of an argument, which decides the members that it can set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Bool,
    Int,
    Float,
    Str,
    Array,
    Bytes,
    /// A struct, whose fields are named values.
    Struct,
    /// A value of another type, such as a date, which only a `ValueRef` takes.
    Other,
    /// A fragment of a message.
    Fragment,
}

impl Class {
    /// The class in the words of an error.
    pub fn describe(self) -> &'static str {
        match self {
            Class::Bool => "a Boolean",
            Class::Int => "an integer",
            Class::Float => "a floating-point number",
            Class::Str => "a string",
            Class::Array => "an array",
            Class::Bytes => "bytes",
            Class::Struct => "a struct",
            Class::Other => "a constant of another type",
            Class::Fragment => "a fragment",
        }
    }
}

impl Arg {
    /// The argument of the element `index` of `array`, or `None` for a null element. An integer
    /// is an `Int`, a floating-point number or a decimal a `Float`, a string a `Str`, a list a
    /// `List` of its items, a binary element that holds a fragment of this version of the format
    /// a `Fragment`, another binary element `Bytes`, a struct a `Struct` of its fields, and an
    /// element of another type a `Value`.
    pub fn from_array(array: &dyn Array, index: usize) -> Option<Arg> {
        // An array of the type `Null` has no null buffer, so `is_null` is false for its elements.
        if array.is_null(index) || *array.data_type() == DataType::Null {
            return None;
        }
        macro_rules! int {
            ($type:ty) => {
                Arg::Int(array.as_primitive::<$type>().value(index).into())
            };
        }
        Some(match array.data_type() {
            DataType::Boolean => Arg::Bool(array.as_boolean().value(index)),
            DataType::Int8 => int!(datatypes::Int8Type),
            DataType::Int16 => int!(datatypes::Int16Type),
            DataType::Int32 => int!(datatypes::Int32Type),
            DataType::Int64 => int!(datatypes::Int64Type),
            DataType::UInt8 => int!(datatypes::UInt8Type),
            DataType::UInt16 => int!(datatypes::UInt16Type),
            DataType::UInt32 => int!(datatypes::UInt32Type),
            DataType::UInt64 => int!(datatypes::UInt64Type),
            DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Decimal32(..)
            | DataType::Decimal64(..)
            | DataType::Decimal128(..)
            | DataType::Decimal256(..) => {
                let element = array.slice(index, 1);
                match cast(&element, &DataType::Float64) {
                    Ok(float) => {
                        Arg::Float(float.as_primitive::<datatypes::Float64Type>().value(0))
                    }
                    Err(_) => Arg::Value(element),
                }
            }
            DataType::Utf8 => Arg::Str(array.as_string::<i32>().value(index).to_string()),
            DataType::LargeUtf8 => Arg::Str(array.as_string::<i64>().value(index).to_string()),
            DataType::Utf8View => Arg::Str(array.as_string_view().value(index).to_string()),
            DataType::List(_) => items(&array.as_list::<i32>().value(index)),
            DataType::LargeList(_) => items(&array.as_list::<i64>().value(index)),
            DataType::FixedSizeList(..) => items(&array.as_fixed_size_list().value(index)),
            DataType::Binary => binary(array.as_binary::<i32>().value(index)),
            DataType::LargeBinary => binary(array.as_binary::<i64>().value(index)),
            DataType::BinaryView => binary(array.as_binary_view().value(index)),
            DataType::FixedSizeBinary(_) => binary(array.as_fixed_size_binary().value(index)),
            DataType::Struct(fields) => {
                let array = array.as_struct();
                Arg::Struct(
                    (fields.iter().zip(array.columns()))
                        .map(|(field, column)| {
                            (
                                field.name().clone(),
                                Arg::from_array(column.as_ref(), index),
                            )
                        })
                        .collect(),
                )
            }
            _ => Arg::Value(array.slice(index, 1)),
        })
    }

    /// The class of the argument.
    pub fn class(&self) -> Class {
        match self {
            Arg::Bool(_) => Class::Bool,
            Arg::Int(_) => Class::Int,
            Arg::Float(_) => Class::Float,
            Arg::Str(_) => Class::Str,
            Arg::Bytes(_) => Class::Bytes,
            Arg::List(_) => Class::Array,
            Arg::Struct(_) => Class::Struct,
            Arg::Fragment(_) => Class::Fragment,
            Arg::Value(_) => Class::Other,
        }
    }

    /// An error that the argument is not a value of `shape`.
    fn rejected(&self, shape: &'static Shape) -> ArgError {
        ArgError::rejected(Expected::Shape(shape), self)
    }

    pub(crate) fn boolean(&self) -> Result<bool, ArgError> {
        match self {
            Arg::Bool(value) => Ok(*value),
            _ => Err(self.rejected(&Shape::Bool)),
        }
    }

    /// The argument as an integer of type `T`, which holds its value.
    pub(crate) fn integer<T: TryFrom<i128>>(&self) -> Result<T, ArgError> {
        match self {
            Arg::Int(value) => T::try_from(*value).map_err(|_| ArgError::range(*value)),
            _ => Err(self.rejected(&Shape::Int)),
        }
    }

    pub(crate) fn float64(&self) -> Result<f64, ArgError> {
        match self {
            Arg::Int(value) => Ok(*value as f64),
            Arg::Float(value) => Ok(*value),
            _ => Err(self.rejected(&Shape::Float)),
        }
    }

    pub(crate) fn float32(&self) -> Result<f32, ArgError> {
        self.float64().map(|value| value as f32)
    }

    pub(crate) fn string(&self) -> Result<String, ArgError> {
        match self {
            Arg::Str(value) => Ok(value.clone()),
            _ => Err(self.rejected(&Shape::Str)),
        }
    }

    pub(crate) fn bytes(&self) -> Result<Bytes, ArgError> {
        match self {
            Arg::Bytes(bytes) => Ok(bytes.clone()),
            _ => Err(self.rejected(&Shape::Bytes)),
        }
    }

    /// The argument as a value of a `ValueRef`: a scalar of the argument's type, or a list of the
    /// items of an array.
    pub(crate) fn value(&self) -> Result<Value, ArgError> {
        Ok(match self {
            Arg::List(items) => {
                Value::list(list_items(items).ok_or_else(|| self.rejected(&Shape::Value))?)
            }
            arg => Value::scalar(arg.array().ok_or_else(|| self.rejected(&Shape::Value))?),
        })
    }

    /// The named options of a struct argument, one for each field that is not null: a Boolean, a
    /// number, an integer, a string, or an array of numbers or of strings. Two fields of one name
    /// are an error.
    pub(crate) fn options(&self) -> Result<BTreeMap<String, proto::OptionValue>, ArgError> {
        let Arg::Struct(fields) = self else {
            return Err(self.rejected(&Shape::Options));
        };
        let mut options = BTreeMap::new();
        for (name, value) in fields {
            let Some(value) = value else {
                continue;
            };
            let value = option_value(value).ok_or_else(|| {
                ArgError::option(format!(
                    "takes for the option `{name}` a Boolean, a number, a string, or an array of \
                     numbers or of strings, not {}",
                    value.class().describe()
                ))
            })?;
            if options.insert(name.clone(), value).is_some() {
                return Err(ArgError::option(format!(
                    "takes each option once, and the struct names `{name}` twice"
                )));
            }
        }
        Ok(options)
    }

    /// The items of an array argument. A null item is an error.
    pub(crate) fn items(&self) -> Result<Vec<&Arg>, ArgError> {
        let Arg::List(items) = self else {
            return Err(self.rejected(&Shape::Array(&Shape::Value)));
        };
        (items.iter())
            .map(|item| item.as_ref().ok_or_else(ArgError::null_item))
            .collect()
    }

    /// The argument as an array of one element, or `None` for a fragment.
    fn array(&self) -> Option<ArrayRef> {
        Some(match self {
            Arg::Bool(value) => Arc::new(BooleanArray::from(vec![*value])),
            Arg::Int(value) => Arc::new(Int64Array::from(vec![i64::try_from(*value).ok()?])),
            Arg::Float(value) => Arc::new(Float64Array::from(vec![*value])),
            Arg::Str(value) => Arc::new(StringArray::from(vec![value.as_str()])),
            Arg::Bytes(value) => Arc::new(BinaryArray::from(vec![value.as_ref()])),
            Arg::List(items) => crate::list_value(list_items(items)?),
            Arg::Struct(fields) => {
                let mut schema = Vec::with_capacity(fields.len());
                let mut columns = Vec::with_capacity(fields.len());
                for (name, value) in fields {
                    let column = match value {
                        Some(value) => value.array()?,
                        None => new_null_array(&DataType::Null, 1),
                    };
                    schema.push(Field::new(name, column.data_type().clone(), true));
                    columns.push(column);
                }
                Arc::new(StructArray::try_new(schema.into(), columns, None).ok()?)
            }
            Arg::Value(array) => array.clone(),
            Arg::Fragment(_) => return None,
        })
    }
}

/// The value of a named option of `arg`, or `None` for an argument that no option value holds.
fn option_value(arg: &Arg) -> Option<proto::OptionValue> {
    use proto::option_value::Value;
    let value = match arg {
        Arg::Bool(value) => Value::Boolean(*value),
        Arg::Int(value) => Value::Integer(i64::try_from(*value).ok()?),
        Arg::Float(value) => Value::Float(*value),
        Arg::Str(value) => Value::String(value.clone()),
        Arg::List(items) => {
            let items: Vec<&Arg> = items.iter().map(Option::as_ref).collect::<Option<_>>()?;
            match items.first() {
                Some(Arg::Str(_)) => Value::Strings(proto::StringList {
                    values: (items.iter())
                        .map(|item| item.string().ok())
                        .collect::<Option<_>>()?,
                }),
                _ => Value::Floats(proto::FloatList {
                    values: (items.iter())
                        .map(|item| item.float64().ok())
                        .collect::<Option<_>>()?,
                }),
            }
        }
        Arg::Bytes(_) | Arg::Struct(_) | Arg::Fragment(_) | Arg::Value(_) => return None,
    };
    Some(proto::OptionValue { value: Some(value) })
}

/// The items of a list, as one array: in the type of the items, `Float64` where integers and
/// floating-point numbers mix, with a null for each `None`.
fn list_items(items: &[Option<Arg>]) -> Option<ArrayRef> {
    let arrays: Vec<Option<ArrayRef>> = (items.iter())
        .map(|item| match item {
            Some(arg) => arg.array().map(Some),
            None => Some(None),
        })
        .collect::<Option<_>>()?;
    let floats = items.iter().any(|item| matches!(item, Some(Arg::Float(_))));
    let target = match arrays.iter().flatten().next() {
        Some(_) if floats => DataType::Float64,
        Some(first) => first.data_type().clone(),
        None => DataType::Null,
    };
    let arrays: Vec<ArrayRef> = (arrays.into_iter())
        .map(|array| match array {
            Some(array) if *array.data_type() == target => Some(array),
            Some(array) => cast(&array, &target).ok(),
            None => Some(new_null_array(&target, 1)),
        })
        .collect::<Option<_>>()?;
    let refs: Vec<&dyn Array> = arrays.iter().map(|array| array.as_ref()).collect();
    match refs.is_empty() {
        true => Some(new_null_array(&DataType::Null, 0)),
        false => concat(&refs).ok(),
    }
}

/// The argument of the items of a list.
fn items(values: &ArrayRef) -> Arg {
    Arg::List(
        (0..values.len())
            .map(|index| Arg::from_array(values.as_ref(), index))
            .collect(),
    )
}

/// The argument of a binary value: a fragment of this version of the format, or bytes.
fn binary(bytes: &[u8]) -> Arg {
    match proto::Fragment::decode(bytes) {
        Ok(fragment)
            if fragment.version == FORMAT_VERSION
                && fragment.r#type.starts_with("avenger.chart.v1.") =>
        {
            Arg::Fragment(fragment)
        }
        _ => Arg::Bytes(Bytes::copy_from_slice(bytes)),
    }
}

/// A fragment of the message `name` in the words of an error: the call of its constructor.
pub(crate) fn describe_fragment(name: &str) -> String {
    match super::generated::constructor_name(name) {
        Some(constructor) => format!("`{constructor}(...)`"),
        None => format!("a `{}`", name.rsplit('.').next().unwrap_or(name)),
    }
}
