use crate::error::VegaSceneGraphError;
use serde::{Deserialize, Serialize};
use sg2d::marks::value::{ColorOrGradient, GradientStop, LinearGradient, RadialGradient};
use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StrokeDashSpec {
    String(String),
    Array(Vec<f32>),
}

impl StrokeDashSpec {
    pub fn to_array(&self) -> Result<Cow<Vec<f32>>, VegaSceneGraphError> {
        match self {
            StrokeDashSpec::Array(a) => Ok(Cow::Borrowed(a)),
            StrokeDashSpec::String(s) => {
                let clean_dash_str = s.replace(',', " ");
                let mut dashes: Vec<f32> = Vec::new();
                for s in clean_dash_str.split_whitespace() {
                    let d = s
                        .parse::<f32>()
                        .map_err(|_| VegaSceneGraphError::InvalidDashString(s.to_string()))?
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
    pub fn to_color_or_grad(&self, opacity: f32) -> Result<ColorOrGradient, VegaSceneGraphError> {
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
            CssColorOrGradient::Gradient(grad) => Ok(match grad.gradient {
                VegaGradientType::Linear => ColorOrGradient::LinearGradient(LinearGradient {
                    x0: grad.x1.unwrap_or(0.0),
                    y0: grad.y1.unwrap_or(0.0),
                    x1: grad.x2.unwrap_or(1.0),
                    y1: grad.y2.unwrap_or(0.0),
                    stops: grad
                        .stops
                        .iter()
                        .map(|s| s.to_grad_stop(opacity))
                        .collect::<Result<Vec<_>, VegaSceneGraphError>>()?,
                }),
                VegaGradientType::Radial => ColorOrGradient::RadialGradient(RadialGradient {
                    x0: grad.x1.unwrap_or(0.5),
                    y0: grad.y1.unwrap_or(0.5),
                    x1: grad.x2.unwrap_or(0.5),
                    y1: grad.y2.unwrap_or(0.5),
                    r0: grad.r1.unwrap_or(0.0),
                    r1: grad.r2.unwrap_or(0.5),
                    stops: vec![],
                }),
            }),
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
    pub fn to_grad_stop(&self, opacity: f32) -> Result<GradientStop, VegaSceneGraphError> {
        let c = csscolorparser::parse(&self.color)?;
        Ok(GradientStop {
            offset: self.offset,
            color: [c.r as f32, c.g as f32, c.b as f32, c.a as f32 * opacity],
        })
    }
}