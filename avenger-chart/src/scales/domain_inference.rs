//! Domain inference from data for scales
//!
//! This module handles the logic of inferring scale domains from data,
//! including special handling for radius-aware padding calculations.

use crate::error::AvengerChartError;
use crate::marks::RadiusExpression;
use crate::scales::domain::{DomainExpr, ScaleDefaultDomain, ScaleDomain};
use crate::serialization::LogicalExprNodeExt;
use crate::utils::DataFrameChartHelpers;
use avenger_scales::scales::{DomainKind, InferDomainFromDataMethod, RangeKind, ScaleImpl};
use datafusion::arrow::array::{Array, AsArray};
use datafusion::arrow::datatypes::{DataType, Field, Float64Type, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::dataframe::DataFrame;
use datafusion::logical_expr::{col, lit};
use datafusion::prelude::SessionContext;
use datafusion_common::ScalarValue;
use indexmap::IndexMap;
use std::sync::Arc;

// Column name constants to avoid magic strings
const POSITION_COL: &str = "__position__";
const RADIUS_LOWER_COL: &str = "__radius_lower__";
const RADIUS_UPPER_COL: &str = "__radius_upper__";
const DOMAIN_COL: &str = "__domain_col__";
const DOMAIN_RESULT_COL: &str = "domain";
const DOMAIN_FIELD: &str = "__domain__";

/// Infers domain from data fields for a scale
pub struct DomainInferrer;

impl DomainInferrer {
    /// Infer domain from data fields and return the inferred domain
    ///
    /// # Arguments
    /// * `scale_impl` - The scale implementation
    /// * `domain` - The current domain with potential DomainExprs
    /// * `range_hint` - Optional range to use for radius-aware padding calculations
    pub async fn infer(
        scale_impl: &Arc<dyn ScaleImpl>,
        mut domain: ScaleDomain,
        range_hint: Option<(f64, f64)>,
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<ScaleDomain, AvengerChartError> {
        // Extract the default domain, replacing it temporarily
        let default_domain = std::mem::replace(
            &mut domain.default_domain,
            ScaleDefaultDomain::Discrete(vec![]),
        );

        if let ScaleDefaultDomain::DomainExprs(data_fields) = default_domain {
            // Process radius-aware domains first, transforming fields in place
            let processed_fields =
                Self::process_radius_domains(scale_impl, data_fields, range_hint, ctx).await?;

            // Process standard domain inference
            let inferred_domain =
                Self::infer_standard_domain(scale_impl, &processed_fields, ctx, params).await?;
            domain.default_domain = inferred_domain;
        } else {
            // Restore the original domain if it wasn't DomainExprs
            domain.default_domain = default_domain;
        }

        Ok(domain)
    }

    /// Process domains that have radius expressions for padding calculations
    /// Returns a new vector with radius-aware domains computed where applicable
    async fn process_radius_domains(
        scale_impl: &Arc<dyn ScaleImpl>,
        mut data_fields: Vec<DomainExpr>,
        range_hint: Option<(f64, f64)>,
        ctx: &SessionContext,
    ) -> Result<Vec<DomainExpr>, AvengerChartError> {
        // Only process radius domains for numeric continuous scales with a range hint
        let is_numeric_continuous = scale_impl.domain_kind() == DomainKind::Numeric
            && scale_impl.range_kind() == RangeKind::Continuous
            && scale_impl.scale_type() == "linear"; // Keep linear check for now as radius handling is specific to linear

        match (is_numeric_continuous, range_hint) {
            (true, Some(range)) => {
                // Process each field with radius expressions
                for field in &mut data_fields {
                    if let Some(radius_expr) = &field.radius {
                        let expr_ser: crate::serialization::SerializableExpr = field.expr.clone().into();
                        let computed_domain = Self::compute_radius_aware_domain(
                            &field.dataframe,
                            &expr_ser,
                            radius_expr,
                            range,
                            ctx,
                        )
                        .await?;

                        if let Some(domain_expr) = computed_domain {
                            *field = domain_expr;
                        }
                    }
                }
                Ok(data_fields)
            }
            _ => Ok(data_fields),
        }
    }

    /// Compute domain with radius-aware padding
    async fn compute_radius_aware_domain(
        dataframe: &Arc<datafusion_proto::protobuf::LogicalPlanNode>,
        position_expr: &crate::serialization::SerializableExpr,
        radius_expr: &RadiusExpression,
        range_hint: (f64, f64),
        ctx: &SessionContext,
    ) -> Result<Option<DomainExpr>, AvengerChartError> {
        let (range_min, range_max) = range_hint;
        let range_width = (range_max - range_min).abs();

        // Convert SerializableExpr to Expr
        let position_expr_df = position_expr.to_expr(ctx)?;

        // Select both position and radius expressions
        let select_exprs = match radius_expr {
            RadiusExpression::Symmetric(expr) => {
                let radius_expr_df = expr.to_expr(ctx)?;
                vec![
                    position_expr_df.alias(POSITION_COL),
                    radius_expr_df.clone().alias(RADIUS_LOWER_COL),
                    radius_expr_df.alias(RADIUS_UPPER_COL),
                ]
            }
            RadiusExpression::Asymmetric { lower, upper } => {
                let lower_expr_df = lower.to_expr(ctx)?;
                let upper_expr_df = upper.to_expr(ctx)?;
                vec![
                    position_expr_df.alias(POSITION_COL),
                    lower_expr_df.alias(RADIUS_LOWER_COL),
                    upper_expr_df.alias(RADIUS_UPPER_COL),
                ]
            }
        };

        // Convert LogicalPlanNode to DataFrame and select
        use crate::serialization::LogicalPlanNodeExt;
        let plan = dataframe.to_logical_plan(ctx)?;
        let df = DataFrame::new(ctx.state().clone(), plan);
        let df_with_exprs = df.select(select_exprs)?;
        let batches = df_with_exprs.collect().await?;

        if batches.is_empty() || batches[0].num_rows() == 0 {
            return Ok(None);
        }

        let batch = &batches[0];
        let position_array = batch.column_by_name(POSITION_COL).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Column '{}' not found in batch",
                POSITION_COL
            ))
        })?;
        let radius_lower_array = batch.column_by_name(RADIUS_LOWER_COL).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Column '{}' not found in batch",
                RADIUS_LOWER_COL
            ))
        })?;
        let radius_upper_array = batch.column_by_name(RADIUS_UPPER_COL).ok_or_else(|| {
            AvengerChartError::InternalError(format!(
                "Column '{}' not found in batch",
                RADIUS_UPPER_COL
            ))
        })?;

        // Cast to Float64
        use datafusion::arrow::compute::cast;
        use datafusion::arrow::datatypes::DataType as ArrowDataType;

        let position_f64 = cast(position_array, &ArrowDataType::Float64)?;
        let radius_lower_f64 = cast(radius_lower_array, &ArrowDataType::Float64)?;
        let radius_upper_f64 = cast(radius_upper_array, &ArrowDataType::Float64)?;

        // Extract values as slices
        let positions = position_f64.as_primitive::<Float64Type>();
        let radius_lower = radius_lower_f64.as_primitive::<Float64Type>();
        let radius_upper = radius_upper_f64.as_primitive::<Float64Type>();

        // Convert to vectors, filtering out nulls
        let position_vec: Vec<f64> = positions.iter().flatten().collect();
        let radius_lower_vec: Vec<f64> = radius_lower.iter().flatten().collect();
        let radius_upper_vec: Vec<f64> = radius_upper.iter().flatten().collect();

        // Validate matching lengths
        let pos_len = position_vec.len();
        let lower_len = radius_lower_vec.len();
        let upper_len = radius_upper_vec.len();

        if pos_len == 0 {
            return Ok(None);
        }

        // Use the padding solver
        use avenger_scales::scales::domain_solver::compute_domain_from_data_with_padding_linear;

        let (d_min, d_max) = if pos_len != lower_len || pos_len != upper_len {
            // Log warning about mismatched lengths and use minimum
            let min_len = pos_len.min(lower_len).min(upper_len);
            if min_len == 0 {
                return Ok(None);
            }
            // Note: In production, we might want to log this mismatch
            compute_domain_from_data_with_padding_linear(
                &position_vec[..min_len],
                &radius_lower_vec[..min_len],
                &radius_upper_vec[..min_len],
                range_width,
            )?
        } else {
            compute_domain_from_data_with_padding_linear(
                &position_vec,
                &radius_lower_vec,
                &radius_upper_vec,
                range_width,
            )?
        };

        // Create a new DataFrame with the computed domain
        use datafusion::arrow::array::Float64Array;

        let domain_array = Float64Array::from(vec![d_min, d_max]);
        let schema = Arc::new(Schema::new(vec![Field::new(
            DOMAIN_FIELD,
            DataType::Float64,
            false,
        )]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(domain_array)])?;

        let domain_df = Arc::new(ctx.read_batch(batch)?);
        let plan = domain_df.logical_plan().clone();
        use datafusion_proto::protobuf::{LogicalExprNode, LogicalPlanNode};
        let plan_node = Arc::new(
            LogicalPlanNode::from_logical_plan(&plan)?
        );
        let expr_node = LogicalExprNode::from_expr(col(DOMAIN_FIELD))?;

        Ok(Some(DomainExpr {
            dataframe: plan_node,
            expr: expr_node,
            radius: None,
        }))
    }

    /// Infer standard domain from data fields (without radius)
    async fn infer_standard_domain(
        scale_impl: &Arc<dyn ScaleImpl>,
        data_fields: &[DomainExpr],
        ctx: &SessionContext,
        params: &IndexMap<String, ScalarValue>,
    ) -> Result<ScaleDefaultDomain, AvengerChartError> {
        // Collect all data into single-column DataFrames
        let mut single_col_dfs: Vec<DataFrame> = Vec::new();

        for field in data_fields {
            // Convert LogicalPlanNode to DataFrame
            use crate::serialization::LogicalPlanNodeExt;
            let plan = field.dataframe.to_logical_plan(ctx)?;
            let df = DataFrame::new(ctx.state().clone(), plan);

            // Convert SerializableExpr to Expr
            let expr_df = field.expr.to_expr(ctx)?;

            // For scales expecting categorical domains, cast numeric values to strings
            let expr = if scale_impl.domain_kind() == DomainKind::Categorical {
                // Categorical scales expect string domains
                use datafusion::arrow::datatypes::DataType;
                use datafusion::logical_expr::cast;
                cast(expr_df, DataType::Utf8)
            } else {
                // Numeric and Temporal scales use values as-is
                expr_df
            };

            let df_with_expr = df.select(vec![expr.alias(DOMAIN_COL)])?;

            single_col_dfs.push(df_with_expr);
        }

        // Union all DataFrames
        let union_df = if single_col_dfs.is_empty() {
            // No data to infer from - return default interval
            use datafusion_proto::protobuf::LogicalExprNode;
            use datafusion::logical_expr::lit;
            let start = LogicalExprNode::from_expr(lit(0.0))?;
            let end = LogicalExprNode::from_expr(lit(1.0))?;
            return Ok(ScaleDefaultDomain::Interval(start, Box::new(end)));
        } else if single_col_dfs.len() > 1 {
            let mut result = single_col_dfs[0].clone();
            for df in single_col_dfs.iter().skip(1) {
                result = result.union(df.clone())?;
            }
            result
        } else {
            single_col_dfs[0].clone()
        };

        // Determine the appropriate method based on scale type
        let method = scale_impl.infer_domain_from_data_method();
        // Use DataFrameChartHelpers to get domain expression
        let domain_expr = match method {
            InferDomainFromDataMethod::Interval => union_df.span()?,
            InferDomainFromDataMethod::Unique => union_df.unique_values()?,
            InferDomainFromDataMethod::All => union_df.all_values()?,
            InferDomainFromDataMethod::Explicit => {
                // Explicit scales shouldn't infer domain from data
                return Ok(ScaleDefaultDomain::NoDefault);
            }
        };

        // Evaluate the domain expression
        let empty_df = ctx.read_empty()?;
        let result_df = empty_df.select(vec![domain_expr.alias(DOMAIN_RESULT_COL)])?;
        let datafusion_params = crate::utils::params_to_datafusion(params);
        let batches = if let Some(param_values) = datafusion_params {
            result_df.with_param_values(param_values)?.collect().await?
        } else {
            result_df.collect().await?
        };

        if batches.is_empty() {
            return Err(AvengerChartError::InternalError(
                "No batches returned from domain inference query".to_string(),
            ));
        }

        let domain_array = batches[0]
            .column_by_name(DOMAIN_RESULT_COL)
            .ok_or_else(|| {
                AvengerChartError::InternalError(format!(
                    "Column '{}' not found in result",
                    DOMAIN_RESULT_COL
                ))
            })?
            .clone();

        // Convert to domain based on method
        Self::array_to_domain(domain_array, method)
    }

    /// Convert an arrow array to a ScaleDefaultDomain
    fn array_to_domain(
        domain_array: Arc<dyn Array>,
        method: InferDomainFromDataMethod,
    ) -> Result<ScaleDefaultDomain, AvengerChartError> {
        let Some(list_array) = domain_array.as_list_opt::<i32>() else {
            return Err(AvengerChartError::InternalError(
                "Expected domain to be a ListArray".to_string(),
            ));
        };

        if list_array.len() == 0 {
            return Ok(ScaleDefaultDomain::Discrete(vec![]));
        }

        let inner_array = list_array.value(0);

        if method == InferDomainFromDataMethod::Interval {
            // For interval domains, extract min and max
            if inner_array.len() >= 2 {
                let min_val = ScalarValue::try_from_array(&inner_array, 0)?;
                let max_val = ScalarValue::try_from_array(&inner_array, inner_array.len() - 1)?;
                use datafusion_proto::protobuf::LogicalExprNode;
                let min_expr = LogicalExprNode::from_expr(lit(min_val))?;
                let max_expr = LogicalExprNode::from_expr(lit(max_val))?;
                Ok(ScaleDefaultDomain::Interval(min_expr, Box::new(max_expr)))
            } else {
                Ok(ScaleDefaultDomain::Discrete(vec![]))
            }
        } else {
            // For discrete domains, extract all values
            use datafusion_proto::protobuf::LogicalExprNode;
            let mut values = Vec::new();
            for i in 0..inner_array.len() {
                let val = ScalarValue::try_from_array(&inner_array, i)?;
                let expr = LogicalExprNode::from_expr(lit(val))?;
                values.push(expr);
            }
            Ok(ScaleDefaultDomain::Discrete(values))
        }
    }
}
