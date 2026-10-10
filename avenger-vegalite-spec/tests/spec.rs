use avenger_vegalite_spec::{
    AggregateOp, Bin, BinOutput, BinParams, Data, FieldType, Mark, MissingNullOrValue, Transform,
    UnitSpec,
};
use serde_json::{json, Value};

const BARS: &str = include_str!("fixtures/aggregate-bars.json");
const HISTOGRAM: &str = include_str!("fixtures/histogram.json");
const TRANSFORMS: &str = include_str!("fixtures/explicit-transforms.json");

fn round_trip(value: Value) -> UnitSpec {
    let spec = UnitSpec::from_json(&value.to_string()).unwrap();
    let serialized = serde_json::to_string(&spec).unwrap();
    assert_eq!(UnitSpec::from_json(&serialized).unwrap(), spec);
    spec
}

fn invalid(value: Value, path: &str, message: &str) {
    let error = UnitSpec::from_json(&value.to_string()).unwrap_err();
    assert_eq!(error.path(), path, "{error}");
    assert!(error.message().contains(message), "{error}");
}

fn bin_spec(bin: Value) -> Value {
    json!({"data": null, "mark": "bar", "encoding": {"x": {"field": "x", "bin": bin}}})
}

#[test]
fn fixture_round_trips_keep_transform_order_and_alternatives() {
    for fixture in [BARS, HISTOGRAM, TRANSFORMS] {
        round_trip(serde_json::from_str(fixture).unwrap());
    }
    let spec = UnitSpec::from_json(TRANSFORMS).unwrap();
    let transforms = spec.transform.unwrap();
    assert!(matches!(&transforms[0], Transform::Bin(bin) if matches!(bin.as_, BinOutput::Pair(_))));
    assert!(matches!(&transforms[1], Transform::Aggregate(agg) if agg.aggregate.len() == 2));
    let encoding = spec.encoding.unwrap();
    assert_eq!(
        encoding.x.unwrap().bin,
        MissingNullOrValue::Value(Bin::Binned)
    );
    assert_eq!(encoding.x2.unwrap().field, "delay_hi");
}

#[test]
fn source_variants_preserve_names_format_and_json_data() {
    let sources = [
        Value::Null,
        json!({"name": "external_binding"}),
        json!({"url": "/does/not/exist.csv", "name": "flights", "format": {"type": "csv"}}),
        json!({"values": [{"nested": {"n": null}, "list": [1, true, "x"]}], "name": "inline", "format": {}}),
    ];
    for data in sources {
        let spec = round_trip(json!({"data": data, "mark": "bar"}));
        assert_eq!(serde_json::to_value(&spec).unwrap()["data"], data);
        match data {
            Value::Null => assert_eq!(spec.data, Data::Empty),
            _ if data.get("values").is_some() => assert!(matches!(spec.data, Data::Inline { .. })),
            _ if data.get("url").is_some() => assert!(matches!(spec.data, Data::Url { .. })),
            _ => assert!(matches!(spec.data, Data::Named { .. })),
        }
    }
    let value = json!({
        "$schema": "https://invalid.example/schema.json", "data": {"name": "rows"},
        "datasets": {"rows": [{"x": 1}]}, "usermeta": {"custom": [null, {"x": true}]},
        "mark": "bar", "title": ["One", "Two"]
    });
    assert_eq!(
        serde_json::to_value(round_trip(value.clone())).unwrap(),
        value
    );
}

#[test]
fn missing_and_null_data_are_different() {
    invalid(json!({"mark": "bar"}), "$", "missing field `data`");
    invalid(json!({"data": null}), "$", "missing field `mark`");
    assert_eq!(
        round_trip(json!({"data": null, "mark": "bar"})).data,
        Data::Empty
    );
}

#[test]
fn optional_properties_keep_omission_null_and_empty_objects() {
    let minimal = round_trip(json!({"data": null, "mark": "bar"}));
    assert_eq!(
        serde_json::to_value(minimal).unwrap(),
        json!({"data": null, "mark": "bar"})
    );
    let value = json!({
        "data": null, "mark": {"type": "bar"}, "transform": [], "datasets": {}, "usermeta": {},
        "encoding": {"x": {"field": "x", "bin": null, "axis": null, "scale": {}, "title": null, "sort": null}}
    });
    let spec = round_trip(value.clone());
    let x = spec.encoding.as_ref().unwrap().x.as_ref().unwrap();
    assert!(x.bin.is_null());
    assert!(x.axis.is_null());
    assert!(x.scale.as_option().is_some());
    assert!(matches!(spec.mark, Mark::Def(_)));
    assert_eq!(serde_json::to_value(spec).unwrap(), value);
}

#[test]
fn nonnullable_optional_properties_reject_null() {
    for (value, path) in [
        (json!({"data": null, "mark": "bar", "title": null}), "title"),
        (
            json!({"data": null, "mark": "bar", "encoding": null}),
            "encoding",
        ),
        (
            json!({"data": null, "mark": {"type": "bar", "opacity": null}}),
            "mark.opacity",
        ),
        (bin_spec(json!({"step": null})), "encoding.x.bin.step"),
        (
            json!({"data": {"values": [], "name": null}, "mark": "bar"}),
            "data.name",
        ),
        (
            json!({"data": null, "mark": "bar", "encoding": {"x": {"field": "x", "aggregate": null}}}),
            "encoding.x.aggregate",
        ),
    ] {
        invalid(value, path, "null is not allowed");
    }
}

#[test]
fn unknown_and_unsupported_properties_are_errors_at_their_locations() {
    for (value, path, message) in [
        (
            json!({"data": null, "mark": "bar", "layer": []}),
            "layer",
            "unknown field",
        ),
        (
            json!({"data": null, "mark": "point"}),
            "mark",
            "unknown variant",
        ),
        (
            json!({"data": null, "mark": {"type": "bar", "tooltip": true}}),
            "mark.tooltip",
            "unknown field",
        ),
        (
            json!({"data": null, "mark": "bar", "encoding": {"color": {"field": "x"}}}),
            "encoding.color",
            "unknown field",
        ),
        (
            bin_spec(json!({"span": 10})),
            "encoding.x.bin.span",
            "unknown field",
        ),
        (
            bin_spec(json!({"extent": {"param": "brush"}})),
            "encoding.x.bin.extent",
            "invalid type",
        ),
        (
            json!({"data": {"url": "file.csv", "format": {"parse": "auto"}}, "mark": "bar"}),
            "data.format.parse",
            "unknown field",
        ),
        (
            json!({"data": null, "mark": "bar", "encoding": {"x2": {"field": "hi", "type": "quantitative"}}}),
            "encoding.x2.type",
            "unknown field",
        ),
    ] {
        invalid(value, path, message);
    }
}

#[test]
fn object_dispatch_rejects_conflicting_or_missing_identifiers() {
    for (value, path, message) in [
        (
            json!({"data": {"url": "a", "values": []}, "mark": "bar"}),
            "data",
            "both values and url",
        ),
        (
            json!({"data": {}, "mark": "bar"}),
            "data",
            "requires values, url, or name",
        ),
        (
            json!({"data": null, "mark": {}, "encoding": {}}),
            "mark",
            "missing field `type`",
        ),
        (
            json!({"data": null, "mark": "bar", "transform": [{"bin": true, "aggregate": []}]}),
            "transform[0]",
            "both bin and aggregate",
        ),
        (
            json!({"data": null, "mark": "bar", "transform": [{"bin": true, "field": "x", "as": "b", "groupby": []}]}),
            "transform[0]",
            "groupby",
        ),
        (
            json!({"data": null, "mark": "bar", "transform": [{"aggregate": [{"op": "count", "as": "n"}], "field": "x"}]}),
            "transform[0]",
            "inside aggregate measures",
        ),
        (
            json!({"data": null, "mark": "bar", "transform": [{}]}),
            "transform[0]",
            "expected a bin, aggregate, or filter",
        ),
        (
            json!({"data": null, "mark": "bar", "transform": [{"bin": true, "field": "x"}]}),
            "transform[0]",
            "missing field `as`",
        ),
    ] {
        invalid(value, path, message);
    }
}

#[test]
fn aggregates_work_in_encodings_and_transforms_without_rewriting_aliases() {
    for op in [
        "count",
        "valid",
        "missing",
        "sum",
        "min",
        "max",
        "mean",
        "average",
        "variance",
        "variancep",
        "stdev",
        "stdevp",
    ] {
        let mut value: Value = serde_json::from_str(BARS).unwrap();
        value["encoding"]["y"]["aggregate"] = json!(op);
        value["transform"] =
            json!([{"aggregate": [{"op": op, "field": "delay", "as": "measure"}]}]);
        let spec = round_trip(value);
        let serialized = serde_json::to_value(spec).unwrap();
        assert_eq!(serialized["encoding"]["y"]["aggregate"], op);
        assert_eq!(serialized["transform"][0]["aggregate"][0]["op"], op);
        assert!(serialized["transform"][0].get("groupby").is_none());
    }
}

#[test]
fn horizontal_fieldless_count_and_explicit_empty_grouping() {
    let value = json!({
        "data": null, "mark": {"type": "bar", "orient": "horizontal"},
        "encoding": {"y": {"field": "airline", "type": "nominal"}, "x": {"aggregate": "count", "type": "quantitative"}},
        "transform": [{"aggregate": [{"op": "count", "as": "n"}], "groupby": []}]
    });
    let spec = round_trip(value.clone());
    let x = spec.encoding.as_ref().unwrap().x.as_ref().unwrap();
    assert_eq!(x.aggregate, Some(AggregateOp::Count));
    assert_eq!(x.field, None);
    assert_eq!(x.field_type, Some(FieldType::Quantitative));
    assert_eq!(serde_json::to_value(spec).unwrap(), value);
}

#[test]
fn aggregate_validation_reports_missing_fields_and_duplicate_aliases() {
    invalid(
        json!({"data": null, "mark": "bar", "encoding": {"y": {"aggregate": "mean"}}}),
        "encoding.y.field",
        "field is required",
    );
    invalid(
        json!({"data": null, "mark": "bar", "encoding": {"x": {}}}),
        "encoding.x.field",
        "field is required",
    );
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"aggregate": [{"op": "sum", "as": "s"}]}]}),
        "transform[0].aggregate[0].field",
        "field is required",
    );
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"aggregate": []}]}),
        "transform[0].aggregate",
        "at least one",
    );
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"aggregate": [{"op": "count", "as": "n"}, {"op": "count", "as": "n"}]}]}),
        "transform[0].aggregate[1].as",
        "aliases must be distinct",
    );
    invalid(
        json!({"data": null, "mark": "bar", "encoding": {"x": {"field": "x", "aggregate": "median"}}}),
        "encoding.x.aggregate",
        "unknown variant",
    );
}

#[test]
fn bin_forms_preserve_defaults_and_explicit_choices() {
    for bin in [
        json!(true),
        json!(false),
        Value::Null,
        json!({}),
        json!("binned"),
        json!({"binned": true, "step": 5.0}),
        json!({"maxbins": 12.5}),
        json!({"step": 5.0, "maxbins": 10.0, "nice": false}),
    ] {
        let spec = round_trip(bin_spec(bin.clone()));
        assert_eq!(
            serde_json::to_value(&spec).unwrap()["encoding"]["x"]["bin"],
            bin
        );
    }
    let spec = round_trip(
        json!({"data": null, "mark": "bar", "encoding": {"x": {"field": "a.b[0]\\.c"}}}),
    );
    let x = spec.encoding.unwrap().x.unwrap();
    assert!(x.bin.is_missing());
    assert_eq!(x.field.as_deref(), Some("a.b[0]\\.c"));
}

#[test]
fn bin_options_accept_numeric_extents_steps_and_schema_divisor_lengths() {
    for divide in [vec![2.0], vec![5.0, 2.0]] {
        let params = json!({"maxbins": 2.0, "step": 1.0, "steps": [1.0, 2.0, 5.0], "minstep": 0.0, "base": 10.0, "divide": divide, "anchor": -5.0, "extent": [1.0, 1.0], "nice": false, "binned": false});
        let spec = round_trip(bin_spec(params.clone()));
        assert_eq!(
            serde_json::to_value(spec).unwrap()["encoding"]["x"]["bin"],
            params
        );
    }
}

#[test]
fn bin_numeric_boundaries_are_validated_with_paths() {
    for (params, suffix, message) in [
        (json!({"maxbins": 1}), "maxbins", "at least 2"),
        (json!({"step": 0}), "step", "positive"),
        (json!({"minstep": -1}), "minstep", "nonnegative"),
        (json!({"base": 1}), "base", "greater than 1"),
        (json!({"steps": []}), "steps", "at least one"),
        (json!({"steps": [-1]}), "steps[0]", "positive"),
        (json!({"steps": [2, 2]}), "steps[1]", "strictly increasing"),
        (json!({"steps": [2, 1]}), "steps[1]", "strictly increasing"),
        (json!({"divide": []}), "divide", "one or two"),
        (json!({"divide": [5, 2, 2]}), "divide", "one or two"),
        (json!({"divide": [1]}), "divide[0]", "greater than 1"),
        (json!({"extent": [2, 1]}), "extent", "ordered"),
    ] {
        invalid(
            bin_spec(params),
            &format!("encoding.x.bin.{suffix}"),
            message,
        );
    }
}

#[test]
fn explicit_bin_has_stricter_shape_than_encoding_bin() {
    for bin in [json!(false), json!("binned")] {
        invalid(
            json!({"data": null, "mark": "bar", "transform": [{"bin": bin, "field": "x", "as": "lo"}]}),
            "transform[0].bin",
            "expected true or",
        );
    }
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"bin": null, "field": "x", "as": "lo"}]}),
        "transform[0].bin",
        "null is not allowed",
    );
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"bin": true, "field": "x", "as": ["lo"]}]}),
        "transform[0].as",
        "did not match",
    );
    invalid(
        json!({"data": null, "mark": "bar", "transform": [{"bin": true, "field": "x", "as": ["lo", "lo"]}]}),
        "transform[0].as",
        "distinct",
    );
    let value = json!({"data": null, "mark": "bar", "transform": [{"bin": true, "field": "x", "as": "lo"}]});
    assert_eq!(
        serde_json::to_value(round_trip(value.clone())).unwrap(),
        value
    );
}

#[test]
fn axis_scale_and_both_prebinned_orientations_round_trip() {
    for (start, end) in [("x", "x2"), ("y", "y2")] {
        let value = json!({"data": null, "mark": "bar", "encoding": {
            start: {"field": "lo", "bin": "binned", "type": "quantitative", "title": ["A", "B"], "sort": "descending", "axis": {"title": null, "format": ".1f", "labelAngle": -90.0, "grid": false}, "scale": {"type": "linear", "zero": false, "nice": true}},
            end: {"field": "hi"}
        }});
        assert_eq!(
            serde_json::to_value(round_trip(value.clone())).unwrap(),
            value
        );
    }
}

#[test]
fn rust_edits_and_direct_serde_need_validation() {
    let mut spec: UnitSpec = serde_json::from_value(bin_spec(json!({"step": 0}))).unwrap();
    assert_eq!(spec.validate().unwrap_err().path(), "encoding.x.bin.step");
    let params = BinParams {
        step: Some(f64::NAN),
        ..Default::default()
    };
    spec.encoding.as_mut().unwrap().x.as_mut().unwrap().bin =
        MissingNullOrValue::Value(Bin::Params(params));
    assert_eq!(spec.validate().unwrap_err().path(), "encoding.x.bin.step");
    spec.encoding.as_mut().unwrap().x.as_mut().unwrap().bin =
        MissingNullOrValue::Value(Bin::Bool(true));
    spec.width = Some(f64::INFINITY);
    assert_eq!(spec.validate().unwrap_err().path(), "width");
    spec.width = Some(300.0);
    spec.validate().unwrap();
}

#[test]
fn layout_and_mark_numbers_are_validated() {
    for (value, path) in [
        (json!({"data": null, "mark": "bar", "height": -1}), "height"),
        (
            json!({"data": null, "mark": {"type": "bar", "opacity": 1.1}}),
            "mark.opacity",
        ),
        (
            json!({"data": null, "mark": {"type": "bar", "size": -1}}),
            "mark.size",
        ),
        (
            json!({"data": null, "mark": "bar", "encoding": {"x": {"field": "x", "axis": {"labelAngle": 361}}}}),
            "encoding.x.axis.labelAngle",
        ),
    ] {
        invalid(value, path, "expected");
    }
}

#[test]
fn duplicate_fields_and_trailing_documents_are_rejected() {
    for input in [
        r#"{"data":null,"mark":"bar","mark":"bar"}"#,
        r#"{"data":{"name":"a","name":"b"},"mark":"bar"}"#,
        r#"{"data":null,"mark":"bar","transform":[{"bin":true,"bin":false,"field":"x","as":"lo"}]}"#,
    ] {
        assert!(UnitSpec::from_json(input)
            .unwrap_err()
            .message()
            .contains("duplicate field"));
    }
    let error = UnitSpec::from_json(r#"{"data":null,"mark":"bar"} {}"#).unwrap_err();
    assert_eq!(error.path(), "$");
    assert!(error.message().contains("trailing"));
}

#[test]
fn numeric_parameters_filters_and_stack_options_round_trip() {
    let json = r#"{"data":null,"mark":"bar","params":[{"name":"cutoff","value":10.0}],"transform":[{"filter":{"field":"v","gte":{"expr":"cutoff"}}}],"encoding":{"y":{"field":"v","type":"quantitative","stack":null}}}"#;
    let spec = avenger_vegalite_spec::UnitSpec::from_json(json).unwrap();
    assert_eq!(
        serde_json::to_value(&spec).unwrap(),
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
    for params in [
        r#"[{"name":"a","value":0},{"name":"a","value":1}]"#,
        r#"[{"name":"datum","value":1}]"#,
        r#"[{"name":"a-b","value":1}]"#,
        r#"[{"name":"a","expr":"2"}]"#,
    ] {
        assert!(avenger_vegalite_spec::UnitSpec::from_json(&format!(
            r#"{{"data":null,"mark":"bar","params":{params}}}"#
        ))
        .is_err());
    }
    for filter in [
        r#"{"field":"x","lt":1}"#,
        r#"{"field":"x","gte":null}"#,
        r#""datum.x > 1""#,
    ] {
        assert!(avenger_vegalite_spec::UnitSpec::from_json(&format!(
            r#"{{"data":null,"mark":"bar","transform":[{{"filter":{filter}}}]}}"#
        ))
        .is_err());
    }
}
