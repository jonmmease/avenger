use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeSchemaSnapshot, PropertySchema, SchemaVersion, ValueShape,
};
use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService, CodeActionKind, CodeActionRequest,
    DocumentRequest, DocumentSnapshot, LineEnding, PositionRequest, RenameError, SemanticTokenKind,
    SourceRevision, WorkspaceAnalysis, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ByteSpan, ContentVersion, InMemorySourceLoader, LoadedSource, ProjectRoot, SourceOrigin,
    SourceSpan,
};
use sha2::{Digest, Sha256};

fn workspace_analysis(text: &str) -> (WorkspaceAnalysis, SourceOrigin, SourceRevision) {
    let origin = SourceOrigin::Memory("editing/chart.avenger".to_owned());
    let revision = SourceRevision::from_text(text);
    let syntax = BTreeMap::from([(
        origin.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            origin.clone(),
            revision.clone(),
            text,
        )),
    )]);
    (
        WorkspaceAnalysis::syntax_only(
            AnalysisGeneration::new(1),
            PathBuf::from("editing"),
            vec![origin.clone()],
            syntax,
            test_registry(),
        ),
        origin,
        revision,
    )
}

fn test_registry() -> NativeSchemaSnapshot {
    let key = NativeKindKey::mark("cartesian", "symbol");
    let mut symbol = KindSchema::new(key.clone(), "symbol");
    symbol.properties.insert(
        "size".to_owned(),
        PropertySchema::optional(ValueShape::Number, "symbol size"),
    );
    symbol.properties.insert(
        "color".to_owned(),
        PropertySchema::optional(ValueShape::String, "symbol color"),
    );
    NativeSchemaSnapshot {
        version: SchemaVersion::V1,
        profile_label: "editing-test".to_owned(),
        entries: BTreeMap::from([(key, symbol)]),
    }
}

#[test]
fn canonical_formatting_is_a_comment_preserving_fixpoint_and_rejects_invalid_source() {
    let source = r#"avenger 1; chart cartesian as chart {
 z: 2; -- trailing
 -- before mark
 mark symbol as points { x: value 1; }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let result = analysis
        .format_document(
            &DocumentRequest {
                source: origin.clone(),
                source_revision: revision,
            },
            LineEnding::Lf,
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(result.edit.new_text.contains("-- trailing"));
    assert!(result.edit.new_text.contains("-- before mark"));
    let (formatted, origin, revision) = workspace_analysis(&result.edit.new_text);
    assert!(
        formatted
            .format_document(
                &DocumentRequest {
                    source: origin,
                    source_revision: revision,
                },
                LineEnding::Lf,
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .is_none()
    );

    let invalid = "avenger 1; chart cartesian as chart {";
    let (analysis, origin, revision) = workspace_analysis(invalid);
    assert!(
        analysis
            .format_document(
                &DocumentRequest {
                    source: origin,
                    source_revision: revision,
                },
                LineEnding::Lf,
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn semantic_tokens_and_safe_rename_share_authored_symbol_identity() {
    let source = r#"avenger 1;

chart cartesian as chart {
  param float64 as width { value: 10.0; }
  param float64 as height { value: 20.0; }
  mark symbol as points { size: $width; }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let tokens = analysis
        .semantic_tokens(
            &DocumentRequest {
                source: origin.clone(),
                source_revision: revision.clone(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(
        tokens
            .tokens
            .iter()
            .any(|token| token.kind == SemanticTokenKind::Parameter && token.modifiers.declaration)
    );
    assert!(
        tokens
            .tokens
            .iter()
            .any(|token| token.kind == SemanticTokenKind::Property)
    );
    assert!(matches!(
        analysis.semantic_tokens(
            &DocumentRequest {
                source: origin.clone(),
                source_revision: SourceRevision::new("stale"),
            },
            &AnalysisCancellation::default(),
        ),
        Err(avenger_lang_analysis::AnalysisQueryError::StaleRevision)
    ));

    let cursor = source.rfind("$width").unwrap() + 2;
    let request = PositionRequest {
        source: origin.clone(),
        byte_offset: cursor,
        source_revision: revision,
    };
    let prepared = analysis
        .prepare_rename(&request, &AnalysisCancellation::default())
        .unwrap()
        .unwrap();
    assert_eq!(prepared.placeholder, "width");
    assert_eq!(&source[prepared.span.range.as_range()], "width");

    let edit = analysis
        .rename(&request, "canvas_width", &AnalysisCancellation::default())
        .unwrap();
    let edits = &edit.sources[&origin].edits;
    assert_eq!(edits.len(), 2);
    let mut renamed = source.to_owned();
    for edit in edits.iter().rev() {
        renamed.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    assert!(renamed.contains("param float64 as canvas_width"));
    assert!(renamed.contains("$canvas_width"));

    assert!(matches!(
        analysis.rename(&request, "height", &AnalysisCancellation::default()),
        Err(RenameError::Collision(name)) if name == "height"
    ));
    assert!(matches!(
        analysis.rename(&request, "$bad", &AnalysisCancellation::default()),
        Err(RenameError::InvalidName(name)) if name == "$bad"
    ));
}

#[test]
fn selection_param_rename_updates_target_resolved_set_references() {
    let source = r#"avenger 1;
chart cartesian {
  param selection as picked {}
  on click { set picked = clear; }
}"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let cursor = source.find("picked").unwrap() + 1;
    let renamed = analysis
        .rename(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: cursor,
                source_revision: revision,
            },
            "selected",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let edits = &renamed.sources[&origin].edits;
    assert_eq!(edits.len(), 2);
    let mut text = source.to_owned();
    for edit in edits.iter().rev() {
        text.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    assert!(text.contains("param selection as selected"));
    assert!(text.contains("set selected = clear"));
}

#[test]
fn references_prefer_the_nearest_lexical_binding_when_names_repeat() {
    let source = r#"avenger 1;

chart cartesian as first {
  -- | Width for the first chart.
  param float64 as width { value: 10.0; }
  mark symbol as points { size: $width; }
}

chart cartesian as second {
  -- | Width for the second chart.
  param float64 as width { value: 20.0; }
  mark symbol as points { size: $width; }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let first_reference = source.find("$width").unwrap() + 2;
    let request = PositionRequest {
        source: origin.clone(),
        byte_offset: first_reference,
        source_revision: revision,
    };

    let hover = analysis
        .hover(&request, &AnalysisCancellation::default())
        .unwrap()
        .unwrap();
    assert!(hover.markdown.contains("Width for the first chart."));
    assert!(!hover.markdown.contains("Width for the second chart."));

    let definition = analysis
        .definition(&request, &AnalysisCancellation::default())
        .unwrap();
    assert_eq!(definition.targets.len(), 1);
    assert_eq!(
        &source[definition.targets[0].selection_span.range.as_range()],
        "width"
    );
    assert_eq!(
        definition.targets[0].selection_span.range.start,
        source.find("param float64 as width").unwrap() + "param float64 as ".len()
    );

    let edit = analysis
        .rename(&request, "first_width", &AnalysisCancellation::default())
        .unwrap();
    assert_eq!(edit.sources[&origin].edits.len(), 2);
}

#[test]
fn nested_state_shadowing_resolves_each_reference_to_its_own_scope() {
    let source = r#"avenger 1;

chart cartesian as chart {
  -- | Outer width.
  param float64 as width { value: 10.0; }

  container group as inner {
    -- | Inner width.
    param float64 as width { value: 20.0; }
    mark symbol as inner_points { size: $width; }
  }

  mark symbol as outer_points { size: $width; }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let reference_request = |offset| PositionRequest {
        source: origin.clone(),
        byte_offset: offset,
        source_revision: revision.clone(),
    };
    let inner_offset = source.find("$width").unwrap() + 2;
    let outer_offset = source.rfind("$width").unwrap() + 2;

    let inner_hover = analysis
        .hover(
            &reference_request(inner_offset),
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(inner_hover.markdown.contains("Inner width."));
    assert!(!inner_hover.markdown.contains("Outer width."));

    let outer_hover = analysis
        .hover(
            &reference_request(outer_offset),
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(outer_hover.markdown.contains("Outer width."));
    assert!(!outer_hover.markdown.contains("Inner width."));

    let inner_definition = analysis
        .definition(
            &reference_request(inner_offset),
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let outer_definition = analysis
        .definition(
            &reference_request(outer_offset),
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(inner_definition.targets.len(), 1);
    assert_eq!(outer_definition.targets.len(), 1);
    assert_ne!(
        inner_definition.targets[0].selection_span,
        outer_definition.targets[0].selection_span
    );

    let inner_rename = analysis
        .rename(
            &reference_request(inner_offset),
            "inner_width",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let outer_rename = analysis
        .rename(
            &reference_request(outer_offset),
            "outer_width",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(inner_rename.sources[&origin].edits.len(), 2);
    assert_eq!(outer_rename.sources[&origin].edits.len(), 2);
}

#[test]
fn local_quick_fixes_are_mechanical_and_typed() {
    let source = r#"avenger 1;

chart cartesian as chart {
  mark symbol as points {
    siez: 10.0;
    size: $threshold;
  }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let actions_at = |needle: &str| {
        let start = source.find(needle).unwrap();
        analysis
            .code_actions(
                &CodeActionRequest {
                    source: origin.clone(),
                    range: SourceSpan {
                        source: analysis.syntax[&origin].parsed.tokens.source(),
                        range: ByteSpan {
                            start,
                            end: start + needle.len(),
                        },
                    },
                    source_revision: revision.clone(),
                    diagnostic_codes: vec!["fixture".to_owned()],
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
    };
    let property = actions_at("siez");
    assert!(property.iter().any(|action| {
        action.title == "Replace `siez` with `size`"
            && action.preferred
            && action.diagnostic_codes == ["fixture"]
    }));

    let param = actions_at("$threshold");
    let action = param
        .iter()
        .find(|action| action.title.contains("Declare parameter `$threshold`"))
        .unwrap();
    let inserted = &action.edit.sources[&origin].edits[0].new_text;
    assert!(inserted.contains("param float64 as threshold"));
    assert!(inserted.contains("value: NULL"));

    let missing_as = "avenger 1; chart cartesian as chart { param float64 width { value: 1.0; } }";
    let (analysis, origin, revision) = workspace_analysis(missing_as);
    let start = missing_as.find("width").unwrap();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "width".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: Vec::new(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Add missing `as` binder")
    );
}

#[test]
fn pin_import_target_is_offered_only_for_unpinned_remote_imports() {
    let source = "avenger 1; import 'https://example.test/badge.mark.avenger' as defs; chart cartesian as chart {}";
    let (analysis, origin, revision) = workspace_analysis(source);
    let start = source.find("https://").unwrap();
    let target = analysis
        .pin_import_target(
            &CodeActionRequest {
                source: origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "https://example.test/badge.mark.avenger".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: Vec::new(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .expect("pin target");
    assert_eq!(target.url, "https://example.test/badge.mark.avenger");
    assert_eq!(
        &source[target.insertion_span.range.start - 1..target.insertion_span.range.start],
        "'"
    );

    let pinned = source.replace(" as defs", " sha256 'abc' as defs");
    let (analysis, origin, revision) = workspace_analysis(&pinned);
    assert!(
        analysis
            .pin_import_target(
                &CodeActionRequest {
                    source: origin.clone(),
                    range: SourceSpan {
                        source: analysis.syntax[&origin].parsed.tokens.source(),
                        range: ByteSpan {
                            start,
                            end: start + 5,
                        },
                    },
                    source_revision: revision,
                    diagnostic_codes: Vec::new(),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn explicit_pin_fetch_hashes_only_valid_remote_definitions() {
    let directory = tempfile::tempdir().unwrap();
    let url = "https://example.test/badge.mark.avenger";
    let origin = SourceOrigin::Http(url.to_owned());
    let definition = "avenger 1; define mark badge { mark symbol {} }";
    let loader = InMemorySourceLoader::default().with_source(LoadedSource::new(
        origin.clone(),
        definition,
        ContentVersion::new("fixture"),
    ));
    let compiler = Compiler::builder()
        .project_root(directory.path())
        .source_loader(Arc::new(loader.clone()))
        .build()
        .unwrap();
    let service = AnalysisService::new(compiler);
    let hash = service
        .pin_http_import(url, &AnalysisCancellation::default())
        .await
        .unwrap();
    assert_eq!(hash, format!("{:x}", Sha256::digest(definition.as_bytes())));

    loader.insert(LoadedSource::new(
        origin,
        "avenger 1; chart cartesian as chart {}",
        ContentVersion::new("not-definition"),
    ));
    assert!(matches!(
        service
            .pin_http_import(url, &AnalysisCancellation::default())
            .await,
        Err(avenger_lang_analysis::PinImportError::NotDefinition)
    ));
}

#[test]
fn rename_covers_import_aliases_and_cross_file_definition_references() {
    let definition_origin = SourceOrigin::Memory("editing/badge.mark.avenger".to_owned());
    let chart_origin = SourceOrigin::Memory("editing/chart.avenger".to_owned());
    let definition = "avenger 1; define mark badge { mark symbol {} }";
    let chart = "avenger 1; import 'badge.mark.avenger' as defs; chart cartesian as chart { mark defs.badge as imported {} }";
    let definition_revision = SourceRevision::from_text(definition);
    let chart_revision = SourceRevision::from_text(chart);
    let syntax = BTreeMap::from([
        (
            definition_origin.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                definition_origin.clone(),
                definition_revision,
                definition,
            )),
        ),
        (
            chart_origin.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                chart_origin.clone(),
                chart_revision.clone(),
                chart,
            )),
        ),
    ]);
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(2),
        PathBuf::from("editing"),
        vec![definition_origin.clone(), chart_origin.clone()],
        syntax,
        test_registry(),
    );

    let alias_use = chart.find("as defs").unwrap() + "as ".len() + 1;
    let alias_edit = analysis
        .rename(
            &PositionRequest {
                source: chart_origin.clone(),
                byte_offset: alias_use,
                source_revision: chart_revision.clone(),
            },
            "library",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(alias_edit.sources.len(), 1);
    assert_eq!(alias_edit.sources[&chart_origin].edits.len(), 2);

    let definition_use = chart.rfind("badge").unwrap() + 1;
    let definition_edit = analysis
        .rename(
            &PositionRequest {
                source: chart_origin.clone(),
                byte_offset: definition_use,
                source_revision: chart_revision,
            },
            "status_badge",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(definition_edit.sources.len(), 2);
    assert_eq!(definition_edit.sources[&definition_origin].edits.len(), 1);
    assert_eq!(definition_edit.sources[&chart_origin].edits.len(), 1);
}

#[tokio::test]
async fn inline_definition_uses_the_compilers_canonical_expansion() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let chart_path = root.join("chart.avenger");
    let definition_path = root.join("badge.mark.avenger");
    let chart = "avenger 1; import 'badge.mark.avenger'; chart cartesian as chart { mark badge as imported {} }";
    let definition =
        "avenger 1; define mark badge { mark symbol as body { x: value 1; y: value 2; } }";
    std::fs::write(&chart_path, chart).unwrap();
    std::fs::write(&definition_path, definition).unwrap();
    let chart_origin = SourceOrigin::File(chart_path.clone());
    let definition_origin = SourceOrigin::File(definition_path);
    let revision = SourceRevision::from_text(chart);
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = AnalysisService::new(compiler.clone())
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root: root,
                roots: vec![ProjectRoot::chart(chart_origin.clone())],
                open_documents: BTreeMap::from([
                    (
                        chart_origin.clone(),
                        DocumentSnapshot::new(chart_origin.clone(), revision.clone(), chart),
                    ),
                    (
                        definition_origin.clone(),
                        DocumentSnapshot::new(
                            definition_origin,
                            SourceRevision::from_text(definition),
                            definition,
                        ),
                    ),
                ]),
                known_disk_sources: Vec::new(),
                native_registry_profile: compiler
                    .language_host()
                    .registry()
                    .profile_id()
                    .as_str()
                    .to_owned(),
            },
            &AnalysisCancellation::default(),
        )
        .await
        .unwrap();
    let start = chart.find("mark badge").unwrap();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: chart_origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&chart_origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "mark badge".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: Vec::new(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let action = actions
        .iter()
        .find(|action| action.kind == CodeActionKind::RefactorInline)
        .expect("inline definition action");
    let edit = &action.edit.sources[&chart_origin].edits[0];
    assert!(edit.new_text.starts_with("container group as imported"));
    assert!(edit.new_text.contains("component_kind: badge"));
    assert!(edit.new_text.contains("private mark symbol"));

    let mut inlined = chart.to_owned();
    inlined.replace_range(edit.span.range.as_range(), &edit.new_text);
    std::fs::write(&chart_path, inlined).unwrap();
    compiler.check_project(&chart_path).await.unwrap();
}

#[tokio::test]
async fn extract_definition_creates_a_compiling_file_and_infers_scalar_slots() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let chart_path = root.join("chart.avenger");
    let chart = r#"avenger 1;

chart cartesian as chart {
  param float64 as point_size { value: 32.0; }
  container group as cluster {
    -- keep this authored explanation
    mark symbol as point { x: value 1; y: value 2; size: $point_size; }
  }
}
"#;
    std::fs::write(&chart_path, chart).unwrap();
    let chart_origin = SourceOrigin::File(chart_path.clone());
    let revision = SourceRevision::from_text(chart);
    let compiler = Compiler::builder().project_root(&root).build().unwrap();
    let analysis = AnalysisService::new(compiler.clone())
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root: root,
                roots: vec![ProjectRoot::chart(chart_origin.clone())],
                open_documents: BTreeMap::from([(
                    chart_origin.clone(),
                    DocumentSnapshot::new(chart_origin.clone(), revision.clone(), chart),
                )]),
                known_disk_sources: Vec::new(),
                native_registry_profile: compiler
                    .language_host()
                    .registry()
                    .profile_id()
                    .as_str()
                    .to_owned(),
            },
            &AnalysisCancellation::default(),
        )
        .await
        .unwrap();
    let start = chart.find("container group as cluster").unwrap();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: chart_origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&chart_origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "container group as cluster".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: Vec::new(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let action = actions
        .iter()
        .find(|action| action.kind == CodeActionKind::RefactorExtract)
        .expect("extract definition action");
    let definition_origin = SourceOrigin::File(chart_path.with_file_name("cluster.mark.avenger"));
    let definition = &action.edit.create_files[&definition_origin];
    assert!(definition.contains("define mark cluster"));
    assert!(definition.contains("slot expr point_size"));
    assert!(definition.contains("size: point_size"));
    assert!(definition.contains("-- keep this authored explanation"));

    let mut extracted = chart.to_owned();
    let mut edits = action.edit.sources[&chart_origin].edits.clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.range.start));
    for edit in edits {
        extracted.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    std::fs::write(&chart_path, extracted).unwrap();
    std::fs::write(
        chart_path.with_file_name("cluster.mark.avenger"),
        definition,
    )
    .unwrap();
    compiler.check_project(&chart_path).await.unwrap();
}
