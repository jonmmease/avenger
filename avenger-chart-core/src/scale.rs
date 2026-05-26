//! Type-safe scale authoring contracts

use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use avenger_scales::scales::{DomainKind, RangeKind, ScaleImpl};
use datafusion::{
    dataframe::DataFrame,
    logical_expr::{Expr, lit},
};
use datafusion_common::ScalarValue;
use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
use palette::Srgba;
use serde::{Deserialize, Serialize};

use crate::{
    Auto, AvengerChartError, DefaultLogicalExprNodeExt, DomainExpr, IntoExpr, LogicalPlanNodeExt,
    Maybe, RadiusExpression, ScaleConfigSpec, ScaleDefaultDomain, ScaleDomain, ScaleOrderingSpec,
    ScaleRange, ScaleSpec, scalar_to_scalar_value,
};

/// Type-safe scale authoring wrapper.
#[derive(Debug, Serialize, Deserialize)]
pub struct Scale<S: ScaleSpec = Auto> {
    #[serde(flatten)]
    pub(crate) config: ScaleConfigSpec,
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
        Scale {
            config: self.config.clone(),
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
            config: ScaleConfigSpec::new(
                if spec.name() != "auto" {
                    Maybe::Set(Box::new(spec) as Box<dyn ScaleSpec>)
                } else {
                    Maybe::Unset
                },
                Maybe::Unset,
                Maybe::Unset,
                Maybe::Unset,
                options,
            ),
            _phantom: PhantomData,
        }
    }

    /// Wrap an owned scale configuration payload in the typed scale API.
    pub fn from_config(config: ScaleConfigSpec) -> Self {
        Self {
            config,
            _phantom: PhantomData,
        }
    }

    /// Return the owned scale configuration payload.
    pub fn into_config(self) -> ScaleConfigSpec {
        self.config
    }

    /// Borrow the owned scale configuration payload.
    pub fn config(&self) -> &ScaleConfigSpec {
        &self.config
    }

    /// Mutably borrow the owned scale configuration payload.
    #[doc(hidden)]
    pub fn config_mut(&mut self) -> &mut ScaleConfigSpec {
        &mut self.config
    }

    /// Return a clone of the underlying scale spec if explicitly set
    /// This allows callers to honor user-chosen scale types (e.g., Ordinal)
    pub fn get_scale_spec(&self) -> Option<Box<dyn ScaleSpec>> {
        self.config
            .scale_spec
            .as_option()
            .map(|spec| spec.clone_box())
    }

    /// Update this scale with properties from another scale
    /// Properties that are Set in `other` override properties in `self`
    pub fn update(mut self, other: Scale<Auto>) -> Self {
        // Check if scale type is changing
        let scale_type_changed = other.config.scale_spec.is_set();

        // Update scale_spec only if explicitly set in other
        if scale_type_changed {
            self.config.scale_spec = other.config.scale_spec;
        }

        // Update domain if set
        if other.config.domain.is_set() {
            self.config.domain = other.config.domain;
        }

        // Update range if set
        if other.config.range.is_set() {
            self.config.range = other.config.range;
        }

        // Update ordering if set. Merge individual fields so plot-level
        // overrides can set only direction or only the expression.
        if let Maybe::Set(other_ordering) = other.config.ordering {
            match &mut self.config.ordering {
                Maybe::Set(ordering) => ordering.merge(other_ordering),
                Maybe::Unset => self.config.ordering = Maybe::Set(other_ordering),
            }
        }

        // Update options - all options in other override those in self
        // (presence in HashMap means it was explicitly set)
        for (key, value) in other.config.options {
            self.config.options.insert(key, value);
        }

        // If scale type changed, filter options to only keep supported ones
        if scale_type_changed && let Some(scale_spec) = self.config.scale_spec.as_option() {
            let scale_impl = scale_spec.create_impl();
            // Get supported options for the new scale type
            let option_definitions = scale_impl.option_definitions();
            let supported_options: std::collections::HashSet<&str> = option_definitions
                .iter()
                .map(|def| def.name.as_str())
                .collect();

            // Filter options to only keep supported ones
            self.config
                .options
                .retain(|key, _| supported_options.contains(key.as_str()));
        }

        self
    }

    /// Set the domain
    pub fn domain<D: Into<ScaleDomain>>(mut self, domain: D) -> Self {
        self.config.domain = Maybe::Set(domain.into());
        self
    }

    /// Set the domain as an interval
    pub fn domain_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.config.domain = Maybe::Set(ScaleDomain::new_interval(min.into(), max.into()));
        self
    }

    /// Set the domain as discrete values
    pub fn domain_discrete(mut self, values: Vec<impl Into<Expr>>) -> Self {
        self.config.domain = Maybe::Set(ScaleDomain::new_discrete(
            values.into_iter().map(|v| v.into()).collect(),
        ));
        self
    }

    /// Order an inferred categorical domain by an aggregate, constant, or category expression.
    pub fn order_by(mut self, expr: impl IntoExpr) -> Self {
        let mut ordering = self
            .config
            .ordering
            .unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_expr = Some(
            LogicalExprNode::from_expr(expr.into_expr())
                .expect("Failed to serialize scale order expression"),
        );
        self.config.ordering = Maybe::Set(ordering);
        self
    }

    /// Sort inferred categorical domains in ascending order by their order expression.
    pub fn order_asc(mut self) -> Self {
        let mut ordering = self
            .config
            .ordering
            .unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_descending = Some(false);
        self.config.ordering = Maybe::Set(ordering);
        self
    }

    /// Sort inferred categorical domains in descending order by their order expression.
    pub fn order_desc(mut self) -> Self {
        let mut ordering = self
            .config
            .ordering
            .unwrap_or_else(ScaleOrderingSpec::empty);
        ordering.order_descending = Some(true);
        self.config.ordering = Maybe::Set(ordering);
        self
    }

    /// Set domain from data field
    pub fn domain_data(mut self, dataframe: Arc<DataFrame>, expr: Expr) -> Self {
        let mut domain = self
            .config
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
        self.config.domain = Maybe::Set(domain);
        self
    }

    /// Set domain from data fields
    pub fn domain_data_fields(mut self, fields: Vec<(Arc<DataFrame>, Expr)>) -> Self {
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
            .config
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.config.domain = Maybe::Set(domain);
        self
    }

    /// Set domain from data fields with radius expressions (accepts pre-serialized LogicalPlanNode)
    ///
    /// This method accepts pre-serialized LogicalPlanNodes to avoid repeated serialization
    /// which can cause hangs due to DataFusion Issue #2659 (circular Arc references).
    pub fn domain_data_fields_with_radius_preserialized(
        mut self,
        fields: Vec<(Arc<LogicalPlanNode>, Expr, Option<RadiusExpression>)>,
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
            .config
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.config.domain = Maybe::Set(domain);
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
            .config
            .domain
            .unwrap_or(ScaleDomain::new_interval(lit(0.0), lit(1.0)));
        domain.default_domain = ScaleDefaultDomain::DomainExprs(exprs);
        self.config.domain = Maybe::Set(domain);
        self
    }

    /// Set the range
    pub fn range(mut self, range: ScaleRange) -> Self {
        self.config.range = Maybe::Set(range);
        self
    }

    /// Set the range as an interval
    pub fn range_interval(mut self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self {
        self.config.range = Maybe::Set(ScaleRange::new_interval(min.into(), max.into()));
        self
    }

    /// Set the range as discrete values from ScalarValues
    pub fn range_discrete(mut self, values: Vec<impl Into<ScalarValue>>) -> Self {
        self.config.range = Maybe::Set(ScaleRange::new_discrete(
            values.into_iter().map(|v| v.into()).collect(),
        ));
        self
    }

    /// Set the range as colors
    pub fn range_colors(mut self, colors: Vec<Srgba>) -> Self {
        self.config.range = Maybe::Set(ScaleRange::new_color(colors));
        self
    }

    pub fn get_scale_type(&self) -> Option<&str> {
        self.config.scale_spec.as_option().map(|spec| spec.name())
    }

    /// Internal method for setting options
    /// This is intentionally undocumented and prefixed with _ to discourage direct use.
    /// External crates implementing custom scales can use this for their typed methods.
    /// Regular users should use the typed methods or Scale<Auto>::option()
    #[doc(hidden)]
    pub fn _option(mut self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        let expr = value.into();
        self.config.options.insert(
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
            self.config.scale_spec
        } else {
            // Set new spec for specific types
            Maybe::Set(Box::new(T::default()) as Box<dyn ScaleSpec>)
        };

        // When changing scale types, we need to add the new type's default options
        // but preserve any existing options that override them
        let mut options = self.config.options;

        // Add scale type defaults for options not already set
        if spec.name() != "auto" {
            for (key, value) in spec.default_options() {
                // Only add if not already present
                options.entry(key).or_insert_with(|| {
                    let scalar_value = scalar_to_scalar_value(&value);
                    let expr = lit(scalar_value);
                    LogicalExprNode::from_expr(expr).expect("Failed to serialize option expr")
                });
            }
        }

        Scale {
            config: ScaleConfigSpec::new(
                scale_spec,
                self.config.domain,
                self.config.range,
                self.config.ordering,
                options,
            ),
            _phantom: PhantomData,
        }
    }

    /// Convert to Auto type for storage
    pub fn into_auto(self) -> Scale<Auto> {
        Scale {
            config: self.config,
            _phantom: PhantomData,
        }
    }

    // ===== Getters =====

    pub fn get_scale_impl(&self) -> Option<Arc<dyn ScaleImpl>> {
        self.config
            .scale_spec
            .as_option()
            .map(|spec| spec.create_impl())
    }

    pub fn get_scale_impl_or_err(&self) -> Result<Arc<dyn ScaleImpl>, AvengerChartError> {
        self.get_scale_impl().ok_or_else(|| {
            AvengerChartError::InternalError(
                "Scale specification not set - this is a bug".to_string(),
            )
        })
    }

    pub fn get_domain(&self) -> Option<&ScaleDomain> {
        self.config.domain.as_option()
    }

    pub fn get_range(&self) -> Option<&ScaleRange> {
        self.config.range.as_option()
    }

    pub fn get_ordering(&self) -> Option<&ScaleOrderingSpec> {
        self.config.ordering.as_option()
    }

    pub fn get_options(&self) -> &HashMap<String, LogicalExprNode> {
        &self.config.options
    }

    /// Get the domain kind for this scale instance
    pub fn domain_kind(&self) -> Option<DomainKind> {
        self.config
            .scale_spec
            .as_option()
            .map(|spec| spec.domain_kind())
    }

    /// Get the range kind for this scale instance
    pub fn range_kind(&self) -> Option<RangeKind> {
        self.config
            .scale_spec
            .as_option()
            .map(|spec| spec.range_kind())
    }

    /// Check if this scale has bands (true for band scales, false for point scales)
    pub fn has_bands(&self) -> bool {
        self.config
            .scale_spec
            .as_option()
            .map(|spec| spec.name() == "band")
            .unwrap_or(false)
    }

    /// Get the scale type name
    pub fn scale_type_name(&self) -> Option<&str> {
        self.config.scale_spec.as_option().map(|spec| spec.name())
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
    /// ```ignore
    /// scale.option("nice", lit(true)).option("clamp", lit(false));
    /// ```
    pub fn option(self, key: impl Into<String>, value: impl Into<Expr>) -> Self {
        self._option(key, value)
    }

    /// Create a scale from a dynamic ScaleSpec (used when type is not known at compile time)
    pub fn from_spec(scale_spec: Box<dyn ScaleSpec>) -> Self {
        Self {
            config: ScaleConfigSpec::new(
                Maybe::Set(scale_spec),
                Maybe::Unset,
                Maybe::Unset,
                Maybe::Unset,
                HashMap::new(), // Start empty, not pre-populated with defaults
            ),
            _phantom: PhantomData,
        }
    }
}
