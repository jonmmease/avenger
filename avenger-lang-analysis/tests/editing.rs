use std::{collections::BTreeMap, path::PathBuf};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeSchemaSnapshot, PropertySchema, SchemaVersion, ValueShape,
};
use avenger_lang_analysis::{
    AnalysisCancellation, AnalysisGeneration, CodeActionRequest, DocumentRequest, DocumentSnapshot,
    LineEnding, PositionRequest, RenameError, SemanticTokenKind, SourceRevision, WorkspaceAnalysis,
    analyze_syntax,
};
use avenger_lang_core::{ByteSpan, SourceOrigin, SourceSpan};

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
  param as width { type: float64; default: 10.0; }
  param as height { type: float64; default: 20.0; }
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
    assert!(renamed.contains("param as canvas_width"));
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
    assert!(inserted.contains("param as threshold"));
    assert!(inserted.contains("type: float64"));

    let missing_as = "avenger 1; chart cartesian as chart { param width { type: float64; } }";
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
