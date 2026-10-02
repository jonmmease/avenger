use super::time::{DateTimeFormatAdapter, TimeMultiFormatSpec};
use avenger_format::{
    NumberFormatBinding, NumberFormatError, NumberFormatProvider, PreparedNumberFormatter,
};
use avenger_format_config::{DateTimeFormatConfig, NumberFormatConfig};
use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberFormatProvider, D3NumberPrecision};
use std::{fmt, sync::Arc};

/// Facts available after selecting numeric labels.
#[derive(Debug, Clone, Copy)]
pub enum NumberLabelContext {
    /// Continuous data labels without a shared tick step.
    Continuous,
    /// Discrete numeric categories.
    Categorical,
    /// Tick spacing and reference magnitude used for shared precision and SI units.
    Ticks { step: f64, reference_value: f64 },
}

type PrepareNumber = dyn Fn(
        Option<&str>,
        NumberLabelContext,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError>
    + Send
    + Sync;

/// Scale policy for selecting a provider's default pattern and numeric precision.
#[derive(Clone)]
pub struct NumberFormatAdapter {
    binding: NumberFormatBinding,
    prepare: Arc<PrepareNumber>,
}

impl fmt::Debug for NumberFormatAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NumberFormatAdapter")
            .field(&self.binding)
            .finish()
    }
}

impl NumberFormatAdapter {
    /// Pair label settings with a scale-specific pattern and precision policy.
    pub fn new(
        binding: NumberFormatBinding,
        prepare: impl Fn(
                Option<&str>,
                NumberLabelContext,
            ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError>
            + Send
            + Sync
            + 'static,
    ) -> Self {
        Self {
            binding,
            prepare: Arc::new(prepare),
        }
    }

    /// Apply D3 defaults while deriving tick precision in a temporary configuration.
    pub fn d3(config: D3NumberFormatConfig) -> Self {
        Self::new(
            NumberFormatBinding::new(D3NumberFormatProvider, config.clone()),
            move |pattern, context| {
                let (pattern, precision) = match context {
                    NumberLabelContext::Continuous => (
                        pattern.filter(|p| !p.is_empty()).unwrap_or(","),
                        D3NumberPrecision::Automatic,
                    ),
                    NumberLabelContext::Categorical => (
                        pattern.filter(|p| !p.is_empty()).unwrap_or("c"),
                        D3NumberPrecision::FromSpecifier,
                    ),
                    NumberLabelContext::Ticks {
                        step,
                        reference_value,
                    } => (
                        pattern.unwrap_or(",f"),
                        D3NumberPrecision::Step {
                            step: if step.is_finite() { step } else { 0.0 },
                            reference_value: if reference_value.is_finite() {
                                reference_value
                            } else {
                                0.0
                            },
                        },
                    ),
                };
                D3NumberFormatProvider.prepare(&config.clone().with_precision(precision), pattern)
            },
        )
    }

    /// Construct scale policy for a saved built-in selection.
    pub fn from_config(config: &NumberFormatConfig) -> Self {
        match config {
            NumberFormatConfig::D3(config) => Self::d3(config.clone()),
        }
    }

    /// Settings used to prepare explicit patterns in text labels.
    pub fn binding(&self) -> &NumberFormatBinding {
        &self.binding
    }

    /// Prepare once for the selected tick set or data-label batch.
    pub fn prepare(
        &self,
        pattern: Option<&str>,
        context: NumberLabelContext,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        (self.prepare)(pattern, context)
    }
}

/// Explicit preparation policies shared by scales and axes.
#[derive(Debug, Clone, Default)]
pub struct ScaleFormatting {
    pub number: Option<NumberFormatAdapter>,
    pub datetime: Option<DateTimeFormatAdapter>,
}

impl ScaleFormatting {
    /// Configure D3 number and datetime policies from explicit settings.
    pub fn d3(
        number: D3NumberFormatConfig,
        datetime: avenger_format_config::D3DateTimeFormatConfig,
    ) -> Self {
        Self {
            number: Some(NumberFormatAdapter::d3(number)),
            datetime: Some(DateTimeFormatAdapter::d3(datetime, Default::default())),
        }
    }

    /// Apply the same bindings to text markup without changing font or layout settings.
    pub fn configure_text_engine(
        &self,
        mut engine: avenger_text::TextEngine,
    ) -> avenger_text::TextEngine {
        if let Some(number) = &self.number {
            engine = engine.with_number_formatting(number.binding().clone());
        }
        if let Some(datetime) = &self.datetime {
            engine = engine.with_datetime_formatting(datetime.binding().clone());
        }
        engine
    }

    /// Resolve built-in policies from typed settings without selecting an implicit provider.
    pub fn from_configs(
        number: Option<&NumberFormatConfig>,
        datetime: Option<&DateTimeFormatConfig>,
    ) -> Self {
        Self {
            number: number.map(NumberFormatAdapter::from_config),
            datetime: datetime.map(|config| {
                DateTimeFormatAdapter::from_config(config, TimeMultiFormatSpec::default())
            }),
        }
    }
}
