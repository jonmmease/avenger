use std::collections::HashMap;

use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::{FromInto, serde_as};

use crate::{Maybe, ScaleDomain, ScaleRange, ScaleSpec, SerializableExpr};

/// Owned scale configuration data shared by chart-facing scale wrappers and channels.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaleConfigSpec {
    pub scale_spec: Maybe<Box<dyn ScaleSpec>>,
    pub domain: Maybe<ScaleDomain>,
    pub range: Maybe<ScaleRange>,
    #[serde_as(as = "HashMap<_, FromInto<SerializableExpr>>")]
    pub options: HashMap<String, LogicalExprNode>,
}

impl ScaleConfigSpec {
    pub fn empty() -> Self {
        Self {
            scale_spec: Maybe::Unset,
            domain: Maybe::Unset,
            range: Maybe::Unset,
            options: HashMap::new(),
        }
    }

    pub fn new(
        scale_spec: Maybe<Box<dyn ScaleSpec>>,
        domain: Maybe<ScaleDomain>,
        range: Maybe<ScaleRange>,
        options: HashMap<String, LogicalExprNode>,
    ) -> Self {
        Self {
            scale_spec,
            domain,
            range,
            options,
        }
    }
}
