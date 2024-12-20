use crate::{
    discrete_to_discrete::DiscreteToDiscreteScale, discrete_to_numeric::DiscreteToNumericScale,
    numeric::NumericScale, numeric_to_discrete::NumericToDiscreteScale,
};

#[derive(Debug)]
pub enum ScaleImpl {
    // numeric -> numeric
    Numeric(Box<dyn NumericScale>),
    // discrete -> numeric
    DiscreteToNumeric(Box<dyn DiscreteToNumericScale>),
    // discrete -> discrete
    DiscreteToDiscrete(Box<dyn DiscreteToDiscreteScale>),
    // numeric -> discrete
    NumericToDiscrete(Box<dyn NumericToDiscreteScale>),
}
