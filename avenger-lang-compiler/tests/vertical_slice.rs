use std::{path::PathBuf, sync::Arc};

use avenger_chart::prelude::{
    Cartesian, CompiledWidget, FacetColumn, FacetColumnSubplotChannels, IntoPlotMark, Subplot,
};
use avenger_chart_external_test::{
    external_compound_mark::ExternalMeanPoint,
    external_coord_system::{Cube, Isometric},
    external_mark::HexBin,
};
use avenger_chart_lang_registry::{
    CoordinatePack, NativeRegistry, NativeRegistryBuilder, RegistryError, ResolvedValue, builtins,
};
use avenger_chart_schema::{
    BodyMode, ChannelSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    ValueShape,
};
use avenger_lang_compiler::{
    ArtifactSerializationError, CompiledChartArtifact, Compiler, CompilerBuilder,
};
use avenger_lang_core::{
    ContentVersion, InMemorySourceLoader, LoadedSource, SourceLoader, SourceOrigin,
};
use datafusion::{arrow::datatypes::DataType, logical_expr::lit, scalar::ScalarValue};
use indexmap::IndexMap;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
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

fn optional_channel(name: &str, docs: &str) -> ChannelSchema {
    ChannelSchema {
        name: name.to_string(),
        required: false,
        shape: ValueShape::SqlExpression,
        docs: docs.to_string(),
    }
}

fn composed_registry() -> Arc<NativeRegistry> {
    let mut builder = NativeRegistryBuilder::new(1, "phase5-downstream-fixture");
    builtins::register_bootstrap_builtins(&mut builder).unwrap();
    builder
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
    builder
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
    builder
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
    builder.register_coordinate_pack(isometric).unwrap();

    let container = CoordinatePack::new(
        "external_facet_column",
        KindSchema::new(
            NativeKindKey::new(NativeKindNamespace::Coordinate, "external_facet_column"),
            "A downstream container for mixed-coordinate child plots.",
        )
        .body_mode(BodyMode::Mixed),
        |_| Ok(FacetColumn),
    )
    .child_plots(|plot, child, placement| {
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
    builder.register_coordinate_pack(container).unwrap();
    Arc::new(builder.build().unwrap())
}

#[tokio::test]
async fn vertical_slice_sql_aggregate_pipeline_propagates_schema_and_evaluates() {
    let root = fixture("02_sql_pipeline");
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let artifact = compiler
        .compile_file(root.join("chart.avenger"))
        .await
        .unwrap();
    let analysis = compiler.analyze_project(&root).await.unwrap();
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
            output total: totals.total;
            transform aggregate as totals {
              total: sum("amount");
            }
          }
          mark symbol as point {
            x: summarized.total;
            y: summarized.total;
            size: value 80.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
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
            x: "x" { scale: linear { domain: [0.0, 4.0]; } axis: { title: 'X'; } }
            y: "y" { scale: linear; axis: { title: 'Y'; } }
            fill: "category" {
              scale: ordinal { domain: ['A', 'B']; range: ['#5778a4', '#e49444']; }
              legend: { title: 'Category'; position: right; }
            }
            size: value 80.0;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
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
async fn vertical_slice_widget_is_opaque_and_exported_state_drives_the_mark() {
    let root = fixture("03_widget_vertical_slice");
    let artifact = Compiler::builder()
        .project_root(&root)
        .build()
        .unwrap()
        .compile_file(root.join("chart.avenger"))
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
async fn vertical_slice_lowering_diagnostics_keep_the_source_label() {
    let cases = [
        (
            "unsupported coordinate/mark pair",
            r#"avenger 1; chart external_isometric as chart { mark symbol as point {} }"#,
            Some(composed_registry()),
            "symbol",
        ),
        (
            "missing data column",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point { x: "missing"; y: "y"; }
            }"#,
            None,
            "missing",
        ),
        (
            "invalid scale config",
            r#"avenger 1; chart cartesian as chart {
                data: { values: [{ x: 1.0; y: 2.0; }]; }
                mark symbol as point { x: "x" { scale: linear { zero: 'yes'; } } y: "y"; }
            }"#,
            None,
            "zero",
        ),
        (
            "registered lowerer failure",
            r#"avenger 1; chart cartesian as chart { mark failing_external_mark as point {} }"#,
            Some(composed_registry()),
            "intentional downstream lowerer failure",
        ),
    ];

    for (case, source, registry, expected) in cases {
        let failure = source_compiler(source, registry)
            .compile_file("chart.avenger")
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
    let project = Compiler::builder()
        .project_root(&root)
        .native_registry(composed.clone())
        .build()
        .unwrap()
        .compile_project(&root)
        .await
        .unwrap();
    assert_eq!(project.charts.len(), 4);
    assert_eq!(
        project
            .chart("external_primitive")
            .unwrap()
            .compiled_plot()
            .marks()[0]
            .mark_type(),
        "hexbin"
    );
    assert!(
        !project
            .chart("external_compound")
            .unwrap()
            .compiled_plot()
            .marks()
            .is_empty()
    );
    assert_eq!(
        project
            .chart("external_coordinate")
            .unwrap()
            .compiled_plot()
            .marks()[0]
            .mark_type(),
        "cube"
    );
    assert_eq!(
        project
            .chart("mixed_coordinates")
            .unwrap()
            .compiled_plot()
            .marks()
            .len(),
        2,
        "both built-in and custom child plots cross the erased child boundary"
    );

    let stock_failure = Compiler::builder()
        .project_root(&root)
        .native_registry(stock.clone())
        .build()
        .unwrap()
        .compile_project(&root)
        .await
        .expect_err("stock registry must reject downstream kinds");
    assert!(stock_failure.diagnostics.iter().all(|diagnostic| {
        diagnostic.code.as_str() == "AVENGER-RESOLVE-020"
            && !diagnostic.primary.span.range.is_empty()
    }));

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

    let artifact = project.chart("external_primitive").unwrap();
    let bytes = artifact.to_bytes().unwrap();
    let decoded = CompiledChartArtifact::from_bytes(&bytes, &composed).unwrap();
    assert_eq!(decoded.compiled_plot().marks()[0].mark_type(), "hexbin");
    assert!(matches!(
        CompiledChartArtifact::from_bytes(&bytes, &stock),
        Err(ArtifactSerializationError::RegistryProfileMismatch { .. })
    ));
}
