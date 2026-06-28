use crate::style::{MathFontConfig, MathStrictness};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstCacheConfig {
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TypstEngineConfig {
    pub font_config: MathFontConfig,
    pub cache: TypstCacheConfig,
    pub strictness: MathStrictness,
}

impl Default for TypstEngineConfig {
    fn default() -> Self {
        Self {
            font_config: MathFontConfig::default(),
            cache: TypstCacheConfig::default(),
            strictness: MathStrictness::default(),
        }
    }
}
