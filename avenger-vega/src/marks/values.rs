use crate::error::AvengerVegaError;
use avenger_common::value::{
    ColorOrGradient, Gradient, GradientStop, LinearGradient, RadialGradient,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrokeDashSpec {
    String(String),
    Array(Vec<f32>),
}

impl StrokeDashSpec {
    pub fn to_array(&self) -> Result<Cow<Vec<f32>>, AvengerVegaError> {
        match self {
            StrokeDashSpec::Array(a) => Ok(Cow::Borrowed(a)),
            StrokeDashSpec::String(s) => {
                let clean_dash_str = s.replace(',', " ");
                let mut dashes: Vec<f32> = Vec::new();
                for s in clean_dash_str.split_whitespace() {
                    let d = s
                        .parse::<f32>()
                        .map_err(|_| AvengerVegaError::InvalidDashString(s.to_string()))?
                        .abs();
                    dashes.push(d);
                }
                Ok(Cow::Owned(dashes))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CssColorOrGradient {
    Color(String),
    Gradient(CssGradient),
}

impl CssColorOrGradient {
    pub fn to_color_or_grad(
        &self,
        opacity: f32,
        gradients: &mut Vec<Gradient>,
    ) -> Result<ColorOrGradient, AvengerVegaError> {
        match self {
            CssColorOrGradient::Color(c) => {
                let c = csscolorparser::parse(c)?;
                Ok(ColorOrGradient::Color([
                    c.r as f32,
                    c.g as f32,
                    c.b as f32,
                    c.a as f32 * opacity,
                ]))
            }
            CssColorOrGradient::Gradient(grad) => {
                // Build gradient
                let grad = match grad.gradient {
                    VegaGradientType::Linear => Gradient::LinearGradient(LinearGradient {
                        x0: grad.x1.unwrap_or(0.0),
                        y0: grad.y1.unwrap_or(0.0),
                        x1: grad.x2.unwrap_or(1.0),
                        y1: grad.y2.unwrap_or(0.0),
                        stops: grad
                            .stops
                            .iter()
                            .map(|s| s.to_grad_stop(opacity))
                            .collect::<Result<Vec<_>, AvengerVegaError>>()?,
                    }),
                    VegaGradientType::Radial => Gradient::RadialGradient(RadialGradient {
                        x0: grad.x1.unwrap_or(0.5),
                        y0: grad.y1.unwrap_or(0.5),
                        x1: grad.x2.unwrap_or(0.5),
                        y1: grad.y2.unwrap_or(0.5),
                        r0: grad.r1.unwrap_or(0.0),
                        r1: grad.r2.unwrap_or(0.5),
                        stops: grad
                            .stops
                            .iter()
                            .map(|s| s.to_grad_stop(opacity))
                            .collect::<Result<Vec<_>, AvengerVegaError>>()?,
                    }),
                };

                // Check if we already have it
                let pos = match gradients.iter().position(|g| g == &grad) {
                    Some(pos) => pos,
                    None => {
                        let pos = gradients.len();
                        gradients.push(grad);
                        pos
                    }
                };
                Ok(ColorOrGradient::GradientIndex(pos as u32))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CssGradient {
    #[serde(default)]
    gradient: VegaGradientType,
    x1: Option<f32>,
    y1: Option<f32>,
    x2: Option<f32>,
    y2: Option<f32>,
    r1: Option<f32>,
    r2: Option<f32>,
    stops: Vec<CssGradientStop>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VegaGradientType {
    #[default]
    Linear,
    Radial,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CssGradientStop {
    offset: f32,
    color: String,
}

impl CssGradientStop {
    pub fn to_grad_stop(&self, opacity: f32) -> Result<GradientStop, AvengerVegaError> {
        let c = csscolorparser::parse(&self.color)?;
        Ok(GradientStop {
            offset: self.offset,
            color: [c.r as f32, c.g as f32, c.b as f32, c.a as f32 * opacity],
        })
    }
}

/// Helper struct that will not drop null values on round trip (de)serialization
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum MissingNullOrValue<V> {
    #[default]
    Missing,
    Null,
    Value(V),
}

impl<V> MissingNullOrValue<V> {
    pub fn is_missing(&self) -> bool {
        matches!(self, MissingNullOrValue::Missing)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, MissingNullOrValue::Null)
    }

    pub fn as_option(&self) -> Option<&V> {
        match self {
            MissingNullOrValue::Missing | MissingNullOrValue::Null => None,
            MissingNullOrValue::Value(v) => Some(v),
        }
    }
}

impl<V> From<Option<V>> for MissingNullOrValue<V> {
    fn from(opt: Option<V>) -> MissingNullOrValue<V> {
        match opt {
            Some(v) => MissingNullOrValue::Value(v),
            None => MissingNullOrValue::Null,
        }
    }
}

impl<'de, V: Deserialize<'de>> Deserialize<'de> for MissingNullOrValue<V> {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::deserialize(deserializer).map(Into::into)
    }
}

impl<V: Serialize> Serialize for MissingNullOrValue<V> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            MissingNullOrValue::Missing => None::<Option<String>>.serialize(serializer),
            MissingNullOrValue::Null => serde_json::Value::Null.serialize(serializer),
            MissingNullOrValue::Value(v) => v.serialize(serializer),
        }
    }
}
