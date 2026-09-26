use std::sync::Arc;

use arrow::array::StringArray;
use avenger_scales::scales::{
    quantile::QuantileScale, quantize::QuantizeScale, threshold::ThresholdScale,
};

#[test]
fn interval_legends_use_the_formatter_and_map_to_their_range_values() {
    use avenger_format::NumberFormatProvider;
    use avenger_format_number_d3::{D3NumberFormatConfig, D3NumberFormatProvider};
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
        scale.config.context.formatters.number = Some(
            D3NumberFormatProvider
                .prepare(&D3NumberFormatConfig::new(), ".1f")
                .unwrap(),
        );
        let entries = scale
            .scale_impl
            .legend_entries(&scale.config)
            .unwrap()
            .unwrap();
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
