//! External compound mark implemented with lower-level chart crates only.

use avenger_chart_cartesian::{Cartesian, CartesianSymbolPositionChannels};
use avenger_chart_core::{
    IntoExpr, IntoPlotMark, MarkGroup, PlotMark, ScaleInferenceHint, ScaleTypePreference,
};
use avenger_chart_marks::Symbol;
use avenger_chart_transforms::Aggregate;
use datafusion::prelude::{col, Expr};

pub const EXTERNAL_MEAN_POINT_FIELD: &str = "__external_mean_value";

/// A tiny aggregate-backed compound mark defined outside `avenger-chart`.
pub struct ExternalMeanPoint {
    category: Expr,
    value: Expr,
}

impl ExternalMeanPoint {
    pub fn new(category: impl IntoExpr, value: impl IntoExpr) -> Self {
        Self {
            category: category.into_expr(),
            value: value.into_expr(),
        }
    }

    pub fn into_group(self) -> MarkGroup<Cartesian> {
        let category = self.category;
        let value = self.value;
        MarkGroup::new()
            .scale_inference_hint(ScaleInferenceHint::new("x", ScaleTypePreference::Band))
            .transform(
                Aggregate::new()
                    .group_by([category.clone()])
                    .mean(EXTERNAL_MEAN_POINT_FIELD, value),
                move |group, _summary| {
                    group.mark(Symbol::new().x(category).y(col(EXTERNAL_MEAN_POINT_FIELD)))
                },
            )
    }
}

impl IntoPlotMark<Cartesian> for ExternalMeanPoint {
    fn into_plot_marks(self) -> Vec<PlotMark<Cartesian>> {
        vec![PlotMark::from_group(self.into_group())]
    }
}
