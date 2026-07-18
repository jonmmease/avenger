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
async fn native_surface_common_mark_state_is_schema_directed_and_preserved() {
    let source = r#"avenger 1;
        chart cartesian as chart {
          data: { values: [{ x: 1.0; y: 2.0; category: 'A'; }]; }
          mark symbol as point {
            x: "x";
            y: "y";
            visible: true;
            details: [x, category];
            zindex: 7;
            facet_data_scope: level(2);
            geometry_space: display;
            exclude_from_scale_domains: true;
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"details\":[\"x\",\"category\"]"));
    assert!(json.contains("\"zindex\":7"));
    assert!(json.contains("\"geometry_space\":\"display\""));
    assert!(json.contains("\"exclude_from_scale_domains\":true"));
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
            x: "category";
            y: "amount";
            orientation: vertical;
            extent: 1.5;
            fill: "category";
          }
          mark violin as distribution {
            x: "category";
            y: "amount";
            orientation: vertical;
            bandwidth: 0.0;
            steps: 40;
            density_extent: [0.0, 7.0];
            counts: false;
            density_extent_resolve: shared;
            density_data_scope: level(1);
            width: 0.8;
            width_normalization: per_violin;
            fill: "category";
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

    let artifact = compiler.compile_file("chart.avenger").await.unwrap();
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
            x: "x";
            y: "y";
            details: [id];
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
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
async fn native_surface_concat_cells_lower_as_mixed_coordinate_subplots() {
    let source = r#"avenger 1;
        chart hconcat as chart {
          data: { values: [{ x: 1.0; y: 2.0; }]; }
          spacing: 12;
          widths: [fr(2), px(180)];

          cell cartesian as left {
            label: 'Left cell';
            mark symbol { x: "x"; y: "y"; }
          }
          cell polar as right {
            mark symbol { theta: "x"; r: "y"; size: value 100; }
          }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
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
                mark symbol {{ x: "x"; y: "y"; }}
              }}
            }}"#
        );
        let artifact = source_compiler(&source, None)
            .compile_file("chart.avenger")
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
                mark rect {{ x: "category"; y: "value"; }}
              }}
            }}"#
        );
        let artifact = source_compiler(&source, None)
            .compile_file("chart.avenger")
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
          variable row as mpg { expr: "mpg"; title: 'MPG'; }
          variable row as hp { expr: "hp"; title: 'Horsepower'; }
          variable column as weight { expr: "weight"; title: 'Weight'; }
          variable column as accel { expr: "accel"; title: 'Acceleration'; }
          domain_coordination: matrix;

          cell cartesian {
            when: repeat.row_id <> repeat.column_id;
            mark symbol { x: repeat.column; y: repeat.row; }
          }
          cell zerod {
            when: repeat.row_id = repeat.column_id;
            mark text { text: repeat.row_title; }
          }
        }"#;
    let artifact = source_compiler(grid, None)
        .compile_file("chart.avenger")
        .await
        .unwrap();
    let json = serde_json::to_string(artifact.compiled_plot()).unwrap();
    assert!(json.contains("\"origin\":\"repeat_grid\""), "{json}");

    let wrap = r#"avenger 1;
        chart repeat_wrap as chart {
          data: { values: [{ mpg: 21.0; hp: 110.0; }]; }
          variable item as mpg { expr: "mpg"; }
          variable item as hp { expr: "hp"; }
          responsive_columns: 180;
          cell cartesian {
            mark symbol { x: repeat.item; y: repeat.item; }
          }
        }"#;
    let artifact = source_compiler(wrap, None)
        .compile_file("chart.avenger")
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
            r#"x: avg("x"); y: avg("y"); key: "category"; width: 120; height: 90;"#,
            "polar",
            r#"r: "r"; theta: "theta";"#,
        ),
        (
            "polar",
            r#"r: avg("r"); theta: avg("theta"); key: "category"; width: 100; height: 80;"#,
            "cartesian",
            r#"x: "x"; y: "y";"#,
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
            .compile_file("chart.avenger")
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
          mark symbol { x: "x"; y: "y"; }
        }"#;
    let artifact = source_compiler(source, None)
        .compile_file("chart.avenger")
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
        .compile_file("chart.avenger")
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
async fn native_surface_text_input_editing_exports_are_reference_driven() {
    let unused = r#"avenger 1;
        chart zerod as chart {
          widget text_input as query {
            position: top;
          }
        }"#;
    let unused = source_compiler(unused, None)
        .compile_file("chart.avenger")
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
            text: $query.cursor_position;
          }
        }"#;
    let referenced = source_compiler(referenced, None)
        .compile_file("chart.avenger")
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
