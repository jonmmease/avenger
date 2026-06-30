use crate::{
    error::FormatError,
    format::{format_number, NumberFormatContext, NumberFormatOverrides},
    typesetting::FormattedNumber,
};

#[derive(Debug, Clone)]
pub struct PreparedNumberTickFormat {
    spec: Option<String>,
    overrides: NumberFormatOverrides,
}

impl PreparedNumberTickFormat {
    pub fn format(
        &self,
        value: f64,
        context: NumberFormatContext<'_>,
    ) -> Result<FormattedNumber, FormatError> {
        format_number(value, self.spec.as_deref(), self.overrides.clone(), context)
    }
}

pub fn prepare_number_tick_format(
    _values: &[f64],
    spec: Option<&str>,
    overrides: NumberFormatOverrides,
    context: NumberFormatContext<'_>,
) -> Result<PreparedNumberTickFormat, FormatError> {
    if let Some(spec) = spec {
        let _ = format_number(0.0, Some(spec), overrides.clone(), context)?;
    }
    Ok(PreparedNumberTickFormat {
        spec: spec.map(ToOwned::to_owned),
        overrides,
    })
}
