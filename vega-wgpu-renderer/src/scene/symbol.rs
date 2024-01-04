use crate::error::VegaWgpuError;
use crate::scene::value::EncodingValue;
use crate::specs::mark::MarkContainerSpec;
use crate::specs::symbol::{SymbolItemSpec, SymbolShape};
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all="kebab-case")]
pub struct SymbolMark {
    pub name: String,
    pub clip: bool,
    pub shape: SymbolShape,
    pub len: u32,
    pub x: EncodingValue<f32>,
    pub y: EncodingValue<f32>,
    pub fill: EncodingValue<[f32; 3]>,
    pub size: EncodingValue<f32>,
}

impl SymbolMark {
    pub fn x_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.x.as_iter(self.len as usize)
    }

    pub fn y_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.y.as_iter(self.len as usize)
    }

    pub fn fill_iter(&self) -> Box<dyn Iterator<Item=&[f32; 3]> + '_> {
        self.fill.as_iter(self.len as usize)
    }

    pub fn size_iter(&self) -> Box<dyn Iterator<Item=&f32> + '_> {
        self.size.as_iter(self.len as usize)
    }
}
impl Default for SymbolMark {
    fn default() -> Self {
        Self {
            name: "".to_string(),
            clip: true,
            shape: Default::default(),
            len: 1,
            x: EncodingValue::Scalar { value: 0.0 },
            y: EncodingValue::Scalar { value: 0.0 },
            fill: EncodingValue::Scalar { value: [0.0, 0.0, 0.0] },
            size: EncodingValue::Scalar { value: 20.0 },
        }
    }
}

impl SymbolMark {
    pub fn from_spec(
        spec: &MarkContainerSpec<SymbolItemSpec>,
        origin: [f32; 2],
    ) -> Result<Self, VegaWgpuError> {
        // Get shape of first item and use that for all items for now
        let first_shape = spec
            .items
            .get(0)
            .and_then(|item| item.shape)
            .unwrap_or_default();

        // Init mark with scalar defaults
        let mut mark = SymbolMark {
            shape: first_shape,
            clip: spec.clip,
            ..Default::default()
        };

        if let Some(name) = &spec.name {
            mark.name = name.clone();
        }

        // Init vector for each encoding channel
        let mut x = Vec::<f32>::new();
        let mut y = Vec::<f32>::new();
        let mut fill = Vec::<[f32; 3]>::new();
        let mut size = Vec::<f32>::new();

        // For each item, append explicit values to corresponding vector
        for item in &spec.items {
            x.push(item.x + origin[0]);
            y.push(item.y + origin[1]);

            if let Some(c) = &item.fill {
                let c = csscolorparser::parse(c)?;
                fill.push([c.r as f32, c.g as f32, c.b as f32])
            }

            if let Some(s) = item.size {
                size.push(s);
            }
        }

        // Override values with vectors
        let len = spec.items.len();
        mark.len = len as u32;

        if x.len() == len {
            mark.x = EncodingValue::Array {values: x};
        }
        if y.len() == len {
            mark.y = EncodingValue::Array {values: y};
        }
        if fill.len() == len {
            mark.fill = EncodingValue::Array {values: fill};
        }
        if size.len() == len {
            mark.size = EncodingValue::Array {values: size};
        }

        Ok(mark)
    }
}