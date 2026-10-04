use crate::skeleton::unsupported;
use avenger_format::NumberFormatError;
use icu_experimental::{
    measure::measureunit::MeasureUnit,
    units::{
        converter::UnitsConverter, converter_factory::ConverterFactory, convertible::Convertible,
    },
};
use num_rational::BigRational;
use num_traits::{One, Zero};

/// An owned input lets a prepared ICU converter retain factors without borrowing quantities.
#[derive(Clone, Debug)]
struct Input(BigRational);
impl Convertible for Input {
    type Factor = BigRational;
    type Addend = BigRational;
    type Result = Option<BigRational>;
    fn mul(self, factor: &BigRational) -> Self::Result {
        Some(self.0 * factor)
    }
    fn mul_add(self, factor: &BigRational, addend: &BigRational) -> Self::Result {
        Some(self.0 * factor + addend)
    }
    fn reciprocal_mul(self, factor: &BigRational) -> Self::Result {
        let product = self.0 * factor;
        (!product.is_zero()).then(|| product.recip())
    }
    fn factor_from_ratio_bigint(value: BigRational) -> BigRational {
        value
    }
    fn addend_from_ratio_bigint(value: BigRational) -> BigRational {
        value
    }
}

/// ICU conversion with exact factors.
#[derive(Debug)]
pub(crate) struct Conversion(UnitsConverter<Input>);
impl Conversion {
    pub fn new(from: &str, to: &str) -> Result<Self, NumberFormatError> {
        let parse = |name| {
            MeasureUnit::try_from_str(name)
                .map_err(|_| unsupported("unit", "unrecognized conversion unit"))
        };
        ConverterFactory::new()
            .converter::<Input>(&parse(from)?, &parse(to)?)
            .map(Self)
            .map_err(|_| unsupported("unit", "units are not convertible"))
    }
    pub fn convert(&self, value: &BigRational) -> Option<BigRational> {
        self.0.convert(Input(value.clone()))
    }
    pub fn positive_ratio(&self) -> Option<BigRational> {
        let zero = self.convert(&BigRational::zero())?;
        let one = self.convert(&BigRational::one())?;
        (zero.is_zero() && one > BigRational::zero()).then_some(one)
    }
}
