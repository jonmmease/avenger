use num_bigint::BigUint;
use num_traits::{One, Zero};

pub(crate) fn shortest(value: f64) -> String {
    ryu_js::Buffer::new().format(value).to_owned()
}

fn scaled_ratio(value: f64, decimal_places: i32) -> (BigUint, BigUint) {
    let bits = value.to_bits();
    let encoded_exponent = ((bits >> 52) & 0x7ff) as i32;
    let significand = (bits & ((1_u64 << 52) - 1))
        | if encoded_exponent == 0 {
            0
        } else {
            1_u64 << 52
        };
    let exponent = if encoded_exponent == 0 {
        -1074
    } else {
        encoded_exponent - 1023 - 52
    };
    let mut numerator = BigUint::from(significand);
    let mut denominator = BigUint::one();
    if exponent >= 0 {
        numerator <<= exponent as usize;
    } else {
        denominator <<= (-exponent) as usize;
    }
    if decimal_places >= 0 {
        numerator *= BigUint::from(10_u8).pow(decimal_places as u32);
    } else {
        denominator *= BigUint::from(10_u8).pow((-decimal_places) as u32);
    }
    (numerator, denominator)
}

// Binary64 values are exact ratios. ECMAScript decimal rounding chooses the larger integer at a tie.
fn rounded_scaled(value: f64, decimal_places: i32) -> BigUint {
    let (numerator, denominator) = scaled_ratio(value, decimal_places);
    let quotient = &numerator / &denominator;
    if (&numerator % &denominator) * 2_u8 >= denominator {
        quotient + 1_u8
    } else {
        quotient
    }
}

pub(crate) fn shortest_parts(value: f64) -> (String, i32) {
    let text = shortest(value);
    let (body, power) = text
        .split_once('e')
        .map(|(body, exponent)| (body, exponent.parse::<i32>().unwrap()))
        .unwrap_or((&text, 0));
    let decimal = body.find('.').unwrap_or(body.len()) as i32;
    let digits = body.replace('.', "");
    let zeros = digits.bytes().take_while(|byte| *byte == b'0').count();
    if zeros == digits.len() {
        return ("0".into(), 0);
    }
    let coefficient = digits[zeros..].trim_end_matches('0').to_owned();
    (coefficient, decimal - zeros as i32 - 1 + power)
}

pub(crate) fn exponent(value: f64) -> Option<i32> {
    (value.is_finite() && value != 0.0).then(|| shortest_parts(value.abs()).1)
}

pub(crate) fn parts(value: f64, precision: usize) -> (String, i32) {
    if value == 0.0 {
        return ("0".repeat(precision), 0);
    }
    let mut exponent = shortest_parts(value).1;
    // A shortest string can round up across a power of ten. Precision rounding needs the exact exponent.
    let (numerator, denominator) = scaled_ratio(value, -exponent);
    if numerator < denominator {
        exponent -= 1;
    }
    let mut digits = rounded_scaled(value, precision as i32 - exponent - 1).to_str_radix(10);
    if digits.len() > precision {
        digits.pop();
        exponent += 1;
    }
    (digits, exponent)
}

pub(crate) fn fixed(value: f64, precision: usize) -> String {
    if !value.is_finite() || value >= 1e21 {
        return shortest(value);
    }
    let digits = rounded_scaled(value, precision as i32).to_str_radix(10);
    let padded = format!(
        "{}{}",
        "0".repeat((precision + 1).saturating_sub(digits.len())),
        digits
    );
    if precision == 0 {
        padded
    } else {
        let index = padded.len() - precision;
        format!("{}.{}", &padded[..index], &padded[index..])
    }
}

pub(crate) fn exponential(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return shortest(value);
    }
    let (digits, exponent) = parts(value, precision + 1);
    let mantissa = if precision == 0 {
        digits
    } else {
        format!("{}.{}", &digits[..1], &digits[1..])
    };
    format!("{mantissa}e{exponent:+}")
}

pub(crate) fn precision(value: f64, precision: usize) -> String {
    if !value.is_finite() {
        return shortest(value);
    }
    let (digits, exponent) = parts(value, precision);
    if exponent < -6 || exponent >= precision as i32 {
        let mantissa = if precision == 1 {
            digits
        } else {
            format!("{}.{}", &digits[..1], &digits[1..])
        };
        format!("{mantissa}e{exponent:+}")
    } else {
        place_decimal(&digits, exponent + 1)
    }
}

pub(crate) fn rounded(value: f64, precision: usize) -> String {
    if !value.is_finite() || value == 0.0 {
        return shortest(value);
    }
    let (digits, exponent) = parts(value, precision);
    place_decimal(&digits, exponent + 1)
}

pub(crate) fn place_decimal(digits: &str, index: i32) -> String {
    if index <= 0 {
        format!("0.{}{}", "0".repeat((-index) as usize), digits)
    } else if index as usize >= digits.len() {
        format!("{}{}", digits, "0".repeat(index as usize - digits.len()))
    } else {
        format!(
            "{}.{}",
            &digits[..index as usize],
            &digits[index as usize..]
        )
    }
}

pub(crate) fn integer(value: f64, radix: u32, upper: bool) -> String {
    if radix == 10 && value.is_infinite() {
        return "∞".into();
    }
    if !value.is_finite() {
        let text = shortest(value);
        return if upper { text.to_uppercase() } else { text };
    }
    if radix == 10 && value < 1e21 {
        return shortest(value.round());
    }
    if radix == 10 && value >= 1e21 {
        let (digits, exponent) = shortest_parts(value);
        return place_decimal(&digits, exponent + 1);
    }
    let integer = rounded_scaled(value, 0);
    if integer.is_zero() {
        return "0".into();
    }
    let text = integer.to_str_radix(radix);
    if upper {
        text.to_uppercase()
    } else {
        text
    }
}
