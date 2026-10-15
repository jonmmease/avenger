use std::sync::Arc;

use arrow::array::{new_empty_array, new_null_array, Array, ArrayRef, AsArray, StructArray};
use arrow::compute::concat;
use arrow::datatypes::DataType;
use datafusion::common::{exec_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDFImpl, Signature, Volatility,
};

use super::config::{config_at, config_struct, config_types, domain_options_at, planning_config};
use super::{external, SerializedScale};
use crate::scales::domain_summary::DomainSummary;
use crate::scales::ScaleImpl;

/// A scale's domain fitted to a summary, returning the fitted configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct FitUdf {
    scale: SerializedScale,
    name: String,
    signature: Signature,
}

impl FitUdf {
    pub(crate) fn new(scale: &dyn ScaleImpl) -> Result<Self> {
        Ok(Self::from_serialized(SerializedScale::new(scale)?))
    }

    pub(crate) fn from_serialized(scale: SerializedScale) -> Self {
        Self {
            name: function_name(scale.scale().as_ref()),
            scale,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    pub(crate) fn serialized_scale(&self) -> &SerializedScale {
        &self.scale
    }
}

/// The name of the function that fits the domain of `scale`.
pub(crate) fn function_name(scale: &dyn ScaleImpl) -> String {
    format!("{}_fit", scale.scale_type())
}

/// A fit argument as the struct array that it is.
fn structs<'a>(value: &'a ArrayRef, name: &str) -> Result<&'a StructArray> {
    match value.as_struct_opt() {
        Some(value) => Ok(value),
        None => exec_err!("a fit's {name} is a struct"),
    }
}

impl ScalarUDFImpl for FitUdf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        self.return_type(types)?;
        Ok(types.to_vec())
    }

    /// The type of the configuration fitted to the summary of no data, which is the type of every
    /// configuration that the fit returns.
    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        let [summary, config, options] = types else {
            return plan_err!(
                "{} takes a summary, a scale configuration, and domain options",
                self.name
            );
        };
        let empty = DomainSummary::from_arrow(&new_null_array(summary, 1), 0).map_err(external)?;
        let config = planning_config(config)?;
        // Domain options without values have their kind's defaults.
        let options = new_null_array(options, 1);
        let options = domain_options_at(structs(&options, "options")?, 0, &DataType::Null)?;
        let fitted = self
            .scale
            .scale()
            .fit_domain(&config, &options, &empty)
            .map_err(external)?;
        Ok(config_struct(&fitted)?.data_type().clone())
    }

    /// Each row's summary is fitted with its row's configuration and domain options.
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let types: Vec<_> = args.args.iter().map(ColumnarValue::data_type).collect();
        let output_type = self.return_type(&types)?;
        let (domain_type, _, _) = config_types(&output_type)?;
        let all_scalar = args
            .args
            .iter()
            .all(|value| matches!(value, ColumnarValue::Scalar(_)));
        let rows = if all_scalar { 1 } else { args.number_rows };
        if rows == 0 {
            return Ok(ColumnarValue::Array(new_empty_array(&output_type)));
        }
        let [summaries, configs, options] = [0, 1, 2].map(|i| args.args[i].to_array(rows));
        let (summaries, configs, options) = (summaries?, configs?, options?);
        let (configs, options) = (
            structs(&configs, "configuration")?,
            structs(&options, "options")?,
        );
        let mut fitted: Vec<ArrayRef> = Vec::with_capacity(rows);
        for row in 0..rows {
            if configs.is_null(row) {
                return exec_err!("{} has a row without a scale configuration", self.name);
            }
            let summary = DomainSummary::from_arrow(summaries.as_ref(), row).map_err(external)?;
            let config = config_at(configs, row)?;
            let options = domain_options_at(options, row, &domain_type)?;
            let config = self
                .scale
                .scale()
                .fit_domain(&config, &options, &summary)
                .map_err(external)?;
            let config = config_struct(&config)?;
            if config.data_type() != &output_type {
                return exec_err!(
                    "{} fitted a configuration of {}, not {output_type}",
                    self.name,
                    config.data_type()
                );
            }
            fitted.push(Arc::new(config));
        }
        let fitted: Vec<&dyn Array> = fitted.iter().map(|config| config.as_ref()).collect();
        let fitted = concat(&fitted)?;
        if all_scalar {
            Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                &fitted, 0,
            )?))
        } else {
            Ok(ColumnarValue::Array(fitted))
        }
    }
}
