use crate::{Error, ProducerDefinition, ProjectionId, Result};
use datafusion::{arrow::datatypes::DataType, common::ScalarValue};
use std::{
    cmp::Ordering,
    ops::{Bound, RangeBounds},
};

/// A typed comparison for one projected dimension.
#[derive(Clone, Debug)]
pub enum ValueTest {
    Equal(ScalarValue),
    OneOf(Vec<ScalarValue>),
    Range {
        lower: Bound<ScalarValue>,
        upper: Bound<ScalarValue>,
    },
}
impl PartialEq for ValueTest {
    fn eq(&self, other: &Self) -> bool {
        let equal = |a: &ScalarValue, b: &ScalarValue| a.data_type() == b.data_type() && a == b;
        let bound_equal = |a: &Bound<ScalarValue>, b: &Bound<ScalarValue>| match (a, b) {
            (Bound::Included(a), Bound::Included(b)) | (Bound::Excluded(a), Bound::Excluded(b)) => {
                equal(a, b)
            }
            (Bound::Unbounded, Bound::Unbounded) => true,
            _ => false,
        };
        match (self, other) {
            (Self::Equal(a), Self::Equal(b)) => equal(a, b),
            (Self::OneOf(a), Self::OneOf(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
            }
            (Self::Range { lower: a, upper: b }, Self::Range { lower: c, upper: d }) => {
                bound_equal(a, c) && bound_equal(b, d)
            }
            _ => false,
        }
    }
}
impl Eq for ValueTest {}
pub(crate) type Tuple = Vec<(ProjectionId, ValueTest)>;

/// Selected tuples for a producer. Empty values remain active and match no rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionValue {
    pub(crate) tuples: Vec<Tuple>,
}

impl SelectionValue {
    /// Select one correlated tuple. Its terms combine with AND.
    /// Values are validated and normalized when applied to a SelectionSet.
    pub fn tuple(terms: impl IntoIterator<Item = (ProjectionId, ValueTest)>) -> Self {
        Self {
            tuples: vec![terms.into_iter().collect()],
        }
    }

    /// Select correlated tuples combined with OR, each containing terms combined with AND.
    /// An empty iterator selects no rows and remains an active contribution.
    pub fn tuples<I>(tuples: impl IntoIterator<Item = I>) -> Self
    where
        I: IntoIterator<Item = (ProjectionId, ValueTest)>,
    {
        Self {
            tuples: tuples
                .into_iter()
                .map(|terms| terms.into_iter().collect())
                .collect(),
        }
    }
    /// Inspect correlated tuples and their projection/comparison pairs.
    pub fn as_tuples(&self) -> &[Vec<(ProjectionId, ValueTest)>] {
        &self.tuples
    }
}

impl ValueTest {
    /// Match one value, including a typed null or NaN.
    pub fn equal(value: impl Into<ScalarValue>) -> Self {
        Self::Equal(value.into())
    }

    /// Match any of the supplied values. An empty set matches no rows.
    pub fn one_of<T: Into<ScalarValue>>(values: impl IntoIterator<Item = T>) -> Self {
        Self::OneOf(values.into_iter().map(Into::into).collect())
    }

    /// Use Rust range bounds, preserving endpoint inclusion and scalar types.
    /// For a fully unbounded range, specify the type: `ValueTest::range::<i64>(..)`.
    pub fn range<T: Clone + Into<ScalarValue>>(range: impl RangeBounds<T>) -> Self {
        Self::Range {
            lower: range.start_bound().map(|value| value.clone().into()),
            upper: range.end_bound().map(|value| value.clone().into()),
        }
    }
}

pub(crate) fn validate_type(t: &DataType) -> Result<()> {
    if matches!(
        t,
        DataType::Null
            | DataType::Boolean
            | DataType::Int8
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt8
            | DataType::UInt16
            | DataType::UInt32
            | DataType::UInt64
            | DataType::Float16
            | DataType::Float32
            | DataType::Float64
            | DataType::Utf8
            | DataType::LargeUtf8
            | DataType::Utf8View
            | DataType::Binary
            | DataType::LargeBinary
            | DataType::BinaryView
            | DataType::FixedSizeBinary(_)
            | DataType::Date32
            | DataType::Date64
            | DataType::Time32(_)
            | DataType::Time64(_)
            | DataType::Timestamp(_, _)
            | DataType::Duration(_)
            | DataType::Decimal32(_, _)
            | DataType::Decimal64(_, _)
            | DataType::Decimal128(_, _)
            | DataType::Decimal256(_, _)
    ) {
        Ok(())
    } else {
        Err(Error::InvalidValue(format!(
            "unsupported selection value type {t}"
        )))
    }
}

// Only checked flat scalar types reach this ordering. Type order prevents
// timestamp zones or decimal precision from collapsing into the same key.
pub(crate) fn scalar_cmp(a: &ScalarValue, b: &ScalarValue) -> Ordering {
    a.data_type()
        .cmp(&b.data_type())
        .then_with(|| a.partial_cmp(b).expect("checked comparable scalar type"))
}
fn normalize(value: ScalarValue) -> Result<ScalarValue> {
    validate_type(&value.data_type())?;
    Ok(match value {
        ScalarValue::Float16(Some(v)) => ScalarValue::Float16(Some(if v.is_nan() {
            half::f16::NAN
        } else if v == half::f16::ZERO {
            half::f16::ZERO
        } else {
            v
        })),
        ScalarValue::Float32(Some(v)) => ScalarValue::Float32(Some(if v.is_nan() {
            f32::NAN
        } else if v == 0.0 {
            0.0
        } else {
            v
        })),
        ScalarValue::Float64(Some(v)) => ScalarValue::Float64(Some(if v.is_nan() {
            f64::NAN
        } else if v == 0.0 {
            0.0
        } else {
            v
        })),
        v => v,
    })
}
fn valid_bound(value: &ScalarValue) -> bool {
    if value.is_null() {
        return false;
    }
    match value {
        ScalarValue::Float16(Some(v)) => !v.is_nan(),
        ScalarValue::Float32(Some(v)) => !v.is_nan(),
        ScalarValue::Float64(Some(v)) => !v.is_nan(),
        _ => true,
    }
}
pub(crate) fn canonical_values(values: Vec<ScalarValue>) -> Result<Vec<ScalarValue>> {
    let mut values = values
        .into_iter()
        .map(normalize)
        .collect::<Result<Vec<_>>>()?;
    if let Some(first) = values.first() {
        if values.iter().any(|v| v.data_type() != first.data_type()) {
            return Err(Error::InvalidValue(
                "a selected set must use one exact value type".into(),
            ));
        }
    }
    values.sort_by(scalar_cmp);
    values.dedup();
    Ok(values)
}
fn checked_bound(bound: Bound<ScalarValue>) -> Result<Bound<ScalarValue>> {
    let bound = match bound {
        Bound::Included(v) => Bound::Included(normalize(v)?),
        Bound::Excluded(v) => Bound::Excluded(normalize(v)?),
        Bound::Unbounded => Bound::Unbounded,
    };
    if let Bound::Included(v) | Bound::Excluded(v) = &bound {
        if !valid_bound(v) {
            return Err(Error::InvalidValue(
                "range bounds must not be null or NaN".into(),
            ));
        }
    }
    Ok(bound)
}
fn canonical_test(test: ValueTest) -> Result<ValueTest> {
    Ok(match test {
        ValueTest::Equal(v) => ValueTest::Equal(normalize(v)?),
        ValueTest::OneOf(v) => {
            let values = canonical_values(v)?;
            if values.len() == 1 {
                ValueTest::Equal(values.into_iter().next().unwrap())
            } else {
                ValueTest::OneOf(values)
            }
        }
        ValueTest::Range { lower, upper } => {
            let lower = checked_bound(lower)?;
            let upper = checked_bound(upper)?;
            if let (
                Bound::Included(a) | Bound::Excluded(a),
                Bound::Included(b) | Bound::Excluded(b),
            ) = (&lower, &upper)
            {
                if a.data_type() != b.data_type() {
                    return Err(Error::InvalidValue(
                        "range bound types must match exactly".into(),
                    ));
                }
                if scalar_cmp(a, b).is_gt() {
                    return Err(Error::InvalidValue(
                        "range lower bound exceeds upper bound".into(),
                    ));
                }
            }
            ValueTest::Range { lower, upper }
        }
    })
}
fn bound_cmp(a: &Bound<ScalarValue>, b: &Bound<ScalarValue>) -> Ordering {
    fn tag(b: &Bound<ScalarValue>) -> u8 {
        match b {
            Bound::Unbounded => 0,
            Bound::Included(_) => 1,
            Bound::Excluded(_) => 2,
        }
    }
    tag(a).cmp(&tag(b)).then_with(|| match (a, b) {
        (Bound::Included(a) | Bound::Excluded(a), Bound::Included(b) | Bound::Excluded(b)) => {
            scalar_cmp(a, b)
        }
        _ => Ordering::Equal,
    })
}
fn test_cmp(a: &ValueTest, b: &ValueTest) -> Ordering {
    fn tag(t: &ValueTest) -> u8 {
        match t {
            ValueTest::Equal(_) => 0,
            ValueTest::OneOf(_) => 1,
            ValueTest::Range { .. } => 2,
        }
    }
    tag(a).cmp(&tag(b)).then_with(|| match (a, b) {
        (ValueTest::Equal(a), ValueTest::Equal(b)) => scalar_cmp(a, b),
        (ValueTest::OneOf(a), ValueTest::OneOf(b)) => a
            .iter()
            .zip(b)
            .map(|(a, b)| scalar_cmp(a, b))
            .find(|c| !c.is_eq())
            .unwrap_or_else(|| a.len().cmp(&b.len())),
        (ValueTest::Range { lower: a, upper: b }, ValueTest::Range { lower: c, upper: d }) => {
            bound_cmp(a, c).then_with(|| bound_cmp(b, d))
        }
        _ => Ordering::Equal,
    })
}
pub(crate) fn tuple_cmp(a: &Tuple, b: &Tuple) -> Ordering {
    a.iter()
        .zip(b)
        .map(|((a_id, a_test), (b_id, b_test))| {
            a_id.cmp(b_id).then_with(|| test_cmp(a_test, b_test))
        })
        .find(|c| !c.is_eq())
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}
pub(crate) fn canonical_tuples(
    producer: &ProducerDefinition,
    tuples: Vec<Tuple>,
) -> Result<Vec<Tuple>> {
    let mut tuples = tuples
        .into_iter()
        .map(|tuple| {
            let mut terms = tuple
                .into_iter()
                .map(|(id, test)| Ok((id, canonical_test(test)?)))
                .collect::<Result<Vec<_>>>()?;
            terms.sort_by(|a, b| a.0.cmp(&b.0));
            if terms.len() != producer.projections().len()
                || terms
                    .iter()
                    .zip(producer.projections())
                    .any(|((id, _), p)| id != p.id())
            {
                return Err(Error::InvalidValue(
                    "each tuple must have exactly one term for every producer projection".into(),
                ));
            }
            Ok(terms)
        })
        .collect::<Result<Vec<_>>>()?;
    tuples.sort_by(tuple_cmp);
    tuples.dedup();
    Ok(tuples)
}
pub(crate) fn canonical_value(
    producer: &ProducerDefinition,
    value: SelectionValue,
) -> Result<SelectionValue> {
    Ok(SelectionValue {
        tuples: canonical_tuples(producer, value.tuples)?,
    })
}

pub(crate) fn same_meaning(
    a: &Tuple,
    ap: &ProducerDefinition,
    b: &Tuple,
    bp: &ProducerDefinition,
) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut matched = vec![false; b.len()];
    for ((_, test), projection) in a.iter().zip(ap.projections()) {
        let index = b.iter().zip(bp.projections()).enumerate().position(
            |(index, ((_, other_test), op))| {
                !matched[index]
                    && projection.same_meaning(op)
                    && ap.pixel_grid(projection.id()) == bp.pixel_grid(op.id())
                    && test == other_test
            },
        );
        match index {
            Some(index) => matched[index] = true,
            None => return false,
        }
    }
    true
}
