use crate::{Options, WidgetError, WidgetId, WidgetSpec};

/// Value mapping for a finite increasing horizontal slider domain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderDomain {
    min: f64,
    max: f64,
    span: f64,
    mode: SliderMode,
}
#[derive(Clone, Copy, Debug, PartialEq)]
enum SliderMode {
    Continuous { keyboard_increment: f64 },
    Stepped { step: f64 },
}
impl SliderDomain {
    /// Continuous values with a keyboard increment of one hundredth of the span.
    pub fn continuous(min: f64, max: f64) -> Result<Self, WidgetError> {
        Self::continuous_with_increment(min, max, (max - min) / 100.0)
    }
    pub fn continuous_with_increment(
        min: f64,
        max: f64,
        keyboard_increment: f64,
    ) -> Result<Self, WidgetError> {
        Self::new(min, max, SliderMode::Continuous { keyboard_increment })
    }
    /// Minimum-anchored steps plus the maximum endpoint, even for a short last interval.
    pub fn stepped(min: f64, max: f64, step: f64) -> Result<Self, WidgetError> {
        Self::new(min, max, SliderMode::Stepped { step })
    }
    fn new(min: f64, max: f64, mode: SliderMode) -> Result<Self, WidgetError> {
        let span = max - min;
        let increment = match mode {
            SliderMode::Continuous { keyboard_increment } => keyboard_increment,
            SliderMode::Stepped { step } => step,
        };
        let spacing = (min.next_up() - min).max(max - max.next_down());
        if increment < spacing
            || !min.is_finite()
            || !max.is_finite()
            || !span.is_finite()
            || span <= 0.0
            || !increment.is_finite()
            || increment <= 0.0
            || min + increment <= min
            || max - increment >= max
        {
            return Err(WidgetError::Invalid("slider bounds require a finite positive span and a representable positive increment".into()));
        }
        if matches!(mode, SliderMode::Stepped { .. }) && span / increment > 2.0_f64.powi(52) {
            return Err(WidgetError::Invalid(
                "slider step creates more positions than can be indexed precisely".into(),
            ));
        }
        Ok(Self {
            min,
            max,
            span,
            mode,
        })
    }
    pub fn min(self) -> f64 {
        self.min
    }
    pub fn max(self) -> f64 {
        self.max
    }
    pub fn step(self) -> Option<f64> {
        match self.mode {
            SliderMode::Stepped { step } => Some(step),
            _ => None,
        }
    }
    /// Clamp a finite value, choosing the nearest allowed step with ties upward.
    pub fn normalize(self, value: f64) -> Result<f64, WidgetError> {
        if !value.is_finite() {
            return Err(WidgetError::Invalid("slider values must be finite".into()));
        }
        let value = value.clamp(self.min, self.max);
        let SliderMode::Stepped { step } = self.mode else {
            return Ok(value);
        };
        if value == self.max {
            return Ok(value);
        }
        let index = ((value - self.min) / step).floor();
        let lower = (self.min + index * step).clamp(self.min, self.max);
        let upper = (self.min + (index + 1.0) * step).min(self.max);
        Ok(if value - lower < upper - value {
            lower
        } else {
            upper
        })
    }
    pub(crate) fn fraction(self, value: f64) -> f32 {
        ((value - self.min) / self.span).clamp(0.0, 1.0) as f32
    }
    pub(crate) fn at_fraction(self, fraction: f32) -> f64 {
        self.normalize(self.min + self.span * f64::from(fraction.clamp(0.0, 1.0)))
            .unwrap()
    }
    pub(crate) fn advance(self, value: f64, steps: i32) -> f64 {
        match self.mode {
            SliderMode::Continuous { keyboard_increment } => {
                let amount = keyboard_increment * f64::from(steps.unsigned_abs());
                if steps >= 0 {
                    value + amount.min(self.max - value)
                } else {
                    value - amount.min(value - self.min)
                }
            }
            SliderMode::Stepped { step } => {
                let index = if value == self.max {
                    let n = (self.span / step).floor();
                    if self.min + n * step < self.max {
                        n + 1.0
                    } else {
                        n
                    }
                } else {
                    ((value - self.min) / step).round()
                };
                let next = (index + f64::from(steps)).max(0.0);
                if next >= self.span / step {
                    self.max
                } else {
                    (self.min + next * step).clamp(self.min, self.max)
                }
            }
        }
    }
}

/// A numeric thumb. Optional readout text uses a fixed width from the slider style.
#[derive(Clone, Debug)]
pub struct Slider {
    pub(crate) options: Options,
    pub(crate) domain: SliderDomain,
    pub(crate) value: f64,
    pub(crate) value_label: Option<String>,
}
impl Slider {
    pub fn new(id: impl Into<WidgetId>, domain: SliderDomain, value: f64) -> Self {
        Self {
            options: Options::new(id),
            domain,
            value,
            value_label: None,
        }
    }
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.options.enabled = enabled;
        self
    }
    pub fn semantic_name(mut self, name: impl Into<String>) -> Self {
        self.options.semantic_name = Some(name.into());
        self
    }
    pub fn value_label(mut self, label: impl Into<String>) -> Self {
        self.value_label = Some(label.into());
        self
    }
}
impl From<Slider> for WidgetSpec {
    fn from(value: Slider) -> Self {
        Self::Slider(value)
    }
}

/// Why a slider transaction ended without a successful release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SliderCancelReason {
    Escape,
    FocusLost,
    CaptureLost,
    Disabled,
    Removed,
    Replaced,
}

pub(crate) fn track(
    rect: crate::Rect,
    style: &crate::SliderStyle,
    has_readout: bool,
) -> (f32, f32) {
    let available = (rect.width
        - if has_readout {
            style.readout_width + style.readout_gap
        } else {
            0.0
        })
    .max(0.0);
    let thumb = style.thumb_size.min(available).min(rect.height);
    (rect.x + thumb / 2.0, (available - thumb).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalization_and_navigation_agree_at_short_last_interval() {
        let d = SliderDomain::stepped(0.0, 10.0, 3.0).unwrap();
        assert_eq!(
            [0.0, 1.5, 3.0, 7.5, 9.0, 9.5, 10.0].map(|v| d.normalize(v).unwrap()),
            [0.0, 3.0, 3.0, 9.0, 9.0, 10.0, 10.0]
        );
        assert_eq!(d.advance(10.0, -1), 9.0);
        assert_eq!(d.advance(9.0, 1), 10.0);
        assert_eq!(d.advance(10.0, -10), 0.0);
        let d = SliderDomain::stepped(-5.0, 5.0, 20.0).unwrap();
        assert_eq!(d.normalize(0.0).unwrap(), 5.0);
        assert_eq!(d.advance(5.0, -1), -5.0);
    }
    #[test]
    fn domains_reject_invalid_or_unrepresentable_arithmetic() {
        for (min, max, step) in [
            (0.0, 0.0, 1.0),
            (0.0, f64::INFINITY, 1.0),
            (-f64::MAX, f64::MAX, 1.0),
            (1e20, 1e20 + 1e10, 1.0),
            (1e20, 1e20 + 1e10, 10_000.0),
            (0.0, 1.0, 1e-30),
            (0.0, 1.0, 0.0),
        ] {
            assert!(SliderDomain::stepped(min, max, step).is_err());
        }
        let d = SliderDomain::continuous(-10.0, 10.0).unwrap();
        assert!(d.normalize(f64::NAN).is_err());
        assert_eq!(d.normalize(20.0).unwrap(), 10.0);
        let d = SliderDomain::continuous(0.0, f64::MAX).unwrap();
        assert_eq!(d.advance(d.max(), 10), d.max());
        assert_eq!(d.at_fraction(1.0), d.max());
    }
}
