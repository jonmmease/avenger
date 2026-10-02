use std::{collections::HashMap, fmt::Debug, sync::Arc};

use avenger_scales::{
    scalar::Scalar,
    scales::{
        band::BandScale, linear::LinearScale, log::LogScale, ordinal::OrdinalScale,
        point::PointScale, pow::PowScale, quantile::QuantileScale, quantize::QuantizeScale,
        symlog::SymlogScale, threshold::ThresholdScale, time::TimeScale, ScaleConfig, ScaleImpl,
    },
};
use datafusion::{
    arrow::datatypes::DataType,
    common::{plan_err, Result},
};
use serde::{Deserialize, Serialize};

/// A serializable, immutable scale implementation descriptor.
///
/// Implementations must describe every setting that affects results in their
/// serialized fields. Factories and scale kernels must be deterministic and
/// independent of mutable external state. The UDF has immutable volatility.
/// Use a versioned typetag name when changing a custom descriptor's semantics.
#[typetag::serde(tag = "type", content = "config")]
pub trait ScaleSpec: Debug + Send + Sync {
    fn create_impl(&self) -> Result<Arc<dyn ScaleImpl>>;

    /// The Arrow type returned by the underlying scale kernel.
    fn output_type(&self, range_type: &DataType) -> Result<DataType>;

    /// The value type accepted by the kernel. DataFusion inserts this cast.
    fn input_type(&self, _domain_type: &DataType, input_type: &DataType) -> Result<DataType> {
        Ok(input_type.clone())
    }

    fn default_options(&self) -> HashMap<String, Scalar> {
        HashMap::new()
    }

    /// Validate shape constraints before calling the underlying kernel.
    fn validate_config(&self, _config: &ScaleConfig) -> Result<()> {
        Ok(())
    }
}

/// Scale implementations available in the current `avenger-scales` crate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinScale {
    Linear,
    Log,
    Pow,
    Sqrt,
    Symlog,
    Time,
    Band,
    Point,
    Ordinal,
    Threshold,
    Quantile,
    Quantize,
}

#[typetag::serde(name = "avenger_builtin_v2")]
impl ScaleSpec for BuiltinScale {
    fn create_impl(&self) -> Result<Arc<dyn ScaleImpl>> {
        Ok(match self {
            Self::Linear => Arc::new(LinearScale),
            Self::Log => Arc::new(LogScale),
            Self::Pow | Self::Sqrt => Arc::new(PowScale),
            Self::Symlog => Arc::new(SymlogScale),
            Self::Time => Arc::new(TimeScale),
            Self::Band => Arc::new(BandScale),
            Self::Point => Arc::new(PointScale),
            Self::Ordinal => Arc::new(OrdinalScale),
            Self::Threshold => Arc::new(ThresholdScale),
            Self::Quantile => Arc::new(QuantileScale),
            Self::Quantize => Arc::new(QuantizeScale),
        })
    }

    fn output_type(&self, range_type: &DataType) -> Result<DataType> {
        match self {
            Self::Ordinal | Self::Threshold | Self::Quantile | Self::Quantize => Ok(
                DataType::Dictionary(Box::new(DataType::Int16), Box::new(range_type.clone())),
            ),
            Self::Linear | Self::Log | Self::Pow | Self::Sqrt
                if matches!(range_type, DataType::Utf8 | DataType::List(_)) =>
            {
                Ok(DataType::new_list(DataType::Float32, true))
            }
            _ if range_type.is_numeric() => Ok(DataType::Float32),
            _ => plan_err!("{self:?} requires a numeric range with the current scale kernel"),
        }
    }

    fn input_type(&self, domain_type: &DataType, input_type: &DataType) -> Result<DataType> {
        match self {
            Self::Band | Self::Point | Self::Ordinal => {
                // The current categorical kernels extract these scalar types.
                if !matches!(
                    domain_type,
                    DataType::Boolean | DataType::Int32 | DataType::Float32 | DataType::Utf8
                ) {
                    return plan_err!("Unsupported categorical scale domain type: {domain_type}");
                }
                if input_type != domain_type && input_type != &DataType::Null {
                    return plan_err!("Categorical scale values must match domain type {domain_type}, got {input_type}");
                }
                Ok(domain_type.clone())
            }
            Self::Time => {
                if !matches!(
                    domain_type,
                    DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _)
                ) {
                    return plan_err!("Time scale requires a date or timestamp domain");
                }
                if input_type != domain_type && input_type != &DataType::Null {
                    return plan_err!("Time scale values must match domain type {domain_type}");
                }
                Ok(domain_type.clone())
            }
            _ => {
                if !domain_type.is_numeric()
                    || !(input_type.is_numeric() || input_type == &DataType::Null)
                {
                    return plan_err!("{self:?} requires numeric domain and values");
                }
                Ok(DataType::Float32)
            }
        }
    }

    fn default_options(&self) -> HashMap<String, Scalar> {
        if self == &Self::Sqrt {
            HashMap::from([("exponent".into(), Scalar::from_f32(0.5))])
        } else {
            HashMap::new()
        }
    }

    fn validate_config(&self, config: &ScaleConfig) -> Result<()> {
        if config.domain.null_count() != 0 {
            return plan_err!("Scale domain elements must not be null");
        }
        let discrete_range = matches!(
            self,
            Self::Ordinal | Self::Threshold | Self::Quantile | Self::Quantize
        );
        if !discrete_range && config.range.null_count() != 0 {
            return plan_err!("Continuous scale range elements must not be null");
        }
        if config.range.is_empty() {
            return plan_err!("Scale range must not be empty");
        }
        match self {
            Self::Band | Self::Point | Self::Ordinal | Self::Quantile => {
                if config.domain.is_empty() {
                    return plan_err!("{self:?} domain must not be empty");
                }
            }
            Self::Threshold => {}
            _ if config.domain.len() != 2 => {
                return plan_err!("{self:?} domain must have two elements")
            }
            _ => {}
        }
        Ok(())
    }
}
