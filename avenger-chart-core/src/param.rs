//! Parameter support for parameterized plots

use std::fmt::Debug;

use avenger_common::cursor::CursorStyle;
use datafusion::{
    arrow::datatypes::DataType, logical_expr::expr::Placeholder, prelude::Expr, scalar::ScalarValue,
};
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{Sharing, serialization::SerializableScalar};

/// A parameter that can be used in plot expressions
#[derive(Debug, Clone)]
pub struct Param {
    /// The name of the parameter
    pub name: String,
    /// The default value of the parameter
    pub default: ScalarValue,
}

impl Param {
    /// Create a new parameter with a name and default value
    pub fn new<S: Into<String>, T: Into<ScalarValue>>(name: S, default: T) -> Self {
        Self {
            name: name.into(),
            default: default.into(),
        }
    }

    /// Create a raw-domain parameter for interaction-driven scale domains.
    ///
    /// The default value is a typed null `List(Float64)` scalar, so a scale
    /// reading `raw_domain(param.expr())` falls back to its inferred or explicit
    /// domain until an interaction writes a concrete two-element domain list.
    pub fn raw_domain<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            default: ScalarValue::new_null_list(DataType::Float64, true, 1),
        }
    }

    /// Create a cursor parameter for app-interaction cursor state.
    pub fn cursor<S: Into<String>>(name: S, default_cursor: CursorStyle) -> Self {
        Self {
            name: name.into(),
            default: ScalarValue::Utf8(Some(default_cursor.as_str().to_string())),
        }
    }

    /// Get a DataFusion expression for this parameter as a placeholder
    pub fn expr(&self) -> Expr {
        Expr::Placeholder(Placeholder {
            id: format!("${}", self.name),
            data_type: Some(self.default.data_type()),
        })
    }
}

/// Compile-time metadata for a chart parameter, including its sharing scope.
///
/// `Sharing` decides which scoped copy of the parameter an event assignment
/// patches and which scoped value a scope reads. `Sharing::Shared` preserves the
/// historical single-global-value behavior.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledParamSpec {
    /// The parameter name.
    pub name: String,
    /// The default value used when no scoped value has been written.
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub default: ScalarValue,
    /// The sharing scope that governs how the parameter is keyed across facets.
    pub sharing: Sharing,
}

impl CompiledParamSpec {
    /// Create a spec from a parameter and an explicit sharing scope.
    pub fn new(param: &Param, sharing: Sharing) -> Self {
        Self {
            name: param.name.clone(),
            default: param.default.clone(),
            sharing,
        }
    }

    /// Create a globally shared spec (the historical default behavior).
    pub fn shared(param: &Param) -> Self {
        Self::new(param, Sharing::Shared)
    }
}

impl From<(String, ScalarValue)> for Param {
    fn from(param: (String, ScalarValue)) -> Self {
        Param::new(param.0, param.1)
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
                assert!(matches!(placeholder.data_type, Some(DataType::List(_))));
            }
            other => panic!("expected placeholder expr, got {other:?}"),
        }
    }

    #[test]
    fn compiled_param_spec_round_trips_sharing() {
        let param = Param::raw_domain("x_domain");
        let spec = CompiledParamSpec::new(&param, Sharing::Level(1));
        let json = serde_json::to_string(&spec).expect("serialize spec");
        let restored: CompiledParamSpec = serde_json::from_str(&json).expect("deserialize spec");
        assert_eq!(restored.name, "x_domain");
        assert_eq!(restored.sharing, Sharing::Level(1));
        assert!(matches!(restored.default, ScalarValue::List(_)));
    }

    #[test]
    fn add_param_default_spec_is_shared() {
        let param = Param::new("width", ScalarValue::Float64(Some(640.0)));
        let spec = CompiledParamSpec::shared(&param);
        assert_eq!(spec.sharing, Sharing::Shared);
        assert_eq!(spec.default, ScalarValue::Float64(Some(640.0)));
    }
}
