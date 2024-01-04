use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all="kebab-case")]
pub enum EncodingValue<T>  {
    Scalar {value: T},
    Array {values: Vec<T>}
}

impl <T> EncodingValue<T> {
    pub fn as_iter(&self, scalar_len: usize) -> Box<dyn Iterator<Item=&T> + '_> {
        match self {
            EncodingValue::Scalar { value } => Box::new(std::iter::repeat(value).take(scalar_len)),
            EncodingValue::Array { values } => Box::new(values.iter())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum StrokeCap {
    Butt,
    Round,
    Square,
}
