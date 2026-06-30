use crate::locale::ResolvedNumberLocale;

pub(crate) fn substitute_digits(input: &str, locale: &ResolvedNumberLocale) -> String {
    let Some(digits) = &locale.digits else {
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
