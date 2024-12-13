use std::fmt::Debug;
use std::hash::Hash;

use crate::error::AvengerScaleError;
use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use indexmap::IndexMap;

/// A discrete scale that maps input values to a fixed set of output values.
/// Supports default values for inputs not found in the domain.
#[derive(Debug, Clone)]
pub struct OrdinalScale<D, R>
where
    D: Clone + Hash + Eq + Debug + Sync + 'static,
    R: Clone + Debug + Sync + 'static,
{
    mapping: IndexMap<D, R>,
    default_value: R,
}

impl<D, R> OrdinalScale<D, R>
where
    D: Clone + Hash + Eq + Debug + Sync + 'static,
    R: Clone + Debug + Sync + 'static,
{
    /// Creates a new ordinal scale from domain and range arrays with a required default value
    pub fn new(domain: &[D], range: &[R], default_value: R) -> Result<Self, AvengerScaleError> {
        if domain.len() != range.len() {
            return Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: domain.len(),
                range_len: range.len(),
            });
        }

        // Create a mapping from domain values to range values
        let mapping = domain
            .iter()
            .cloned()
            .zip(range.iter().cloned())
            .collect::<IndexMap<_, _>>();

        Ok(Self {
            mapping,
            default_value,
        })
    }

    /// Returns the current default value
    pub fn get_default_value(&self) -> &R {
        &self.default_value
    }

    /// Returns the current domain as a vector
    pub fn domain(&self) -> Vec<D> {
        self.mapping.keys().cloned().collect()
    }

    /// Returns the current range as a vector
    pub fn range(&self) -> Vec<R> {
        self.mapping.values().cloned().collect()
    }

    /// Maps input values to their corresponding range values using the ordinal mapping
    pub fn scale<'a>(&self, values: impl Into<ScalarOrArrayRef<'a, D>>) -> ScalarOrArray<R> {
        values
            .into()
            .map(|v| self.mapping.get(v).unwrap_or(&self.default_value).clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_ordinal_scale() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = vec!["a", "b", "c"];
        let range = vec!["red", "green", "blue"];

        // Create scale with a default value
        let scale = OrdinalScale::new(&domain, &range, "gray")?;

        // Create input values to scale
        let values = vec!["b", "a", "d", "b", "d"];

        // Apply scale
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(result, vec!["green", "red", "gray", "green", "gray"]);

        Ok(())
    }

    #[test]
    fn test_simple_ordinal_scale_optional() -> Result<(), AvengerScaleError> {
        // Create domain and range arrays
        let domain = vec!["a", "b", "c"];

        // If there is no default value, then range elements must be optional
        let range = vec![Some("red"), Some("green"), Some("blue")];

        // Create scale with a None default value
        let scale = OrdinalScale::new(&domain, &range, None)?;

        // Create input values to scale
        let values = vec!["b", "a", "d", "b", "d"];

        // Apply scale
        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(
            result,
            vec![Some("green"), Some("red"), None, Some("green"), None]
        );

        Ok(())
    }

    #[test]
    fn test_domain_range_mismatch() {
        let domain = vec![1, 2, 3];
        let range = vec!["a", "b"];

        assert!(matches!(
            OrdinalScale::new(&domain, &range, "default"),
            Err(AvengerScaleError::DomainRangeMismatch {
                domain_len: 3,
                range_len: 2,
            })
        ));
    }

    #[test]
    fn test_custom_types() -> Result<(), AvengerScaleError> {
        #[derive(Debug, Clone, Hash, Eq, PartialEq)]
        struct CustomDomain(String);

        #[derive(Debug, Clone, PartialEq)]
        struct CustomRange(i32);

        let domain = vec![
            CustomDomain("a".into()),
            CustomDomain("b".into()),
            CustomDomain("c".into()),
        ];
        let range = vec![CustomRange(1), CustomRange(2), CustomRange(3)];

        let scale = OrdinalScale::new(&domain, &range, CustomRange(0))?;
        let values = vec![
            CustomDomain("b".into()),
            CustomDomain("a".into()),
            CustomDomain("d".into()),
        ];

        let result = scale.scale(&values).as_vec(values.len(), None);
        assert_eq!(result, vec![CustomRange(2), CustomRange(1), CustomRange(0)]);

        Ok(())
    }
}
