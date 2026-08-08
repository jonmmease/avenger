//! Conservative source edits and semantic presentation.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_schema::{NativeKindNamespace, ProjectionPolicy, PropertySchema, ValueShape};
use avenger_lang_core::{
    ByteSpan, SourceFile, SourceId, SourceOrigin, SourceSpan,
    ast::{ImportClause, Name},
    module_graph::normalize_path,
    sql::{LosslessTokenKind, TokenClass, tokenize_lossless},
    syntax::{ImportClauseSyntax, format_source, parse_file},
};
use sqlparser::tokenizer::Token;

use crate::{
    AnalysisCancellation, AnalysisQueryError, CodeAction, CodeActionKind, CodeActionRequest,
    CompletionOptions, DocumentRequest, DocumentSnapshot, FormattingResult, IndexedReference,
    IndexedSymbol, IndexedValueKind, LineEnding, PinImportTarget, PositionRequest,
    PrepareRenameResult, SemanticToken, SemanticTokenKind, SemanticTokenModifiers,
    SemanticTokensResult, SourceRevision, SourceTextEdit, VersionedSourceEdits, WorkspaceAnalysis,
    WorkspaceEdit, analyze_syntax,
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
    let channel_mode_spans = channel_mode_semantic_spans(syntax);
    // Channel modes are syntax-highlighted as constants by the grammar. Keep
    // them out of the generic property tokens below without emitting an LSP
    // token that would override the editor's theme-specific constant style.
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
    for span in document
        .property_names
        .keys()
        .filter(|span| !channel_mode_spans.contains(span))
    {
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
    for spans in crate::sql_intelligence::contextual_semantic_token_spans(analysis, &request.source)
    {
        if spans.root_is_builtin {
            tokens.push(SemanticToken {
                span: spans.root,
                kind: SemanticTokenKind::Namespace,
                modifiers: SemanticTokenModifiers {
                    default_library: true,
                    ..SemanticTokenModifiers::default()
                },
            });
        }
        for span in spans.properties {
            tokens.push(SemanticToken {
                span,
                kind: SemanticTokenKind::Property,
                modifiers: SemanticTokenModifiers {
                    readonly: true,
                    ..SemanticTokenModifiers::default()
                },
            });
        }
        if let Some(span) = spans.field {
            tokens.push(SemanticToken {
                span,
                kind: SemanticTokenKind::Field,
                modifiers: SemanticTokenModifiers {
                    readonly: true,
                    ..SemanticTokenModifiers::default()
                },
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

fn channel_mode_semantic_spans(syntax: &crate::SyntaxAnalysis) -> BTreeSet<SourceSpan> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::ChannelMode { .. } => {
                Some(node.span)
            }
            _ => None,
        })
        .collect()
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
    missing_required_properties_action(analysis, request, &mut actions);
    missing_state_action_members_action(analysis, request, &mut actions);
    ambiguous_qualification_actions(analysis, request, cancellation, &mut actions);
    missing_param_action(analysis, request, &mut actions);
    channel_mode_actions(analysis, request, &mut actions);
    contextual_reference_actions(analysis, request, &mut actions);
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

fn missing_required_properties_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    const DIAGNOSTIC: &str = "AVENGER-RESOLVE-022";
    if !request
        .diagnostic_codes
        .iter()
        .any(|code| code == DIAGNOSTIC)
    {
        return;
    }
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let Some(document) = analysis.semantic_index.documents.get(&request.source) else {
        return;
    };
    let Some(symbol) = document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.native_kind.is_some() && spans_overlap(symbol.declaration_span, request.range)
        })
        .min_by_key(|symbol| symbol.declaration_span.range.len())
    else {
        return;
    };
    let Some(schema) = crate::intelligence::schema_for_symbol(
        &analysis.registry,
        symbol,
        &analysis.semantic_index,
    ) else {
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
            ) && node.span.range.start <= symbol.selection_span.range.start
                && symbol.selection_span.range.end <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())
    else {
        return;
    };
    let authored = syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name }
                if node.parent == Some(declaration.id) =>
            {
                Some(name.as_str())
            }
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut missing = schema
        .properties
        .iter()
        .filter(|(name, property)| property.required && !authored.contains(name.as_str()))
        .map(|(name, property)| (name.as_str(), &property.shape, false))
        .chain(
            schema
                .channels
                .iter()
                .filter(|(name, channel)| channel.required && !authored.contains(name.as_str()))
                .map(|(name, channel)| (name.as_str(), &channel.shape, true)),
        )
        .collect::<Vec<_>>();
    missing.sort_by_key(|(name, _, _)| *name);
    if missing.is_empty() {
        return;
    }

    let tokens = syntax.parsed.tokens.tokens();
    let Some(open) = tokens
        .iter()
        .find(|token| {
            symbol.declaration_span.range.start <= token.span().range.start
                && token.span().range.end <= symbol.declaration_span.range.end
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
            symbol.declaration_span.range.start <= token.span().range.start
                && token.span().range.end <= symbol.declaration_span.range.end
                && matches!(token.token(), Some(Token::RBrace))
        })
        .map(|token| token.span())
    else {
        return;
    };
    if open.range.end > close.range.start {
        return;
    }

    let text = syntax.parsed.tokens.text();
    let line_start = text[..symbol.declaration_span.range.start]
        .rfind(['\n', '\r'])
        .map_or(0, |position| position + 1);
    let parent_indent = text[line_start..symbol.declaration_span.range.start]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect::<String>();
    let indent = format!("{parent_indent}  ");
    let newline = if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    let members = missing
        .iter()
        .map(|(name, shape, channel)| {
            let value = required_value_placeholder(shape);
            let value = if *channel && required_channel_needs_mode(shape) {
                format!("encoded {value}")
            } else {
                value
            };
            format!("{indent}{name}: {value};")
        })
        .collect::<Vec<_>>()
        .join(newline);
    let body_span = SourceSpan {
        source: open.source,
        range: ByteSpan {
            start: open.range.end,
            end: close.range.start,
        },
    };
    let body = &text[body_span.range.as_range()];
    let (edit_span, new_text) = if body.trim().is_empty() {
        (
            body_span,
            format!("{newline}{members}{newline}{parent_indent}"),
        )
    } else {
        let leading_len = body.len() - body.trim_start_matches(char::is_whitespace).len();
        let leading = &body[..leading_len];
        if leading.contains(['\n', '\r']) {
            (
                SourceSpan::empty(open.source, open.range.end),
                format!("{newline}{members}"),
            )
        } else {
            (
                SourceSpan {
                    source: open.source,
                    range: ByteSpan {
                        start: open.range.end,
                        end: open.range.end + leading_len,
                    },
                },
                format!("{newline}{members}{newline}{indent}"),
            )
        }
    };
    let title = if missing.len() == 1 {
        let (name, _, channel) = missing[0];
        format!(
            "Add required `{name}:` {}",
            if channel { "channel" } else { "property" }
        )
    } else {
        "Add all missing required properties".to_owned()
    };
    let mut action = quick_fix(title, request, edit_span, new_text, true);
    action.diagnostic_codes = vec![DIAGNOSTIC.to_owned()];
    output.push(action);
}

fn missing_state_action_members_action(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let supported_codes = [
        "AVENGER-RESOLVE-115",
        "AVENGER-RESOLVE-122",
        "AVENGER-RESOLVE-123",
        "AVENGER-RESOLVE-137",
        "AVENGER-RESOLVE-140",
        "AVENGER-RESOLVE-159",
    ];
    let diagnostic_codes = request
        .diagnostic_codes
        .iter()
        .filter(|code| supported_codes.contains(&code.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if diagnostic_codes.is_empty() {
        return;
    }
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
                    if avenger_lang_core::ast::is_state_action_keyword(keyword)
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
    let text = syntax.parsed.tokens.text();
    let header_end = text[declaration.span.range.as_range()]
        .find('{')
        .map_or(declaration.span.range.end, |offset| {
            declaration.span.range.start + offset
        });
    let header = &text[declaration.span.range.start..header_end];
    let from_scene = header
        .split_whitespace()
        .collect::<Vec<_>>()
        .windows(2)
        .any(|words| words == ["from", "scene"]);

    let authored = syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| {
            if node.parent != Some(declaration.id) {
                return None;
            }
            match &node.kind {
                avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name }
                | avenger_lang_core::syntax::TolerantSyntaxNodeKind::Declaration {
                    keyword: name,
                    ..
                } => Some(name.as_str()),
                _ => None,
            }
        })
        .collect::<BTreeSet<_>>();

    let has_code = |code: &str| diagnostic_codes.iter().any(|value| value == code);
    let mut required = BTreeMap::<&str, &str>::new();
    if has_code("AVENGER-RESOLVE-115")
        && matches!(keyword.as_str(), "insert" | "replace" | "upsert" | "toggle")
        && !from_scene
    {
        required.insert("row", "row { }");
    }
    if has_code("AVENGER-RESOLVE-122") && keyword == "patch" {
        required.insert("key", "key { }");
        required.insert("fields", "fields { }");
    }
    if has_code("AVENGER-RESOLVE-123") && keyword == "delete" {
        required.insert("key", "key { }");
    }
    if has_code("AVENGER-RESOLVE-137")
        && matches!(keyword.as_str(), "replace" | "upsert" | "toggle")
        && !from_scene
    {
        required.insert("clause", "clause { }");
    }
    if has_code("AVENGER-RESOLVE-159") && keyword == "delete" {
        required.insert("ids", "ids: ['id'];");
    }
    if has_code("AVENGER-RESOLVE-140") && from_scene {
        required.insert("geometry", "geometry: rect(0.0, 0.0, 0.0, 0.0);");
        required.insert("policy", "policy: intersects;");
        required.insert("marks", "marks: [mark_name];");
        required.insert(
            "fields",
            "fields: [{ id: 'field'; datum: 'field'; field: \"field\"; }];",
        );
    }
    required.retain(|name, _| !authored.contains(name));
    if required.is_empty() {
        return;
    }

    let tokens = syntax.parsed.tokens.tokens();
    let Some(open) = tokens
        .iter()
        .find(|token| {
            declaration.span.range.start <= token.span().range.start
                && token.span().range.end <= declaration.span.range.end
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
            declaration.span.range.start <= token.span().range.start
                && token.span().range.end <= declaration.span.range.end
                && matches!(token.token(), Some(Token::RBrace))
        })
        .map(|token| token.span())
    else {
        return;
    };
    if open.range.end > close.range.start {
        return;
    }

    let line_start = text[..declaration.span.range.start]
        .rfind(['\n', '\r'])
        .map_or(0, |position| position + 1);
    let parent_indent = text[line_start..declaration.span.range.start]
        .chars()
        .take_while(|character| matches!(character, ' ' | '\t'))
        .collect::<String>();
    let indent = format!("{parent_indent}  ");
    let newline = if text.contains("\r\n") {
        "\r\n"
    } else if text.contains('\r') {
        "\r"
    } else {
        "\n"
    };
    let members = required
        .values()
        .map(|member| format!("{indent}{member}"))
        .collect::<Vec<_>>()
        .join(newline);
    let body_span = SourceSpan {
        source: open.source,
        range: ByteSpan {
            start: open.range.end,
            end: close.range.start,
        },
    };
    let body = &text[body_span.range.as_range()];
    let (edit_span, new_text) = if body.trim().is_empty() {
        (
            body_span,
            format!("{newline}{members}{newline}{parent_indent}"),
        )
    } else {
        (
            SourceSpan::empty(open.source, open.range.end),
            format!("{newline}{members}"),
        )
    };
    let title = if required.len() == 1 {
        format!(
            "Add required `{}` action member",
            required.keys().next().unwrap()
        )
    } else {
        "Add all missing required action members".to_owned()
    };
    let mut action = quick_fix(title, request, edit_span, new_text, true);
    action.diagnostic_codes = diagnostic_codes;
    output.push(action);
}

fn required_channel_needs_mode(shape: &ValueShape) -> bool {
    !matches!(
        shape,
        ValueShape::ChannelConfig | ValueShape::RasterDimensionChannel | ValueShape::PatternChannel
    )
}

fn required_value_placeholder(shape: &ValueShape) -> String {
    match shape {
        ValueShape::Boolean => "true".to_owned(),
        ValueShape::Integer => "0".to_owned(),
        ValueShape::Number => "0.0".to_owned(),
        ValueShape::String => "''".to_owned(),
        ValueShape::Identifier => "name".to_owned(),
        ValueShape::Atom { values } => values
            .first()
            .map_or_else(|| "value".to_owned(), |value| value.value.clone()),
        ValueShape::SqlExpression => "NULL".to_owned(),
        ValueShape::SqlProjection { policy, .. } => match policy {
            ProjectionPolicy::Named | ProjectionPolicy::Select => "NULL AS value".to_owned(),
        },
        ValueShape::SqlQuery => "SELECT NULL AS value".to_owned(),
        ValueShape::ChannelConfig => "{ }".to_owned(),
        ValueShape::ConfiguredExpression(properties) => {
            format!("NULL {}", required_object_placeholder(properties))
        }
        ValueShape::ConfiguredReference {
            namespaces,
            properties,
        } => format!(
            "{} {}",
            typed_reference_placeholder(namespaces.iter().next().copied()),
            required_object_placeholder(properties)
        ),
        ValueShape::PatternChannel => "pattern { }".to_owned(),
        ValueShape::CoordinationScope => "shared".to_owned(),
        ValueShape::FacetDataScope => "filtered".to_owned(),
        ValueShape::RasterDimension | ValueShape::RasterDimensionChannel => "dim value".to_owned(),
        ValueShape::ScalarBinding => "$value".to_owned(),
        ValueShape::TableBinding => "$table".to_owned(),
        ValueShape::SelectionBinding => "$selection".to_owned(),
        ValueShape::WidgetData => "{ values: []; }".to_owned(),
        ValueShape::StateActionBlock => "{ }".to_owned(),
        ValueShape::MarkBlock => "{ mark group { } }".to_owned(),
        ValueShape::TypedReference { namespaces } => {
            typed_reference_placeholder(namespaces.iter().next().copied())
        }
        ValueShape::Union(shapes) => shapes
            .first()
            .map_or_else(|| "NULL".to_owned(), required_value_placeholder),
        ValueShape::OneOrMany(shape) => required_value_placeholder(shape),
        ValueShape::Array(shape) => format!("[{}]", required_value_placeholder(shape)),
        ValueShape::Map(shape) => {
            format!("{{ value: {}; }}", required_value_placeholder(shape))
        }
        ValueShape::ChannelMap => "{ value: encoded NULL; }".to_owned(),
        ValueShape::Object(properties) => required_object_placeholder(properties),
        ValueShape::Any => "NULL".to_owned(),
    }
}

fn required_object_placeholder(properties: &BTreeMap<String, PropertySchema>) -> String {
    let members = properties
        .iter()
        .filter(|(_, property)| property.required)
        .map(|(name, property)| format!("{name}: {};", required_value_placeholder(&property.shape)))
        .collect::<Vec<_>>();
    if members.is_empty() {
        "{ }".to_owned()
    } else {
        format!("{{ {} }}", members.join(" "))
    }
}

fn typed_reference_placeholder(namespace: Option<NativeKindNamespace>) -> String {
    let keyword = match namespace {
        Some(NativeKindNamespace::Coordinate) => "chart",
        Some(NativeKindNamespace::Adjust) => "adjust",
        Some(NativeKindNamespace::Mark) => "mark",
        Some(NativeKindNamespace::Transform) => "transform",
        Some(NativeKindNamespace::Tool) => "tool",
        Some(NativeKindNamespace::Widget) => "widget",
        Some(NativeKindNamespace::Scale) => "scale",
        Some(NativeKindNamespace::Axis) => "axis",
        Some(NativeKindNamespace::Legend) => "legend",
        Some(NativeKindNamespace::Layout) => "layout",
        Some(NativeKindNamespace::View) => "view",
        Some(NativeKindNamespace::Resource) => "resource",
        None => "value",
    };
    if namespace.is_some() {
        format!("{keyword} name")
    } else {
        keyword.to_owned()
    }
}

fn channel_mode_actions(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    for node in syntax.parsed.nodes.iter().filter(|node| {
        matches!(
            &node.kind,
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name }
                if matches!(
                    name.as_str(),
                    "scale" | "axis" | "legend" | "domain_contribution" | "band"
                )
        ) && spans_overlap(node.span, request.range)
    }) {
        if request
            .diagnostic_codes
            .iter()
            .any(|code| code == "AVENGER-RESOLVE-198")
        {
            let mut parent = node.parent;
            while let Some(id) = parent {
                let Some(candidate) = syntax.parsed.nodes.iter().find(|node| node.id == id) else {
                    break;
                };
                if matches!(
                    candidate.kind,
                    avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { .. }
                ) && !crate::intelligence::channel_body_has_effective_encoded(syntax, candidate)
                {
                    let avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name } =
                        &node.kind
                    else {
                        unreachable!("filtered property node")
                    };
                    output.push(quick_fix(
                        format!("Remove ineffective `{name}:` channel configuration"),
                        request,
                        node.span,
                        String::new(),
                        true,
                    ));
                    break;
                }
                parent = candidate.parent;
            }
            continue;
        }
        let Some(owner) = crate::intelligence::owner_symbol(
            &analysis.semantic_index,
            &request.source,
            node.span.range.start,
        ) else {
            continue;
        };
        let Some(schema) = crate::intelligence::schema_for_symbol(
            &analysis.registry,
            owner,
            &analysis.semantic_index,
        ) else {
            continue;
        };
        let mut parent = node.parent;
        let mut channel_property = None;
        while let Some(id) = parent {
            let Some(candidate) = syntax.parsed.nodes.iter().find(|node| node.id == id) else {
                break;
            };
            if let avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name } =
                &candidate.kind
                && schema.channels.contains_key(name)
            {
                channel_property = Some(candidate);
                break;
            }
            parent = candidate.parent;
        }
        let Some(channel_property) = channel_property else {
            continue;
        };
        if crate::intelligence::channel_body_has_effective_encoded(syntax, channel_property) {
            continue;
        }
        let avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name } = &node.kind
        else {
            unreachable!("filtered property node")
        };
        output.push(quick_fix(
            format!("Remove ineffective `{name}:` channel configuration"),
            request,
            node.span,
            String::new(),
            true,
        ));
    }
    for node in syntax.parsed.nodes.iter().filter(|node| {
        matches!(
            node.kind,
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { .. }
        ) && spans_overlap(node.span, request.range)
    }) {
        let avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { name } = &node.kind
        else {
            continue;
        };
        if matches!(name.as_str(), "scaled" | "value")
            && has_property_ancestor(&syntax.parsed.nodes, node.parent)
            && let Some(span) = crate::intelligence::first_word_span(syntax, node.span, name)
        {
            let replacement = if name == "scaled" {
                "encoded"
            } else {
                "direct"
            };
            output.push(quick_fix(
                format!("Use `{replacement}:` channel branch"),
                request,
                span,
                replacement.to_owned(),
                true,
            ));
            continue;
        }

        let significant = syntax
            .parsed
            .tokens
            .tokens()
            .iter()
            .filter(|token| {
                node.span.range.start <= token.span().range.start
                    && token.span().range.end <= node.span.range.end
                    && !matches!(
                        token.kind(),
                        LosslessTokenKind::Token(
                            TokenClass::Whitespace(_) | TokenClass::Comment(_)
                        ) | LosslessTokenKind::Eof
                    )
            })
            .collect::<Vec<_>>();
        let colon = significant
            .iter()
            .position(|token| matches!(token.token(), Some(Token::Colon)));
        let first = colon.and_then(|colon| significant.get(colon + 1));
        if first.is_some_and(|token| {
            matches!(
                token.token(),
                Some(Token::Word(word))
                    if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("value")
            )
        }) && colon.is_some_and(|colon| significant.get(colon + 2).is_some())
        {
            output.push(quick_fix(
                "Use `direct` channel mode".to_owned(),
                request,
                first.expect("checked first token").span(),
                "direct".to_owned(),
                true,
            ));
            continue;
        }

        if request
            .diagnostic_codes
            .iter()
            .any(|code| code == "AVENGER-RESOLVE-195")
        {
            let Some(first) = first else {
                continue;
            };
            output.push(quick_fix(
                "Add `encoded` channel mode".to_owned(),
                request,
                SourceSpan::empty(first.span().source, first.span().range.start),
                "encoded ".to_owned(),
                true,
            ));
            continue;
        }

        let owner = crate::intelligence::owner_symbol(
            &analysis.semantic_index,
            &request.source,
            node.span.range.start,
        );
        let Some(owner) = owner else {
            continue;
        };
        let Some(channel) = crate::intelligence::schema_for_symbol(
            &analysis.registry,
            owner,
            &analysis.semantic_index,
        )
        .and_then(|schema| schema.channels.get(name)) else {
            continue;
        };
        if matches!(
            channel.shape,
            ValueShape::ChannelConfig | ValueShape::RasterDimensionChannel
        ) {
            continue;
        }
        let Some(colon) = significant
            .iter()
            .position(|token| matches!(token.token(), Some(Token::Colon)))
        else {
            continue;
        };
        let Some(first) = significant.get(colon + 1) else {
            continue;
        };
        let first_word = match first.token() {
            Some(Token::Word(word)) if word.quote_style.is_none() => {
                Some(word.value.to_ascii_lowercase())
            }
            _ => None,
        };
        match first_word.as_deref() {
            Some("encoded" | "direct" | "none" | "pattern" | "dim") => {}
            Some("value") if significant.get(colon + 2).is_some() => {
                output.push(quick_fix(
                    "Use `direct` channel mode".to_owned(),
                    request,
                    first.span(),
                    "direct".to_owned(),
                    true,
                ));
            }
            _ => output.push(quick_fix(
                "Add `encoded` channel mode".to_owned(),
                request,
                SourceSpan::empty(first.span().source, first.span().range.start),
                "encoded ".to_owned(),
                true,
            )),
        }
    }
}

fn has_property_ancestor(
    nodes: &[avenger_lang_core::syntax::TolerantSyntaxNode],
    mut parent: Option<avenger_lang_core::syntax::TolerantSyntaxNodeId>,
) -> bool {
    while let Some(id) = parent {
        let Some(node) = nodes.iter().find(|node| node.id == id) else {
            return false;
        };
        if matches!(
            node.kind,
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::Property { .. }
        ) {
            return true;
        }
        parent = node.parent;
    }
    false
}

fn contextual_reference_actions(
    analysis: &WorkspaceAnalysis,
    request: &CodeActionRequest,
    output: &mut Vec<CodeAction>,
) {
    let Some(syntax) = analysis.syntax.get(&request.source) else {
        return;
    };
    let text = syntax.parsed.tokens.text();
    for island in syntax.parsed.nodes.iter().filter(|node| {
        matches!(
            node.kind,
            avenger_lang_core::syntax::TolerantSyntaxNodeKind::SqlIsland {
                site: avenger_lang_core::syntax::SqlIslandSite::ChannelModePayload
                    | avenger_lang_core::syntax::SqlIslandSite::PropertyValue
                    | avenger_lang_core::syntax::SqlIslandSite::CursorActionRhs
                    | avenger_lang_core::syntax::SqlIslandSite::StateActionRhs
                    | avenger_lang_core::syntax::SqlIslandSite::ArrayElement
                    | avenger_lang_core::syntax::SqlIslandSite::ParamInitializer
                    | avenger_lang_core::syntax::SqlIslandSite::OutputSource,
                ..
            }
        ) && spans_overlap(node.span, request.range)
    }) {
        let source = &text[island.span.range.as_range()];
        let datum = legacy_datum_literals(source)
            .into_iter()
            .chain(unquoted_datum_fields(source))
            .map(|(start, len, field)| {
                (
                    start,
                    len,
                    format!("datum.\"{}\"", field.replace('"', "\"\"")),
                )
            });
        for (relative, replacement_len, replacement) in datum.chain(legacy_contextual_calls(source))
        {
            let span = SourceSpan {
                source: island.span.source,
                range: ByteSpan {
                    start: island.span.range.start + relative,
                    end: island.span.range.start + relative + replacement_len,
                },
            };
            if !spans_overlap(span, request.range) {
                continue;
            }
            output.push(quick_fix(
                format!("Use `{replacement}`"),
                request,
                span,
                replacement,
                true,
            ));
        }
    }
}

fn legacy_contextual_calls(source: &str) -> Vec<(usize, usize, String)> {
    let source_file = SourceFile::new(
        SourceId::new(0),
        SourceOrigin::Memory("contextual-code-action".to_owned()),
        source.to_owned(),
    );
    let tokens = tokenize_lossless(&source_file);
    let tokens = tokens
        .tokens()
        .iter()
        .filter(|token| {
            !matches!(
                token.kind(),
                LosslessTokenKind::Token(TokenClass::Whitespace(_) | TokenClass::Comment(_))
                    | LosslessTokenKind::Eof
            )
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let Some(Token::Word(function)) = token.token() else {
            continue;
        };
        if function.quote_style.is_some() {
            continue;
        }
        let name = function.value.to_ascii_lowercase();
        let replacement = match name.as_str() {
            "event_path" | "legend_value"
                if matches!(
                    tokens.get(index + 1).and_then(|token| token.token()),
                    Some(Token::LParen)
                ) && matches!(
                    tokens.get(index + 2).and_then(|token| token.token()),
                    Some(Token::RParen)
                ) =>
            {
                Some((
                    if name == "event_path" {
                        "event.path"
                    } else {
                        "event.legend.value"
                    }
                    .to_owned(),
                    index + 2,
                ))
            }
            "channel" | "event_coord" | "start_coord" | "event_domain_start"
            | "event_domain_end" | "item_channel" | "item_bbox"
                if simple_call_word(&tokens, index).is_some() =>
            {
                let argument = simple_call_word(&tokens, index).unwrap();
                let replacement = match name.as_str() {
                    "channel" => format!("channel.{argument}"),
                    "event_coord" => format!("event.coord.{argument}"),
                    "start_coord" => format!("event.start.coord.{argument}"),
                    "event_domain_start" => format!("event.domain.{argument}.start"),
                    "event_domain_end" => format!("event.domain.{argument}.end"),
                    "item_channel" => format!("item.channel.{argument}"),
                    "item_bbox" => format!("item.bbox.{argument}"),
                    _ => unreachable!(),
                };
                Some((replacement, index + 3))
            }
            "event_facet_value" => simple_call_integer(&tokens, index).map(|value| {
                (
                    format!("event.facet[{}]", value.saturating_add(1)),
                    index + 3,
                )
            }),
            "item_data" => simple_call_string(&tokens, index).map(|field| {
                (
                    format!("item.data.\"{}\"", field.replace('"', "\"\"")),
                    index + 3,
                )
            }),
            "view_x" | "view_y" => {
                simple_two_word_call(&tokens, index).and_then(|(view, field)| {
                    let axis = if name == "view_x" { "x" } else { "y" };
                    let suffix = match field.as_str() {
                        "pixels" => "pixels",
                        "domain_start" => "domain.start",
                        "domain_end" => "domain.end",
                        _ => return None,
                    };
                    Some((format!("{view}.{axis}.{suffix}"), index + 5))
                })
            }
            _ => None,
        };
        let Some((replacement, end_index)) = replacement else {
            continue;
        };
        let start = token.span().range.start;
        let end = tokens[end_index].span().range.end;
        output.push((start, end - start, replacement));
    }
    output
}

fn simple_call_word(
    tokens: &[&avenger_lang_core::sql::LosslessToken],
    index: usize,
) -> Option<String> {
    if !matches!(tokens.get(index + 1)?.token(), Some(Token::LParen))
        || !matches!(tokens.get(index + 3)?.token(), Some(Token::RParen))
    {
        return None;
    }
    let Some(Token::Word(argument)) = tokens.get(index + 2)?.token() else {
        return None;
    };
    argument
        .quote_style
        .is_none()
        .then(|| argument.value.clone())
}

fn simple_call_integer(
    tokens: &[&avenger_lang_core::sql::LosslessToken],
    index: usize,
) -> Option<u32> {
    if !matches!(tokens.get(index + 1)?.token(), Some(Token::LParen))
        || !matches!(tokens.get(index + 3)?.token(), Some(Token::RParen))
    {
        return None;
    }
    match tokens.get(index + 2)?.token() {
        Some(Token::Number(value, false)) => value.parse().ok(),
        _ => None,
    }
}

fn simple_call_string(
    tokens: &[&avenger_lang_core::sql::LosslessToken],
    index: usize,
) -> Option<String> {
    if !matches!(tokens.get(index + 1)?.token(), Some(Token::LParen))
        || !matches!(tokens.get(index + 3)?.token(), Some(Token::RParen))
    {
        return None;
    }
    match tokens.get(index + 2)?.token() {
        Some(Token::SingleQuotedString(value)) => Some(value.clone()),
        _ => None,
    }
}

fn simple_two_word_call(
    tokens: &[&avenger_lang_core::sql::LosslessToken],
    index: usize,
) -> Option<(String, String)> {
    if !matches!(tokens.get(index + 1)?.token(), Some(Token::LParen))
        || !matches!(tokens.get(index + 3)?.token(), Some(Token::Comma))
        || !matches!(tokens.get(index + 5)?.token(), Some(Token::RParen))
    {
        return None;
    }
    let Some(Token::Word(first)) = tokens.get(index + 2)?.token() else {
        return None;
    };
    let Some(Token::Word(second)) = tokens.get(index + 4)?.token() else {
        return None;
    };
    (first.quote_style.is_none() && second.quote_style.is_none())
        .then(|| (first.value.clone(), second.value.clone()))
}

fn legacy_datum_literals(source: &str) -> Vec<(usize, usize, String)> {
    let lower = source.to_ascii_lowercase();
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("datum(") {
        let start = cursor + found;
        if !is_datum_token_start(source, start) {
            cursor = start + 6;
            continue;
        }
        let mut position = start + 6;
        if source.as_bytes().get(position) != Some(&b'\'') {
            cursor = position;
            continue;
        }
        position += 1;
        let mut field = String::new();
        let mut closed = false;
        while position < source.len() {
            let Some(character) = source[position..].chars().next() else {
                break;
            };
            if character == '\'' {
                if source.as_bytes().get(position + 1) == Some(&b'\'') {
                    field.push('\'');
                    position += 2;
                    continue;
                }
                position += 1;
                closed = true;
                break;
            }
            field.push(character);
            position += character.len_utf8();
        }
        if closed && source.as_bytes().get(position) == Some(&b')') {
            output.push((start, position + 1 - start, field));
            cursor = position + 1;
        } else {
            cursor = (start + 6).min(source.len());
        }
    }
    output
}

fn unquoted_datum_fields(source: &str) -> Vec<(usize, usize, String)> {
    let lower = source.to_ascii_lowercase();
    let mut output = Vec::new();
    let mut cursor = 0;
    while let Some(found) = lower[cursor..].find("datum.") {
        let start = cursor + found;
        let field_start = start + 6;
        if !is_datum_token_start(source, start) || source.as_bytes().get(field_start) == Some(&b'"')
        {
            cursor = field_start;
            continue;
        }
        let mut end = field_start;
        while end < source.len() {
            let Some(character) = source[end..].chars().next() else {
                break;
            };
            if character == '_' || character.is_alphanumeric() {
                end += character.len_utf8();
            } else {
                break;
            }
        }
        if end > field_start {
            output.push((start, end - start, source[field_start..end].to_owned()));
        }
        cursor = end.max(field_start);
    }
    output
}

fn is_datum_token_start(source: &str, start: usize) -> bool {
    let source = SourceFile::new(
        SourceId::new(0),
        SourceOrigin::Memory("datum-code-action".to_owned()),
        source.to_owned(),
    );
    tokenize_lossless(&source).tokens().iter().any(|token| {
        token.span().range.start == start
            && matches!(
                token.token(),
                Some(Token::Word(word))
                    if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("datum")
            )
    })
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
    // Column completion is intentionally quote-gated. Ask completion about a
    // transient quoted view of the invalid bare reference so this quick fix
    // reuses the same scoped ambiguity analysis without weakening that gate.
    let mut quoted_text = text.to_owned();
    quoted_text.insert(request.range.range.end, '"');
    quoted_text.insert(request.range.range.start, '"');
    let quoted_revision = SourceRevision::from_text(&quoted_text);
    let mut syntax = analysis.syntax.clone();
    syntax.insert(
        request.source.clone(),
        analyze_syntax(&DocumentSnapshot::new(
            request.source.clone(),
            quoted_revision.clone(),
            quoted_text,
        )),
    );
    let quoted_analysis = analysis.with_syntax(analysis.generation, syntax);
    let completion = quoted_analysis.complete(
        &PositionRequest {
            source: request.source.clone(),
            byte_offset: request.range.range.end + 1,
            source_revision: quoted_revision,
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
    let sql_type = match data_type {
        "boolean" => "BOOLEAN",
        "int64" => "BIGINT",
        "float64" => "DOUBLE",
        "utf8" => "VARCHAR",
        _ => return,
    };
    let declaration = format!(
        "\n{indent}param CAST(NULL AS {sql_type}) as {};",
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
    use super::{apply_line_ending, legacy_contextual_calls};
    use crate::LineEnding;

    #[test]
    fn line_endings_are_applied_after_canonical_formatting() {
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Lf), "a\nb\n");
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Crlf), "a\r\nb\r\n");
        assert_eq!(apply_line_ending("a\nb\n", LineEnding::Cr), "a\rb\r");
    }

    #[test]
    fn contextual_call_migrations_are_token_safe_and_complete() {
        let cases = [
            ("channel(x)", "channel.x"),
            ("event_coord(x)", "event.coord.x"),
            ("start_coord(x)", "event.start.coord.x"),
            ("event_domain_start(x)", "event.domain.x.start"),
            ("event_domain_end(x)", "event.domain.x.end"),
            ("event_path()", "event.path"),
            ("event_facet_value(0)", "event.facet[1]"),
            ("legend_value()", "event.legend.value"),
            ("item_channel(fill)", "item.channel.fill"),
            ("item_data('display label')", "item.data.\"display label\""),
            ("item_bbox(top)", "item.bbox.top"),
            ("view_x(viewport, pixels)", "viewport.x.pixels"),
            ("view_y(viewport, domain_start)", "viewport.y.domain.start"),
            ("view_y(viewport, domain_end)", "viewport.y.domain.end"),
        ];
        for (authored, expected) in cases {
            let migrations = legacy_contextual_calls(authored);
            assert_eq!(
                migrations,
                vec![(0, authored.len(), expected.to_owned())],
                "{authored}"
            );
        }
        assert!(legacy_contextual_calls("event_coord(x + 1)").is_empty());
        assert!(legacy_contextual_calls("'event_coord(x)'").is_empty());
        assert!(legacy_contextual_calls("-- event_coord(x)\n1").is_empty());
    }
}
