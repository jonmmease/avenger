use std::cmp::Ordering;
use std::slice;

use arrow::array::{Array, ArrayRef, UInt32Array};
use arrow::compute::{cast, concat, take};
use arrow::datatypes::DataType;
use arrow::error::ArrowError;
use arrow::row::{Row, RowConverter, Rows, SortField};

use super::{CategoryOrder, DomainFitError, Extents};

/// The support of a discrete domain: distinct categories, each with the largest pixel extents of
/// the marks drawn at it. See the [module docs](super).
#[derive(Debug, Clone)]
pub struct UniqueSupport {
    /// Distinct non-null categories, ascending in Arrow's row encoding. Struct categories, the
    /// paths of nested bands, compare level by level.
    keys: ArrayRef,
    /// Each category's largest lower extent.
    lower: Vec<f64>,
    /// Each category's largest upper extent.
    upper: Vec<f64>,
}

impl PartialEq for UniqueSupport {
    fn eq(&self, other: &Self) -> bool {
        self.keys.to_data() == other.keys.to_data()
            && self.lower == other.lower
            && self.upper == other.upper
    }
}

impl UniqueSupport {
    /// The distinct categories of `values` and their largest extents. Null categories are skipped,
    /// and dictionary-encoded categories take their values' type.
    pub fn new(values: &ArrayRef, extents: Extents) -> Result<Self, DomainFitError> {
        let (lower, upper) = extents.per_row(values.len())?;
        Self::from_parts(plain(values)?, lower, upper)
    }

    /// The categories of both supports, each with its largest extents. A support without
    /// categories changes nothing, and supports of different category types do not combine.
    pub fn union(&self, other: &Self) -> Result<Self, DomainFitError> {
        if self.keys.is_empty() {
            return Ok(other.clone());
        }
        if other.keys.is_empty() {
            return Ok(self.clone());
        }
        if self.keys.data_type() != other.keys.data_type() {
            return Err(DomainFitError::InvalidInput(format!(
                "categories of types {} and {} do not combine",
                self.keys.data_type(),
                other.keys.data_type()
            )));
        }
        let keys = concat(&[self.keys.as_ref(), other.keys.as_ref()]).map_err(invalid)?;
        Self::from_parts(
            keys,
            [self.lower.as_slice(), &other.lower].concat(),
            [self.upper.as_slice(), &other.upper].concat(),
        )
    }

    /// The categories in `order`.
    pub fn domain(&self, order: &CategoryOrder) -> Result<ArrayRef, DomainFitError> {
        match order {
            CategoryOrder::Ascending => Ok(self.keys.clone()),
            CategoryOrder::Descending => {
                let reversed = UInt32Array::from_iter_values((0..self.keys.len() as u32).rev());
                take(&self.keys, &reversed, None).map_err(invalid)
            }
            CategoryOrder::Explicit(keys) => plain(keys),
        }
    }

    /// The largest lower and upper extents at each of `domain`'s categories, with zero for
    /// categories that have no data.
    pub fn extents_at(&self, domain: &ArrayRef) -> Result<(Vec<f64>, Vec<f64>), DomainFitError> {
        let mut lower = vec![0.0; domain.len()];
        let mut upper = vec![0.0; domain.len()];
        if self.keys.is_empty() || domain.is_empty() {
            return Ok((lower, upper));
        }
        let domain = plain(domain)?;
        if domain.data_type() != self.keys.data_type() {
            return Err(DomainFitError::InvalidInput(format!(
                "a domain of type {} has no extents in categories of type {}",
                domain.data_type(),
                self.keys.data_type()
            )));
        }
        let converter = converter(self.keys.data_type())?;
        let keys = rows(&converter, &self.keys)?;
        let domain_rows = rows(&converter, &domain)?;
        for i in 0..domain.len() {
            if domain.is_null(i) {
                continue;
            }
            if let Some(index) = find(&keys, domain_rows.row(i)) {
                lower[i] = self.lower[index];
                upper[i] = self.upper[index];
            }
        }
        Ok((lower, upper))
    }

    /// Sorts and deduplicates categories, keeping each one's largest extents.
    fn from_parts(
        keys: ArrayRef,
        lower: Vec<f64>,
        upper: Vec<f64>,
    ) -> Result<Self, DomainFitError> {
        let converter = converter(keys.data_type())?;
        let rows = rows(&converter, &keys)?;
        let mut order: Vec<usize> = (0..keys.len()).filter(|&i| keys.is_valid(i)).collect();
        order.sort_by(|&i, &j| rows.row(i).cmp(&rows.row(j)));
        let (mut kept, mut kept_lower, mut kept_upper) = (Vec::new(), Vec::new(), Vec::new());
        for i in order {
            match kept.last() {
                Some(&last) if rows.row(last as usize) == rows.row(i) => {
                    let end = kept_lower.len() - 1;
                    kept_lower[end] = f64::max(kept_lower[end], lower[i]);
                    kept_upper[end] = f64::max(kept_upper[end], upper[i]);
                }
                _ => {
                    kept.push(i as u32);
                    kept_lower.push(lower[i]);
                    kept_upper.push(upper[i]);
                }
            }
        }
        Ok(Self {
            keys: take(&keys, &UInt32Array::from(kept), None).map_err(invalid)?,
            lower: kept_lower,
            upper: kept_upper,
        })
    }
}

/// Categories with dictionary encoding replaced by their values' type, so that encoded and plain
/// categories combine.
fn plain(keys: &ArrayRef) -> Result<ArrayRef, DomainFitError> {
    match keys.data_type() {
        DataType::Dictionary(_, value_type) => cast(keys, value_type).map_err(invalid),
        _ => Ok(keys.clone()),
    }
}

fn converter(data_type: &DataType) -> Result<RowConverter, DomainFitError> {
    RowConverter::new(vec![SortField::new(data_type.clone())]).map_err(invalid)
}

fn rows(converter: &RowConverter, keys: &ArrayRef) -> Result<Rows, DomainFitError> {
    converter
        .convert_columns(slice::from_ref(keys))
        .map_err(invalid)
}

/// The index of `row` in ascending `rows`.
fn find(rows: &Rows, row: Row) -> Option<usize> {
    let (mut low, mut high) = (0, rows.num_rows());
    while low < high {
        let middle = (low + high) / 2;
        match rows.row(middle).cmp(&row) {
            Ordering::Less => low = middle + 1,
            Ordering::Greater => high = middle,
            Ordering::Equal => return Some(middle),
        }
    }
    None
}

fn invalid(error: ArrowError) -> DomainFitError {
    DomainFitError::InvalidInput(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{AsArray, Int32Array, StringArray, StructArray};
    use arrow::datatypes::{Field, Fields};

    use super::*;

    fn strings(values: &[Option<&str>]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    fn names(array: &ArrayRef) -> Vec<&str> {
        array
            .as_string::<i32>()
            .iter()
            .map(Option::unwrap)
            .collect()
    }

    #[test]
    fn categories_are_distinct_ascending_and_skip_nulls() {
        let values = strings(&[Some("b"), None, Some("a"), Some("b"), Some("c")]);
        let support = UniqueSupport::new(&values, Extents::None).unwrap();
        assert_eq!(
            names(&support.domain(&CategoryOrder::Ascending).unwrap()),
            ["a", "b", "c"]
        );
        assert_eq!(
            names(&support.domain(&CategoryOrder::Descending).unwrap()),
            ["c", "b", "a"]
        );
    }

    /// Building from all the data equals the union of supports built from its parts, in either
    /// order, including each category's largest extents.
    #[test]
    fn union_matches_one_support_of_all_the_data() {
        let values = strings(&[
            Some("x"),
            Some("y"),
            Some("x"),
            Some("z"),
            Some("y"),
            Some("w"),
        ]);
        let lower = [1.0, 5.0, 3.0, 0.0, 2.0, 4.0];
        let upper = [6.0, 1.0, 2.0, 7.0, 9.0, 0.0];
        let all = UniqueSupport::new(
            &values,
            Extents::PerRow {
                lower: &lower,
                upper: &upper,
            },
        )
        .unwrap();
        for split in 1..values.len() {
            let part = |range: std::ops::Range<usize>| {
                UniqueSupport::new(
                    &values.slice(range.start, range.len()),
                    Extents::PerRow {
                        lower: &lower[range.clone()],
                        upper: &upper[range],
                    },
                )
                .unwrap()
            };
            let (first, second) = (part(0..split), part(split..values.len()));
            assert_eq!(first.union(&second).unwrap(), all);
            assert_eq!(second.union(&first).unwrap(), all);
        }
        assert_eq!(
            names(&all.domain(&CategoryOrder::Ascending).unwrap()),
            ["w", "x", "y", "z"]
        );
        assert_eq!(all.lower, [4.0, 3.0, 5.0, 0.0]);
        assert_eq!(all.upper, [0.0, 6.0, 9.0, 7.0]);
    }

    #[test]
    fn numbers_sort_by_value() {
        let values: ArrayRef = Arc::new(Int32Array::from(vec![10, -2, 3, 10]));
        let support = UniqueSupport::new(&values, Extents::None).unwrap();
        assert_eq!(
            support
                .keys
                .as_primitive::<arrow::datatypes::Int32Type>()
                .values(),
            &[-2, 3, 10]
        );
    }

    /// Nested paths sort level by level, so each level sorts within its parent.
    #[test]
    fn nested_paths_sort_within_their_parents() {
        let fields = Fields::from(vec![
            Field::new("outer", DataType::Utf8, false),
            Field::new("inner", DataType::Utf8, false),
        ]);
        let paths: ArrayRef = Arc::new(StructArray::new(
            fields,
            vec![
                strings(&[Some("b"), Some("a"), Some("b"), Some("a")]),
                strings(&[Some("y"), Some("z"), Some("x"), Some("y")]),
            ],
            None,
        ));
        let support = UniqueSupport::new(&paths, Extents::None).unwrap();
        let path = |domain: &ArrayRef, i: usize| {
            let parts = domain.as_struct();
            (
                parts.column(0).as_string::<i32>().value(i).to_string(),
                parts.column(1).as_string::<i32>().value(i).to_string(),
            )
        };
        let ascending: Vec<_> = (0..4)
            .map(|i| path(&support.domain(&CategoryOrder::Ascending).unwrap(), i))
            .collect();
        let descending = support.domain(&CategoryOrder::Descending).unwrap();
        let descending: Vec<_> = (0..4).map(|i| path(&descending, i)).collect();
        let pairs = |items: &[(&str, &str)]| {
            items
                .iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ascending,
            pairs(&[("a", "y"), ("a", "z"), ("b", "x"), ("b", "y")])
        );
        assert_eq!(
            descending,
            pairs(&[("b", "y"), ("b", "x"), ("a", "z"), ("a", "y")])
        );
    }

    /// An explicit order is the domain as listed: listed categories without data have no extents,
    /// and unlisted categories are left out.
    #[test]
    fn an_explicit_order_is_the_domain_as_listed() {
        let values = strings(&[Some("a"), Some("b")]);
        let support = UniqueSupport::new(
            &values,
            Extents::Constant {
                lower: 3.0,
                upper: 4.0,
            },
        )
        .unwrap();
        let listed = strings(&[Some("c"), Some("a")]);
        let domain = support.domain(&CategoryOrder::Explicit(listed)).unwrap();
        assert_eq!(names(&domain), ["c", "a"]);
        assert_eq!(
            support.extents_at(&domain).unwrap(),
            (vec![0.0, 3.0], vec![0.0, 4.0])
        );
    }

    #[test]
    fn dictionary_categories_combine_with_plain_ones() {
        let encoded: ArrayRef = Arc::new(
            vec!["b", "a"]
                .into_iter()
                .collect::<arrow::array::DictionaryArray<arrow::datatypes::Int32Type>>(),
        );
        let plain = strings(&[Some("c")]);
        let union = UniqueSupport::new(&encoded, Extents::None)
            .unwrap()
            .union(&UniqueSupport::new(&plain, Extents::None).unwrap())
            .unwrap();
        assert_eq!(
            names(&union.domain(&CategoryOrder::Ascending).unwrap()),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn mismatched_categories_and_extents_are_invalid() {
        let words = UniqueSupport::new(&strings(&[Some("a")]), Extents::None).unwrap();
        let numbers: ArrayRef = Arc::new(Int32Array::from(vec![1]));
        let numbers = UniqueSupport::new(&numbers, Extents::None).unwrap();
        assert!(matches!(
            words.union(&numbers),
            Err(DomainFitError::InvalidInput(_))
        ));
        let empty = UniqueSupport::new(&strings(&[]), Extents::None).unwrap();
        assert_eq!(empty.union(&numbers).unwrap(), numbers);
        assert!(matches!(
            UniqueSupport::new(
                &strings(&[Some("a")]),
                Extents::PerRow {
                    lower: &[1.0, 2.0],
                    upper: &[1.0]
                }
            ),
            Err(DomainFitError::InvalidInput(_))
        ));
        assert!(matches!(
            UniqueSupport::new(
                &strings(&[Some("a")]),
                Extents::Constant {
                    lower: -1.0,
                    upper: 0.0
                }
            ),
            Err(DomainFitError::InvalidInput(_))
        ));
    }
}
