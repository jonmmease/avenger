use avenger_chart_core::{Maybe, MaybeOptionalExpr};
use datafusion_proto::protobuf::LogicalExprNode;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

/// Options for polar coordinate system.
#[serde_as]
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PolarOptions {
    /// Background color for the plot area.
    #[serde_as(as = "MaybeOptionalExpr")]
    pub plot_background_color: Maybe<Option<LogicalExprNode>>,
}
