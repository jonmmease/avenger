use avenger_chart::{Chart, RenderOptions};
use avenger_chart_definition::{ChartDefinition, Node, Plot, Value};
use avenger_datafusion_dataflow::{
    datafusion::{common::ScalarValue, execution::context::SessionContext},
    Runtime, RuntimeConfig, TableSnapshot,
};
use avenger_vegalite_compiler::{compile_vegalite, spec::UnitSpec, FromVegaLite, VegaLiteOptions};
use serde_json::{json, Value as Json};
use std::{collections::BTreeMap, path::Path, sync::Arc};

fn runtime() -> anyhow::Result<Runtime> {
    Ok(Runtime::with_session_state_and_codec(
        SessionContext::new().state(),
        RuntimeConfig {
            function_versions: avenger_transform::function_versions(),
            ..Default::default()
        },
        Arc::new(avenger_transform::TransformExtensionCodec::default()),
    )?)
}
async fn compile(v: &Json) -> anyhow::Result<ChartDefinition> {
    Ok(compile_vegalite(
        &UnitSpec::from_json(&v.to_string())?,
        &BTreeMap::new(),
        Path::new("."),
    )
    .await?)
}
fn plot(d: &ChartDefinition) -> &Plot {
    let Node::Plot(p) = &d.root().children[0] else {
        panic!()
    };
    p
}
async fn rows(d: &ChartDefinition) -> anyhow::Result<TableSnapshot> {
    let prepared = runtime()?.prepare(d.dataflow()).await?;
    let mut inputs = prepared.inputs();
    for p in d.parameters() {
        inputs = inputs.scalar(&p.input, p.initial.clone().unwrap())?;
    }
    let output = d.dataflow().interface().root().table_output("rows")?;
    Ok(prepared
        .query(&[output], &[], &inputs.finish()?)
        .await?
        .table(&output)?
        .clone())
}
fn column(table: &TableSnapshot, name: &str) -> Vec<ScalarValue> {
    table
        .batches()
        .iter()
        .flat_map(|b| {
            let a = b.column_by_name(name).unwrap();
            (0..a.len())
                .map(|i| ScalarValue::try_from_array(a, i).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}
fn field(v: &Value) -> &str {
    match v {
        Value::Field(f) => f,
        Value::Scaled(_, v) | Value::BandPosition(_, v, _) => field(v),
        _ => panic!("not a column {v:?}"),
    }
}
fn floats(v: Vec<ScalarValue>) -> Vec<f64> {
    v.into_iter()
        .map(|v| match v {
            ScalarValue::Float64(Some(v)) => v,
            _ => panic!("{v:?}"),
        })
        .collect()
}

#[tokio::test]
async fn pinned_bar_cases_compile_and_render() -> anyhow::Result<()> {
    let cases: Json = serde_json::from_str(include_str!("fixtures/basic-bars.json"))?;
    for case in cases["cases"].as_array().unwrap() {
        let d = compile(&case["spec"])
            .await
            .with_context(|| case["name"].to_string())?;
        let c = Chart::prepare(d, chart_options()).await?;
        let f = c
            .render(Default::default())
            .await
            .with_context(|| case["name"].to_string())?;
        assert_eq!(
            f.plots()[0].rect.width,
            case["width"].as_f64().unwrap() as f32,
            "{}",
            case["name"]
        );
        assert_eq!(
            f.plots()[0].rect.height,
            case["height"].as_f64().unwrap() as f32,
            "{}",
            case["name"]
        );
        let mut actual = Vec::new();
        rectangles(&f.scenegraph().marks, &mut actual);
        let expected = case["rects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| ["x", "y", "width", "height"].map(|k| r[k].as_f64().unwrap() as f32))
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len(), "{}", case["name"]);
        for (a, b) in actual.iter().zip(expected) {
            for (a, b) in a.iter().zip(b) {
                assert!(
                    (a - b).abs() < 0.02,
                    "{}: actual {actual:?}, expected {}",
                    case["name"],
                    case["rects"]
                );
            }
        }
        assert!(!f.scenegraph().marks.is_empty());
    }
    Ok(())
}
use anyhow::Context;

#[tokio::test]
async fn repeated_categories_stack_positive_and_negative_independently() -> anyhow::Result<()> {
    let d=compile(&json!({"data":{"values":[{"c":"A","v":4},{"c":"A","v":6},{"c":"A","v":-3},{"c":"A","v":-2}]},"mark":"bar","encoding":{"x":{"field":"c","type":"nominal"},"y":{"field":"v","type":"quantitative"}}})).await?;
    let t = rows(&d).await?;
    let avenger_chart_definition::Encoding::Rect(encoding) = &plot(&d).marks[0].encoding else {
        panic!()
    };
    assert_eq!(
        floats(column(&t, field(encoding.y.as_ref().unwrap()))),
        vec![0., 4., 0., -3.]
    );
    assert_eq!(
        floats(column(&t, field(encoding.y2.as_ref().unwrap()))),
        vec![4., 10., -3., -5.]
    );
    Ok(())
}
#[tokio::test]
async fn aggregate_family_and_null_rules() -> anyhow::Result<()> {
    for (op, expected) in [
        ("count", 4.),
        ("valid", 3.),
        ("missing", 1.),
        ("sum", 12.),
        ("min", 2.),
        ("max", 6.),
        ("mean", 4.),
        ("average", 4.),
        ("variance", 4.),
        ("variancep", 8. / 3.),
        ("stdev", 2.),
        ("stdevp", (8_f64 / 3.).sqrt()),
    ] {
        let d=compile(&json!({"data":{"values":[{"v":2},{"v":4},{"v":6},{"v":null}]},"mark":"bar","encoding":{"y":{"field":"v","aggregate":op,"type":"quantitative"}}})).await?;
        let t = rows(&d).await?;
        let avenger_chart_definition::Encoding::Rect(encoding) = &plot(&d).marks[0].encoding else {
            panic!()
        };
        assert!(
            (floats(column(&t, field(encoding.y2.as_ref().unwrap())))[0] - expected).abs() < 1e-9,
            "{op}"
        );
    }
    Ok(())
}
#[tokio::test]
async fn parameter_update_changes_rows_and_discrete_size_without_recompile() -> anyhow::Result<()> {
    let v = json!({"data":{"values":[{"c":"A","v":2},{"c":"B","v":6}]},"params":[{"name":"cutoff","value":0}],"transform":[{"filter":{"field":"v","gte":{"expr":"cutoff"}}}],"mark":"bar","encoding":{"x":{"field":"c","type":"nominal"},"y":{"aggregate":"sum","field":"v","type":"quantitative"}}});
    let c = Chart::from_vegalite(
        &UnitSpec::from_json(&v.to_string())?,
        &BTreeMap::new(),
        VegaLiteOptions {
            chart: chart_options(),
            ..Default::default()
        },
    )
    .await?;
    let first = c.render(Default::default()).await?;
    let next = c
        .render(RenderOptions::default().parameter("cutoff", ScalarValue::Float64(Some(4.))))
        .await?;
    assert_eq!(first.plots()[0].rect.width, 40.);
    assert_eq!(next.plots()[0].rect.width, 20.);
    let again = c.render(Default::default()).await?;
    assert_eq!(again.report().physical_plans, 0);
    let empty = c
        .render(RenderOptions::default().parameter("cutoff", ScalarValue::Float64(Some(10.))))
        .await?;
    assert_eq!(empty.plots()[0].rect.width, 20.);
    Ok(())
}

fn rectangles(marks: &[avenger_scenegraph::marks::mark::SceneMark], out: &mut Vec<[f32; 4]>) {
    use avenger_scenegraph::marks::mark::SceneMark;
    for m in marks {
        match m {
            SceneMark::Group(g) => rectangles(&g.marks, out),
            SceneMark::Rect(r) if r.name == "bars" => {
                let x = r.x_vec();
                let y = r.y_vec();
                let w = r.width.as_ref().unwrap().as_vec(r.len as usize, None);
                let h = r.height.as_ref().unwrap().as_vec(r.len as usize, None);
                out.extend((0..r.len as usize).map(|i| [x[i], y[i], w[i], h[i]]));
            }
            _ => {}
        }
    }
}
#[tokio::test]
async fn local_sources_named_precedence_and_portability() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let mut reference = None;
    for (name, content) in [
        ("rows.csv", "c,v\nA,2\nB,4\n"),
        ("rows.tsv", "c\tv\nA\t2\nB\t4\n"),
        ("rows.json", r#"[{"c":"A","v":2},{"c":"B","v":4}]"#),
    ] {
        std::fs::write(temp.path().join(name), content)?;
        let spec=UnitSpec::from_json(&json!({"data":{"url":name},"mark":"bar","encoding":{"x":{"field":"c"},"y":{"field":"v","type":"quantitative","aggregate":"sum"}}}).to_string())?;
        let d = compile_vegalite(&spec, &BTreeMap::new(), temp.path()).await?;
        std::fs::remove_file(temp.path().join(name))?;
        let captured = rows(&d).await?;
        assert_eq!(captured.num_rows(), 2);
        let rt = runtime()?;
        let bytes = d.to_bytes_with_codec(Arc::new(
            avenger_transform::TransformExtensionCodec::default(),
        ))?;
        let restored = ChartDefinition::from_bytes(&bytes, &rt)?;
        let c = Chart::prepare(restored, chart_options()).await?;
        assert_eq!(
            c.render(Default::default()).await?.plots()[0].rect.width,
            40.
        );
        reference = Some(captured);
    }
    let binding = reference.unwrap();
    let name = binding.schema().fields()[0].name().clone();
    let spec=UnitSpec::from_json(&json!({"data":{"name":"rows"},"datasets":{"rows":[{"unused":1}]},"mark":"bar","encoding":{"x":{"field":name},"y":{"aggregate":"count"}}}).to_string())?;
    assert!(compile_vegalite(
        &spec,
        &BTreeMap::from([("rows".into(), binding)]),
        temp.path()
    )
    .await
    .is_ok());
    Ok(())
}
#[tokio::test]
async fn explicit_transforms_prebins_sort_and_diagnostics() -> anyhow::Result<()> {
    let d=compile(&json!({"data":{"values":[{"v":1},{"v":6},{"v":9}]},"transform":[{"bin":{"step":5,"extent":[0,10]},"field":"v","as":["lo","hi"]},{"aggregate":[{"op":"count","as":"n"}],"groupby":["lo","hi"]}],"mark":"bar","encoding":{"x":{"field":"lo","type":"quantitative","bin":"binned"},"x2":{"field":"hi"},"y":{"field":"n","type":"quantitative","stack":null}}})).await?;
    assert_eq!(rows(&d).await?.num_rows(), 2);
    let c = Chart::prepare(d, chart_options()).await?;
    assert!(c.render(Default::default()).await.is_ok());
    for (spec, path) in [
        (
            json!({"data":{"values":[{"a":1}]},"mark":"bar","encoding":{"x":{"field":"missing"}}}),
            "encoding.x.field",
        ),
        (
            json!({"data":{"url":"https://example.com/data.csv"},"mark":"bar"}),
            "data.url",
        ),
        (
            json!({"data":{"values":[{"a":1}]},"mark":"bar","encoding":{"x":{"field":"a","type":"temporal"}}}),
            "encoding.x.type",
        ),
        (
            json!({"data":{"values":[{"a":1}]},"mark":"bar","transform":[{"filter":{"field":"a","gte":{"expr":"a + 1"}}}]}),
            "transform[0].filter.gte.expr",
        ),
    ] {
        let e = compile_vegalite(
            &UnitSpec::from_json(&spec.to_string())?,
            &BTreeMap::new(),
            Path::new("."),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(e.path(), path);
    }
    Ok(())
}

#[tokio::test]
async fn numeric_comparison_handles_null_nan_infinity_and_parameter_overrides() -> anyhow::Result<()>
{
    use avenger_datafusion_dataflow::arrow::{array::Float64Array, record_batch::RecordBatch};
    let b = RecordBatch::try_from_iter([(
        "v",
        Arc::new(Float64Array::from(vec![
            None,
            Some(0.),
            Some(5.),
            Some(15.),
            Some(35.),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(f64::NEG_INFINITY),
        ])) as _,
    )])?;
    let spec = UnitSpec::from_json(
        r#"{"data":{"name":"values"},"params":[{"name":"cutoff","value":0}],"transform":[{"filter":{"field":"v","gte":{"expr":"cutoff"}}}],"mark":"bar","encoding":{"y":{"aggregate":"count"}}}"#,
    )?;
    let d = compile_vegalite(
        &spec,
        &BTreeMap::from([(
            "values".into(),
            TableSnapshot::from_batches(b.schema(), vec![b])?,
        )]),
        Path::new("."),
    )
    .await?;
    let prepared = runtime()?.prepare(d.dataflow()).await?;
    let root = d.dataflow().interface().root();
    let output = root.table_output("rows")?;
    let input = root.scalar_input("cutoff")?;
    for (threshold, count) in [
        (Some(0.), 6.),
        (Some(10.), 3.),
        (Some(30.), 2.),
        (Some(50.), 1.),
        (None, 6.),
        (Some(f64::NEG_INFINITY), 7.),
        (Some(f64::NAN), 0.),
    ] {
        let inputs = prepared
            .inputs()
            .scalar(&input, ScalarValue::Float64(threshold))?
            .finish()?;
        let result = prepared.query(&[output], &[], &inputs).await?;
        let table = result.table(&output)?;
        if count == 0. {
            assert_eq!(table.num_rows(), 0);
        } else {
            let avenger_chart_definition::Encoding::Rect(r) = &plot(&d).marks[0].encoding else {
                panic!()
            };
            assert_eq!(
                floats(column(table, field(r.y2.as_ref().unwrap()))),
                vec![count],
                "threshold {threshold:?}"
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn bin_extent_follows_authored_filter_and_round_trips() -> anyhow::Result<()> {
    let v = json!({"data":{"values":[{"v":1},{"v":6},{"v":11},{"v":19}]},"params":[{"name":"cutoff","value":0}],"transform":[{"filter":{"field":"v","gte":{"expr":"cutoff"}}}],"mark":"bar","encoding":{"x":{"field":"v","bin":{"step":5}},"y":{"aggregate":"count"}}});
    let d = compile(&v).await?;
    let bytes = d.to_bytes_with_codec(Arc::new(
        avenger_transform::TransformExtensionCodec::default(),
    ))?;
    let d = ChartDefinition::from_bytes(&bytes, &runtime()?)?;
    let c = Chart::prepare(d, chart_options()).await?;
    let f = c.render(Default::default()).await?;
    assert_eq!(
        f.plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (0., 20.)
    );
    let filtered = c
        .render(RenderOptions::default().parameter("cutoff", ScalarValue::Float64(Some(10.))))
        .await?;
    assert_eq!(
        filtered.plots()[0].scales["x"].numeric_interval_domain_f64()?,
        (10., 20.)
    );
    let empty = c
        .render(RenderOptions::default().parameter("cutoff", ScalarValue::Float64(Some(50.))))
        .await?;
    assert!(empty.plots()[0].scales["x"]
        .numeric_interval_domain_f64()?
        .0
        .is_finite());
    Ok(())
}

#[tokio::test]
async fn categories_keep_typed_order_null_identity_and_literal_names() -> anyhow::Result<()> {
    for (sort, expected) in [
        (json!(null), vec!["B", "null", "NULL", "A"]),
        (json!("ascending"), vec!["NULL", "A", "B", "null"]),
        (json!("descending"), vec!["null", "B", "A", "NULL"]),
    ] {
        let d=compile(&json!({"data":{"values":[{"a.b":"B","v":1,"unused":{"nested":2}},{"a.b":"null","v":2},{"a.b":null,"v":3},{"a.b":"A","v":4}]},"mark":"bar","encoding":{"x":{"field":"a\\.b","sort":sort},"y":{"aggregate":"sum","field":"v"}}})).await?;
        let c = Chart::prepare(d, chart_options()).await?;
        let f = c.render(Default::default()).await?;
        let a = f.plots()[0].scales["x"].domain();
        let actual = (0..a.len())
            .map(|i| {
                if a.is_null(i) {
                    "NULL".into()
                } else {
                    ScalarValue::try_from_array(a, i).unwrap().to_string()
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
    Ok(())
}

#[tokio::test]
async fn empty_zero_size_and_disabled_scales_are_valid_frames() -> anyhow::Result<()> {
    for spec in [
        json!({"data":null,"mark":"bar","encoding":{"y":{"aggregate":"count"}}}),
        json!({"data":{"values":[{"v":2}]},"width":0,"mark":"bar","encoding":{"x":{"field":"v"},"y":{"aggregate":"count","field":"ignored"}}}),
        json!({"data":{"values":[{"v":-20},{"v":400}]},"mark":{"type":"bar","size":6},"encoding":{"x":{"field":"v","type":"quantitative","scale":null},"y":{"aggregate":"count","axis":null}}}),
    ] {
        let d = compile(&spec).await?;
        let c = Chart::prepare(d, chart_options()).await?;
        let f = c.render(Default::default()).await?;
        assert!(f.scenegraph().width.is_finite() && f.scenegraph().height.is_finite());
    }
    Ok(())
}

fn d3_formatting() -> avenger_scales::formatter::ScaleFormatting {
    avenger_scales::formatter::ScaleFormatting::d3(Default::default(), Default::default())
}

fn chart_options() -> avenger_chart::ChartOptions {
    avenger_chart::ChartOptions::default().with_formatting(d3_formatting())
}
