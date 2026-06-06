use crate::{AvengerChartError, CoordinationScope};

/// Scale-domain coordination target for a channel.
///
/// The scope chooses the logical owner path, while the group chooses the
/// semantic bucket within that owner. The default group uses the resolved scale
/// name, preserving ordinary x/y scale-domain behavior.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DomainCoordination {
    pub scope: CoordinationScope,
    pub group: DomainCoordinationGroup,
}

impl DomainCoordination {
    pub fn new(scope: CoordinationScope, group: DomainCoordinationGroup) -> Self {
        Self {
            scope: scope.to_normalized(),
            group,
        }
    }

    pub fn scale_name(scope: CoordinationScope) -> Self {
        Self::new(scope, DomainCoordinationGroup::ScaleName)
    }

    pub fn named(
        scope: CoordinationScope,
        group: impl Into<String>,
    ) -> Result<Self, AvengerChartError> {
        Ok(Self::new(
            scope,
            DomainCoordinationGroup::named(group.into())?,
        ))
    }

    pub fn with_scope(mut self, scope: CoordinationScope) -> Self {
        self.scope = scope.to_normalized();
        self
    }

    pub fn with_group(mut self, group: DomainCoordinationGroup) -> Self {
        self.group = group;
        self
    }
}

impl Default for DomainCoordination {
    fn default() -> Self {
        Self::scale_name(CoordinationScope::Free)
    }
}

/// Semantic group for scale-domain coordination.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DomainCoordinationGroup {
    /// Use the resolved scale name as the coordination group.
    ScaleName,
    /// Use an explicit author-provided semantic group id.
    Named(String),
}

impl DomainCoordinationGroup {
    pub fn named(group: impl Into<String>) -> Result<Self, AvengerChartError> {
        let group = group.into();
        validate_domain_group_id(&group)?;
        Ok(Self::Named(group))
    }
}

/// Validate a public domain-group id.
///
/// The rules intentionally match structural/tool-style ids: non-empty ASCII
/// identifier-ish strings with no periods. Periods are reserved for future
/// namespacing.
pub fn validate_domain_group_id(id: &str) -> Result<(), AvengerChartError> {
    if id.is_empty() {
        return Err(AvengerChartError::InvalidArgument(
            "Domain group id cannot be empty".to_string(),
        ));
    }
    if id.contains('.') {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Domain group id '{id}' cannot contain periods"
        )));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Domain group id '{id}' must contain only ASCII letters, numbers, underscores, or hyphens"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_coordination_serializes() {
        let coordination = DomainCoordination::named(CoordinationScope::Shared, "height").unwrap();

        let json = serde_json::to_string(&coordination).expect("serialize");
        let restored: DomainCoordination = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.scope, CoordinationScope::Level(u8::MAX));
        assert_eq!(
            restored.group,
            DomainCoordinationGroup::Named("height".to_string())
        );
    }

    #[test]
    fn domain_group_id_validation() {
        validate_domain_group_id("height").unwrap();
        validate_domain_group_id("bill_length-mm").unwrap();
        validate_domain_group_id("").unwrap_err();
        validate_domain_group_id("outer.inner").unwrap_err();
        validate_domain_group_id("height mm").unwrap_err();
        validate_domain_group_id("é").unwrap_err();
    }
}
