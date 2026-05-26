use std::collections::HashMap;

use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{Maybe, ScaleDomain, ScaleRange, ScaleSpec, SerializableExpr};

/// Optional categorical scale-domain ordering configuration.
#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScaleOrderingSpec {
    #[serde_as(as = "Option<FromInto<SerializableExpr>>")]
    pub order_expr: Option<LogicalExprNode>,
    pub order_descending: Option<bool>,
}

impl ScaleOrderingSpec {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn has_order_expr(&self) -> bool {
        self.order_expr.is_some()
    }

    pub fn order_descending(&self) -> bool {
        self.order_descending.unwrap_or(false)
    }

    pub fn merge(&mut self, other: ScaleOrderingSpec) {
        if other.order_expr.is_some() {
            self.order_expr = other.order_expr;
        }
        if other.order_descending.is_some() {
            self.order_descending = other.order_descending;
        }
    }
}

/// Owned scale configuration data shared by chart-facing scale wrappers and channels.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleConfigSpec {
    pub scale_spec: Maybe<Box<dyn ScaleSpec>>,
    pub domain: Maybe<ScaleDomain>,
    pub range: Maybe<ScaleRange>,
    pub ordering: Maybe<ScaleOrderingSpec>,
    #[serde_as(as = "HashMap<_, FromInto<SerializableExpr>>")]
    pub options: HashMap<String, LogicalExprNode>,
}

impl ScaleConfigSpec {
    pub fn empty() -> Self {
        Self {
            scale_spec: Maybe::Unset,
            domain: Maybe::Unset,
            range: Maybe::Unset,
            ordering: Maybe::Unset,
            options: HashMap::new(),
        }
    }

    pub fn new(
        scale_spec: Maybe<Box<dyn ScaleSpec>>,
        domain: Maybe<ScaleDomain>,
        range: Maybe<ScaleRange>,
        ordering: Maybe<ScaleOrderingSpec>,
        options: HashMap<String, LogicalExprNode>,
    ) -> Self {
        Self {
            scale_spec,
            domain,
            range,
            ordering,
            options,
        }
    }
}
