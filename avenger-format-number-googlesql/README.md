# GoogleSQL number formatting

`avenger-format-number-googlesql` formats `f64` values with the printf-style patterns of GoogleSQL's `FORMAT`, with GoogleSQL's text. `GoogleSqlNumberFormatProvider` prepares the patterns for the labels of charts, with the syntax name `googlesql` and optionally the symbols of a d3 number locale, and implements the [number formatter interface](../avenger-format/README.md) from `avenger-format`.

```rust
use avenger_format::NumberFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;

fn main() -> Result<(), avenger_format::NumberFormatError> {
    let provider = GoogleSqlNumberFormatProvider::new();
    assert_eq!(provider.prepare("$%'.2f")?.format(1234.5).text, "$1,234.50");
    assert_eq!(provider.prepare("%.1f%%")?.format(12.25).text, "12.2%");
    assert_eq!(provider.prepare("%.3g")?.format(1234.5).text, "1.23e+03");
    Ok(())
}
```

A pattern is a whole `FORMAT` string: literal text, `%%` for a percent sign, and exactly one conversion of a number, such as `x = %.1f mm`. The conversion is `%`, then flags, a width, a precision, and one of the specifiers `d i o x X f F e E g G`:

- `d` and `i` print a decimal integer, `o` an octal integer, and `x` and `X` a hexadecimal integer in lowercase or uppercase. The precision is the least number of digits, and `%.0d` and `%.0i` print 0 as nothing, where `%.0x`, `%.0o`, and `%'.0d` print `0`.
- `f` and `F` print the precision's decimals, 6 by default. `%f` of 1e300 prints every digit of the value.
- `e` and `E` print one digit, the precision's decimals, and an exponent with a sign and at least two digits, such as `1.234500e+03`.
- `g` and `G` take the exponent X of the value in `%e` at the precision P less one, where P is 6 by default and 0 counts as 1. They print scientific notation when X is below -4 or at least P, and decimals otherwise, with P - 1 - X decimals. Trailing zeros go, as does a trailing point.

The flags come in any order:

- `-` pads on the right, and `0` pads with zeros between the sign and the digits. The zeros of `0` are not grouped. As in C, a precision of `%d` and `%i` turns `0` off, but `0` pads `%o`, `%x`, `%X`, and grouped decimals after their precision, as in `%08.3x` of 255, `000000ff`.
- `+` writes `+` before a value that is not negative, and a space writes a space.
- `#` writes the prefixes `0`, `0x`, and `0X` before octal and hexadecimal integers, zero too, keeps the point of `f`, `e`, and `g` where they have no decimals, and keeps the trailing zeros of `g`. An octal integer takes no prefix where its precision already writes leading zeros, so `%#.6o` prints 8 as `000010`.
- `'` groups the digits before the point of `d`, `i`, `f`, `F`, and `g` in decimals by `,` in threes, as in `1,234.5`. It groups octal digits by `,` and hexadecimal digits by `:` in fours, as in `bc:614e`. The zeros of an integer's precision group too, as in `00,000,042`. Scientific notation is not grouped.

The text of a value rounds the exact binary value, half to even on exact ties, so `%.0f` prints 2.5 as `2` and 3.5 as `4`, and `%.2f` prints 1.005 as `1.00`, since the double nearest 1.005 is below it. Negative zero prints without a sign, and a negative value that rounds to zero keeps its sign, as `-0.0` for -0.04 in `%.1f`. NaN and the infinities print as `nan`, `inf`, and `-inf` in `f`, `e`, and `g`, and in uppercase in `F`, `E`, and `G`, and pad with spaces even with `0`.

## A chart's patterns

GoogleSQL applies the integer conversions to `INT64` values only, but a chart's values are `f64`. The integer conversions round a value half to even to an integer, and print that integer exactly at any size, so `%d` of 2.5 prints `2` and of 1e300 prints all of its digits. An integer has no sign at zero, so `%d` of -0.4 prints `0`. NaN and the infinities print in the integer conversions as in `%f`. A negative value prints in `o`, `x`, and `X` as `-` and the digits of its magnitude, as GoogleSQL's `execute_query` prints a negative `INT64`.

`prepare` rejects a pattern without a conversion or with several, a `*` width or precision, which reads an argument that a chart's pattern does not have, a specifier other than the eleven above, such as `%s`, `%t`, `%T`, `%p`, `%P`, `%u`, or `%a`, and a lone `%` at the end. The error is `NumberFormatError::InvalidPattern`, with the byte position of the conversion or of the `*`, and names the element. A width or precision over 1,000 is an error too.

The empty pattern stands for no pattern, as for an axis or a label without a format:

- `format` prints a single value, such as a tooltip's, grouped, with the fewest decimals that read back as the value, as `1,234,567.891`. A magnitude of at least 1e21, or above zero and below 1e-6, prints as `%g`, as `1e+25` and `1e-09`.
- `format_all` with `PrecisionSharing::Shared`, as for evenly spaced ticks, prints each tick as `%'.Nf`, with N decimals from the resolution of the ticks' step, as `TickStep::infer` finds it. So ticks from 0 to 1 by 0.25 print `0.00`, `0.25`, and on to `1.00`, and ticks 0, 20000, and 40000 print `0`, `20,000`, and `40,000`. A tick that rounds to zero prints `0` without a sign. Ticks whose largest magnitude is extreme, as above, share the decimals of their mantissas in scientific notation, as `%.Ne`, with N the places from the leading digit of the largest tick to the resolution of their step, so ticks 1e-7, 1.5e-7, and 2e-7 print `1.0e-07`, `1.5e-07`, and `2.0e-07`, and a zero tick prints `0`.
- `format_all` with `PrecisionSharing::PerValue` prints each value as `format` does.

A written pattern formats as written in `format` and in `format_all`, since GoogleSQL's precision defaults to 6 and never leaves the precision open.

`FormattedNumber` holds the text of a written pattern as plain text. Without a pattern, a value in scientific notation also holds its mantissa and exponent, for a label to set as a power of ten.

## GoogleSQL's `FORMAT`

The same engine prints the elements of GoogleSQL's `FORMAT` function, which avenger-sql implements. `pieces` parses a whole pattern into literal text and `Element`s, leniently: an element is `%`, its flags, a width and a precision of digits or `*`, and any character as its specifier, so a caller decides which specifiers it takes and with which messages. A pattern that ends inside an element is the only error.

`Element::format_number` prints a `Value`: an `INT64`, exactly at any size, a `FLOAT64`, or a `Decimal`, the digits and scale of a `NUMERIC` or `BIGNUMERIC`, which prints exactly and rounds half away from zero, as GoogleSQL's `NUMERIC` does. `Element::format_text` prints the text of `%s`, `%t`, and `%T`: its first characters up to the precision, padded with spaces to the width. Both take the values of `*` arguments, where a negative width pads on the right and a negative precision is none.

```rust
use avenger_format_number_googlesql::{pieces, Decimal, Locale, Piece, Value};

let Piece::Element(element) = &pieces("%'.2f").unwrap()[0] else { unreachable!() };
let decimal = Decimal { negative: false, digits: "123456789", scale: 3 };
assert_eq!(element.format_number(&Locale::default(), None, None, Value::Decimal(decimal)), "123,456.79");
let Piece::Element(element) = &pieces("%-*s|").unwrap()[0] else { unreachable!() };
assert_eq!(element.format_text(Some(5), None, "ab"), "ab   ");
```

## Locales

GoogleSQL's `FORMAT` has no locale: it always groups with `,`, writes the point as `.`, and writes `-` and ASCII digits. A provider without a locale writes exactly that. A locale from d3's number locales, such as `with_locale("de-DE")`, changes the symbols, as an extension of GoogleSQL:

- `decimal` replaces the point of `f`, `F`, `e`, `E`, `g`, and `G`.
- `thousands` and `grouping` are what `'` inserts between the digits of `d`, `i`, `f`, `F`, and `g` in decimals. The group sizes cycle from the right, and the last one repeats, so en-IN's `[3, 2]` groups 1234567 as `12,34,567`. An empty `thousands` or `grouping` inserts nothing. The zeros of `0` stay ungrouped.
- `minus` replaces the sign `-`. Most of d3's locales leave it to its default, U+2212 `−`.
- `nan` replaces `nan` and `NAN` as written, in every specifier: GoogleSQL's case rule applies only without a locale. `inf` keeps GoogleSQL's text and case, since d3's locales have no text for it.
- `numerals` replace the digits of decimal output, the exponent's digits, and the zeros of padding.

Octal and hexadecimal digits, and their groups of four by `,` and `:`, keep GoogleSQL's text in every locale. The locale's `currency` and `percent` do not apply: `$` and `%%` in a pattern are literal text. A width counts characters, so a multi-byte separator or minus pads as one character.

```rust
use avenger_format::NumberFormatProvider;
use avenger_format_number_googlesql::{GoogleSqlNumberFormatProvider, NumberLocaleSpec};

fn main() -> Result<(), avenger_format::NumberFormatError> {
    let german = NumberLocaleSpec {
        decimal: ",".into(),
        thousands: ".".into(),
        grouping: vec![3],
        ..Default::default()
    };
    let provider = GoogleSqlNumberFormatProvider::new()
        .with_custom_locale("de-DE", german)
        .with_locale("de_DE");
    assert_eq!(provider.prepare("%'.2f")?.format(-1234567.891).text, "−1.234.567,89");
    assert_eq!(provider.prepare("")?.format(1234.5).text, "1.234,5");
    Ok(())
}
```

The provider selects a locale by name, with hyphens or underscores, as the d3 providers do: a custom definition from `with_custom_locale` first, then a bundled one. An unknown name is `NumberFormatError::LocaleUnavailable`, and a group size of zero is `NumberFormatError::InvalidLocaleData`. The default build bundles en-US, whose minus and NaN differ from GoogleSQL's, and the `all-locales` feature bundles all of d3's number locales, from `avenger-format-locales-d3`. The empty pattern follows the locale too, so an axis without a pattern writes the locale's point, groups, and minus.

## Known gaps

The recordings do not cover `0` together with the precision of an integer, nor `#` together with `0`. The crate follows C there: the precision turns `0` off, and the zeros of `0` go after the prefix of `#`, as in `0x002a`.

## Tests

The tests compare each case of `tests/data/format.tsv`, which GoogleSQL's `execute_query` recorded, with the provider's text. The float conversions ran on `FLOAT64` values, and the integer conversions on `INT64` values. The cases run without a locale, and `tests/locale.rs` checks the symbols of d3's de-DE, en-IN, fr-FR, and ar-EG locales and of custom ones.
