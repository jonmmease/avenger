use std::sync::Arc;

use arrow::array::StringArray;
use avenger_scales::{
    formatter::DefaultFormatter,
    scales::{quantile::QuantileScale, quantize::QuantizeScale, threshold::ThresholdScale},
};

#[test]
fn interval_legends_use_the_formatter_and_map_to_their_range_values() {
    let range = Arc::new(StringArray::from(vec!["low", "medium", "high"]));
    let cases = [
        (
            ThresholdScale::configured(vec![20.0, 40.0], range.clone()),
            ["< 20.0", "20.0 - 40.0", "≥ 40.0"],
        ),
        (
            QuantizeScale::configured((0.0, 60.0), range.clone()),
            ["0.0 - 20.0", "20.0 - 40.0", "40.0 - 60.0"],
        ),
        (
            QuantileScale::configured(vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0], range),
            ["0.0 - 20.0", "20.0 - 40.0", "40.0 - 60.0"],
        ),
    ];
    for (mut scale, expected_labels) in cases {
        scale.config.context.formatters.number = Arc::new(
            DefaultFormatter {
                format_str: Some(".1f".to_string()),
                ..Default::default()
            }
            .prepare_number()
            .unwrap(),
        );
        let entries = scale.scale_impl.legend_entries(&scale.config).unwrap();
        assert_eq!(entries.len(), expected_labels.len());
        for ((entry, label), value) in entries
            .iter()
            .zip(expected_labels)
            .zip(["low", "medium", "high"])
        {
            assert_eq!(entry.label, label);
            let mapped = scale
                .scale_to_string(&entry.representative_value.to_array())
                .unwrap();
            assert_eq!(mapped.as_vec(1, None), vec![value.to_string()]);
        }
    }
}
