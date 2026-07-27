//! Conservative source edits and semantic presentation.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_schema::ValueShape;
use avenger_lang_core::{
    ByteSpan, SourceFile, SourceId, SourceOrigin, SourceSpan,
    ast::{ImportClause, Name},
    module_graph::normalize_path,
    sql::{LosslessTokenKind, TokenClass},
    syntax::{ImportClauseSyntax, format_source, parse_file},
};
use sqlparser::tokenizer::Token;

use crate::{
    AnalysisCancellation, AnalysisQueryError, CodeAction, CodeActionKind, CodeActionRequest,
    CompletionOptions, DocumentRequest, FormattingResult, IndexedReference, IndexedSymbol,
    IndexedValueKind, LineEnding, PinImportTarget, PositionRequest, PrepareRenameResult,
    SemanticToken, SemanticTokenKind, SemanticTokenModifiers, SemanticTokensResult, SourceTextEdit,
    VersionedSourceEdits, WorkspaceAnalysis, WorkspaceEdit,
};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RenameError {
    #[error(transparent)]
    Query(#[from] AnalysisQueryError),
    #[error("the cursor is not on a safely renameable authored symbol")]
    NotRenameable,
    #[error("`{0}` is not a valid Avenger name")]
    InvalidName(String),
    #[error("`{0}` already exists in the affected scope")]
    Collision(String),
    #[error("the semantic index does not contain a safe authored span for every reference")]
    IncompleteReferences,
}

pub(crate) fn format_document(
    analysis: &WorkspaceAnalysis,
    request: &DocumentRequest,
    line_ending: LineEnding,
    cancellation: &AnalysisCancellation,
) -> Result<Option<FormattingResult>, AnalysisQueryError> {
    let syntax = document_syntax(analysis, request, cancellation)?;
    let text = syntax.parsed.tokens.text();
    let source = SourceFile::new(SourceId::new(0), request.source.clone(), text.to_owned());
    let Ok(formatted) = format_source(&source) else {
        return Ok(None);
    };
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;
    let formatted = apply_line_ending(&formatted, line_ending);
    if formatted == text {
        return Ok(None);
    }
    Ok(Some(FormattingResult {
        edit: SourceTextEdit {
            span: SourceSpan {
                source: source.id,
                range: ByteSpan {
                    start: 0,
                    end: text.len(),
                },
            },
            new_text: formatted,
        },
        generation: analysis.generation,
        source_revision: request.source_revision.clone(),
    }))
}

fn apply_line_ending(text: &str, line_ending: LineEnding) -> String {
    match line_ending {
        LineEnding::Lf => text.to_owned(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
        LineEnding::Cr => text.replace('\n', "\r"),
    }
}

pub(crate) fn semantic_tokens(
    analysis: &WorkspaceAnalysis,
    request: &DocumentRequest,
    cancellation: &AnalysisCancellation,
) -> Result<SemanticTokensResult, AnalysisQueryError> {
    let syntax = document_syntax(analysis, request, cancellation)?;
    let Some(document) = analysis.semantic_index.documents.get(&request.source) else {
        return Ok(SemanticTokensResult {
            tokens: Vec::new(),
            generation: analysis.generation,
            source_revision: request.source_revision.clone(),
        });
    };
    let mut tokens = Vec::new();
    for symbol in &document.symbols {
        tokens.push(SemanticToken {
            span: symbol.selection_span,
            kind: semantic_kind(symbol.value_kind, true, true),
            modifiers: SemanticTokenModifiers {
                declaration: true,
                readonly: false,
                deprecated: false,
                default_library: false,
            },
        });
    }
    for span in document.property_names.keys() {
        tokens.push(SemanticToken {
            span: *span,
            kind: SemanticTokenKind::Property,
            modifiers: SemanticTokenModifiers::default(),
        });
    }
    for span in crate::intelligence::physical_type_spans(syntax) {
        tokens.push(SemanticToken {
            span,
            kind: SemanticTokenKind::Type,
            modifiers: SemanticTokenModifiers::default(),
        });
    }
    for reference in &document.references {
        if let Some(span) = semantic_reference_span(analysis, reference) {
            tokens.push(SemanticToken {
                span,
                kind: if reference.target_identity.is_some() {
                    semantic_kind(reference.value_kind, false, false)
                } else {
                    SemanticTokenKind::UnresolvedReference
                },
                modifiers: SemanticTokenModifiers::default(),
            });
        }
    }
    let text = syntax.parsed.tokens.text();
    tokens.retain(|token| {
        token.span.range.start < token.span.range.end
            && token.span.range.end <= text.len()
            && !text[token.span.range.as_range()].contains(['\n', '\r'])
    });
    tokens.sort_by_key(|token| (token.span.range.start, token.span.range.len()));
    let mut end = 0;
    tokens.retain(|token| {
        if token.span.range.start < end {
            false
        } else {
            end = token.span.range.end;
            true
        }
    });
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;
    Ok(SemanticTokensResult {
        tokens,
        generation: analysis.generation,
        source_revision: request.source_revision.clone(),
    })
}

fn semantic_reference_span(
    analysis: &WorkspaceAnalysis,
    reference: &IndexedReference,
) -> Option<SourceSpan> {
    let syntax = analysis.syntax.get(&reference.origin)?;
    let raw = syntax
        .parsed
        .tokens
        .text()
        .get(reference.span.range.as_range())?;
    let relative = if raw.starts_with('$') {
        let name = reference.name.split('.').next()?;
        1..1 + name.len()
    } else {
        let name = reference.name.rsplit('.').next()?;
        if !raw.ends_with(name) {
            return None;
        }
        raw.len() - name.len()..raw.len()
    };
    Some(SourceSpan {
        source: reference.span.source,
        range: ByteSpan {
            start: reference.span.range.start + relative.start,
            end: reference.span.range.start + relative.end,
        },
    })
}

fn semantic_kind(kind: IndexedValueKind, declaration: bool, authored: bool) -> SemanticTokenKind {
    match kind {
        IndexedValueKind::Scalar => SemanticTokenKind::Parameter,
        IndexedValueKind::Field | IndexedValueKind::Output => SemanticTokenKind::Field,
        IndexedValueKind::Declaration if declaration && authored => SemanticTokenKind::Binding,
        IndexedValueKind::Event => SemanticTokenKind::Function,
        IndexedValueKind::Declaration
        | IndexedValueKind::Table
        | IndexedValueKind::Selection
        | IndexedValueKind::Mark
        | IndexedValueKind::Tool
        | IndexedValueKind::Widget => SemanticTokenKind::Variable,
    }
}

pub(crate) fn prepare_rename(
    analysis: &WorkspaceAnalysis,
    request: &PositionRequest,
    cancellation: &AnalysisCancellation,
) -> Result<Option<PrepareRenameResult>, AnalysisQueryError> {
    position_syntax(analysis, request, cancellation)?;
    let Some(symbol) = rename_symbol(analysis, request) else {
        return Ok(None);
    };
    if !references_are_complete(analysis, symbol) {
        return Ok(None);
    }
    Ok(Some(PrepareRenameResult {
        span: rename_span_at_cursor(analysis, request, symbol).unwrap_or(symbol.selection_span),
        placeholder: symbol.name.clone(),
        generation: analysis.generation,
        source_revision: request.source_revision.clone(),
    }))
}

pub(crate) fn rename(
    analysis: &WorkspaceAnalysis,
    request: &PositionRequest,
    new_name: &str,
    cancellation: &AnalysisCancellation,
) -> Result<WorkspaceEdit, RenameError> {
    position_syntax(analysis, request, cancellation)?;
    Name::new(new_name.to_owned()).map_err(|_| RenameError::InvalidName(new_name.to_owned()))?;
    let symbol = rename_symbol(analysis, request).ok_or(RenameError::NotRenameable)?;
    if new_name == symbol.name {
        return Ok(WorkspaceEdit::default());
    }
    if analysis
        .semantic_index
        .documents
        .get(&symbol.origin)
        .is_some_and(|document| {
            document.symbols.iter().any(|candidate| {
                candidate.identity != symbol.identity
                    && candidate.parent == symbol.parent
                    && candidate.name == new_name
            })
        })
    {
        return Err(RenameError::Collision(new_name.to_owned()));
    }
    if !references_are_complete(analysis, symbol) {
        return Err(RenameError::IncompleteReferences);
    }

    let mut replacements = BTreeMap::<SourceOrigin, BTreeMap<SourceSpan, String>>::new();
    let declaration_replacement = unaliased_import_name(analysis, symbol).map_or_else(
        || new_name.to_owned(),
        |imported| format!("{imported} as {new_name}"),
    );
    replacements
        .entry(symbol.origin.clone())
        .or_default()
        .insert(symbol.selection_span, declaration_replacement);
    for document in analysis.semantic_index.documents.values() {
        for reference in document
            .references
            .iter()
            .filter(|reference| reference.target_identity.as_deref() == Some(&symbol.identity))
        {
            if symbol.exported
                && symbol.parent.is_none()
                && is_named_import_reference(analysis, symbol, reference)
            {
                continue;
            }
            let Some(span) = reference_name_span(analysis, reference, &symbol.name) else {
                return Err(RenameError::IncompleteReferences);
            };
            replacements
                .entry(reference.origin.clone())
                .or_default()
                .insert(span, new_name.to_owned());
        }
    }
    if symbol.keyword == "import" {
        add_local_import_reference_replacements(analysis, symbol, new_name, &mut replacements)?;
    }
    if symbol.exported && symbol.parent.is_none() {
        add_export_import_replacements(analysis, symbol, new_name, &mut replacements)?;
    }
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;

    let mut sources = BTreeMap::new();
    for (origin, replacements) in replacements {
        let Some(syntax) = analysis.syntax.get(&origin) else {
            return Err(RenameError::IncompleteReferences);
        };
        let mut edits = replacements
            .into_iter()
            .map(|(span, new_text)| SourceTextEdit { span, new_text })
            .collect::<Vec<_>>();
        edits.sort_by_key(|edit| edit.span.range.start);
        sources.insert(
            origin,
            VersionedSourceEdits {
                source_revision: syntax.revision.clone(),
                edits,
            },
        );
    }
    Ok(WorkspaceEdit {
        sources,
        create_files: BTreeMap::new(),
    })
}

fn unaliased_import_name(analysis: &WorkspaceAnalysis, symbol: &IndexedSymbol) -> Option<String> {
    if symbol.keyword != "import" {
        return None;
    }
    let strict = analysis
        .syntax
        .get(&symbol.origin)?
        .parsed
        .strict
        .as_ref()?;
    for (import, syntax) in strict.ast.imports.iter().zip(&strict.module_syntax.imports) {
        let (
            ImportClause::Named(specifiers),
            ImportClauseSyntax::Named {
                specifiers: spans, ..
            },
        ) = (&import.clause, &syntax.clause)
        else {
            continue;
        };
        for (specifier, spans) in specifiers.iter().zip(spans) {
            if spans.local.range == symbol.selection_span.range
                && spans.alias_keyword.is_none()
                && specifier.imported == specifier.local
            {
                return Some(specifier.imported.to_string());
            }
        }
    }
    None
}

fn add_export_import_replacements(
    analysis: &WorkspaceAnalysis,
    symbol: &IndexedSymbol,
    new_name: &str,
    replacements: &mut BTreeMap<SourceOrigin, BTreeMap<SourceSpan, String>>,
) -> Result<(), RenameError> {
    for (importer, syntax) in &analysis.syntax {
        let Some(strict) = syntax.parsed.strict.as_ref() else {
            continue;
        };
        for (import, import_syntax) in strict.ast.imports.iter().zip(&strict.module_syntax.imports)
        {
            if resolve_import_origin(importer, &import.source).as_ref() != Some(&symbol.origin) {
                continue;
            }
            let (
                ImportClause::Named(specifiers),
                ImportClauseSyntax::Named {
                    specifiers: spans, ..
                },
            ) = (&import.clause, &import_syntax.clause)
            else {
                continue;
            };
            for (specifier, spans) in specifiers.iter().zip(spans) {
                if specifier.imported.as_str() != symbol.name {
                    continue;
                }
                let replacement = if spans.alias_keyword.is_none() {
                    format!("{new_name} as {}", specifier.local)
                } else {
                    new_name.to_owned()
                };
                let prior = replacements
                    .entry(importer.clone())
                    .or_default()
                    .insert(spans.imported, replacement.clone());
                if prior.is_some_and(|prior| prior != replacement) {
                    return Err(RenameError::IncompleteReferences);
                }
            }
        }
    }
    Ok(())
}

fn is_named_import_reference(
    analysis: &WorkspaceAnalysis,
    symbol: &IndexedSymbol,
    reference: &IndexedReference,
) -> bool {
    let Some(strict) = analysis
        .syntax
        .get(&reference.origin)
        .and_then(|syntax| syntax.parsed.strict.as_ref())
    else {
        return false;
    };
    strict
        .ast
        .imports
        .iter()
        .zip(&strict.module_syntax.imports)
        .filter(|(import, _)| {
            resolve_import_origin(&reference.origin, &import.source).as_ref()
                == Some(&symbol.origin)
        })
        .any(|(import, syntax)| {
            let (
                ImportClause::Named(specifiers),
                ImportClauseSyntax::Named {
                    specifiers: spans, ..
                },
            ) = (&import.clause, &syntax.clause)
            else {
                return false;
            };
            specifiers.iter().zip(spans).any(|(specifier, spans)| {
                specifier.imported.as_str() == symbol.name
                    && (reference.name == specifier.local.as_str()
                        || reference.span == spans.imported)
            })
        })
}

fn add_local_import_reference_replacements(
    analysis: &WorkspaceAnalysis,
    symbol: &IndexedSymbol,
    new_name: &str,
    replacements: &mut BTreeMap<SourceOrigin, BTreeMap<SourceSpan, String>>,
) -> Result<(), RenameError> {
    let Some(strict) = analysis
        .syntax
        .get(&symbol.origin)
        .and_then(|syntax| syntax.parsed.strict.as_ref())
    else {
        return Ok(());
    };
    let local = strict
        .ast
        .imports
        .iter()
        .zip(&strict.module_syntax.imports)
        .find_map(|(import, syntax)| {
            let (
                ImportClause::Named(specifiers),
                ImportClauseSyntax::Named {
                    specifiers: spans, ..
                },
            ) = (&import.clause, &syntax.clause)
            else {
                return None;
            };
            specifiers.iter().zip(spans).find_map(|(specifier, spans)| {
                (spans.local.range == symbol.selection_span.range).then(|| {
                    (
                        specifier.local.to_string(),
                        resolve_import_origin(&symbol.origin, &import.source),
                        specifier.imported.to_string(),
                    )
                })
            })
        });
    let Some((local, imported_origin, imported_name)) = local else {
        return Ok(());
    };
    let target_identity = imported_origin.and_then(|origin| {
        analysis
            .semantic_index
            .documents
            .get(&origin)?
            .symbols
            .iter()
            .find(|candidate| {
                candidate.parent.is_none() && candidate.exported && candidate.name == imported_name
            })
            .map(|candidate| candidate.identity.clone())
    });
    let Some(document) = analysis.semantic_index.documents.get(&symbol.origin) else {
        return Ok(());
    };
    for reference in document.references.iter().filter(|reference| {
        reference.name == local
            && target_identity
                .as_deref()
                .is_some_and(|identity| reference.target_identity.as_deref() == Some(identity))
    }) {
        let Some(span) = reference_name_span(analysis, reference, &local) else {
            return Err(RenameError::IncompleteReferences);
        };
        replacements
            .entry(symbol.origin.clone())
            .or_default()
            .insert(span, new_name.to_owned());
    }
    Ok(())
}

fn resolve_import_origin(importer: &SourceOrigin, source: &str) -> Option<SourceOrigin> {
    match importer {
        SourceOrigin::File(path) => Some(SourceOrigin::File(normalize_path(
            &path.parent()?.join(source),
        ))),
        SourceOrigin::Memory(path) => Some(SourceOrigin::Memory(
            normalize_path(&std::path::Path::new(path).parent()?.join(source))
                .to_string_lossy()
                .into_owned(),
        )),
        SourceOrigin::Std(path) => Some(SourceOrigin::Std(
            normalize_path(&std::path::Path::new(path).parent()?.join(source))
                .to_string_lossy()
                .into_owned(),
        )),
        SourceOrigin::Http(_) => None,
    }
}

pub(crate) fn code_actions(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    cancellation: &AnalysisCancellation,
) -> Result<Vec<CodeAction>, AnalysisQueryError> {
    document_syntax(
        analysis,
        &DocumentRequest {
            source: request.source.clone(),
            source_revision: request.source_revision.clone(),
        },
        cancellation,
    )?;
    let mut actions = Vec::new();
    close_property_action(analysis, request, &mut actions);
    missing_as_action(analysis, request, &mut actions);
    ambiguous_qualification_actions(analysis, request, cancellation, &mut actions);
    missing_param_action(analysis, request, &mut actions);
    inline_definition_action(analysis, request, &mut actions);
    extract_definition_action(analysis, request, &mut actions);
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;
    actions.sort_by(|left, right| {
        (!left.preferred, &left.title).cmp(&(!right.preferred, &right.title))
    });
    actions.dedup_by(|left, right| left.title == right.title && left.edit == right.edit);
    Ok(actions)
}

pub(crate) fn pin_import_target(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    cancellation: &AnalysisCancellation,
) -> Result<Option<PinImportTarget>, AnalysisQueryError> {
    let syntax = document_syntax(
        analysis,
        &DocumentRequest {
            source: request.source.clone(),
            source_revision: request.source_revision.clone(),
        },
        cancellation,
    )?;
    let Some(strict) = syntax.parsed.strict.as_ref() else {
        return Ok(None);
    };
    let Some((import, span)) = strict
        .ast
        .imports
        .iter()
        .zip(
            strict
                .module_syntax
                .imports
                .iter()
                .map(|import| import.span),
        )
        .find(|(import, span)| {
            import.sha256.is_none()
                && (import.source.starts_with("https://") || import.source.starts_with("http://"))
                && spans_overlap(*span, request.range)
        })
    else {
        return Ok(None);
    };
    let Some(source_token) = syntax.parsed.tokens.tokens().iter().find(|token| {
        span.range.start <= token.span().range.start
            && token.span().range.end <= span.range.end
            && matches!(token.kind(), LosslessTokenKind::Token(TokenClass::String))
    }) else {
        return Ok(None);
    };
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;
    Ok(Some(PinImportTarget {
        url: import.source.clone(),
        insertion_span: SourceSpan::empty(span.source, source_token.span().range.end),
        generation: analysis.generation,
        source_revision: request.source_revision.clone(),
    }))
}

fn inline_definition_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let Some(declaration) = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                &node.kind,
                avenger_lang_core::syntax::TolerantSyntaxNodeKind::Declaration { keyword, .. }
                    if matches!(keyword.as_str(), "mark" | "tool" | "transform")
            ) && spans_overlap(node.span, request.range)
        })
        .min_by_key(|node| node.span.range.len())
    else {
        return;
    };

    for root in analysis.semantic_roots.values() {
        let Ok(project) = &root.result else {
            continue;
        };
        let Some(resolved) = project.resolved_module_graph.as_deref() else {
            continue;
        };
        let Some(mapping) = resolved
            .expansion_source_map
            .mappings
            .iter()
            .find(|mapping| {
                let Some(instantiation) = mapping.instantiation else {
                    return false;
                };
                mapping.definition.is_some()
                    && mapping.authored == instantiation
                    && instantiation.range == declaration.span.range
                    && resolved
                        .sources
                        .get(instantiation.source)
                        .is_some_and(|source| source.origin == request.source)
            })
        else {
            continue;
        };
        let Some(expanded_source) = resolved.sources.get(mapping.expanded.source) else {
            continue;
        };
        let Some(expanded) = expanded_source
            .text()
            .get(mapping.expanded.range.as_range())
        else {
            continue;
        };
        let authored = syntax.parsed.tokens.text();
        let replacement = reindent_expanded_declaration(
            expanded_source.text(),
            mapping.expanded.range.start,
            expanded,
            authored,
            declaration.span.range.start,
        );
        output.push(CodeAction {
            title: "Inline imported definition".to_owned(),
            kind: CodeActionKind::RefactorInline,
            diagnostic_codes: Vec::new(),
            preferred: false,
            edit: WorkspaceEdit {
                sources: BTreeMap::from([(
                    request.source.clone(),
                    VersionedSourceEdits {
                        source_revision: request.source_revision.clone(),
                        edits: vec![SourceTextEdit {
                            span: declaration.span,
                            new_text: replacement,
                        }],
                    },
                )]),
                create_files: BTreeMap::new(),
            },
        });
        return;
    }
}

fn extract_definition_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let SourceOrigin::File(source_path) = &request.source else {
        return;
    };
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let Some(document) = analysis.semantic_index.documents.get(&request.source) else {
        return;
    };
    let Some(group) = document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.keyword == "mark"
                && symbol.native_kind.as_deref() == Some("group")
                && !symbol.name.is_empty()
                && spans_overlap(symbol.declaration_span, request.range)
        })
        .min_by_key(|symbol| symbol.declaration_span.range.len())
    else {
        return;
    };
    if Name::new(group.name.clone()).is_err() {
        return;
    }
    let definition_path = source_path.with_file_name(format!("{}.avenger", group.name));
    if definition_path.exists() {
        return;
    }
    let definition_origin = SourceOrigin::File(definition_path.clone());
    if analysis.syntax.contains_key(&definition_origin) {
        return;
    }

    let tokens = syntax.parsed.tokens.tokens();
    let Some(open) = tokens
        .iter()
        .find(|token| {
            group.declaration_span.range.start <= token.span().range.start
                && token.span().range.end <= group.declaration_span.range.end
                && matches!(token.token(), Some(Token::LBrace))
        })
        .map(|token| token.span())
    else {
        return;
    };
    let Some(close) = tokens
        .iter()
        .rev()
        .find(|token| {
            group.declaration_span.range.start <= token.span().range.start
                && token.span().range.end <= group.declaration_span.range.end
                && matches!(token.token(), Some(Token::RBrace))
        })
        .map(|token| token.span())
    else {
        return;
    };
    let text = syntax.parsed.tokens.text();
    let body_range = open.range.end..close.range.start;
    let Some(body) = text.get(body_range.clone()) else {
        return;
    };

    let mut free_scalars = BTreeMap::<String, Vec<SourceSpan>>::new();
    for reference in document.references.iter().filter(|reference| {
        group.declaration_span.range.start <= reference.span.range.start
            && reference.span.range.end <= group.declaration_span.range.end
    }) {
        let target = reference.target_identity.as_deref().and_then(|identity| {
            analysis
                .semantic_index
                .documents
                .values()
                .flat_map(|document| &document.symbols)
                .find(|symbol| symbol.identity == identity)
        });
        let internal = target.is_some_and(|symbol| {
            symbol.origin == request.source
                && group.declaration_span.range.start <= symbol.declaration_span.range.start
                && symbol.declaration_span.range.end <= group.declaration_span.range.end
        });
        if internal {
            continue;
        }
        if reference.value_kind != IndexedValueKind::Scalar
            || reference.name.contains('.')
            || !text[reference.span.range.as_range()].starts_with('$')
        {
            return;
        }
        free_scalars
            .entry(reference.name.clone())
            .or_default()
            .push(reference.span);
    }
    if free_scalars.keys().any(|name| {
        document.symbols.iter().any(|symbol| {
            symbol.name == *name
                && group.declaration_span.range.start <= symbol.declaration_span.range.start
                && symbol.declaration_span.range.end <= group.declaration_span.range.end
        })
    }) {
        return;
    }

    let mut definition_body = body.to_owned();
    let mut replacements = free_scalars
        .iter()
        .flat_map(|(name, spans)| {
            spans
                .iter()
                .map(move |span| (span.range.as_range(), name.as_str()))
        })
        .collect::<Vec<_>>();
    replacements.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, name) in replacements {
        let relative = range.start - body_range.start..range.end - body_range.start;
        definition_body.replace_range(relative, name);
    }
    let slots = free_scalars
        .keys()
        .map(|name| format!("  slot expr {name};\n"))
        .collect::<String>();
    let raw_definition = format!(
        "avenger 1;\n\nexport define mark {} {{\n{}{definition_body}\n}}\n",
        group.name, slots
    );
    let definition_file =
        SourceFile::new(SourceId::new(0), definition_origin.clone(), raw_definition);
    let Ok(definition_text) = format_source(&definition_file) else {
        return;
    };

    let properties = free_scalars
        .keys()
        .map(|name| format!(" {name}: ${name};"))
        .collect::<String>();
    let replacement = format!("mark {} as {} {{{properties} }}", group.name, group.name);
    let root_start = syntax
        .parsed
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Declaration { keyword, .. }
                if keyword == "chart"
                    && node.span.range.start <= group.declaration_span.range.start
                    && group.declaration_span.range.end <= node.span.range.end =>
            {
                Some(node.span.range.start)
            }
            _ => None,
        })
        .unwrap_or(group.declaration_span.range.start);
    let Some(import_at) = text[..root_start].rfind(';').map(|position| position + 1) else {
        return;
    };
    let Some(definition_file_name) = definition_path.file_name() else {
        return;
    };
    let import = format!(
        "\nimport {{ {} }} from '{}';",
        group.name,
        definition_file_name.to_string_lossy()
    );

    let mut candidate = text.to_owned();
    candidate.replace_range(group.declaration_span.range.as_range(), &replacement);
    candidate.insert_str(import_at, &import);
    let candidate_file = SourceFile::new(SourceId::new(0), request.source.clone(), candidate);
    if parse_file(&candidate_file).is_err() {
        return;
    }

    output.push(CodeAction {
        title: format!("Extract mark group as `{}` definition", group.name),
        kind: CodeActionKind::RefactorExtract,
        diagnostic_codes: Vec::new(),
        preferred: false,
        edit: WorkspaceEdit {
            sources: BTreeMap::from([(
                request.source.clone(),
                VersionedSourceEdits {
                    source_revision: request.source_revision.clone(),
                    edits: vec![
                        SourceTextEdit {
                            span: group.declaration_span,
                            new_text: replacement,
                        },
                        SourceTextEdit {
                            span: SourceSpan::empty(group.declaration_span.source, import_at),
                            new_text: import,
                        },
                    ],
                },
            )]),
            create_files: BTreeMap::from([(definition_origin, definition_text)]),
        },
    });
}

fn reindent_expanded_declaration(
    expanded_source: &str,
    expanded_start: usize,
    expanded: &str,
    authored_source: &str,
    authored_start: usize,
) -> String {
    fn indent_before(text: &str, offset: usize) -> &str {
        let start = text[..offset]
            .rfind(['\n', '\r'])
            .map_or(0, |position| position + 1);
        let prefix = &text[start..offset];
        let whitespace_start = prefix
            .char_indices()
            .rev()
            .find(|(_, character)| !character.is_whitespace())
            .map_or(0, |(position, character)| position + character.len_utf8());
        &prefix[whitespace_start..]
    }

    let from = indent_before(expanded_source, expanded_start);
    let to = indent_before(authored_source, authored_start);
    let mut lines = expanded.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return String::new();
    };
    let mut output = first.to_owned();
    for line in lines {
        if let Some(rest) = line.strip_prefix(from) {
            output.push_str(to);
            output.push_str(rest);
        } else {
            output.push_str(line);
        }
    }
    output
}

fn close_property_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(document) = analysis.semantic_index.documents.get(&request.source) else {
        return;
    };
    let Some((span, authored)) = document
        .property_names
        .iter()
        .find(|(span, _)| spans_overlap(**span, request.range))
    else {
        return;
    };
    let Some(owner) = crate::intelligence::owner_symbol(
        &analysis.semantic_index,
        &request.source,
        span.range.start,
    ) else {
        return;
    };
    let schema =
        crate::intelligence::schema_for_symbol(&analysis.registry, owner, &analysis.semantic_index);
    if schema.is_some_and(|schema| {
        crate::intelligence::property_schema(schema, authored).is_some()
            || schema.additional_properties.is_some()
    }) || crate::intelligence::core_properties(&owner.keyword)
        .iter()
        .any(|(name, _)| *name == authored)
    {
        return;
    }
    let mut candidates = schema
        .into_iter()
        .flat_map(|schema| schema.properties.keys().chain(schema.channels.keys()))
        .map(String::as_str)
        .chain(
            crate::intelligence::core_properties(&owner.keyword)
                .iter()
                .map(|(name, _)| *name),
        )
        .map(|candidate| (edit_distance(authored, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .collect::<Vec<_>>();
    candidates.sort_unstable();
    let Some((distance, candidate)) = candidates.first().copied() else {
        return;
    };
    if candidates
        .get(1)
        .is_some_and(|(next_distance, _)| *next_distance == distance)
    {
        return;
    }
    output.push(quick_fix(
        format!("Replace `{authored}` with `{candidate}`"),
        request,
        *span,
        candidate.to_owned(),
        true,
    ));
}

fn missing_as_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let Some(declaration) = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                avenger_lang_core::syntax::TolerantSyntaxNodeKind::Declaration { .. }
            ) && spans_overlap(node.span, request.range)
        })
        .min_by_key(|node| node.span.range.len())
    else {
        return;
    };
    let avenger_lang_core::syntax::TolerantSyntaxNodeKind::Declaration { keyword, .. } =
        &declaration.kind
    else {
        return;
    };
    if !matches!(
        keyword.as_str(),
        "param" | "store" | "selection" | "mark" | "view"
    ) {
        return;
    }
    let tokens = significant_header_tokens(syntax, declaration.span);
    let Some(keyword_index) = tokens
        .iter()
        .position(|(_, token)| matches!(token, Some(Token::Word(word)) if word.value == *keyword))
    else {
        return;
    };
    if tokens[..tokens
        .iter()
        .position(|(_, token)| matches!(token, Some(Token::LBrace | Token::SemiColon)))
        .unwrap_or(tokens.len())]
        .iter()
        .any(|(_, token)| matches!(token, Some(Token::Word(word)) if word.value == "as"))
    {
        return;
    }
    let Some((name_span, Some(Token::Word(_)))) = tokens
        .iter()
        .take_while(|(_, token)| !matches!(token, Some(Token::LBrace | Token::SemiColon)))
        .skip(keyword_index + 1)
        .filter(|(_, token)| matches!(token, Some(Token::Word(_))))
        .last()
    else {
        return;
    };
    output.push(quick_fix(
        "Add missing `as` binder".to_owned(),
        request,
        SourceSpan::empty(name_span.source, name_span.range.start),
        "as ".to_owned(),
        true,
    ));
}

fn ambiguous_qualification_actions(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    cancellation: &AnalysisCancellation,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let text = syntax.parsed.tokens.text();
    let Some(authored) = text.get(request.range.range.as_range()) else {
        return;
    };
    if authored.is_empty() || authored.contains('.') {
        return;
    }
    let completion = analysis.complete(
        &PositionRequest {
            source: request.source.clone(),
            byte_offset: request.range.range.end,
            source_revision: request.source_revision.clone(),
        },
        CompletionOptions::default(),
        cancellation,
    );
    let Ok(completion) = completion else {
        return;
    };
    let insertions = completion
        .items
        .iter()
        .filter(|item| item.label == authored && item.insert_text.contains('.'))
        .map(|item| item.insert_text.clone())
        .collect::<BTreeSet<_>>();
    if insertions.len() < 2 {
        return;
    }
    for insertion in insertions {
        output.push(quick_fix(
            format!("Qualify `{authored}` as `{insertion}`"),
            request,
            request.range,
            insertion,
            false,
        ));
    }
}

fn missing_param_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let Some(document) = analysis.semantic_index.documents.get(&request.source) else {
        return;
    };
    let Some(reference) = document.references.iter().find(|reference| {
        reference.target_identity.is_none()
            && reference.value_kind == IndexedValueKind::Scalar
            && spans_overlap(reference.span, request.range)
            && !reference.name.contains('.')
    }) else {
        return;
    };
    if document
        .symbols
        .iter()
        .any(|symbol| symbol.name == reference.name)
    {
        return;
    }
    let Some(property_name) = syntax
        .parsed
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name }
                if node.span.range.start <= reference.span.range.start
                    && reference.span.range.end <= node.span.range.end =>
            {
                Some(name.as_str())
            }
            _ => None,
        })
    else {
        return;
    };
    let Some(owner) = crate::intelligence::owner_symbol(
        &analysis.semantic_index,
        &request.source,
        reference.span.range.start,
    ) else {
        return;
    };
    let Some(shape) =
        crate::intelligence::schema_for_symbol(&analysis.registry, owner, &analysis.semantic_index)
            .and_then(|schema| crate::intelligence::property_schema(schema, property_name))
            .map(|property| property.shape)
    else {
        return;
    };
    let Some(data_type) = unambiguous_physical_type(shape) else {
        return;
    };
    let Some(container) = document
        .symbols
        .iter()
        .filter(|symbol| {
            (matches!(symbol.keyword.as_str(), "chart" | "plot")
                || (symbol.keyword == "mark" && symbol.native_kind.as_deref() == Some("group")))
                && symbol.scope_span.range.start <= reference.span.range.start
                && reference.span.range.end <= symbol.scope_span.range.end
        })
        .min_by_key(|symbol| symbol.scope_span.range.len())
    else {
        return;
    };
    let Some(open) = syntax
        .parsed
        .tokens
        .tokens()
        .iter()
        .find(|token| {
            container.declaration_span.range.start <= token.span().range.start
                && token.span().range.end <= container.declaration_span.range.end
                && matches!(token.token(), Some(Token::LBrace))
        })
        .map(|token| token.span())
    else {
        return;
    };
    let text = syntax.parsed.tokens.text();
    let line_start = text[..container.declaration_span.range.start]
        .rfind(['\n', '\r'])
        .map_or(0, |position| position + 1);
    let parent_indent = text[line_start..container.declaration_span.range.start]
        .chars()
        .take_while(|character| character.is_whitespace())
        .collect::<String>();
    let indent = format!("{parent_indent}  ");
    let declaration = format!(
        "\n{indent}param {data_type} as {} {{\n{indent}  value: NULL;\n{indent}}}",
        reference.name
    );
    output.push(quick_fix(
        format!("Declare parameter `${}` as `{data_type}`", reference.name),
        request,
        SourceSpan::empty(open.source, open.range.end),
        declaration,
        true,
    ));
}

fn unambiguous_physical_type(shape: &ValueShape) -> Option<&'static str> {
    match shape {
        ValueShape::Boolean => Some("boolean"),
        ValueShape::Integer => Some("int64"),
        ValueShape::Number => Some("float64"),
        ValueShape::String => Some("utf8"),
        ValueShape::Union(shapes) => {
            let types = shapes
                .iter()
                .filter_map(unambiguous_physical_type)
                .collect::<BTreeSet<_>>();
            (types.len() == 1).then(|| *types.first().unwrap())
        }
        _ => None,
    }
}

fn significant_header_tokens(
    syntax: &crate::SyntaxAnalysis,
    declaration: SourceSpan,
) -> Vec<(SourceSpan, Option<&Token>)> {
    syntax
        .parsed
        .tokens
        .tokens()
        .iter()
        .filter(|token| {
            declaration.range.start <= token.span().range.start
                && token.span().range.end <= declaration.range.end
                && !matches!(
                    token.kind(),
                    LosslessTokenKind::Token(TokenClass::Whitespace(_) | TokenClass::Comment(_))
                        | LosslessTokenKind::Eof
                )
        })
        .map(|token| (token.span(), token.token()))
        .take_while(|(_, token)| !matches!(token, Some(Token::LBrace | Token::SemiColon)))
        .chain(
            syntax
                .parsed
                .tokens
                .tokens()
                .iter()
                .filter(|token| {
                    declaration.range.start <= token.span().range.start
                        && token.span().range.end <= declaration.range.end
                        && matches!(token.token(), Some(Token::LBrace | Token::SemiColon))
                })
                .take(1)
                .map(|token| (token.span(), token.token())),
        )
        .collect()
}

fn quick_fix(
    title: String,
    request: &CodeActionRequest,
    span: SourceSpan,
    new_text: String,
    preferred: bool,
) -> CodeAction {
    CodeAction {
        title,
        kind: CodeActionKind::QuickFix,
        diagnostic_codes: request.diagnostic_codes.clone(),
        preferred,
        edit: WorkspaceEdit {
            sources: BTreeMap::from([(
                request.source.clone(),
                VersionedSourceEdits {
                    source_revision: request.source_revision.clone(),
                    edits: vec![SourceTextEdit { span, new_text }],
                },
            )]),
            create_files: BTreeMap::new(),
        },
    }
}

fn spans_overlap(left: SourceSpan, right: SourceSpan) -> bool {
    left.source == right.source
        && left.range.start <= right.range.end
        && right.range.start <= left.range.end
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut prior = (0..=right.len()).collect::<Vec<_>>();
    for (row, left) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right) in right.iter().enumerate() {
            current.push(
                (prior[column + 1] + 1)
                    .min(current[column] + 1)
                    .min(prior[column] + usize::from(left != *right)),
            );
        }
        prior = current;
    }
    prior[right.len()]
}

fn rename_symbol<'a>(
    analysis: &'a WorkspaceAnalysis,
    request: &PositionRequest,
) -> Option<&'a IndexedSymbol> {
    analysis
        .semantic_index
        .symbol_at(&request.source, request.byte_offset)
        .or_else(|| {
            analysis
                .semantic_index
                .reference_at(&request.source, request.byte_offset)
                .and_then(|reference| reference.target_identity.as_deref())
                .and_then(|identity| {
                    analysis
                        .semantic_index
                        .documents
                        .values()
                        .flat_map(|document| &document.symbols)
                        .find(|symbol| symbol.identity == identity)
                })
        })
        .filter(|symbol| renameable_kind(symbol.value_kind, &symbol.keyword))
}

fn renameable_kind(kind: IndexedValueKind, keyword: &str) -> bool {
    matches!(
        kind,
        IndexedValueKind::Scalar
            | IndexedValueKind::Table
            | IndexedValueKind::Selection
            | IndexedValueKind::Output
    ) || matches!(
        keyword,
        "chart" | "define" | "catalog" | "schema" | "table" | "import"
    )
}

fn references_are_complete(analysis: &WorkspaceAnalysis, symbol: &IndexedSymbol) -> bool {
    let indexed = analysis
        .semantic_index
        .documents
        .values()
        .flat_map(|document| &document.references)
        .filter(|reference| reference.target_identity.as_deref() == Some(&symbol.identity))
        .all(|reference| {
            reference_name_span(analysis, reference, &symbol.name).is_some()
                || (symbol.exported
                    && symbol.parent.is_none()
                    && is_named_import_reference(analysis, symbol, reference))
        });
    indexed
        && (symbol.keyword != "output_alias" || output_column_references_are_safe(analysis, symbol))
}

fn output_column_references_are_safe(analysis: &WorkspaceAnalysis, symbol: &IndexedSymbol) -> bool {
    let Some(syntax) = analysis.syntax.get(&symbol.origin) else {
        return false;
    };
    let mut covered = vec![symbol.selection_span];
    covered.extend(
        analysis
            .semantic_index
            .documents
            .values()
            .flat_map(|document| &document.references)
            .filter(|reference| reference.target_identity.as_deref() == Some(&symbol.identity))
            .filter_map(|reference| reference_name_span(analysis, reference, &symbol.name)),
    );
    syntax.parsed.tokens.tokens().iter().all(|token| {
        let is_same_identifier = matches!(
            token.token(),
            Some(sqlparser::tokenizer::Token::Word(word)) if word.value == symbol.name
        );
        !is_same_identifier
            || covered.iter().any(|span| {
                span.source == token.span().source
                    && span.range.start <= token.span().range.start
                    && token.span().range.end <= span.range.end
            })
    })
}

fn rename_span_at_cursor(
    analysis: &WorkspaceAnalysis,
    request: &PositionRequest,
    symbol: &IndexedSymbol,
) -> Option<SourceSpan> {
    if let Some(reference) = analysis
        .semantic_index
        .reference_at(&request.source, request.byte_offset)
    {
        reference_name_span(analysis, reference, &symbol.name)
    } else {
        Some(symbol.selection_span)
    }
}

fn reference_name_span(
    analysis: &WorkspaceAnalysis,
    reference: &IndexedReference,
    old_name: &str,
) -> Option<SourceSpan> {
    let syntax = analysis.syntax.get(&reference.origin)?;
    let text = syntax.parsed.tokens.text();
    let raw = text.get(reference.span.range.as_range())?;
    let relative = if raw.starts_with('$')
        && reference
            .name
            .split('.')
            .next()
            .is_some_and(|name| name == old_name)
    {
        1..1 + old_name.len()
    } else if reference
        .name
        .rsplit('.')
        .next()
        .is_some_and(|name| name == old_name)
        && raw.ends_with(old_name)
    {
        raw.len() - old_name.len()..raw.len()
    } else if raw == old_name {
        0..raw.len()
    } else {
        return None;
    };
    Some(SourceSpan {
        source: reference.span.source,
        range: ByteSpan {
            start: reference.span.range.start + relative.start,
            end: reference.span.range.start + relative.end,
        },
    })
}

fn document_syntax<'a>(
    analysis: &'a WorkspaceAnalysis,
    request: &DocumentRequest,
    cancellation: &AnalysisCancellation,
) -> Result<&'a crate::SyntaxAnalysis, AnalysisQueryError> {
    cancellation
        .check()
        .map_err(|_| AnalysisQueryError::Cancelled)?;
    let syntax = analysis
        .syntax
        .get(&request.source)
        .ok_or(AnalysisQueryError::UnknownSource)?;
    if syntax.revision != request.source_revision {
        return Err(AnalysisQueryError::StaleRevision);
    }
    Ok(syntax)
}

fn position_syntax<'a>(
    analysis: &'a WorkspaceAnalysis,
    request: &PositionRequest,
    cancellation: &AnalysisCancellation,
) -> Result<&'a crate::SyntaxAnalysis, AnalysisQueryError> {
    document_syntax(
        analysis,
        &DocumentRequest {
            source: request.source.clone(),
            source_revision: request.source_revision.clone(),
        },
        cancellation,
    )
}

#[cfg(test)]
mod tests {
    use super::apply_line_ending;
    use crate::LineEnding;

    #[test]
    fn line_endings_are_applied_after_canonical_formatting() {
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Lf), "a\nb\n");
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Crlf), "a\r\nb\r\n");
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Cr), "a\rb\r");
    }
}
