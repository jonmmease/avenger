# avenger-format-number-icu

Format numbers with ICU number skeletons and compiled locale data. The crate implements Avenger's number formatter traits in Rust.

```rust
use avenger_format::NumberFormatProvider;
use avenger_format_number_icu::IcuNumberFormatProvider;

let formatter = IcuNumberFormatProvider::new()
    .with_locale("fr-FR")
    .prepare(".00 group-off")?;
assert_eq!(formatter.format(1234.5).text, "1234,50");
# Ok::<(), avenger_format::NumberFormatError>(())
```

See [supported skeletons](skeletons.md) for syntax and options.
