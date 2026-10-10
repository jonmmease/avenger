use serde::de;
use serde::{Deserialize, Deserializer, Serialize};

use crate::presence::present;
use crate::{Bin, BinOutput};

/// Supported aggregate spellings. `mean` and `average` retain distinct representations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregateOp {
    Count,
    Valid,
    Missing,
    Sum,
    Min,
    Max,
    Mean,
    Average,
    Variance,
    Variancep,
    Stdev,
    Stdevp,
}

/// One aggregate output. All operations except count require a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregatedFieldDef {
    pub op: AggregateOp,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub field: Option<String>,
    #[serde(rename = "as")]
    pub as_: String,
}

/// Explicit aggregate measures and optional grouping fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregateTransform {
    pub aggregate: Vec<AggregatedFieldDef>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    pub groupby: Option<Vec<String>>,
}

/// An explicit bin transform. Validation accepts only true or a parameter object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BinTransform {
    pub bin: Bin,
    pub field: String,
    #[serde(rename = "as")]
    pub as_: BinOutput,
}

/// An authored transform, applied in list order.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Transform {
    Bin(BinTransform),
    Aggregate(AggregateTransform),
    Filter(FilterTransform),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformFields {
    #[serde(default, deserialize_with = "present")]
    filter: Option<FieldPredicate>,
    #[serde(default, deserialize_with = "present")]
    bin: Option<Bin>,
    #[serde(default, deserialize_with = "present")]
    field: Option<String>,
    #[serde(rename = "as", default, deserialize_with = "present")]
    as_: Option<BinOutput>,
    #[serde(default, deserialize_with = "present")]
    aggregate: Option<Vec<AggregatedFieldDef>>,
    #[serde(default, deserialize_with = "present")]
    groupby: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for Transform {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = TransformFields::deserialize(deserializer)?;
        if let Some(filter) = fields.filter {
            if fields.bin.is_some()
                || fields.aggregate.is_some()
                || fields.field.is_some()
                || fields.as_.is_some()
                || fields.groupby.is_some()
            {
                return Err(de::Error::custom(
                    "filter cannot be combined with another transform",
                ));
            }
            return Ok(Self::Filter(FilterTransform { filter }));
        }
        match (fields.bin, fields.aggregate) {
            (Some(_), Some(_)) => Err(de::Error::custom(
                "a transform cannot contain both bin and aggregate",
            )),
            (Some(bin), None) => {
                if fields.groupby.is_some() {
                    return Err(de::Error::custom(
                        "groupby is not supported on a bin transform",
                    ));
                }
                Ok(Self::Bin(BinTransform {
                    bin,
                    field: fields
                        .field
                        .ok_or_else(|| de::Error::missing_field("field"))?,
                    as_: fields.as_.ok_or_else(|| de::Error::missing_field("as"))?,
                }))
            }
            (None, Some(aggregate)) => {
                if fields.field.is_some() || fields.as_.is_some() {
                    return Err(de::Error::custom(
                        "field and as belong inside aggregate measures",
                    ));
                }
                Ok(Self::Aggregate(AggregateTransform {
                    aggregate,
                    groupby: fields.groupby,
                }))
            }
            (None, None) => Err(de::Error::custom(
                "expected a bin, aggregate, or filter transform",
            )),
        }
    }
}

/// A structured numeric field comparison in authored transform order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilterTransform {
    pub filter: FieldPredicate,
}

/// A greater-than-or-equal field predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldPredicate {
    pub field: String,
    pub gte: PredicateOperand,
}

/// A numeric literal or an expression reference resolved by the compiler.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PredicateOperand {
    Number(f64),
    Expr(ExpressionReference),
}

/// Authored expression text. The initial compiler accepts one parameter name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpressionReference {
    pub expr: String,
}
