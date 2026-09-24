use crate::locale::ResolvedNumberLocale;

/// Replace ASCII digits throughout the completed label, including its affixes and padding.
pub(crate) fn substitute_digits(input: &str, locale: &ResolvedNumberLocale) -> String {
    let Some(digits) = &locale.numerals else {
        return input.to_string();
    };

    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        if let Some(digit) = ch.to_digit(10) {
            output.push_str(&digits[digit as usize]);
        } else {
            output.push(ch);
        }
    }
    output
}
