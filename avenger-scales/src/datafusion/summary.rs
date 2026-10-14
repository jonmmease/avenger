use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

use arrow::array::{new_empty_array, Array, ArrayRef, AsArray};
use arrow::datatypes::{DataType, Float64Type};
use datafusion::common::{exec_err, plan_err, Result, ScalarValue};
use datafusion::logical_expr::function::AccumulatorArgs;
use datafusion::logical_expr::{Accumulator, AggregateUDFImpl, Signature, Volatility};
use datafusion::physical_expr::expressions::Literal;
use datafusion::physical_expr::PhysicalExpr;

use super::config::options_at;
use super::{external, PlannedScale};
use crate::scalar::Scalar;
use crate::scales::domain_summary::{DomainSummary, Extents};
use crate::scales::{DomainKind, ScaleConfig, ScaleContext, ScaleImpl};

/// An aggregate that summarizes values for fitting a scale's domain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct SummaryUdaf {
    scale: PlannedScale,
    name: String,
    signature: Signature,
}

impl SummaryUdaf {
    pub(crate) fn new(scale: &dyn ScaleImpl) -> Result<Self> {
        Ok(Self::from_planned(PlannedScale::new(scale)?))
    }

    pub(crate) fn from_planned(scale: PlannedScale) -> Self {
        Self {
            name: function_name(scale.scale().as_ref()),
            scale,
            signature: Signature::user_defined(Volatility::Immutable),
        }
    }

    pub(crate) fn planned_scale(&self) -> &PlannedScale {
        &self.scale
    }

    /// The scale's summary of `values` with `options`.
    fn summarize(
        &self,
        options: &HashMap<String, Scalar>,
        values: &ArrayRef,
        extents: Extents,
    ) -> Result<DomainSummary> {
        let config = ScaleConfig {
            domain: new_empty_array(&DataType::Null),
            range: new_empty_array(&DataType::Null),
            options: options.clone(),
            context: ScaleContext::default(),
        };
        self.scale
            .scale()
            .summarize(&config, values, extents)
            .map_err(external)
    }
}

/// The name of the aggregate that summarizes values for `scale`.
pub(crate) fn function_name(scale: &dyn ScaleImpl) -> String {
    format!("{}_summary", scale.scale_type())
}

impl AggregateUDFImpl for SummaryUdaf {
    fn name(&self) -> &str {
        &self.name
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn coerce_types(&self, types: &[DataType]) -> Result<Vec<DataType>> {
        let (values, options, extents) = match types {
            [values, options] => (values, options, &[][..]),
            [values, options, extents @ ..] if extents.len() == 2 => (values, options, extents),
            _ => {
                return plan_err!(
                    "{} takes values, options, and optionally lower and upper extents",
                    self.name
                )
            }
        };
        if !matches!(options, DataType::Struct(_)) {
            return plan_err!("a scale's options are a struct, not {options}");
        }
        let mut coerced = vec![match self.scale.scale().domain_kind() {
            DomainKind::Numeric => numeric(values, "values")?,
            _ => values.clone(),
        }];
        coerced.push(options.clone());
        for extent in extents {
            coerced.push(numeric(extent, "extents")?);
        }
        Ok(coerced)
    }

    /// The type of the summary of no values, which is the type of every summary of these values.
    fn return_type(&self, types: &[DataType]) -> Result<DataType> {
        let Some(values) = types.first() else {
            return plan_err!("{} takes values and options", self.name);
        };
        let empty = self.summarize(&HashMap::new(), &new_empty_array(values), Extents::None)?;
        Ok(empty.to_arrow().data_type().clone())
    }

    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let Some(ScalarValue::Struct(options)) = literal(&args.exprs[1]) else {
            return plan_err!("{}'s options are a struct literal", self.name);
        };
        let extents = match args.exprs.get(2..) {
            Some([lower, upper]) => match (literal(lower), literal(upper)) {
                (Some(lower), Some(upper)) => ExtentArgs::Constant {
                    lower: extent_literal(lower)?,
                    upper: extent_literal(upper)?,
                },
                _ => ExtentArgs::PerRow,
            },
            _ => ExtentArgs::None,
        };
        let options = options_at(options, 0)?;
        let value_type = args.expr_fields[0].data_type();
        let summary = self.summarize(&options, &new_empty_array(value_type), Extents::None)?;
        Ok(Box::new(SummaryAccumulator {
            udaf: self.clone(),
            options,
            extents,
            summary,
        }))
    }
}

fn numeric(data_type: &DataType, name: &str) -> Result<DataType> {
    if data_type.is_numeric() || data_type == &DataType::Null {
        Ok(DataType::Float64)
    } else {
        plan_err!("summary {name} are numeric, not {data_type}")
    }
}

/// The value of an argument that is a literal.
fn literal(expr: &Arc<dyn PhysicalExpr>) -> Option<&ScalarValue> {
    (expr.as_ref() as &dyn Any)
        .downcast_ref::<Literal>()
        .map(Literal::value)
}

fn extent_literal(value: &ScalarValue) -> Result<f64> {
    match value.cast_to(&DataType::Float64)? {
        ScalarValue::Float64(Some(extent)) => Ok(extent),
        _ => plan_err!("a literal extent is a number"),
    }
}

/// How the extents arrive.
#[derive(Debug)]
enum ExtentArgs {
    None,
    /// Literal extents, the same for every value.
    Constant {
        lower: f64,
        upper: f64,
    },
    /// Extent arguments that vary between values.
    PerRow,
}

#[derive(Debug)]
struct SummaryAccumulator {
    udaf: SummaryUdaf,
    options: HashMap<String, Scalar>,
    extents: ExtentArgs,
    summary: DomainSummary,
}

impl SummaryAccumulator {
    fn merge(&mut self, summary: &DomainSummary) -> Result<()> {
        self.summary = self.summary.merge(summary).map_err(external)?;
        Ok(())
    }
}

impl Accumulator for SummaryAccumulator {
    fn update_batch(&mut self, args: &[ArrayRef]) -> Result<()> {
        let values = &args[0];
        // A missing value is skipped with its extents, which are then zero.
        let per_row = |extents: &ArrayRef| -> Result<Vec<f64>> {
            let extents = extents.as_primitive::<Float64Type>();
            if (0..values.len()).any(|i| values.is_valid(i) && extents.is_null(i)) {
                return exec_err!("{} has a value without an extent", self.udaf.name);
            }
            Ok(extents.iter().map(|extent| extent.unwrap_or(0.0)).collect())
        };
        let (lower, upper);
        let extents = match self.extents {
            ExtentArgs::None => Extents::None,
            ExtentArgs::Constant { lower, upper } => Extents::Constant { lower, upper },
            ExtentArgs::PerRow => {
                (lower, upper) = (per_row(&args[2])?, per_row(&args[3])?);
                Extents::PerRow {
                    lower: &lower,
                    upper: &upper,
                }
            }
        };
        let summary = self.udaf.summarize(&self.options, values, extents)?;
        self.merge(&summary)
    }

    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        for row in 0..states[0].len() {
            let summary = DomainSummary::from_arrow(states[0].as_ref(), row).map_err(external)?;
            self.merge(&summary)?;
        }
        Ok(())
    }

    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        Ok(vec![self.evaluate()?])
    }

    fn evaluate(&mut self) -> Result<ScalarValue> {
        Ok(ScalarValue::Struct(Arc::new(self.summary.to_arrow())))
    }

    fn size(&self) -> usize {
        let f64s = |count: usize| count * size_of::<f64>();
        size_of_val(self)
            + match &self.summary {
                DomainSummary::Interval(summary) => {
                    let (lower, upper) = summary.chains();
                    f64s(2 * (lower.len() + upper.len()))
                }
                DomainSummary::Categories(summary) => {
                    let (keys, lower, upper) = summary.parts();
                    keys.get_array_memory_size() + f64s(lower.len() + upper.len())
                }
                DomainSummary::Sample(summary) => f64s(summary.values().len()),
            }
    }
}
