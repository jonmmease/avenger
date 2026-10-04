use crate::arithmetic::Literal;
use crate::skeleton::{invalid, parse_digits};
use avenger_format::NumberFormatError;
use fixed_decimal::{Decimal, SignedRoundingMode};

#[derive(Debug, Clone, Copy)]
pub(crate) struct Digits {
    pub min: i16,
    pub max: Option<i16>,
}

/// Fraction and significant limits share one rounding decision before display padding.
#[derive(Debug, Clone)]
pub(crate) struct Precision {
    increment: Option<Literal>,
    fraction: Option<Digits>,
    significant: Option<Digits>,
    relaxed: bool,
    retain: bool,
    hide_whole: bool,
}

impl Default for Precision {
    fn default() -> Self {
        Self::fraction(Digits {
            min: 0,
            max: Some(6),
        })
    }
}

impl Precision {
    pub fn currency(&self, increment: Literal) -> Self {
        let mut result = Self::increment(increment);
        result.hide_whole = self.hide_whole;
        result
    }

    pub fn increment(increment: Literal) -> Self {
        Self {
            increment: Some(increment),
            fraction: None,
            significant: None,
            relaxed: true,
            retain: false,
            hide_whole: false,
        }
    }

    pub fn compact() -> Self {
        Self {
            increment: None,
            fraction: Some(Digits {
                min: 0,
                max: Some(0),
            }),
            significant: Some(Digits {
                min: 1,
                max: Some(2),
            }),
            relaxed: true,
            retain: false,
            hide_whole: false,
        }
    }

    pub fn fraction(digits: Digits) -> Self {
        Self {
            increment: None,
            fraction: Some(digits),
            significant: None,
            relaxed: true,
            retain: false,
            hide_whole: false,
        }
    }
    pub fn significant(digits: Digits) -> Self {
        Self {
            increment: None,
            fraction: None,
            significant: Some(digits),
            relaxed: true,
            retain: false,
            hide_whole: false,
        }
    }
    pub fn options(&mut self, options: &[&str], position: usize) -> Result<(), NumberFormatError> {
        for option in options {
            if *option == "w" && !self.hide_whole {
                self.hide_whole = true;
            } else if option.starts_with('@')
                && self.fraction.is_some()
                && self.significant.is_none()
                && !self.hide_whole
            {
                let (blueprint, priority) = match option.as_bytes().last() {
                    Some(b'r') => (&option[..option.len() - 1], Some(true)),
                    Some(b's') => (&option[..option.len() - 1], Some(false)),
                    _ => (*option, None),
                };
                let mut sig = parse_digits(blueprint, '@', position)?;
                if let Some(relaxed) = priority {
                    if sig.max.is_none() {
                        return Err(invalid(
                            "priority requires maximum significant digits",
                            position,
                        ));
                    }
                    self.relaxed = relaxed;
                } else {
                    self.retain = true;
                    if sig.max.is_none() {
                        sig.max = Some(sig.min);
                        sig.min = 1;
                        self.relaxed = true;
                    } else if sig.min == 1 {
                        self.relaxed = false;
                    } else {
                        return Err(invalid("combined precision requires a priority", position));
                    }
                }
                self.significant = Some(sig);
            } else {
                return Err(invalid("invalid precision option", position));
            }
        }
        Ok(())
    }

    /// Significant-digit rounding alone keeps one integer digit for zero, as in ICU.
    pub fn keeps_zero_integer(&self) -> bool {
        self.significant.is_some() && self.fraction.is_none()
    }

    pub fn apply(&self, value: &mut Decimal, mode: SignedRoundingMode) {
        if let Some(increment) = &self.increment {
            increment.round(value, mode);
            if self.hide_whole {
                value.trim_end_if_integer();
            }
            return;
        }
        let magnitude = value.nonzero_magnitude_start();
        let frac_round = self
            .fraction
            .and_then(|d| d.max)
            .map(|n| -n)
            .unwrap_or(i16::MIN + 1);
        let mut sig_round = self
            .significant
            .and_then(|d| d.max)
            .map(|n| magnitude - n + 1)
            .unwrap_or(i16::MIN + 1);
        let rounding = match (self.fraction, self.significant) {
            (Some(_), Some(_)) if self.relaxed => frac_round.min(sig_round),
            (Some(_), Some(_)) => frac_round.max(sig_round),
            (Some(_), _) => frac_round,
            _ => sig_round,
        };
        if rounding > i16::MIN + 1 {
            value.round_with_mode(rounding, mode);
        }
        if !value.is_zero()
            && value.nonzero_magnitude_start() != magnitude
            && frac_round == sig_round
        {
            sig_round += 1;
        }
        value.trim_end();
        let frac_display = -self.fraction.map_or(0, |d| d.min);
        let sig_display =
            value.nonzero_magnitude_start() - self.significant.map_or(1, |d| d.min) + 1;
        let display = match (self.fraction, self.significant) {
            (Some(_), Some(_)) if self.retain => frac_display.min(sig_display),
            (Some(_), Some(_)) if (sig_round <= frac_round) == self.relaxed => sig_display,
            (Some(_), _) => frac_display,
            _ => sig_display,
        };
        value.pad_end(display.min(0));
        if self.hide_whole {
            value.trim_end_if_integer();
        }
    }
}
