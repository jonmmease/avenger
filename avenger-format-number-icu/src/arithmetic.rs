use fixed_decimal::Decimal;

/// Use the shortest decimal that round-trips to the same f64, retaining negative zero.
pub(crate) fn from_float(value: f64) -> Decimal {
    Decimal::try_from_f64(value, fixed_decimal::FloatPrecision::RoundTrip)
        .expect("finite f64 fits Decimal")
}
