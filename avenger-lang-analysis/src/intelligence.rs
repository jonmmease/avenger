use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    ValueShape,
};
use avenger_lang_core::{
    ByteSpan, ResolvedDeclaration, ResolvedModuleGraph, ResolvedTarget, SourceFile, SourceId,
    SourceOrigin, SourceSpan, allowed_child_declarations,
    ast::Visibility,
    sql::{LosslessTokenKind, TokenClass},
    syntax::{TolerantSyntaxNodeId, TolerantSyntaxNodeKind, parse_file},
};
use sqlparser::tokenizer::Token;

use crate::{
    AnalysisCancellation, AnalysisGeneration, CompletionItem, CompletionKind, CompletionOrigin,
    CompletionResult, CompletionTextFormat, DatasetContext, HoverResult, NavigationResult,
    NavigationTarget, PositionRequest, RootAnalysis, SymbolKind, SyntaxAnalysis,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompletionOptions {
    pub snippets: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AnalysisQueryError {
    #[error("the requested source is not part of this analysis")]
    UnknownSource,
    #[error("the requested source revision is stale")]
    StaleRevision,
    #[error("the analysis query was cancelled")]
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IndexedValueKind {
    Declaration,
    Scalar,
    Table,
    Selection,
    Mark,
    Tool,
    Widget,
    Event,
    Output,
    Field,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedSymbol {
    pub identity: String,
    pub name: String,
    pub kind: SymbolKind,
    pub value_kind: IndexedValueKind,
    pub origin: SourceOrigin,
    pub declaration_span: SourceSpan,
    pub selection_span: SourceSpan,
    pub scope_span: SourceSpan,
    pub parent: Option<usize>,
    pub keyword: String,
    pub native_kind: Option<String>,
    pub visibility: Visibility,
    pub exported: bool,
    pub detail: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedReference {
    pub name: String,
    pub origin: SourceOrigin,
    pub span: SourceSpan,
    pub target_identity: Option<String>,
    pub value_kind: IndexedValueKind,
}

#[derive(Clone, Debug, Default)]
pub struct DocumentSemanticIndex {
    pub symbols: Vec<IndexedSymbol>,
    pub references: Vec<IndexedReference>,
    pub property_names: BTreeMap<SourceSpan, String>,
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceSemanticIndex {
    pub documents: BTreeMap<SourceOrigin, DocumentSemanticIndex>,
    pub public_bindings: BTreeMap<String, IndexedBinding>,
    pub public_references: BTreeMap<(SourceOrigin, String), IndexedBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedBinding {
    pub path: String,
    pub value_kind: IndexedValueKind,
    pub detail: String,
    pub target: Option<NavigationTarget>,
}

impl WorkspaceSemanticIndex {
    pub(crate) fn build(
        syntax: &BTreeMap<SourceOrigin, SyntaxAnalysis>,
        semantic_roots: &BTreeMap<String, RootAnalysis>,
    ) -> Self {
        let mut index = Self::default();
        for (origin, analysis) in syntax {
            index
                .documents
                .insert(origin.clone(), build_document_index(origin, analysis));
        }
        for root in semantic_roots.values() {
            let Ok(analysis) = &root.result else {
                continue;
            };
            let Some(project) = analysis.resolved_project.as_deref() else {
                continue;
            };
            index.enrich_from_resolved(project);
        }
        // Install syntax-derived import bindings after semantic enrichment so
        // imported-name references point at the final declaration identities.
        // The same pass remains the complete fallback for malformed modules
        // that have no current resolved graph.
        index.install_tolerant_imports(syntax);
        index.resolve_lexical_references();
        index
    }

    fn install_tolerant_imports(&mut self, syntax: &BTreeMap<SourceOrigin, SyntaxAnalysis>) {
        for (importer, analysis) in syntax {
            for import in scan_imports(analysis) {
                let Some(imported) = resolve_local_import(importer, &import.source) else {
                    continue;
                };
                let Some(symbols) = self.documents.get(&imported).map(|document| {
                    document
                        .symbols
                        .iter()
                        .filter(|symbol| symbol.parent.is_none() && symbol.exported)
                        .cloned()
                        .collect::<Vec<_>>()
                }) else {
                    continue;
                };

                let bindings = match &import.clause {
                    ScannedImportClause::Named(specifiers) => specifiers
                        .iter()
                        .filter_map(|specifier| {
                            symbols
                                .iter()
                                .find(|symbol| symbol.name == specifier.imported)
                                .cloned()
                                .map(|symbol| {
                                    (
                                        specifier.local.clone(),
                                        symbol,
                                        (specifier.imported_span != specifier.local_span)
                                            .then_some(specifier.imported_span),
                                    )
                                })
                        })
                        .collect::<Vec<_>>(),
                    ScannedImportClause::Namespace { local, .. } => symbols
                        .iter()
                        .cloned()
                        .map(|symbol| (format!("{local}.{}", symbol.name), symbol, None))
                        .collect::<Vec<_>>(),
                };
                for (path, symbol, imported_span) in bindings {
                    let value_kind = imported_symbol_value_kind(&symbol);
                    self.public_references
                        .entry((importer.clone(), path.clone()))
                        .or_insert_with(|| IndexedBinding {
                            path: path.clone(),
                            value_kind,
                            detail: format!(
                                "imported {} definition",
                                symbol.native_kind.as_deref().unwrap_or("Avenger")
                            ),
                            target: Some(NavigationTarget {
                                origin: symbol.origin.clone(),
                                span: symbol.declaration_span,
                                selection_span: symbol.selection_span,
                            }),
                        });
                    if let Some(span) = imported_span
                        && let Some(document) = self.documents.get_mut(importer)
                    {
                        document.references.push(IndexedReference {
                            name: symbol.name.clone(),
                            origin: importer.clone(),
                            span,
                            target_identity: Some(symbol.identity.clone()),
                            value_kind,
                        });
                    }

                    let header_references = self
                        .documents
                        .get(importer)
                        .into_iter()
                        .flat_map(|document| &document.symbols)
                        .filter_map(|declaration| {
                            let header = declaration_header(
                                analysis,
                                declaration.declaration_span,
                                &declaration.keyword,
                                Some(&declaration.name),
                            );
                            (header.native_kind.as_deref() == Some(path.as_str()))
                                .then_some(header.native_kind_span)
                                .flatten()
                                .map(|span| IndexedReference {
                                    name: path.clone(),
                                    origin: importer.clone(),
                                    span,
                                    target_identity: None,
                                    value_kind,
                                })
                        })
                        .collect::<Vec<_>>();
                    if let Some(document) = self.documents.get_mut(importer) {
                        for reference in header_references {
                            if !document
                                .references
                                .iter()
                                .any(|existing| existing.span == reference.span)
                            {
                                document.references.push(reference);
                            }
                        }
                    }
                }
            }
        }
    }

    fn enrich_from_resolved(&mut self, project: &ResolvedModuleGraph) {
        let mut declarations = BTreeMap::new();
        for file in project.source_modules.values() {
            collect_resolved_declarations(&file.roots, &mut declarations);
        }

        for declaration in declarations.values() {
            let authored = project.expansion_source_map.authored_span(declaration.span);
            let Some(source) = project.sources.get(authored.source) else {
                continue;
            };
            let Some(document) = self.documents.get_mut(&source.origin) else {
                continue;
            };
            let candidate = document
                .symbols
                .iter_mut()
                .filter(|symbol| symbol.keyword == declaration.keyword)
                .filter(|symbol| ranges_overlap(symbol.declaration_span, authored))
                .min_by_key(|symbol| {
                    symbol
                        .declaration_span
                        .range
                        .start
                        .abs_diff(authored.range.start)
                });
            if let Some(symbol) = candidate {
                symbol.identity = declaration.id.to_string();
                symbol.visibility = declaration.visibility;
                symbol.native_kind.clone_from(&declaration.kind);
                if let Some(public_path) = &declaration.public_path {
                    symbol.detail = Some(format!(
                        "{} `{public_path}`{}",
                        declaration.keyword,
                        declaration
                            .kind
                            .as_ref()
                            .map(|kind| format!(" ({kind})"))
                            .unwrap_or_default()
                    ));
                }
            }
        }

        for param in project.params.values() {
            self.enrich_state_symbol(
                &param.declaration.to_string(),
                IndexedValueKind::Scalar,
                format!("param: {}", param.data_type),
            );
        }
        for store in project.stores.values() {
            let fields = store
                .fields
                .iter()
                .map(|field| format!("{}: {}", field.name, field.data_type))
                .collect::<Vec<_>>()
                .join(", ");
            self.enrich_state_symbol(
                &store.declaration.to_string(),
                IndexedValueKind::Table,
                format!("store {{ {fields} }}"),
            );
        }
        for selection in project.selections.values() {
            self.enrich_state_symbol(
                &selection.declaration.to_string(),
                IndexedValueKind::Selection,
                "selection".to_owned(),
            );
        }

        for (path, target) in &project.public_targets {
            let (value_kind, detail, _) = target_metadata(target);
            if !matches!(
                value_kind,
                IndexedValueKind::Scalar
                    | IndexedValueKind::Table
                    | IndexedValueKind::Selection
                    | IndexedValueKind::Output
            ) {
                continue;
            }
            let identity = target_declaration_identity(project, target, &declarations);
            let target = identity
                .as_deref()
                .and_then(|identity| self.navigation_for_identity(identity));
            self.public_bindings
                .entry(path.clone())
                .or_insert_with(|| IndexedBinding {
                    path: path.clone(),
                    value_kind,
                    detail,
                    target,
                });
        }
        for file in project.source_modules.values() {
            let Some(importer) = project
                .sources
                .get(file.source)
                .map(|source| source.origin.clone())
            else {
                continue;
            };
            let imported_definitions =
                file.local_bindings
                    .local
                    .iter()
                    .filter_map(|((_, local), export)| {
                        let avenger_lang_core::ModuleId::Source(module) = &export.module else {
                            return None;
                        };
                        project
                            .definitions
                            .values()
                            .find(|definition| {
                                &definition.item.module == module
                                    && definition.source_name == export.name
                            })
                            .map(|definition| (local.clone(), definition))
                    })
                    .chain(file.local_bindings.namespaces.iter().flat_map(
                        |(namespace, module)| {
                            let avenger_lang_core::ModuleId::Source(module) = module else {
                                return Vec::new().into_iter();
                            };
                            project
                                .definitions
                                .values()
                                .filter(|definition| &definition.item.module == module)
                                .map(|definition| {
                                    (
                                        format!("{namespace}.{}", definition.source_name),
                                        definition,
                                    )
                                })
                                .collect::<Vec<_>>()
                                .into_iter()
                        },
                    ));
            for (path, definition) in imported_definitions {
                let identity = definition.declaration.to_string();
                let target = self.navigation_for_identity(&identity);
                let value_kind = match definition.kind {
                    avenger_lang_core::DefinitionKind::Mark => IndexedValueKind::Mark,
                    avenger_lang_core::DefinitionKind::Tool => IndexedValueKind::Tool,
                    avenger_lang_core::DefinitionKind::Transform => IndexedValueKind::Declaration,
                };
                self.public_references
                    .entry((importer.clone(), path.clone()))
                    .or_insert_with(|| IndexedBinding {
                        path,
                        value_kind,
                        detail: format!("imported {:?} definition", definition.kind)
                            .to_ascii_lowercase(),
                        target,
                    });
            }
        }
    }

    fn enrich_state_symbol(
        &mut self,
        declaration_identity: &str,
        value_kind: IndexedValueKind,
        detail: String,
    ) {
        for document in self.documents.values_mut() {
            if let Some(symbol) = document
                .symbols
                .iter_mut()
                .find(|symbol| symbol.identity == declaration_identity)
            {
                symbol.value_kind = value_kind;
                symbol.detail = Some(detail);
                return;
            }
        }
    }

    fn navigation_for_identity(&self, identity: &str) -> Option<NavigationTarget> {
        self.documents.values().find_map(|document| {
            document
                .symbols
                .iter()
                .find(|symbol| symbol.identity == identity)
                .map(|symbol| NavigationTarget {
                    origin: symbol.origin.clone(),
                    span: symbol.declaration_span,
                    selection_span: symbol.selection_span,
                })
        })
    }

    fn resolve_lexical_references(&mut self) {
        let by_name = self
            .documents
            .values()
            .flat_map(|document| &document.symbols)
            .fold(
                BTreeMap::<String, Vec<(String, IndexedValueKind)>>::new(),
                |mut map, symbol| {
                    map.entry(symbol.name.clone())
                        .or_default()
                        .push((symbol.identity.clone(), symbol.value_kind));
                    map
                },
            );
        let identity_by_location = self
            .documents
            .values()
            .flat_map(|document| &document.symbols)
            .map(|symbol| {
                (
                    (symbol.origin.clone(), symbol.selection_span),
                    symbol.identity.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        for document in self.documents.values_mut() {
            let local_targets = document
                .references
                .iter()
                .map(|reference| {
                    let first = reference.name.split('.').next().unwrap_or(&reference.name);
                    let last = reference.name.rsplit('.').next().unwrap_or(&reference.name);
                    let mut candidates = document
                        .symbols
                        .iter()
                        .filter(|symbol| symbol.name == first || symbol.name == last)
                        .filter_map(|symbol| {
                            lexical_scope_rank(document, symbol, reference.span.range.start)
                                .map(|rank| (rank, symbol.identity.clone(), symbol.value_kind))
                        })
                        .collect::<Vec<_>>();
                    candidates.sort_by_key(|candidate| candidate.0);
                    let candidate = candidates.first()?;
                    let is_unique_at_rank =
                        candidates.get(1).is_none_or(|other| other.0 != candidate.0);
                    is_unique_at_rank.then(|| (candidate.1.clone(), candidate.2))
                })
                .collect::<Vec<_>>();
            for (reference, local_target) in document.references.iter_mut().zip(local_targets) {
                if let Some(reference_target) = self
                    .public_references
                    .get(&(reference.origin.clone(), reference.name.clone()))
                {
                    reference.value_kind = reference_target.value_kind;
                    reference.target_identity =
                        reference_target.target.as_ref().and_then(|target| {
                            identity_by_location
                                .get(&(target.origin.clone(), target.selection_span))
                                .cloned()
                        });
                    continue;
                }
                if let Some(bind) = self.public_bindings.get(&reference.name) {
                    reference.value_kind = bind.value_kind;
                    reference.target_identity = bind.target.as_ref().and_then(|target| {
                        identity_by_location
                            .get(&(target.origin.clone(), target.selection_span))
                            .cloned()
                    });
                    continue;
                }
                if let Some((identity, value_kind)) = local_target {
                    reference.target_identity = Some(identity);
                    reference.value_kind = value_kind;
                    continue;
                }
                let first = reference.name.split('.').next().unwrap_or(&reference.name);
                let last = reference.name.rsplit('.').next().unwrap_or(&reference.name);
                if let Some(candidates) = by_name.get(first).or_else(|| by_name.get(last))
                    && candidates.len() == 1
                {
                    reference.target_identity = Some(candidates[0].0.clone());
                    reference.value_kind = candidates[0].1;
                }
            }
        }
    }

    pub fn symbol_at(&self, origin: &SourceOrigin, offset: usize) -> Option<&IndexedSymbol> {
        self.documents.get(origin)?.symbols.iter().find(|symbol| {
            symbol.selection_span.range.start <= offset && offset <= symbol.selection_span.range.end
        })
    }

    pub fn reference_at(&self, origin: &SourceOrigin, offset: usize) -> Option<&IndexedReference> {
        self.documents
            .get(origin)?
            .references
            .iter()
            .find(|reference| {
                reference.span.range.start <= offset && offset <= reference.span.range.end
            })
    }

    fn symbol_by_identity(&self, identity: &str) -> Option<&IndexedSymbol> {
        self.documents
            .values()
            .flat_map(|document| &document.symbols)
            .find(|symbol| symbol.identity == identity)
    }
}

fn build_document_index(origin: &SourceOrigin, syntax: &SyntaxAnalysis) -> DocumentSemanticIndex {
    let mut output = DocumentSemanticIndex::default();
    let declaration_nodes = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. }))
        .collect::<Vec<_>>();
    for node in &declaration_nodes {
        let TolerantSyntaxNodeKind::Declaration { keyword, name } = &node.kind else {
            continue;
        };
        let mut header = declaration_header(syntax, node.span, keyword, name.as_deref());
        let parent = parent_declaration(&syntax.parsed.nodes, node.parent).and_then(|parent_id| {
            declaration_nodes
                .iter()
                .position(|candidate| candidate.id == parent_id)
        });
        if parent.is_none() {
            header.exported = syntax
                .parsed
                .module_syntax
                .items
                .iter()
                .find(|item| {
                    item.declaration_span.range.start == node.span.range.start
                        || (item.declaration_span.range.start <= node.span.range.start
                            && node.span.range.end <= item.declaration_span.range.end)
                })
                .is_some_and(|item| item.exported);
        }
        let ordinal = output.symbols.len();
        let selection_span = header.name_span.unwrap_or_else(|| {
            header
                .keyword_span
                .unwrap_or(SourceSpan::empty(node.span.source, node.span.range.start))
        });
        let name = header
            .name
            .clone()
            .or_else(|| name.clone())
            .unwrap_or_else(|| keyword.clone());
        let native_kind = header.native_kind.clone();
        output.symbols.push(IndexedSymbol {
            identity: format!(
                "syntax:{}:{}:{ordinal}",
                origin.canonical_uri(),
                node.span.range.start
            ),
            name,
            kind: symbol_kind(keyword),
            value_kind: value_kind(keyword),
            origin: origin.clone(),
            declaration_span: node.span,
            selection_span,
            scope_span: node.span,
            parent,
            keyword: keyword.clone(),
            native_kind,
            visibility: header.visibility,
            exported: header.exported,
            detail: Some(keyword.clone()),
            documentation: header.documentation.clone(),
        });
        if let (Some(path), Some(span)) = (header.native_kind, header.native_kind_span)
            && path.contains('.')
        {
            output.references.push(IndexedReference {
                name: path,
                origin: origin.clone(),
                span,
                target_identity: None,
                value_kind: IndexedValueKind::Declaration,
            });
        }
    }

    for node in &syntax.parsed.nodes {
        if let TolerantSyntaxNodeKind::Property { name } = &node.kind
            && let Some(span) = first_word_span(syntax, node.span, name)
        {
            output.property_names.insert(span, name.clone());
        }
    }

    for (alias, selection_span, declaration_span) in scan_import_aliases(syntax) {
        let ordinal = output.symbols.len();
        output.symbols.push(IndexedSymbol {
            identity: format!(
                "syntax:{}:import-alias:{}",
                origin.canonical_uri(),
                selection_span.range.start
            ),
            name: alias,
            kind: SymbolKind::Schema,
            value_kind: IndexedValueKind::Declaration,
            origin: origin.clone(),
            declaration_span,
            selection_span,
            scope_span: SourceSpan {
                source: selection_span.source,
                range: ByteSpan {
                    start: 0,
                    end: syntax.parsed.tokens.text().len(),
                },
            },
            parent: None,
            keyword: "import".to_owned(),
            native_kind: None,
            visibility: Visibility::Default,
            exported: false,
            detail: Some("import alias".to_owned()),
            documentation: None,
        });
        debug_assert_eq!(output.symbols.len(), ordinal + 1);
    }

    let occupied = output
        .symbols
        .iter()
        .map(|symbol| symbol.selection_span)
        .chain(output.property_names.keys().copied())
        .collect::<Vec<_>>();
    output
        .references
        .extend(scan_references(origin, syntax, &occupied));
    let aliases = output
        .symbols
        .iter()
        .filter(|symbol| symbol.keyword == "import")
        .map(|symbol| (symbol.name.clone(), symbol.identity.clone()))
        .collect::<BTreeMap<_, _>>();
    let alias_references = output
        .references
        .iter()
        .filter_map(|reference| {
            let alias = reference.name.split('.').next()?;
            let identity = aliases.get(alias)?;
            (reference.name.contains('.')).then(|| IndexedReference {
                name: alias.to_owned(),
                origin: origin.clone(),
                span: SourceSpan {
                    source: reference.span.source,
                    range: ByteSpan {
                        start: reference.span.range.start,
                        end: reference.span.range.start + alias.len(),
                    },
                },
                target_identity: Some(identity.clone()),
                value_kind: IndexedValueKind::Declaration,
            })
        })
        .collect::<Vec<_>>();
    output.references.extend(alias_references);
    output
}

#[derive(Default)]
struct DeclarationHeader {
    name: Option<String>,
    native_kind: Option<String>,
    native_kind_span: Option<SourceSpan>,
    keyword_span: Option<SourceSpan>,
    name_span: Option<SourceSpan>,
    visibility: Visibility,
    exported: bool,
    documentation: Option<String>,
}

fn declaration_header(
    syntax: &SyntaxAnalysis,
    span: SourceSpan,
    keyword: &str,
    fallback_name: Option<&str>,
) -> DeclarationHeader {
    let tokens = significant_tokens(syntax, Some(span));
    let mut output = DeclarationHeader::default();
    let keyword_index = tokens
        .iter()
        .position(|token| {
            token
                .word()
                .is_some_and(|word| word.eq_ignore_ascii_case(keyword))
        })
        .unwrap_or(0);
    output.keyword_span = tokens.get(keyword_index).map(|token| token.span);
    output.exported = tokens[..keyword_index]
        .iter()
        .any(|token| token.word() == Some("export"));
    if keyword_index > 0 {
        output.visibility = match tokens[keyword_index - 1].word() {
            Some("private") => Visibility::Private,
            Some("public") => Visibility::Public,
            _ => Visibility::Default,
        };
    }
    let header_end = tokens
        .iter()
        .position(|token| matches!(token.token, Some(Token::LBrace | Token::SemiColon)))
        .unwrap_or(tokens.len());
    let header = &tokens[..header_end];
    let binder = header
        .iter()
        .position(|token| {
            token
                .word()
                .is_some_and(|word| word.eq_ignore_ascii_case("as"))
        })
        .and_then(|index| header.get(index + 1));
    let recovered =
        fallback_name.and_then(|name| header.iter().rfind(|token| token.word() == Some(name)));
    let fallback = recovered.or_else(|| match keyword {
        "define" | "param" => header.get(keyword_index + 2),
        _ => None,
    });
    let named = binder.or(fallback).filter(|token| token.word().is_some());
    output.name = named
        .and_then(SigToken::word)
        .map(str::to_owned)
        .or_else(|| fallback_name.map(str::to_owned));
    output.name_span = named.map(|token| token.span);
    let kind_start = keyword_index + 1;
    let has_kind = matches!(
        keyword,
        "chart"
            | "plot"
            | "mark"
            | "transform"
            | "tool"
            | "widget"
            | "resource"
            | "catalog"
            | "schema"
            | "table"
            | "define"
    );
    if has_kind {
        let kind_end = if keyword == "define" {
            (kind_start + 1).min(header.len())
        } else {
            header
                .iter()
                .position(|token| {
                    token
                        .word()
                        .is_some_and(|word| word.eq_ignore_ascii_case("as"))
                })
                .unwrap_or(header.len())
        };
        let kind_tokens = header.get(kind_start..kind_end).unwrap_or_default();
        let mut path = String::new();
        for token in kind_tokens {
            if let Some(word) = token.word() {
                path.push_str(word);
            } else if matches!(token.token, Some(Token::Period)) {
                path.push('.');
            } else {
                break;
            }
        }
        if !path.is_empty() {
            output.native_kind = Some(path);
            output.native_kind_span =
                kind_tokens
                    .first()
                    .zip(kind_tokens.last())
                    .map(|(first, last)| SourceSpan {
                        source: first.span.source,
                        range: ByteSpan {
                            start: first.span.range.start,
                            end: last.span.range.end,
                        },
                    });
        }
    }
    output.documentation = leading_doc_comment(syntax, span.range.start);
    output
}

fn leading_doc_comment(syntax: &SyntaxAnalysis, start: usize) -> Option<String> {
    let text = syntax.parsed.tokens.text();
    let prefix = &text[..start.min(text.len())];
    let mut lines = Vec::new();
    for line in prefix.lines().rev() {
        let trimmed = line.trim();
        if let Some(doc) = trimmed.strip_prefix("-- |") {
            lines.push(doc.trim().to_owned());
        } else if trimmed.is_empty() && lines.is_empty() {
            continue;
        } else {
            break;
        }
    }
    lines.reverse();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn parent_declaration(
    nodes: &[avenger_lang_core::syntax::TolerantSyntaxNode],
    mut parent: Option<TolerantSyntaxNodeId>,
) -> Option<TolerantSyntaxNodeId> {
    while let Some(id) = parent {
        let node = &nodes[id.get() as usize];
        if matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. }) {
            return Some(id);
        }
        parent = node.parent;
    }
    None
}

fn collect_resolved_declarations<'a>(
    roots: &'a [ResolvedDeclaration],
    output: &mut BTreeMap<String, &'a ResolvedDeclaration>,
) {
    for declaration in roots {
        output.insert(declaration.id.to_string(), declaration);
        collect_resolved_declarations(&declaration.children, output);
    }
}

fn target_metadata(target: &ResolvedTarget) -> (IndexedValueKind, String, Option<String>) {
    match target {
        ResolvedTarget::Declaration(id) => (
            IndexedValueKind::Declaration,
            "declaration".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Param(id) => (
            IndexedValueKind::Scalar,
            "scalar parameter".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Store(id) => (
            IndexedValueKind::Table,
            "table-valued store".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Selection(id) => (
            IndexedValueKind::Selection,
            "selection".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Mark(id) => (
            IndexedValueKind::Mark,
            "mark".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Tool(id) => (
            IndexedValueKind::Tool,
            "tool".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Widget(id) => (
            IndexedValueKind::Widget,
            "widget".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Event(id) => (
            IndexedValueKind::Event,
            "event".to_owned(),
            Some(id.to_string()),
        ),
        ResolvedTarget::Output(output) => (
            IndexedValueKind::Output,
            format!("transform output `{}`", output.name),
            Some(output.producer.to_string()),
        ),
        ResolvedTarget::DefinitionParam {
            instance, alias, ..
        } => (
            IndexedValueKind::Scalar,
            format!("component parameter `{alias}`"),
            Some(instance.to_string()),
        ),
        ResolvedTarget::DefinitionStore {
            instance, alias, ..
        } => (
            IndexedValueKind::Table,
            format!("component store `{alias}`"),
            Some(instance.to_string()),
        ),
        ResolvedTarget::DefinitionSelection {
            instance, alias, ..
        } => (
            IndexedValueKind::Selection,
            format!("component selection `{alias}`"),
            Some(instance.to_string()),
        ),
        ResolvedTarget::DefinitionStructural {
            instance, alias, ..
        } => (
            IndexedValueKind::Declaration,
            format!("component export `{alias}`"),
            Some(instance.to_string()),
        ),
        ResolvedTarget::DefinitionSlot {
            definition, name, ..
        }
        | ResolvedTarget::DefinitionChannel {
            definition, name, ..
        } => (
            IndexedValueKind::Declaration,
            format!("definition interface `{name}`"),
            Some(definition.to_string()),
        ),
        ResolvedTarget::Part { declaration, alias } => (
            IndexedValueKind::Mark,
            format!("mark part `{alias}`"),
            Some(declaration.to_string()),
        ),
        ResolvedTarget::Reserved { namespace, path } => (
            IndexedValueKind::Declaration,
            format!("reserved {namespace} `{}`", path.join(".")),
            None,
        ),
    }
}

fn target_declaration_identity(
    project: &ResolvedModuleGraph,
    target: &ResolvedTarget,
    declarations: &BTreeMap<String, &ResolvedDeclaration>,
) -> Option<String> {
    match target {
        ResolvedTarget::Declaration(id) => Some(id.to_string()),
        ResolvedTarget::Param(id) => project
            .params
            .get(id)
            .map(|value| value.declaration.to_string()),
        ResolvedTarget::Store(id) => project
            .stores
            .get(id)
            .map(|value| value.declaration.to_string()),
        ResolvedTarget::Selection(id) => project
            .selections
            .get(id)
            .map(|value| value.declaration.to_string()),
        ResolvedTarget::Output(output) => Some(output.producer.to_string()),
        ResolvedTarget::DefinitionParam { instance, .. }
        | ResolvedTarget::DefinitionStore { instance, .. }
        | ResolvedTarget::DefinitionSelection { instance, .. }
        | ResolvedTarget::DefinitionStructural { instance, .. } => Some(instance.to_string()),
        ResolvedTarget::DefinitionSlot { definition, .. }
        | ResolvedTarget::DefinitionChannel { definition, .. } => Some(definition.to_string()),
        ResolvedTarget::Part { declaration, .. } => Some(declaration.to_string()),
        ResolvedTarget::Mark(_)
        | ResolvedTarget::Tool(_)
        | ResolvedTarget::Widget(_)
        | ResolvedTarget::Event(_) => declarations.values().find_map(|declaration| {
            (declaration.runtime_target.as_ref() == Some(target))
                .then(|| declaration.id.to_string())
        }),
        ResolvedTarget::Reserved { .. } => None,
    }
}

fn ranges_overlap(left: SourceSpan, right: SourceSpan) -> bool {
    left.range.start <= right.range.end && right.range.start <= left.range.end
}

#[derive(Clone, Copy)]
struct SigToken<'a> {
    token: Option<&'a Token>,
    raw: &'a str,
    span: SourceSpan,
}

impl SigToken<'_> {
    fn word(&self) -> Option<&str> {
        match self.token {
            Some(Token::Word(word)) => Some(word.value.as_str()),
            _ => None,
        }
    }
}

fn significant_tokens<'a>(
    syntax: &'a SyntaxAnalysis,
    within: Option<SourceSpan>,
) -> Vec<SigToken<'a>> {
    syntax
        .parsed
        .tokens
        .tokens()
        .iter()
        .filter(|token| {
            !matches!(
                token.kind(),
                LosslessTokenKind::Token(TokenClass::Whitespace(_) | TokenClass::Comment(_))
                    | LosslessTokenKind::Eof
            )
        })
        .filter(|token| {
            within.is_none_or(|span| {
                span.range.start <= token.span().range.start
                    && token.span().range.end <= span.range.end
            })
        })
        .map(|token| SigToken {
            token: token.token(),
            raw: syntax.parsed.tokens.raw(token),
            span: token.span(),
        })
        .collect()
}

#[derive(Clone, Debug)]
struct ScannedImportSpecifier {
    imported: String,
    local: String,
    imported_span: SourceSpan,
    local_span: SourceSpan,
}

#[derive(Clone, Debug)]
enum ScannedImportClause {
    Named(Vec<ScannedImportSpecifier>),
    Namespace {
        local: String,
        local_span: SourceSpan,
    },
}

#[derive(Clone, Debug)]
struct ScannedImport {
    source: String,
    declaration_span: SourceSpan,
    member_span: Option<SourceSpan>,
    clause: ScannedImportClause,
}

fn scan_imports(syntax: &SyntaxAnalysis) -> Vec<ScannedImport> {
    let tokens = significant_tokens(syntax, None);
    let mut output = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        if tokens[index].word() != Some("import") {
            index += 1;
            continue;
        }
        let end = tokens[index + 1..]
            .iter()
            .position(|token| matches!(token.token, Some(Token::SemiColon)))
            .map_or(tokens.len(), |offset| index + 1 + offset);
        let statement = &tokens[index..end];
        let source_index = statement
            .iter()
            .position(|token| token.word() == Some("from"))
            .and_then(|from| statement.get(from + 1).map(|_| from + 1));
        let Some(source_token) = source_index.and_then(|source| statement.get(source)) else {
            index = end.saturating_add(1);
            continue;
        };
        let Some(source) = unquote(source_token.raw) else {
            index = end.saturating_add(1);
            continue;
        };

        let mut member_span = None;
        let clause = if statement
            .get(1)
            .is_some_and(|token| matches!(token.token, Some(Token::LBrace)))
        {
            let member_end = statement
                .iter()
                .position(|token| matches!(token.token, Some(Token::RBrace)))
                .map_or(source_token.span.range.start, |close| {
                    statement[close].span.range.start
                });
            member_span = Some(SourceSpan {
                source: statement[1].span.source,
                range: ByteSpan {
                    start: statement[1].span.range.end,
                    end: member_end,
                },
            });
            let mut specifiers = Vec::new();
            let mut cursor = 2;
            while cursor < statement.len()
                && !matches!(statement[cursor].token, Some(Token::RBrace))
            {
                let Some(imported) = statement[cursor].word() else {
                    cursor += 1;
                    continue;
                };
                if matches!(imported, "as" | "from") {
                    cursor += 1;
                    continue;
                }
                let imported_span = statement[cursor].span;
                let mut local = imported.to_owned();
                let mut local_span = imported_span;
                if statement.get(cursor + 1).and_then(SigToken::word) == Some("as")
                    && let Some(alias) = statement.get(cursor + 2)
                    && let Some(alias_name) = alias.word()
                {
                    local = alias_name.to_owned();
                    local_span = alias.span;
                    cursor += 2;
                }
                specifiers.push(ScannedImportSpecifier {
                    imported: imported.to_owned(),
                    local,
                    imported_span,
                    local_span,
                });
                cursor += 1;
            }
            ScannedImportClause::Named(specifiers)
        } else if statement
            .get(1)
            .is_some_and(|token| matches!(token.token, Some(Token::Mul)))
        {
            let Some(alias) = statement
                .iter()
                .position(|token| token.word() == Some("as"))
                .and_then(|as_index| statement.get(as_index + 1))
            else {
                index = end.saturating_add(1);
                continue;
            };
            let Some(local) = alias.word() else {
                index = end.saturating_add(1);
                continue;
            };
            ScannedImportClause::Namespace {
                local: local.to_owned(),
                local_span: alias.span,
            }
        } else {
            index = end.saturating_add(1);
            continue;
        };

        let end_span = tokens
            .get(end)
            .map_or(source_token.span.range.end, |token| token.span.range.end);
        output.push(ScannedImport {
            source: source.to_owned(),
            declaration_span: SourceSpan {
                source: tokens[index].span.source,
                range: ByteSpan {
                    start: tokens[index].span.range.start,
                    end: end_span,
                },
            },
            member_span,
            clause,
        });
        index = end.saturating_add(1);
    }
    output
}

fn scan_import_aliases(syntax: &SyntaxAnalysis) -> Vec<(String, SourceSpan, SourceSpan)> {
    scan_imports(syntax)
        .into_iter()
        .flat_map(|import| match import.clause {
            ScannedImportClause::Named(specifiers) => specifiers
                .into_iter()
                .map(|specifier| {
                    (
                        specifier.local,
                        specifier.local_span,
                        import.declaration_span,
                    )
                })
                .collect::<Vec<_>>(),
            ScannedImportClause::Namespace { local, local_span } => {
                vec![(local, local_span, import.declaration_span)]
            }
        })
        .collect()
}

fn imported_symbol_value_kind(symbol: &IndexedSymbol) -> IndexedValueKind {
    match (symbol.keyword.as_str(), symbol.native_kind.as_deref()) {
        ("define", Some("mark")) => IndexedValueKind::Mark,
        ("define", Some("tool")) => IndexedValueKind::Tool,
        ("catalog" | "schema" | "table", _) => IndexedValueKind::Table,
        _ => symbol.value_kind,
    }
}

fn completion_kind_for_value(value_kind: IndexedValueKind) -> CompletionKind {
    match value_kind {
        IndexedValueKind::Table => CompletionKind::Table,
        IndexedValueKind::Field => CompletionKind::Field,
        IndexedValueKind::Scalar | IndexedValueKind::Selection | IndexedValueKind::Output => {
            CompletionKind::Variable
        }
        _ => CompletionKind::Declaration,
    }
}

fn unquote(value: &str) -> Option<&str> {
    let quote = value.chars().next()?;
    let inner = value.strip_prefix(quote)?.strip_suffix(quote)?;
    matches!(quote, '\'' | '"').then_some(inner)
}

fn resolve_local_import(importer: &SourceOrigin, specifier: &str) -> Option<SourceOrigin> {
    let normalized = |path: &Path| {
        avenger_lang_core::module_graph::normalize_path(path)
            .to_string_lossy()
            .into_owned()
    };
    match importer {
        SourceOrigin::File(path) => Some(SourceOrigin::File(
            avenger_lang_core::module_graph::normalize_path(&path.parent()?.join(specifier)),
        )),
        SourceOrigin::Memory(path) => Some(SourceOrigin::Memory(normalized(
            &Path::new(path).parent()?.join(specifier),
        ))),
        SourceOrigin::Std(path) => Some(SourceOrigin::Std(normalized(
            &Path::new(path).parent()?.join(specifier),
        ))),
        SourceOrigin::Http(_) => None,
    }
}

fn first_word_span(syntax: &SyntaxAnalysis, within: SourceSpan, name: &str) -> Option<SourceSpan> {
    significant_tokens(syntax, Some(within))
        .into_iter()
        .find(|token| token.word().is_some_and(|word| word == name))
        .map(|token| token.span)
}

fn scan_references(
    origin: &SourceOrigin,
    syntax: &SyntaxAnalysis,
    occupied: &[SourceSpan],
) -> Vec<IndexedReference> {
    let tokens = significant_tokens(syntax, None);
    let mut output = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        if token.word() == Some("set") {
            let mut next = index + 1;
            let Some(first) = tokens.get(next).and_then(SigToken::word) else {
                index += 1;
                continue;
            };
            let mut path = first.to_owned();
            let start = tokens[next].span.range.start;
            let mut end = tokens[next].span.range.end;
            next += 1;
            while next + 1 < tokens.len()
                && matches!(tokens[next].token, Some(Token::Period))
                && tokens[next + 1].word().is_some()
            {
                path.push('.');
                path.push_str(tokens[next + 1].word().unwrap());
                end = tokens[next + 1].span.range.end;
                next += 2;
            }
            if path != "cursor" {
                output.push(IndexedReference {
                    name: path,
                    origin: origin.clone(),
                    span: SourceSpan {
                        source: token.span.source,
                        range: ByteSpan { start, end },
                    },
                    target_identity: None,
                    value_kind: IndexedValueKind::Declaration,
                });
            }
            index = next;
            continue;
        }
        if occupied.contains(&token.span) {
            index += 1;
            continue;
        }
        if let Some(Token::Placeholder(value)) = token.token
            && let Some(base) = value.strip_prefix('$')
        {
            let mut path = base.to_owned();
            let mut end = token.span.range.end;
            let mut next = index + 1;
            while next + 1 < tokens.len()
                && matches!(tokens[next].token, Some(Token::Period))
                && tokens[next + 1].word().is_some()
            {
                path.push('.');
                path.push_str(tokens[next + 1].word().unwrap());
                end = tokens[next + 1].span.range.end;
                next += 2;
            }
            if next + 1 < tokens.len() && matches!(tokens[next].token, Some(Token::AtSign)) {
                end = tokens[next + 1].span.range.end;
                next += 2;
            }
            output.push(IndexedReference {
                name: path,
                origin: origin.clone(),
                span: SourceSpan {
                    source: token.span.source,
                    range: ByteSpan {
                        start: token.span.range.start,
                        end,
                    },
                },
                target_identity: None,
                value_kind: IndexedValueKind::Scalar,
            });
            index = next;
            continue;
        }
        if let Some(family) = token.word()
            && matches!(
                family,
                "mark" | "selection" | "tool" | "widget" | "resource"
            )
            && !declaration_header_contains(syntax, token.span)
        {
            let mut next = index + 1;
            let Some(first) = tokens.get(next).and_then(SigToken::word) else {
                index += 1;
                continue;
            };
            let mut path = first.to_owned();
            let start = tokens[next].span.range.start;
            let mut end = tokens[next].span.range.end;
            next += 1;
            while next + 1 < tokens.len()
                && matches!(tokens[next].token, Some(Token::Period))
                && tokens[next + 1].word().is_some()
            {
                path.push('.');
                path.push_str(tokens[next + 1].word().unwrap());
                end = tokens[next + 1].span.range.end;
                next += 2;
            }
            output.push(IndexedReference {
                name: path,
                origin: origin.clone(),
                span: SourceSpan {
                    source: token.span.source,
                    range: ByteSpan { start, end },
                },
                target_identity: None,
                value_kind: match family {
                    "mark" => IndexedValueKind::Mark,
                    "selection" => IndexedValueKind::Selection,
                    "tool" => IndexedValueKind::Tool,
                    "widget" => IndexedValueKind::Widget,
                    _ => IndexedValueKind::Declaration,
                },
            });
            index = next;
            continue;
        }
        index += 1;
    }
    output
}

fn declaration_header_contains(syntax: &SyntaxAnalysis, span: SourceSpan) -> bool {
    syntax.parsed.nodes.iter().any(|node| {
        if !matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. })
            || span.range.start < node.span.range.start
        {
            return false;
        }
        let header_end = significant_tokens(syntax, Some(node.span))
            .into_iter()
            .find(|token| matches!(token.token, Some(Token::LBrace | Token::SemiColon)))
            .map_or(node.span.range.end, |token| token.span.range.end);
        span.range.end <= header_end
    })
}

fn symbol_kind(keyword: &str) -> SymbolKind {
    match keyword.to_ascii_lowercase().as_str() {
        "chart" => SymbolKind::Chart,
        "define" => SymbolKind::Definition,
        "catalog" => SymbolKind::Catalog,
        "schema" => SymbolKind::Schema,
        "table" => SymbolKind::Table,
        "mark" => SymbolKind::Mark,
        "transform" => SymbolKind::Transform,
        "param" => SymbolKind::Param,
        "store" => SymbolKind::Store,
        "selection" => SymbolKind::Selection,
        "tool" => SymbolKind::Tool,
        "widget" => SymbolKind::Widget,
        "view" => SymbolKind::View,
        "on" => SymbolKind::Event,
        "field" => SymbolKind::Field,
        _ => SymbolKind::Definition,
    }
}

fn value_kind(keyword: &str) -> IndexedValueKind {
    match keyword {
        "param" => IndexedValueKind::Scalar,
        "store" | "table" => IndexedValueKind::Table,
        "selection" => IndexedValueKind::Selection,
        "mark" => IndexedValueKind::Mark,
        "tool" => IndexedValueKind::Tool,
        "widget" => IndexedValueKind::Widget,
        "on" => IndexedValueKind::Event,
        "field" => IndexedValueKind::Field,
        "output" => IndexedValueKind::Output,
        _ => IndexedValueKind::Declaration,
    }
}

pub(crate) struct QueryContext<'a> {
    generation: AnalysisGeneration,
    project_root: &'a Path,
    known_sources: &'a [SourceOrigin],
    registry: &'a NativeSchemaSnapshot,
    syntax: &'a BTreeMap<SourceOrigin, SyntaxAnalysis>,
    index: &'a WorkspaceSemanticIndex,
    semantic_roots: &'a BTreeMap<String, RootAnalysis>,
    dataset_contexts: &'a BTreeMap<SourceOrigin, Vec<DatasetContext>>,
}

impl<'a> QueryContext<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        generation: AnalysisGeneration,
        project_root: &'a Path,
        known_sources: &'a [SourceOrigin],
        registry: &'a NativeSchemaSnapshot,
        syntax: &'a BTreeMap<SourceOrigin, SyntaxAnalysis>,
        index: &'a WorkspaceSemanticIndex,
        semantic_roots: &'a BTreeMap<String, RootAnalysis>,
        dataset_contexts: &'a BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    ) -> Self {
        Self {
            generation,
            project_root,
            known_sources,
            registry,
            syntax,
            index,
            semantic_roots,
            dataset_contexts,
        }
    }

    fn syntax(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<&'a SyntaxAnalysis, AnalysisQueryError> {
        cancellation
            .check()
            .map_err(|_| AnalysisQueryError::Cancelled)?;
        let syntax = self
            .syntax
            .get(&request.source)
            .ok_or(AnalysisQueryError::UnknownSource)?;
        if syntax.revision != request.source_revision {
            return Err(AnalysisQueryError::StaleRevision);
        }
        Ok(syntax)
    }

    pub(crate) fn complete(
        &self,
        request: &PositionRequest,
        options: CompletionOptions,
        cancellation: &AnalysisCancellation,
    ) -> Result<CompletionResult, AnalysisQueryError> {
        let syntax = self.syntax(request, cancellation)?;
        let text = syntax.parsed.tokens.text();
        let cursor = request.byte_offset.min(text.len());
        let replacement = replacement_span(syntax, cursor);
        let prefix = &text[replacement.range.start..cursor];
        let mut items = Vec::new();
        let set_target = set_action_target(text, cursor);
        let set_target_kind =
            set_target.and_then(|target| self.state_target_kind(&request.source, cursor, target));
        let action_target = set_action_rhs_target(text, cursor).and(set_target_kind);

        let sql = crate::sql_intelligence::complete_sql(
            request,
            syntax,
            self.index,
            self.semantic_roots,
            self.dataset_contexts,
            cancellation,
        );
        let sql_incomplete = sql.as_ref().is_some_and(|result| result.is_incomplete);
        if let Some(sql) = sql {
            items.extend(sql.items);
            if let Some(property) = property_value_context(syntax, cursor) {
                self.complete_property_value(
                    property,
                    prefix,
                    replacement,
                    cursor,
                    &request.source,
                    &mut items,
                );
            }
            if let Some(kind) = action_target {
                complete_state_operations(kind, prefix, replacement, &mut items);
            }
            if output_alias_context(text, replacement.range.start)
                && candidate_matches("as", prefix)
            {
                items.push(item(
                    "as".to_owned(),
                    replacement,
                    "as".to_owned(),
                    CompletionKind::Keyword,
                    Some("output alias".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
        } else if let Some(import) = scan_imports(syntax).into_iter().find(|import| {
            import
                .member_span
                .is_some_and(|span| span.range.start <= cursor && cursor <= span.range.end)
        }) {
            self.complete_import_members(&request.source, &import, prefix, replacement, &mut items);
        } else if let Some(import_prefix) = import_prefix(text, cursor) {
            self.complete_imports(import_prefix, replacement, &mut items);
        } else if prefix.starts_with('$') {
            self.complete_bindings(prefix, replacement, &request.source, cursor, &mut items);
        } else if struct_field_name_context(text, cursor) {
            if candidate_matches("'<name>'", prefix) {
                let mut candidate = item(
                    "'<name>'".to_owned(),
                    replacement,
                    "'${1:name}'".to_owned(),
                    CompletionKind::Snippet,
                    Some("Arrow struct member name".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                );
                candidate.insert_text_format = CompletionTextFormat::Snippet;
                items.push(candidate);
            }
        } else if param_binder_context(text, cursor) {
            if candidate_matches("as", prefix) {
                items.push(item(
                    "as".to_owned(),
                    replacement,
                    "as".to_owned(),
                    CompletionKind::Keyword,
                    Some("parameter binder".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
        } else if field_nullable_context(text, cursor) {
            if candidate_matches("nullable", prefix) {
                items.push(item(
                    "nullable".to_owned(),
                    replacement,
                    "nullable".to_owned(),
                    CompletionKind::Keyword,
                    Some("nullable Arrow field".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
        } else if physical_type_header_context(text, cursor) {
            complete_physical_types(prefix, replacement, &mut items);
            if param_type_header_context(text, cursor) {
                for parameter_type in ["store", "selection"] {
                    if candidate_matches(parameter_type, prefix) {
                        items.push(item(
                            parameter_type.to_owned(),
                            replacement,
                            parameter_type.to_owned(),
                            CompletionKind::Type,
                            Some("parameter category".to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        ));
                    }
                }
            }
        } else if let Some(values) = fixed_header_candidates(text, cursor) {
            for value in values {
                if candidate_matches(value, prefix) {
                    items.push(item(
                        (*value).to_owned(),
                        replacement,
                        (*value).to_owned(),
                        CompletionKind::Keyword,
                        Some("declaration header".to_owned()),
                        None,
                        CompletionOrigin::Syntax,
                        false,
                        "00",
                    ));
                }
            }
        } else if let Some(values) = set_modifier_candidates(text, cursor, set_target_kind) {
            for value in values {
                if candidate_matches(value, prefix) {
                    items.push(item(
                        (*value).to_owned(),
                        replacement,
                        (*value).to_owned(),
                        CompletionKind::Keyword,
                        Some("state action modifier".to_owned()),
                        None,
                        CompletionOrigin::Syntax,
                        false,
                        "00",
                    ));
                }
            }
        } else if set_target_context(text, cursor) {
            self.complete_state_targets(prefix, replacement, &request.source, cursor, &mut items);
        } else if let Some(kind) = action_target {
            complete_state_operations(kind, prefix, replacement, &mut items);
        } else if let Some((namespace, typed)) = declaration_kind_context(text, cursor) {
            self.complete_native_kinds(
                namespace,
                typed,
                replacement,
                &request.source,
                cursor,
                &mut items,
            );
        } else if let Some(property) = property_value_context(syntax, cursor) {
            self.complete_property_value(
                property,
                prefix,
                replacement,
                cursor,
                &request.source,
                &mut items,
            );
        } else if is_property_name_context(text, cursor) {
            let nested_property = enclosing_property(syntax, cursor).is_some();
            if inside_legend_overlay_block(syntax, cursor) {
                if candidate_matches("mark", prefix) {
                    let mut candidate = item(
                        "mark".to_owned(),
                        replacement,
                        declaration_snippet("mark"),
                        CompletionKind::Keyword,
                        Some("legend overlay mark".to_owned()),
                        Some(
                            "A Cartesian mark local to this continuous-colorbar overlay."
                                .to_owned(),
                        ),
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    );
                    candidate.insert_text_format = CompletionTextFormat::Snippet;
                    items.push(candidate);
                }
            } else {
                self.complete_properties(
                    syntax,
                    &request.source,
                    cursor,
                    prefix,
                    replacement,
                    &mut items,
                );
            }
            if !nested_property && !inside_legend_overlay_block(syntax, cursor) {
                self.complete_declarations(
                    &request.source,
                    cursor,
                    prefix,
                    replacement,
                    options,
                    &mut items,
                );
            }
        } else {
            self.complete_declarations(
                &request.source,
                cursor,
                prefix,
                replacement,
                options,
                &mut items,
            );
            if prefix.starts_with('$') || prefix.is_empty() {
                self.complete_bindings(prefix, replacement, &request.source, cursor, &mut items);
            }
        }

        cancellation
            .check()
            .map_err(|_| AnalysisQueryError::Cancelled)?;
        rank_and_deduplicate(&mut items, prefix);
        Ok(CompletionResult {
            items,
            is_incomplete: sql_incomplete,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        })
    }

    fn complete_native_kinds(
        &self,
        namespace: NativeKindNamespace,
        typed: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        output: &mut Vec<CompletionItem>,
    ) {
        let overlay_mark = namespace == NativeKindNamespace::Mark
            && self
                .syntax
                .get(origin)
                .is_some_and(|syntax| inside_legend_overlay_block(syntax, cursor));
        if namespace == NativeKindNamespace::Mark && candidate_matches("group", typed) {
            output.push(item(
                "group".to_owned(),
                replacement,
                "group".to_owned(),
                CompletionKind::Declaration,
                Some("language-owned mark".to_owned()),
                Some(
                    "A logical mark group for nested marks, shared data, transforms, and views."
                        .to_owned(),
                ),
                CompletionOrigin::AuthoringSchema,
                false,
                "00",
            ));
        }
        for (key, schema) in &self.registry.entries {
            if key.namespace != namespace
                || (overlay_mark && key.coordinate.as_deref() != Some("cartesian"))
                || !candidate_matches(&key.kind, typed)
            {
                continue;
            }
            output.push(item(
                key.kind.clone(),
                replacement,
                key.kind.clone(),
                CompletionKind::Declaration,
                Some(format!("{:?}", key.namespace).to_ascii_lowercase()),
                Some(schema.docs.clone()),
                CompletionOrigin::AuthoringSchema,
                false,
                "10",
            ));
        }
        for ((reference_origin, _), reference) in &self.index.public_references {
            if reference_origin != origin {
                continue;
            }
            let Some(target) = reference.target.as_ref().and_then(|target| {
                self.index
                    .documents
                    .get(&target.origin)
                    .and_then(|document| {
                        document
                            .symbols
                            .iter()
                            .find(|symbol| symbol.selection_span == target.selection_span)
                    })
            }) else {
                continue;
            };
            let target_namespace = match target.native_kind.as_deref() {
                Some("mark") => NativeKindNamespace::Mark,
                Some("tool") => NativeKindNamespace::Tool,
                Some("transform") => NativeKindNamespace::Transform,
                _ => continue,
            };
            if target_namespace != namespace || !candidate_matches(&reference.path, typed) {
                continue;
            }
            output.push(item(
                reference.path.clone(),
                replacement,
                reference.path.clone(),
                CompletionKind::Declaration,
                Some(reference.detail.clone()),
                target.documentation.clone(),
                CompletionOrigin::LexicalScope,
                false,
                "00",
            ));
        }
    }

    fn complete_properties(
        &self,
        syntax: &SyntaxAnalysis,
        origin: &SourceOrigin,
        cursor: usize,
        prefix: &str,
        replacement: SourceSpan,
        output: &mut Vec<CompletionItem>,
    ) {
        let Some(owner) = owner_symbol(self.index, origin, cursor) else {
            return;
        };
        let authored = authored_properties(syntax, owner.scope_span, cursor);
        if owner.keyword == "slot"
            && let Some(shape) = declaration_header_role(syntax, owner)
        {
            let properties: &[(&str, &str)] = match shape.as_str() {
                "enum" => &[
                    ("default", "Optional default value."),
                    ("values", "Closed enum value inventory."),
                ],
                "function" => &[
                    ("default", "Optional default function."),
                    ("class", "Required function class."),
                ],
                "ref" => &[
                    ("default", "Optional default reference."),
                    ("kind", "Required reference kind."),
                ],
                "block" => &[
                    ("default", "Optional default block."),
                    ("exposes", "Names exposed by the block."),
                ],
                "channel" => &[("default", "Optional physical channel identity.")],
                _ => &[("default", "Optional slot default value.")],
            };
            for (name, docs) in properties {
                if !authored.contains(*name) && candidate_matches(name, prefix) {
                    output.push(item(
                        (*name).to_owned(),
                        replacement,
                        format!("{name}: "),
                        CompletionKind::Property,
                        Some("slot property".to_owned()),
                        Some((*docs).to_owned()),
                        CompletionOrigin::Syntax,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        if owner.keyword == "dimension" {
            let parent_keyword = owner
                .parent
                .and_then(|parent| self.index.documents.get(origin)?.symbols.get(parent))
                .map(|parent| parent.keyword.as_str());
            let properties: &[(&str, &str)] = match parent_keyword {
                Some("equality") => &[
                    ("field", "Selected data field."),
                    ("value", "Selected equality value."),
                ],
                Some("interval") => &[
                    ("field", "Selected data field."),
                    ("from", "Interval start expression."),
                    ("to", "Interval end expression."),
                ],
                _ => &[],
            };
            for (name, docs) in properties {
                if !authored.contains(*name) && candidate_matches(name, prefix) {
                    output.push(item(
                        (*name).to_owned(),
                        replacement,
                        format!("{name}: "),
                        CompletionKind::Property,
                        Some("predicate member property".to_owned()),
                        Some((*docs).to_owned()),
                        CompletionOrigin::Syntax,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        if owner.keyword == "variable" {
            for (name, docs) in [
                ("expr", "Expression bound to this repeat variable."),
                ("title", "Display title for this repeat variable."),
            ] {
                if !authored.contains(name) && candidate_matches(name, prefix) {
                    output.push(item(
                        name.to_owned(),
                        replacement,
                        format!("{name}: "),
                        CompletionKind::Property,
                        Some("repeat variable property".to_owned()),
                        Some(docs.to_owned()),
                        CompletionOrigin::Syntax,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        if owner.keyword == "adjust"
            && declaration_header_role(syntax, owner).as_deref() == Some("expr")
            && let Some(parent) = owner
                .parent
                .and_then(|parent| self.index.documents.get(origin)?.symbols.get(parent))
            && let Some(schema) = schema_for_symbol(self.registry, parent, self.index)
        {
            for (name, channel) in &schema.channels {
                if !authored.contains(name) && candidate_matches(name, prefix) {
                    output.push(item(
                        name.clone(),
                        replacement,
                        format!("{name}: "),
                        CompletionKind::Property,
                        Some("adjusted mark channel".to_owned()),
                        Some(channel.docs.clone()),
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        let property_path = enclosing_property_path(syntax, cursor);
        if property_path.last() == Some(&"legend") {
            let key = NativeKindKey::new(NativeKindNamespace::Legend, "standard");
            if let Some(schema) = self.registry.entries.get(&key) {
                for (name, property) in &schema.properties {
                    if authored.contains(name) || !candidate_matches(name, prefix) {
                        continue;
                    }
                    output.push(property_item(name, property, replacement));
                }
            }
            return;
        }
        if let Some(schema) = schema_for_symbol(self.registry, owner, self.index) {
            if property_path.len() == 1
                && schema.channels.contains_key(property_path[0])
                && enclosing_property(syntax, cursor).is_some()
            {
                for (name, docs) in [
                    ("scale", "Scale configuration for this channel."),
                    ("axis", "Axis configuration for this channel."),
                    ("legend", "Legend configuration for this channel."),
                ] {
                    if !authored.contains(name) && candidate_matches(name, prefix) {
                        output.push(item(
                            name.to_owned(),
                            replacement,
                            format!("{name}: {{ }}"),
                            CompletionKind::Property,
                            Some("channel configuration".to_owned()),
                            Some(docs.to_owned()),
                            CompletionOrigin::AuthoringSchema,
                            false,
                            "10",
                        ));
                    }
                }
                return;
            }
            let nested = nested_object_properties(syntax, schema, cursor);
            if nested.is_none() && enclosing_property(syntax, cursor).is_some() {
                return;
            }
            let properties = nested.unwrap_or(&schema.properties);
            for (name, property) in properties {
                if authored.contains(name) || !candidate_matches(name, prefix) {
                    continue;
                }
                output.push(property_item(name, property, replacement));
            }
            for (name, channel) in &schema.channels {
                if authored.contains(name) || !candidate_matches(name, prefix) {
                    continue;
                }
                output.push(item(
                    name.clone(),
                    replacement,
                    format!("{name}: "),
                    CompletionKind::Property,
                    Some(if channel.required {
                        "required channel".to_owned()
                    } else {
                        "channel".to_owned()
                    }),
                    Some(channel.docs.clone()),
                    CompletionOrigin::AuthoringSchema,
                    false,
                    if channel.required { "00" } else { "20" },
                ));
            }
        }
        if enclosing_property(syntax, cursor).is_some() {
            return;
        }
        for &(name, docs) in core_properties(&owner.keyword) {
            if authored.contains(name) || !candidate_matches(name, prefix) {
                continue;
            }
            output.push(item(
                name.to_owned(),
                replacement,
                format!("{name}: "),
                CompletionKind::Property,
                Some("core property".to_owned()),
                Some(docs.to_owned()),
                CompletionOrigin::Syntax,
                false,
                "10",
            ));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_property_value(
        &self,
        property_name: &str,
        prefix: &str,
        replacement: SourceSpan,
        cursor: usize,
        origin: &SourceOrigin,
        output: &mut Vec<CompletionItem>,
    ) {
        let owner = owner_symbol(self.index, origin, cursor);
        if property_name == "default"
            && let Some(owner) = owner
            && owner.keyword == "slot"
            && let Some(syntax) = self.syntax.get(origin)
            && declaration_header_role(syntax, owner).as_deref() == Some("channel")
        {
            let mut channels = BTreeSet::from([
                "x".to_owned(),
                "y".to_owned(),
                "x2".to_owned(),
                "y2".to_owned(),
                "color".to_owned(),
                "fill".to_owned(),
                "stroke".to_owned(),
                "size".to_owned(),
                "opacity".to_owned(),
                "shape".to_owned(),
                "text".to_owned(),
            ]);
            channels.extend(
                self.registry
                    .entries
                    .values()
                    .flat_map(|schema| schema.channels.keys().cloned()),
            );
            for channel in channels {
                if candidate_matches(&channel, prefix) {
                    output.push(item(
                        channel.clone(),
                        replacement,
                        channel,
                        CompletionKind::EnumValue,
                        Some("physical channel identity".to_owned()),
                        None,
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        let property = owner
            .and_then(|owner| schema_for_symbol(self.registry, owner, self.index))
            .and_then(|schema| property_schema(schema, property_name));
        if let Some(property) = property {
            complete_shape(property.shape, prefix, replacement, output);
            self.complete_reference_values(
                property.shape,
                prefix,
                replacement,
                origin,
                cursor,
                output,
            );
        }
        for value in core_property_values(property_name) {
            if candidate_matches(value, prefix) {
                output.push(item(
                    (*value).to_owned(),
                    replacement,
                    (*value).to_owned(),
                    CompletionKind::EnumValue,
                    Some(format!("{property_name} value")),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
        }
        if property_name == "type" || owner.is_some_and(|owner| owner.keyword == "field") {
            complete_physical_types(prefix, replacement, output);
        }
        if prefix.starts_with('$')
            || property.is_some_and(|property| shape_accepts_binding(property.shape))
        {
            self.complete_bindings(prefix, replacement, origin, cursor, output);
        }
    }

    fn complete_bindings(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        output: &mut Vec<CompletionItem>,
    ) {
        let typed = prefix.strip_prefix('$').unwrap_or(prefix);
        let document = self.index.documents.get(origin);
        if let Some(document) = document {
            for symbol in &document.symbols {
                if !matches!(
                    symbol.value_kind,
                    IndexedValueKind::Scalar | IndexedValueKind::Table
                ) || symbol.selection_span.range.start >= cursor
                    || !scope_visible(document, symbol, cursor)
                    || !candidate_matches(&symbol.name, typed)
                {
                    continue;
                }
                let label = format!("${}", symbol.name);
                let bucket = format!("00:{:020}", symbol.selection_span.range.start);
                output.push(item(
                    label.clone(),
                    replacement,
                    label,
                    CompletionKind::Variable,
                    symbol.detail.clone(),
                    symbol.documentation.clone(),
                    CompletionOrigin::LexicalScope,
                    false,
                    &bucket,
                ));
            }
        }
        for binding in self.index.public_bindings.values() {
            if !matches!(
                binding.value_kind,
                IndexedValueKind::Scalar | IndexedValueKind::Table
            ) || !candidate_matches(&binding.path, typed)
            {
                continue;
            }
            let label = format!("${}", binding.path);
            output.push(item(
                label.clone(),
                replacement,
                label,
                CompletionKind::Variable,
                Some(binding.detail.clone()),
                None,
                CompletionOrigin::LexicalScope,
                false,
                "10",
            ));
        }
        if let Some((base, qualifier)) = typed.split_once('@') {
            for temporal in ["start", "previous"] {
                if candidate_matches(temporal, qualifier) {
                    let label = format!("${base}@{temporal}");
                    output.push(item(
                        label.clone(),
                        replacement,
                        label,
                        CompletionKind::Variable,
                        Some(format!("{base} at {temporal}")),
                        None,
                        CompletionOrigin::LexicalScope,
                        false,
                        "00",
                    ));
                }
            }
        }
    }

    fn complete_state_targets(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        output: &mut Vec<CompletionItem>,
    ) {
        let Some(document) = self.index.documents.get(origin) else {
            return;
        };
        for symbol in &document.symbols {
            if !matches!(
                symbol.value_kind,
                IndexedValueKind::Scalar | IndexedValueKind::Table | IndexedValueKind::Selection
            ) || symbol.selection_span.range.start >= cursor
                || !scope_visible(document, symbol, cursor)
                || !candidate_matches(&symbol.name, prefix)
            {
                continue;
            }
            output.push(item(
                symbol.name.clone(),
                replacement,
                symbol.name.clone(),
                CompletionKind::Variable,
                symbol.detail.clone(),
                symbol.documentation.clone(),
                CompletionOrigin::LexicalScope,
                false,
                "00",
            ));
        }
        for binding in self.index.public_bindings.values() {
            if matches!(
                binding.value_kind,
                IndexedValueKind::Scalar | IndexedValueKind::Table | IndexedValueKind::Selection
            ) && candidate_matches(&binding.path, prefix)
            {
                output.push(item(
                    binding.path.clone(),
                    replacement,
                    binding.path.clone(),
                    CompletionKind::Variable,
                    Some(binding.detail.clone()),
                    None,
                    CompletionOrigin::LexicalScope,
                    false,
                    "10",
                ));
            }
        }
        if candidate_matches("cursor", prefix) {
            output.push(item(
                "cursor".to_owned(),
                replacement,
                "cursor".to_owned(),
                CompletionKind::Variable,
                Some("window cursor".to_owned()),
                None,
                CompletionOrigin::Syntax,
                false,
                "10",
            ));
        }
    }

    fn state_target_kind(
        &self,
        origin: &SourceOrigin,
        cursor: usize,
        target: &str,
    ) -> Option<IndexedValueKind> {
        let document = self.index.documents.get(origin)?;
        let local_name = target.rsplit('.').next().unwrap_or(target);
        let local = document
            .symbols
            .iter()
            .filter(|symbol| {
                symbol.name == local_name
                    && matches!(
                        symbol.value_kind,
                        IndexedValueKind::Scalar
                            | IndexedValueKind::Table
                            | IndexedValueKind::Selection
                    )
                    && symbol.selection_span.range.start < cursor
            })
            .filter_map(|symbol| {
                lexical_scope_rank(document, symbol, cursor).map(|rank| (rank, symbol))
            })
            .min_by_key(|(rank, symbol)| {
                (*rank, std::cmp::Reverse(symbol.selection_span.range.start))
            })
            .map(|(_, symbol)| symbol.value_kind);
        if local.is_some() && !target.contains('.') {
            return local;
        }
        self.index
            .public_bindings
            .get(target)
            .or_else(|| {
                self.index
                    .public_references
                    .get(&(origin.clone(), target.to_owned()))
            })
            .map(|binding| binding.value_kind)
            .or(local)
    }

    fn complete_reference_values(
        &self,
        shape: &ValueShape,
        prefix: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        output: &mut Vec<CompletionItem>,
    ) {
        let mut namespaces = BTreeSet::new();
        collect_reference_namespaces(shape, &mut namespaces);
        if namespaces.is_empty() {
            return;
        }
        if let Some(document) = self.index.documents.get(origin) {
            for symbol in &document.symbols {
                let Some(namespace) = namespace_for_symbol(symbol) else {
                    continue;
                };
                if !namespaces.contains(&namespace)
                    || symbol.selection_span.range.start >= cursor
                    || !scope_visible(document, symbol, cursor)
                    || !candidate_matches(&symbol.name, prefix)
                {
                    continue;
                }
                output.push(item(
                    symbol.name.clone(),
                    replacement,
                    symbol.name.clone(),
                    CompletionKind::Variable,
                    symbol.detail.clone(),
                    symbol.documentation.clone(),
                    CompletionOrigin::LexicalScope,
                    false,
                    "00",
                ));
            }
        }
        for ((reference_origin, _), reference) in &self.index.public_references {
            if reference_origin != origin {
                continue;
            }
            let allowed = match reference.value_kind {
                IndexedValueKind::Mark => namespaces.contains(&NativeKindNamespace::Mark),
                IndexedValueKind::Tool => namespaces.contains(&NativeKindNamespace::Tool),
                IndexedValueKind::Widget => namespaces.contains(&NativeKindNamespace::Widget),
                _ => false,
            };
            if allowed && candidate_matches(&reference.path, prefix) {
                output.push(item(
                    reference.path.clone(),
                    replacement,
                    reference.path.clone(),
                    CompletionKind::Variable,
                    Some(reference.detail.clone()),
                    None,
                    CompletionOrigin::LexicalScope,
                    false,
                    "10",
                ));
            }
        }
    }

    fn complete_declarations(
        &self,
        origin: &SourceOrigin,
        cursor: usize,
        prefix: &str,
        replacement: SourceSpan,
        options: CompletionOptions,
        output: &mut Vec<CompletionItem>,
    ) {
        let parent = owner_symbol(self.index, origin, cursor);
        let keywords: Vec<&str> = if let Some(parent) = parent {
            allowed_child_declarations(&parent.keyword).collect()
        } else {
            vec![
                "import", "export", "chart", "define", "catalog", "schema", "table",
            ]
        };
        for keyword in keywords {
            if keyword == "mark" && candidate_matches("mark group", prefix) {
                let (insert_text, format) = if options.snippets {
                    (
                        "mark group as ${1:name} {\n  $0\n}".to_owned(),
                        CompletionTextFormat::Snippet,
                    )
                } else {
                    ("mark group".to_owned(), CompletionTextFormat::PlainText)
                };
                let mut candidate = item(
                    "mark group".to_owned(),
                    replacement,
                    insert_text,
                    CompletionKind::Keyword,
                    Some("logical mark group".to_owned()),
                    Some(
                        "A language-owned group for nested marks, shared data, transforms, and views."
                            .to_owned(),
                    ),
                    CompletionOrigin::AuthoringSchema,
                    false,
                    "10",
                );
                candidate.insert_text_format = format;
                output.push(candidate);
            }
            let Some(label) = source_declaration_label(keyword) else {
                continue;
            };
            if !candidate_matches(label, prefix) {
                continue;
            }
            let (insert_text, format) = if options.snippets {
                (declaration_snippet(label), CompletionTextFormat::Snippet)
            } else {
                (label.to_owned(), CompletionTextFormat::PlainText)
            };
            let mut candidate = item(
                label.to_owned(),
                replacement,
                insert_text,
                CompletionKind::Keyword,
                Some("declaration".to_owned()),
                None,
                CompletionOrigin::Syntax,
                false,
                "20",
            );
            candidate.insert_text_format = format;
            output.push(candidate);
        }
    }

    fn complete_imports(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        output: &mut Vec<CompletionItem>,
    ) {
        for origin in self.known_sources {
            let SourceOrigin::File(path) = origin else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(self.project_root) else {
                continue;
            };
            let path = relative.to_string_lossy().replace('\\', "/");
            if candidate_matches(&path, prefix) {
                output.push(item(
                    path.clone(),
                    replacement,
                    path,
                    CompletionKind::Module,
                    Some("local Avenger source".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "10",
                ));
            }
        }
        for standard in ["std:marks/error_bar"] {
            if candidate_matches(standard, prefix) {
                output.push(item(
                    standard.to_owned(),
                    replacement,
                    standard.to_owned(),
                    CompletionKind::Module,
                    Some("standard library".to_owned()),
                    None,
                    CompletionOrigin::Syntax,
                    false,
                    "20",
                ));
            }
        }
        for module in self.registry.modules.keys() {
            let module = module.as_str();
            if candidate_matches(module, prefix) {
                output.push(item(
                    module.to_owned(),
                    replacement,
                    module.to_owned(),
                    CompletionKind::Module,
                    Some("host-provided native module".to_owned()),
                    None,
                    CompletionOrigin::AuthoringSchema,
                    false,
                    "30",
                ));
            }
        }
    }

    fn complete_import_members(
        &self,
        importer: &SourceOrigin,
        import: &ScannedImport,
        prefix: &str,
        replacement: SourceSpan,
        output: &mut Vec<CompletionItem>,
    ) {
        let already_imported = match &import.clause {
            ScannedImportClause::Named(specifiers) => specifiers
                .iter()
                .map(|specifier| specifier.imported.as_str())
                .collect::<BTreeSet<_>>(),
            ScannedImportClause::Namespace { .. } => return,
        };
        if let Some(imported) = resolve_local_import(importer, &import.source)
            && let Some(document) = self.index.documents.get(&imported)
        {
            for symbol in document
                .symbols
                .iter()
                .filter(|symbol| symbol.parent.is_none() && symbol.exported)
                .filter(|symbol| !already_imported.contains(symbol.name.as_str()))
                .filter(|symbol| candidate_matches(&symbol.name, prefix))
            {
                output.push(item(
                    symbol.name.clone(),
                    replacement,
                    symbol.name.clone(),
                    completion_kind_for_value(imported_symbol_value_kind(symbol)),
                    Some(format!("exported {}", symbol.keyword)),
                    symbol.documentation.clone(),
                    CompletionOrigin::LexicalScope,
                    false,
                    "10",
                ));
            }
            return;
        }

        let Ok(module_id) = avenger_chart_schema::NativeModuleId::new(import.source.clone()) else {
            return;
        };
        let Some(module) = self.registry.modules.get(&module_id) else {
            return;
        };
        for (name, export) in &module.exports {
            if already_imported.contains(name.as_str()) || !candidate_matches(name, prefix) {
                continue;
            }
            let value_kind = match export.category {
                NativeKindNamespace::Mark => IndexedValueKind::Mark,
                NativeKindNamespace::Tool => IndexedValueKind::Tool,
                _ => IndexedValueKind::Declaration,
            };
            output.push(item(
                name.clone(),
                replacement,
                name.clone(),
                completion_kind_for_value(value_kind),
                Some(format!("native {:?} export", export.category).to_ascii_lowercase()),
                Some(export.docs.clone()),
                CompletionOrigin::AuthoringSchema,
                false,
                "20",
            ));
        }
    }

    pub(crate) fn hover(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<HoverResult>, AnalysisQueryError> {
        let syntax = self.syntax(request, cancellation)?;
        let symbol = self.index.symbol_at(&request.source, request.byte_offset);
        let reference = self
            .index
            .reference_at(&request.source, request.byte_offset);
        let property = self
            .index
            .documents
            .get(&request.source)
            .and_then(|document| {
                document.property_names.iter().find(|(span, _)| {
                    span.range.start <= request.byte_offset && request.byte_offset <= span.range.end
                })
            });
        let (span, markdown) = if let Some(symbol) = symbol {
            let mut markdown = format!("```avenger\n{} {}\n```", symbol.keyword, symbol.name);
            if let Some(detail) = &symbol.detail {
                markdown.push_str(&format!("\n\n{detail}"));
            }
            if let Some(docs) = &symbol.documentation {
                markdown.push_str(&format!("\n\n{docs}"));
            }
            markdown.push_str(&format!(
                "\n\n- Visibility: `{}`\n- Source: `{}`",
                match symbol.visibility {
                    Visibility::Default => "default",
                    Visibility::Private => "private",
                    Visibility::Public => "public",
                },
                symbol.origin.canonical_uri()
            ));
            if let Some(kind) = &symbol.native_kind {
                markdown.push_str(&format!("\n- Resolved kind: `{kind}`"));
            }
            (symbol.selection_span, markdown)
        } else if let Some(reference) = reference {
            let target = reference
                .target_identity
                .as_deref()
                .and_then(|identity| self.index.symbol_by_identity(identity));
            let sigil = matches!(
                reference.value_kind,
                IndexedValueKind::Scalar
                    | IndexedValueKind::Table
                    | IndexedValueKind::Selection
                    | IndexedValueKind::Output
            )
            .then_some("$")
            .unwrap_or("");
            let mut markdown = format!("```avenger\n{sigil}{}\n```", reference.name);
            if let Some(target) = target {
                if let Some(detail) = &target.detail {
                    markdown.push_str(&format!("\n\n{detail}"));
                }
                if let Some(docs) = &target.documentation {
                    markdown.push_str(&format!("\n\n{docs}"));
                }
                markdown.push_str(&format!(
                    "\n\n- Visibility: `{}`\n- Source: `{}`",
                    match target.visibility {
                        Visibility::Default => "default",
                        Visibility::Private => "private",
                        Visibility::Public => "public",
                    },
                    target.origin.canonical_uri()
                ));
                if let Some(kind) = &target.native_kind {
                    markdown.push_str(&format!("\n- Resolved kind: `{kind}`"));
                }
            } else {
                markdown.push_str(&format!(
                    "\n\n{}\n\nSource: `{}`",
                    format!("{:?}", reference.value_kind).to_ascii_lowercase(),
                    reference.origin.canonical_uri()
                ));
            }
            (reference.span, markdown)
        } else if let Some((span, name)) = property {
            let owner = owner_symbol(self.index, &request.source, request.byte_offset);
            let schema =
                owner.and_then(|owner| schema_for_symbol(self.registry, owner, self.index));
            let property = schema.and_then(|schema| property_schema(schema, name));
            let docs = property
                .map(|property| property.docs)
                .unwrap_or("Avenger property");
            (*span, format!("`{name}`\n\n{docs}"))
        } else if let Some(span) = physical_type_spans(syntax).into_iter().find(|span| {
            span.range.start <= request.byte_offset && request.byte_offset <= span.range.end
        }) {
            let spelling = &syntax.parsed.tokens.text()[span.range.as_range()];
            let docs = if avenger_lang_core::PhysicalType::CONSTRUCTORS.contains(&spelling) {
                "Canonical physical Apache Arrow type used for state and schema validation."
            } else {
                "Canonical Avenger declaration type or role."
            };
            (span, format!("`{spelling}`\n\n{docs}"))
        } else {
            return Ok(None);
        };
        Ok(Some(HoverResult {
            span,
            markdown,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        }))
    }

    pub(crate) fn definition(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<NavigationResult, AnalysisQueryError> {
        self.syntax(request, cancellation)?;
        let identity = self
            .index
            .symbol_at(&request.source, request.byte_offset)
            .map(|symbol| symbol.identity.as_str())
            .or_else(|| {
                self.index
                    .reference_at(&request.source, request.byte_offset)
                    .and_then(|reference| reference.target_identity.as_deref())
            });
        let targets = identity
            .and_then(|identity| self.index.symbol_by_identity(identity))
            .map(|symbol| {
                vec![NavigationTarget {
                    origin: symbol.origin.clone(),
                    span: symbol.declaration_span,
                    selection_span: symbol.selection_span,
                }]
            })
            .unwrap_or_default();
        Ok(NavigationResult {
            targets,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        })
    }

    pub(crate) fn references(
        &self,
        request: &PositionRequest,
        include_declaration: bool,
        cancellation: &AnalysisCancellation,
    ) -> Result<NavigationResult, AnalysisQueryError> {
        self.syntax(request, cancellation)?;
        let identity = self
            .index
            .symbol_at(&request.source, request.byte_offset)
            .map(|symbol| symbol.identity.clone())
            .or_else(|| {
                self.index
                    .reference_at(&request.source, request.byte_offset)
                    .and_then(|reference| reference.target_identity.clone())
            });
        let Some(identity) = identity else {
            return Ok(NavigationResult {
                targets: Vec::new(),
                generation: self.generation,
                source_revision: request.source_revision.clone(),
            });
        };
        let mut targets = Vec::new();
        if include_declaration && let Some(symbol) = self.index.symbol_by_identity(&identity) {
            targets.push(NavigationTarget {
                origin: symbol.origin.clone(),
                span: symbol.selection_span,
                selection_span: symbol.selection_span,
            });
        }
        for document in self.index.documents.values() {
            targets.extend(
                document
                    .references
                    .iter()
                    .filter(|reference| reference.target_identity.as_deref() == Some(&identity))
                    .map(|reference| NavigationTarget {
                        origin: reference.origin.clone(),
                        span: reference.span,
                        selection_span: reference.span,
                    }),
            );
        }
        targets.sort_by_key(|target| {
            (
                target.origin.canonical_uri(),
                target.selection_span.range.start,
            )
        });
        targets.dedup();
        Ok(NavigationResult {
            targets,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        })
    }
}

pub(crate) fn physical_type_spans(syntax: &SyntaxAnalysis) -> Vec<SourceSpan> {
    let mut output = Vec::new();
    for node in &syntax.parsed.nodes {
        let TolerantSyntaxNodeKind::Declaration { keyword, name } = &node.kind else {
            continue;
        };
        let tokens = significant_tokens(syntax, Some(node.span));
        if matches!(keyword.as_str(), "store" | "selection") {
            if let Some(token) = tokens
                .iter()
                .find(|token| token.word() == Some(keyword.as_str()))
            {
                output.push(token.span);
            }
            continue;
        }
        if matches!(keyword.as_str(), "slot" | "variable" | "adjust") {
            if let Some(index) = tokens
                .iter()
                .position(|token| token.word() == Some(keyword.as_str()))
                && let Some(token) = tokens.get(index + 1)
                && token.word().is_some()
            {
                output.push(token.span);
            }
            continue;
        }
        if !matches!(keyword.as_str(), "param" | "field") {
            continue;
        }
        let Some(keyword_index) = tokens
            .iter()
            .position(|token| token.word() == Some(keyword.as_str()))
        else {
            continue;
        };
        let type_end = if keyword == "param" {
            tokens.iter().position(|token| token.word() == Some("as"))
        } else {
            name.as_deref()
                .and_then(|name| tokens.iter().rposition(|token| token.word() == Some(name)))
        };
        let Some(type_end) = type_end else {
            continue;
        };
        output.extend(
            tokens[keyword_index + 1..type_end]
                .iter()
                .filter(|token| {
                    token.word().is_some_and(|word| {
                        avenger_lang_core::PhysicalType::CONSTRUCTORS.contains(&word)
                    })
                })
                .map(|token| token.span),
        );
    }
    output
}

pub(crate) fn owner_symbol<'a>(
    index: &'a WorkspaceSemanticIndex,
    origin: &SourceOrigin,
    cursor: usize,
) -> Option<&'a IndexedSymbol> {
    index
        .documents
        .get(origin)?
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.scope_span.range.start <= cursor && cursor <= symbol.scope_span.range.end
        })
        .min_by_key(|symbol| symbol.scope_span.range.len())
}

fn scope_visible(
    document: &DocumentSemanticIndex,
    candidate: &IndexedSymbol,
    cursor: usize,
) -> bool {
    lexical_scope_rank(document, candidate, cursor).is_some()
}

fn lexical_scope_rank(
    document: &DocumentSemanticIndex,
    candidate: &IndexedSymbol,
    cursor: usize,
) -> Option<usize> {
    let owner = document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.scope_span.range.start <= cursor && cursor <= symbol.scope_span.range.end
        })
        .min_by_key(|symbol| symbol.scope_span.range.len());
    let Some(owner) = owner else {
        return candidate.parent.is_none().then_some(usize::MAX);
    };
    let mut current = Some(owner);
    let mut rank = 0;
    while let Some(symbol) = current {
        let ordinal = document
            .symbols
            .iter()
            .position(|item| item.identity == symbol.identity);
        if candidate.parent == ordinal || candidate.identity == symbol.identity {
            return Some(rank);
        }
        current = symbol
            .parent
            .and_then(|parent| document.symbols.get(parent));
        rank += 1;
    }
    candidate.parent.is_none().then_some(rank)
}

pub(crate) fn schema_for_symbol<'a>(
    registry: &'a NativeSchemaSnapshot,
    symbol: &IndexedSymbol,
    index: &WorkspaceSemanticIndex,
) -> Option<&'a KindSchema> {
    let namespace = namespace_for_keyword(&symbol.keyword)?;
    let kind = symbol.native_kind.as_ref()?;
    let coordinate = if namespace == NativeKindNamespace::Mark {
        chart_coordinate(index, symbol)
    } else {
        None
    };
    registry
        .entries
        .get(&NativeKindKey {
            namespace,
            kind: kind.clone(),
            coordinate: coordinate.clone(),
        })
        .or_else(|| {
            registry.entries.iter().find_map(|(key, schema)| {
                (key.namespace == namespace
                    && key.kind == *kind
                    && (coordinate.is_none() || key.coordinate == coordinate))
                    .then_some(schema)
            })
        })
}

fn chart_coordinate(index: &WorkspaceSemanticIndex, symbol: &IndexedSymbol) -> Option<String> {
    let document = index.documents.get(&symbol.origin)?;
    let mut parent = symbol.parent;
    while let Some(ordinal) = parent {
        let symbol = document.symbols.get(ordinal)?;
        if symbol.keyword == "chart" || symbol.keyword == "plot" {
            return symbol.native_kind.clone();
        }
        parent = symbol.parent;
    }
    None
}

fn namespace_for_keyword(keyword: &str) -> Option<NativeKindNamespace> {
    match keyword {
        "chart" | "plot" => Some(NativeKindNamespace::Coordinate),
        "mark" => Some(NativeKindNamespace::Mark),
        "transform" => Some(NativeKindNamespace::Transform),
        "tool" => Some(NativeKindNamespace::Tool),
        "widget" => Some(NativeKindNamespace::Widget),
        "resource" => Some(NativeKindNamespace::Resource),
        "scale" => Some(NativeKindNamespace::Scale),
        "axis" => Some(NativeKindNamespace::Axis),
        "legend" => Some(NativeKindNamespace::Legend),
        "view" => Some(NativeKindNamespace::View),
        _ => None,
    }
}

fn namespace_for_symbol(symbol: &IndexedSymbol) -> Option<NativeKindNamespace> {
    match symbol.value_kind {
        IndexedValueKind::Mark => Some(NativeKindNamespace::Mark),
        IndexedValueKind::Tool => Some(NativeKindNamespace::Tool),
        IndexedValueKind::Widget => Some(NativeKindNamespace::Widget),
        _ => namespace_for_keyword(&symbol.keyword),
    }
}

fn collect_reference_namespaces(shape: &ValueShape, output: &mut BTreeSet<NativeKindNamespace>) {
    match shape {
        ValueShape::TypedReference { namespaces }
        | ValueShape::ConfiguredReference { namespaces, .. } => {
            output.extend(namespaces.iter().copied());
        }
        ValueShape::Union(shapes) => {
            for shape in shapes {
                collect_reference_namespaces(shape, output);
            }
        }
        ValueShape::OneOrMany(shape) | ValueShape::Array(shape) => {
            collect_reference_namespaces(shape, output);
        }
        _ => {}
    }
}

fn declaration_kind_context(text: &str, cursor: usize) -> Option<(NativeKindNamespace, &str)> {
    let prefix = &text[..cursor.min(text.len())];
    let start = prefix
        .rfind(['{', '}', ';', '\n'])
        .map_or(0, |position| position + 1);
    let fragment = prefix[start..].trim_start();
    let mut words = fragment.split_whitespace();
    let keyword = words.next()?;
    let namespace = namespace_for_keyword(keyword)?;
    let typed = words.next().unwrap_or("");
    if words.next().is_some() {
        return None;
    }
    Some((namespace, typed))
}

fn statement_fragment(text: &str, cursor: usize) -> &str {
    let prefix = &text[..cursor.min(text.len())];
    let start = prefix
        .rfind(['{', '}', ';', '\n'])
        .map_or(0, |position| position + 1);
    prefix[start..].trim_start()
}

fn param_type_header_context(text: &str, cursor: usize) -> bool {
    let fragment = statement_fragment(text, cursor);
    fragment.strip_prefix("param").is_some_and(|rest| {
        rest.chars().next().is_some_and(char::is_whitespace)
            && split_header_type(rest.trim_start()).is_none()
            && !struct_field_argument_is_name(fragment)
    })
}

fn physical_type_header_context(text: &str, cursor: usize) -> bool {
    if param_type_header_context(text, cursor) {
        return true;
    }
    let fragment = statement_fragment(text, cursor);
    fragment.strip_prefix("field").is_some_and(|rest| {
        rest.chars().next().is_some_and(char::is_whitespace)
            && !rest.contains(';')
            && split_header_type(rest.trim_start()).is_none()
            && !struct_field_argument_is_name(fragment)
    })
}

fn set_target_context(text: &str, cursor: usize) -> bool {
    let fragment = statement_fragment(text, cursor);
    fragment.strip_prefix("set").is_some_and(|rest| {
        rest.chars().next().is_some_and(char::is_whitespace)
            && !rest.contains('=')
            && rest.split_whitespace().count() <= 1
    })
}

fn set_action_target(text: &str, cursor: usize) -> Option<&str> {
    let fragment = statement_fragment(text, cursor);
    let rest = fragment.strip_prefix("set")?;
    rest.chars()
        .next()
        .is_some_and(char::is_whitespace)
        .then_some(())?;
    rest.trim_start()
        .split(|character: char| character.is_whitespace() || character == '=')
        .next()
        .filter(|target| !target.is_empty())
}

fn set_action_rhs_target(text: &str, cursor: usize) -> Option<&str> {
    let fragment = statement_fragment(text, cursor);
    fragment
        .contains('=')
        .then(|| set_action_target(text, cursor))?
}

fn set_modifier_candidates(
    text: &str,
    cursor: usize,
    target_kind: Option<IndexedValueKind>,
) -> Option<&'static [&'static str]> {
    let fragment = statement_fragment(text, cursor);
    let rest = fragment.strip_prefix("set")?.trim_start();
    if rest.contains('=') {
        return None;
    }
    let mut words = rest.split_whitespace();
    words.next()?;
    let tail = words.collect::<Vec<_>>();
    if tail.is_empty() {
        if !fragment
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
        {
            return None;
        }
        return Some(if target_kind == Some(IndexedValueKind::Selection) {
            &["at"]
        } else {
            &["at", "replacing"]
        });
    }
    match tail.as_slice() {
        [partial] if "at".starts_with(*partial) => Some(&["at"]),
        ["at"]
            if fragment
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace) =>
        {
            Some(&["current", "start"])
        }
        ["at", partial] if "current".starts_with(*partial) || "start".starts_with(*partial) => {
            Some(&["current", "start"])
        }
        [partial]
            if target_kind != Some(IndexedValueKind::Selection)
                && "replacing".starts_with(*partial) =>
        {
            Some(&["replacing"])
        }
        ["replacing"]
            if fragment
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace) =>
        {
            Some(&["scopes"])
        }
        ["replacing", partial] if "scopes".starts_with(*partial) => Some(&["scopes"]),
        _ => None,
    }
}

fn complete_state_operations(
    kind: IndexedValueKind,
    prefix: &str,
    replacement: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    let operations: &[&str] = match kind {
        IndexedValueKind::Table => &[
            "clear",
            "insert_rows",
            "replace_rows",
            "upsert_rows",
            "update_by_key",
            "delete_by_key",
            "toggle_rows",
        ],
        IndexedValueKind::Selection => &[
            "clear",
            "clear_in_scope",
            "replace_all_clauses",
            "replace_clauses_in_scope",
            "upsert_clauses",
            "toggle_clauses",
            "delete_clauses",
            "delete_clauses_in_scope",
            "replace_all_from_scene_query",
            "replace_from_scene_query_in_scope",
            "upsert_from_scene_query",
            "toggle_from_scene_query",
        ],
        _ => &[],
    };
    for operation in operations {
        if candidate_matches(operation, prefix) {
            output.push(item(
                (*operation).to_owned(),
                replacement,
                (*operation).to_owned(),
                CompletionKind::Function,
                Some(
                    match kind {
                        IndexedValueKind::Table => "store update operation",
                        IndexedValueKind::Selection => "selection update operation",
                        _ => "state update operation",
                    }
                    .to_owned(),
                ),
                None,
                CompletionOrigin::Syntax,
                false,
                "00",
            ));
        }
    }
}

fn param_binder_context(text: &str, cursor: usize) -> bool {
    let fragment = statement_fragment(text, cursor);
    let Some(data_type) = fragment.strip_prefix("param") else {
        return false;
    };
    let Some((data_type, tail)) = split_header_type(data_type.trim_start()) else {
        return false;
    };
    if tail == "as"
        && fragment
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace)
    {
        return false;
    }
    if tail.split_whitespace().count() > 1
        || tail
            .split_whitespace()
            .next()
            .is_some_and(|word| word != "as" && !"as".starts_with(word))
    {
        return false;
    }
    if matches!(data_type, "store" | "selection") {
        return true;
    }
    let probe = SourceFile::new(
        SourceId::new(u32::MAX),
        SourceOrigin::Memory("completion-type-probe.avenger".to_owned()),
        format!("avenger 1; chart cartesian {{ param {data_type} as value {{ value: NULL; }} }}"),
    );
    parse_file(&probe).is_ok()
}

fn field_nullable_context(text: &str, cursor: usize) -> bool {
    let fragment = statement_fragment(text, cursor);
    let Some(rest) = fragment.strip_prefix("field") else {
        return false;
    };
    let Some((data_type, tail)) = split_header_type(rest.trim_start()) else {
        return false;
    };
    let words = tail.split_whitespace().collect::<Vec<_>>();
    if words.is_empty()
        || words.len() > 2
        || words
            .get(1)
            .is_some_and(|word| !"nullable".starts_with(*word))
        || (words.len() == 1
            && !fragment
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace))
    {
        return false;
    }
    let probe = SourceFile::new(
        SourceId::new(u32::MAX),
        SourceOrigin::Memory("completion-field-type-probe.avenger".to_owned()),
        format!(
            "avenger 1; chart cartesian {{ param store as rows {{ field {data_type} value; }} }}"
        ),
    );
    parse_file(&probe).is_ok()
}

fn split_header_type(value: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            character if character.is_whitespace() && depth == 0 => {
                let remainder = &value[index..];
                if remainder.trim_start().starts_with('(') {
                    continue;
                }
                return Some((value[..index].trim_end(), remainder.trim_start()));
            }
            _ => {}
        }
    }
    None
}

fn struct_field_name_context(text: &str, cursor: usize) -> bool {
    struct_field_argument_is_name(statement_fragment(text, cursor))
}

fn struct_field_argument_is_name(fragment: &str) -> bool {
    let mut stack = Vec::<(String, usize)>::new();
    let mut identifier = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in fragment.chars() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                identifier.clear();
            }
            character if character.is_alphanumeric() || character == '_' => {
                identifier.push(character)
            }
            '(' => {
                stack.push((std::mem::take(&mut identifier), 0));
            }
            ',' => {
                if let Some((_, argument)) = stack.last_mut() {
                    *argument += 1;
                }
                identifier.clear();
            }
            ')' => {
                stack.pop();
                identifier.clear();
            }
            character if character.is_whitespace() => {}
            _ => identifier.clear(),
        }
    }
    stack
        .last()
        .is_some_and(|(function, argument)| function == "field" && *argument >= 1)
}

fn fixed_header_candidates(text: &str, cursor: usize) -> Option<&'static [&'static str]> {
    let fragment = statement_fragment(text, cursor);
    let mut words = fragment.split_whitespace();
    let keyword = words.next()?;
    let remaining = words.count();
    if remaining > 1
        || (remaining == 1
            && fragment
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace))
    {
        return None;
    }
    match keyword {
        "slot" => Some(&[
            "expr",
            "expr_list",
            "literal",
            "number",
            "string",
            "boolean",
            "enum",
            "function",
            "ref",
            "block",
            "channel",
        ]),
        "variable" => Some(&["row", "column", "item"]),
        "adjust" => Some(&["expr", "nudge", "jitter", "dodge"]),
        _ => None,
    }
}

fn output_alias_context(text: &str, prefix_start: usize) -> bool {
    let fragment = statement_fragment(text, prefix_start);
    let Some(source) = fragment.strip_prefix("output") else {
        return false;
    };
    source.chars().next().is_some_and(char::is_whitespace) && !contains_top_level_word(source, "as")
}

fn contains_top_level_word(value: &str, expected: &str) -> bool {
    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    let mut word = String::new();
    let flush = |word: &mut String, depth: usize| {
        let matched = depth == 0 && word.eq_ignore_ascii_case(expected);
        word.clear();
        matched
    };
    for character in value.chars() {
        if let Some(active_quote) = quote {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == active_quote {
                quote = None;
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                if flush(&mut word, depth) {
                    return true;
                }
                quote = Some(character);
            }
            '(' | '[' => {
                if flush(&mut word, depth) {
                    return true;
                }
                depth += 1;
            }
            ')' | ']' => {
                if flush(&mut word, depth) {
                    return true;
                }
                depth = depth.saturating_sub(1);
            }
            character if character.is_alphanumeric() || character == '_' => word.push(character),
            _ => {
                if flush(&mut word, depth) {
                    return true;
                }
            }
        }
    }
    flush(&mut word, depth)
}

fn property_value_context(syntax: &SyntaxAnalysis, cursor: usize) -> Option<&str> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if node.span.range.start <= cursor && cursor <= node.span.range.end =>
            {
                let body_started =
                    significant_tokens(syntax, Some(node.span))
                        .into_iter()
                        .any(|token| {
                            matches!(token.token, Some(Token::LBrace))
                                && token.span.range.start < cursor
                        });
                (!body_started).then_some((node.span.range.len(), name.as_str()))
            }
            _ => None,
        })
        .min_by_key(|(length, _)| *length)
        .map(|(_, name)| name)
}

fn is_property_name_context(text: &str, cursor: usize) -> bool {
    let prefix = &text[..cursor.min(text.len())];
    let start = prefix
        .rfind(['{', '}', ';', '\n'])
        .map_or(0, |position| position + 1);
    !prefix[start..].contains(':')
}

fn authored_properties(
    syntax: &SyntaxAnalysis,
    scope: SourceSpan,
    cursor: usize,
) -> BTreeSet<String> {
    let property_owner = enclosing_property(syntax, cursor).map(|node| node.id);
    let declaration_owner = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. })
                && node.span.range.start == scope.range.start
        })
        .map(|node| node.id)
        .next();
    let expected_parent = property_owner.or(declaration_owner);
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if node.parent == expected_parent
                    && scope.range.start <= node.span.range.start
                    && node.span.range.end <= scope.range.end =>
            {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

fn enclosing_property(
    syntax: &SyntaxAnalysis,
    cursor: usize,
) -> Option<&avenger_lang_core::syntax::TolerantSyntaxNode> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::Property { .. })
                && node.span.range.start <= cursor
                && cursor <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())
}

fn enclosing_property_path(syntax: &SyntaxAnalysis, cursor: usize) -> Vec<&str> {
    let Some(innermost) = enclosing_property(syntax, cursor) else {
        return Vec::new();
    };
    let mut path = Vec::new();
    let mut current = Some(innermost.id);
    while let Some(id) = current {
        let node = &syntax.parsed.nodes[id.get() as usize];
        if let TolerantSyntaxNodeKind::Property { name } = &node.kind {
            path.push(name.as_str());
        }
        current = node.parent;
    }
    path.reverse();
    path
}

fn inside_legend_overlay_block(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let path = enclosing_property_path(syntax, cursor);
    path.windows(2).any(|pair| pair == ["legend", "overlay"])
}

fn nested_object_properties<'a>(
    syntax: &SyntaxAnalysis,
    schema: &'a KindSchema,
    cursor: usize,
) -> Option<&'a BTreeMap<String, PropertySchema>> {
    let mut properties = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::Property { .. })
                && node.span.range.start <= cursor
                && cursor <= node.span.range.end
        })
        .collect::<Vec<_>>();
    properties.sort_by_key(|node| node.span.range.len());
    let innermost = properties.first()?;
    let mut path = Vec::new();
    let mut current = Some(innermost.id);
    while let Some(id) = current {
        let node = &syntax.parsed.nodes[id.get() as usize];
        if let TolerantSyntaxNodeKind::Property { name } = &node.kind {
            path.push(name.as_str());
        }
        current = node.parent;
    }
    path.reverse();

    let first = path.first()?;
    let mut shape = &schema.properties.get(*first)?.shape;
    for name in path.into_iter().skip(1) {
        shape = match shape {
            ValueShape::Map(value) => value,
            ValueShape::Object(properties) => &properties.get(name)?.shape,
            ValueShape::ConfiguredExpression(properties) => &properties.get(name)?.shape,
            ValueShape::ConfiguredReference { properties, .. } => &properties.get(name)?.shape,
            ValueShape::Union(shapes) => shapes.iter().find_map(|shape| match shape {
                ValueShape::Object(properties) => properties.get(name).map(|item| &item.shape),
                _ => None,
            })?,
            _ => return None,
        };
    }
    match shape {
        ValueShape::Object(properties) | ValueShape::ConfiguredExpression(properties) => {
            Some(properties)
        }
        ValueShape::ConfiguredReference { properties, .. } => Some(properties),
        _ => None,
    }
}

fn declaration_header_role(syntax: &SyntaxAnalysis, symbol: &IndexedSymbol) -> Option<String> {
    let tokens = significant_tokens(syntax, Some(symbol.declaration_span));
    let source_keyword = match symbol.keyword.as_str() {
        "store" | "selection" => "param",
        keyword => keyword,
    };
    let keyword = tokens
        .iter()
        .position(|token| token.word() == Some(source_keyword))?;
    tokens.get(keyword + 1)?.word().map(str::to_owned)
}

#[derive(Clone, Copy)]
pub(crate) struct PropertyView<'a> {
    pub(crate) shape: &'a ValueShape,
    pub(crate) docs: &'a str,
}

pub(crate) fn property_schema<'a>(schema: &'a KindSchema, name: &str) -> Option<PropertyView<'a>> {
    schema
        .properties
        .get(name)
        .map(|property| PropertyView {
            shape: &property.shape,
            docs: &property.docs,
        })
        .or_else(|| {
            schema.channels.get(name).map(|channel| PropertyView {
                shape: &channel.shape,
                docs: &channel.docs,
            })
        })
        .or_else(|| {
            schema
                .additional_properties
                .as_ref()
                .map(|property| PropertyView {
                    shape: &property.shape,
                    docs: &property.docs,
                })
        })
}

fn property_item(name: &str, property: &PropertySchema, replacement: SourceSpan) -> CompletionItem {
    item(
        name.to_owned(),
        replacement,
        format!("{name}: "),
        CompletionKind::Property,
        Some(if property.required {
            format!("required · {}", shape_label(&property.shape))
        } else {
            shape_label(&property.shape)
        }),
        Some(property.docs.clone()),
        CompletionOrigin::AuthoringSchema,
        false,
        if property.required { "00" } else { "20" },
    )
}

fn complete_shape(
    shape: &ValueShape,
    prefix: &str,
    replacement: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    match shape {
        ValueShape::Boolean => {
            for value in ["true", "false"] {
                if candidate_matches(value, prefix) {
                    output.push(item(
                        value.to_owned(),
                        replacement,
                        value.to_owned(),
                        CompletionKind::EnumValue,
                        Some("boolean".to_owned()),
                        None,
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
        }
        ValueShape::Atom { values } => {
            for value in values {
                if candidate_matches(&value.value, prefix) {
                    output.push(item(
                        value.value.clone(),
                        replacement,
                        value.value.clone(),
                        CompletionKind::EnumValue,
                        Some("enum value".to_owned()),
                        Some(value.docs.clone()),
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
        }
        ValueShape::Union(shapes) => {
            for shape in shapes {
                complete_shape(shape, prefix, replacement, output);
            }
        }
        ValueShape::OneOrMany(shape) | ValueShape::Array(shape) => {
            complete_shape(shape, prefix, replacement, output)
        }
        _ => {}
    }
}

fn shape_accepts_binding(shape: &ValueShape) -> bool {
    match shape {
        ValueShape::ScalarBinding
        | ValueShape::TableBinding
        | ValueShape::SelectionBinding
        | ValueShape::SqlExpression
        | ValueShape::SqlQuery
        | ValueShape::Any => true,
        ValueShape::Union(shapes) => shapes.iter().any(shape_accepts_binding),
        _ => false,
    }
}

fn shape_label(shape: &ValueShape) -> String {
    match shape {
        ValueShape::Boolean => "boolean",
        ValueShape::Integer => "integer",
        ValueShape::Number => "number",
        ValueShape::String => "string",
        ValueShape::Identifier => "identifier",
        ValueShape::Atom { .. } => "enum",
        ValueShape::SqlExpression => "SQL expression",
        ValueShape::SqlQuery => "SQL query",
        ValueShape::ScalarBinding => "scalar binding",
        ValueShape::TableBinding => "table binding",
        ValueShape::SelectionBinding => "selection binding",
        ValueShape::TypedReference { .. } => "reference",
        ValueShape::Array(_) | ValueShape::OneOrMany(_) => "array",
        ValueShape::Object(_) | ValueShape::Map(_) => "object",
        _ => "value",
    }
    .to_owned()
}

fn complete_physical_types(
    prefix: &str,
    replacement: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    for constructor in avenger_lang_core::PhysicalType::CONSTRUCTORS {
        if !candidate_matches(constructor, prefix) {
            continue;
        }
        output.push(item(
            (*constructor).to_owned(),
            replacement,
            (*constructor).to_owned(),
            CompletionKind::Type,
            Some("Arrow physical type".to_owned()),
            None,
            CompletionOrigin::Syntax,
            false,
            "00",
        ));
    }
}

pub(crate) fn core_properties(keyword: &str) -> &'static [(&'static str, &'static str)] {
    match keyword {
        "param" => &[
            ("value", "Required initial scalar value."),
            ("sharing", "State-sharing policy."),
        ],
        "store" => &[
            ("sharing", "State-sharing policy."),
            ("primary_key", "Fields that uniquely identify store rows."),
        ],
        "selection" => &[
            ("empty", "Selection behavior when no values are selected."),
            ("combine", "How multiple selection clauses combine."),
        ],
        "chart" | "plot" => &[
            ("data", "Data relation visible to this plot."),
            ("title", "Chart title expression."),
            ("subtitle", "Chart subtitle expression."),
            ("layout", "Canvas and plot-area layout."),
            ("theme", "Theme configuration."),
            ("time", "Temporal defaults."),
            ("format", "Formatting defaults."),
            ("guide", "Guide styling."),
        ],
        "cell" => &[
            ("at", "Cell position."),
            ("data", "Cell data relation."),
            ("label", "Cell label."),
            ("when", "Cell predicate."),
        ],
        "view" | "mark" => &[("data", "Data relation visible here.")],
        "tool" => &[("id", "Stable tool component identifier.")],
        "on" => &[
            ("target", "Event target."),
            ("scope", "Event scope."),
            ("surface", "Event surface."),
            ("filter", "Event filter expression."),
            ("throttle_ms", "Event throttle duration in milliseconds."),
            ("consume", "Whether the event is consumed."),
            ("mode", "Preview or exact evaluation mode."),
            ("settle_exact", "Request exact evaluation after settling."),
            ("between", "Paired event interval."),
        ],
        _ => &[],
    }
}

fn core_property_values(property: &str) -> &'static [&'static str] {
    match property {
        "sharing" => &["shared", "free", "level("],
        "empty" => &["all", "none"],
        "combine" => &["union", "intersect"],
        "mode" => &["preview", "exact"],
        "consume" | "settle_exact" => &["true", "false"],
        _ => &[],
    }
}

fn replacement_span(syntax: &SyntaxAnalysis, cursor: usize) -> SourceSpan {
    let text = syntax.parsed.tokens.text();
    let mut start = cursor.min(text.len());
    while start > 0 {
        let character = text[..start].chars().next_back().unwrap();
        if is_completion_character(character) {
            start -= character.len_utf8();
        } else {
            break;
        }
    }
    let mut end = cursor.min(text.len());
    while end < text.len() {
        let character = text[end..].chars().next().unwrap();
        if is_completion_character(character) {
            end += character.len_utf8();
        } else {
            break;
        }
    }
    SourceSpan {
        source: syntax.parsed.nodes[0].span.source,
        range: ByteSpan { start, end },
    }
}

fn is_completion_character(character: char) -> bool {
    character == '$'
        || character == '@'
        || character == '.'
        || character == '_'
        || character.is_alphanumeric()
}

fn import_prefix(text: &str, cursor: usize) -> Option<&str> {
    let prefix = &text[..cursor.min(text.len())];
    let statement = prefix
        .rsplit_once(';')
        .map_or(prefix, |(_, statement)| statement)
        .trim_start();
    let import = statement.strip_prefix("import")?;
    let (quote_start, quote) = import
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '\'' | '"'))?;
    let before = &import[..quote_start];
    if !before
        .split_whitespace()
        .last()
        .is_some_and(|word| word == "from")
    {
        return None;
    }
    let value = &import[quote_start + quote.len_utf8()..];
    (!value.contains(quote)).then_some(value)
}

fn candidate_matches(candidate: &str, typed: &str) -> bool {
    let typed = typed.trim_start_matches('$').to_ascii_lowercase();
    candidate.to_ascii_lowercase().contains(&typed)
}

#[allow(clippy::too_many_arguments)]
fn item(
    label: String,
    replacement: SourceSpan,
    insert_text: String,
    kind: CompletionKind,
    detail: Option<String>,
    documentation: Option<String>,
    origin: CompletionOrigin,
    deprecated: bool,
    bucket: &str,
) -> CompletionItem {
    CompletionItem {
        filter_text: Some(label.clone()),
        sort_key: format!("{bucket}:{}", label.to_ascii_lowercase()),
        label,
        replacement,
        insert_text,
        insert_text_format: CompletionTextFormat::PlainText,
        kind,
        detail,
        documentation,
        origin,
        deprecated,
    }
}

fn rank_and_deduplicate(items: &mut Vec<CompletionItem>, prefix: &str) {
    let typed = prefix.trim_start_matches('$').to_ascii_lowercase();
    for item in items.iter_mut() {
        let label = item.label.trim_start_matches('$').to_ascii_lowercase();
        let match_rank = if label == typed {
            "0"
        } else if label.starts_with(&typed) {
            "1"
        } else {
            "2"
        };
        item.sort_key = format!("{match_rank}:{}", item.sort_key);
    }
    items.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.label.cmp(&right.label))
    });
    items
        .dedup_by(|left, right| left.label == right.label && left.insert_text == right.insert_text);
}

fn declaration_snippet(keyword: &str) -> String {
    match keyword {
        "param" => "param ${1:float64} as ${2:name} {\n  value: ${3:NULL};\n  $0\n}".to_owned(),
        "param store" | "param selection" => {
            format!("{keyword} as ${{1:name}} {{\n  $0\n}}")
        }
        "mark" | "transform" | "tool" | "widget" | "resource" => {
            format!("{keyword} ${{1:kind}} as ${{2:name}} {{\n  $0\n}}")
        }
        "slot channel" => "slot channel ${1:name};".to_owned(),
        "slot" => "slot ${1:expr} ${2:name};".to_owned(),
        "variable" => "variable ${1:row} ${2:name} {\n  $0\n}".to_owned(),
        "field" => "field ${1:utf8} ${2:name};".to_owned(),
        "output" => "output ${1:value} as ${2:name};".to_owned(),
        "adjust" => "adjust expr {\n  $0\n}".to_owned(),
        "view" => "view ${1:kind} as ${2:name} {\n  $0\n}".to_owned(),
        _ => keyword.to_owned(),
    }
}

fn source_declaration_label(semantic_keyword: &str) -> Option<&str> {
    match semantic_keyword {
        "store" => Some("param store"),
        "selection" => Some("param selection"),
        "channel" => Some("slot channel"),
        "dimension" => None,
        "param" => Some("param"),
        "slot" => Some("slot"),
        "adjust" => Some("adjust"),
        "variable" => Some("variable"),
        "field" => Some("field"),
        "output" => Some("output"),
        "view" => Some("view"),
        "mark" => Some("mark"),
        "transform" => Some("transform"),
        "tool" => Some("tool"),
        "widget" => Some("widget"),
        "resource" => Some("resource"),
        "catalog" => Some("catalog"),
        "schema" => Some("schema"),
        "table" => Some("table"),
        "cell" => Some("cell"),
        "plot" => Some("plot"),
        "axis" => Some("axis"),
        "legend" => Some("legend"),
        "layout" => Some("layout"),
        "theme" => Some("theme"),
        "derive" => Some("derive"),
        "layer" => Some("layer"),
        "level" => Some("level"),
        "part" => Some("part"),
        "when" => Some("when"),
        "row" => Some("row"),
        "key" => Some("key"),
        "fields" => Some("fields"),
        "scale_hint" => Some("scale_hint"),
        "frame" => Some("frame"),
        "on" => Some("on"),
        "set" => Some("set"),
        "scale_edit" => Some("scale_edit"),
        "match" => Some("match"),
        "splice" => Some("splice"),
        "export" => Some("export"),
        other => Some(other),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use avenger_lang_compiler::Compiler;
    use avenger_lang_core::{SourceFile, SourceId};

    use super::*;
    use crate::{DocumentSnapshot, SourceRevision, analyze_syntax};

    fn fixture(text: &str) -> (SourceOrigin, SyntaxAnalysis) {
        let origin = SourceOrigin::Memory("fixture.avenger".to_owned());
        let snapshot = DocumentSnapshot::new(
            origin.clone(),
            SourceRevision::from_text(text),
            Arc::<str>::from(text),
        );
        (origin, analyze_syntax(&snapshot))
    }

    fn completion_labels(marked: &str) -> Vec<String> {
        let cursor = marked.find('|').expect("completion marker");
        let text = marked.replacen('|', "", 1);
        let (origin, syntax) = fixture(&text);
        let revision = syntax.revision.clone();
        let syntax = BTreeMap::from([(origin.clone(), syntax)]);
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let semantic_roots = BTreeMap::new();
        let dataset_contexts = BTreeMap::new();
        QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
        )
        .complete(
            &PositionRequest {
                source: origin,
                byte_offset: cursor,
                source_revision: revision,
            },
            CompletionOptions::default(),
            &AnalysisCancellation::default(),
        )
        .unwrap()
        .items
        .into_iter()
        .map(|item| item.label)
        .collect()
    }

    #[test]
    fn index_preserves_exact_binder_and_reference_spans() {
        let text = "avenger 1; chart cartesian as chart { param float64 as width { value: 1.0; } mark symbol as points { size: $width; } }";
        let (origin, syntax) = fixture(text);
        let index = build_document_index(&origin, &syntax);
        let width = index
            .symbols
            .iter()
            .find(|symbol| symbol.name == "width")
            .unwrap();
        assert_eq!(&text[width.selection_span.range.as_range()], "width");
        let reference = index
            .references
            .iter()
            .find(|reference| reference.name == "width")
            .unwrap();
        assert_eq!(&text[reference.span.range.as_range()], "$width");
    }

    #[test]
    fn index_preserves_unified_header_names_and_semantic_categories() {
        let text = r#"avenger 1; chart cartesian {
          param store as rows {}
          param selection as picked {}
          mark group as layer {}
          variable row mpg {}
          field float64 amount;
          output amount as total;
        }"#;
        let (origin, syntax) = fixture(text);
        let index = build_document_index(&origin, &syntax);
        for (name, keyword, value_kind) in [
            ("rows", "store", IndexedValueKind::Table),
            ("picked", "selection", IndexedValueKind::Selection),
            ("layer", "mark", IndexedValueKind::Mark),
            ("mpg", "variable", IndexedValueKind::Declaration),
            ("amount", "field", IndexedValueKind::Field),
            ("total", "output", IndexedValueKind::Output),
        ] {
            let symbol = index
                .symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap_or_else(|| panic!("missing {name}: {:?}", index.symbols));
            assert_eq!(symbol.keyword, keyword);
            assert_eq!(symbol.value_kind, value_kind);
            assert_eq!(&text[symbol.selection_span.range.as_range()], name);
        }
    }

    #[test]
    fn registry_completion_has_no_native_kind_switch() {
        let text = "avenger 1; chart cartesian as chart { mark sy }";
        let (origin, syntax) = fixture(text);
        let revision = syntax.revision.clone();
        let syntax = BTreeMap::from([(origin.clone(), syntax)]);
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let semantic_roots = BTreeMap::new();
        let dataset_contexts = BTreeMap::new();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
        );
        let result = context
            .complete(
                &PositionRequest {
                    source: origin,
                    byte_offset: text.find("sy").unwrap() + 2,
                    source_revision: revision,
                },
                CompletionOptions::default(),
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert!(result.items.iter().any(|item| item.label == "symbol"));
    }

    #[test]
    fn completion_replacement_never_splits_unicode() {
        let text = "avenger 1; chart cartesian as chart { param float64 as café { value: 1.0; } mark symbol { size: $caf; } }";
        let (_, syntax) = fixture(text);
        let cursor = text.find("$caf").unwrap() + "$caf".len();
        let span = replacement_span(&syntax, cursor);
        assert_eq!(&text[span.range.as_range()], "$caf");
    }

    #[test]
    fn unified_header_and_set_target_completion_use_semantic_categories() {
        let types = completion_labels("avenger 1; chart cartesian { param | }");
        assert!(types.contains(&"float64".to_owned()));
        assert!(types.contains(&"store".to_owned()));
        assert!(types.contains(&"selection".to_owned()));

        let fields = completion_labels("avenger 1; chart cartesian { field utf| }");
        assert!(fields.contains(&"utf8".to_owned()));

        let targets = completion_labels(
            "avenger 1; chart cartesian { param float64 as width { value: 1; } param store as rows {} param selection as picked {} on click { set | = 1; } }",
        );
        for target in ["width", "rows", "picked", "cursor"] {
            assert!(targets.contains(&target.to_owned()), "{targets:?}");
        }

        let store_ops = completion_labels(
            "avenger 1; chart cartesian { param store as rows {} on click { set rows = |; } }",
        );
        assert!(
            store_ops.contains(&"insert_rows".to_owned()),
            "{store_ops:?}"
        );
        assert!(
            !store_ops.contains(&"toggle_clauses".to_owned()),
            "{store_ops:?}"
        );

        let selection_ops = completion_labels(
            "avenger 1; chart cartesian { param selection as picked {} on click { set picked = |; } }",
        );
        assert!(
            selection_ops.contains(&"toggle_clauses".to_owned()),
            "{selection_ops:?}"
        );
        assert!(
            !selection_ops.contains(&"insert_rows".to_owned()),
            "{selection_ops:?}"
        );

        let scalar_modifiers = completion_labels(
            "avenger 1; chart cartesian { param float64 as width { value: 1; } on click { set width | = 2; } }",
        );
        assert!(
            scalar_modifiers.contains(&"at".to_owned()),
            "{scalar_modifiers:?}"
        );
        assert!(
            scalar_modifiers.contains(&"replacing".to_owned()),
            "{scalar_modifiers:?}"
        );

        let selection_modifiers = completion_labels(
            "avenger 1; chart cartesian { param selection as picked {} on click { set picked | = clear; } }",
        );
        assert!(
            selection_modifiers.contains(&"at".to_owned()),
            "{selection_modifiers:?}"
        );
        assert!(
            !selection_modifiers.contains(&"replacing".to_owned()),
            "{selection_modifiers:?}"
        );

        let bindings = completion_labels(
            "avenger 1; chart cartesian { param float64 as width { value: 1; } param store as rows {} param selection as picked {} mark symbol { size: $|; } }",
        );
        assert!(bindings.contains(&"$width".to_owned()));
        assert!(bindings.contains(&"$rows".to_owned()));
        assert!(!bindings.contains(&"$picked".to_owned()));

        let binder = completion_labels("avenger 1; chart cartesian { param float64 | }");
        assert!(binder.contains(&"as".to_owned()), "{binder:?}");
        assert!(!binder.contains(&"float64".to_owned()), "{binder:?}");

        let field = completion_labels(
            "avenger 1; chart cartesian { param store as rows { field utf8 name | } }",
        );
        assert!(field.contains(&"nullable".to_owned()), "{field:?}");

        let member = completion_labels(
            "avenger 1; chart cartesian { param struct(field(float64, |)) as value { value: NULL; } }",
        );
        assert!(member.contains(&"'<name>'".to_owned()), "{member:?}");
        assert!(!member.contains(&"float64".to_owned()), "{member:?}");

        let slot_name =
            completion_labels("avenger 1; define mark sample { slot expr | mark symbol {} }");
        assert!(!slot_name.contains(&"expr".to_owned()), "{slot_name:?}");
        assert!(!slot_name.contains(&"as".to_owned()), "{slot_name:?}");

        let slot_shapes = completion_labels("avenger 1; define mark sample { slot | }");
        for shape in ["expr", "expr_list", "literal", "channel"] {
            assert!(slot_shapes.contains(&shape.to_owned()), "{slot_shapes:?}");
        }
        assert!(
            !slot_shapes.contains(&"value".to_owned()),
            "{slot_shapes:?}"
        );
        assert!(!slot_shapes.contains(&"list".to_owned()), "{slot_shapes:?}");

        let variable_name =
            completion_labels("avenger 1; chart repeat_grid { variable row | cell cartesian {} }");
        assert!(
            !variable_name.contains(&"row".to_owned()),
            "{variable_name:?}"
        );
        assert!(
            !variable_name.contains(&"as".to_owned()),
            "{variable_name:?}"
        );

        let channel_body = completion_labels(
            "avenger 1; define mark sample { slot channel position { | } mark symbol {} }",
        );
        assert_eq!(channel_body, ["default"]);
        let channel_default = completion_labels(
            "avenger 1; define mark sample { slot channel position { default: |; } mark symbol {} }",
        );
        assert!(
            channel_default.contains(&"x".to_owned()),
            "{channel_default:?}"
        );
        assert!(
            channel_default.contains(&"color".to_owned()),
            "{channel_default:?}"
        );

        let equality = completion_labels("avenger 1; chart cartesian { equality { id { | } } }");
        assert!(equality.contains(&"field".to_owned()), "{equality:?}");
        assert!(equality.contains(&"value".to_owned()), "{equality:?}");

        let interval = completion_labels("avenger 1; chart cartesian { interval { x { | } } }");
        for property in ["field", "from", "to"] {
            assert!(interval.contains(&property.to_owned()), "{interval:?}");
        }

        let adjust =
            completion_labels("avenger 1; chart cartesian { mark symbol { adjust expr { | } } }");
        assert!(adjust.contains(&"x".to_owned()), "{adjust:?}");
        assert!(adjust.contains(&"fill".to_owned()), "{adjust:?}");

        let output_alias = completion_labels(
            "avenger 1; define transform sample { output CAST(value AS float64) |; }",
        );
        assert!(output_alias.contains(&"as".to_owned()), "{output_alias:?}");
    }

    #[test]
    fn declaration_completion_emits_only_canonical_source_starters() {
        let labels = completion_labels("avenger 1; chart cartesian { | }");
        assert!(labels.contains(&"param store".to_owned()));
        assert!(labels.contains(&"param selection".to_owned()));
        assert!(labels.contains(&"mark group".to_owned()));
        assert!(!labels.contains(&"store".to_owned()));
        assert!(!labels.contains(&"selection".to_owned()));
        assert!(!labels.contains(&"group".to_owned()));
        assert!(!labels.contains(&"dimension".to_owned()));
    }

    #[test]
    fn group_and_legend_overlay_completion_use_mark_semantics() {
        let mark_kinds = completion_labels("avenger 1; chart cartesian { mark | }");
        assert!(mark_kinds.contains(&"group".to_owned()), "{mark_kinds:?}");

        let channel_configs =
            completion_labels("avenger 1; chart cartesian { mark symbol { fill: 'x' { | } } }");
        assert!(
            channel_configs.contains(&"legend".to_owned()),
            "{channel_configs:?}"
        );

        let legend_properties = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: 'x' { legend: { | } } } }",
        );
        assert!(
            legend_properties.contains(&"overlay".to_owned()),
            "{legend_properties:?}"
        );

        let overlay_declarations = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: 'x' { legend: { overlay: { | } } } } }",
        );
        assert_eq!(overlay_declarations, vec!["mark"]);

        let overlay_mark_kinds = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: 'x' { legend: { overlay: { mark | } } } } }",
        );
        assert!(
            overlay_mark_kinds.contains(&"group".to_owned()),
            "{overlay_mark_kinds:?}"
        );
        assert!(
            overlay_mark_kinds.contains(&"rect".to_owned()),
            "{overlay_mark_kinds:?}"
        );
        assert!(
            !overlay_mark_kinds.contains(&"arc".to_owned()),
            "{overlay_mark_kinds:?}"
        );
    }

    #[test]
    fn parallel_frame_completion_uses_nested_authoring_schema() {
        let frame = completion_labels(
            "avenger 1; chart parallel { dimensions: { mpg: { | } } mark parallel_line { dimensions: { mpg: \"mpg\"; } } }",
        );
        assert!(frame.contains(&"axis".to_owned()), "{frame:?}");

        let axis = completion_labels(
            "avenger 1; chart parallel { dimensions: { mpg: { axis: { | } } } mark parallel_line { dimensions: { mpg: \"mpg\"; } } }",
        );
        for property in ["title", "visible", "grid", "tick_count"] {
            assert!(axis.contains(&property.to_owned()), "{axis:?}");
        }
        assert!(!axis.contains(&"data".to_owned()), "{axis:?}");
    }

    #[test]
    fn tolerant_imports_complete_and_navigate_qualified_definitions() {
        let chart_text = "avenger 1; import * as defs from 'definitions.avenger'; chart cartesian as chart { mark defs.badge as badge {} }";
        let definition_text = "avenger 1; export define mark badge { mark symbol {} }";
        let chart_origin = SourceOrigin::Memory("multi/chart.avenger".to_owned());
        let definition_origin = SourceOrigin::Memory("multi/definitions.avenger".to_owned());
        let chart = analyze_syntax(&DocumentSnapshot::new(
            chart_origin.clone(),
            SourceRevision::from_text(chart_text),
            chart_text,
        ));
        let definition = analyze_syntax(&DocumentSnapshot::new(
            definition_origin.clone(),
            SourceRevision::from_text(definition_text),
            definition_text,
        ));
        let syntax = BTreeMap::from([
            (chart_origin.clone(), chart),
            (definition_origin.clone(), definition),
        ]);
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let semantic_roots = BTreeMap::new();
        let dataset_contexts = BTreeMap::new();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
        );
        let reference_start = chart_text.find("defs.badge").unwrap();
        let navigation = context
            .definition(
                &PositionRequest {
                    source: chart_origin.clone(),
                    byte_offset: reference_start + 2,
                    source_revision: SourceRevision::from_text(chart_text),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert_eq!(navigation.targets.len(), 1);
        assert_eq!(navigation.targets[0].origin, definition_origin);

        let completion = context
            .complete(
                &PositionRequest {
                    source: chart_origin,
                    byte_offset: reference_start + "defs.ba".len(),
                    source_revision: SourceRevision::from_text(chart_text),
                },
                CompletionOptions::default(),
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert!(
            completion
                .items
                .iter()
                .any(|item| item.label == "defs.badge")
        );
    }

    #[test]
    fn tolerant_named_imports_navigate_exported_definition_bindings() {
        let chart_text = "avenger 1; import { dot } from 'dot.avenger'; chart cartesian as chart { mark dot as points {} }";
        let definition_text = "avenger 1; export define mark dot { mark symbol {} }";
        let chart_origin = SourceOrigin::Memory("multi/chart.avenger".to_owned());
        let definition_origin = SourceOrigin::Memory("multi/dot.avenger".to_owned());
        let chart = analyze_syntax(&DocumentSnapshot::new(
            chart_origin.clone(),
            SourceRevision::from_text(chart_text),
            chart_text,
        ));
        let definition = analyze_syntax(&DocumentSnapshot::new(
            definition_origin.clone(),
            SourceRevision::from_text(definition_text),
            definition_text,
        ));
        let syntax = BTreeMap::from([
            (chart_origin.clone(), chart),
            (definition_origin.clone(), definition),
        ]);
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let semantic_roots = BTreeMap::new();
        let dataset_contexts = BTreeMap::new();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
        );
        let reference_start = chart_text.find("mark dot").unwrap() + "mark ".len();
        let navigation = context
            .definition(
                &PositionRequest {
                    source: chart_origin,
                    byte_offset: reference_start + 1,
                    source_revision: SourceRevision::from_text(chart_text),
                },
                &AnalysisCancellation::default(),
            )
            .unwrap();
        assert_eq!(navigation.targets.len(), 1, "index: {index:#?}");
        assert_eq!(navigation.targets[0].origin, definition_origin);
    }

    #[test]
    fn tolerant_import_bindings_are_scoped_to_each_importer() {
        let chart_text = "avenger 1; import { dot } from 'dot.avenger'; chart cartesian as chart { mark dot as points {} }";
        let definition_text = "avenger 1; export define mark dot { mark symbol {} }";
        let origins = [
            (
                SourceOrigin::Memory("multi/a/chart.avenger".to_owned()),
                SourceOrigin::Memory("multi/a/dot.avenger".to_owned()),
            ),
            (
                SourceOrigin::Memory("multi/b/chart.avenger".to_owned()),
                SourceOrigin::Memory("multi/b/dot.avenger".to_owned()),
            ),
        ];
        let mut syntax = BTreeMap::new();
        for (chart_origin, definition_origin) in &origins {
            syntax.insert(
                chart_origin.clone(),
                analyze_syntax(&DocumentSnapshot::new(
                    chart_origin.clone(),
                    SourceRevision::from_text(chart_text),
                    chart_text,
                )),
            );
            syntax.insert(
                definition_origin.clone(),
                analyze_syntax(&DocumentSnapshot::new(
                    definition_origin.clone(),
                    SourceRevision::from_text(definition_text),
                    definition_text,
                )),
            );
        }
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let semantic_roots = BTreeMap::new();
        let dataset_contexts = BTreeMap::new();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
        );
        let reference = chart_text.find("mark dot").unwrap() + "mark d".len();
        for (chart_origin, definition_origin) in origins {
            let navigation = context
                .definition(
                    &PositionRequest {
                        source: chart_origin,
                        byte_offset: reference,
                        source_revision: SourceRevision::from_text(chart_text),
                    },
                    &AnalysisCancellation::default(),
                )
                .unwrap();
            assert_eq!(navigation.targets.len(), 1);
            assert_eq!(navigation.targets[0].origin, definition_origin);
        }
    }

    #[test]
    fn source_span_constructor_used_by_fixture_is_valid() {
        let source = SourceFile::new(SourceId::new(7), SourceOrigin::Memory("x".to_owned()), "x");
        assert_eq!(source.text(), "x");
    }
}
