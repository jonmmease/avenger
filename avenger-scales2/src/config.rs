use ordered_float::OrderedFloat;
use palette::Srgba;
use std::collections::HashMap;

#[cfg(feature = "temporal")]
use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

use crate::error::AvengerScaleError;

/// General scale configuration to use across all scales
#[derive(Debug, Clone)]
pub struct ScaleConfig {
    pub domain: ScaleDomainConfig,
    pub range: ScaleConfigRange,
    pub round: Option<bool>,
    pub clamp: Option<bool>,
    pub range_offset: Option<f32>,
    pub nice: Option<usize>,

    /// Additional scale specific options
    pub options: HashMap<String, ScaleConfigScalar>,
}

impl ScaleConfig {
    pub fn new() -> Self {
        Self {
            domain: ScaleDomainConfig::new_interval(0.0, 1.0),
            range: ScaleConfigRange::Interval(0.0, 1.0),
            round: None,
            clamp: None,
            range_offset: None,
            nice: None,
            options: HashMap::new(),
        }
    }

    pub fn with_numeric_range(self, start: f32, end: f32) -> Self {
        Self {
            range: ScaleConfigRange::Interval(start, end),
            ..self
        }
    }

    pub fn get_numeric_interval_domain(&self) -> Result<(f32, f32), AvengerScaleError> {
        if let ScaleDomainConfig::Interval(start, end) = self.domain {
            Ok((start, end))
        } else {
            Err(AvengerScaleError::ScaleOperationNotSupported(
                "get_numeric_interval_domain".to_string(),
            ))
        }
    }

    pub fn get_numeric_interval_range(&self) -> Result<(f32, f32), AvengerScaleError> {
        if let ScaleConfigRange::Interval(start, end) = self.range {
            Ok((start, end))
        } else {
            Err(AvengerScaleError::ScaleOperationNotSupported(
                "get_numeric_interval_range".to_string(),
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub enum ScaleDomainConfig {
    // Intervals
    Interval(f32, f32),
    // Discrete values
    Discrete(DiscreteDomainConfig),
}

impl ScaleDomainConfig {
    pub fn new_interval(start: f32, end: f32) -> Self {
        Self::Interval(start, end)
    }
}

#[derive(Debug, Clone)]
pub enum ScaleConfigRange {
    Interval(f32, f32),
    Discrete(DiscreteRangeConfig),
}

impl ScaleConfigRange {
    /// Whether the scale is considered discrete
    pub fn is_discrete(&self) -> bool {
        matches!(self, ScaleConfigRange::Discrete(_))
    }

    pub fn colors(&self) -> Result<Vec<Srgba>, AvengerScaleError> {
        if let ScaleConfigRange::Discrete(DiscreteRangeConfig::Colors(colors)) = self {
            Ok(colors.clone())
        } else {
            Err(AvengerScaleError::ScaleOperationNotSupported(
                "colors".to_string(),
            ))
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscreteRangeConfig {
    Numbers(Vec<f32>),
    Colors(Vec<Srgba>),
    Strings(Vec<String>),
    Indices(Vec<usize>),
}
impl DiscreteRangeConfig {
    pub fn len(&self) -> usize {
        match self {
            DiscreteRangeConfig::Numbers(vec) => vec.len(),
            DiscreteRangeConfig::Colors(vec) => vec.len(),
            DiscreteRangeConfig::Strings(vec) => vec.len(),
            DiscreteRangeConfig::Indices(vec) => vec.len(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum DiscreteDomainConfig {
    Indices(Vec<usize>),
    Numbers(Vec<f32>),
    Strings(Vec<String>),
    #[cfg(feature = "temporal")]
    Dates(Vec<NaiveDate>),
    #[cfg(feature = "temporal")]
    Timestamps(Vec<NaiveDateTime>),
    #[cfg(feature = "temporal")]
    Timestamptz(Vec<DateTime<Utc>>),
}

impl DiscreteDomainConfig {
    pub fn len(&self) -> usize {
        match self {
            DiscreteDomainConfig::Indices(vec) => vec.len(),
            DiscreteDomainConfig::Numbers(vec) => vec.len(),
            DiscreteDomainConfig::Strings(vec) => vec.len(),
            #[cfg(feature = "temporal")]
            DiscreteDomainConfig::Dates(vec) => vec.len(),
            #[cfg(feature = "temporal")]
            DiscreteDomainConfig::Timestamps(vec) => vec.len(),
            #[cfg(feature = "temporal")]
            DiscreteDomainConfig::Timestamptz(vec) => vec.len(),
        }
    }
}

impl From<Vec<String>> for DiscreteDomainConfig {
    fn from(value: Vec<String>) -> Self {
        Self::Strings(value)
    }
}
impl From<Vec<&str>> for DiscreteDomainConfig {
    fn from(value: Vec<&str>) -> Self {
        Self::Strings(value.iter().map(|s| s.to_string()).collect())
    }
}

impl From<Vec<f32>> for DiscreteDomainConfig {
    fn from(value: Vec<f32>) -> Self {
        Self::Numbers(value)
    }
}

impl From<Vec<usize>> for DiscreteDomainConfig {
    fn from(value: Vec<usize>) -> Self {
        Self::Indices(value)
    }
}

#[cfg(feature = "temporal")]
impl From<Vec<NaiveDate>> for DiscreteDomainConfig {
    fn from(value: Vec<NaiveDate>) -> Self {
        Self::Dates(value)
    }
}

#[cfg(feature = "temporal")]
impl From<Vec<NaiveDateTime>> for DiscreteDomainConfig {
    fn from(value: Vec<NaiveDateTime>) -> Self {
        Self::Timestamps(value)
    }
}

#[cfg(feature = "temporal")]
impl From<Vec<DateTime<Utc>>> for DiscreteDomainConfig {
    fn from(value: Vec<DateTime<Utc>>) -> Self {
        Self::Timestamptz(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScaleConfigScalar {
    Number(OrderedFloat<f32>),
    Color(Srgba),
    String(String),
    Index(usize),
    #[cfg(feature = "temporal")]
    Date(NaiveDate),
    #[cfg(feature = "temporal")]
    Timestamp(NaiveDateTime),
    #[cfg(feature = "temporal")]
    Timestamptz(DateTime<Utc>),
}

impl TryFrom<ScaleConfigScalar> for f32 {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Number(value) => Ok(value.into_inner()),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "f32".to_string(),
                scalar: value,
            }),
        }
    }
}

impl TryFrom<ScaleConfigScalar> for Srgba {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Color(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "Srgba".to_string(),
                scalar: value,
            }),
        }
    }
}

impl TryFrom<ScaleConfigScalar> for String {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::String(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "String".to_string(),
                scalar: value,
            }),
        }
    }
}

impl TryFrom<ScaleConfigScalar> for usize {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Index(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "usize".to_string(),
                scalar: value,
            }),
        }
    }
}

#[cfg(feature = "temporal")]
impl TryFrom<ScaleConfigScalar> for NaiveDate {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Date(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "NaiveDate".to_string(),
                scalar: value,
            }),
        }
    }
}

#[cfg(feature = "temporal")]
impl TryFrom<ScaleConfigScalar> for NaiveDateTime {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Timestamp(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "NaiveDateTime".to_string(),
                scalar: value,
            }),
        }
    }
}

#[cfg(feature = "temporal")]
impl TryFrom<ScaleConfigScalar> for DateTime<Utc> {
    type Error = AvengerScaleError;

    fn try_from(value: ScaleConfigScalar) -> Result<Self, Self::Error> {
        match value {
            ScaleConfigScalar::Timestamptz(value) => Ok(value),
            _ => Err(AvengerScaleError::InvalidScaleConfigScalarValue {
                expected_type: "DateTime<Utc>".to_string(),
                scalar: value,
            }),
        }
    }
}

impl From<f32> for ScaleConfigScalar {
    fn from(value: f32) -> Self {
        Self::Number(OrderedFloat::from(value))
    }
}

impl From<Srgba> for ScaleConfigScalar {
    fn from(value: Srgba) -> Self {
        Self::Color(value)
    }
}

impl From<String> for ScaleConfigScalar {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<usize> for ScaleConfigScalar {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}

#[cfg(feature = "temporal")]
impl From<NaiveDate> for ScaleConfigScalar {
    fn from(value: NaiveDate) -> Self {
        Self::Date(value)
    }
}

#[cfg(feature = "temporal")]
impl From<NaiveDateTime> for ScaleConfigScalar {
    fn from(value: NaiveDateTime) -> Self {
        Self::Timestamp(value)
    }
}

#[cfg(feature = "temporal")]
impl From<DateTime<Utc>> for ScaleConfigScalar {
    fn from(value: DateTime<Utc>) -> Self {
        Self::Timestamptz(value)
    }
}

pub trait ScaleConfigScalarMapUtils {
    fn try_get_f32(&self, key: &str) -> Option<f32>;
    fn try_get_srgba(&self, key: &str) -> Option<Srgba>;
    fn try_get_string(&self, key: &str) -> Option<String>;
    fn try_get_usize(&self, key: &str) -> Option<usize>;
    #[cfg(feature = "temporal")]
    fn try_get_date(&self, key: &str) -> Option<NaiveDate>;
    #[cfg(feature = "temporal")]
    fn try_get_timestamp(&self, key: &str) -> Option<NaiveDateTime>;
    #[cfg(feature = "temporal")]
    fn try_get_timestamptz(&self, key: &str) -> Option<DateTime<Utc>>;
}

impl ScaleConfigScalarMapUtils for HashMap<String, ScaleConfigScalar> {
    fn try_get_f32(&self, key: &str) -> Option<f32> {
        let value = self.get(key)?.clone();
        f32::try_from(value).ok()
    }

    fn try_get_srgba(&self, key: &str) -> Option<Srgba> {
        let value = self.get(key)?.clone();
        Srgba::try_from(value).ok()
    }

    fn try_get_string(&self, key: &str) -> Option<String> {
        let value = self.get(key)?.clone();
        String::try_from(value).ok()
    }

    fn try_get_usize(&self, key: &str) -> Option<usize> {
        let value = self.get(key)?.clone();
        usize::try_from(value).ok()
    }

    #[cfg(feature = "temporal")]
    fn try_get_date(&self, key: &str) -> Option<NaiveDate> {
        let value = self.get(key)?.clone();
        NaiveDate::try_from(value).ok()
    }

    #[cfg(feature = "temporal")]
    fn try_get_timestamp(&self, key: &str) -> Option<NaiveDateTime> {
        let value = self.get(key)?.clone();
        NaiveDateTime::try_from(value).ok()
    }

    #[cfg(feature = "temporal")]
    fn try_get_timestamptz(&self, key: &str) -> Option<DateTime<Utc>> {
        let value = self.get(key)?.clone();
        DateTime::try_from(value).ok()
    }
}
