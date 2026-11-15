//! What the transforms share: the fields of a struct of a start and an end, the values of
//! window partitions, and [`StructGroupSplit`], the rule that groups an aggregate by the fields
//! of a struct key.

use std::collections::BTreeMap;
use std::sync::Arc;

use datafusion::arrow::array::{Array, AsArray, StructArray};
use datafusion::arrow::buffer::NullBuffer;
use datafusion::arrow::datatypes::{DataType, Field, FieldRef, Fields};
use datafusion::common::tree_node::Transformed;
use datafusion::common::{Column, JoinType, NullEquality, Result, ScalarValue, Spans};
use datafusion::functions::core::expr_fn::get_field;
use datafusion::logical_expr::{
    Aggregate, ColumnarValue, Expr, ExprSchemable, LogicalPlan, LogicalPlanBuilder,
    ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Subquery, Volatility,
    lit, when,
};
use datafusion::optimizer::{ApplyOrder, OptimizerConfig, OptimizerRule};

use crate::shared::shared;

// ---------------------------------------------------------------------------------------
// Structs and options
// ---------------------------------------------------------------------------------------

/// The fields of the result of `bin` and `stack`: a `start` and an `end`.
pub(crate) fn interval_fields() -> Fields {
    Fields::from(vec![
        Field::new("start", DataType::Float64, false),
        Field::new("end", DataType::Float64, false),
    ])
}

// ---------------------------------------------------------------------------------------
// Values of window partitions
// ---------------------------------------------------------------------------------------

/// A value of each window partition: aggregates of the partition's rows, with unique
/// aliases, and an expression of the aggregates by those aliases.
pub(crate) struct PartitionValue {
    pub aggregates: Vec<Expr>,
    pub value: Expr,
}

/// The input with each value of [`PartitionValue`] for each row: without keys, each value is a
/// scalar subquery of the input; with keys, one aggregate of the input by the keys, joined back
/// to the rows on the keys, where NULL keys match, as PARTITION BY puts them in one partition.
/// The rows and the values read one run of the input, through a shared node. `name` names the
/// aliases. Returns the builder of the input or the join, and the expression
/// of each value.
pub(crate) fn partition_values(
    input: LogicalPlan,
    keys: &[Column],
    values: Vec<PartitionValue>,
    name: &str,
) -> Result<(LogicalPlanBuilder, Vec<Expr>)> {
    if values.is_empty() {
        return Ok((LogicalPlanBuilder::from(input), vec![]));
    }
    // The rows and the values of their partitions read one run of the input.
    let input = shared(input);
    if keys.is_empty() {
        let mut exprs = Vec::with_capacity(values.len());
        for v in values {
            let subquery = LogicalPlanBuilder::from(input.clone())
                .aggregate(Vec::<Expr>::new(), v.aggregates)?
                .project(vec![v.value])?
                .build()?;
            exprs.push(Expr::ScalarSubquery(Subquery {
                subquery: Arc::new(subquery),
                outer_ref_columns: vec![],
                spans: Spans::new(),
            }));
        }
        return Ok((LogicalPlanBuilder::from(input), exprs));
    }
    let partitions = format!("__{name}_partitions");
    let key = |k: usize| format!("__{name}_key_{k}");
    let group: Vec<Expr> = keys
        .iter()
        .enumerate()
        .map(|(k, c)| Expr::Column(c.clone()).alias(key(k)))
        .collect();
    let mut projection: Vec<Expr> = (0..keys.len())
        .map(|k| Expr::Column(Column::from_name(key(k))))
        .collect();
    let mut aggregates = Vec::new();
    let mut exprs = Vec::with_capacity(values.len());
    for (i, v) in values.into_iter().enumerate() {
        aggregates.extend(v.aggregates);
        let alias = format!("__{name}_value_{i}");
        projection.push(v.value.alias(&alias));
        exprs.push(Expr::Column(Column::new(Some(partitions.as_str()), alias)));
    }
    let per_partition = LogicalPlanBuilder::from(input.clone())
        .aggregate(group, aggregates)?
        .project(projection)?
        .alias(partitions.as_str())?
        .build()?;
    let right: Vec<Column> = (0..keys.len())
        .map(|k| Column::new(Some(partitions.as_str()), key(k)))
        .collect();
    let builder = LogicalPlanBuilder::from(input).join_detailed(
        per_partition,
        JoinType::Inner,
        (keys.to_vec(), right),
        None,
        NullEquality::NullEqualsNull,
    )?;
    Ok((builder, exprs))
}

// ---------------------------------------------------------------------------------------
// The rule StructGroupSplit: GROUP BY struct -> GROUP BY its fields, then the struct again
// ---------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct StructGroupSplit;

/// A UDF that makes the struct of a field from one array for each of its fields. The struct
/// has exactly the type and the metadata of the field: unlike `named_struct`, its fields keep
/// their nullability. If the field can be NULL, the first argument is true where the struct
/// is NULL.
pub(crate) fn struct_udf(field: &Field) -> ScalarUDF {
    let DataType::Struct(fields) = field.data_type() else {
        unreachable!("a struct field")
    };
    let nullable = field.is_nullable();
    ScalarUDF::new_from_impl(MakeStruct {
        fields: fields.clone(),
        nullable,
        metadata: field.metadata().clone().into_iter().collect(),
        signature: Signature::any(fields.len() + usize::from(nullable), Volatility::Immutable),
    })
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct MakeStruct {
    fields: Fields,
    nullable: bool,
    metadata: BTreeMap<String, String>,
    signature: Signature,
}

impl ScalarUDFImpl for MakeStruct {
    fn name(&self) -> &str {
        if self.nullable {
            "__make_nullable_struct"
        } else {
            "__make_struct"
        }
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Struct(self.fields.clone()))
    }
    fn return_field_from_args(&self, _: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(Arc::new(
            Field::new(
                self.name(),
                DataType::Struct(self.fields.clone()),
                self.nullable,
            )
            .with_metadata(self.metadata.clone().into_iter().collect()),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let n = args.number_rows;
        let mut arrays = args
            .args
            .iter()
            .map(|a| a.to_array(n))
            .collect::<Result<Vec<_>>>()?;
        let nulls = if self.nullable {
            let is_null = arrays.remove(0);
            let is_null = is_null.as_boolean();
            Some(NullBuffer::from_iter(
                (0..n).map(|i| !(is_null.is_valid(i) && is_null.value(i))),
            ))
        } else {
            None
        };
        Ok(ColumnarValue::Array(Arc::new(StructArray::try_new(
            self.fields.clone(),
            arrays,
            nulls.filter(|n| n.null_count() > 0),
        )?)))
    }
}

/// Whether the fields of a struct key can be group keys of their own: at least one, each with
/// a name that is not empty and no other field has, and none a struct.
fn splittable(fields: &Fields) -> bool {
    let mut names = std::collections::HashSet::new();
    !fields.is_empty()
        && fields.iter().all(|f| {
            !f.name().is_empty()
                && names.insert(f.name())
                && !matches!(f.data_type(), DataType::Struct(_))
        })
}

impl OptimizerRule for StructGroupSplit {
    fn name(&self) -> &str {
        "struct_group_split"
    }

    fn apply_order(&self) -> Option<ApplyOrder> {
        Some(ApplyOrder::BottomUp)
    }

    fn rewrite(
        &self,
        plan: LogicalPlan,
        _config: &dyn OptimizerConfig,
    ) -> Result<Transformed<LogicalPlan>> {
        let LogicalPlan::Aggregate(agg) = &plan else {
            return Ok(Transformed::no(plan));
        };
        if agg
            .group_expr
            .iter()
            .any(|e| matches!(e, Expr::GroupingSet(_)))
        {
            return Ok(Transformed::no(plan));
        }
        let input_schema = agg.input.schema();
        let mut group = Vec::new();
        // For each group key: the fields that replace it and whether it can be NULL, or None
        // if it stays.
        let mut split: Vec<Option<(Fields, bool)>> = Vec::new();
        for (i, e) in agg.group_expr.iter().enumerate() {
            let (_, field) = e.to_field(input_schema)?;
            match field.data_type().clone() {
                // The rule reads the fields by name, and does not split nested structs. A
                // NULL struct and a struct of NULL fields are different keys, so a key that
                // can be NULL also groups by whether it is NULL, and its fields are NULL
                // where it is.
                DataType::Struct(fields) if splittable(&fields) => {
                    let inner = e.clone().unalias();
                    let nullable = field.is_nullable();
                    if nullable {
                        group.push(inner.clone().is_null().alias(format!("__group_{i}_null")));
                    }
                    for (j, f) in fields.iter().enumerate() {
                        let value = get_field(inner.clone(), f.name().as_str());
                        let value = if nullable {
                            when(
                                inner.clone().is_null(),
                                lit(ScalarValue::try_from(f.data_type())?),
                            )
                            .otherwise(value)?
                        } else {
                            value
                        };
                        group.push(value.alias(format!("__group_{i}_{j}")));
                    }
                    split.push(Some((fields, nullable)));
                }
                _ => {
                    group.push(e.clone());
                    split.push(None);
                }
            }
        }
        if split.iter().all(Option::is_none) {
            return Ok(Transformed::no(plan));
        }

        let n_group = group.len();
        let new_agg = LogicalPlan::Aggregate(Aggregate::try_new(
            agg.input.clone(),
            group,
            agg.aggr_expr.clone(),
        )?);
        let new_schema = new_agg.schema().clone();
        let column = |k: usize| Expr::Column(Column::from(new_schema.qualified_field(k)));

        // The projection gives each output column the name, qualifier and type it had.
        let mut exprs = Vec::new();
        let mut k = 0;
        for (i, fields) in split.iter().enumerate() {
            let (q, f) = agg.schema.qualified_field(i);
            let expr = match fields {
                None => {
                    k += 1;
                    column(k - 1)
                }
                Some((fields, nullable)) => {
                    let n = fields.len() + usize::from(*nullable);
                    let args = (k..k + n).map(column).collect();
                    k += n;
                    struct_udf(f).call(args)
                }
            };
            // An alias to the same name confuses push_down_leaf_projections (see below).
            let same =
                matches!(&expr, Expr::Column(c) if c.relation.as_ref() == q && &c.name == f.name());
            exprs.push(if same {
                expr
            } else {
                expr.alias_qualified(q.cloned(), f.name())
            });
        }
        for a in 0..agg.aggr_expr.len() {
            let (q, f) = agg.schema.qualified_field(split.len() + a);
            let (nq, nf) = new_schema.qualified_field(n_group + a);
            // An alias to the same name confuses DataFusion's push_down_leaf_projections,
            // which then keeps both the column and the alias, two fields of one name.
            if nq == q && nf.name() == f.name() {
                exprs.push(column(n_group + a));
            } else {
                exprs.push(column(n_group + a).alias_qualified(q.cloned(), f.name()));
            }
        }
        Ok(Transformed::yes(
            LogicalPlanBuilder::from(new_agg).project(exprs)?.build()?,
        ))
    }
}
