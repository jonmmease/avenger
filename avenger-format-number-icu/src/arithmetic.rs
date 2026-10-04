use crate::skeleton::{invalid, unsupported};
use avenger_format::NumberFormatError;
use fixed_decimal::{Decimal, Sign, SignedRoundingMode as R, UnsignedRoundingMode as U};
use num_bigint::BigUint;
use num_integer::Integer;

/// Exact decimal literal, retaining written scale for increment padding.
#[derive(Debug, Clone)]
pub(crate) struct Literal {
    coefficient: BigUint,
    exponent: i16,
    negative: bool,
    fraction_digits: i16,
}

impl Literal {
    pub fn power(exponent: i16) -> Self {
        Self {
            coefficient: 1u8.into(),
            exponent,
            negative: false,
            fraction_digits: (-exponent).max(0),
        }
    }

    pub fn parse(text: &str, position: usize) -> Result<Self, NumberFormatError> {
        let syntax = || invalid("invalid decimal literal", position);
        let bounds = || {
            unsupported(
                text,
                "decimal literals support at most 999 digits and magnitudes from -1000 to 1000",
            )
        };
        if text.len() > 1010 {
            return Err(bounds());
        }
        let (negative, unsigned) = if let Some(s) = text.strip_prefix('-') {
            (true, s)
        } else {
            (false, text.strip_prefix('+').unwrap_or(text))
        };
        let mut pieces = unsigned.split(['e', 'E']);
        let mantissa = pieces.next().ok_or_else(syntax)?;
        let power: i32 = match pieces.next() {
            Some(s) => s.parse().map_err(|_| syntax())?,
            None => 0,
        };
        if pieces.next().is_some() {
            return Err(syntax());
        }
        if !(-1000..=1000).contains(&power) {
            return Err(bounds());
        }
        let mut digits = String::new();
        let mut fraction = None;
        for b in mantissa.bytes() {
            if b.is_ascii_digit() {
                digits.push(char::from(b));
                if let Some(n) = &mut fraction {
                    *n += 1;
                }
            } else if b == b'.' && fraction.is_none() {
                fraction = Some(0i32);
            } else {
                return Err(syntax());
            }
        }
        if digits.is_empty() {
            return Err(syntax());
        }
        if digits.len() > 999 {
            return Err(bounds());
        }
        let exponent = power - fraction.unwrap_or(0);
        let significant = digits.trim_start_matches('0');
        let magnitude = exponent + significant.len() as i32 - 1;
        if exponent < -1000 || magnitude > 1000 {
            return Err(bounds());
        }
        let fraction_digits = (-exponent).max(0) as i16;
        let trimmed = digits.trim_end_matches('0');
        let zeros = digits.len() - trimmed.len();
        let (coefficient, exponent) = if trimmed.is_empty() {
            (BigUint::from(0u8), 0)
        } else {
            (
                BigUint::parse_bytes(trimmed.as_bytes(), 10).ok_or_else(syntax)?,
                (exponent + zeros as i32) as i16,
            )
        };
        Ok(Self {
            negative: negative && coefficient != BigUint::from(0u8),
            coefficient,
            exponent,
            fraction_digits,
        })
    }

    pub fn positive(&self) -> bool {
        !self.negative && self.coefficient != BigUint::from(0u8)
    }

    pub fn scale(&self, value: &mut Decimal) {
        // ICU preserves zero and non-finite signs before applying an arbitrary multiplier.
        if value.is_zero() {
            return;
        }
        if self.coefficient == BigUint::from(1u8) {
            value.multiply_pow10(self.exponent);
            if self.negative {
                value.sign = if value.sign == Sign::Negative {
                    Sign::None
                } else {
                    Sign::Negative
                };
            }
        } else {
            let (coefficient, exponent) = coefficient(value);
            let negative = (value.sign == Sign::Negative) ^ self.negative;
            let product = coefficient * &self.coefficient;
            *value = from_coefficient(
                &product,
                exponent + self.exponent,
                negative && product != BigUint::from(0u8),
            );
        }
    }

    pub fn round(&self, value: &mut Decimal, mode: R) {
        let (coefficient, exponent) = coefficient(value);
        let common = exponent.min(self.exponent);
        let numerator = coefficient * ten((exponent - common) as u32);
        let divisor = &self.coefficient * ten((self.exponent - common) as u32);
        let negative = value.sign == Sign::Negative;
        let (mut quotient, remainder) = numerator.div_rem(&divisor);
        if round_up(&quotient, &remainder, &divisor, negative, mode) {
            quotient += 1u8;
        }
        *value = from_coefficient(&(quotient * &self.coefficient), self.exponent, negative);
        value.trim_end();
        value.pad_end(-self.fraction_digits);
    }
}

fn ten(power: u32) -> BigUint {
    BigUint::from(10u8).pow(power)
}

/// Return the unsigned coefficient and power of ten, independent of display padding.
fn coefficient(value: &Decimal) -> (BigUint, i16) {
    let low = value.nonzero_magnitude_end();
    let high = value.nonzero_magnitude_start();
    let digits: Vec<u8> = (low..=high)
        .rev()
        .map(|m| b'0' + value.digit_at(m))
        .collect();
    (
        BigUint::parse_bytes(&digits, 10).expect("decimal digits form an integer"),
        low,
    )
}

fn from_coefficient(coefficient: &BigUint, exponent: i16, negative: bool) -> Decimal {
    let mut value: Decimal = coefficient
        .to_str_radix(10)
        .parse()
        .expect("validated coefficient fits Decimal");
    value.multiply_pow10(exponent);
    value.sign = if negative { Sign::Negative } else { Sign::None };
    value
}

fn round_up(
    quotient: &BigUint,
    remainder: &BigUint,
    divisor: &BigUint,
    negative: bool,
    mode: R,
) -> bool {
    if *remainder == BigUint::from(0u8) {
        return false;
    }
    let half = (remainder * 2u8).cmp(divisor);
    match mode {
        R::Ceil => !negative,
        R::Floor => negative,
        R::Unsigned(U::Expand) => true,
        R::Unsigned(U::Trunc) => false,
        R::Unsigned(U::HalfExpand) => !half.is_lt(),
        R::Unsigned(U::HalfTrunc) => half.is_gt(),
        _ => half.is_gt() || (half.is_eq() && quotient.is_odd()),
    }
}

pub(crate) fn rational(value: &Decimal) -> num_rational::BigRational {
    use num_bigint::BigInt;
    let (coefficient, exponent) = coefficient(value);
    let sign = if value.sign == Sign::Negative {
        num_bigint::Sign::Minus
    } else {
        num_bigint::Sign::Plus
    };
    let coefficient = BigInt::from_biguint(sign, coefficient);
    if exponent >= 0 {
        num_rational::Ratio::from_integer(coefficient * BigInt::from(ten(exponent as u32)))
    } else {
        num_rational::Ratio::new(coefficient, BigInt::from(ten((-exponent) as u32)))
    }
}

/// Round a conversion result to 34 significant digits, as DECIMAL128 does, without passing it
/// through f64.
pub(crate) fn decimal128(value: &num_rational::BigRational) -> Decimal {
    use num_traits::{Signed, Zero};
    if value.is_zero() {
        return Decimal::from(0);
    }
    let n = value.numer().magnitude();
    let d = value.denom().magnitude();
    let mut magnitude = n.to_str_radix(10).len() as i16 - d.to_str_radix(10).len() as i16;
    let below = if magnitude >= 0 {
        n < &(d * ten(magnitude as u32))
    } else {
        &(n * ten((-magnitude) as u32)) < d
    };
    if below {
        magnitude -= 1;
    }
    let exponent = magnitude - 33;
    let (numerator, divisor) = if exponent < 0 {
        (n * ten((-exponent) as u32), d.clone())
    } else {
        (n.clone(), d * ten(exponent as u32))
    };
    let (mut quotient, remainder) = numerator.div_rem(&divisor);
    if round_up(
        &quotient,
        &remainder,
        &divisor,
        value.is_negative(),
        R::Unsigned(U::HalfEven),
    ) {
        quotient += 1u8;
    }
    let mut result = from_coefficient(&quotient, exponent, value.is_negative());
    result.trim_end();
    result
}

/// Use the shortest decimal that round-trips to the same f64, retaining negative zero.
pub(crate) fn from_float(value: f64) -> Decimal {
    Decimal::try_from_f64(value, fixed_decimal::FloatPrecision::RoundTrip)
        .expect("finite f64 fits Decimal")
}
