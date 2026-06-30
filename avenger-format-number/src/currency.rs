use crate::{error::FormatError, locale::ResolvedNumberLocale};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CurrencyMetadata {
    pub code: &'static str,
    pub default_fraction_digits: u8,
}

// Source: Unicode CLDR v48 supplemental/currencyData.json. Codes are the union
// of CLDR region currency history and fraction metadata. Fraction digits use
// `currencyData.fractions.*._digits`, falling back to CLDR DEFAULT digits.
const CURRENCY_FRACTION_DIGITS: &[(&str, u8)] = &[
    ("ADP", 0),
    ("AED", 2),
    ("AFA", 2),
    ("AFN", 0),
    ("ALK", 2),
    ("ALL", 0),
    ("AMD", 2),
    ("ANG", 2),
    ("AOA", 2),
    ("AOK", 2),
    ("AON", 2),
    ("AOR", 2),
    ("ARA", 2),
    ("ARL", 2),
    ("ARM", 2),
    ("ARP", 2),
    ("ARS", 2),
    ("ATS", 2),
    ("AUD", 2),
    ("AWG", 2),
    ("AZM", 2),
    ("AZN", 2),
    ("BAD", 2),
    ("BAM", 2),
    ("BAN", 2),
    ("BBD", 2),
    ("BDT", 2),
    ("BEC", 2),
    ("BEF", 2),
    ("BEL", 2),
    ("BGL", 2),
    ("BGM", 2),
    ("BGN", 2),
    ("BGO", 2),
    ("BHD", 3),
    ("BIF", 0),
    ("BMD", 2),
    ("BND", 2),
    ("BOB", 2),
    ("BOL", 2),
    ("BOP", 2),
    ("BOV", 2),
    ("BRB", 2),
    ("BRC", 2),
    ("BRE", 2),
    ("BRL", 2),
    ("BRN", 2),
    ("BRR", 2),
    ("BRZ", 2),
    ("BSD", 2),
    ("BTN", 2),
    ("BUK", 2),
    ("BWP", 2),
    ("BYB", 2),
    ("BYN", 2),
    ("BYR", 0),
    ("BZD", 2),
    ("CAD", 2),
    ("CDF", 2),
    ("CHE", 2),
    ("CHF", 2),
    ("CHW", 2),
    ("CLE", 2),
    ("CLF", 4),
    ("CLP", 0),
    ("CNH", 2),
    ("CNX", 2),
    ("CNY", 2),
    ("COP", 0),
    ("COU", 2),
    ("CRC", 2),
    ("CSD", 2),
    ("CSK", 2),
    ("CUC", 2),
    ("CUP", 2),
    ("CVE", 2),
    ("CYP", 2),
    ("CZK", 2),
    ("DDM", 2),
    ("DEM", 2),
    ("DJF", 0),
    ("DKK", 2),
    ("DOP", 2),
    ("DZD", 2),
    ("ECS", 2),
    ("ECV", 2),
    ("EEK", 2),
    ("EGP", 2),
    ("ERN", 2),
    ("ESA", 2),
    ("ESB", 2),
    ("ESP", 0),
    ("ETB", 2),
    ("EUR", 2),
    ("FIM", 2),
    ("FJD", 2),
    ("FKP", 2),
    ("FRF", 2),
    ("GBP", 2),
    ("GEK", 2),
    ("GEL", 2),
    ("GHC", 2),
    ("GHS", 2),
    ("GIP", 2),
    ("GMD", 2),
    ("GNF", 0),
    ("GNS", 2),
    ("GQE", 2),
    ("GRD", 2),
    ("GTQ", 2),
    ("GWE", 2),
    ("GWP", 2),
    ("GYD", 2),
    ("HKD", 2),
    ("HNL", 2),
    ("HRD", 2),
    ("HRK", 2),
    ("HTG", 2),
    ("HUF", 0),
    ("IDR", 0),
    ("IEP", 2),
    ("ILP", 2),
    ("ILR", 2),
    ("ILS", 2),
    ("INR", 2),
    ("IQD", 0),
    ("IRR", 0),
    ("ISJ", 2),
    ("ISK", 0),
    ("ITL", 0),
    ("JMD", 2),
    ("JOD", 3),
    ("JPY", 0),
    ("KES", 2),
    ("KGS", 2),
    ("KHR", 2),
    ("KMF", 0),
    ("KPW", 0),
    ("KRH", 2),
    ("KRO", 2),
    ("KRW", 0),
    ("KWD", 3),
    ("KYD", 2),
    ("KZT", 2),
    ("LAK", 0),
    ("LBP", 0),
    ("LKR", 2),
    ("LRD", 2),
    ("LSL", 2),
    ("LTL", 2),
    ("LTT", 2),
    ("LUC", 2),
    ("LUF", 0),
    ("LUL", 2),
    ("LVL", 2),
    ("LVR", 2),
    ("LYD", 3),
    ("MAD", 2),
    ("MAF", 2),
    ("MCF", 2),
    ("MDC", 2),
    ("MDL", 2),
    ("MGA", 0),
    ("MGF", 0),
    ("MKD", 2),
    ("MKN", 2),
    ("MLF", 2),
    ("MMK", 0),
    ("MNT", 2),
    ("MOP", 2),
    ("MRO", 0),
    ("MRU", 2),
    ("MTL", 2),
    ("MTP", 2),
    ("MUR", 2),
    ("MVP", 2),
    ("MVR", 2),
    ("MWK", 2),
    ("MXN", 2),
    ("MXP", 2),
    ("MXV", 2),
    ("MYR", 2),
    ("MZE", 2),
    ("MZM", 2),
    ("MZN", 2),
    ("NAD", 2),
    ("NGN", 2),
    ("NIC", 2),
    ("NIO", 2),
    ("NLG", 2),
    ("NOK", 2),
    ("NPR", 2),
    ("NZD", 2),
    ("OMR", 3),
    ("PAB", 2),
    ("PEI", 2),
    ("PEN", 2),
    ("PES", 2),
    ("PGK", 2),
    ("PHP", 2),
    ("PKR", 0),
    ("PLN", 2),
    ("PLZ", 2),
    ("PTE", 2),
    ("PYG", 0),
    ("QAR", 2),
    ("RHD", 2),
    ("ROL", 2),
    ("RON", 2),
    ("RSD", 2),
    ("RUB", 2),
    ("RUR", 2),
    ("RWF", 0),
    ("SAR", 2),
    ("SBD", 2),
    ("SCR", 2),
    ("SDD", 2),
    ("SDG", 2),
    ("SDP", 2),
    ("SEK", 2),
    ("SGD", 2),
    ("SHP", 2),
    ("SIT", 2),
    ("SKK", 2),
    ("SLE", 2),
    ("SLL", 0),
    ("SOS", 0),
    ("SRD", 2),
    ("SRG", 2),
    ("SSP", 2),
    ("STD", 0),
    ("STN", 2),
    ("SUR", 2),
    ("SVC", 2),
    ("SYP", 0),
    ("SZL", 2),
    ("THB", 2),
    ("TJR", 2),
    ("TJS", 2),
    ("TMM", 0),
    ("TMT", 2),
    ("TND", 3),
    ("TOP", 2),
    ("TPE", 2),
    ("TRL", 0),
    ("TRY", 2),
    ("TTD", 2),
    ("TWD", 2),
    ("TZS", 2),
    ("UAH", 2),
    ("UAK", 2),
    ("UGS", 2),
    ("UGX", 0),
    ("USD", 2),
    ("USN", 2),
    ("USS", 2),
    ("UYI", 0),
    ("UYP", 2),
    ("UYU", 2),
    ("UYW", 4),
    ("UZS", 2),
    ("VEB", 2),
    ("VED", 2),
    ("VEF", 2),
    ("VES", 2),
    ("VND", 0),
    ("VNN", 2),
    ("VUV", 0),
    ("WST", 2),
    ("XAD", 2),
    ("XAF", 0),
    ("XAG", 2),
    ("XAU", 2),
    ("XBA", 2),
    ("XBB", 2),
    ("XBC", 2),
    ("XBD", 2),
    ("XCD", 2),
    ("XCG", 2),
    ("XDR", 2),
    ("XEU", 2),
    ("XFO", 2),
    ("XFU", 2),
    ("XOF", 0),
    ("XPD", 2),
    ("XPF", 0),
    ("XPT", 2),
    ("XRE", 2),
    ("XSU", 2),
    ("XTS", 2),
    ("XUA", 2),
    ("XXX", 2),
    ("YDD", 2),
    ("YER", 0),
    ("YUD", 2),
    ("YUM", 2),
    ("YUN", 2),
    ("YUR", 2),
    ("ZAL", 2),
    ("ZAR", 2),
    ("ZMK", 0),
    ("ZMW", 2),
    ("ZRN", 2),
    ("ZRZ", 2),
    ("ZWD", 0),
    ("ZWG", 2),
    ("ZWL", 2),
    ("ZWR", 2),
];

pub fn currency_metadata(code: &str) -> Option<CurrencyMetadata> {
    CURRENCY_FRACTION_DIGITS
        .binary_search_by(|(metadata_code, _)| metadata_code.cmp(&code))
        .ok()
        .map(|idx| {
            let (code, default_fraction_digits) = CURRENCY_FRACTION_DIGITS[idx];
            CurrencyMetadata {
                code,
                default_fraction_digits,
            }
        })
}

pub fn validate_currency_code(code: &str) -> Result<(), FormatError> {
    if currency_metadata(code).is_some() {
        Ok(())
    } else {
        Err(FormatError::InvalidCurrencyCode(code.to_string()))
    }
}

pub(crate) fn currency_display_text(
    locale: &ResolvedNumberLocale,
    code: &str,
    display: crate::locale::CurrencyDisplay,
) -> String {
    let Some(names) = locale.currency_names.get(code) else {
        return code.to_string();
    };

    match display {
        crate::locale::CurrencyDisplay::Symbol => names
            .symbol
            .as_ref()
            .or(names.narrow_symbol.as_ref())
            .cloned()
            .unwrap_or_else(|| code.to_string()),
        crate::locale::CurrencyDisplay::Code => code.to_string(),
        crate::locale::CurrencyDisplay::Name => {
            names.name.clone().unwrap_or_else(|| code.to_string())
        }
        crate::locale::CurrencyDisplay::NarrowSymbol => names
            .narrow_symbol
            .as_ref()
            .or(names.symbol.as_ref())
            .cloned()
            .unwrap_or_else(|| code.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{currency_metadata, validate_currency_code};

    #[test]
    fn resolves_global_currency_fraction_digits() {
        assert_eq!(
            currency_metadata("BHD")
                .expect("BHD metadata")
                .default_fraction_digits,
            3
        );
        assert_eq!(
            currency_metadata("CLF")
                .expect("CLF metadata")
                .default_fraction_digits,
            4
        );
        assert_eq!(
            currency_metadata("XOF")
                .expect("XOF metadata")
                .default_fraction_digits,
            0
        );
    }

    #[test]
    fn rejects_unknown_currency_codes() {
        assert!(validate_currency_code("ZZZ").is_err());
    }
}
