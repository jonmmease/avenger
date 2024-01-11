use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum EncodingValue<T> {
    Scalar { value: T },
    Array { values: Vec<T> },
}

impl<T> EncodingValue<T> {
    pub fn as_iter<'a>(
        &'a self,
        scalar_len: usize,
        indices: Option<&'a Vec<usize>>,
    ) -> Box<dyn Iterator<Item = &T> + '_> {
        match self {
            EncodingValue::Scalar { value } => Box::new(std::iter::repeat(value).take(scalar_len)),
            EncodingValue::Array { values } => match indices {
                None => Box::new(values.iter()),
                Some(indices) => Box::new(indices.iter().map(|i| &values[*i])),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}
