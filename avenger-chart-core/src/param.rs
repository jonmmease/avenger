//! Parameter support for parameterized plots

use std::fmt::Debug;

use std::sync::Arc;

use datafusion::{
    arrow::datatypes::{DataType, Field},
    logical_expr::expr::Placeholder,
    prelude::Expr,
    scalar::ScalarValue,
};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{
    AvengerChartError, CoordinationScope, DomainCoordination, ParamRef, StateMigrationKey,
    serialization::{SerializableDataType, SerializableScalar},
};

/// A parameter that can be used in plot expressions
#[derive(Debug, Clone)]
pub struct Param {
    /// The name of the parameter
    pub name: String,
    /// The default value of the parameter
    pub default: ScalarValue,
    /// Optional stable identity used only for compatible hot-reload migration.
    pub migration_key: Option<StateMigrationKey>,
}

impl Param {
    /// Create a parameter from a precisely typed Arrow scalar value.
    pub fn new<S: Into<String>>(name: S, default: impl Into<ScalarValue>) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
            migration_key: None,
        }
    }

    pub fn migration_key(mut self, migration_key: StateMigrationKey) -> Self {
        self.migration_key = Some(migration_key);
        self
    }

    /// Create a raw-domain parameter for interaction-driven scale domains.
    ///
    /// The default value is a typed null `List(Float64)` scalar, so a scale
    /// reading `raw_domain(param.expr())` falls back to its inferred or explicit
    /// domain until an interaction writes a concrete two-element domain list.
    pub fn raw_domain<S: Into<String>>(name: S) -> Self {
        Self::new(name, ScalarValue::new_null_list(DataType::Float64, true, 1))
    }

    /// Get a DataFusion expression for this parameter as a placeholder
    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder::new_with_field(
            format!("${}", self.name),
            Some(Arc::new(Field::new("", self.default.data_type(), true))),
        ))
    }
}

/// Validate a parameter value against its declared physical Arrow type.
///
/// Equality is deliberately exact and recursive. Numeric widening, list-item
/// coercion, struct field reordering, and timezone changes must be explicit at
/// the authoring or host-binding boundary.
pub fn validate_param_value(
    expected: &DataType,
    value: &ScalarValue,
) -> Result<(), AvengerChartError> {
    let actual = value.data_type();
    if actual != *expected {
        return Err(AvengerChartError::InvalidArgument(format!(
            "Parameter value has physical Arrow type {actual:?}, expected {expected:?}"
        )));
    }
    Ok(())
}

/// Compile-time metadata for a chart parameter, including its sharing scope.
///
/// `CoordinationScope` decides which scoped copy of the parameter an event assignment
/// patches and which scoped value a scope reads. `CoordinationScope::Shared` preserves the
/// historical single-global-value behavior.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledParamSpec {
    /// Opaque runtime identity assigned at the root compilation boundary.
    pub runtime_id: ParamRef,
    /// The parameter name.
    pub name: String,
    /// Optional hot-reload migration metadata, never used for runtime lookup.
    #[serde(default)]
    pub migration_key: Option<StateMigrationKey>,
    /// The authoritative physical Arrow type.
    #[serde_as(as = "FromInto<SerializableDataType>")]
    pub data_type: DataType,
    /// The default value used when no scoped value has been written.
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub default: ScalarValue,
    /// The sharing scope that governs how the parameter is keyed across facets.
    pub sharing: CoordinationScope,
    /// Optional scale-domain coordination metadata for raw-domain params.
    ///
    /// This is compile-time validation metadata. Runtime parameter storage is
    /// still governed by `sharing`.
    #[serde(default)]
    pub domain_coordination: Option<DomainCoordination>,
}

impl CompiledParamSpec {
    /// Create a spec from a parameter and an explicit sharing scope.
    pub fn new(param: &Param, sharing: CoordinationScope) -> Self {
        Self {
            runtime_id: ParamRef::unresolved_authoring(),
            name: param.name.clone(),
            migration_key: param.migration_key.clone(),
            data_type: param.default.data_type(),
            default: param.default.clone(),
            sharing,
            domain_coordination: None,
        }
    }

    /// Create a globally shared spec (the historical default behavior).
    pub fn shared(param: &Param) -> Self {
        Self::new(param, CoordinationScope::Shared)
    }

    /// Attach raw-domain scale coordination metadata.
    pub fn with_domain_coordination(mut self, coordination: DomainCoordination) -> Self {
        self.domain_coordination = Some(coordination);
        self
    }
}

impl From<(String, ScalarValue)> for Param {
    fn from((name, default): (String, ScalarValue)) -> Self {
        Self::new(name, default)
    }
}

impl From<Param> for Expr {
    fn from(param: Param) -> Self {
        param.expr()
    }
}

impl From<&Param> for Expr {
    fn from(param: &Param) -> Self {
        param.expr()
    }
}

#[cfg(test)]
mod tests {
    use datafusion::arrow::datatypes::{Fields, TimeUnit};

    use super::*;

    #[test]
    fn raw_domain_default_is_nullable_float64_list() {
        let param = Param::raw_domain("x_domain");
        // The default value is a typed null List(Float64) scalar.
        assert!(matches!(param.default, ScalarValue::List(_)));
        match param.default.data_type() {
            DataType::List(field) => assert_eq!(field.data_type(), &DataType::Float64),
            other => panic!("expected List(Float64) data type, got {other:?}"),
        }
        // The placeholder expr carries the list data type for downstream binding.
        match param.expr() {
            Expr::Placeholder(placeholder) => {
                assert_eq!(placeholder.id, "$x_domain");
                let data_type = placeholder
                    .field
                    .as_ref()
                    .expect("raw domain placeholder field")
                    .data_type();
                assert!(matches!(data_type, DataType::List(_)));
            }
            other => panic!("expected placeholder expr, got {other:?}"),
        }
    }

    #[test]
    fn compiled_param_spec_round_trips_sharing() {
        let param = Param::raw_domain("x_domain");
        let mut allocator = crate::CompiledIdentityAllocator::new("param-spec-test");
        let mut spec = CompiledParamSpec::new(&param, CoordinationScope::Level(1))
            .with_domain_coordination(
                DomainCoordination::named(CoordinationScope::Level(1), "x").unwrap(),
            );
        spec.runtime_id = allocator.allocate_param();
        spec.migration_key = Some(allocator.migration_key("root/param:x_domain"));
        let json = serde_json::to_string(&spec).expect("serialize spec");
        let restored: CompiledParamSpec = serde_json::from_str(&json).expect("deserialize spec");
        assert_eq!(restored.name, "x_domain");
        assert_eq!(restored.sharing, CoordinationScope::Level(1));
        assert_eq!(
            restored.domain_coordination.as_ref().unwrap().group,
            crate::DomainCoordinationGroup::Named("x".to_string())
        );
        assert!(matches!(restored.default, ScalarValue::List(_)));
        assert_eq!(restored.data_type, spec.data_type);
        assert_eq!(restored.runtime_id, spec.runtime_id);
        assert_eq!(restored.migration_key, spec.migration_key);
        assert_ne!(
            restored.runtime_id.as_opaque_str(),
            restored.migration_key.unwrap().as_opaque_str()
        );
    }

    #[test]
    fn compiled_param_spec_deserializes_without_domain_coordination() {
        let param = Param::raw_domain("x_domain");
        let spec = CompiledParamSpec::new(&param, CoordinationScope::Level(1));
        let mut json = serde_json::to_value(&spec).expect("serialize spec");
        json.as_object_mut()
            .expect("spec object")
            .remove("domain_coordination");
        let restored: CompiledParamSpec = serde_json::from_value(json).expect("deserialize spec");
        assert_eq!(restored.name, "x_domain");
        assert_eq!(restored.domain_coordination, None);
    }

    #[test]
    fn add_param_default_spec_is_shared() {
        let param = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let spec = CompiledParamSpec::shared(&param);
        assert_eq!(spec.sharing, CoordinationScope::Shared);
        assert_eq!(spec.data_type, DataType::Float64);
        assert_eq!(spec.default, ScalarValue::Float64(Some(640.0)));
    }

    #[test]
    fn exact_value_validation_rejects_numeric_width_mismatch() {
        let error = validate_param_value(&DataType::Int32, &ScalarValue::Int64(Some(1)))
            .expect_err("int64 must not satisfy an int32 destination");
        assert!(error.to_string().contains("Int64"));
        assert!(error.to_string().contains("Int32"));
    }

    #[test]
    fn typed_nulls_preserve_recursive_struct_list_and_map_types() {
        let struct_type = DataType::Struct(Fields::from(vec![
            Field::new("label", DataType::Utf8, true),
            Field::new(
                "values",
                DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
                true,
            ),
        ]));
        let map_entry = Field::new(
            "entries",
            DataType::Struct(Fields::from(vec![
                Field::new("keys", DataType::Utf8, false),
                Field::new("values", struct_type.clone(), true),
            ])),
            false,
        );
        let map_type = DataType::Map(Arc::new(map_entry), false);

        for (name, data_type) in [("record", struct_type), ("lookup", map_type)] {
            let default = ScalarValue::try_from(&data_type).expect("typed recursive null");
            let param = Param::new(name, default);
            assert_eq!(param.default.data_type(), data_type);
            assert_eq!(CompiledParamSpec::shared(&param).data_type, data_type);
        }
    }

    #[test]
    fn exact_value_validation_rejects_struct_order_and_timestamp_timezone_mismatch() {
        let declared = DataType::Struct(Fields::from(vec![
            Field::new("left", DataType::Int32, true),
            Field::new("right", DataType::Utf8, true),
        ]));
        let reordered = DataType::Struct(Fields::from(vec![
            Field::new("right", DataType::Utf8, true),
            Field::new("left", DataType::Int32, true),
        ]));
        let reordered_null = ScalarValue::try_from(&reordered).unwrap();
        assert!(validate_param_value(&declared, &reordered_null).is_err());

        let declared = DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()));
        let actual = ScalarValue::TimestampMillisecond(None, Some("America/New_York".into()));
        assert!(validate_param_value(&declared, &actual).is_err());
    }
}
