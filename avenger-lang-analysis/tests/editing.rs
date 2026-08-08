use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    SchemaVersion, ValueShape,
};
use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, AnalysisService, CodeActionKind, CodeActionRequest,
    DocumentRequest, DocumentSnapshot, LineEnding, PositionRequest, RenameError, SemanticTokenKind,
    SourceRevision, WorkspaceAnalysis, WorkspaceSnapshot, analyze_syntax,
};
use avenger_lang_compiler::Compiler;
use avenger_lang_core::{
    ByteSpan, ContentVersion, InMemorySourceLoader, LoadedSource, ModuleRoot, SourceOrigin,
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
    let symbol_key = NativeKindKey::mark("cartesian", "symbol");
    let mut symbol = KindSchema::new(symbol_key.clone(), "symbol");
    symbol.properties.insert(
        "size".to_owned(),
        PropertySchema::optional(ValueShape::Number, "symbol size"),
    );
    symbol.properties.insert(
        "color".to_owned(),
        PropertySchema::optional(ValueShape::String, "symbol color"),
    );
    let stack_key = NativeKindKey::new(NativeKindNamespace::Transform, "stack");
    let stack = KindSchema::new(stack_key.clone(), "stack").property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "stack field"),
    );
    let slider_key = NativeKindKey::new(NativeKindNamespace::Widget, "slider");
    let slider = KindSchema::new(slider_key.clone(), "slider")
        .property(
            "min",
            PropertySchema::required(ValueShape::Number, "minimum"),
        )
        .property(
            "max",
            PropertySchema::required(ValueShape::Number, "maximum"),
        );
    NativeSchemaSnapshot {
        version: SchemaVersion::V1,
        profile_label: "editing-test".to_owned(),
        entries: BTreeMap::from([
            (symbol_key, symbol),
            (stack_key, stack),
            (slider_key, slider),
        ]),
        modules: BTreeMap::new(),
    }
}

async fn resolved_workspace_analysis(
    text: &str,
) -> (WorkspaceAnalysis, SourceOrigin, SourceRevision) {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let origin = SourceOrigin::File(project_root.join("chart.avenger"));
    let revision = SourceRevision::from_text(text);
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .unwrap();
    let analysis = AnalysisService::new(compiler.clone())
        .analyze_workspace(
            WorkspaceSnapshot {
                generation: AnalysisGeneration::new(1),
                project_root,
                roots: vec![ModuleRoot::requested(origin.clone())],
                open_documents: BTreeMap::from([(
                    origin.clone(),
                    DocumentSnapshot::new(origin.clone(), revision.clone(), text),
                )]),
                known_disk_sources: vec![origin.clone()],
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
    (analysis, origin, revision)
}

async fn resolved_workspace_analysis_with_last_good(
    valid_text: &str,
    current_text: &str,
) -> (WorkspaceAnalysis, SourceOrigin, SourceRevision) {
    let directory = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(directory.path()).unwrap();
    let origin = SourceOrigin::File(project_root.join("chart.avenger"));
    let compiler = Compiler::builder()
        .project_root(&project_root)
        .build()
        .unwrap();
    let profile = compiler
        .language_host()
        .registry()
        .profile_id()
        .as_str()
        .to_owned();
    let snapshot = |generation, text: &str| WorkspaceSnapshot {
        generation: AnalysisGeneration::new(generation),
        project_root: project_root.clone(),
        roots: vec![ModuleRoot::requested(origin.clone())],
        open_documents: BTreeMap::from([(
            origin.clone(),
            DocumentSnapshot::new(origin.clone(), SourceRevision::from_text(text), text),
        )]),
        known_disk_sources: vec![origin.clone()],
        native_registry_profile: profile.clone(),
    };
    let service = AnalysisService::new(compiler);
    let valid = service
        .analyze_workspace(snapshot(1, valid_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(valid.semantic_roots[&origin.canonical_uri()].result.is_ok());
    let current = service
        .analyze_workspace(snapshot(2, current_text), &AnalysisCancellation::default())
        .await
        .unwrap();
    assert!(
        current.semantic_roots[&origin.canonical_uri()]
            .result
            .is_err()
    );
    let revision = SourceRevision::from_text(current_text);
    (current.with_last_good_semantics(&valid), origin, revision)
}

#[tokio::test]
async fn typed_boundary_hover_reports_sql_source_and_arrow_destination() {
    let source = r#"avenger 1;
chart zerod as chart {
  param CAST(3.9 AS INT) as narrowed;
  store as rows {
    field int16 amount;
    field struct(field(int16, 'x'), field(utf8, 'label')) nested;
    row {
      amount: '4';
      nested: { x: 1 + 2.5; label: upper('ok'); }
    }
  }
  on cursor_moved as update {
    set narrowed to '9';
    set cursor to 42;
  }
}
"#;
    let (analysis, origin, revision) = resolved_workspace_analysis(source).await;
    let hover_at = |needle: &str| {
        analysis
            .hover(
                &PositionRequest {
                    source: origin.clone(),
                    byte_offset: source.find(needle).unwrap() + 1,
                    source_revision: revision.clone(),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .unwrap()
    };

    let narrowed = hover_at("'9'");
    assert!(narrowed.markdown.contains("Typed SQL boundary"));
    assert!(narrowed.markdown.contains("Utf8"));
    assert!(narrowed.markdown.contains("Int32"));
    assert!(narrowed.markdown.contains("strict DataFusion/Arrow `CAST`"));

    let action_target = hover_at("narrowed to");
    assert!(
        action_target
            .markdown
            .contains("`set` action on a scalar parameter")
    );
    assert!(action_target.markdown.contains("physical Arrow boundary"));
    assert!(action_target.markdown.contains("```avenger\nnarrowed\n```"));
    assert!(
        !action_target
            .markdown
            .contains("```avenger\n$narrowed\n```")
    );

    let initializer = analysis
        .hover(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: source.find("3.9").unwrap() + 1,
                source_revision: revision.clone(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(
        initializer
            .as_ref()
            .is_none_or(|hover| !hover.markdown.contains("Typed SQL boundary")),
        "a param initializer establishes its own type: {initializer:?}"
    );

    let nested = hover_at("1 + 2.5");
    assert!(
        nested.markdown.contains("field `nested.x`"),
        "{}",
        nested.markdown
    );
    assert!(nested.markdown.contains("Int16"), "{}", nested.markdown);

    let store = hover_at("'4'");
    assert!(
        store.markdown.contains("store `$rows` field `amount`"),
        "{}",
        store.markdown
    );
    assert!(store.markdown.contains("Int16"), "{}", store.markdown);

    let action = hover_at("'9'");
    assert!(
        action.markdown.contains("assignment to param `$narrowed`"),
        "{}",
        action.markdown
    );
    assert!(action.markdown.contains("Int32"), "{}", action.markdown);

    let cursor = hover_at("42;");
    assert!(
        cursor.markdown.contains("cursor assignment"),
        "{}",
        cursor.markdown
    );
    assert!(cursor.markdown.contains("Utf8"), "{}", cursor.markdown);
}

#[test]
fn canonical_formatting_is_a_comment_preserving_fixpoint_and_rejects_invalid_source() {
    let source = r#"avenger 1; chart cartesian as chart {
 z: 2; -- trailing
 -- before mark
 mark symbol as points { x: direct 1; }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let result = analysis
        .format_document(
            &DocumentRequest {
                source: origin.clone(),
                source_revision: revision.clone(),
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
  param 10.0 as width;
  param 20.0 as height;
  mark symbol as points { size: encoded $width; }
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
    let encoded = source.find("encoded").unwrap();
    assert!(
        !tokens
            .tokens
            .iter()
            .any(|token| token.span.range.start == encoded)
    );
    let width_reference = source.rfind("$width").unwrap() + 1;
    assert!(tokens.tokens.iter().any(|token| {
        token.kind == SemanticTokenKind::Parameter
            && token.span.range.start == width_reference
            && &source[token.span.range.as_range()] == "width"
    }));
    let mode_hover = analysis
        .hover(
            &PositionRequest {
                source: origin.clone(),
                byte_offset: encoded + 1,
                source_revision: revision.clone(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .unwrap();
    assert!(mode_hover.markdown.contains("scale"));
    assert!(mode_hover.markdown.contains("domain inference"));
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
    assert!(renamed.contains("param 10.0 as canvas_width"));
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

#[tokio::test]
async fn projection_alias_tokens_hover_and_safe_rename_share_output_identity() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ amount: 2.0; }, { amount: 4.0; }]; }
  transform aggregate as totals {
    expressions: sum("amount") AS total;
  }
  mark symbol { x: encoded totals.total; y: encoded totals.total; }
}"#;
    let (analysis, origin, revision) = resolved_workspace_analysis(source).await;
    let alias_start = source.find("AS total").unwrap() + "AS ".len();
    let request = PositionRequest {
        source: origin.clone(),
        byte_offset: alias_start + 1,
        source_revision: revision.clone(),
    };
    let semantic = analysis
        .semantic_tokens(
            &DocumentRequest {
                source: origin.clone(),
                source_revision: revision.clone(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(
        semantic.tokens.iter().any(|token| {
            token.span.range.start == alias_start
                && token.kind == SemanticTokenKind::Field
                && token.modifiers.declaration
        }),
        "{:#?}",
        semantic.tokens
    );
    let hover = analysis
        .hover(&request, &AnalysisCancellation::default())
        .unwrap()
        .unwrap();
    assert!(hover.markdown.contains("transform output column `total`"));
    assert!(hover.markdown.contains("Arrow type: `Float64`"));

    let prepared = analysis
        .prepare_rename(&request, &AnalysisCancellation::default())
        .unwrap();
    assert!(
        prepared.is_some(),
        "{:#?}",
        analysis.semantic_index.documents[&origin]
    );
    let prepared = prepared.unwrap();
    assert_eq!(prepared.placeholder, "total");
    let edit = analysis
        .rename(&request, "amount_total", &AnalysisCancellation::default())
        .unwrap();
    assert_eq!(edit.sources[&origin].edits.len(), 3);
    let mut renamed = source.to_owned();
    for edit in edit.sources[&origin].edits.iter().rev() {
        renamed.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    assert!(renamed.contains("AS amount_total"));
    assert_eq!(renamed.matches("totals.amount_total").count(), 2);

    let unsafe_source = source.replace(
        "mark symbol",
        "transform calculate { expressions: \"total\" + 1 AS adjusted; }\n  mark symbol",
    );
    let (analysis, origin, revision) = resolved_workspace_analysis(&unsafe_source).await;
    let alias_start = unsafe_source.find("AS total").unwrap() + "AS ".len();
    assert!(
        analysis
            .prepare_rename(
                &PositionRequest {
                    source: origin,
                    byte_offset: alias_start + 1,
                    source_revision: revision,
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn selection_rename_updates_target_resolved_action_references() {
    let source = r#"avenger 1;
chart cartesian {
  selection as picked {}
  on click { clear picked; }
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
    assert!(text.contains("selection as selected"));
    assert!(text.contains("clear selected"));
}

#[test]
fn chart_runnables_carry_exact_selectors_and_reject_ambiguous_anonymous_charts() {
    let source = r#"avenger 1;

chart cartesian as first {}
chart polar as second {}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let result = analysis
        .chart_runnables(
            &DocumentRequest {
                source: origin,
                source_revision: revision,
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(
        result
            .runnables
            .iter()
            .map(|runnable| runnable.selector.as_deref())
            .collect::<Vec<_>>(),
        [Some("first"), Some("second")]
    );

    let anonymous = "avenger 1; chart cartesian {}";
    let (analysis, origin, revision) = workspace_analysis(anonymous);
    let result = analysis
        .chart_runnables(
            &DocumentRequest {
                source: origin,
                source_revision: revision,
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(result.runnables.len(), 1);
    assert_eq!(result.runnables[0].selector, None);

    let ambiguous = "avenger 1; chart cartesian {} chart polar as named {}";
    let (analysis, origin, revision) = workspace_analysis(ambiguous);
    let result = analysis
        .chart_runnables(
            &DocumentRequest {
                source: origin,
                source_revision: revision,
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(result.runnables.len(), 1);
    assert_eq!(result.runnables[0].selector.as_deref(), Some("named"));
}

#[test]
fn references_prefer_the_nearest_lexical_binding_when_names_repeat() {
    let source = r#"avenger 1;

chart cartesian as first {
  -- | Width for the first chart.
  param 10.0 as width;
  mark symbol as points { size: encoded $width; }
}

chart cartesian as second {
  -- | Width for the second chart.
  param 20.0 as width;
  mark symbol as points { size: encoded $width; }
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
        source.find("param 10.0 as width").unwrap() + "param 10.0 as ".len()
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
  param 10.0 as width;

  mark group as inner {
    -- | Inner width.
    param 20.0 as width;
    mark symbol as inner_points { size: encoded $width; }
  }

  mark symbol as outer_points { size: encoded $width; }
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
    size: encoded $threshold;
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
    assert!(inserted.contains("param CAST(NULL AS DOUBLE) as threshold;"));

    let missing_as = "avenger 1; chart cartesian as chart { param 1.0 width; }";
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
                source_revision: revision.clone(),
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
fn missing_required_member_fixes_are_schema_driven_and_parseable() {
    let source = r#"avenger 1;
chart cartesian as chart {
  transform stack as stacked {}
  widget slider as control {
  }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let actions_at = |needle: &str, diagnostic_codes: Vec<String>| {
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
                    diagnostic_codes,
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
    };

    assert!(
        actions_at("transform stack", Vec::new())
            .iter()
            .all(|action| !action.title.contains("required")),
        "the schema fix must be tied to the missing-required diagnostic"
    );
    let stack = actions_at("transform stack", vec!["AVENGER-RESOLVE-022".to_owned()]);
    let stack = stack
        .iter()
        .find(|action| action.title == "Add required `field:` property")
        .expect("stack field action");
    assert!(stack.preferred);
    assert_eq!(stack.diagnostic_codes, ["AVENGER-RESOLVE-022"]);
    assert_eq!(
        stack.edit.sources[&origin].edits[0].new_text,
        "\n    field: NULL;\n  "
    );

    let slider = actions_at("widget slider", vec!["AVENGER-RESOLVE-022".to_owned()]);
    let slider = slider
        .iter()
        .find(|action| action.title == "Add all missing required properties")
        .expect("slider properties action");
    let edit = &slider.edit.sources[&origin].edits[0];
    assert!(edit.new_text.contains("    max: 0.0;"));
    assert!(edit.new_text.contains("    min: 0.0;"));

    let fixed = format!(
        "{}{}{}",
        &source[..edit.span.range.start],
        edit.new_text,
        &source[edit.span.range.end..]
    );
    let fixed_revision = SourceRevision::from_text(&fixed);
    let fixed_syntax = analyze_syntax(&DocumentSnapshot::new(origin, fixed_revision, fixed));
    assert!(fixed_syntax.parsed.strict.is_some());
}

#[test]
fn direct_state_action_payload_fixes_are_verb_aware_and_parseable() {
    let source = r#"avenger 1;
chart cartesian as chart {
  selection as picked { combine: union; empty: none; }
  on click {
    replace picked from scene {
    }
  }
}
"#;
    let (analysis, origin, revision) = workspace_analysis(source);
    let start = source.find("replace picked").unwrap();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "replace picked".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: vec!["AVENGER-RESOLVE-140".to_owned()],
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let action = actions
        .iter()
        .find(|action| action.title == "Add all missing required action members")
        .expect("scene-query action payload fix");
    let edit = &action.edit.sources[&origin].edits[0];
    for member in ["geometry:", "policy:", "marks:", "fields:"] {
        assert!(edit.new_text.contains(member), "{}", edit.new_text);
    }
    assert_eq!(action.diagnostic_codes, ["AVENGER-RESOLVE-140"]);
    let fixed = format!(
        "{}{}{}",
        &source[..edit.span.range.start],
        edit.new_text,
        &source[edit.span.range.end..]
    );
    let fixed_revision = SourceRevision::from_text(&fixed);
    let fixed_syntax = analyze_syntax(&DocumentSnapshot::new(origin, fixed_revision, fixed));
    assert!(fixed_syntax.parsed.strict.is_some());
}

#[tokio::test]
async fn native_missing_required_diagnostic_offers_the_schema_fix() {
    let source = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  transform stack as stacked {
  }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
  }
}
"#;
    let (analysis, origin, revision) = resolved_workspace_analysis(source).await;
    let failure = analysis.semantic_roots[&origin.canonical_uri()]
        .result
        .as_ref()
        .expect_err("missing stack field should fail resolution");
    assert!(
        failure
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "AVENGER-RESOLVE-022")
    );
    let start = source.find("  }\n  mark symbol").unwrap() + 2;
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + 1,
                    },
                },
                source_revision: revision,
                diagnostic_codes: vec!["AVENGER-RESOLVE-022".to_owned()],
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let action = actions
        .iter()
        .find(|action| action.title == "Add required `field:` property")
        .expect("native stack quick fix");
    assert_eq!(
        action.edit.sources[&origin].edits[0].new_text,
        "\n    field: NULL;\n  "
    );
}

#[tokio::test]
async fn missing_required_fix_uses_the_current_kind_over_last_good_semantics() {
    let valid = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  transform sql as changed {
    query: SELECT * FROM input;
  }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
  }
}
"#;
    let current = r#"avenger 1;
chart cartesian as chart {
  data: { values: [{ x: 1.0; y: 2.0; }]; }
  transform stack as changed {
  }
  mark symbol as points {
    x: encoded "x";
    y: encoded "y";
  }
}
"#;
    let (analysis, origin, revision) =
        resolved_workspace_analysis_with_last_good(valid, current).await;
    let stack = analysis.semantic_index.documents[&origin]
        .symbols
        .iter()
        .find(|symbol| symbol.name == "changed")
        .expect("changed transform symbol");
    assert_eq!(stack.native_kind.as_deref(), Some("stack"));

    let start = current.find("  }\n  mark symbol").unwrap() + 2;
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: origin,
                range: SourceSpan {
                    source: stack.declaration_span.source,
                    range: ByteSpan {
                        start,
                        end: start + 1,
                    },
                },
                source_revision: revision,
                diagnostic_codes: vec!["AVENGER-RESOLVE-022".to_owned()],
            },
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert!(
        actions
            .iter()
            .any(|action| action.title == "Add required `field:` property")
    );
    assert!(
        actions
            .iter()
            .all(|action| action.title != "Add required `query:` property")
    );
}

#[test]
fn removed_state_syntax_has_no_compatibility_code_actions() {
    for (category, code) in [
        ("store", "AVENGER-PARSE-026"),
        ("selection", "AVENGER-PARSE-027"),
    ] {
        let source = format!("avenger 1; chart cartesian {{ param {category} as state {{}} }}");
        let (analysis, origin, revision) = workspace_analysis(&source);
        let start = source.find(category).unwrap();
        let actions = analysis
            .code_actions(
                &CodeActionRequest {
                    source: origin.clone(),
                    range: SourceSpan {
                        source: analysis.syntax[&origin].parsed.tokens.source(),
                        range: ByteSpan {
                            start,
                            end: start + category.len(),
                        },
                    },
                    source_revision: revision,
                    diagnostic_codes: vec![code.to_owned()],
                },
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert!(
            actions
                .iter()
                .all(|action| !action.title.contains(&format!("`{category} as`"))),
            "unexpected compatibility action: {actions:#?}"
        );
    }
}

#[test]
fn channel_mode_migration_actions_are_contextual_and_mechanical() {
    fn actions(source: &str, needle: &str) -> Vec<avenger_lang_analysis::CodeAction> {
        let (analysis, origin, revision) = workspace_analysis(source);
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
                    source_revision: revision,
                    diagnostic_codes: match needle {
                        "x: \"x\"" => vec!["AVENGER-RESOLVE-195".to_owned()],
                        "scale:" => vec!["AVENGER-RESOLVE-198".to_owned()],
                        _ => Vec::new(),
                    },
                },
                &AnalysisCancellation::default(),
            )
            .unwrap()
    }

    let modern_source = r#"avenger 1;
chart cartesian as chart {
  mark symbol as points {
    x: "x";
    stroke: direct '#000000' {
      scale: linear;
    }
  }
}
"#;
    let bare_actions = actions(modern_source, "x: \"x\"");
    assert!(
        bare_actions
            .iter()
            .any(|action| action.title == "Add `encoded` channel mode"),
        "actions={bare_actions:#?}"
    );
    assert!(
        actions(
            "avenger 1; chart cartesian { mark symbol { y: value 1; } }",
            "value 1"
        )
        .iter()
        .any(|action| action.title == "Use `direct` channel mode")
    );
    assert!(
        actions(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'a' { when { predicate: true; scaled: 'b'; } } } }",
            "scaled:"
        )
            .iter()
            .any(|action| action.title == "Use `encoded:` channel branch")
    );
    assert!(
        actions(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'a' { otherwise: { value: 'b'; } } } }",
            "value:"
        )
            .iter()
            .any(|action| action.title == "Use `direct:` channel branch")
    );
    assert!(
        actions(modern_source, "scale:")
            .iter()
            .any(|action| action.title == "Remove ineffective `scale:` channel configuration")
    );
}

#[test]
fn pin_import_target_is_offered_only_for_unpinned_remote_imports() {
    let source = "avenger 1; import * as defs from 'https://example.test/badge.avenger'; chart cartesian as chart {}";
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
                        end: start + "https://example.test/badge.avenger".len(),
                    },
                },
                source_revision: revision,
                diagnostic_codes: Vec::new(),
            },
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .expect("pin target");
    assert_eq!(target.url, "https://example.test/badge.avenger");
    assert_eq!(
        &source[target.insertion_span.range.start - 1..target.insertion_span.range.start],
        "'"
    );

    let pinned = source.replace("badge.avenger'", "badge.avenger' sha256 'abc'");
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
    let url = "https://example.test/badge.avenger";
    let origin = SourceOrigin::Http(url.to_owned());
    let definition = "avenger 1; export define mark badge { mark symbol {} }";
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
    let definition_origin = SourceOrigin::Memory("editing/badge.avenger".to_owned());
    let chart_origin = SourceOrigin::Memory("editing/chart.avenger".to_owned());
    let definition = "avenger 1; export define mark badge { mark symbol {} }";
    let chart = "avenger 1; import * as defs from 'badge.avenger'; chart cartesian as chart { mark defs.badge as imported {} }";
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

#[test]
fn rename_preserves_local_import_bindings_across_export_changes() {
    let definition_origin = SourceOrigin::Memory("editing/library.avenger".to_owned());
    let direct_origin = SourceOrigin::Memory("editing/direct.avenger".to_owned());
    let alias_origin = SourceOrigin::Memory("editing/alias.avenger".to_owned());
    let namespace_origin = SourceOrigin::Memory("editing/namespace.avenger".to_owned());
    let definition = "avenger 1; export define mark badge { mark symbol {} }";
    let direct = "avenger 1; import { badge } from 'library.avenger'; chart cartesian { mark badge as direct {} }";
    let alias = "avenger 1; import { badge as b } from 'library.avenger'; chart cartesian { mark b as aliased {} }";
    let namespace = "avenger 1; import * as defs from 'library.avenger'; chart cartesian { mark defs.badge as qualified {} }";
    let sources = [
        (definition_origin.clone(), definition),
        (direct_origin.clone(), direct),
        (alias_origin.clone(), alias),
        (namespace_origin.clone(), namespace),
    ];
    let syntax = sources
        .iter()
        .map(|(origin, text)| {
            (
                origin.clone(),
                analyze_syntax(&DocumentSnapshot::new(
                    origin.clone(),
                    SourceRevision::from_text(text),
                    *text,
                )),
            )
        })
        .collect();
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(3),
        PathBuf::from("editing"),
        sources.iter().map(|(origin, _)| origin.clone()).collect(),
        syntax,
        test_registry(),
    );

    let edit = analysis
        .rename(
            &PositionRequest {
                source: definition_origin.clone(),
                byte_offset: definition.find("badge").unwrap() + 1,
                source_revision: SourceRevision::from_text(definition),
            },
            "status_badge",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    let apply = |origin: &SourceOrigin, source: &str| {
        let mut output = source.to_owned();
        for edit in edit.sources[origin].edits.iter().rev() {
            output.replace_range(edit.span.range.as_range(), &edit.new_text);
        }
        output
    };
    assert!(apply(&definition_origin, definition).contains("define mark status_badge"));
    let renamed_direct = apply(&direct_origin, direct);
    assert!(renamed_direct.contains("import { status_badge as badge }"));
    assert!(renamed_direct.contains("mark badge as direct"));
    let renamed_alias = apply(&alias_origin, alias);
    assert!(renamed_alias.contains("import { status_badge as b }"));
    assert!(renamed_alias.contains("mark b as aliased"));
    let renamed_namespace = apply(&namespace_origin, namespace);
    assert!(renamed_namespace.contains("mark defs.status_badge as qualified"));
}

#[test]
fn rename_of_unaliased_import_introduces_an_explicit_local_alias() {
    let definition_origin = SourceOrigin::Memory("editing/library.avenger".to_owned());
    let chart_origin = SourceOrigin::Memory("editing/chart.avenger".to_owned());
    let definition = "avenger 1; export define mark badge { mark symbol {} }";
    let chart = "avenger 1; import { badge } from 'library.avenger'; chart cartesian { mark badge as direct {} }";
    let syntax = BTreeMap::from([
        (
            definition_origin.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                definition_origin.clone(),
                SourceRevision::from_text(definition),
                definition,
            )),
        ),
        (
            chart_origin.clone(),
            analyze_syntax(&DocumentSnapshot::new(
                chart_origin.clone(),
                SourceRevision::from_text(chart),
                chart,
            )),
        ),
    ]);
    let analysis = WorkspaceAnalysis::syntax_only(
        AnalysisGeneration::new(4),
        PathBuf::from("editing"),
        vec![definition_origin, chart_origin.clone()],
        syntax,
        test_registry(),
    );
    let edit = analysis
        .rename(
            &PositionRequest {
                source: chart_origin.clone(),
                byte_offset: chart.find("badge").unwrap() + 1,
                source_revision: SourceRevision::from_text(chart),
            },
            "marker",
            &AnalysisCancellation::default(),
        )
        .unwrap();
    assert_eq!(edit.sources.len(), 1);
    let mut renamed = chart.to_owned();
    for edit in edit.sources[&chart_origin].edits.iter().rev() {
        renamed.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    assert!(renamed.contains("import { badge as marker }"));
    assert!(renamed.contains("mark marker as direct"));
}

#[tokio::test]
async fn inline_definition_uses_the_compilers_canonical_expansion() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let chart_path = root.join("chart.avenger");
    let definition_path = root.join("badge.avenger");
    let chart = "avenger 1; import { badge } from 'badge.avenger'; chart cartesian as chart { mark badge as imported {} }";
    let definition =
        "avenger 1; export define mark badge { mark symbol as body { x: direct 1; y: direct 2; } }";
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
                roots: vec![ModuleRoot::requested(chart_origin.clone())],
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
    assert!(edit.new_text.starts_with("mark group as imported"));
    assert!(edit.new_text.contains("component_kind: badge"));
    assert!(edit.new_text.contains("private mark symbol"));

    let mut inlined = chart.to_owned();
    inlined.replace_range(edit.span.range.as_range(), &edit.new_text);
    std::fs::write(&chart_path, inlined).unwrap();
    compiler.check_module(&chart_path).await.unwrap();
}

#[tokio::test]
async fn extract_definition_creates_a_compiling_file_and_infers_scalar_slots() {
    let directory = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(directory.path()).unwrap();
    let chart_path = root.join("chart.avenger");
    let chart = r#"avenger 1;

chart cartesian as chart {
  param 32.0 as point_size;
  mark group as cluster {
    -- keep this authored explanation
    mark symbol as point { x: direct 1; y: direct 2; size: encoded $point_size; }
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
                roots: vec![ModuleRoot::requested(chart_origin.clone())],
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
    let start = chart.find("mark group as cluster").unwrap();
    let actions = analysis
        .code_actions(
            &CodeActionRequest {
                source: chart_origin.clone(),
                range: SourceSpan {
                    source: analysis.syntax[&chart_origin].parsed.tokens.source(),
                    range: ByteSpan {
                        start,
                        end: start + "mark group as cluster".len(),
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
    let definition_origin = SourceOrigin::File(chart_path.with_file_name("cluster.avenger"));
    let definition = &action.edit.create_files[&definition_origin];
    assert!(definition.contains("export define mark cluster"));
    assert!(definition.contains("slot expr point_size"));
    assert!(definition.contains("size: encoded point_size"));
    assert!(definition.contains("-- keep this authored explanation"));

    let mut extracted = chart.to_owned();
    let mut edits = action.edit.sources[&chart_origin].edits.clone();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.range.start));
    for edit in edits {
        extracted.replace_range(edit.span.range.as_range(), &edit.new_text);
    }
    std::fs::write(&chart_path, extracted).unwrap();
    std::fs::write(chart_path.with_file_name("cluster.avenger"), definition).unwrap();
    compiler.check_module(&chart_path).await.unwrap();
}
