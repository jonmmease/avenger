use avenger_format::TickSpacing;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_typst_label::LabelEngine;

use super::{
    format_values,
    marks::{make_axis, AxisParts, TickLabels},
    opts::AxisConfig,
    pixel_range, tick_label_text,
};
use crate::error::AvengerGuidesError;

/// The gap between the labels and the title.
pub(crate) const TITLE_MARGIN: f32 = 4.0;

/// Axis marks for a band scale, with a tick and a label at the center of each band. The text
/// engine formats numeric and temporal categories with `config.tick_format`, each on its own,
/// and other categories show as text.
pub fn make_band_axis_marks(
    scale: &ConfiguredScale,
    title: &str,
    origin: [f32; 2],
    config: &AxisConfig,
    text_engine: &LabelEngine,
) -> Result<SceneGroup, AvengerGuidesError> {
    let scale = scale.clone().with_option("band", 0.5);
    let domain = scale.domain();
    let positions = scale.scale_to_numeric(domain)?.as_vec(domain.len(), None);
    let labels = if config.style.labels_visible {
        let ticks = format_values(
            domain,
            &config.tick_format,
            TickSpacing::Varying,
            text_engine,
        )?;
        let (text, syntax) = tick_label_text(ticks, config.tick_label.as_deref())?;
        TickLabels {
            positions: positions.clone(),
            text,
            syntax,
        }
    } else {
        TickLabels::default()
    };
    let parts = AxisParts {
        range: pixel_range(&scale)?,
        ticks: positions,
        labels,
        title,
        title_margin: TITLE_MARGIN,
    };
    make_axis(parts, origin, config, text_engine)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{ArrayRef, StringArray};
    use avenger_scales::scales::band::BandScale;

    use super::super::opts::{AxisOrientation, AxisStyle};
    use super::*;

    fn config(pattern: &str, tick_label: Option<&str>, style: AxisStyle) -> AxisConfig {
        AxisConfig {
            orientation: AxisOrientation::Bottom,
            dimensions: [100.0, 100.0],
            grid: false,
            tick_format: pattern.to_string(),
            tick_label: tick_label.map(str::to_string),
            style,
        }
    }

    #[test]
    fn category_labels_bind_each_category() {
        let domain = Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef;
        let scale = BandScale::configured(domain, (0.0, 100.0));
        let config = config(",", Some("*#tick*"), AxisStyle::default());
        let ticks = format_values(
            scale.domain(),
            &config.tick_format,
            TickSpacing::Varying,
            &crate::test_engine(),
        )
        .unwrap();
        let (text, _) = tick_label_text(ticks, config.tick_label.as_deref()).unwrap();
        assert_eq!(text, [r#"*#("a")*"#, r#"*#("b")*"#]);
    }
}
