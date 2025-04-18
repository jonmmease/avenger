use ordered_float::OrderedFloat;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use strum::VariantNames;

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
#[strum(serialize_all = "snake_case")]
pub enum TextBaseline {
    Alphabetic,
    Top,
    Middle,
    #[default]
    Bottom,
    LineTop,
    LineBottom,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, VariantNames)]
#[cfg_attr(feature = "serde", serde(untagged))]
#[strum(serialize_all = "snake_case")]
pub enum FontWeight {
    Name(FontWeightNameSpec),
    Number(f32),
}

impl std::hash::Hash for FontWeight {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::Name(spec) => spec.hash(state),
            Self::Number(num) => OrderedFloat::from(*num).hash(state),
        }
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::Name(FontWeightNameSpec::Normal)
    }
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum FontWeightNameSpec {
    #[default]
    Normal,
    Bold,
}

#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Hash, VariantNames)]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[strum(serialize_all = "snake_case")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
}
