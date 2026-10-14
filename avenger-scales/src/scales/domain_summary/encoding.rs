//! The Arrow form of summaries, in which queries return them.

use std::sync::Arc;

use arrow::array::{
    new_empty_array, Array, ArrayRef, AsArray, Float64Array, ListArray, StructArray,
};
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{Field, Fields, Float64Type};

use super::{
    CategorySummary, DomainFitError, DomainSummary, Extents, IntervalSummary, SampleSummary,
};

const INTERVAL_FIELDS: [&str; 4] = [
    "lower_values",
    "lower_extents",
    "upper_values",
    "upper_extents",
];
const CATEGORY_FIELDS: [&str; 3] = ["keys", "lower_extents", "upper_extents"];
const SAMPLE_FIELDS: [&str; 1] = ["values"];

impl DomainSummary {
    /// The summary as a one-row struct, the form in which queries return it. Each field is a list.
    /// An interval summary has the markers on its two hull chains: `lower_values` and
    /// `lower_extents`, and `upper_values` and `upper_extents`. A category summary has `keys`, its
    /// distinct categories, with their largest `lower_extents` and `upper_extents`. A sample
    /// summary has its `values`.
    ///
    /// The struct's type depends only on the kind of summary and the type of its categories, so the
    /// summary of no values gives the type of every summary that a scale makes from values of a
    /// type.
    pub fn to_arrow(&self) -> StructArray {
        match self {
            DomainSummary::Interval(summary) => {
                let (lower, upper) = summary.chains();
                let values = |chain: &[(f64, f64)]| f64_list(chain.iter().map(|marker| marker.0));
                let extents = |chain: &[(f64, f64)]| f64_list(chain.iter().map(|marker| marker.1));
                struct_of(
                    &INTERVAL_FIELDS,
                    vec![values(lower), extents(lower), values(upper), extents(upper)],
                )
            }
            DomainSummary::Categories(summary) => {
                let (keys, lower, upper) = summary.parts();
                struct_of(
                    &CATEGORY_FIELDS,
                    vec![
                        list(keys.clone()),
                        f64_list(lower.iter().copied()),
                        f64_list(upper.iter().copied()),
                    ],
                )
            }
            DomainSummary::Sample(summary) => struct_of(
                &SAMPLE_FIELDS,
                vec![f64_list(summary.values().iter().copied())],
            ),
        }
    }

    /// The summary at `row` of `array`, a struct in the form that [`to_arrow`](Self::to_arrow)
    /// gives. A null row is the summary of no values. Decoding rebuilds the summary from its
    /// markers or categories, so it rejects values and extents that a summary cannot hold, and the
    /// categories need not be distinct or sorted.
    pub fn from_arrow(array: &dyn Array, row: usize) -> Result<Self, DomainFitError> {
        let array = array
            .as_struct_opt()
            .ok_or_else(|| invalid("a summary is a struct".into()))?;
        if row >= array.len() {
            return Err(invalid(format!(
                "row {row} is past the {} summaries",
                array.len()
            )));
        }
        let lists = (0..array.num_columns())
            .map(|index| {
                let column = array.column(index).as_list_opt::<i32>().ok_or_else(|| {
                    invalid(format!(
                        "summary field {} is not a list",
                        array.fields()[index].name()
                    ))
                })?;
                Ok(if array.is_null(row) || column.is_null(row) {
                    new_empty_array(&column.value_type())
                } else {
                    column.value(row)
                })
            })
            .collect::<Result<Vec<ArrayRef>, DomainFitError>>()?;
        let names: Vec<&str> = array.fields().iter().map(|f| f.name().as_str()).collect();
        if names == INTERVAL_FIELDS {
            let [lower_values, lower_extents, upper_values, upper_extents] =
                [&lists[0], &lists[1], &lists[2], &lists[3]].map(f64_values);
            let (lower_values, upper_values) = (lower_values?, upper_values?);
            // Each chain's markers reach no farther on the other side, and the chains of these
            // markers are the encoded chains.
            let values = [lower_values.as_slice(), &upper_values].concat();
            let lower = [lower_extents?, vec![0.0; upper_values.len()]].concat();
            let upper = [vec![0.0; lower_values.len()], upper_extents?].concat();
            Ok(DomainSummary::Interval(IntervalSummary::new(
                &values, &lower, &upper,
            )?))
        } else if names == CATEGORY_FIELDS {
            let (lower, upper) = (f64_values(&lists[1])?, f64_values(&lists[2])?);
            let extents = Extents::PerRow {
                lower: &lower,
                upper: &upper,
            };
            Ok(DomainSummary::Categories(CategorySummary::new(
                &lists[0], extents,
            )?))
        } else if names == SAMPLE_FIELDS {
            Ok(DomainSummary::Sample(SampleSummary::new(&f64_values(
                &lists[0],
            )?)?))
        } else {
            Err(invalid(format!("no kind of summary has fields {names:?}")))
        }
    }
}

/// `values` as one list.
fn list(values: ArrayRef) -> ArrayRef {
    let field = Arc::new(Field::new_list_field(values.data_type().clone(), true));
    let offsets = OffsetBuffer::from_lengths([values.len()]);
    Arc::new(ListArray::new(field, offsets, values, None))
}

fn f64_list(values: impl Iterator<Item = f64>) -> ArrayRef {
    list(Arc::new(Float64Array::from_iter_values(values)))
}

fn struct_of(names: &[&str], columns: Vec<ArrayRef>) -> StructArray {
    let fields: Fields = names
        .iter()
        .zip(&columns)
        .map(|(name, column)| Field::new(*name, column.data_type().clone(), true))
        .collect();
    StructArray::new(fields, columns, None)
}

fn f64_values(array: &ArrayRef) -> Result<Vec<f64>, DomainFitError> {
    match array.as_primitive_opt::<Float64Type>() {
        Some(values) if values.null_count() == 0 => Ok(values.values().to_vec()),
        _ => Err(invalid(format!(
            "summary values and extents are non-null f64 values, not {}",
            array.data_type()
        ))),
    }
}

fn invalid(message: String) -> DomainFitError {
    DomainFitError::InvalidInput(message)
}

#[cfg(test)]
mod tests {
    use arrow::array::{Int32Array, StringArray};
    use arrow::buffer::NullBuffer;
    use arrow::compute::concat;
    use arrow::datatypes::DataType;

    use super::*;

    fn round_trip(summary: &DomainSummary) -> DomainSummary {
        DomainSummary::from_arrow(&summary.to_arrow(), 0).unwrap()
    }

    fn strings(values: &[&str]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    #[test]
    fn every_kind_of_summary_round_trips() {
        let interval = IntervalSummary::new(
            &[0.0, 1.0, 2.5, 4.0, 7.0],
            &[3.0, 9.0, 1.0, 12.0, 0.0],
            &[2.0, 0.0, 8.0, 5.0, 1.0],
        )
        .unwrap();
        let categories = CategorySummary::new(
            &strings(&["b", "a", "b", "c"]),
            Extents::PerRow {
                lower: &[1.0, 2.0, 3.0, 4.0],
                upper: &[4.0, 3.0, 2.0, 1.0],
            },
        )
        .unwrap();
        let paths: ArrayRef = Arc::new(StructArray::new(
            Fields::from(vec![
                Field::new("group", DataType::Utf8, true),
                Field::new("item", DataType::Int32, true),
            ]),
            vec![
                strings(&["x", "x", "w"]),
                Arc::new(Int32Array::from(vec![2, 1, 2])),
            ],
            None,
        ));
        let nested = CategorySummary::new(&paths, Extents::None).unwrap();
        let sample = SampleSummary::new(&[3.0, 1.0, 2.0]).unwrap();
        for summary in [
            DomainSummary::Interval(interval),
            DomainSummary::Categories(categories),
            DomainSummary::Categories(nested),
            DomainSummary::Sample(sample),
            DomainSummary::Interval(IntervalSummary::new(&[], &[], &[]).unwrap()),
            DomainSummary::Categories(CategorySummary::new(&strings(&[]), Extents::None).unwrap()),
        ] {
            assert_eq!(round_trip(&summary), summary);
        }
    }

    #[test]
    fn rows_decode_separately_and_null_rows_have_no_values() {
        let summary = |values: &[f64]| DomainSummary::Sample(SampleSummary::new(values).unwrap());
        let (a, b) = (summary(&[1.0, 2.0]), summary(&[5.0]));
        let rows = concat(&[&a.to_arrow(), &b.to_arrow()]).unwrap();
        assert_eq!(DomainSummary::from_arrow(&rows, 0).unwrap(), a);
        assert_eq!(DomainSummary::from_arrow(&rows, 1).unwrap(), b);
        let rows = rows.as_struct();
        let with_null = StructArray::new(
            rows.fields().clone(),
            rows.columns().to_vec(),
            Some(NullBuffer::from(vec![true, false])),
        );
        assert_eq!(
            DomainSummary::from_arrow(&with_null, 1).unwrap(),
            summary(&[])
        );
        assert!(DomainSummary::from_arrow(&with_null, 2).is_err());
    }

    #[test]
    fn arrays_that_are_not_summaries_are_invalid() {
        let invalid = |array: &dyn Array| {
            matches!(
                DomainSummary::from_arrow(array, 0),
                Err(DomainFitError::InvalidInput(_))
            )
        };
        assert!(invalid(&Float64Array::from(vec![1.0])));
        assert!(invalid(&struct_of(
            &["other"],
            vec![f64_list([1.0].into_iter())]
        )));
        let with_null = list(Arc::new(Float64Array::from(vec![Some(1.0), None])));
        assert!(invalid(&struct_of(&SAMPLE_FIELDS, vec![with_null])));
        let interval = |lower_extents: &[f64]| {
            struct_of(
                &INTERVAL_FIELDS,
                vec![
                    f64_list([0.0, 1.0].into_iter()),
                    f64_list(lower_extents.iter().copied()),
                    f64_list([1.0].into_iter()),
                    f64_list([0.0].into_iter()),
                ],
            )
        };
        assert!(!invalid(&interval(&[1.0, 2.0])));
        assert!(invalid(&interval(&[1.0, -2.0])));
        assert!(invalid(&interval(&[1.0])));
    }
}
