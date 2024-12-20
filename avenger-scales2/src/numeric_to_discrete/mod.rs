mod threshold;

use crate::{
    config::{DiscreteRangeConfig, ScaleConfigScalar},
    error::AvengerScaleError,
};
use std::collections::HashMap;
use std::fmt::Debug;

/// Config for numeric scales
#[derive(Debug, Clone)]
pub struct NumericToDiscreteScaleConfig {
    pub domain: Vec<f32>,
    pub range: DiscreteRangeConfig,

    /// Additional scale specific options
    pub options: HashMap<String, ScaleConfigScalar>,
}

pub trait NumericToDiscreteScale: Debug + Send + Sync + 'static {
    /// Scale numeric values to indices into the domain vector
    fn scale(
        &self,
        config: &NumericToDiscreteScaleConfig,
        values: &[f32],
    ) -> Result<Vec<Option<usize>>, AvengerScaleError>;
}
