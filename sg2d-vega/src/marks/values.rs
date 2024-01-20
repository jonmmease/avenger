use crate::error::VegaSceneGraphError;
use serde::{Deserialize, Serialize};
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
