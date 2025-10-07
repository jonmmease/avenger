//! Type-safe scale system with compile-time method resolution

use crate::error::AvengerChartError;
use crate::maybe::Maybe;
use crate::scales::domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
use crate::scales::domain_inference::DomainInferrer;
use crate::scales::range::ScaleRange;
use crate::scales::spec::*;
use crate::serialization::{LogicalExprNodeExt, SerializableExpr};
use crate::utils::{ScalarValueHelpers, eval_to_scalars, scalar_to_scalar_value};
use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{Expr, lit};
use datafusion_common::ScalarValue;
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use palette::Srgba;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;

/// Helper to infer default domain from scale implementation
fn infer_default_domain(scale_impl: &Arc<dyn ScaleImpl>) -> ScaleDomain {
    // Use DomainKind and RangeKind to determine default domain structure
    match (scale_impl.domain_kind(), scale_impl.range_kind()) {
        // Categorical domains are always discrete
        (DomainKind::Categorical, _) => ScaleDomain::new_discrete(vec![]),
        // Numeric/Temporal domains with continuous ranges use intervals
        (DomainKind::Numeric | DomainKind::Temporal, RangeKind::Continuous) => {
            ScaleDomain::new_interval(lit(0.0), lit(1.0))
        }
        // Numeric/Temporal domains with discrete ranges are discrete
        (DomainKind::Numeric | DomainKind::Temporal, RangeKind::Discrete) => {
            ScaleDomain::new_discrete(vec![])
        }
    }
}

/// Type-safe scale with compile-time method resolution
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct Scale<S: ScaleSpec = Auto> {
    pub(crate) scale_spec: Maybe<Box<dyn ScaleSpec>>,
    pub(crate) domain: Maybe<ScaleDomain>,
    pub(crate) range: Maybe<ScaleRange>,
    #[serde_as(as = "HashMap<_, FromInto<SerializableExpr>>")]
    options: HashMap<String, LogicalExprNode>,
    #[serde(skip)]
    pub(crate) _phantom: PhantomData<S>,
}

// ===== Generic methods available on ALL scales =====
impl<S: ScaleSpec + Default> Default for Scale<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: ScaleSpec> Clone for Scale<S> {
    fn clone(&self) -> Self {
        // Clone the scale_spec using clone_box
        let scale_spec = self.scale_spec.as_option().map(|spec| spec.clone_box());

        Scale {
            scale_spec: match scale_spec {
                Some(spec) => Maybe::Set(spec),
                None => Maybe::Unset,
            },
            domain: self.domain.clone(),
            range: self.range.clone(),
            options: self.options.clone(),
            _phantom: PhantomData,
        }
    }
}

impl<S: ScaleSpec> Scale<S> {
    /// Create a new scale for user configuration (all fields unset except typed scale_impl)
    /// This is called when users configure scales via closures
    pub fn new() -> Self
    where
        S: Default,
    {
        // Create a default instance of the scale spec
        let spec = S::default();

        // Get default options for the scale type
        // These are necessary for certain scales to function correctly (e.g., Sqrt needs exponent=0.5)
        let mut options = HashMap::new();
        for (key, value) in spec.default_options() {
            let scalar_value = scalar_to_scalar_value(&value);
            let expr = lit(scalar_value);
            options.insert(
                key,
                LogicalExprNode::from_expr(expr).expect("Failed to serialize option expr"),
            );
        }

        Self {
            // For typed scales (e.g., Scale<Linear>), set the spec
            // For Scale<Auto>, leave unset to be determined later
            scale_spec: if spec.name() != "auto" {
                Maybe::Set(Box::new(spec) as Box<dyn ScaleSpec>)
            } else {
                Maybe::Unset
            },
            domain: Maybe::Unset,
            range: Maybe::Unset,
            options,
            _phantom: PhantomData,
        }
    }

    /// Update this scale with properties from another scale
    /// Properties that are Set in `other` override properties in `self`
    pub fn update(mut self, other: Scale<Auto>) -> Self {
        // Check if scale type is changing
        let scale_type_changed = other.scale_spec.is_set();

        // Update scale_spec only if explicitly set in other
        if scale_type_changed {
            self.scale_spec = other.scale_spec;
        }

        // Update domain if set
        if other.domain.is_set() {
            self.domain = other.domain;
        }

        // Update range if set
        if other.range.is_set() {
            self.range = other.range;
        }

        // Update options - all options in other override those in self
        // (presence in HashMap means it was explicitly set)
        for (key, value) in other.options {
            self.options.insert(key, value);
        }

        // If scale type changed, filter options to only keep supported ones
        if scale_type_changed {
            if let Some(scale_spec) = self.scale_spec.as_option() {
                let scale_impl = scale_spec.create_impl();
                // Get supported options for the new scale type
                let option_definitions = scale_impl.option_definitions();
                let supported_options: std::collections::HashSet<&str> = option_definitions
                    .iter()
                    .map(|def| def.name.as_str())
                    .collect();

                // Filter options to only keep supported ones
                self.options
                    .retain(|key, _| supported_options.contains(key.as_str()));
            }
        }

        self
    }

    /// Set the domain
    pub fn domain<D: Into<ScaleDomain>>(mut self, domain: D) -> Self {
        self.domain = Maybe::Set(domain.into());
        self
    }

    /// Set the domain as an interval
    pub fn domain_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.domain = Maybe::Set(ScaleDomain::new_interval(min.into(), max.into()));
        self
    }

    /// Set the domain as discrete values
    pub fn domain_discrete(mut self, values: Vec<impl Into<Expr>>) -> Self {
        self.domain = Maybe::Set(ScaleDomain::new_discrete(
            values.into_iter().map(|v| v.into()).collect(),
        ));
        self
    }

    /// Set domain from data field
    pub fn domain_data(mut self, dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        use crate::serialization::LogicalPlanNodeExt;
        let mut domain = self
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        let plan = dataframe.logical_plan().clone();
        domain.default_domain = ScaleDefaultDomain::DomainExprs(vec![DomainExpr {
            dataframe: Arc::new(
                LogicalPlanNode::from_logical_plan(&plan)
                    .expect("Failed to serialize logical plan"),
            ),
            expr: LogicalExprNode::from_expr(expr).expect("Failed to serialize expr"),
            radius: None,
        }]);
        self.domain = Maybe::Set(domain);
        self
    }

    /// Set domain from data fields
    pub fn domain_data_fields(mut self, fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
        use crate::serialization::LogicalPlanNodeExt;
        let exprs = fields
            .into_iter()
            .map(|(df, expr)| {
                let plan = df.logical_plan().clone();
                DomainExpr {
                    dataframe: Arc::new(
                        LogicalPlanNode::from_logical_plan(&plan)
                            .expect("Failed to serialize logical plan"),
                    ),
                    expr: LogicalExprNode::from_expr(expr).expect("Failed to serialize expr"),
                    radius: None,
                }
            })
            .collect();
        let mut domain = self
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.domain = Maybe::Set(domain);
        self
    }

    /// Set domain from data fields with radius expressions (accepts pre-serialized LogicalPlanNode)
    ///
    /// This method accepts pre-serialized LogicalPlanNodes to avoid repeated serialization
    /// which can cause hangs due to DataFusion Issue #2659 (circular Arc references).
    pub fn domain_data_fields_with_radius_preserialized(
        mut self,
        fields: Vec<(
            Arc<LogicalPlanNode>,
            Expr,
            Option<crate::marks::RadiusExpression>,
        )>,
    ) -> Self {
        let exprs = fields
            .into_iter()
            .map(|(serialized_plan, expr, radius)| {
                let serialized_expr =
                    LogicalExprNode::from_expr(expr).expect("Failed to serialize expr");

                DomainExpr {
                    dataframe: serialized_plan,
                    expr: serialized_expr,
                    radius,
                }
            })
            .collect();
        let mut domain = self
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.domain = Maybe::Set(domain);
        self
    }

    /// Set domain from data fields (accepts pre-serialized LogicalPlanNode, no radius)
    ///
    /// This method accepts pre-serialized LogicalPlanNodes to avoid repeated serialization
    /// which can cause hangs due to DataFusion Issue #2659 (circular Arc references).
    pub fn domain_data_fields_preserialized(
        mut self,
        fields: Vec<(Arc<LogicalPlanNode>, Expr)>,
    ) -> Self {
        let exprs = fields
            .into_iter()
            .map(|(serialized_plan, expr)| {
                let serialized_expr =
                    LogicalExprNode::from_expr(expr).expect("Failed to serialize expr");

                DomainExpr {
                    dataframe: serialized_plan,
                    expr: serialized_expr,
                    radius: None,
                }
            })
            .collect();
        let mut domain = self
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.domain = Maybe::Set(domain);
        self
    }

    /// Set the range
    pub fn range(mut self, range: ScaleRange) -> Self {
        self.range = Maybe::Set(range);
        self
    }

    /// Set the range as an interval
    pub fn range_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.range = Maybe::Set(ScaleRange::new_interval(min.into(), max.into()));
        self
    }

    /// Set the range as discrete values from ScalarValues
    pub fn range_discrete(mut self, values: Vec<impl Into<ScalarValue>>) -> Self {
        self.range = Maybe::Set(ScaleRange::new_discrete(
            values.into_iter().map(|v| v.into()).collect(),
        ));
        self
    }

    /// Set the range as colors
    pub fn range_colors(mut self, colors: Vec<Srgba>) -> Self {
        self.range = Maybe::Set(ScaleRange::new_color(colors));
        self
    }

    pub fn get_scale_type(&self) -> Option<&str> {
        self.scale_spec.as_option().map(|spec| spec.name())
    }

    /// Internal method for setting options
    /// This is intentionally undocumented and prefixed with _ to discourage direct use.
    /// External crates implementing custom scales can use this for their typed methods.
    /// Regular users should use the typed methods or Scale<Auto>::option()
    #[doc(hidden)]
    pub fn _option(mut self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        let expr = value.into();
        self.options.insert(
            key.into(),
            LogicalExprNode::from_expr(expr).expect("Failed to serialize option expr"),
        );
        self
    }

    /// Convert to a different scale type
    pub fn into_type<T: ScaleSpec + Default>(self) -> Scale<T> {
        let spec = T::default();

        // For Auto, preserve the existing scale spec
        // For other types, create the appropriate spec
        let scale_spec = if spec.name() == "auto" {
            // Preserve the existing scale spec for Auto
            self.scale_spec
        } else {
            // Set new spec for specific types
            Maybe::Set(Box::new(T::default()) as Box<dyn ScaleSpec>)
        };

        // When changing scale types, we need to add the new type's default options
        // but preserve any existing options that override them
        let mut options = self.options;

        // Add scale type defaults for options not already set
        if spec.name() != "auto" {
            for (key, value) in spec.default_options() {
                // Only add if not already present
                if !options.contains_key(&key) {
                    let scalar_value = scalar_to_scalar_value(&value);
                    let expr = lit(scalar_value);
                    options.insert(
                        key,
                        LogicalExprNode::from_expr(expr).expect("Failed to serialize option expr"),
                    );
                }
            }
        }

        Scale {
            scale_spec,
            domain: self.domain,
            range: self.range,
            options,
            _phantom: PhantomData,
        }
    }

    /// Convert to Auto type for storage
    pub fn into_auto(self) -> Scale<Auto> {
        Scale {
            scale_spec: self.scale_spec,
            domain: self.domain,
            range: self.range,
            options: self.options,
            _phantom: PhantomData,
        }
    }

    // ===== Getters =====

    pub fn get_scale_impl(&self) -> Option<Arc<dyn ScaleImpl>> {
        self.scale_spec.as_option().map(|spec| spec.create_impl())
    }

    pub fn get_scale_impl_or_err(&self) -> Result<Arc<dyn ScaleImpl>, AvengerChartError> {
        self.get_scale_impl().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Scale specification not set - this is a bug".to_string(),
            )
        })
    }

    pub fn get_domain(&self) -> Option<&ScaleDomain> {
        self.domain.as_option()
    }

    pub fn get_range(&self) -> Option<&ScaleRange> {
        self.range.as_option()
    }

    pub fn get_options(&self) -> &HashMap<String, LogicalExprNode> {
        &self.options
    }

    /// Get the domain kind for this scale instance
    pub fn domain_kind(&self) -> Option<DomainKind> {
        self.scale_spec.as_option().map(|spec| spec.domain_kind())
    }

    /// Get the range kind for this scale instance
    pub fn range_kind(&self) -> Option<RangeKind> {
        self.scale_spec.as_option().map(|spec| spec.range_kind())
    }

    /// Check if this scale has bands (true for band scales, false for point scales)
    pub fn has_bands(&self) -> bool {
        self.scale_spec
            .as_option()
            .map(|spec| spec.name() == "band")
            .unwrap_or(false)
    }

    /// Get the scale type name
    pub fn scale_type_name(&self) -> Option<&str> {
        self.scale_spec.as_option().map(|spec| spec.name())
    }

    // ===== Domain inference and configuration =====

    /// Infer domain from data fields
    pub async fn infer_domain_from_data(
        mut self,
        _plot_area_width: f32,
        _plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, datafusion_common::ScalarValue>,
    ) -> Result<Self, AvengerChartError> {
        // Get scale implementation (required for inference)
        let scale_impl = self.get_scale_impl_or_err()?;

        // Calculate range hint for radius-aware padding
        let range_hint = match self.range.as_ref() {
            Maybe::Set(ScaleRange::Numeric(start, end)) => {
                let start_expr = start.to_expr(ctx)?;
                let end_expr = end.to_expr(ctx)?;
                let datafusion_params = crate::utils::params_to_datafusion(params);
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

        // Infer domain using DomainInferrer - unwrap the Maybe<ScaleDomain> or use a default
        let current_domain = self.domain.unwrap_or(infer_default_domain(&scale_impl));
        let inferred_domain =
            DomainInferrer::infer(&scale_impl, current_domain, range_hint, ctx, params).await?;
        self.domain = Maybe::Set(inferred_domain);
        Ok(self)
    }

    /// Apply normalization (zero, nice, padding) to domain
    pub async fn normalize_domain(
        mut self,
        plot_area_width: f32,
        plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
    ) -> Result<Self, AvengerChartError> {
        // Skip for non-numeric ranges
        if !matches!(self.range.as_ref(), Maybe::Set(ScaleRange::Numeric(_, _))) {
            return Ok(self);
        }

        // Skip for discrete domains - normalization doesn't apply to categorical data
        if let Maybe::Set(domain) = self.domain.as_ref() {
            if matches!(
                &domain.default_domain,
                ScaleDefaultDomain::Discrete(_) | ScaleDefaultDomain::NoDefault
            ) {
                return Ok(self);
            }
        } else {
            // No domain set, skip normalization
            return Ok(self);
        }

        // Create ConfiguredScale to apply normalization
        let empty_params = indexmap::IndexMap::new();
        let configured = self
            .create_configured_scale(plot_area_width, plot_area_height, ctx, &empty_params)
            .await?;

        // Update domain with normalized values
        // This will only succeed for scales with numeric interval domains
        // Discrete scales will return Err and keep their original domain
        if let Ok((min, max)) = configured.numeric_interval_domain() {
            self = self.domain_interval(lit(min as f64), lit(max as f64));
        }

        Ok(self)
    }

    /// Create a ConfiguredScale for rendering
    pub async fn create_configured_scale(
        &self,
        _plot_area_width: f32,
        _plot_area_height: f32,
        ctx: &datafusion::prelude::SessionContext,
        params: &indexmap::IndexMap<String, ScalarValue>,
    ) -> Result<avenger_scales::scales::ConfiguredScale, AvengerChartError> {
        use avenger_scales::scales::{ConfiguredScale, ScaleConfig, ScaleContext};
        use datafusion::arrow::array::{ArrayRef, Float32Array, StringArray};

        // Get scale implementation (required)
        let scale_impl = self.get_scale_impl_or_err()?;

        // Get domain or error
        let domain_spec = self.domain.as_option().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Domain must be specified for scale before creating ConfiguredScale".to_string(),
            )
        })?;

        // Extract domain values as arrow array
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
                let datafusion_params = crate::utils::params_to_datafusion(params);
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

                let start_f32 = start_val.as_f32()?;
                let end_f32 = end_val.as_f32()?;
                Arc::new(Float32Array::from(vec![start_f32, end_f32])) as ArrayRef
            }
            ScaleDefaultDomain::Discrete(values) => {
                // Determine domain type based on scale's domain kind
                let exprs: Vec<Expr> = values
                    .iter()
                    .map(|v| v.to_expr(ctx))
                    .collect::<Result<Vec<_>, _>>()?;
                let scalars = eval_to_scalars(exprs, Some(ctx), None).await?;

                match scale_impl.domain_kind() {
                    DomainKind::Numeric => {
                        // Numeric domains need float values
                        let mut float_values = Vec::new();
                        for scalar in scalars {
                            float_values.push(scalar.as_f32()?);
                        }
                        Arc::new(Float32Array::from(float_values)) as ArrayRef
                    }
                    DomainKind::Categorical => {
                        // Categorical domains need string values
                        let mut string_values = Vec::new();
                        for scalar in scalars {
                            string_values.push(scalar.as_scalar_string()?);
                        }
                        Arc::new(StringArray::from(string_values)) as ArrayRef
                    }
                    DomainKind::Temporal => {
                        // Temporal domains - convert to appropriate temporal type
                        // For now, treat as numeric (timestamps)
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

        // Extract range values - use default if not set
        let range = match self.range.as_ref() {
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
                // Convert SerializableScalar values back to ScalarValue
                let scalar_values: Vec<ScalarValue> =
                    values.iter().map(|v| v.as_scalar().clone()).collect();

                // Check if all values are numeric - if so, keep as Float32Array
                // This handles cases like stroke_width which uses ordinal scale with numeric range
                let all_numeric = scalar_values.iter().all(|v| v.as_f32().is_ok());

                if all_numeric {
                    let mut float_values = Vec::new();
                    for scalar in &scalar_values {
                        float_values.push(scalar.as_f32()?);
                    }
                    Arc::new(Float32Array::from(float_values)) as ArrayRef
                } else {
                    // For non-numeric discrete values, convert to strings
                    let mut string_values = Vec::new();
                    for scalar in &scalar_values {
                        string_values.push(scalar.as_scalar_string()?);
                    }
                    Arc::new(StringArray::from(string_values)) as ArrayRef
                }
            }
            Maybe::Set(ScaleRange::Color(colors)) => {
                // Convert colors to a list array of RGBA values
                use datafusion::arrow::array::Float32Builder;
                use datafusion::arrow::array::ListBuilder;

                let mut list_builder = ListBuilder::new(Float32Builder::new());

                for color in colors {
                    // Append a new list entry for this color (color is [r, g, b, a])
                    let values_builder = list_builder.values();
                    values_builder.append_value(color[0]); // red
                    values_builder.append_value(color[1]); // green
                    values_builder.append_value(color[2]); // blue
                    values_builder.append_value(color[3]); // alpha
                    list_builder.append(true);
                }

                Arc::new(list_builder.finish()) as ArrayRef
            }
            Maybe::Unset => {
                // Default range [0, 1]
                Arc::new(Float32Array::from(vec![0.0_f32, 1.0_f32])) as ArrayRef
            }
        };

        // Extract user-set options as HashMap<String, Scalar>
        let mut scalar_options = HashMap::new();

        // First apply user-set options (including scale-type defaults like Sqrt's exponent)
        for (key, value_expr) in &self.options {
            // Evaluate the expression to get a scalar value
            let expr = value_expr.to_expr(ctx)?;
            let scalars = eval_to_scalars(vec![expr], Some(ctx), None).await?;
            if let Some(scalar_value) = scalars.first() {
                // Convert to avenger_scales::scalar::Scalar
                let scalar = scalar_value.as_scale_scalar()?;
                scalar_options.insert(key.clone(), scalar);
            }
        }

        // Then get default options from the scale implementation
        let default_options = scale_impl.default_options();

        // Apply implementation defaults only for options not already set
        // This ensures Sqrt's exponent:0.5 isn't overridden by PowScale's exponent:1.0
        for (key, value) in default_options {
            if !scalar_options.contains_key(&key) {
                scalar_options.insert(key, value);
            }
        }

        // Convert padding to clip_padding_lower and clip_padding_upper for numeric continuous scales
        if scalar_options.contains_key("padding") {
            // Check if this is a numeric continuous scale that supports clip padding
            if scale_impl.domain_kind() == DomainKind::Numeric
                && scale_impl.range_kind() == RangeKind::Continuous
            {
                if let Some(padding_value) = scalar_options.get("padding").cloned() {
                    // For continuous scales, padding becomes clip_padding
                    if !scalar_options.contains_key("clip_padding_lower") {
                        scalar_options
                            .insert("clip_padding_lower".to_string(), padding_value.clone());
                    }
                    if !scalar_options.contains_key("clip_padding_upper") {
                        scalar_options.insert("clip_padding_upper".to_string(), padding_value);
                    }
                    // Remove the padding option as it's been converted
                    scalar_options.remove("padding");
                }
            }
            // For band and point scales, padding is valid and should be kept as-is
            // For other scales (ordinal, threshold, etc.), padding is not used
        }

        // Create scale context using default
        let context = ScaleContext::default();

        // Create scale config
        let config = ScaleConfig {
            domain,
            range,
            options: scalar_options,
            context,
        };

        // Create configured scale
        Ok(ConfiguredScale { scale_impl, config })
    }
}

// ===== Linear scale specific methods =====
impl Scale<Linear> {
    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }

    /// Set padding as a fraction of the domain
    pub fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }
}

// ===== Log scale specific methods =====
impl Scale<Log> {
    /// Set the logarithm base
    pub fn base(self, value: f32) -> Self {
        self._option("base", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Pow scale specific methods =====
impl Scale<Pow> {
    /// Set the exponent
    pub fn exponent(self, value: f32) -> Self {
        self._option("exponent", lit(value))
    }

    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Sqrt scale specific methods =====
impl Scale<Sqrt> {
    /// Set whether to include zero in the domain
    pub fn zero(self, value: bool) -> Self {
        self._option("zero", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Symlog scale specific methods =====
impl Scale<Symlog> {
    /// Set the constant parameter
    pub fn constant(self, value: f32) -> Self {
        self._option("constant", lit(value))
    }

    /// Set whether to nice the domain
    pub fn nice(self, value: bool) -> Self {
        self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Band scale specific methods =====
impl Scale<Band> {
    /// Set the inner padding (between bands)
    pub fn padding_inner(self, value: f32) -> Self {
        self._option("padding_inner", lit(value))
    }

    /// Set the outer padding (before first and after last band)
    pub fn padding_outer(self, value: f32) -> Self {
        self._option("padding_outer", lit(value))
    }

    /// Set the alignment (0 = left, 0.5 = center, 1 = right)
    pub fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }

    /// Set whether to round positions to pixel boundaries
    pub fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

// ===== Point scale specific methods =====
impl Scale<Point> {
    /// Set the padding (as a fraction of the step)
    pub fn padding(self, value: f32) -> Self {
        self._option("padding", lit(value))
    }

    /// Set the alignment (0 = left, 0.5 = center, 1 = right)
    pub fn align(self, value: f32) -> Self {
        self._option("align", lit(value))
    }

    /// Set whether to round positions to pixel boundaries
    pub fn round(self, value: bool) -> Self {
        self._option("round", lit(value))
    }
}

// ===== Ordinal scale specific methods =====
impl Scale<Ordinal> {
    /// Set the value to use for unknown inputs
    pub fn unknown(self, value: impl Into<Expr>) -> Self {
        self._option("unknown", value.into())
    }
}

// ===== Time scale specific methods =====
impl Scale<Time> {
    /// Set whether to nice the domain to time intervals
    pub fn nice(mut self, value: bool) -> Self {
        let expr = lit(value);
        self.options.insert(
            "nice".to_string(),
            LogicalExprNode::from_expr(expr).expect("Failed to serialize option expr"),
        );
        self
        // self._option("nice", lit(value))
    }

    /// Set whether to clamp values outside the domain
    pub fn clamp(self, value: bool) -> Self {
        self._option("clamp", lit(value))
    }
}

// ===== Auto scale for dynamic construction =====
impl Scale<Auto> {
    /// Get the ScaleImpl from this Scale
    pub fn to_scale_impl(&self) -> Result<Arc<dyn ScaleImpl>, AvengerChartError> {
        self.get_scale_impl_or_err()
    }

    /// Set a generic option
    ///
    /// This method allows dynamic configuration when the scale type isn't known at compile time.
    /// For typed scales (like Scale<Linear>), use the specific methods instead (e.g., `.nice()`, `.zero()`)
    ///
    /// # Example
    /// ```no_run
    /// use avenger_chart::scales::{Scale, Auto, Linear};
    /// use datafusion::prelude::lit;
    /// use std::sync::Arc;
    ///
    /// // Create an Auto scale from a Linear implementation
    /// let scale = Scale::<Linear>::new()
    ///     .into_auto()
    ///     .option("nice", lit(true))
    ///     .option("clamp", lit(false));
    /// ```
    pub fn option(self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        self._option(key, value)
    }

    /// Create a scale from a dynamic ScaleSpec (used when type is not known at compile time)
    pub fn from_spec(scale_spec: Box<dyn ScaleSpec>) -> Self {
        Self {
            scale_spec: Maybe::Set(scale_spec),
            domain: Maybe::Unset,
            range: Maybe::Unset,
            options: HashMap::new(), // Start empty, not pre-populated with defaults
            _phantom: PhantomData,
        }
    }
}
