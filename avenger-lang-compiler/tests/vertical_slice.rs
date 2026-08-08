use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc};

use avenger_chart::prelude::{
    Cartesian, CompiledWidget, FacetColumn, FacetColumnSubplotChannels, IntoPlotMark, Subplot,
};
use avenger_chart_app::{ChartAppOptions, chart_avenger_app};
use avenger_chart_core::PositionBoundary;
use avenger_chart_external_test::{
    external_compound_mark::ExternalMeanPoint,
    external_coord_system::{Cube, Isometric},
    external_mark::HexBin,
};
use avenger_chart_lang_registry::{
    CoordinatePack, NativeRegistry, NativeRegistryBuilder, RegistryError, ResolvedValue, builtins,
};
use avenger_chart_schema::{
    BodyMode, ChannelSchema, KindSchema, NativeKindKey, NativeKindNamespace, NativeModuleId,
    NativeModuleImplementationProfileId, PropertySchema, ValueShape,
};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_lang_compiler::{
    ArtifactSerializationError, CompiledChartArtifact, Compiler, CompilerBuilder,
};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceOrigin,
};
use avenger_scenegraph::marks::mark::SceneMark;
use datafusion::{
    arrow::datatypes::DataType, logical_expr::lit, prelude::SessionContext, scalar::ScalarValue,
};
use indexmap::IndexMap;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn assert_expansion_baseline(name: &str, actual: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/baselines/expansion")
        .join(name);
    if std::env::var_os("AVENGER_LANG_UPDATE_BASELINES").is_some() {
        fs::write(&path, actual).unwrap();
    }
    assert_eq!(actual, fs::read_to_string(path).unwrap());
}

fn source_compiler(source: &str, registry: Option<Arc<NativeRegistry>>) -> Compiler {
    let origin = SourceOrigin::File("/project/chart.avenger".into());
    let loader = Arc::new(
        InMemorySourceLoader::default().with_source(LoadedSource::new(
            origin,
            source,
            ContentVersion::new("sha256:vertical-slice"),
        )),
    );
    let mut builder = Compiler::builder()
        .project_root("/project")
        .source_loader(loader as Arc<dyn SourceLoader>);
    if let Some(registry) = registry {
        builder = builder.native_registry(registry);
    }
    builder.build().unwrap()
}

#[tokio::test]
async fn double_quoted_channel_columns_preserve_arrow_field_case() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [ { Horsepower: 130; Miles_per_Gallon: 18.0; } ];
          }
          mark group as plot {
            mark symbol as points {
              x: encoded "Horsepower";
              y: encoded "Miles_per_Gallon";
            }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let mark = &artifact.compiled_plot().marks()[0];
    assert_eq!(
        mark.data_context().encoding("x").as_deref(),
        Some("Horsepower")
    );
    assert_eq!(
        mark.data_context().encoding("y").as_deref(),
        Some("Miles_per_Gallon")
    );
}

#[tokio::test]
async fn configured_channel_band_reaches_the_compiled_position_boundary() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [ { category: 'A'; value: 2.0; } ];
          }
          mark rect as bars {
            x: encoded "category";
            x2: encoded channel.x { band: 1.0; }
            y: encoded "value";
            y2: encoded 0.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let mark = &artifact.compiled_plot().marks()[0];
    assert!(matches!(
        mark.data_context()
            .channel("x2")
            .and_then(|channel| channel.get_position_boundary()),
        Some(PositionBoundary::BandExpr { .. })
    ));
}

#[tokio::test]
async fn scalar_params_use_datafusion_planned_arrow_types() {
    let source = r#"avenger 1;
        chart zerod as chart {
          param 1 as integer;
          param 1.5 as decimal;
          param -0.0 as negative_zero;
          param 'hello' as string;
          param true as boolean;
          param 1e2 as exponent;
          param CASE WHEN true THEN 1 ELSE 2 END as conditional;
          param 1 + 2 as arithmetic;
          param upper('hello') as udf_string;
          param arrow_cast('abc', 'Binary') as binary;
          param CAST('2026-08-02' AS DATE) as date;
          param CAST(NULL AS DOUBLE) as typed_null;
          param $lower + 10 as upper;
          param 2 as lower;
        }"#;
    let analysis = source_compiler(source, None)
        .analyze_module("chart.avenger")
        .await
        .unwrap();
    let project = analysis.resolved_module_graph.as_deref().unwrap();
    let data_type = |name: &str| {
        let param = project
            .params
            .values()
            .find(|param| param.source_name == name)
            .unwrap();
        analysis.param_types.get(&param.id).unwrap().clone()
    };
    assert_eq!(data_type("integer"), DataType::Int64);
    assert_eq!(data_type("decimal"), DataType::Decimal128(2, 1));
    assert_eq!(data_type("negative_zero"), DataType::Float64);
    assert_eq!(data_type("string"), DataType::Utf8);
    assert_eq!(data_type("boolean"), DataType::Boolean);
    assert_eq!(data_type("exponent"), DataType::Decimal128(1, -2));
    assert_eq!(data_type("conditional"), DataType::Int64);
    assert_eq!(data_type("arithmetic"), DataType::Int64);
    assert_eq!(data_type("udf_string"), DataType::Utf8);
    assert_eq!(data_type("binary"), DataType::Binary);
    assert_eq!(data_type("date"), DataType::Date32);
    assert_eq!(data_type("typed_null"), DataType::Float64);
    assert_eq!(data_type("lower"), DataType::Int64);
    assert_eq!(data_type("upper"), DataType::Int64);

    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let defaults = artifact.compiled_plot().get_default_params();
    assert_eq!(defaults["typed_null"].data_type(), DataType::Float64);
    assert!(defaults["typed_null"].is_null());
}

#[tokio::test]
async fn scalar_param_rejects_an_untyped_null_initializer() {
    let failure = source_compiler(
        "avenger 1; chart zerod as chart { param NULL as missing_type; }",
        None,
    )
    .check_module("chart.avenger")
    .await
    .unwrap_err();
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-PARAM-001"),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn inferred_param_must_match_a_native_fixed_type_requirement() {
    let failure = source_compiler(
        r#"avenger 1;
        chart zerod as chart {
          param 2.5 as threshold;
          widget slider as input {
            default: 2.5;
            max: 10.0;
            min: 0.0;
            position: bottom;
            value_param: $threshold;
          }
        }"#,
        None,
    )
    .check_module("chart.avenger")
    .await
    .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "AVENGER-PARAM-002"
                && (diagnostic.primary.message.contains("Float64")
                    || diagnostic.message.contains("Float64"))
                && (diagnostic.primary.message.contains("Decimal128")
                    || diagnostic.message.contains("Decimal128"))
        }),
        "{:#?}",
        failure.diagnostics
    );
}

fn optional_channel(name: &str, docs: &str) -> ChannelSchema {
    ChannelSchema {
        name: name.to_string(),
        required: false,
        shape: ValueShape::SqlExpression,
        item_type: None,
        docs: docs.to_string(),
    }
}

fn scene_has_group(marks: &[SceneMark], name: &str) -> bool {
    marks.iter().any(|mark| {
        matches!(mark, SceneMark::Group(group) if group.name == name || scene_has_group(&group.marks, name))
    })
}

fn collect_symbol_positions_and_fills(
    marks: &[SceneMark],
    origin: [f32; 2],
    output: &mut Vec<([f32; 2], [f32; 4])>,
) {
    for mark in marks {
        match mark {
            SceneMark::Group(group) => collect_symbol_positions_and_fills(
                &group.marks,
                [origin[0] + group.origin[0], origin[1] + group.origin[1]],
                output,
            ),
            SceneMark::Symbol(symbol) => output.extend(
                symbol
                    .x_iter()
                    .zip(symbol.y_iter())
                    .zip(symbol.fill_iter())
                    .map(|((x, y), fill)| {
                        ([origin[0] + x, origin[1] + y], fill.color_or_transparent())
                    }),
            ),
            _ => {}
        }
    }
}

fn composed_registry() -> Arc<NativeRegistry> {
    let mut builder = NativeRegistryBuilder::new(1, "phase5-downstream-fixture");
    builtins::register_bootstrap_builtins(&mut builder).unwrap();
    let mut module = builder
        .native_module(
            NativeModuleId::new("native:com.acme.compiler-fixture@1").unwrap(),
            "Compiler downstream-extension fixture.",
            NativeModuleImplementationProfileId::new("compiler-fixture-rust-v1").unwrap(),
        )
        .unwrap();
    module
        .registry()
        .register_mark::<Cartesian>(
            "cartesian",
            "external_hexbin",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "external_hexbin"),
                "A Cartesian primitive mark implemented by a downstream crate.",
            )
            .channel(optional_channel("x", "Optional horizontal position."))
            .channel(optional_channel("y", "Optional vertical position.")),
            |declaration| {
                let mut mark = HexBin::<Cartesian>::new().x(1.0).y(1.0);
                if let Some(name) = &declaration.source_name {
                    mark = mark.id(name.clone());
                }
                Ok(mark.into_plot_marks())
            },
        )
        .unwrap();
    module
        .registry()
        .register_mark::<Cartesian>(
            "cartesian",
            "external_mean_point",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "external_mean_point"),
                "An aggregate-backed compound mark implemented by a downstream crate.",
            )
            .property(
                "category",
                PropertySchema::optional(
                    ValueShape::SqlExpression,
                    "Optional category expression.",
                ),
            )
            .property(
                "value",
                PropertySchema::optional(ValueShape::SqlExpression, "Optional value expression."),
            ),
            |_| Ok(ExternalMeanPoint::new(lit("all"), lit(2.0)).into_plot_marks()),
        )
        .unwrap();
    module
        .registry()
        .register_mark::<Cartesian>(
            "cartesian",
            "failing_external_mark",
            KindSchema::new(
                NativeKindKey::mark("cartesian", "failing_external_mark"),
                "A deterministic downstream lowerer failure fixture.",
            ),
            |_| {
                Err(RegistryError::Lowering {
                    kind: "failing_external_mark".to_string(),
                    message: "intentional downstream lowerer failure".to_string(),
                })
            },
        )
        .unwrap();

    let isometric = CoordinatePack::new(
        "external_isometric",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
            "A downstream isometric coordinate system.",
        )
        .body_mode(BodyMode::Mixed)
        .property(
            "angle",
            PropertySchema::optional(ValueShape::Number, "Projection angle in radians."),
        ),
        |_| Ok(Isometric::new()),
    )
    .mark(
        "external_cube",
        KindSchema::new(
            NativeKindKey::mark("external_isometric", "external_cube"),
            "A cube mark implemented by a downstream crate.",
        )
        .channel(optional_channel("iso_x", "Optional isometric x position."))
        .channel(optional_channel("iso_y", "Optional isometric y position."))
        .channel(optional_channel("iso_z", "Optional isometric z position.")),
        |declaration| {
            let mut mark = Cube::<Isometric>::new().iso_x(1.0).iso_y(2.0).iso_z(3.0);
            if let Some(name) = &declaration.source_name {
                mark = mark.id(name.clone());
            }
            Ok(mark.into_plot_marks())
        },
    );
    module
        .registry()
        .register_coordinate_pack(isometric)
        .unwrap();

    let container = CoordinatePack::new(
        "external_facet_column",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
            "A downstream container for mixed-coordinate child plots.",
        )
        .body_mode(BodyMode::Mixed),
        |_| Ok(FacetColumn),
    )
    .child_plots(|plot, child, placement, _parent| {
        let column = match placement.properties.get("column") {
            Some(ResolvedValue::Expr(expr)) => expr.clone(),
            Some(ResolvedValue::String(value)) => lit(value.clone()),
            _ => {
                return Err(RegistryError::InvalidPropertyType {
                    property: "column".to_string(),
                    expected: "a scalar expression".to_string(),
                });
            }
        };
        Ok(plot.mark(Subplot::<FacetColumn>::new(child).column(column)))
    });
    module
        .registry()
        .register_coordinate_pack(container)
        .unwrap();
    for (name, key, docs) in [
        (
            "hexbin",
            NativeKindKey::mark("cartesian", "external_hexbin"),
            "Downstream Cartesian hexbin mark.",
        ),
        (
            "mean_point",
            NativeKindKey::mark("cartesian", "external_mean_point"),
            "Downstream Cartesian aggregate mark.",
        ),
        (
            "failing_mark",
            NativeKindKey::mark("cartesian", "failing_external_mark"),
            "Deterministically failing downstream mark.",
        ),
        (
            "isometric",
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_isometric"),
            "Downstream isometric coordinate.",
        ),
        (
            "cube",
            NativeKindKey::mark("external_isometric", "external_cube"),
            "Downstream isometric cube mark.",
        ),
        (
            "facet_column",
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
            "Downstream facet-column coordinate.",
        ),
    ] {
        module.export(name, key, docs).unwrap();
    }
    module.finish().unwrap();
    Arc::new(builder.build().unwrap())
}

#[tokio::test]
async fn vertical_slice_sql_aggregate_pipeline_propagates_schema_and_evaluates() {
    let root = fixture("02_sql_pipeline");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let artifact = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let analysis = compiler
        .analyze_module(root.join("chart.avenger"))
        .await
        .unwrap();
    let sql_schema = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| dataset.schema.as_ref())
        .find(|schema| schema.field_with_name("doubled").is_ok())
        .expect("SQL transform output schema");
    assert_eq!(
        sql_schema.field_with_name("category").unwrap().data_type(),
        &DataType::Utf8
    );
    assert_eq!(
        sql_schema.field_with_name("doubled").unwrap().data_type(),
        &DataType::Float64
    );
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn all_projection_list_native_transforms_compile_and_evaluate() {
    let root = fixture("13_projection_transforms");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    for chart in [
        "aggregate_chart",
        "join_aggregate_chart",
        "scalar_aggregate_chart",
        "calculate_chart",
        "window_chart",
        "select_chart",
    ] {
        let artifact = compiler
            .compile_chart(root.join("charts.avenger"), Some(chart))
            .await
            .unwrap_or_else(|failure| panic!("{chart}: {:#?}", failure.diagnostics));
        let evaluated = artifact
            .compiled_plot()
            .evaluate(&SessionContext::new(), None)
            .await
            .unwrap_or_else(|error| panic!("{chart}: {error}"));
        assert!(!evaluated.scene_graph.marks.is_empty(), "{chart}");
    }
}

#[tokio::test]
async fn legend_overlay_compiles_nested_groups_without_inheriting_chart_rows() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: {
    values: [
      { x: 1.0; y: 2.0; value: 5.0; },
      { x: 2.0; y: 3.0; value: 8.0; }
    ];
  }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
    fill: encoded "value" {
      legend: {
        overlay: {
          mark group as thresholds {
            data: { values: [{ lo: 2.0; hi: 7.0; }]; }
            mark rect as band {
              x: encoded 0.0;
              x2: encoded 1.0;
              y: encoded "lo";
              y2: encoded "hi";
              fill: direct 'rgba(37, 99, 235, 0.20)';
            }
          }
          mark rule as midpoint {
            x: encoded 0.0;
            x2: encoded 1.0;
            y: encoded 5.0;
            stroke: direct '#1d4ed8';
          }
        }
      }
    }
  }
}"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    assert!(scene_has_group(
        &evaluated.scene_graph.marks,
        "fill-colorbar-overlays"
    ));
}

#[tokio::test]
async fn native_surface_pipeline_remains_one_parent_stage_and_exports_typed_outputs() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [
              { category: 'A'; amount: 2.0; },
              { category: 'A'; amount: 3.0; }
            ];
          }
          transform pipeline as summarized {
            scope: level(2);
            output totals.total as total;
            transform aggregate as totals {
              expressions: sum("amount") AS total;
            }
          }
          mark symbol as point {
            x: encoded summarized.total;
            y: encoded summarized.total;
            size: direct 80.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert_eq!(
        json.matches("\"type\":\"pipeline\"").count(),
        1,
        "the DSL pipeline must remain one compiled parent stage"
    );
    assert!(json.contains("\"stages\":["));
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn native_surface_explicit_channel_modes_map_to_runtime_policy() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          mark symbol as point {
            x: encoded 1.0;
            y: direct 2.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(
        json.contains("\"Scaled\""),
        "encoded channel literal did not lower as scaled: {json}"
    );
    assert!(
        json.contains("\"Value\""),
        "direct channel literal did not bypass scaling: {json}"
    );
}

#[tokio::test]
async fn explicit_channel_modes_control_transform_output_metadata() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ amount: 1.0; }, { amount: 4.0; }, { amount: 9.0; }]; }
  transform bin as bins {
    field: "amount";
    maxbins: 3;
  }
  mark rect as bars {
    x: encoded bins.start;
    x2: direct bins.end;
    y: direct 0.0;
    y2: direct 20.0;
  }
}
"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(
        json.contains("\"Scaled\""),
        "encoded transform output did not preserve channel metadata: {json}"
    );
    assert!(
        json.contains("\"Value\""),
        "direct transform output did not discard scale metadata: {json}"
    );
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn mixed_conditional_channel_modes_lower_in_source_order() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: {
    values: [
      { x: 1.0; category: 'A'; selected: false; alert: true; },
      { x: 2.0; category: 'B'; selected: true; alert: false; }
    ];
  }
  mark symbol as points {
    x: encoded "x";
    y: direct "x" * 20.0;
    fill: encoded "category" {
      when { predicate: "selected"; direct: '#2563eb'; }
      when { predicate: "alert"; encoded: "category"; }
      otherwise: { direct: '#94a3b8'; }
      legend: { title: 'Category'; }
    }
  }
}
"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"Conditional\""), "{json}");
    assert!(json.contains("\"Value\""), "{json}");
    assert!(json.contains("\"Scaled\""), "{json}");
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn native_surface_common_mark_state_and_channel_domain_policy_are_preserved() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: { values: [{ x: 1.0; y: 2.0; category: 'A'; }]; }
          mark symbol as point {
            x: encoded "x" {
              domain_contribution: exclude;
              scale: linear {
                domain: [0.0, 2.0];
              }
            }
            y: encoded "y";
            fill_pattern: encoded "category" {
              domain_contribution: exclude;
            }
            visible: true;
            details: [x, category];
            zindex: 7;
            facet_data_scope: level(2);
            geometry_space: display;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"details\":[\"x\",\"category\"]"));
    assert!(json.contains("\"zindex\":7"));
    assert!(json.contains("\"geometry_space\":\"display\""));
    assert_eq!(
        json.matches("\"scale_domain_inference\":\"exclude\"")
            .count(),
        3,
        "x plus the pattern channel and its scalar surrogate should preserve exclusion: {json}"
    );
}

#[tokio::test]
async fn native_surface_statistical_compound_marks_lower_with_public_parts() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [
              { category: 'A'; amount: 1.0; },
              { category: 'A'; amount: 2.0; },
              { category: 'A'; amount: 3.0; },
              { category: 'B'; amount: 2.0; },
              { category: 'B'; amount: 4.0; },
              { category: 'B'; amount: 6.0; }
            ];
          }
          mark box_plot as summary {
            x: encoded "category";
            y: encoded "amount";
            orientation: vertical;
            extent: 1.5;
            fill: encoded "category";
          }
          mark violin as distribution {
            x: encoded "category";
            y: encoded "amount";
            orientation: vertical;
            bandwidth: 0.0;
            steps: 40;
            density_extent: [0.0, 7.0];
            counts: false;
            density_extent_resolve: shared;
            density_data_scope: level(1);
            width: 0.8;
            width_normalization: per_violin;
            fill: encoded "category";
          }
        }"#;
    let compiler = source_compiler(source, None);
    let schemas = compiler.language_host().registry().snapshot();
    let box_plot = schemas
        .entries
        .get(&avenger_chart_schema::NativeKindKey::mark(
            "cartesian",
            "box_plot",
        ))
        .expect("box_plot schema");
    assert_eq!(
        box_plot
            .parts
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "box",
            "lower_cap",
            "median",
            "outliers",
            "upper_cap",
            "whiskers",
        ]
    );
    let violin = schemas
        .entries
        .get(&avenger_chart_schema::NativeKindKey::mark(
            "cartesian",
            "violin",
        ))
        .expect("violin schema");
    assert!(violin.parts.contains_key("body"));

    let artifact = compiler.compile_chart("chart.avenger", None).await.unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("summary.box"), "{json}");
    assert!(json.contains("distribution.body"), "{json}");
    assert!(json.contains("\"scope\":{\"level\":1}"));
}

#[tokio::test]
async fn native_surface_registered_selection_tool_lowers_from_dsl() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: { values: [{ id: 'a'; x: 1.0; y: 2.0; }]; }
          selection as picked {
            empty: none;
            combine: union;
          }
          tool point_selection as pick_points {
            selection: picked;
            fields: [id];
            shift_toggle: true;
            double_click_clear: true;
          }
          mark symbol as points {
            x: encoded "x";
            y: encoded "y";
            details: [id];
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let behavior = artifact
        .compiled_plot()
        .tool_behaviors()
        .iter()
        .find(|behavior| {
            behavior
                .exports
                .iter()
                .any(|export| export.alias == "selection")
        })
        .expect("point-selection behavior");
    assert!(
        behavior
            .exports
            .iter()
            .any(|export| export.alias == "enabled")
    );
    assert!(!artifact.compiled_plot().event_bindings().is_empty());
}

#[tokio::test]
async fn native_surface_tool_instances_own_distinct_generated_state_and_exports() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ id: 'a'; x: 1.0; y: 2.0; }]; }
  selection as first_selection { empty: none; }
  selection as second_selection { empty: none; }
  tool point_selection as first { selection: first_selection; fields: [id]; }
  tool point_selection as second { selection: second_selection; fields: [id]; }
  mark symbol as points { x: encoded "x"; y: encoded "y"; details: [id]; }
}"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();

    let behaviors = artifact
        .compiled_plot()
        .tool_behaviors()
        .iter()
        .filter(|behavior| matches!(behavior.source_id.as_str(), "first" | "second"))
        .collect::<Vec<_>>();
    assert_eq!(behaviors.len(), 2);
    assert_ne!(behaviors[0].instance_id, behaviors[1].instance_id);

    let mut generated_runtime_ids = std::collections::BTreeSet::new();
    let mut generated_migration_keys = std::collections::BTreeSet::new();
    let mut selection_runtime_ids = std::collections::BTreeSet::new();
    for instance in ["first", "second"] {
        let enabled = format!("chart.{instance}.enabled");
        let enabled_runtime_id = artifact
            .interface
            .public_targets
            .get(&enabled)
            .unwrap_or_else(|| panic!("missing {enabled}: {:?}", artifact.interface));
        assert!(generated_runtime_ids.insert(enabled_runtime_id.clone()));
        let binding = artifact
            .interface
            .params
            .values()
            .find(|binding| &binding.runtime_id == enabled_runtime_id)
            .expect("generated enabled param binding");
        assert!(
            generated_migration_keys.insert(
                binding
                    .migration_key
                    .clone()
                    .expect("generated tool state migration key")
            )
        );

        let selection = format!("chart.{instance}.selection");
        let selection_runtime_id = artifact
            .interface
            .public_targets
            .get(&selection)
            .unwrap_or_else(|| panic!("missing {selection}: {:?}", artifact.interface));
        assert!(selection_runtime_ids.insert(selection_runtime_id.clone()));
    }
    assert_eq!(generated_runtime_ids.len(), 2);
    assert_eq!(generated_migration_keys.len(), 2);
    assert_eq!(selection_runtime_ids.len(), 2);

    for behavior in behaviors {
        let aliases = behavior
            .exports
            .iter()
            .map(|export| export.alias.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(aliases, ["enabled", "selection"].into());
    }
}

#[tokio::test]
async fn native_surface_concat_cells_lower_as_mixed_coordinate_subplots() {
    let source = r#"avenger 1;
        chart hconcat as chart {
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          spacing: 12;
          widths: [fr(2), px(180)];

          cell cartesian as left {
            label: 'Left cell';
            mark symbol { x: encoded "x"; y: encoded "y"; }
          }
          cell polar as right {
            mark symbol { theta: encoded "x"; r: encoded "y"; size: direct 100; }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(artifact.compiled_plot().marks().len(), 2);
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"key\":\"left\""), "{json}");
    assert!(json.contains("\"type\":\"HConcat\""), "{json}");
}

#[tokio::test]
async fn native_surface_vconcat_grid_and_wrap_lower_through_registered_packs() {
    for (coordinate, properties, placement, expected_type) in [
        ("vconcat", "spacing: 4; heights: [auto];", "", "VConcat"),
        (
            "grid_concat",
            "rows: 1; columns: 1; column_widths: [fr(1)]; row_heights: [px(120)];",
            "at { row: 0; column: 0; }",
            "GridConcat",
        ),
        (
            "wrap_concat",
            "columns: 1; axis_guide_visibility: outer_edges;",
            "",
            "WrapConcat",
        ),
    ] {
        let source = format!(
            r#"avenger 1;
            chart {coordinate} as chart {{
              data: {{ values: [{{ x: 1.0; y: 2.0; }}]; }}
              {properties}
              cell cartesian {placement} {{
                mark symbol {{ x: encoded "x"; y: encoded "y"; }}
              }}
            }}"#
        );
        let artifact = source_compiler(&source, None)
            .compile_chart("chart.avenger", None)
            .await
            .unwrap_or_else(|error| panic!("{coordinate}: {error:?}"));
        let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
        assert!(
            json.contains(&format!("\"type\":\"{expected_type}\"")),
            "{coordinate}: {json}"
        );
    }
}

#[tokio::test]
async fn native_surface_facets_lower_configured_dimensions_and_nested_cells() {
    for (coordinate, dimensions, expected_types) in [
        (
            "facet",
            r#"row: "region" {
                title: 'Region';
                slots: shared;
                empty_cells: hole;
                order_by: sum("value");
                order: desc;
              }
              column: "segment" {
                slots: free;
                empty_cells: empty_subplot;
              }"#,
            &["FacetRow", "FacetColumn"][..],
        ),
        (
            "facet_wrap",
            r#"facet: "region" {
                responsive_columns: 190;
                slots: shared;
                order_by: median("value");
                order: desc;
              }"#,
            &["FacetWrap"][..],
        ),
    ] {
        let source = format!(
            r#"avenger 1;
            chart {coordinate} as chart {{
              data: {{ values: [
                {{ region: 'east'; segment: 'a'; category: 'x'; value: 2.0; }},
                {{ region: 'west'; segment: 'b'; category: 'y'; value: 3.0; }}
              ]; }}
              {dimensions}
              cell cartesian as leaf {{
                mark rect {{ x: encoded "category"; y: encoded "value"; }}
              }}
            }}"#
        );
        let artifact = source_compiler(&source, None)
            .compile_chart("chart.avenger", None)
            .await
            .unwrap_or_else(|error| panic!("{coordinate}: {error:?}"));
        let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
        for expected in expected_types {
            assert!(
                json.contains(expected),
                "{coordinate}: missing {expected}: {json}"
            );
        }
    }
}

#[tokio::test]
async fn native_surface_repeat_grid_and_wrap_lower_reserved_repeat_values() {
    let grid = r#"avenger 1;
        chart repeat_grid as chart {
          data: { values: [{ mpg: 21.0; hp: 110.0; weight: 2500.0; accel: 12.0; }]; }
          variable row mpg { expr: "mpg"; title: 'MPG'; }
          variable row hp { expr: "hp"; title: 'Horsepower'; }
          variable column weight { expr: "weight"; title: 'Weight'; }
          variable column accel { expr: "accel"; title: 'Acceleration'; }
          domain_coordination: matrix;

          cell cartesian {
            when: repeat.row_id <> repeat.column_id;
            mark symbol { x: encoded repeat.column; y: encoded repeat.row; }
          }
          cell zerod {
            when: repeat.row_id = repeat.column_id;
            mark text { text: encoded repeat.row_title; }
          }
        }"#;
    let artifact = source_compiler(grid, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"origin\":\"repeat_grid\""), "{json}");

    let wrap = r#"avenger 1;
        chart repeat_wrap as chart {
          data: { values: [{ mpg: 21.0; hp: 110.0; }]; }
          variable item mpg { expr: "mpg"; }
          variable item hp { expr: "hp"; }
          responsive_columns: 180;
          cell cartesian {
            mark symbol { x: encoded repeat.item; y: encoded repeat.item; }
          }
        }"#;
    let artifact = source_compiler(wrap, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"origin\":\"repeat_wrap\""), "{json}");
}

#[tokio::test]
async fn native_surface_positioned_subplot_marks_embed_mixed_coordinate_plots() {
    for (coordinate, placement, child_coordinate, child_channels) in [
        (
            "cartesian",
            r#"x: encoded avg("x") { scale: linear { domain: [0.0, 2.0]; } }
               y: encoded avg("y") { scale: linear { domain: [0.0, 4.0]; } }
               key: encoded "category"; width: 120; height: 90;"#,
            "polar",
            r#"r: encoded "r"; theta: encoded "theta";"#,
        ),
        (
            "polar",
            r#"r: encoded avg("r"); theta: encoded avg("theta"); key: encoded "category"; width: 100; height: 80;"#,
            "cartesian",
            r#"x: encoded "x"; y: encoded "y";"#,
        ),
    ] {
        let source = format!(
            r#"avenger 1;
            chart {coordinate} as chart {{
              data: {{ values: [{{ x: 1.0; y: 2.0; r: 3.0; theta: 0.5; category: 'a'; }}]; }}
              mark subplot as inset {{
                {placement}
                plot {child_coordinate} {{
                  mark symbol {{
                    {child_channels}
                  }}
                }}
              }}
            }}"#
        );
        let artifact = source_compiler(&source, None)
            .compile_chart("chart.avenger", None)
            .await
            .unwrap_or_else(|error| panic!("{coordinate}: {error:?}"));
        let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
        assert!(
            json.contains("CompiledPositionedSubplot"),
            "{coordinate}: {json}"
        );
    }
}

#[tokio::test]
async fn native_surface_coordinate_family_project_compiles_all_stock_roots() {
    let root = fixture("09_native_coordinate_families");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let mut charts = BTreeMap::new();
    for chart in [
        "polar", "parallel", "geo", "treemap", "concat", "repeat", "facet", "subplot",
    ] {
        charts.insert(
            chart,
            compiler
                .compile_chart(root.join(format!("{chart}.avenger")), None)
                .await
                .unwrap(),
        );
    }
    for (chart, expected) in [
        ("polar", "CompiledPolarSymbol"),
        ("parallel", "CompiledParallelLine"),
        ("geo", "CompiledGeoSymbol"),
        ("treemap", "CompiledTreeRect"),
        ("concat", "HConcat"),
        ("repeat", "WrapConcat"),
        ("facet", "FacetWrap"),
        ("subplot", "CompiledPositionedSubplot"),
    ] {
        let json = serde_json::to_string(charts[chart].compiled_plot()).unwrap();
        assert!(
            json.contains(expected),
            "{chart}: missing {expected}: {json}"
        );
    }
    let parallel = serde_json::to_value(charts["parallel"].compiled_plot()).unwrap();
    assert_eq!(
        parallel["coord_transform"]["order"],
        serde_json::json!(["horsepower", "mileage"]),
        "parallel dimension order must survive language lowering"
    );
    assert_eq!(
        parallel["coord_transform"]["dimensions"][0]["id"],
        "horsepower"
    );
    assert!(
        !parallel["coord_transform"]["dimensions"][0]["axis"].is_null(),
        "configured sparse dimension must carry its axis"
    );
    assert!(
        parallel["coord_transform"]["dimensions"][1]["axis"].is_null(),
        "empty sparse dimension must retain default axis configuration"
    );
}

#[tokio::test]
async fn parallel_sparse_frame_configuration_validates_mark_owned_dimensions() {
    let valid = r#"avenger 1;
chart parallel as chart {
  dimensions: {
    first: { axis: { title: 'First'; visible: true; } }
    second: {}
  }
  order: [first, second];
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  mark parallel_line {
    dimensions: { first: encoded "x"; second: encoded "y"; }
  }
  mark parallel_symbol {
    dimensions: { first: encoded "x" + 1; second: encoded "y"; }
  }
}"#;
    source_compiler(valid, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();

    let unbound = valid.replace("second: {}", "second: {}\n    unbound: {} ");
    assert!(
        source_compiler(&unbound, None)
            .compile_chart("chart.avenger", None)
            .await
            .is_err(),
        "a configured frame id must be bound by at least one parallel mark"
    );

    let invalid_order = valid.replace(
        "order: [first, second];",
        "order: [first, second, missing];",
    );
    assert!(
        source_compiler(&invalid_order, None)
            .compile_chart("chart.avenger", None)
            .await
            .is_err(),
        "explicit order must exactly match discovered dimension ids"
    );
}

#[tokio::test]
async fn native_surface_store_backed_mark_uses_runtime_relation_without_metadata_columns() {
    let root = fixture("10_native_surface_contracts");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("store_backed.avenger"), None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"store_data\":{"), "{json}");
    assert!(!json.contains("__avenger_store_owner_key"), "{json}");
    assert!(!json.contains("__avenger_store_revision"), "{json}");

    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.groups().is_empty());
}

#[tokio::test]
async fn vertical_slice_title_subtitle_and_fixed_auto_layout_lower_through_registry() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          title: 'Phase 5 title';
          subtitle: 'Registry-backed subtitle';
          layout: {
            canvas: { width: 420; height: 310; }
            plot: auto;
          }
          data: { values: [{ category: 'A'; x: 1.0; y: 2.0; }]; }
          mark symbol as point {
            x: encoded "x" { scale: linear { domain: [0.0, 4.0]; } axis: { title: 'X'; } }
            y: encoded "y" { scale: linear; axis: { title: 'Y'; } }
            fill: encoded "category" {
              scale: ordinal { domain: ['A', 'B']; range: ['#5778a4', '#e49444']; }
              legend: { title: 'Category'; position: right; }
            }
            size: direct 80.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert_eq!(
        (evaluated.scene_graph.width, evaluated.scene_graph.height),
        (420.0, 310.0)
    );
}

#[tokio::test]
async fn direct_canvas_params_remain_available_for_host_resize_binding() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          param 640.0 as canvas_width;
          param 420.0 as canvas_height;
          layout: {
            canvas: { width: $canvas_width; height: $canvas_height; }
            plot: auto;
          }
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          mark symbol { x: encoded "x"; y: encoded "y"; }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();

    assert_eq!(
        artifact.compiled_plot().get_layout_spec().resize_params(),
        avenger_chart::layout::ChartResizeParams {
            width_param: Some("canvas_width".to_string()),
            height_param: Some("canvas_height".to_string()),
        }
    );
}

#[tokio::test]
async fn native_surface_chart_theme_time_and_format_context_lower() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          theme css: 'chart { font-size: 15px; }';
          theme css: 'mark.symbol { fill-opacity: 0.7; }';
          time: { timezone: 'America/New_York'; week_start: monday; }
          format: {
            number_locale: 'de-DE';
            datetime_locale: 'fr-FR';
            datetime_timezone: 'Europe/Paris';
          }
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          mark symbol { x: encoded "x"; y: encoded "y"; }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    for expected in [
        "America/New_York",
        "de-DE",
        "fr-FR",
        "Europe/Paris",
        "font-size: 15px",
        "fill-opacity: 0.7",
    ] {
        assert!(json.contains(expected), "missing {expected}: {json}");
    }

    let file_source = r#"avenger 1;
        chart cartesian as chart {
          theme css from 'theme.css';
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          mark symbol { x: encoded "x"; y: encoded "y"; }
        }"#;
    let root = std::env::temp_dir().join(format!("avenger-theme-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("chart.avenger"), file_source).unwrap();
    std::fs::write(
        root.join("theme.css"),
        "chart { font-family: 'Theme File'; }",
    )
    .unwrap();
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("Theme File"), "{json}");
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn native_surface_component_exports_preserve_parts_without_private_paths() {
    let root = fixture("10_native_surface_contracts");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("theme_parts.avenger"), None)
        .await
        .unwrap();

    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("theme_parts.pair.glyph")
    );
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("theme_parts.pair.annotation")
    );
    assert!(
        artifact
            .interface
            .public_targets
            .keys()
            .all(|path| { !path.contains(".body.") && !path.ends_with(".point") })
    );

    let glyph = artifact
        .compiled_plot()
        .marks()
        .iter()
        .find(|mark| {
            mark.state()
                .identity
                .public_aliases
                .iter()
                .any(|alias| alias == "pair.glyph")
        })
        .expect("exported component glyph");
    let component = glyph
        .state()
        .identity
        .component
        .as_ref()
        .expect("component part provenance");
    assert_eq!(component.component_kind, "point_pair");
    assert_eq!(component.component_id.as_deref(), Some("pair"));
    assert_eq!(component.part_alias, "glyph");

    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("point_pair::part(glyph)"), "{json}");
}

#[tokio::test]
async fn native_surface_public_routing_hoists_and_exports_exact_targets() {
    let root = fixture("10_native_surface_contracts");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("public_routing.avenger"), None)
        .await
        .unwrap();

    for path in ["public_routing.layer.visible", "public_routing.layer.label"] {
        assert!(
            artifact.interface.public_targets.contains_key(path),
            "missing {path}: {:?}",
            artifact.interface.public_targets
        );
    }
    assert!(
        artifact
            .interface
            .public_targets
            .keys()
            .all(|path| { !path.contains(".implementation.") && !path.ends_with(".secret") })
    );

    let aliases = artifact
        .compiled_plot()
        .marks()
        .iter()
        .flat_map(|mark| mark.state().identity.public_aliases.iter().cloned())
        .collect::<Vec<_>>();
    assert!(
        aliases.contains(&"layer.visible".to_string()),
        "{aliases:?}"
    );
    assert!(aliases.contains(&"layer.label".to_string()), "{aliases:?}");
    assert!(aliases.iter().all(|path| !path.contains("implementation")));
}

#[tokio::test]
async fn native_surface_mark_adjustments_and_derived_marks_lower_in_order() {
    let root = fixture("10_native_surface_contracts");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("mark_effects.avenger"), None)
        .await
        .unwrap();

    let json = serde_json::to_value(artifact.compiled_plot()).unwrap();
    let effects = &json["marks"][0]["effects"];
    let adjustments = effects["adjustments"].as_array().unwrap();
    assert_eq!(adjustments.len(), 4);
    assert!(adjustments[0].get("Expr").is_some());
    assert_eq!(adjustments[1]["Transform"]["transform"]["type"], "nudge");
    assert_eq!(adjustments[2]["Transform"]["transform"]["type"], "jitter");
    assert_eq!(adjustments[3]["Transform"]["transform"]["type"], "dodge");
    assert_eq!(effects["derived"].as_array().unwrap().len(), 3);
    assert!(effects["derived"][0].get("Symbol").is_some());
    assert!(effects["derived"][1].get("Rule").is_some());
    assert!(effects["derived"][2].get("Text").is_some());
    assert_eq!(
        effects["derived"][2]["Text"]["assignments"][0]["data_fields"],
        serde_json::json!(["name"])
    );
    assert!(
        json["marks"][1]["effects"]["derived"][0]
            .get("Rect")
            .is_some()
    );

    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
    let scene = serde_json::to_string(&evaluated.scene_graph).unwrap();
    assert!(
        scene.contains("text"),
        "derived text missing from scene: {scene}"
    );
}

#[tokio::test]
async fn vertical_slice_widget_is_opaque_and_exported_state_drives_the_mark() {
    let root = fixture("03_widget_vertical_slice");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();

    assert_eq!(artifact.compiled_plot().widgets().len(), 1);
    let attachment = &artifact.compiled_plot().widgets()[0];
    let CompiledWidget::Composed(widget) = &attachment.widget else {
        panic!("the registered widget must remain an opaque composed widget")
    };
    assert_eq!(widget.kind, "radio-button-list");
    assert_ne!(attachment.instance_id.as_opaque_str(), "choice");

    let export_id = artifact
        .interface
        .widget_exports
        .get("chart.choice.value")
        .expect("public widget value export");
    assert_eq!(
        artifact.interface.public_targets["chart.choice.value"],
        *export_id
    );
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("chart.choice")
    );
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("chart.selected_points.points")
    );

    let context = datafusion::prelude::SessionContext::new();
    let first = artifact
        .compiled_plot()
        .evaluate(&context, None)
        .await
        .unwrap();
    let second = artifact
        .compiled_plot()
        .evaluate(
            &context,
            Some(IndexMap::from([(
                "choice__value".to_string(),
                ScalarValue::Utf8(Some("B".to_string())),
            )])),
        )
        .await
        .unwrap();
    assert_ne!(
        serde_json::to_vec(&first.scene_graph).unwrap(),
        serde_json::to_vec(&second.scene_graph).unwrap(),
        "changing the public widget state must change the filtered mark render"
    );
}

#[tokio::test]
async fn native_surface_all_six_builtin_widgets_lower_through_one_schema_contract() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          widget checkbox as enabled {
            position: right;
            label: 'Enabled';
            default: true;
          }
          widget button as refresh {
            position: right;
            label: 'Refresh';
            variant: accent;
          }
          widget checkbox_list as regions {
            position: right;
            data: { values: [
              { value: 'east'; label: 'East'; },
              { value: 'west'; label: 'West'; }
            ]; }
          }
          widget radio_button_list as choice {
            position: right;
            data: { values: [
              { value: 'a'; label: 'A'; },
              { value: 'b'; label: 'B'; }
            ]; }
          }
          widget slider as threshold {
            position: bottom;
            min: 0.0;
            max: 10.0;
            step: 0.5;
          }
          widget text_input as query {
            position: top;
            placeholder: 'Search';
            commit: on_enter_or_blur;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(artifact.compiled_plot().widgets().len(), 6);
    let kinds = artifact
        .compiled_plot()
        .widgets()
        .iter()
        .map(|attachment| attachment.widget.kind())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "checkbox",
            "button",
            "checkbox-list",
            "radio-button-list",
            "slider",
            "text-input"
        ]
    );
}

#[tokio::test]
async fn native_surface_button_actions_preserve_order_and_shared_state_targets() {
    let source = r#"avenger 1;
        chart zerod as chart {
          param 'initial' as query;
          store as history {
            field utf8 id;
            primary_key: [id];
          }
          selection as picked {
            empty: none;
            combine: union;
          }
          widget button as clear {
            position: right;
            label: 'Clear';
            action: {
              set query to '';
              insert history {
                row { id: 'clear'; }
              }
              clear picked;
            }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let bindings = artifact.compiled_plot().param_change_bindings();
    assert_eq!(bindings.len(), 1);
    assert_eq!(bindings[0].source_param_name, "clear__activations");
    let steps = bindings[0].action.ordered_steps();
    assert_eq!(steps.len(), 3);
    assert!(matches!(
        &steps[0],
        avenger_chart_core::ChartActionStep::SetParam(assignment)
            if assignment.param_name == "query"
    ));
    assert!(matches!(
        &steps[1],
        avenger_chart_core::ChartActionStep::SetStore(assignment)
            if assignment.store_name == "history"
    ));
    assert!(matches!(
        &steps[2],
        avenger_chart_core::ChartActionStep::SetSelection(assignment)
            if assignment.selection_id == "picked"
    ));

    let bytes = artifact.to_bytes().unwrap();
    let registry = builtins::stock_registry().unwrap();
    let restored = CompiledChartArtifact::from_bytes(&bytes, &registry).unwrap();
    assert_eq!(restored.compiled_plot().param_change_bindings(), bindings);
}

#[tokio::test]
async fn native_surface_widget_disk_projects_cover_all_builtin_kinds_and_hosting_tiers() {
    let root = fixture("11_widget_surface");
    let cases = [
        ("checkbox", "checkbox", false),
        ("button", "button", false),
        ("checkbox_list", "checkbox-list", false),
        ("radio_button_list", "radio-button-list", false),
        ("slider", "slider", false),
        ("text_input", "text-input", true),
    ];
    let registry = builtins::stock_registry().unwrap();
    for (file, runtime_kind, native) in cases {
        let artifact = Compiler::builder()
            .project_root(&root)
            .build()
            .unwrap()
            .compile_chart(root.join(format!("{file}.avenger")), None)
            .await
            .unwrap_or_else(|failure| panic!("{file} failed: {:?}", failure.diagnostics));
        let [attachment] = artifact.compiled_plot().widgets() else {
            panic!("{file} must compile exactly one widget")
        };
        assert_eq!(attachment.widget.kind(), runtime_kind);
        assert_eq!(
            matches!(attachment.widget, CompiledWidget::Native(_)),
            native,
            "{file} hosting tier"
        );
        match &attachment.widget {
            CompiledWidget::Composed(widget) => {
                assert!(!widget.relative_target_paths.is_empty());
                assert!(bincode::serialize(&widget.measure).is_ok());
            }
            CompiledWidget::Native(widget) => {
                assert!(bincode::serialize(&widget.measure).is_ok());
            }
        }
        assert!(
            artifact
                .interface
                .public_targets
                .contains_key(&format!("{file}.{}", attachment.widget.id()))
        );
        let bytes = artifact.to_bytes().unwrap();
        let restored = CompiledChartArtifact::from_bytes(&bytes, &registry).unwrap();
        assert_eq!(restored.compiled_plot().widgets().len(), 1);
    }

    let radio = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("radio_button_list.avenger"), None)
        .await
        .unwrap();
    assert!(radio.interface.params.contains_key("choice_state"));
    assert!(!radio.interface.params.contains_key("choice__value"));
    let CompiledWidget::Composed(radio_widget) = &radio.compiled_plot().widgets()[0].widget else {
        panic!("radio list must use composed hosting")
    };
    assert!(radio_widget.items.is_some());
    let relational_without_order = std::fs::read_to_string(root.join("radio_button_list.avenger"))
        .unwrap()
        .replace("    order_by: [\"rank\", \"value\"];\n", "")
        .replace("chart zerod as radio_button_list", "chart zerod as chart");
    let failure = source_compiler(&relational_without_order, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap_err();
    let diagnostics = format!("{:?}", failure.diagnostics);
    assert!(
        diagnostics.contains("requires a nonempty total `order_by`"),
        "unexpected diagnostics: {diagnostics}"
    );

    let checkbox = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("checkbox.avenger"), None)
        .await
        .unwrap();
    assert!(checkbox.interface.params.contains_key("enabled_state"));
    assert!(!checkbox.interface.params.contains_key("enabled__checked"));

    let checkbox_list = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("checkbox_list.avenger"), None)
        .await
        .unwrap();
    assert!(checkbox_list.interface.selections.contains_key("regions"));
    assert_eq!(
        checkbox_list
            .compiled_plot()
            .selection_specs()
            .keys()
            .filter(|name| name.as_str() == "regions")
            .count(),
        1
    );

    let slider = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("slider.avenger"), None)
        .await
        .unwrap();
    assert!(slider.interface.params.contains_key("threshold_state"));
    assert!(!slider.interface.params.contains_key("threshold__value"));

    let text = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("text_input.avenger"), None)
        .await
        .unwrap();
    assert!(text.interface.params.contains_key("query_state"));
    assert!(!text.interface.params.contains_key("query__value"));
    assert!(text.interface.params.contains_key("query__cursor"));
    assert!(text.interface.params.contains_key("query__selected_text"));
    for name in ["query_state", "query__cursor", "query__selected_text"] {
        assert_eq!(
            text.compiled_plot().param_specs()[name].migration_key,
            text.interface.params[name].migration_key,
            "compiled widget state must retain the resolver's migration key for {name}"
        );
        assert!(text.interface.params[name].migration_key.is_some());
    }
}

#[tokio::test]
async fn native_surface_inline_view_helpers_and_local_transforms_lower() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [
              { x: 1.0; y: 2.0; },
              { x: 3.0; y: 4.0; }
            ];
          }
          mark group as viewed_points {
            view cartesian as viewport {
              x_domain: "x";
              y_domain: "y";
              stale_policy: retarget_cached;
              throttle_ms: 16;
              transform filter {
                predicate: viewport.x.pixels > 0;
              }
              mark symbol { x: encoded "x"; y: encoded "y"; }
            }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    for expected in [
        "\"kind\":\"cartesian\"",
        "\"source_name\":\"viewport\"",
        "\"stale_policy\":\"RetargetCached\"",
    ] {
        assert!(json.contains(expected), "missing {expected}: {json}");
    }

    let mark_owned = source.replace(
        r#"mark group as viewed_points {
            view cartesian as viewport {
              x_domain: "x";
              y_domain: "y";
              stale_policy: retarget_cached;
              throttle_ms: 16;
              transform filter {
                predicate: viewport.x.pixels > 0;
              }
              mark symbol { x: encoded "x"; y: encoded "y"; }
            }
          }"#,
        r#"mark symbol as viewed_point {
            x: encoded "x";
            y: encoded "y";
            view cartesian as viewport {
              x_domain: "x";
              y_domain: "y";
              transform filter {
                predicate: viewport.x.pixels > 0;
              }
            }
          }"#,
    );
    let mark_artifact = source_compiler(&mark_owned, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let mark_json = serde_json::to_string(mark_artifact.compiled_plot()).unwrap();
    assert!(mark_json.contains("viewed_point"), "{mark_json}");
    assert!(
        mark_json.contains("\"source_name\":\"viewport\""),
        "{mark_json}"
    );
}

#[tokio::test]
async fn native_surface_inline_view_raster_fixture_preserves_materialization_contract() {
    let root = fixture("07_inline_view_raster");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    for expected in [
        "CompiledCartesianUniformRaster2D",
        "\"type\":\"rasterize_2d\"",
        "RetargetCached",
        "\"raster_name\":\"raster\"",
    ] {
        assert!(json.contains(expected), "missing {expected}: {json}");
    }
    assert_eq!(artifact.compiled_plot().marks().len(), 1);
}

#[tokio::test]
async fn native_surface_geo_tile_resources_lower_through_typed_references() {
    let source = r#"avenger 1;
        chart geo as chart {
          projection: mercator;
          center_lon_lat: [-73.9857, 40.7484];
          zoom: 11;
          tiles: osm { zindex: -10; }

          resource tiles as osm {
            kind: xyz;
            url: 'https://tile.example/{z}/{x}/{y}.png';
            min_zoom: 0;
            max_zoom: 19;
            attribution: 'Example tiles';
            loading_policy: smooth_zoom;
          }
          mark symbol as station {
            lon_lat: [-73.9857, 40.7484];
            size: direct 64.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    for expected in [
        "https://tile.example/{z}/{x}/{y}.png",
        "Example tiles",
        "smooth-zoom",
        "\"zindex\":-10",
    ] {
        assert!(json.contains(expected), "missing {expected}: {json}");
    }
}

#[tokio::test]
async fn mark_channel_access_preserves_the_referenced_expression_type() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: { values: [{ row: 1; }]; }
          mark rect as interval {
            x: encoded 'a';
            x2: encoded channel.x || '-end';
            y: encoded 0.0;
            y2: encoded 1.0;
          }
        }"#;
    source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn item_data_access_preserves_struct_physical_types() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: {
            values: [{
              x: 1.0;
              y: 2.0;
              metadata: { label: 'A'; weight: 3; }
            }];
          }
          mark symbol as points {
            x: encoded "x";
            y: encoded "y";
            derive text as labels {
              text: 'present';
              x: item.channel.x;
              y: item.channel.y;
              defined: item.data."metadata" IS NOT NULL;
            }
          }
        }"#;
    source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn event_domain_facet_and_legend_property_accesses_lower() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          param 0.0 as domain_start;
          param 0.0 as domain_end;
          param '' as facet_value;
          param '' as legend_hit;
          data: {
            values: [
              { x: 1.0; y: 2.0; value: 3.0; },
              { x: 2.0; y: 3.0; value: 4.0; }
            ];
          }
          mark symbol as points {
            x: encoded "x";
            y: encoded "y";
            fill: encoded "value" {
              legend: { title: 'Value'; }
            }
          }
          on cursor_moved as inspect_plot {
            target: mark points;
            set domain_start to event.domain.x.start;
            set domain_end to event.domain.x.end;
            set facet_value to event.facet[1];
          }
          on cursor_moved as inspect_legend {
            surface: legend fill;
            set legend_hit to event.legend.value;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(artifact.compiled_plot().event_bindings().len(), 2);
}

#[tokio::test]
async fn native_surface_event_filters_between_and_ordered_param_cursor_actions_lower() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          param true as enabled;
          param 0.0 as drag_x { sharing: free; }
          param [0.0, 0.0] as drag_domain;
          store as hovered {
            field utf8 id;
            field float64 x;
            primary_key: [id];
          }
          selection as picked { empty: none; combine: union; }
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          mark symbol as points { x: encoded "x"; y: encoded "y"; }
          on cursor_moved as drag {
            target: mark points;
            filter: $enabled AND (selection_contains(picked, datum."x") OR true);
            throttle_ms: 16;
            consume: true;
            mode: preview;
            settle_exact: true;
            between: {
              start: mouse_down { filter: $enabled; }
              end: mouse_up { filter: $enabled; }
            }
            set drag_x at start to event.coord.x;
            insert hovered {
              row { id: 'point'; x: event.coord.x; }
            }
            toggle picked {
              clause {
                id: 'point';
                equality {
                  x { field: "x"; value: event.coord.x; }
                }
              }
            }
            upsert picked {
              clause {
                id: 'range';
                interval {
                  x {
                    field: "x";
                    from: event.start.coord.x;
                    to: event.coord.x;
                  }
                }
              }
            }
            replace picked from scene {
              geometry: polygon(event.path);
              policy: intersects;
              marks: [points];
              fields: [{ id: 'x'; datum: 'x'; field: "x"; }];
              unique_by: ['x'];
              sharing: free;
            }
            delete picked { ids: ['point']; }
            set drag_domain to span_ordered(event.coord.x, event.start.coord.x);
            set cursor to 'crosshair';
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let bindings = artifact.compiled_plot().event_bindings();
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.mark_ids(), &["points"]);
    assert_eq!(binding.throttle_ms, Some(16));
    assert!(binding.consume);
    assert!(binding.between.is_some());
    let steps = binding.action.ordered_steps();
    assert_eq!(steps.len(), 8);
    assert!(matches!(
        &steps[0],
        avenger_chart_core::ChartActionStep::SetParam(action)
            if action.param_name == "drag_x"
                && action.scope == avenger_chart_core::ChartEventAssignmentScope::Start
    ));
    assert!(matches!(
        &steps[1],
        avenger_chart_core::ChartActionStep::SetStore(action)
            if action.store_name == "hovered"
    ));
    assert!(matches!(
        &steps[2],
        avenger_chart_core::ChartActionStep::SetSelection(action)
            if action.selection_id == "picked"
                && matches!(
                    action.update,
                    avenger_chart_core::SelectionUpdate::ToggleClauses { ref clauses }
                        if clauses.len() == 1
                )
    ));
    assert!(matches!(
        &steps[3],
        avenger_chart_core::ChartActionStep::SetSelection(action)
            if matches!(
                action.update,
                avenger_chart_core::SelectionUpdate::UpsertClauses { .. }
            )
    ));
    assert!(matches!(
        &steps[4],
        avenger_chart_core::ChartActionStep::SetSelection(action)
            if matches!(
                action.update,
                avenger_chart_core::SelectionUpdate::ReplaceAllFromSceneQuery { .. }
            )
    ));
    assert!(matches!(
        &steps[5],
        avenger_chart_core::ChartActionStep::SetSelection(action)
            if matches!(
                action.update,
                avenger_chart_core::SelectionUpdate::DeleteClauses { .. }
            )
    ));
    assert!(matches!(
        &steps[6],
        avenger_chart_core::ChartActionStep::SetParam(action)
            if action.param_name == "drag_domain"
    ));
    assert!(matches!(
        &steps[7],
        avenger_chart_core::ChartActionStep::SetCursor(_)
    ));
}

#[tokio::test]
async fn every_state_action_verb_lowers_to_the_existing_runtime_update_algebra() {
    let clause = |id: &str| {
        format!(
            r#"clause {{
                id: '{id}';
                equality {{ id {{ field: "id"; value: '{id}'; }} }}
              }}"#
        )
    };
    let source = format!(
        r#"avenger 1;
        chart cartesian as chart {{
          data: {{ values: [{{ id: 'a'; value: 1.0; }}]; }}
          store as rows {{
            primary_key: [id];
            field utf8 id;
            field float64 value;
          }}
          selection as picked {{ empty: none; combine: union; }}
          mark symbol as points {{ x: encoded "value"; y: encoded "value"; }}
          on click {{
            clear rows;
            insert rows {{ row {{ id: 'insert'; value: 1.0; }} }}
            replace rows {{ row {{ id: 'replace'; value: 2.0; }} }}
            upsert rows {{ row {{ id: 'upsert'; value: 3.0; }} }}
            patch rows {{ key {{ id: 'upsert'; }} fields {{ value: 4.0; }} }}
            delete rows {{ key {{ id: 'replace'; }} }}
            toggle rows {{ row {{ id: 'toggle'; value: 5.0; }} }}

            clear picked;
            clear picked within free;
            replace picked {{ {} }}
            replace picked within free {{ {} }}
            upsert picked {{ {} }}
            toggle picked {{ {} }}
            delete picked {{ ids: ['replace']; }}
            delete picked within free {{ ids: ['scoped']; }}

            replace picked from scene {{
              geometry: rect(0.0, 0.0, 1.0, 1.0);
              policy: intersects;
              marks: [points];
              fields: [{{ id: 'id'; datum: 'id'; field: "id"; }}];
            }}
            replace picked from scene within free {{
              geometry: rect(0.0, 0.0, 1.0, 1.0);
              policy: intersects;
              marks: [points];
              fields: [{{ id: 'id'; datum: 'id'; field: "id"; }}];
            }}
            upsert picked from scene {{
              geometry: rect(0.0, 0.0, 1.0, 1.0);
              policy: intersects;
              marks: [points];
              fields: [{{ id: 'id'; datum: 'id'; field: "id"; }}];
            }}
            toggle picked from scene {{
              geometry: rect(0.0, 0.0, 1.0, 1.0);
              policy: intersects;
              marks: [points];
              fields: [{{ id: 'id'; datum: 'id'; field: "id"; }}];
            }}
          }}
        }}"#,
        clause("replace"),
        clause("scoped"),
        clause("upsert"),
        clause("toggle"),
    );
    let artifact = source_compiler(&source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let steps = artifact.compiled_plot().event_bindings()[0]
        .action
        .ordered_steps();
    let variants = steps
        .iter()
        .map(|step| match step {
            avenger_chart_core::ChartActionStep::SetStore(action) => match &action.update {
                avenger_chart_core::StoreUpdate::Clear => "store.clear",
                avenger_chart_core::StoreUpdate::InsertRows { .. } => "store.insert",
                avenger_chart_core::StoreUpdate::ReplaceRows { .. } => "store.replace",
                avenger_chart_core::StoreUpdate::UpsertRows { .. } => "store.upsert",
                avenger_chart_core::StoreUpdate::UpdateByKey { .. } => "store.patch",
                avenger_chart_core::StoreUpdate::DeleteByKey { .. } => "store.delete",
                avenger_chart_core::StoreUpdate::ToggleRows { .. } => "store.toggle",
            },
            avenger_chart_core::ChartActionStep::SetSelection(action) => match &action.update {
                avenger_chart_core::SelectionUpdate::Clear => "selection.clear",
                avenger_chart_core::SelectionUpdate::ClearInScope { .. } => {
                    "selection.clear_scoped"
                }
                avenger_chart_core::SelectionUpdate::ReplaceAllClauses { .. } => {
                    "selection.replace"
                }
                avenger_chart_core::SelectionUpdate::ReplaceClausesInScope { .. } => {
                    "selection.replace_scoped"
                }
                avenger_chart_core::SelectionUpdate::UpsertClauses { .. } => "selection.upsert",
                avenger_chart_core::SelectionUpdate::ToggleClauses { .. } => "selection.toggle",
                avenger_chart_core::SelectionUpdate::DeleteClauses { .. } => "selection.delete",
                avenger_chart_core::SelectionUpdate::DeleteClausesInScope { .. } => {
                    "selection.delete_scoped"
                }
                avenger_chart_core::SelectionUpdate::ReplaceAllFromSceneQuery { .. } => {
                    "selection.replace_scene"
                }
                avenger_chart_core::SelectionUpdate::ReplaceFromSceneQueryInScope { .. } => {
                    "selection.replace_scene_scoped"
                }
                avenger_chart_core::SelectionUpdate::UpsertFromSceneQuery { .. } => {
                    "selection.upsert_scene"
                }
                avenger_chart_core::SelectionUpdate::ToggleFromSceneQuery { .. } => {
                    "selection.toggle_scene"
                }
                avenger_chart_core::SelectionUpdate::ToggleEqualityValue { .. } => {
                    panic!("the DSL does not expose ToggleEqualityValue")
                }
            },
            _ => panic!("this fixture contains only store and selection actions"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        variants,
        [
            "store.clear",
            "store.insert",
            "store.replace",
            "store.upsert",
            "store.patch",
            "store.delete",
            "store.toggle",
            "selection.clear",
            "selection.clear_scoped",
            "selection.replace",
            "selection.replace_scoped",
            "selection.upsert",
            "selection.toggle",
            "selection.delete",
            "selection.delete_scoped",
            "selection.replace_scene",
            "selection.replace_scene_scoped",
            "selection.upsert_scene",
            "selection.toggle_scene",
        ]
    );
}

#[tokio::test]
async fn scalar_parameter_initializers_infer_nested_arrow_types() {
    let source = r#"avenger 1;
        chart zerod as chart {
          param named_struct(
              'position', named_struct('x', CAST(1 AS DOUBLE), 'y', CAST(NULL AS DOUBLE)),
              'labels', ['a', 'b']
            ) as pointer;
          param CASE WHEN false THEN named_struct('x', CAST(0 AS DOUBLE)) ELSE NULL END as empty_pointer;
          param [CAST(1 + 1.9 AS SMALLINT), CAST('3' AS SMALLINT)] as fixed_values;
          param map(
            ['first', 'second'],
            [CAST(1 + 1.9 AS INT), CAST('4' AS INT)]
          ) as mapped_values;
          param CAST([] AS INT[]) as empty_values;
          param map(CAST([] AS VARCHAR[]), CAST([] AS INT[])) as empty_map;
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let defaults = artifact.compiled_plot().get_default_params();
    let pointer = defaults.get("pointer").expect("pointer default");
    assert!(matches!(pointer, ScalarValue::Struct(_)) && !pointer.is_null());
    assert!(matches!(
        pointer.data_type(),
        DataType::Struct(ref fields)
            if matches!(fields[0].data_type(), DataType::Struct(_))
                && matches!(fields[1].data_type(), DataType::List(_))
    ));
    assert!(
        defaults
            .get("empty_pointer")
            .expect("typed null struct default")
            .is_null()
    );
    assert!(matches!(
        defaults["fixed_values"].data_type(),
        DataType::List(_)
    ));
    assert!(matches!(
        defaults["mapped_values"].data_type(),
        DataType::Map(_, _)
    ));
    assert!(matches!(
        defaults["empty_values"].data_type(),
        DataType::List(_)
    ));
    assert!(matches!(
        defaults["empty_map"].data_type(),
        DataType::Map(_, _)
    ));
}

#[tokio::test]
async fn typed_boundaries_plan_sql_then_strictly_cast_to_declared_arrow_types() {
    let source = r#"avenger 1;
        chart zerod as chart {
          param CAST(3.9 AS INT) as narrowed;
          param CAST('0.75' AS DOUBLE) as parsed;
          param CAST(6 * 2.05 AS DECIMAL(10, 2)) as subtotal;
          param (CAST($source AS DOUBLE) + 1.5) as derived;
          param CAST('2' AS SMALLINT) as source;
          param (-0.0) as negative_zero;
          param named_struct('x', CAST(1 + 2.9 AS INT), 'label', upper('ok')) as nested;
          store as rows {
            field utf8 id;
            field int32 amount;
            primary_key: [id];
            row { id: upper('a'); amount: $source + 1.9; }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    let defaults = artifact.compiled_plot().get_default_params();
    assert_eq!(defaults["narrowed"], ScalarValue::Int32(Some(3)));
    assert_eq!(defaults["parsed"], ScalarValue::Float64(Some(0.75)));
    assert_eq!(
        defaults["subtotal"],
        ScalarValue::Decimal128(Some(1230), 10, 2)
    );
    assert_eq!(defaults["source"], ScalarValue::Int16(Some(2)));
    assert_eq!(defaults["derived"], ScalarValue::Float64(Some(3.5)));
    let ScalarValue::Float64(Some(negative_zero)) = defaults["negative_zero"] else {
        panic!("negative_zero should be a non-null float64")
    };
    assert!(negative_zero.is_sign_negative());
    let ScalarValue::Struct(nested) = &defaults["nested"] else {
        panic!("nested should be a struct")
    };
    assert_eq!(
        ScalarValue::try_from_array(nested.column(0), 0).unwrap(),
        ScalarValue::Int32(Some(3))
    );
    assert_eq!(
        ScalarValue::try_from_array(nested.column(1), 0).unwrap(),
        ScalarValue::Utf8(Some("OK".to_owned()))
    );
    let store = artifact.compiled_plot().store_specs().get("rows").unwrap();
    let row = store.initial.as_ref().expect("initial store row");
    assert_eq!(
        ScalarValue::try_from_array(row.column(0), 0).unwrap(),
        ScalarValue::Utf8(Some("A".to_owned()))
    );
    assert_eq!(
        ScalarValue::try_from_array(row.column(1), 0).unwrap(),
        ScalarValue::Int32(Some(3))
    );

    let invalid = r#"avenger 1;
        chart zerod as chart {
          param CAST('not an integer' AS INT) as bad;
        }"#;
    let diagnostics = source_compiler(invalid, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap_err();
    assert!(
        diagnostics
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cannot be cast")
                || diagnostic.primary.message.contains("cannot be cast")),
        "{:#?}",
        diagnostics.diagnostics
    );

    let duplicate_map_keys = r#"avenger 1;
        chart zerod as chart {
          param map([true, true], [CAST(1 AS INT), CAST(2 AS INT)]) as bad;
        }"#;
    let diagnostics = source_compiler(duplicate_map_keys, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap_err();
    assert!(
        diagnostics.diagnostics.iter().any(|diagnostic| {
            diagnostic
                .message
                .to_ascii_lowercase()
                .contains("duplicate")
                || diagnostic
                    .primary
                    .message
                    .to_ascii_lowercase()
                    .contains("duplicate")
        }),
        "keys that collide after their destination cast must be rejected: {:#?}",
        diagnostics.diagnostics
    );
}

#[tokio::test]
async fn native_surface_interactive_brush_fixture_runs_native_box_selection() {
    let root = fixture("03_interactive_brush");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    assert!(
        artifact
            .compiled_plot()
            .store_specs()
            .contains_key("__tool_brush__store")
    );
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("chart.brush.store")
    );
    let compiled = match Arc::try_unwrap(artifact.compiled) {
        Ok(compiled) => compiled,
        Err(_) => panic!("fixture owns its compiled plot"),
    };
    let mut app = chart_avenger_app(
        compiled,
        Arc::new(datafusion::prelude::SessionContext::new()),
        ChartAppOptions::default(),
    )
    .await
    .unwrap();

    let mut initial_points = Vec::new();
    collect_symbol_positions_and_fills(
        &app.scene_graph().marks,
        app.scene_graph().origin,
        &mut initial_points,
    );
    assert_eq!(initial_points.len(), 2);
    let unselected = [203.0 / 255.0, 213.0 / 255.0, 225.0 / 255.0, 1.0];
    assert!(initial_points.iter().all(|(_, fill)| *fill == unselected));
    let [point_x, point_y] = initial_points[0].0;

    app.update_state(
        &WindowEvent::CursorMoved(WindowCursorMoved {
            position: [point_x - 10.0, point_y - 10.0],
        }),
        Instant::now(),
    )
    .await;
    app.update_state(
        &WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
        }),
        Instant::now(),
    )
    .await;
    let first_status = app
        .update_with_status(
            &WindowEvent::CursorMoved(WindowCursorMoved {
                position: [point_x, point_y],
            }),
            Instant::now(),
        )
        .await
        .unwrap()
        .status;
    let second_status = app
        .update_with_status(
            &WindowEvent::CursorMoved(WindowCursorMoved {
                position: [point_x + 10.0, point_y + 10.0],
            }),
            Instant::now(),
        )
        .await
        .unwrap()
        .status;

    assert!(first_status.rerender);
    assert!(second_status.rerender);
    let mut selected_points = Vec::new();
    collect_symbol_positions_and_fills(
        &app.scene_graph().marks,
        app.scene_graph().origin,
        &mut selected_points,
    );
    let selected = [37.0 / 255.0, 99.0 / 255.0, 235.0 / 255.0, 1.0];
    assert!(
        selected_points.iter().any(|(_, fill)| *fill == selected),
        "the brush selection should drive the conditional fill channel: {selected_points:?}"
    );
    let state = app.app_state_mut().clone();
    let metrics = state.event_metrics().await;
    assert_eq!(metrics.store_patch_events, 2);
    assert_eq!(metrics.evaluation_errors, 0);
}

#[tokio::test]
async fn native_surface_text_input_editing_exports_are_reference_driven() {
    let unused = r#"avenger 1;
        chart zerod as chart {
          widget text_input as query {
            position: top;
          }
        }"#;
    let unused = source_compiler(unused, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert!(unused.interface.params.contains_key("query__value"));
    assert!(!unused.interface.params.contains_key("query__cursor"));
    assert!(!unused.interface.params.contains_key("query__selected_text"));

    let referenced = r#"avenger 1;
        chart zerod as chart {
          widget text_input as query {
            position: top;
          }
          mark text as cursor_label {
            text: direct $query.cursor_position;
          }
        }"#;
    let referenced = source_compiler(referenced, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert!(referenced.interface.params.contains_key("query__value"));
    assert!(referenced.interface.params.contains_key("query__cursor"));
    assert!(
        !referenced
            .interface
            .params
            .contains_key("query__selected_text")
    );
    assert!(
        referenced
            .interface
            .widget_exports
            .contains_key("chart.query.cursor_position")
    );
}

#[tokio::test]
async fn vertical_slice_lowering_diagnostics_keep_the_source_label() {
    let cases = [
        (
            "unsupported coordinate/mark pair",
            r#"avenger 1;
            import * as acme from 'native:com.acme.compiler-fixture@1';
            chart acme.isometric as chart { mark symbol as point {} }"#,
            Some(composed_registry()),
            "symbol",
        ),
        (
            "missing data column",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point { x: encoded "missing"; y: encoded "y"; }
            }"#,
            None,
            "missing",
        ),
        (
            "invalid scale config",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point { x: encoded "x" { scale: linear { zero: 'yes'; } } y: encoded "y"; }
            }"#,
            None,
            "zero",
        ),
        (
            "removed mark-wide domain exclusion",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point {
                    x: encoded "x";
                    y: encoded "y";
                    exclude_from_scale_domains: true;
                }
            }"#,
            None,
            "exclude_from_scale_domains",
        ),
        (
            "invalid channel domain contribution",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point {
                    x: encoded "x" { domain_contribution: maybe; }
                    y: encoded "y";
                }
            }"#,
            None,
            "domain_contribution",
        ),
        (
            "registered lowerer failure",
            r#"avenger 1;
            import * as acme from 'native:com.acme.compiler-fixture@1';
            chart cartesian as chart { mark acme.failing_mark as point {} }"#,
            Some(composed_registry()),
            "intentional downstream lowerer failure",
        ),
    ];

    for (case, source, registry, expected) in cases {
        let failure = source_compiler(source, registry)
            .compile_chart("chart.avenger", None)
            .await
            .expect_err(case);
        let diagnostic = failure.diagnostics.first().expect("one diagnostic");
        assert_eq!(diagnostic.primary.span.source.get(), 0, "{case}");
        assert!(!diagnostic.primary.span.range.is_empty(), "{case}");
        assert!(
            diagnostic.primary.message.contains(expected)
                || diagnostic.message.contains(expected)
                || diagnostic.notes.iter().any(|note| note.contains(expected)),
            "{case}: {diagnostic:#?}"
        );
    }
}

#[tokio::test]
async fn vertical_slice_composed_registry_compiles_extensions_and_nested_coordinates() {
    let stock = Arc::new(builtins::bootstrap_registry().unwrap());
    let composed = composed_registry();
    assert_ne!(stock.profile_id(), composed.profile_id());

    let root = fixture("04_composed_extension");
    let compiler = Compiler::builder()
        .project_root(&root)
        .native_registry(composed.clone())
        .build()
        .unwrap();
    let mut charts = BTreeMap::new();
    for chart in [
        "external_primitive",
        "external_compound",
        "external_coordinate",
        "mixed_coordinates",
    ] {
        charts.insert(
            chart,
            compiler
                .compile_chart(root.join(format!("{chart}.avenger")), None)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        charts["external_primitive"].compiled_plot().marks()[0].mark_type(),
        "hexbin"
    );
    assert!(
        !charts["external_compound"]
            .compiled_plot()
            .marks()
            .is_empty()
    );
    assert_eq!(
        charts["external_coordinate"].compiled_plot().marks()[0].mark_type(),
        "cube"
    );
    assert_eq!(
        charts["mixed_coordinates"].compiled_plot().marks().len(),
        2,
        "both built-in and custom child plots cross the erased child boundary"
    );

    let stock_compiler = Compiler::builder()
        .project_root(&root)
        .native_registry(stock.clone())
        .build()
        .unwrap();
    let mut stock_diagnostics = Vec::new();
    for chart in [
        "external_primitive",
        "external_compound",
        "external_coordinate",
        "mixed_coordinates",
    ] {
        stock_diagnostics.extend(
            stock_compiler
                .compile_chart(root.join(format!("{chart}.avenger")), None)
                .await
                .expect_err("stock registry must reject downstream kinds")
                .diagnostics,
        );
    }
    assert!(
        stock_diagnostics.iter().all(|diagnostic| {
            diagnostic.code.as_str() == "AVENGER-MODULE-016"
                && !diagnostic.primary.span.range.is_empty()
        }),
        "{stock_diagnostics:#?}"
    );

    let schema = CompilerBuilder::default()
        .project_root(&root)
        .native_registry(composed.clone())
        .build()
        .unwrap()
        .language_host()
        .semantic_json_schema();
    let schema_text = serde_json::to_string(schema.as_value()).unwrap();
    for expected in [
        "external_isometric",
        "external_cube",
        "iso_x",
        "angle",
        "external_hexbin",
        "external_mean_point",
    ] {
        assert!(
            schema_text.contains(expected),
            "semantic schema omits {expected}"
        );
    }
    let docs = composed.snapshot().markdown_reference();
    for expected in [
        "external_isometric",
        "external_cube",
        "iso_x",
        "angle",
        "external_hexbin",
        "external_mean_point",
    ] {
        assert!(
            docs.contains(expected),
            "documentation inventory omits {expected}"
        );
    }

    let artifact = &charts["external_primitive"];
    let bytes = artifact.to_bytes().unwrap();
    let decoded = CompiledChartArtifact::from_bytes(&bytes, &composed).unwrap();
    assert_eq!(decoded.compiled_plot().marks()[0].mark_type(), "hexbin");
    let stock_error = CompiledChartArtifact::from_bytes(&bytes, &stock).unwrap_err();
    assert!(
        matches!(
            stock_error,
            ArtifactSerializationError::MissingNativeModule { .. }
        ),
        "{stock_error:?}"
    );
}

#[tokio::test]
async fn expansion_custom_mark_compiles_through_canonical_group_source() {
    let root = fixture("04_custom_error_bar");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let expanded = compiler
        .expand_module(root.join("chart.avenger"))
        .await
        .unwrap();
    assert_expansion_baseline("04_custom_error_bar.avenger", &expanded.text);

    for removed in [
        "import { error_bar } from 'error_bar.avenger';",
        "mark error_bar as errors",
        "slot expr",
        "define mark",
    ] {
        assert!(!expanded.text.contains(removed), "{}", expanded.text);
    }
    for retained in [
        "mark group as errors",
        "component_kind: error_bar;",
        " as stem;",
        " as point;",
        "private mark group as __av_",
        "fill: direct '#dc2626';",
        "public mark text as labels",
        "widget slider as threshold",
    ] {
        assert!(expanded.text.contains(retained), "{}", expanded.text);
    }
    let definition_mappings = expanded
        .source_map
        .mappings
        .iter()
        .filter(|mapping| mapping.definition.is_some())
        .collect::<Vec<_>>();
    assert!(!definition_mappings.is_empty());
    assert!(definition_mappings.iter().all(|mapping| {
        mapping.instantiation.is_some()
            && mapping.expanded.source != mapping.authored.source
            && mapping.definition.unwrap().source != mapping.instantiation.unwrap().source
    }));

    let artifact = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let expanded_artifact = source_compiler(&expanded.text, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(artifact.compiled_plot()).unwrap(),
        serde_json::to_value(expanded_artifact.compiled_plot()).unwrap(),
        "compiling imported definitions and canonical expanded source must agree"
    );
    assert_eq!(artifact.interface, expanded_artifact.interface);
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("chart.errors.stem")
    );
    assert!(
        artifact
            .interface
            .public_targets
            .contains_key("chart.errors.point")
    );
    let component_parts = artifact
        .compiled_plot()
        .marks()
        .iter()
        .filter_map(|mark| mark.state().identity.component.as_ref())
        .map(|component| {
            (
                component.component_kind.as_str(),
                component.part_alias.as_str(),
            )
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(component_parts.contains(&("error_bar", "stem")));
    assert!(component_parts.contains(&("error_bar", "point")));
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    let expanded_evaluated = expanded_artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&evaluated.scene_graph).unwrap(),
        serde_json::to_value(&expanded_evaluated.scene_graph).unwrap()
    );
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn expansion_custom_tool_lowers_canonical_behavior_state_events_scale_and_chrome() {
    let root = fixture("05_custom_tool");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let expanded = compiler
        .expand_module(root.join("chart.avenger"))
        .await
        .unwrap();
    assert_expansion_baseline("05_custom_tool.avenger", &expanded.text);
    for retained in [
        "tool behavior as inspector",
        "component_kind: inspect_points;",
        " as enabled;",
        " as hovered;",
        " as chrome;",
        "private param true as __av_",
        "private selection as __av_",
        "private tool point_selection as __av_",
        "scale_edit {",
        "private mark group as __av_",
    ] {
        assert!(expanded.text.contains(retained), "{}", expanded.text);
    }
    let set_param = expanded.text.find("_enabled to").unwrap();
    let set_selection = expanded.text.find("clear __av_").unwrap();
    assert!(set_param < set_selection, "{}", expanded.text);

    let artifact = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let expanded_artifact = source_compiler(&expanded.text, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(artifact.compiled_plot()).unwrap(),
        serde_json::to_value(expanded_artifact.compiled_plot()).unwrap(),
        "custom tool compilation must equal canonical behavior compilation"
    );
    assert_eq!(artifact.interface, expanded_artifact.interface);
    let behavior = artifact
        .compiled_plot()
        .tool_behaviors()
        .iter()
        .find(|behavior| behavior.component_kind == "inspect_points")
        .expect("compiled custom behavior");
    assert_eq!(behavior.source_id, "inspector");
    let aliases = behavior
        .exports
        .iter()
        .map(|export| export.alias.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert!(aliases.contains("enabled"), "{aliases:?}");
    assert!(aliases.contains("hovered"), "{aliases:?}");
    assert!(
        artifact
            .compiled_plot()
            .param_specs()
            .keys()
            .any(|name| name.ends_with("_enabled"))
    );
    assert!(
        artifact
            .compiled_plot()
            .selection_specs()
            .keys()
            .any(|name| name.ends_with("_hovered"))
    );
    assert!(
        artifact
            .compiled_plot()
            .event_bindings()
            .iter()
            .any(|binding| {
                binding.event_type == avenger_chart_core::ChartEventType::Click
                    && binding.action.steps.len() == 2
            })
    );
    for target in [
        "chart.inspector.enabled",
        "chart.inspector.hovered",
        "chart.inspector.chrome",
    ] {
        assert!(
            artifact.interface.public_targets.contains_key(target),
            "missing {target}: {:?}",
            artifact.interface.public_targets.keys().collect::<Vec<_>>()
        );
    }
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    let expanded_evaluated = expanded_artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&evaluated.scene_graph).unwrap(),
        serde_json::to_value(&expanded_evaluated.scene_graph).unwrap()
    );
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn expansion_custom_transform_projects_exact_outputs_and_hides_intermediates() {
    let root = fixture("05_custom_transform_pipeline");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let expanded = compiler
        .expand_module(root.join("chart.avenger"))
        .await
        .unwrap();
    assert_expansion_baseline("05_custom_transform_pipeline.avenger", &expanded.text);

    for retained in [
        "transform pipeline as summary",
        "output adjusted;",
        "output doubled;",
        "transform sql as __av_",
        "AS doubled",
        "AS adjusted",
    ] {
        assert!(expanded.text.contains(retained), "{}", expanded.text);
    }
    for removed in [
        "import 'summarize.avenger'",
        "transform summarize as summary",
        "define transform",
        "__summarize_doubled",
        "__summarize_tripled",
        "__summarize_scratch",
    ] {
        assert!(!expanded.text.contains(removed), "{}", expanded.text);
    }

    let artifact = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let expanded_artifact = source_compiler(&expanded.text, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(artifact.compiled_plot()).unwrap(),
        serde_json::to_value(expanded_artifact.compiled_plot()).unwrap(),
    );
    assert_eq!(artifact.interface, expanded_artifact.interface);

    let analysis = compiler
        .analyze_module(root.join("chart.avenger"))
        .await
        .unwrap();
    let public_schema = analysis
        .datasets
        .iter()
        .map(|(_, dataset)| dataset.schema.as_ref())
        .find(|schema| {
            schema.field_with_name("doubled").is_ok()
                && schema.field_with_name("adjusted").is_ok()
                && schema.field_with_name("category").is_ok()
                && schema
                    .fields()
                    .iter()
                    .all(|field| !field.name().starts_with("__"))
        })
        .expect("defined pipeline public schema");
    let public_names = public_schema
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect::<Vec<_>>();
    assert_eq!(public_names, ["category", "value", "doubled", "adjusted"]);
    assert!(
        public_schema
            .fields()
            .iter()
            .all(|field| !field.name().starts_with("__"))
    );

    let evaluated = artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    let expanded_evaluated = expanded_artifact
        .compiled_plot()
        .evaluate(&datafusion::prelude::SessionContext::new(), None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&evaluated.scene_graph).unwrap(),
        serde_json::to_value(&expanded_evaluated.scene_graph).unwrap()
    );
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn outputs_slot_compiles_caller_authored_projection_columns() {
    let compiler = source_compiler(
        r#"
avenger 1;

define transform summarize {
  slot outputs measures;
  transform aggregate { expressions: measures; }
}

chart cartesian as chart {
  data: {
    values: [
      { category: 'A'; amount: 2.0; },
      { category: 'B'; amount: 4.0; }
    ];
  }
  transform summarize as stats {
    measures: sum("amount") AS total, avg("amount") AS average;
  }
  mark symbol { x: encoded stats.total; y: encoded stats.average; }
}
"#,
        None,
    );
    let artifact = compiler
        .compile_chart("chart.avenger", Some("chart"))
        .await
        .unwrap();
    let analysis = compiler.analyze_module("chart.avenger").await.unwrap();
    assert!(analysis.datasets.iter().any(|(_, dataset)| {
        dataset.schema.field_with_name("total").is_ok()
            && dataset.schema.field_with_name("average").is_ok()
    }));
    let evaluated = artifact
        .compiled_plot()
        .evaluate(&SessionContext::new(), None)
        .await
        .unwrap();
    assert!(!evaluated.scene_graph.marks.is_empty());
}

#[tokio::test]
async fn outputs_slot_rejects_aliases_dropped_before_the_definition_boundary() {
    let compiler = source_compiler(
        r#"
avenger 1;

define transform broken {
  slot outputs columns;
  transform calculate { expressions: columns; }
  transform select { expressions: "category"; }
}

chart cartesian as chart {
  data: { values: [{ category: 'A'; amount: 2.0; }]; }
  transform broken as projected {
    columns: "amount" + 1 AS adjusted;
  }
  mark symbol { x: encoded "category"; y: encoded projected.adjusted; }
}
"#,
        None,
    );
    let failure = compiler
        .compile_chart("chart.avenger", Some("chart"))
        .await
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("lowering")
                || diagnostic.primary.message.contains("adjusted")
        }),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn native_aggregate_rejects_counting_a_nullable_value_as_row_count() {
    let compiler = source_compiler(
        r#"
avenger 1;

chart cartesian as chart {
  data: {
    values: [
      { amount: 2.0; },
      { amount: null; }
    ];
  }
  transform aggregate {
    expressions: count("amount") AS count;
  }
}
"#,
        None,
    );
    let failure = compiler
        .compile_chart("chart.avenger", Some("chart"))
        .await
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("row count")
                || diagnostic.primary.message.contains("row count")
        }),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn native_select_rejects_duplicate_unaliased_result_columns() {
    let compiler = source_compiler(
        r#"
avenger 1;

chart cartesian as chart {
  data: { values: [{ category: 'A'; amount: 2.0; }]; }
  transform select {
    expressions: "category", "category";
  }
}
"#,
        None,
    );
    let failure = compiler
        .compile_chart("chart.avenger", Some("chart"))
        .await
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.message.contains("duplicate column")
                || diagnostic.primary.message.contains("duplicate column")
        }),
        "{:#?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn defined_transform_rejects_reserved_private_columns_in_its_input_schema() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("summarize.avenger"),
        include_str!("fixtures/projects/05_custom_transform_pipeline/summarize.avenger"),
    )
    .unwrap();
    fs::write(
        root.path().join("chart.avenger"),
        r#"
avenger 1;
import { summarize } from 'summarize.avenger';
chart cartesian as chart {
  data: {
    values: [{ category: 'A'; value: 2.0; __private_existing: 1.0; }];
  }
  transform summarize as summary { measure: "value"; }
  mark symbol { x: encoded "category"; y: encoded summary.adjusted; }
}
"#,
    )
    .unwrap();

    let compiler = Compiler::builder()
        .project_root(root.path())
        .build()
        .unwrap();
    let failure = compiler
        .compile_chart(root.path().join("chart.avenger"), None)
        .await
        .unwrap_err();
    assert!(
        failure.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "AVENGER-LOWER-003"
                && diagnostic.primary.message.contains("__private_existing")
        }),
        "{:?}",
        failure.diagnostics
    );
}

#[tokio::test]
async fn expansion_preserves_composed_and_native_widgets_adjacent_to_all_definition_kinds() {
    let root = fixture("05_definition_widget_adjacency");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let expanded = compiler
        .expand_module(root.join("chart.avenger"))
        .await
        .unwrap();
    for retained in [
        "transform pipeline",
        "mark group as points",
        "tool behavior as pointer",
        "widget slider as threshold",
        "widget text_input as search",
    ] {
        assert!(expanded.text.contains(retained), "{}", expanded.text);
    }
    for removed in ["transform pass", "mark dot", "tool cursor_tool", "define "] {
        assert!(!expanded.text.contains(removed), "{}", expanded.text);
    }

    let artifact = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap();
    let expanded_artifact = source_compiler(&expanded.text, None)
        .compile_chart("chart.avenger", None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(artifact.compiled_plot()).unwrap(),
        serde_json::to_value(expanded_artifact.compiled_plot()).unwrap(),
    );
    assert_eq!(artifact.interface, expanded_artifact.interface);
    let widget_kinds = artifact
        .compiled_plot()
        .widgets()
        .iter()
        .map(|attachment| attachment.widget.kind())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        widget_kinds,
        std::collections::BTreeSet::from(["slider", "text-input"])
    );
    assert!(
        artifact
            .compiled_plot()
            .widgets()
            .iter()
            .any(|attachment| matches!(attachment.widget, CompiledWidget::Composed(_)))
    );
    assert!(
        artifact
            .compiled_plot()
            .widgets()
            .iter()
            .any(|attachment| matches!(attachment.widget, CompiledWidget::Native(_)))
    );
}

#[tokio::test]
async fn expansion_lowering_diagnostics_remap_to_definition_with_instance_trace() {
    let root = fixture("05_definition_lowering_error");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let expanded = compiler
        .expand_module(root.join("chart.avenger"))
        .await
        .unwrap();
    let definition_source = expanded
        .sources
        .iter()
        .find_map(|(id, source)| {
            source
                .origin
                .display_name()
                .ends_with("broken.avenger")
                .then_some(*id)
        })
        .expect("definition source id");
    let chart_source = expanded
        .sources
        .iter()
        .find_map(|(id, source)| {
            source
                .origin
                .display_name()
                .ends_with("chart.avenger")
                .then_some(*id)
        })
        .expect("chart source id");

    let failure = compiler
        .compile_chart(root.join("chart.avenger"), None)
        .await
        .unwrap_err();
    let diagnostic = failure.diagnostics.first().expect("lowering diagnostic");
    assert_eq!(diagnostic.code.as_str(), "AVENGER-LOWER-002");
    assert_eq!(diagnostic.primary.span.source, definition_source);
    assert!(diagnostic.trace.iter().any(|frame| {
        frame.span.source == chart_source
            && frame.message == "while expanding this definition instance"
    }));
}
