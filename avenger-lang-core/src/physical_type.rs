//! Canonical physical Arrow type algebra used at language boundaries.
//!
//! This mirrors Arrow's physical types without depending on the Arrow crate;
//! the compiler layer performs the mechanical conversion to `DataType`.

use std::{collections::BTreeSet, fmt};

use serde::{Deserialize, Serialize};

use crate::ast::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeUnit {
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl fmt::Display for TimeUnit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Second => "second",
            Self::Millisecond => "millisecond",
            Self::Microsecond => "microsecond",
            Self::Nanosecond => "nanosecond",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalUnit {
    YearMonth,
    DayTime,
    MonthDayNano,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PhysicalField {
    pub name: String,
    pub data_type: PhysicalType,
    /// Nested list/map elements and struct fields are nullable in language v1.
    pub nullable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum PhysicalType {
    Boolean,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float16,
    Float32,
    Float64,
    Utf8,
    LargeUtf8,
    Binary,
    LargeBinary,
    Date32,
    Date64,
    Time32(TimeUnit),
    Time64(TimeUnit),
    Timestamp {
        unit: TimeUnit,
        timezone: Option<String>,
    },
    Duration(TimeUnit),
    Interval(IntervalUnit),
    FixedSizeBinary(i32),
    Decimal128 {
        precision: u8,
        scale: i8,
    },
    Decimal256 {
        precision: u8,
        scale: i8,
    },
    List(Box<PhysicalType>),
    LargeList(Box<PhysicalType>),
    FixedSizeList {
        element: Box<PhysicalType>,
        length: i32,
    },
    Struct(Vec<PhysicalField>),
    Map {
        key: Box<PhysicalType>,
        value: Box<PhysicalType>,
    },
}

impl PhysicalType {
    /// Canonical physical Arrow type constructors accepted by the language.
    /// Compiler analysis and editor completion share this inventory.
    pub const CONSTRUCTORS: &'static [&'static str] = &[
        "boolean",
        "int8",
        "int16",
        "int32",
        "int64",
        "uint8",
        "uint16",
        "uint32",
        "uint64",
        "float16",
        "float32",
        "float64",
        "utf8",
        "large_utf8",
        "binary",
        "large_binary",
        "date32",
        "date64",
        "time32",
        "time64",
        "timestamp",
        "duration",
        "interval",
        "fixed_size_binary",
        "decimal128",
        "decimal256",
        "list",
        "large_list",
        "fixed_size_list",
        "struct",
        "map",
    ];

    pub fn parse(value: &Value) -> Result<Self, PhysicalTypeError> {
        parse_type(value, "type")
    }

    pub(crate) fn accepts_inference_literal(
        &self,
        value: &Value,
    ) -> Result<(), PhysicalValueError> {
        match value {
            Value::Null => Ok(()),
            Value::Bool(_) if matches!(self, Self::Boolean) => Ok(()),
            Value::Str(_) if matches!(self, Self::Utf8 | Self::LargeUtf8) => Ok(()),
            Value::Num(number) => validate_number(self, number.as_str()),
            Value::Array(values) => match self {
                Self::List(element) | Self::LargeList(element) => {
                    for value in values {
                        element.accepts_inference_literal(value)?;
                    }
                    Ok(())
                }
                Self::FixedSizeList { element, length }
                    if usize::try_from(*length).ok() == Some(values.len()) =>
                {
                    for value in values {
                        element.accepts_inference_literal(value)?;
                    }
                    Ok(())
                }
                _ => Err(PhysicalValueError::Shape {
                    expected: self.to_string(),
                    found: "array".to_owned(),
                }),
            },
            Value::Block { head: None, body } => match self {
                Self::Struct(fields) => {
                    if !body.children.is_empty() {
                        return Err(PhysicalValueError::Shape {
                            expected: self.to_string(),
                            found: "object with child declarations".to_owned(),
                        });
                    }
                    for (name, _) in body.props.iter() {
                        if !fields.iter().any(|field| field.name == name.as_str()) {
                            return Err(PhysicalValueError::UnknownField(name.to_string()));
                        }
                    }
                    for field in fields {
                        match body.props.get(&field.name) {
                            Some(value) => field.data_type.accepts_inference_literal(value)?,
                            None if field.nullable => {}
                            None => {
                                return Err(PhysicalValueError::MissingField(field.name.clone()));
                            }
                        }
                    }
                    Ok(())
                }
                Self::Map { key, value } => {
                    for (name, item) in body.props.iter() {
                        key.accepts_inference_literal(&Value::Str(name.to_string()))?;
                        value.accepts_inference_literal(item)?;
                    }
                    Ok(())
                }
                _ => Err(PhysicalValueError::Shape {
                    expected: self.to_string(),
                    found: "object".to_owned(),
                }),
            },
            // General expressions are typed by DataFusion in Phase 6. This
            // method deliberately checks only destination-typed literals.
            Value::Expr(_) => Ok(()),
            other => Err(PhysicalValueError::Shape {
                expected: self.to_string(),
                found: value_shape(other),
            }),
        }
    }
}

impl fmt::Display for PhysicalType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean => formatter.write_str("boolean"),
            Self::Int8 => formatter.write_str("int8"),
            Self::Int16 => formatter.write_str("int16"),
            Self::Int32 => formatter.write_str("int32"),
            Self::Int64 => formatter.write_str("int64"),
            Self::UInt8 => formatter.write_str("uint8"),
            Self::UInt16 => formatter.write_str("uint16"),
            Self::UInt32 => formatter.write_str("uint32"),
            Self::UInt64 => formatter.write_str("uint64"),
            Self::Float16 => formatter.write_str("float16"),
            Self::Float32 => formatter.write_str("float32"),
            Self::Float64 => formatter.write_str("float64"),
            Self::Utf8 => formatter.write_str("utf8"),
            Self::LargeUtf8 => formatter.write_str("large_utf8"),
            Self::Binary => formatter.write_str("binary"),
            Self::LargeBinary => formatter.write_str("large_binary"),
            Self::Date32 => formatter.write_str("date32"),
            Self::Date64 => formatter.write_str("date64"),
            Self::Time32(unit) => write!(formatter, "time32({unit})"),
            Self::Time64(unit) => write!(formatter, "time64({unit})"),
            Self::Timestamp { unit, timezone } => {
                write!(formatter, "timestamp({unit}")?;
                if let Some(timezone) = timezone {
                    write!(formatter, ",'{}'", timezone.replace('\'', "''"))?;
                }
                formatter.write_str(")")
            }
            Self::Duration(unit) => write!(formatter, "duration({unit})"),
            Self::Interval(unit) => formatter.write_str(match unit {
                IntervalUnit::YearMonth => "interval(year_month)",
                IntervalUnit::DayTime => "interval(day_time)",
                IntervalUnit::MonthDayNano => "interval(month_day_nano)",
            }),
            Self::FixedSizeBinary(length) => write!(formatter, "fixed_size_binary({length})"),
            Self::Decimal128 { precision, scale } => {
                write!(formatter, "decimal128({precision},{scale})")
            }
            Self::Decimal256 { precision, scale } => {
                write!(formatter, "decimal256({precision},{scale})")
            }
            Self::List(element) => write!(formatter, "list({element})"),
            Self::LargeList(element) => write!(formatter, "large_list({element})"),
            Self::FixedSizeList { element, length } => {
                write!(formatter, "fixed_size_list({element},{length})")
            }
            Self::Struct(fields) => {
                formatter.write_str("struct(")?;
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(",")?;
                    }
                    write!(
                        formatter,
                        "field({},'{}')",
                        field.data_type,
                        field.name.replace('\'', "''")
                    )?;
                }
                formatter.write_str(")")
            }
            Self::Map { key, value } => write!(formatter, "map({key},{value})"),
        }
    }
}

fn parse_type(value: &Value, context: &str) -> Result<PhysicalType, PhysicalTypeError> {
    if let Value::Atom(atom) = value {
        return parse_atomic(atom.as_str()).ok_or_else(|| PhysicalTypeError::UnknownType {
            name: atom.to_string(),
            context: context.to_owned(),
        });
    }
    let Value::Call { function, args } = value else {
        return Err(PhysicalTypeError::ExpectedType {
            context: context.to_owned(),
            found: value_shape(value),
        });
    };
    let name = function.as_str();
    match name {
        "time32" => {
            arity(name, args, 1)?;
            let unit = parse_unit(&args[0], name)?;
            if !matches!(unit, TimeUnit::Second | TimeUnit::Millisecond) {
                return Err(PhysicalTypeError::InvalidUnit {
                    constructor: name.to_owned(),
                    unit: unit.to_string(),
                });
            }
            Ok(PhysicalType::Time32(unit))
        }
        "time64" => {
            arity(name, args, 1)?;
            let unit = parse_unit(&args[0], name)?;
            if !matches!(unit, TimeUnit::Microsecond | TimeUnit::Nanosecond) {
                return Err(PhysicalTypeError::InvalidUnit {
                    constructor: name.to_owned(),
                    unit: unit.to_string(),
                });
            }
            Ok(PhysicalType::Time64(unit))
        }
        "timestamp" => {
            if !(1..=2).contains(&args.len()) {
                return Err(PhysicalTypeError::Arity {
                    constructor: name.to_owned(),
                    expected: "one or two".to_owned(),
                    actual: args.len(),
                });
            }
            let unit = parse_unit(&args[0], name)?;
            let timezone = args
                .get(1)
                .map(|value| match value {
                    Value::Str(timezone) if !timezone.is_empty() => Ok(timezone.clone()),
                    _ => Err(PhysicalTypeError::ExpectedString {
                        context: "timestamp timezone".to_owned(),
                    }),
                })
                .transpose()?;
            Ok(PhysicalType::Timestamp { unit, timezone })
        }
        "duration" => {
            arity(name, args, 1)?;
            Ok(PhysicalType::Duration(parse_unit(&args[0], name)?))
        }
        "interval" => {
            arity(name, args, 1)?;
            let Value::Atom(unit) = &args[0] else {
                return Err(PhysicalTypeError::ExpectedAtom {
                    context: "interval unit".to_owned(),
                });
            };
            let unit = match unit.as_str() {
                "year_month" => IntervalUnit::YearMonth,
                "day_time" => IntervalUnit::DayTime,
                "month_day_nano" => IntervalUnit::MonthDayNano,
                value => {
                    return Err(PhysicalTypeError::InvalidUnit {
                        constructor: name.to_owned(),
                        unit: value.to_owned(),
                    });
                }
            };
            Ok(PhysicalType::Interval(unit))
        }
        "fixed_size_binary" => {
            arity(name, args, 1)?;
            Ok(PhysicalType::FixedSizeBinary(positive_i32(&args[0], name)?))
        }
        "decimal128" | "decimal256" => {
            arity(name, args, 2)?;
            let precision = unsigned_u8(&args[0], "decimal precision")?;
            let scale = signed_i8(&args[1], "decimal scale")?;
            let maximum = if name == "decimal128" { 38 } else { 76 };
            if precision == 0 || precision > maximum || i16::from(scale) > i16::from(precision) {
                return Err(PhysicalTypeError::InvalidDecimal {
                    constructor: name.to_owned(),
                    precision,
                    scale,
                });
            }
            if name == "decimal128" {
                Ok(PhysicalType::Decimal128 { precision, scale })
            } else {
                Ok(PhysicalType::Decimal256 { precision, scale })
            }
        }
        "list" | "large_list" => {
            arity(name, args, 1)?;
            let element = Box::new(parse_type(&args[0], "list element")?);
            if name == "list" {
                Ok(PhysicalType::List(element))
            } else {
                Ok(PhysicalType::LargeList(element))
            }
        }
        "fixed_size_list" => {
            arity(name, args, 2)?;
            Ok(PhysicalType::FixedSizeList {
                element: Box::new(parse_type(&args[0], "fixed-size list element")?),
                length: positive_i32(&args[1], "fixed-size list length")?,
            })
        }
        "struct" => {
            let mut fields = Vec::with_capacity(args.len());
            let mut names = BTreeSet::new();
            for value in args {
                let Value::Call { function, args } = value else {
                    return Err(PhysicalTypeError::ExpectedField);
                };
                if function.as_str() != "field" || args.len() != 2 {
                    return Err(PhysicalTypeError::ExpectedField);
                }
                if matches!((&args[0], &args[1]), (Value::Str(_), _)) {
                    return Err(PhysicalTypeError::LegacyFieldOrder);
                }
                let Value::Str(name) = &args[1] else {
                    return Err(PhysicalTypeError::ExpectedString {
                        context: "struct field name".to_owned(),
                    });
                };
                if name.is_empty() {
                    return Err(PhysicalTypeError::EmptyFieldName);
                }
                if !names.insert(name.clone()) {
                    return Err(PhysicalTypeError::DuplicateField(name.clone()));
                }
                fields.push(PhysicalField {
                    name: name.clone(),
                    data_type: parse_type(&args[0], "struct field")?,
                    nullable: true,
                });
            }
            Ok(PhysicalType::Struct(fields))
        }
        "map" => {
            arity(name, args, 2)?;
            Ok(PhysicalType::Map {
                key: Box::new(parse_type(&args[0], "map key")?),
                value: Box::new(parse_type(&args[1], "map value")?),
            })
        }
        _ => Err(PhysicalTypeError::UnknownType {
            name: name.to_owned(),
            context: context.to_owned(),
        }),
    }
}

fn parse_atomic(name: &str) -> Option<PhysicalType> {
    Some(match name {
        "boolean" => PhysicalType::Boolean,
        "int8" => PhysicalType::Int8,
        "int16" => PhysicalType::Int16,
        "int32" => PhysicalType::Int32,
        "int64" => PhysicalType::Int64,
        "uint8" => PhysicalType::UInt8,
        "uint16" => PhysicalType::UInt16,
        "uint32" => PhysicalType::UInt32,
        "uint64" => PhysicalType::UInt64,
        "float16" => PhysicalType::Float16,
        "float32" => PhysicalType::Float32,
        "float64" => PhysicalType::Float64,
        "utf8" => PhysicalType::Utf8,
        "large_utf8" => PhysicalType::LargeUtf8,
        "binary" => PhysicalType::Binary,
        "large_binary" => PhysicalType::LargeBinary,
        "date32" => PhysicalType::Date32,
        "date64" => PhysicalType::Date64,
        _ => return None,
    })
}

fn parse_unit(value: &Value, constructor: &str) -> Result<TimeUnit, PhysicalTypeError> {
    let Value::Atom(unit) = value else {
        return Err(PhysicalTypeError::ExpectedAtom {
            context: format!("{constructor} unit"),
        });
    };
    match unit.as_str() {
        "second" => Ok(TimeUnit::Second),
        "millisecond" => Ok(TimeUnit::Millisecond),
        "microsecond" => Ok(TimeUnit::Microsecond),
        "nanosecond" => Ok(TimeUnit::Nanosecond),
        unit => Err(PhysicalTypeError::InvalidUnit {
            constructor: constructor.to_owned(),
            unit: unit.to_owned(),
        }),
    }
}

fn arity(name: &str, args: &[Value], expected: usize) -> Result<(), PhysicalTypeError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(PhysicalTypeError::Arity {
            constructor: name.to_owned(),
            expected: expected.to_string(),
            actual: args.len(),
        })
    }
}

fn positive_i32(value: &Value, context: &str) -> Result<i32, PhysicalTypeError> {
    let parsed = signed_i128(value, context)?;
    i32::try_from(parsed)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| PhysicalTypeError::ExpectedPositiveInteger {
            context: context.to_owned(),
        })
}

fn unsigned_u8(value: &Value, context: &str) -> Result<u8, PhysicalTypeError> {
    u8::try_from(signed_i128(value, context)?).map_err(|_| PhysicalTypeError::ExpectedInteger {
        context: context.to_owned(),
    })
}

fn signed_i8(value: &Value, context: &str) -> Result<i8, PhysicalTypeError> {
    i8::try_from(signed_i128(value, context)?).map_err(|_| PhysicalTypeError::ExpectedInteger {
        context: context.to_owned(),
    })
}

fn signed_i128(value: &Value, context: &str) -> Result<i128, PhysicalTypeError> {
    let Value::Num(number) = value else {
        return Err(PhysicalTypeError::ExpectedInteger {
            context: context.to_owned(),
        });
    };
    let spelling = number.as_str();
    if spelling.contains(['.', 'e']) {
        return Err(PhysicalTypeError::ExpectedInteger {
            context: context.to_owned(),
        });
    }
    spelling
        .parse()
        .map_err(|_| PhysicalTypeError::ExpectedInteger {
            context: context.to_owned(),
        })
}

fn validate_number(data_type: &PhysicalType, spelling: &str) -> Result<(), PhysicalValueError> {
    macro_rules! parse_as {
        ($type:ty) => {
            spelling
                .parse::<$type>()
                .map(|_| ())
                .map_err(|_| PhysicalValueError::Number {
                    value: spelling.to_owned(),
                    expected: data_type.to_string(),
                })
        };
    }
    match data_type {
        PhysicalType::Int8 => parse_as!(i8),
        PhysicalType::Int16 => parse_as!(i16),
        PhysicalType::Int32 => parse_as!(i32),
        PhysicalType::Int64 => parse_as!(i64),
        PhysicalType::UInt8 => parse_as!(u8),
        PhysicalType::UInt16 => parse_as!(u16),
        PhysicalType::UInt32 => parse_as!(u32),
        PhysicalType::UInt64 => parse_as!(u64),
        PhysicalType::Float16 | PhysicalType::Float32 => parse_as!(f32),
        PhysicalType::Float64 => parse_as!(f64),
        PhysicalType::Decimal128 { precision, scale }
        | PhysicalType::Decimal256 { precision, scale } => {
            validate_decimal_literal(spelling, *precision, *scale)
        }
        _ => Err(PhysicalValueError::Shape {
            expected: data_type.to_string(),
            found: "number".to_owned(),
        }),
    }
}

fn validate_decimal_literal(
    spelling: &str,
    precision: u8,
    scale: i8,
) -> Result<(), PhysicalValueError> {
    if spelling.contains('e') {
        return Err(PhysicalValueError::Number {
            value: spelling.to_owned(),
            expected: format!("decimal({precision},{scale})"),
        });
    }
    let unsigned = spelling.strip_prefix('-').unwrap_or(spelling);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    let integer_digits = integer.trim_start_matches('0').len();
    let fractional_digits = fraction.len();
    let allowed_fraction = usize::try_from(scale.max(0)).expect("nonnegative scale");
    let total_digits = integer_digits + fractional_digits;
    if fractional_digits > allowed_fraction || total_digits > usize::from(precision) {
        return Err(PhysicalValueError::Number {
            value: spelling.to_owned(),
            expected: format!("decimal({precision},{scale})"),
        });
    }
    Ok(())
}

fn value_shape(value: &Value) -> String {
    match value {
        Value::Str(_) => "string",
        Value::Num(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Null => "null",
        Value::Column(_) => "column",
        Value::Atom(_) => "atom",
        Value::Expr(_) => "SQL expression",
        Value::Projection(_) => "SQL projection list",
        Value::Query(_) => "SQL query",
        Value::Relation(_) => "relation path",
        Value::Binding { .. } => "binding",
        Value::Ref { .. } => "reference",
        Value::Channel { mode, .. } => match mode {
            crate::ast::ChannelMode::Encoded => "encoded channel value",
            crate::ast::ChannelMode::Direct => "direct channel value",
        },
        Value::Dim(_) => "dimension",
        Value::Pattern(_) => "pattern",
        Value::Env(_) => "environment value",
        Value::None => "none",
        Value::Array(_) => "array",
        Value::Block { .. } => "block",
        Value::Call { .. } => "call",
    }
    .to_owned()
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PhysicalTypeError {
    #[error("expected a physical Arrow type in {context}, found {found}")]
    ExpectedType { context: String, found: String },
    #[error("unknown physical Arrow type or constructor `{name}` in {context}")]
    UnknownType { name: String, context: String },
    #[error("`{constructor}` expects {expected} arguments, found {actual}")]
    Arity {
        constructor: String,
        expected: String,
        actual: usize,
    },
    #[error("expected an atom for {context}")]
    ExpectedAtom { context: String },
    #[error("expected a string for {context}")]
    ExpectedString { context: String },
    #[error("expected an integer for {context}")]
    ExpectedInteger { context: String },
    #[error("expected a positive integer for {context}")]
    ExpectedPositiveInteger { context: String },
    #[error("invalid `{constructor}` unit `{unit}`")]
    InvalidUnit { constructor: String, unit: String },
    #[error("invalid {constructor} precision {precision} and scale {scale}")]
    InvalidDecimal {
        constructor: String,
        precision: u8,
        scale: i8,
    },
    #[error("struct arguments must be `field(<type>, '<name>')`")]
    ExpectedField,
    #[error("struct fields use `field(<type>, '<name>')`; the name-first order was removed")]
    LegacyFieldOrder,
    #[error("struct field names cannot be empty")]
    EmptyFieldName,
    #[error("duplicate struct field `{0}`")]
    DuplicateField(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PhysicalValueError {
    #[error("expected {expected}, found {found}")]
    Shape { expected: String, found: String },
    #[error("numeric literal `{value}` is not representable as {expected}")]
    Number { value: String, expected: String },
    #[error("unknown struct field `{0}`")]
    UnknownField(String),
    #[error("missing struct field `{0}`")]
    MissingField(String),
}
