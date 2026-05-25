//! Runtime helpers and built-in option extension traits for chart scales.

use std::{collections::HashMap, sync::Arc};

use avenger_chart_core::{
    AvengerChartError, DefaultLogicalExprNodeExt, Maybe, ScalarValueHelpers, Scale,
    ScaleDefaultDomain, ScaleDomain, ScaleRange, ScaleSpec, eval_to_scalars, params_to_datafusion,
};
use avenger_scales::scales::{ConfiguredScale, DomainKind, RangeKind, ScaleConfig, ScaleContext};
use datafusion::{
    arrow::array::{
        ArrayRef, Date32Array, Date64Array, Float32Array, Float32Builder, ListBuilder, StringArray,
    },
    logical_expr::{Expr, lit},
    prelude::SessionContext,
};
use datafusion_common::ScalarValue;
use indexmap::IndexMap;

use crate::{
    Band, Linear, Log, Ordinal, Point, Pow, Sqrt, Symlog, Time, domain_inference::DomainInferrer,
};

/// Runtime scale methods used by the built-in scale builder.
#[allow(async_fn_in_trait)]
pub trait ScaleRuntimeExt: Sized {
    async fn normalize_domain(
        self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError>;

    async fn create_configured_scale(
        &self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<ConfiguredScale, AvengerChartError>;
}

impl<S: ScaleSpec> ScaleRuntimeExt for Scale<S> {
    async fn normalize_domain(
        mut self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError> {
        if !matches!(
            self.config().range.as_ref(),
            Maybe::Set(ScaleRange::Numeric(_, _))
        ) {
            return Ok(self);
        }

        if let Maybe::Set(domain) = self.config().domain.as_ref() {
            if matches!(
                &domain.default_domain,
                ScaleDefaultDomain::Discrete(_) | ScaleDefaultDomain::NoDefault
            ) {
                return Ok(self);
            }
        } else {
            return Ok(self);
        }

        let configured = self
            .create_configured_scale(plot_area_width, plot_area_height, ctx, params)
            .await?;

        if let Ok((min, max)) = configured.numeric_interval_domain() {
            self = self.domain_interval(lit(min as f64), lit(max as f64));
        }

        Ok(self)
    }

    async fn create_configured_scale(
        &self,
        _plot_area_width: f32,
        _plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<ConfiguredScale, AvengerChartError> {
        let scale_impl = self.get_scale_impl_or_err()?;

        let domain_spec = self.config().domain.as_option().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Domain must be specified for scale before creating ConfiguredScale".to_string(),
            )
        })?;

        let domain = match &domain_spec.default_domain {
            ScaleDefaultDomain::NoDefault => {
                return Err(AvengerChartError::InternalError(
                    "Domain must be specified for scale before creating ConfiguredScale"
                        .to_string(),
                ));
            }
            ScaleDefaultDomain::Interval(start, end) => {
                let start_expr = start.to_expr(ctx)?;
                let end_expr = end.to_expr(ctx)?;
                let datafusion_params = params_to_datafusion(params);
                let scalars = eval_to_scalars(
                    vec![start_expr, end_expr],
                    Some(ctx),
                    datafusion_params.as_ref(),
                )
                .await?;
                let [start_val, end_val] = scalars.as_slice() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected two scalar values for interval domain".to_string(),
                    ));
                };

                if scale_impl.domain_kind() == DomainKind::Temporal {
                    match (start_val, end_val) {
                        (ScalarValue::Date32(Some(s)), ScalarValue::Date32(Some(e))) => {
                            Arc::new(Date32Array::from(vec![*s, *e])) as ArrayRef
                        }
                        (ScalarValue::Date64(Some(s)), ScalarValue::Date64(Some(e))) => {
                            Arc::new(Date64Array::from(vec![*s, *e])) as ArrayRef
                        }
                        _ => {
                            let start_i64 = start_val.as_f64()? as i64;
                            let end_i64 = end_val.as_f64()? as i64;
                            Arc::new(Date64Array::from(vec![start_i64, end_i64])) as ArrayRef
                        }
                    }
                } else {
                    let start_f32 = start_val.as_f32()?;
                    let end_f32 = end_val.as_f32()?;
                    Arc::new(Float32Array::from(vec![start_f32, end_f32])) as ArrayRef
                }
            }
            ScaleDefaultDomain::Discrete(values) => {
                let exprs: Vec<Expr> = values
                    .iter()
                    .map(|v| v.to_expr(ctx))
                    .collect::<Result<Vec<_>, _>>()?;
                let scalars = eval_to_scalars(exprs, Some(ctx), None).await?;

                match scale_impl.domain_kind() {
                    DomainKind::Numeric => {
                        let mut float_values = Vec::new();
                        for scalar in scalars {
                            float_values.push(scalar.as_f32()?);
                        }
                        Arc::new(Float32Array::from(float_values)) as ArrayRef
                    }
                    DomainKind::Categorical => {
                        let all_numeric = scalars.iter().all(|s| s.as_f32().is_ok());
                        if all_numeric {
                            let mut float_values = Vec::new();
                            for scalar in scalars {
                                float_values.push(scalar.as_f32()?);
                            }
                            Arc::new(Float32Array::from(float_values)) as ArrayRef
                        } else {
                            let mut string_values = Vec::new();
                            for scalar in scalars {
                                string_values.push(scalar.as_scalar_string()?);
                            }
                            Arc::new(StringArray::from(string_values)) as ArrayRef
                        }
                    }
                    DomainKind::Temporal => {
                        let mut float_values = Vec::new();
                        for scalar in scalars {
                            float_values.push(scalar.as_f32()?);
                        }
                        Arc::new(Float32Array::from(float_values)) as ArrayRef
                    }
                }
            }
            ScaleDefaultDomain::DomainExprs(_) => {
                return Err(AvengerChartError::InternalError(
                    "Domain must be resolved before creating ConfiguredScale".to_string(),
                ));
            }
        };

        let range = match self.config().range.as_ref() {
            Maybe::Set(ScaleRange::Numeric(start, end)) => {
                let start_expr = start.to_expr(ctx)?;
                let end_expr = end.to_expr(ctx)?;
                let scalars = eval_to_scalars(vec![start_expr, end_expr], Some(ctx), None).await?;
                let [start_val, end_val] = scalars.as_slice() else {
                    return Err(AvengerChartError::InternalError(
                        "Expected two scalar values for numeric range".to_string(),
                    ));
                };

                let start_f32 = start_val.as_f32()?;
                let end_f32 = end_val.as_f32()?;
                Arc::new(Float32Array::from(vec![start_f32, end_f32])) as ArrayRef
            }
            Maybe::Set(ScaleRange::Discrete(values)) => {
                let scalar_values: Vec<ScalarValue> =
                    values.iter().map(|v| v.as_scalar().clone()).collect();
                let all_numeric = scalar_values.iter().all(|v| v.as_f32().is_ok());

                if all_numeric {
                    let mut float_values = Vec::new();
                    for scalar in &scalar_values {
                        float_values.push(scalar.as_f32()?);
                    }
                    Arc::new(Float32Array::from(float_values)) as ArrayRef
                } else {
                    let mut string_values = Vec::new();
                    for scalar in &scalar_values {
                        string_values.push(scalar.as_scalar_string()?);
                    }
                    Arc::new(StringArray::from(string_values)) as ArrayRef
                }
            }
            Maybe::Set(ScaleRange::Color(colors)) => {
                let mut list_builder = ListBuilder::new(Float32Builder::new());

                for color in colors {
                    let values_builder = list_builder.values();
                    values_builder.append_value(color[0]);
                    values_builder.append_value(color[1]);
                    values_builder.append_value(color[2]);
                    values_builder.append_value(color[3]);
                    list_builder.append(true);
                }

                Arc::new(list_builder.finish()) as ArrayRef
            }
            Maybe::Unset => Arc::new(Float32Array::from(vec![0.0_f32, 1.0_f32])) as ArrayRef,
        };

        let mut scalar_options = HashMap::new();
        for (key, value_expr) in &self.config().options {
            let expr = value_expr.to_expr(ctx)?;
            let scalars = eval_to_scalars(vec![expr], Some(ctx), None).await?;
            if let Some(scalar_value) = scalars.first() {
                let scalar = scalar_value.as_scale_scalar()?;
                scalar_options.insert(key.clone(), scalar);
            }
        }

        for (key, value) in scale_impl.default_options() {
            scalar_options.entry(key).or_insert(value);
        }

        if scalar_options.contains_key("padding")
            && scale_impl.domain_kind() == DomainKind::Numeric
            && scale_impl.range_kind() == RangeKind::Continuous
            && let Some(padding_value) = scalar_options.get("padding").cloned()
        {
            scalar_options
                .entry("clip_padding_lower".to_string())
                .or_insert_with(|| padding_value.clone());
            scalar_options
                .entry("clip_padding_upper".to_string())
                .or_insert(padding_value);
            scalar_options.remove("padding");
        }

        Ok(ConfiguredScale {
            scale_impl,
            config: ScaleConfig {
                domain,
                range,
                options: scalar_options,
                context: ScaleContext::default(),
            },
        })
    }
}

/// Optional runtime domain inference helper retained for scale-builder internals.
#[allow(async_fn_in_trait)]
pub trait ScaleDomainInferenceExt: Sized {
    async fn infer_domain_from_data(
        self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError>;
}

impl<S: ScaleSpec> ScaleDomainInferenceExt for Scale<S> {
    async fn infer_domain_from_data(
        mut self,
        _plot_area_width: f32,
        _plot_area_height: f32,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<Self, AvengerChartError> {
        let scale_impl = self.get_scale_impl_or_err()?;
        let range_hint = match self.config().range.as_ref() {
            Maybe::Set(ScaleRange::Numeric(start, end)) => {
                let start_expr = start.to_expr(ctx)?;
                let end_expr = end.to_expr(ctx)?;
                let datafusion_params = params_to_datafusion(params);
                let scalars = eval_to_scalars(
                    vec![start_expr, end_expr],
                    Some(ctx),
                    datafusion_params.as_ref(),
                )
                .await?;
                if scalars.len() == 2 {
                    Some((scalars[0].as_f64()?, scalars[1].as_f64()?))
                } else {
                    None
                }
            }
            _ => None,
        };

        let current_domain = self
            .config()
            .domain
            .clone()
            .unwrap_or_else(|| infer_default_domain(&scale_impl));
        let inferred_domain =
            DomainInferrer::infer(&scale_impl, current_domain, range_hint, ctx, params).await?;
        self.config_mut().domain = Maybe::Set(inferred_domain);
        Ok(self)
    }
}

fn infer_default_domain(scale_impl: &Arc<dyn avenger_scales::scales::ScaleImpl>) -> ScaleDomain {
    match (scale_impl.domain_kind(), scale_impl.range_kind()) {
        (DomainKind::Categorical, _) => ScaleDomain::new_discrete(vec![]),
        (DomainKind::Numeric | DomainKind::Temporal, RangeKind::Continuous) => {
            ScaleDomain::new_interval(lit(0.0), lit(1.0))
        }
        (DomainKind::Numeric | DomainKind::Temporal, RangeKind::Discrete) => {
            ScaleDomain::new_discrete(vec![])
        }
    }
}

pub trait LinearScaleExt {
    fn zero(self, value: bool) -> Self;
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
    fn padding(self, value: f32) -> Self;
}

impl LinearScaleExt for Scale<Linear> {
    fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
    fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }
}

pub trait LogScaleExt {
    fn base(self, value: f32) -> Self;
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
}

impl LogScaleExt for Scale<Log> {
    fn base(self, value: f32) -> Self {
        self._option("base", lit(value))
    }
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

pub trait PowScaleExt {
    fn exponent(self, value: f32) -> Self;
    fn zero(self, value: bool) -> Self;
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
}

impl PowScaleExt for Scale<Pow> {
    fn exponent(self, value: f32) -> Self {
        self._option("exponent", lit(value))
    }
    fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

pub trait SqrtScaleExt {
    fn zero(self, value: bool) -> Self;
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
}

impl SqrtScaleExt for Scale<Sqrt> {
    fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

pub trait SymlogScaleExt {
    fn constant(self, value: f32) -> Self;
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
}

impl SymlogScaleExt for Scale<Symlog> {
    fn constant(self, value: f32) -> Self {
        self._option("constant", lit(value))
    }
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }
    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

pub trait BandScaleExt {
    fn padding_inner(self, value: f32) -> Self;
    fn padding_outer(self, value: f32) -> Self;
    fn align(self, value: f32) -> Self;
    fn round(self, value: bool) -> Self;
}

impl BandScaleExt for Scale<Band> {
    fn padding_inner(self, value: f32) -> Self {
        self._option("padding_inner", lit(value))
    }
    fn padding_outer(self, value: f32) -> Self {
        self._option("padding_outer", lit(value))
    }
    fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }
    fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

pub trait PointScaleExt {
    fn padding(self, value: f32) -> Self;
    fn align(self, value: f32) -> Self;
    fn round(self, value: bool) -> Self;
}

impl PointScaleExt for Scale<Point> {
    fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }
    fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }
    fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

pub trait OrdinalScaleExt {
    fn unknown(self, value: impl Into<Expr>) -> Self;
}

impl OrdinalScaleExt for Scale<Ordinal> {
    fn unknown(self, value: impl Into<Expr>) -> Self {
        self._option("unknown", value.into())
    }
}

pub trait TimeScaleExt {
    fn nice(self, value: bool) -> Self;
    fn clamp(self, value: bool) -> Self;
}

impl TimeScaleExt for Scale<Time> {
    fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}
