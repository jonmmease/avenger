use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    ValueShape,
};
use avenger_lang_core::{
    BindingCategory, ByteSpan, ModuleId, ResolvedDeclaration, ResolvedModuleGraph,
    ResolvedOutputHandle, ResolvedTarget, ResolvedValue, SourceFile, SourceId, SourceOrigin,
    SourceSpan, StateSharing, allowed_child_declarations,
    ast::{BindingTime, StateActionVerb, Visibility, is_state_action_keyword},
    sql::{LosslessTokenKind, TokenClass},
    syntax::{SqlIslandSite, TolerantSyntaxNodeId, TolerantSyntaxNodeKind, parse_file},
};
use sqlparser::tokenizer::Token;

use crate::completion_rank::{candidate_matches, rank_and_deduplicate};
use crate::{
    AnalysisCancellation, AnalysisGeneration, CompletionItem, CompletionKind, CompletionOrigin,
    CompletionQualification, CompletionResult, CompletionSemanticKind, CompletionTextFormat,
    CompletionValidity, DatasetContext, HoverResult, NavigationResult, NavigationTarget,
    PositionRequest, RootAnalysis, SymbolKind, SyntaxAnalysis,
};
use avenger_lang_compiler::{ModuleAnalysis, ParamTypeProvenance};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionOptions {
    pub snippets: bool,
    pub invocation: CompletionInvocation,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CompletionInvocation {
    #[default]
    Invoked,
    TriggerCharacter(String),
    TriggerForIncompleteCompletions,
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
    pub sql_islands: Vec<SqlIslandDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlIslandDescriptor {
    pub identity: String,
    pub authored_fingerprint: String,
    pub site: SqlIslandSite,
    pub span: SourceSpan,
    pub declaration_keyword: Option<String>,
    pub declaration_name: Option<String>,
    pub declaration_path: Vec<SqlIslandOwner>,
    pub property_path: Vec<String>,
    pub channel_mode: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlIslandOwner {
    pub keyword: String,
    pub name: String,
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
            let Some(project) = analysis.resolved_module_graph.as_deref() else {
                continue;
            };
            index.enrich_from_resolved(analysis, project, syntax);
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

    fn enrich_from_resolved(
        &mut self,
        analysis: &ModuleAnalysis,
        project: &ResolvedModuleGraph,
        syntax: &BTreeMap<SourceOrigin, SyntaxAnalysis>,
    ) {
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
                // The tolerant syntax describes the document currently open in the
                // editor, while `project` may be the last successfully resolved
                // snapshot. Preserve a current authored kind when one was recovered,
                // and use the semantic kind only to fill an incomplete header.
                if symbol.native_kind.is_none() {
                    symbol.native_kind.clone_from(&declaration.kind);
                }
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

        self.install_output_alias_symbols(project, syntax, &declarations);

        for param in project.params.values() {
            let detail = analysis.param_types.info(&param.id).map_or_else(
                || "param: type unavailable".to_owned(),
                |type_info| {
                    let provenance = match type_info.provenance {
                        ParamTypeProvenance::Inferred => "inferred from the SQL initializer",
                        ParamTypeProvenance::SchemaFixed => "fixed by a native schema",
                    };
                    let sharing = match param.sharing {
                        StateSharing::Shared => "shared".to_owned(),
                        StateSharing::Free => "free".to_owned(),
                        StateSharing::Level(level) => format!("level({level})"),
                    };
                    format!(
                        "param: {}\n\n- Initializer: `{}`\n- Type source: {}\n- Nullability: typed nulls are permitted\n- Sharing: `{sharing}`",
                        type_info.data_type,
                        resolved_value_summary(&param.initializer),
                        provenance,
                    )
                },
            );
            self.enrich_state_symbol(
                &param.declaration.to_string(),
                IndexedValueKind::Scalar,
                detail,
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

            let mut relations = BTreeMap::<String, &avenger_lang_core::ResolvedCatalogTable>::new();
            for table in project
                .catalog_tables
                .values()
                .filter(|table| table.file == file.id)
            {
                relations.insert(table.path.join("."), table);
            }
            for ((category, local), export) in &file.local_bindings.local {
                if *category != BindingCategory::Data {
                    continue;
                }
                let ModuleId::Source(module) = &export.module else {
                    continue;
                };
                for table in project.catalog_tables.values().filter(|table| {
                    table.relation.defining_item.module == *module
                        && table.path.first() == Some(&export.name)
                }) {
                    let path = std::iter::once(local.clone())
                        .chain(table.path.iter().skip(1).cloned())
                        .collect::<Vec<_>>()
                        .join(".");
                    relations.insert(path, table);
                }
            }
            for (namespace, module) in &file.local_bindings.namespaces {
                let ModuleId::Source(module) = module else {
                    continue;
                };
                let Some(exports) = project
                    .source_modules
                    .get(module)
                    .map(|module| &module.exports)
                else {
                    continue;
                };
                for table in project.catalog_tables.values().filter(|table| {
                    table.relation.defining_item.module == *module
                        && table.path.first().is_some_and(|root| {
                            exports
                                .exports
                                .get(root)
                                .is_some_and(|export| export.category == BindingCategory::Data)
                        })
                }) {
                    let path = std::iter::once(namespace.clone())
                        .chain(table.path.iter().cloned())
                        .collect::<Vec<_>>()
                        .join(".");
                    relations.insert(path, table);
                }
            }
            for (path, table) in relations {
                let target = self.navigation_for_identity(table.id.as_str());
                self.public_references.insert(
                    (importer.clone(), path.clone()),
                    IndexedBinding {
                        path,
                        value_kind: IndexedValueKind::Table,
                        detail: format!("{} table relation", table.kind),
                        target,
                    },
                );
            }
        }
        self.install_output_references(project, syntax);
    }

    fn install_output_alias_symbols(
        &mut self,
        project: &ResolvedModuleGraph,
        syntax: &BTreeMap<SourceOrigin, SyntaxAnalysis>,
        declarations: &BTreeMap<String, &ResolvedDeclaration>,
    ) {
        for declaration in declarations.values().copied() {
            if declaration.keyword != "transform" || declaration.transform_outputs.is_empty() {
                continue;
            }
            let authored = project.expansion_source_map.authored_span(declaration.span);
            let Some(origin) = project
                .sources
                .get(authored.source)
                .map(|source| source.origin.clone())
            else {
                continue;
            };
            let Some(document_syntax) = syntax.get(&origin) else {
                continue;
            };
            let authored_syntax_span = SourceSpan {
                source: document_syntax.parsed.tokens.source(),
                range: authored.range,
            };
            let expected = declaration
                .transform_outputs
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>();
            let aliases = projection_alias_spans(document_syntax, authored, &expected);
            let Some(document) = self.documents.get_mut(&origin) else {
                continue;
            };
            let parent = document
                .symbols
                .iter()
                .position(|symbol| symbol.identity == declaration.id.to_string());
            let scope_span = parent
                .and_then(|index| document.symbols[index].parent)
                .map(|index| document.symbols[index].scope_span)
                .unwrap_or_else(|| SourceSpan {
                    source: document_syntax.parsed.tokens.source(),
                    range: ByteSpan {
                        start: 0,
                        end: document_syntax.parsed.tokens.text().len(),
                    },
                });
            for (name, selection_span) in aliases {
                let Some(handle) = declaration.transform_outputs.get(&name) else {
                    continue;
                };
                let identity = output_identity(handle);
                if document
                    .symbols
                    .iter()
                    .any(|symbol| symbol.identity == identity)
                {
                    continue;
                }
                document.symbols.push(IndexedSymbol {
                    identity,
                    name: name.clone(),
                    kind: SymbolKind::Field,
                    value_kind: IndexedValueKind::Output,
                    origin: origin.clone(),
                    declaration_span: authored_syntax_span,
                    selection_span,
                    scope_span,
                    parent,
                    keyword: "output_alias".to_owned(),
                    native_kind: declaration.kind.clone(),
                    visibility: declaration.visibility,
                    exported: false,
                    detail: Some(match handle.shape {
                        avenger_lang_core::ResolvedOutputShape::Expression => {
                            if declaration.kind.as_deref() == Some("scalar_aggregate") {
                                format!("derived scalar output `{name}`")
                            } else {
                                format!("transform output column `{name}`")
                            }
                        }
                        avenger_lang_core::ResolvedOutputShape::RasterDimension => {
                            format!("transform raster dimension `{name}`")
                        }
                        avenger_lang_core::ResolvedOutputShape::Opaque => {
                            format!("transform output `{name}`")
                        }
                    }),
                    documentation: Some(
                        "Caller-authored output declared by this SQL projection alias.".to_owned(),
                    ),
                });
            }
        }
    }

    fn install_output_references(
        &mut self,
        project: &ResolvedModuleGraph,
        syntax: &BTreeMap<SourceOrigin, SyntaxAnalysis>,
    ) {
        let mut references = Vec::new();
        for module in project.source_modules.values() {
            collect_output_references(&module.roots, &mut references);
        }
        for (declaration, path, handle) in references {
            let authored = project.expansion_source_map.authored_span(declaration.span);
            let Some(origin) = project
                .sources
                .get(authored.source)
                .map(|source| source.origin.clone())
            else {
                continue;
            };
            let Some(document_syntax) = syntax.get(&origin) else {
                continue;
            };
            let spans = sql_path_spans(document_syntax, authored, &path);
            let Some(document) = self.documents.get_mut(&origin) else {
                continue;
            };
            let identity = output_identity(&handle);
            for span in spans {
                if document
                    .symbols
                    .iter()
                    .any(|symbol| symbol.selection_span == span)
                {
                    continue;
                }
                if let Some(reference) = document
                    .references
                    .iter_mut()
                    .find(|reference| reference.span == span)
                {
                    reference.target_identity = Some(identity.clone());
                    reference.value_kind = IndexedValueKind::Output;
                    continue;
                }
                document.references.push(IndexedReference {
                    name: path.join("."),
                    origin: origin.clone(),
                    span,
                    target_identity: Some(identity.clone()),
                    value_kind: IndexedValueKind::Output,
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
                if reference.target_identity.is_some() {
                    continue;
                }
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

fn resolved_value_summary(value: &ResolvedValue) -> String {
    match value {
        ResolvedValue::String(value) => format!("'{}'", value.replace('\'', "''")),
        ResolvedValue::Number(value) | ResolvedValue::Atom(value) => value.clone(),
        ResolvedValue::Boolean(value) => value.to_string(),
        ResolvedValue::Null => "NULL".to_owned(),
        ResolvedValue::Column(value) => format!("\"{}\"", value.replace('\"', "\"\"")),
        ResolvedValue::Expression(value) => value.sql.clone(),
        ResolvedValue::Relation(reference) => reference.authored_path.join("."),
        ResolvedValue::Binding(binding) => {
            let mut value = format!("${}", binding.authored_path.join("."));
            match binding.time {
                BindingTime::Current => {}
                BindingTime::Start => value.push_str("@start"),
                BindingTime::Previous => value.push_str("@previous"),
            }
            value
        }
        ResolvedValue::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(resolved_value_summary)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ResolvedValue::Call { function, args } => format!(
            "{function}({})",
            args.iter()
                .map(resolved_value_summary)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => "<complex expression>".to_owned(),
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

    output.sql_islands = syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| sql_island_descriptor(syntax, node))
        .collect();
    for island in &mut output.sql_islands {
        island.identity = format!(
            "sql-island:{}:{}:{}:{}:{}",
            origin.canonical_uri(),
            island.site.manifest_name(),
            island.property_path.join("."),
            island.span.range.start,
            island.authored_fingerprint
        );
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

fn sql_island_descriptor(
    syntax: &SyntaxAnalysis,
    node: &avenger_lang_core::syntax::TolerantSyntaxNode,
) -> Option<SqlIslandDescriptor> {
    let TolerantSyntaxNodeKind::SqlIsland { site, fingerprint } = &node.kind else {
        return None;
    };
    let mut declaration_keyword = None;
    let mut declaration_name = None;
    let mut declaration_path = Vec::new();
    let mut property_path = Vec::new();
    let mut parent = node.parent;
    while let Some(id) = parent {
        let ancestor = &syntax.parsed.nodes[id.get() as usize];
        match &ancestor.kind {
            TolerantSyntaxNodeKind::Property { name } => property_path.push(name.clone()),
            TolerantSyntaxNodeKind::Declaration { keyword, name } => {
                if declaration_keyword.is_none() {
                    declaration_keyword = Some(keyword.clone());
                    declaration_name = name.clone();
                }
                declaration_path.push(SqlIslandOwner {
                    keyword: keyword.clone(),
                    name: name.clone().unwrap_or_else(|| keyword.clone()),
                });
            }
            _ => {}
        }
        parent = ancestor.parent;
    }
    property_path.reverse();
    declaration_path.reverse();
    let channel_mode = syntax.parsed.nodes.iter().find_map(|candidate| {
        let TolerantSyntaxNodeKind::ChannelMode {
            mode,
            expression_span: Some(expression_span),
            ..
        } = &candidate.kind
        else {
            return None;
        };
        (expression_span.range.start <= node.span.range.start
            && node.span.range.end <= expression_span.range.end)
            .then(|| mode.clone())
    });
    let occurrence = syntax
        .parsed
        .nodes
        .iter()
        .take(node.id.get() as usize)
        .filter(|candidate| {
            candidate.parent == node.parent
                && matches!(
                    &candidate.kind,
                    TolerantSyntaxNodeKind::SqlIsland {
                        site: candidate_site,
                        ..
                    } if candidate_site == site
                )
        })
        .count();
    let mut identity = Sha256::new();
    identity.update(b"avenger-sql-island-identity-v1\0");
    identity.update(site.manifest_name().as_bytes());
    for owner in &declaration_path {
        identity.update(b"\0owner\0");
        identity.update(owner.keyword.as_bytes());
        identity.update(b"\0");
        identity.update(owner.name.as_bytes());
    }
    for property in &property_path {
        identity.update(b"\0property\0");
        identity.update(property.as_bytes());
    }
    identity.update(b"\0occurrence\0");
    identity.update((occurrence as u64).to_le_bytes());
    Some(SqlIslandDescriptor {
        identity: format!("{:x}", identity.finalize()),
        authored_fingerprint: fingerprint.clone(),
        site: *site,
        span: node.span,
        declaration_keyword,
        declaration_name,
        declaration_path,
        property_path,
        channel_mode,
    })
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
    let header_end = declaration_header_end(&tokens);
    let header = &tokens[..header_end];
    let binder = top_level_word_position(header, "as").and_then(|index| header.get(index + 1));
    let recovered =
        fallback_name.and_then(|name| header.iter().rfind(|token| token.word() == Some(name)));
    let fallback = recovered.or_else(|| match keyword {
        "define" => header.get(keyword_index + 2),
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
            | "adjust"
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
            top_level_word_position(header, "as").unwrap_or(header.len())
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

fn declaration_header_end(tokens: &[SigToken<'_>]) -> usize {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.token {
            Some(Token::LParen | Token::LBracket) => depth += 1,
            Some(Token::RParen | Token::RBracket) => depth = depth.saturating_sub(1),
            Some(Token::LBrace | Token::SemiColon) if depth == 0 => return index,
            _ => {}
        }
    }
    tokens.len()
}

fn top_level_word_position(tokens: &[SigToken<'_>], expected: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        match token.token {
            Some(Token::LParen | Token::LBracket | Token::LBrace) => depth += 1,
            Some(Token::RParen | Token::RBracket | Token::RBrace) => {
                depth = depth.saturating_sub(1)
            }
            Some(Token::Word(word)) if depth == 0 && word.value.eq_ignore_ascii_case(expected) => {
                return Some(index);
            }
            _ => {}
        }
    }
    None
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

fn projection_alias_spans(
    syntax: &SyntaxAnalysis,
    authored: SourceSpan,
    expected: &BTreeSet<String>,
) -> Vec<(String, SourceSpan)> {
    let declaration = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. }))
        .filter(|node| {
            node.span.range.start <= authored.range.start
                && authored.range.end <= node.span.range.end
                || authored.range.start <= node.span.range.start
                    && node.span.range.end <= authored.range.end
        })
        .min_by_key(|node| node.span.range.len());
    let Some(declaration) = declaration else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for property in syntax.parsed.nodes.iter().filter(|node| {
        matches!(node.kind, TolerantSyntaxNodeKind::Property { .. })
            && parent_declaration(&syntax.parsed.nodes, node.parent) == Some(declaration.id)
    }) {
        let tokens = significant_tokens(syntax, Some(property.span));
        let Some(colon) = tokens
            .iter()
            .position(|token| matches!(token.token, Some(Token::Colon)))
        else {
            continue;
        };
        let mut depth = 0usize;
        let mut index = colon + 1;
        while index < tokens.len() {
            match tokens[index].token {
                Some(Token::LParen | Token::LBracket | Token::LBrace) => depth += 1,
                Some(Token::RParen | Token::RBracket | Token::RBrace) => {
                    depth = depth.saturating_sub(1);
                }
                Some(Token::SemiColon) if depth == 0 => break,
                Some(Token::Word(word)) if depth == 0 && word.value.eq_ignore_ascii_case("as") => {
                    if let Some(alias) = tokens.get(index + 1)
                        && let Some(Token::Word(word)) = alias.token
                        && word.quote_style.is_none()
                        && expected.contains(&word.value)
                    {
                        output.push((word.value.clone(), alias.span));
                    }
                    index += 1;
                }
                _ => {}
            }
            index += 1;
        }
    }
    output.sort_by_key(|(_, span)| span.range.start);
    output.dedup_by_key(|(_, span)| *span);
    output
}

fn output_identity(handle: &ResolvedOutputHandle) -> String {
    format!("output:{}:{}", handle.producer, handle.name)
}

fn collect_output_references<'a>(
    declarations: &'a [ResolvedDeclaration],
    output: &mut Vec<(&'a ResolvedDeclaration, Vec<String>, ResolvedOutputHandle)>,
) {
    fn collect_value<'a>(
        declaration: &'a ResolvedDeclaration,
        value: &ResolvedValue,
        output: &mut Vec<(&'a ResolvedDeclaration, Vec<String>, ResolvedOutputHandle)>,
    ) {
        let mut collect_references = |references: &[avenger_lang_core::ResolvedSqlReference]| {
            output.extend(references.iter().filter_map(|reference| {
                let ResolvedTarget::Output(handle) = &reference.target else {
                    return None;
                };
                Some((declaration, reference.authored_path.clone(), handle.clone()))
            }));
        };
        match value {
            ResolvedValue::Expression(expression) => collect_references(&expression.references),
            ResolvedValue::Projection(projection) => {
                for item in &projection.items {
                    if let Some(expression) = &item.expression {
                        collect_references(&expression.references);
                    }
                }
            }
            ResolvedValue::Query(query) => collect_references(&query.references),
            ResolvedValue::Array(values) | ResolvedValue::Call { args: values, .. } => {
                for value in values {
                    collect_value(declaration, value, output);
                }
            }
            ResolvedValue::Channel {
                expression: value, ..
            }
            | ResolvedValue::Pattern(value) => {
                collect_value(declaration, value, output);
            }
            ResolvedValue::ChannelValue(channel) => {
                collect_value(declaration, &channel.head.expression, output);
                if let Some(otherwise) = &channel.otherwise {
                    collect_value(declaration, &otherwise.expression, output);
                }
                for condition in &channel.conditions {
                    collect_value(declaration, &condition.predicate, output);
                    collect_value(declaration, &condition.branch.expression, output);
                }
                for value in channel.configuration.values() {
                    collect_value(declaration, value, output);
                }
            }
            ResolvedValue::Object {
                head, properties, ..
            } => {
                if let Some(head) = head {
                    collect_value(declaration, head, output);
                }
                for value in properties.values() {
                    collect_value(declaration, value, output);
                }
            }
            ResolvedValue::String(_)
            | ResolvedValue::Relation(_)
            | ResolvedValue::Number(_)
            | ResolvedValue::Boolean(_)
            | ResolvedValue::Null
            | ResolvedValue::Atom(_)
            | ResolvedValue::Column(_)
            | ResolvedValue::Binding(_)
            | ResolvedValue::Reference(_)
            | ResolvedValue::Dimension(_)
            | ResolvedValue::Environment(_)
            | ResolvedValue::None
            | ResolvedValue::DefinitionArgument(_)
            | ResolvedValue::Invalid => {}
        }
    }

    for declaration in declarations {
        for value in declaration.properties.values() {
            collect_value(declaration, value, output);
        }
        collect_output_references(&declaration.children, output);
    }
}

fn sql_path_spans(
    syntax: &SyntaxAnalysis,
    authored: SourceSpan,
    path: &[String],
) -> Vec<SourceSpan> {
    if path.is_empty() {
        return Vec::new();
    }
    let within = SourceSpan {
        source: syntax.parsed.tokens.source(),
        range: authored.range,
    };
    let tokens = significant_tokens(syntax, Some(within));
    let mut output = Vec::new();
    for start in 0..tokens.len() {
        let Some(first) = tokens[start].word() else {
            continue;
        };
        if first != path[0] {
            continue;
        }
        let mut cursor = start + 1;
        let mut matched = true;
        for component in &path[1..] {
            if !tokens
                .get(cursor)
                .is_some_and(|token| matches!(token.token, Some(Token::Period)))
                || tokens.get(cursor + 1).and_then(SigToken::word) != Some(component.as_str())
            {
                matched = false;
                break;
            }
            cursor += 2;
        }
        if matched {
            let end = if path.len() == 1 {
                tokens[start].span.range.end
            } else {
                tokens[cursor - 1].span.range.end
            };
            output.push(SourceSpan {
                source: tokens[start].span.source,
                range: ByteSpan {
                    start: tokens[start].span.range.start,
                    end,
                },
            });
        }
    }
    output
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

pub(crate) fn first_word_span(
    syntax: &SyntaxAnalysis,
    within: SourceSpan,
    name: &str,
) -> Option<SourceSpan> {
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
        if token.word() == Some("table")
            && tokens
                .get(index + 1)
                .is_some_and(|token| matches!(token.token, Some(Token::Colon)))
        {
            let mut next = index + 2;
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
                value_kind: IndexedValueKind::Table,
            });
            index = next;
            continue;
        }
        if token.word().and_then(StateActionVerb::parse).is_some() {
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
        let tokens = significant_tokens(syntax, Some(node.span));
        let header_end = tokens
            .get(declaration_header_end(&tokens))
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
    _project_root: &'a Path,
    known_sources: &'a [SourceOrigin],
    registry: &'a NativeSchemaSnapshot,
    syntax: &'a BTreeMap<SourceOrigin, SyntaxAnalysis>,
    index: &'a WorkspaceSemanticIndex,
    semantic_roots: &'a BTreeMap<String, RootAnalysis>,
    dataset_contexts: &'a BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    completion_cache: &'a crate::sql_intelligence::SqlCompletionCache,
}

#[derive(Clone, Debug)]
enum StructuralCursorState {
    Nothing,
    NameInvention,
    ImportClause,
    ImportMember(ScannedImport),
    ImportPath(String),
    ImportSource,
    ImportTail,
    TokenInsertion(&'static [&'static str]),
    StructFieldName,
    ParamBinder,
    FieldNullable,
    ParamHeader,
    PhysicalType,
    FixedHeader(&'static [&'static str]),
    ActionModifier(&'static [&'static str]),
    ActionTarget(StateActionVerb),
    ActionBody(&'static [&'static str]),
    DeclarationKind {
        namespace: NativeKindNamespace,
        typed: String,
    },
    DeclarationTail,
    PropertyValue(String),
    PropertyName,
    Declaration,
}

fn structural_cursor_state(
    syntax: &SyntaxAnalysis,
    cursor: usize,
    prefix: &str,
    action_target_kind: Option<IndexedValueKind>,
) -> StructuralCursorState {
    let text = syntax.parsed.tokens.text();
    if let Some(state) = version_header_state(syntax, cursor) {
        state
    } else if let Some(state) = export_cursor_state(syntax, cursor) {
        state
    } else if name_invention_context(syntax, cursor) {
        StructuralCursorState::NameInvention
    } else if let Some(import) = scan_imports(syntax).into_iter().find(|import| {
        import
            .member_span
            .is_some_and(|span| span.range.start <= cursor && cursor <= span.range.end)
    }) {
        StructuralCursorState::ImportMember(import)
    } else if let Some(import_prefix) = import_prefix(text, cursor) {
        StructuralCursorState::ImportPath(import_prefix.to_owned())
    } else if let Some(state) = import_cursor_state(syntax, cursor) {
        state
    } else if structural_noncode_context(syntax, cursor) {
        StructuralCursorState::Nothing
    } else if struct_field_name_context(syntax, cursor) {
        StructuralCursorState::StructFieldName
    } else if declaration_binder_context(syntax, cursor) {
        StructuralCursorState::ParamBinder
    } else if field_name_invention_context(syntax, cursor) {
        StructuralCursorState::NameInvention
    } else if field_nullable_context(syntax, cursor) {
        StructuralCursorState::FieldNullable
    } else if param_initializer_context(syntax, cursor) {
        StructuralCursorState::ParamHeader
    } else if physical_type_header_context(syntax, cursor) {
        StructuralCursorState::PhysicalType
    } else if let Some(values) = fixed_header_candidates(syntax, cursor) {
        StructuralCursorState::FixedHeader(values)
    } else if let Some(values) = action_modifier_candidates(syntax, cursor, action_target_kind) {
        StructuralCursorState::ActionModifier(values)
    } else if let Some(verb) = action_target_context(syntax, cursor) {
        StructuralCursorState::ActionTarget(verb)
    } else if let Some(values) = action_body_candidates(syntax, cursor, action_target_kind) {
        StructuralCursorState::ActionBody(values)
    } else if let Some((namespace, typed, complete)) = declaration_kind_context(syntax, cursor) {
        if complete {
            StructuralCursorState::DeclarationTail
        } else {
            StructuralCursorState::DeclarationKind { namespace, typed }
        }
    } else if let Some(property) = property_value_context(syntax, cursor)
        .map(str::to_owned)
        .or_else(|| incomplete_property_value_context(syntax, cursor))
    {
        StructuralCursorState::PropertyValue(property)
    } else if is_property_name_context(syntax, cursor) {
        StructuralCursorState::PropertyName
    } else if prefix.starts_with('$') {
        // `$` is a namespace entry character, never a reason to fall back to
        // unrelated declaration or property candidates.
        StructuralCursorState::Nothing
    } else if declaration_start_context(syntax, cursor) {
        StructuralCursorState::Declaration
    } else {
        StructuralCursorState::Nothing
    }
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
        completion_cache: &'a crate::sql_intelligence::SqlCompletionCache,
    ) -> Self {
        Self {
            generation,
            _project_root: project_root,
            known_sources,
            registry,
            syntax,
            index,
            semantic_roots,
            dataset_contexts,
            completion_cache,
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
        let action_target = state_action_target(syntax, cursor);
        let action_target_kind = action_target
            .as_deref()
            .and_then(|target| self.state_target_kind(&request.source, cursor, target));
        let structural_state = structural_cursor_state(syntax, cursor, prefix, action_target_kind);
        let structural_token_insertion =
            matches!(&structural_state, StructuralCursorState::TokenInsertion(_));
        let state_action_block = enclosing_property(syntax, cursor)
            .and_then(|node| match &node.kind {
                TolerantSyntaxNodeKind::Property { name } => Some(name.as_str()),
                _ => None,
            })
            .and_then(|name| {
                schema_owner_symbol(self.index, self.registry, &request.source, cursor)
                    .and_then(|owner| schema_for_symbol(self.registry, owner, self.index))
                    .and_then(|schema| property_schema(schema, name))
            })
            .is_some_and(|property| matches!(property.shape, ValueShape::StateActionBlock));
        let structural_selection_binding = match &structural_state {
            StructuralCursorState::PropertyValue(property_name) => {
                schema_owner_symbol(self.index, self.registry, &request.source, cursor)
                    .and_then(|owner| schema_for_symbol(self.registry, owner, self.index))
                    .and_then(|schema| property_schema(schema, property_name))
                    .is_some_and(|property| matches!(property.shape, ValueShape::SelectionBinding))
            }
            _ => false,
        };

        let sql = if structural_selection_binding {
            None
        } else {
            crate::sql_intelligence::complete_sql(
                request,
                syntax,
                self.registry,
                self.index,
                self.semantic_roots,
                self.dataset_contexts,
                &options.invocation,
                options.snippets,
                self.completion_cache,
                cancellation,
            )
        };
        let sql_domain = sql.is_some();
        let sql_incomplete = sql.as_ref().is_some_and(|result| result.is_incomplete);
        if let Some(sql) = sql {
            items.extend(sql.items);
        } else {
            match structural_state {
                StructuralCursorState::Nothing | StructuralCursorState::NameInvention => {}
                StructuralCursorState::ImportClause => {
                    for (label, insert_text, detail) in [
                        (
                            "named import",
                            "{ ${1:name} } from '${2:./module.avenger}';$0",
                            "import selected module exports",
                        ),
                        (
                            "namespace import",
                            "* as ${1:namespace} from '${2:./module.avenger}';$0",
                            "import all public exports under one namespace",
                        ),
                    ] {
                        let mut candidate = item(
                            label.to_owned(),
                            replacement,
                            if options.snippets {
                                insert_text.to_owned()
                            } else if label == "named import" {
                                "{ } from ''".to_owned()
                            } else {
                                "* as namespace from ''".to_owned()
                            },
                            CompletionKind::Snippet,
                            Some(detail.to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        );
                        candidate.insert_text_format = if options.snippets {
                            CompletionTextFormat::Snippet
                        } else {
                            CompletionTextFormat::PlainText
                        };
                        candidate.validity = CompletionValidity::Scaffold;
                        items.push(candidate);
                    }
                }
                StructuralCursorState::ImportMember(import) => self.complete_import_members(
                    &request.source,
                    &import,
                    prefix,
                    replacement,
                    &mut items,
                ),
                StructuralCursorState::ImportPath(import_prefix) => {
                    self.complete_imports(&request.source, &import_prefix, replacement, &mut items);
                }
                StructuralCursorState::ImportSource => {
                    self.complete_import_source_starts(
                        &request.source,
                        replacement,
                        options.snippets,
                        &mut items,
                    );
                }
                StructuralCursorState::ImportTail => {
                    for (label, insert_text, detail) in [
                        (";", ";", "finish the import"),
                        (
                            "sha256",
                            if options.snippets {
                                "sha256 '${1:digest}';$0"
                            } else {
                                "sha256 ''"
                            },
                            "pin the imported module bytes",
                        ),
                    ] {
                        let mut candidate = item(
                            label.to_owned(),
                            replacement,
                            insert_text.to_owned(),
                            CompletionKind::Keyword,
                            Some(detail.to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            if label == ";" { "00" } else { "10" },
                        );
                        if label == "sha256" && options.snippets {
                            candidate.insert_text_format = CompletionTextFormat::Snippet;
                            candidate.validity = CompletionValidity::Scaffold;
                        }
                        items.push(candidate);
                    }
                }
                StructuralCursorState::TokenInsertion(values) => {
                    let insertion = SourceSpan::empty(replacement.source, cursor);
                    for value in values {
                        items.push(item(
                            (*value).to_owned(),
                            insertion,
                            (*value).to_owned(),
                            CompletionKind::Keyword,
                            Some("required structural delimiter".to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        ));
                    }
                }
                StructuralCursorState::StructFieldName => {
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
                        candidate.validity = CompletionValidity::Scaffold;
                        items.push(candidate);
                    }
                }
                StructuralCursorState::ParamBinder => {
                    if candidate_matches("as", prefix) {
                        items.push(item(
                            "as".to_owned(),
                            replacement,
                            "as".to_owned(),
                            CompletionKind::Keyword,
                            Some("declaration binder".to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        ));
                    }
                }
                StructuralCursorState::FieldNullable => {
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
                }
                StructuralCursorState::ParamHeader => {
                    for (label, detail) in [
                        ("CAST", "SQL cast expression"),
                        ("NULL", "SQL null literal; cast it to infer a concrete type"),
                        ("true", "SQL boolean literal"),
                        ("false", "SQL boolean literal"),
                    ] {
                        if candidate_matches(label, prefix) {
                            items.push(item(
                                label.to_owned(),
                                replacement,
                                label.to_owned(),
                                CompletionKind::Keyword,
                                Some(detail.to_owned()),
                                None,
                                CompletionOrigin::Syntax,
                                false,
                                "00",
                            ));
                        }
                    }
                }
                StructuralCursorState::PhysicalType => {
                    complete_physical_types(prefix, replacement, &mut items);
                }
                StructuralCursorState::FixedHeader(values) => {
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
                }
                StructuralCursorState::ActionModifier(values) => {
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
                }
                StructuralCursorState::ActionTarget(verb) => self.complete_state_targets(
                    verb,
                    prefix,
                    replacement,
                    &request.source,
                    cursor,
                    &mut items,
                ),
                StructuralCursorState::ActionBody(values) => {
                    for value in values {
                        if !candidate_matches(value, prefix) {
                            continue;
                        }
                        let declaration = matches!(*value, "row" | "key" | "fields" | "clause");
                        let insert_text = if options.snippets {
                            match *value {
                                "row" | "key" | "fields" | "clause" => {
                                    format!("{value} {{\n  $0\n}}")
                                }
                                _ => format!("{value}: $0;"),
                            }
                        } else if declaration {
                            format!("{value} {{ }}")
                        } else {
                            format!("{value}: ")
                        };
                        let mut candidate = item(
                            (*value).to_owned(),
                            replacement,
                            insert_text,
                            if declaration {
                                CompletionKind::Declaration
                            } else {
                                CompletionKind::Property
                            },
                            Some("state action payload member".to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        );
                        if options.snippets {
                            candidate.insert_text_format = CompletionTextFormat::Snippet;
                            candidate.validity = CompletionValidity::Scaffold;
                        }
                        items.push(candidate);
                    }
                }
                StructuralCursorState::DeclarationKind { namespace, typed } => self
                    .complete_native_kinds(
                        namespace,
                        &typed,
                        replacement,
                        &request.source,
                        cursor,
                        &mut items,
                    ),
                StructuralCursorState::DeclarationTail => {
                    for (label, insert_text, detail, validity) in [
                        (
                            "as",
                            "as ",
                            "bind a source name",
                            CompletionValidity::Strict,
                        ),
                        (
                            "body",
                            if options.snippets {
                                "{\n  $0\n}"
                            } else {
                                "{ }"
                            },
                            "anonymous declaration body",
                            CompletionValidity::Scaffold,
                        ),
                    ] {
                        if !candidate_matches(label, prefix) {
                            continue;
                        }
                        let mut candidate = item(
                            label.to_owned(),
                            replacement,
                            insert_text.to_owned(),
                            CompletionKind::Keyword,
                            Some(detail.to_owned()),
                            None,
                            CompletionOrigin::Syntax,
                            false,
                            if label == "as" { "00" } else { "10" },
                        );
                        candidate.validity = validity;
                        if options.snippets && validity == CompletionValidity::Scaffold {
                            candidate.insert_text_format = CompletionTextFormat::Snippet;
                        }
                        items.push(candidate);
                    }
                }
                StructuralCursorState::PropertyValue(property) => self.complete_property_value(
                    &property,
                    prefix,
                    replacement,
                    cursor,
                    &request.source,
                    options.snippets,
                    &mut items,
                ),
                StructuralCursorState::PropertyName => {
                    let nested_property = enclosing_property(syntax, cursor).is_some();
                    let legend_overlay = inside_legend_overlay_block(syntax, cursor);
                    let event_actions_started = event_actions_started(syntax, cursor);
                    if state_action_block {
                        self.complete_state_action_heads(
                            prefix,
                            replacement,
                            options.snippets,
                            &mut items,
                        );
                    } else if legend_overlay {
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
                            candidate.validity = CompletionValidity::Scaffold;
                            items.push(candidate);
                        }
                    } else if !event_actions_started {
                        self.complete_properties(
                            syntax,
                            &request.source,
                            cursor,
                            prefix,
                            replacement,
                            &mut items,
                        );
                    }
                    if !nested_property && !legend_overlay && !state_action_block {
                        self.complete_declarations(
                            &request.source,
                            cursor,
                            prefix,
                            replacement,
                            &options,
                            &mut items,
                        );
                    }
                }
                StructuralCursorState::Declaration => {
                    if state_action_block {
                        self.complete_state_action_heads(
                            prefix,
                            replacement,
                            options.snippets,
                            &mut items,
                        );
                    } else {
                        let payload_owner = owner_symbol(self.index, &request.source, cursor)
                            .map(|owner| owner.keyword.as_str());
                        if matches!(payload_owner, Some("row" | "key" | "fields" | "clause")) {
                            self.complete_properties(
                                syntax,
                                &request.source,
                                cursor,
                                prefix,
                                replacement,
                                &mut items,
                            );
                        }
                        if !matches!(payload_owner, Some("row" | "key" | "fields")) {
                            self.complete_declarations(
                                &request.source,
                                cursor,
                                prefix,
                                replacement,
                                &options,
                                &mut items,
                            );
                        }
                    }
                }
            }
        }

        cancellation
            .check()
            .map_err(|_| AnalysisQueryError::Cancelled)?;
        if !sql_domain {
            if prefix.starts_with('$') {
                items.retain(|item| {
                    matches!(
                        item.semantic_kind,
                        CompletionSemanticKind::ScalarParam
                            | CompletionSemanticKind::StoreParam
                            | CompletionSemanticKind::SelectionParam
                    )
                });
            }
            annotate_structural_usage_prevalence(&mut items, self.index);
            rank_and_deduplicate(
                &mut items,
                if structural_token_insertion {
                    ""
                } else {
                    prefix
                },
                &options.invocation,
            );
        }
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
        let output_start = output.len();
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
        if namespace == NativeKindNamespace::Adjust && candidate_matches("expr", typed) {
            output.push(item(
                "expr".to_owned(),
                replacement,
                "expr".to_owned(),
                CompletionKind::Declaration,
                Some("language-owned adjustment".to_owned()),
                Some("Assign mark channels directly with item-frame SQL expressions.".to_owned()),
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
                Some("adjust") => NativeKindNamespace::Adjust,
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
        for item in &mut output[output_start..] {
            item.semantic_kind = CompletionSemanticKind::NativeKind;
            item.semantic_identity = format!("NativeKind:{namespace:?}:{}", item.label);
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
        let authored = if owner.keyword == "when" {
            authored_declaration_properties(syntax, owner.scope_span)
        } else {
            authored_properties(syntax, owner.scope_span, cursor)
        };
        if matches!(owner.keyword.as_str(), "row" | "key" | "fields")
            && let Some(fields) = self.action_store_fields(origin, cursor)
        {
            for (name, data_type, primary_key) in fields {
                let valid_member = match owner.keyword.as_str() {
                    "row" => true,
                    "key" => primary_key,
                    "fields" => !primary_key,
                    _ => false,
                };
                if valid_member && !authored.contains(&name) && candidate_matches(&name, prefix) {
                    output.push(item(
                        name.clone(),
                        replacement,
                        format!("{name}: "),
                        CompletionKind::Property,
                        Some(format!("store field: {data_type}")),
                        Some(format!(
                            "Destination-typed `{data_type}` field in this `{}` payload.",
                            owner.keyword
                        )),
                        CompletionOrigin::DatasetSchema,
                        false,
                        "00",
                    ));
                }
            }
            return;
        }
        if owner.keyword == "clause" {
            if !authored.contains("id") && candidate_matches("id", prefix) {
                output.push(item(
                    "id".to_owned(),
                    replacement,
                    "id: ".to_owned(),
                    CompletionKind::Property,
                    Some("stable selection clause identity".to_owned()),
                    Some("Required non-empty UTF-8 identity for this selection clause.".to_owned()),
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
            return;
        }
        if owner.keyword == "when" {
            if !authored.contains("predicate") && candidate_matches("predicate", prefix) {
                output.push(item(
                    "predicate".to_owned(),
                    replacement,
                    "predicate: ".to_owned(),
                    CompletionKind::Property,
                    Some("conditional channel predicate".to_owned()),
                    Some("Boolean SQL expression evaluated for this ordered branch.".to_owned()),
                    CompletionOrigin::Syntax,
                    false,
                    "00",
                ));
            }
            if !authored.contains("encoded") && !authored.contains("direct") {
                for mode in ["encoded", "direct"] {
                    if candidate_matches(mode, prefix) {
                        output.push(item(
                            mode.to_owned(),
                            replacement,
                            format!("{mode}: "),
                            CompletionKind::Property,
                            Some("conditional channel mode".to_owned()),
                            Some(channel_mode_docs(mode).to_owned()),
                            CompletionOrigin::Syntax,
                            false,
                            "01",
                        ));
                    }
                }
            }
            return;
        }
        if owner.keyword == "slot"
            && let Some(shape) = declaration_header_role(syntax, owner)
        {
            let properties: &[(&str, &str)] = match shape.as_str() {
                "enum" => &[
                    ("default", "Optional default value."),
                    ("values", "Closed enum value inventory."),
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
        if property_path.last() == Some(&"otherwise") {
            if !authored.contains("encoded") && !authored.contains("direct") {
                for mode in ["encoded", "direct"] {
                    if candidate_matches(mode, prefix) {
                        output.push(item(
                            mode.to_owned(),
                            replacement,
                            format!("{mode}: "),
                            CompletionKind::Property,
                            Some("conditional fallback mode".to_owned()),
                            Some(channel_mode_docs(mode).to_owned()),
                            CompletionOrigin::Syntax,
                            false,
                            "00",
                        ));
                    }
                }
            }
            return;
        }
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
        let owner_schema = schema_for_symbol(self.registry, owner, self.index);
        if let Some(schema) = owner_schema {
            if property_path.len() == 1
                && schema.channels.contains_key(property_path[0])
                && enclosing_property(syntax, cursor).is_some()
            {
                let encoded_effective = enclosing_property(syntax, cursor)
                    .is_none_or(|property| channel_body_has_effective_encoded(syntax, property));
                for (name, insertion, docs) in [
                    (
                        "scale",
                        "scale: { }",
                        "Scale configuration for this channel.",
                    ),
                    ("axis", "axis: { }", "Axis configuration for this channel."),
                    (
                        "legend",
                        "legend: { }",
                        "Legend configuration for this channel.",
                    ),
                    (
                        "domain_contribution",
                        "domain_contribution: ",
                        "Whether this channel contributes to automatic scale-domain inference (`infer` or `exclude`).",
                    ),
                ] {
                    if !encoded_effective {
                        continue;
                    }
                    if !authored.contains(name) && candidate_matches(name, prefix) {
                        output.push(item(
                            name.to_owned(),
                            replacement,
                            insertion.to_owned(),
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
            if authored.contains(name)
                || owner_schema.is_some_and(|schema| {
                    schema.properties.contains_key(name) || schema.channels.contains_key(name)
                })
                || !candidate_matches(name, prefix)
            {
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
        snippets: bool,
        output: &mut Vec<CompletionItem>,
    ) {
        if property_name == "table" {
            let typed = self
                .syntax
                .get(origin)
                .map(|syntax| relation_path_prefix(syntax, cursor))
                .unwrap_or_default();
            self.complete_relation_paths(&typed, replacement, origin, output);
            return;
        }
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
        let owner_schema = schema_owner_symbol(self.index, self.registry, origin, cursor)
            .and_then(|owner| schema_for_symbol(self.registry, owner, self.index));
        if let Some(channel) = owner_schema.and_then(|schema| schema.channels.get(property_name)) {
            let has_head_mode = self.syntax.get(origin).is_some_and(|syntax| {
                enclosing_property(syntax, cursor)
                    .is_some_and(|property| channel_property_has_head_mode(syntax, property))
            });
            if !has_head_mode {
                complete_channel_value(channel, prefix, replacement, output);
            }
            if prefix.starts_with('$') {
                self.complete_bindings(
                    prefix,
                    replacement,
                    origin,
                    cursor,
                    &[IndexedValueKind::Scalar],
                    output,
                );
            }
            return;
        }
        let property = owner_schema.and_then(|schema| property_schema(schema, property_name));
        if let Some(property) = property {
            complete_shape(property.shape, prefix, replacement, snippets, output);
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
        if prefix.starts_with('$') {
            let allowed = property
                .map(|property| binding_kinds(property.shape))
                .unwrap_or_else(|| match property_name {
                    "data" => vec![IndexedValueKind::Table],
                    _ => Vec::new(),
                });
            self.complete_bindings(prefix, replacement, origin, cursor, &allowed, output);
        }
    }

    fn complete_relation_paths(
        &self,
        typed: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        output: &mut Vec<CompletionItem>,
    ) {
        for ((reference_origin, _), relation) in &self.index.public_references {
            if reference_origin != origin || relation.value_kind != IndexedValueKind::Table {
                continue;
            }
            if !candidate_matches(&relation.path, typed) {
                continue;
            }
            output.push(item(
                relation.path.clone(),
                replacement,
                relation.path.clone(),
                CompletionKind::Table,
                Some(relation.detail.clone()),
                None,
                CompletionOrigin::LexicalScope,
                false,
                "00",
            ));
        }
    }

    fn complete_bindings(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        allowed: &[IndexedValueKind],
        output: &mut Vec<CompletionItem>,
    ) {
        if allowed.is_empty() {
            return;
        }
        let typed = prefix.strip_prefix('$').unwrap_or(prefix);
        let document = self.index.documents.get(origin);
        if let Some(document) = document {
            for symbol in &document.symbols {
                if !matches!(
                    symbol.value_kind,
                    IndexedValueKind::Scalar
                        | IndexedValueKind::Table
                        | IndexedValueKind::Selection
                ) || !allowed.contains(&symbol.value_kind)
                    || !scope_visible(document, symbol, cursor)
                    || !candidate_matches(&symbol.name, typed)
                {
                    continue;
                }
                let label = format!("${}", symbol.name);
                let bucket = format!("00:{:020}", symbol.selection_span.range.start);
                let mut completion = item(
                    label.clone(),
                    replacement,
                    label,
                    CompletionKind::Variable,
                    symbol.detail.clone(),
                    symbol.documentation.clone(),
                    CompletionOrigin::LexicalScope,
                    false,
                    &bucket,
                );
                completion.semantic_kind = match symbol.value_kind {
                    IndexedValueKind::Scalar => CompletionSemanticKind::ScalarParam,
                    IndexedValueKind::Table => CompletionSemanticKind::StoreParam,
                    IndexedValueKind::Selection => CompletionSemanticKind::SelectionParam,
                    _ => unreachable!(),
                };
                completion.semantic_identity =
                    format!("{:?}:{}", completion.semantic_kind, symbol.identity);
                output.push(completion);
            }
        }
        for binding in self.index.public_bindings.values() {
            if !matches!(
                binding.value_kind,
                IndexedValueKind::Scalar | IndexedValueKind::Table | IndexedValueKind::Selection
            ) || !allowed.contains(&binding.value_kind)
                || !candidate_matches(&binding.path, typed)
            {
                continue;
            }
            let label = format!("${}", binding.path);
            let mut completion = item(
                label.clone(),
                replacement,
                label,
                CompletionKind::Variable,
                Some(binding.detail.clone()),
                None,
                CompletionOrigin::LexicalScope,
                false,
                "10",
            );
            completion.semantic_kind = match binding.value_kind {
                IndexedValueKind::Scalar => CompletionSemanticKind::ScalarParam,
                IndexedValueKind::Table => CompletionSemanticKind::StoreParam,
                IndexedValueKind::Selection => CompletionSemanticKind::SelectionParam,
                _ => unreachable!(),
            };
            completion.semantic_identity =
                format!("{:?}:{}", completion.semantic_kind, binding.path);
            output.push(completion);
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
        verb: StateActionVerb,
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
            if !action_target_kind_allowed(verb, symbol.value_kind)
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
            if action_target_kind_allowed(verb, binding.value_kind)
                && candidate_matches(&binding.path, prefix)
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
        if verb == StateActionVerb::Set && candidate_matches("cursor", prefix) {
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

    fn action_store_fields(
        &self,
        origin: &SourceOrigin,
        cursor: usize,
    ) -> Option<Vec<(String, String, bool)>> {
        self.semantic_roots.values().find_map(|root| {
            let analysis = root.result.as_ref().ok()?;
            let project = analysis.resolved_module_graph.as_deref()?;
            let mut declarations = BTreeMap::new();
            for module in project.source_modules.values() {
                collect_resolved_declarations(&module.roots, &mut declarations);
            }
            let action = declarations
                .values()
                .filter(|declaration| is_state_action_keyword(&declaration.keyword))
                .filter_map(|declaration| {
                    let authored = project.expansion_source_map.authored_span(declaration.span);
                    let source = project.sources.get(authored.source)?;
                    (source.origin.canonical_uri() == origin.canonical_uri()
                        && authored.range.start <= cursor
                        && cursor <= authored.range.end)
                        .then_some((authored.range.len(), *declaration))
                })
                .min_by_key(|(span_len, _)| *span_len)?
                .1;
            let ResolvedTarget::Store(id) = &action.state_lvalue.as_ref()?.target else {
                return None;
            };
            let store = project.stores.get(id)?;
            Some(
                store
                    .fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            field.data_type.to_string(),
                            store.primary_key.contains(&field.name),
                        )
                    })
                    .collect(),
            )
        })
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
        if matches!(shape, ValueShape::SelectionBinding) {
            self.complete_typed_state_references(
                prefix,
                replacement,
                origin,
                cursor,
                IndexedValueKind::Selection,
                output,
            );
        }
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

    fn complete_typed_state_references(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        origin: &SourceOrigin,
        cursor: usize,
        expected: IndexedValueKind,
        output: &mut Vec<CompletionItem>,
    ) {
        let Some(document) = self.index.documents.get(origin) else {
            return;
        };
        let owner_span = schema_owner_symbol(self.index, self.registry, origin, cursor)
            .map(|owner| owner.declaration_span.range);
        for symbol in &document.symbols {
            if symbol.value_kind != expected
                || symbol.selection_span.range.start >= cursor
                || !scope_visible(document, symbol, cursor)
                || owner_span.is_some_and(|owner| {
                    owner.start <= symbol.selection_span.range.start
                        && symbol.selection_span.range.end <= owner.end
                })
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
            if binding.value_kind == expected && candidate_matches(&binding.path, prefix) {
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
    }

    fn complete_declarations(
        &self,
        origin: &SourceOrigin,
        cursor: usize,
        prefix: &str,
        replacement: SourceSpan,
        options: &CompletionOptions,
        output: &mut Vec<CompletionItem>,
    ) {
        let parent = owner_symbol(self.index, origin, cursor);
        if parent.is_some_and(|parent| parent.keyword == "on") {
            self.complete_state_action_heads(prefix, replacement, options.snippets, output);
            return;
        }
        let keywords: Vec<&str> = if let Some(parent) = parent {
            allowed_child_declarations(&parent.keyword).collect()
        } else {
            let has_prior_module_item = self.index.documents.get(origin).is_some_and(|document| {
                document.symbols.iter().any(|symbol| {
                    symbol.parent.is_none()
                        && symbol.declaration_span.range.start < cursor
                        && matches!(
                            symbol.keyword.as_str(),
                            "chart" | "define" | "catalog" | "schema" | "table"
                        )
                })
            });
            let mut keywords = vec!["export", "chart", "define", "catalog", "schema", "table"];
            if !has_prior_module_item {
                keywords.insert(0, "import");
            }
            keywords
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

    fn complete_state_action_heads(
        &self,
        prefix: &str,
        replacement: SourceSpan,
        snippets: bool,
        output: &mut Vec<CompletionItem>,
    ) {
        for verb in StateActionVerb::ALL {
            let label = verb.as_str();
            if !candidate_matches(label, prefix) {
                continue;
            }
            let mut candidate = item(
                label.to_owned(),
                replacement,
                if snippets {
                    declaration_snippet(label)
                } else {
                    label.to_owned()
                },
                CompletionKind::Keyword,
                Some("state action verb".to_owned()),
                None,
                CompletionOrigin::Syntax,
                false,
                "00",
            );
            if snippets {
                candidate.insert_text_format = CompletionTextFormat::Snippet;
                candidate.validity = CompletionValidity::Scaffold;
            }
            output.push(candidate);
        }
    }

    fn complete_imports(
        &self,
        importer: &SourceOrigin,
        prefix: &str,
        replacement: SourceSpan,
        output: &mut Vec<CompletionItem>,
    ) {
        for (path, detail, origin) in self.import_source_inventory(importer) {
            if candidate_matches(&path, prefix) {
                output.push(item(
                    path.clone(),
                    replacement,
                    path,
                    CompletionKind::Module,
                    Some(detail),
                    None,
                    origin,
                    false,
                    "10",
                ));
            }
        }
    }

    fn complete_import_source_starts(
        &self,
        importer: &SourceOrigin,
        replacement: SourceSpan,
        snippets: bool,
        output: &mut Vec<CompletionItem>,
    ) {
        let mut scaffold = item(
            "'<module>'".to_owned(),
            replacement,
            if snippets {
                "'${1:./module.avenger}'".to_owned()
            } else {
                "''".to_owned()
            },
            CompletionKind::Snippet,
            Some("exact Avenger module origin".to_owned()),
            None,
            CompletionOrigin::Syntax,
            false,
            "00",
        );
        scaffold.insert_text_format = if snippets {
            CompletionTextFormat::Snippet
        } else {
            CompletionTextFormat::PlainText
        };
        scaffold.validity = CompletionValidity::Scaffold;
        output.push(scaffold);
        for (path, detail, origin) in self.import_source_inventory(importer) {
            output.push(item(
                path.clone(),
                replacement,
                format!("'{path}'"),
                CompletionKind::Module,
                Some(detail),
                None,
                origin,
                false,
                "10",
            ));
        }
    }

    fn import_source_inventory(
        &self,
        importer: &SourceOrigin,
    ) -> Vec<(String, String, CompletionOrigin)> {
        let mut output = Vec::new();
        for origin in self.known_sources {
            let (SourceOrigin::File(importer), SourceOrigin::File(path)) = (importer, origin)
            else {
                continue;
            };
            if importer == path {
                continue;
            }
            let Some(path) = relative_module_path(importer, path) else {
                continue;
            };
            output.push((
                path,
                "local Avenger source".to_owned(),
                CompletionOrigin::Syntax,
            ));
        }
        for standard in ["std:datasets", "std:marks", "std:tools", "std:transforms"] {
            output.push((
                standard.to_owned(),
                "standard library module".to_owned(),
                CompletionOrigin::Syntax,
            ));
        }
        for module in self.registry.modules.keys() {
            output.push((
                module.as_str().to_owned(),
                "host-provided native module".to_owned(),
                CompletionOrigin::AuthoringSchema,
            ));
        }
        output.sort_by(|left, right| left.0.cmp(&right.0));
        output.dedup_by(|left, right| left.0 == right.0);
        output
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
        if let Some((span, mode, channel_name)) = channel_mode_at(syntax, request.byte_offset) {
            let mut markdown = format!("`{mode}` channel mode\n\n{}", channel_mode_docs(mode));
            if let Some(channel_name) = channel_name
                && let Some(owner) = owner_symbol(self.index, &request.source, span.range.start)
                && let Some(schema) = schema_for_symbol(self.registry, owner, self.index)
                && let Some(channel) = schema.channels.get(&channel_name)
            {
                markdown.push_str(&format!(
                    "\n\nChannel: `{channel_name}` on `{}`.",
                    owner.detail.as_deref().unwrap_or(owner.keyword.as_str())
                ));
                if let Some(item_type) = &channel.item_type {
                    markdown.push_str(&format!(" Expected Arrow output type: `{item_type}`."));
                }
            }
            return Ok(Some(HoverResult {
                span,
                markdown,
                generation: self.generation,
                source_revision: request.source_revision.clone(),
            }));
        }
        if let Some((span, markdown)) = crate::sql_intelligence::contextual_hover(
            request,
            syntax,
            self.registry,
            self.semantic_roots,
            self.dataset_contexts,
        ) {
            return Ok(Some(HoverResult {
                span,
                markdown,
                generation: self.generation,
                source_revision: request.source_revision.clone(),
            }));
        }
        if let Some((span, markdown)) = crate::sql_intelligence::column_hover(
            request,
            syntax,
            self.registry,
            self.index,
            self.semantic_roots,
            self.dataset_contexts,
            self.completion_cache,
            cancellation,
        ) {
            return Ok(Some(HoverResult {
                span,
                markdown,
                generation: self.generation,
                source_revision: request.source_revision.clone(),
            }));
        }
        if let Some((span, markdown)) = crate::sql_intelligence::typed_boundary_hover(
            request,
            syntax,
            self.semantic_roots,
            self.dataset_contexts,
        ) {
            return Ok(Some(HoverResult {
                span,
                markdown,
                generation: self.generation,
                source_revision: request.source_revision.clone(),
            }));
        }
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
            if symbol.value_kind == IndexedValueKind::Output
                && let Some(detail) = self.output_column_detail(symbol)
            {
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
            let action = enclosing_state_action(syntax, request.byte_offset)
                .filter(|(_, action_target, _)| action_target == &reference.name);
            let sigil = action.is_none()
                && matches!(
                    reference.value_kind,
                    IndexedValueKind::Scalar
                        | IndexedValueKind::Table
                        | IndexedValueKind::Selection
                );
            let sigil = if sigil { "$" } else { "" };
            let mut markdown = format!("```avenger\n{sigil}{}\n```", reference.name);
            if let Some((verb, _, header)) = action {
                markdown.push_str(&format!(
                    "\n\n{}",
                    state_action_hover_docs(verb, reference.value_kind, &header)
                ));
            }
            if let Some(target) = target {
                if let Some(detail) = &target.detail {
                    markdown.push_str(&format!("\n\n{detail}"));
                }
                if target.value_kind == IndexedValueKind::Output
                    && let Some(detail) = self.output_column_detail(target)
                {
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

    fn output_column_detail(&self, symbol: &IndexedSymbol) -> Option<String> {
        self.semantic_roots.values().find_map(|root| {
            let project = root.result.as_ref().ok()?;
            project.datasets.iter().find_map(|(_, dataset)| {
                let source = project.sources.get(dataset.provenance.stage_span.source)?;
                if source.origin.canonical_uri() != symbol.origin.canonical_uri()
                    || dataset.provenance.stage_span.range.start > symbol.selection_span.range.start
                    || symbol.selection_span.range.end > dataset.provenance.stage_span.range.end
                {
                    return None;
                }
                let column = dataset
                    .columns
                    .iter()
                    .find(|column| column.name == symbol.name)?;
                Some(format!(
                    "- Arrow type: `{:?}`\n- Nullable: `{}`\n- Result kind: `column`",
                    column.data_type, column.nullable
                ))
            })
        })
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

fn state_action_hover_docs(
    verb: StateActionVerb,
    target_kind: IndexedValueKind,
    header: &[String],
) -> String {
    let category = match target_kind {
        IndexedValueKind::Scalar => "scalar parameter",
        IndexedValueKind::Table => "store",
        IndexedValueKind::Selection => "selection",
        _ => "state binding",
    };
    let payload = match (target_kind, verb) {
        (IndexedValueKind::Scalar, StateActionVerb::Set) => {
            "The SQL expression is cast at the target parameter's physical Arrow boundary."
        }
        (IndexedValueKind::Table, StateActionVerb::Clear) => {
            "Clears every row in the routed store."
        }
        (IndexedValueKind::Table, StateActionVerb::Patch) => {
            "Updates non-key fields of the row identified by the exact `key` block."
        }
        (IndexedValueKind::Table, StateActionVerb::Delete) => {
            "Deletes the row identified by the exact `key` block."
        }
        (IndexedValueKind::Table, _) => {
            "Uses destination-typed `row` payloads and the store's primary-key contract."
        }
        (IndexedValueKind::Selection, StateActionVerb::Clear) => {
            "Clears all clauses, or only the clauses selected by `within`."
        }
        (IndexedValueKind::Selection, StateActionVerb::Delete) => {
            "Deletes the supplied clause IDs, optionally limited by `within`."
        }
        (IndexedValueKind::Selection, _)
            if header.windows(2).any(|words| words == ["from", "scene"]) =>
        {
            "Builds clauses from a scene hit query; a scoped replace takes its coordination scope from `within`."
        }
        (IndexedValueKind::Selection, _) => "Mutates the supplied typed selection clauses.",
        _ => "The resolved target category determines this verb's payload contract.",
    };
    let route = if header.windows(2).any(|words| words == ["at", "start"]) {
        "gesture-start owner"
    } else {
        "current routed owner"
    };
    let replacing = if header
        .windows(2)
        .any(|words| words == ["replacing", "scopes"])
    {
        " Existing concrete owner copies are removed before this action."
    } else {
        ""
    };
    format!(
        "`{}` action on a {category}; writes the {route}. {payload}{replacing}",
        verb.as_str()
    )
}

fn channel_mode_at(
    syntax: &SyntaxAnalysis,
    offset: usize,
) -> Option<(SourceSpan, &str, Option<String>)> {
    for node in &syntax.parsed.nodes {
        if node.span.range.start <= offset
            && offset <= node.span.range.end
            && let TolerantSyntaxNodeKind::ChannelMode { mode, .. } = &node.kind
        {
            let mut parent = node.parent;
            let mut channel = None;
            while let Some(id) = parent {
                let Some(parent_node) = syntax.parsed.nodes.iter().find(|node| node.id == id)
                else {
                    break;
                };
                if let TolerantSyntaxNodeKind::Property { name } = &parent_node.kind
                    && !matches!(
                        name.as_str(),
                        "encoded" | "direct" | "otherwise" | "predicate"
                    )
                {
                    channel = Some(name.clone());
                    break;
                }
                parent = parent_node.parent;
            }
            return Some((node.span, mode.as_str(), channel));
        }
    }
    None
}

fn complete_channel_value(
    channel: &avenger_chart_schema::ChannelSchema,
    prefix: &str,
    replacement: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    let mut push = |label: &str, insert: &str, detail: &str, rank: &str| {
        if candidate_matches(label, prefix) {
            output.push(item(
                label.to_owned(),
                replacement,
                insert.to_owned(),
                CompletionKind::Keyword,
                Some(detail.to_owned()),
                None,
                CompletionOrigin::AuthoringSchema,
                false,
                rank,
            ));
        }
    };
    match &channel.shape {
        ValueShape::ChannelConfig => {
            push("{ }", "{ }", "configuration-only channel", "00");
        }
        ValueShape::RasterDimensionChannel => {
            push("dim", "dim ", "raster dimension channel", "00");
        }
        _ => {
            push(
                "encoded",
                "encoded ",
                "apply the registered channel encoding policy",
                "00",
            );
            push(
                "direct",
                "direct ",
                "use the expression directly in channel output space",
                "00",
            );
            if matches!(channel.shape, ValueShape::PatternChannel) {
                push("pattern", "pattern { }", "structured direct pattern", "10");
            }
        }
    }
    if !channel.required {
        push("none", "none;", "explicitly absent optional channel", "20");
    }
}

pub(crate) fn physical_type_spans(syntax: &SyntaxAnalysis) -> Vec<SourceSpan> {
    let mut output = Vec::new();
    for node in &syntax.parsed.nodes {
        let TolerantSyntaxNodeKind::Declaration { keyword, name } = &node.kind else {
            continue;
        };
        let tokens = significant_tokens(syntax, Some(node.span));
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
        if keyword != "field" {
            continue;
        }
        let Some(keyword_index) = tokens
            .iter()
            .position(|token| token.word() == Some(keyword.as_str()))
        else {
            continue;
        };
        let type_end = name
            .as_deref()
            .and_then(|name| tokens.iter().rposition(|token| token.word() == Some(name)));
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

fn schema_owner_symbol<'a>(
    index: &'a WorkspaceSemanticIndex,
    registry: &NativeSchemaSnapshot,
    origin: &SourceOrigin,
    cursor: usize,
) -> Option<&'a IndexedSymbol> {
    let document = index.documents.get(origin)?;
    let mut current = document
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            symbol.scope_span.range.start <= cursor && cursor <= symbol.scope_span.range.end
        })
        .min_by_key(|(_, symbol)| symbol.scope_span.range.len())
        .map(|(position, _)| position);
    while let Some(position) = current {
        let symbol = document.symbols.get(position)?;
        if schema_for_symbol(registry, symbol, index).is_some() {
            return Some(symbol);
        }
        current = symbol.parent;
    }
    None
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
        "adjust" => Some(NativeKindNamespace::Adjust),
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

fn declaration_kind_context(
    syntax: &SyntaxAnalysis,
    cursor: usize,
) -> Option<(NativeKindNamespace, String, bool)> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let tokens = if tokens.first().and_then(SigToken::word) == Some("export") {
        &tokens[1..]
    } else {
        &tokens[..]
    };
    let keyword = tokens.first()?.word()?;
    let namespace = namespace_for_keyword(keyword)?;
    if tokens[1..]
        .iter()
        .any(|token| token.word().is_none() && !matches!(token.token, Some(Token::Period)))
    {
        return None;
    }
    let typed = statement_source_after(syntax, tokens, 1, cursor)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let complete =
        !typed.is_empty() && !typed.ends_with('.') && cursor_has_gap_after(tokens.last(), cursor);
    Some((namespace, typed, complete))
}

fn export_cursor_state(syntax: &SyntaxAnalysis, cursor: usize) -> Option<StructuralCursorState> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let export = tokens
        .first()
        .filter(|token| token.word() == Some("export"))?;
    const EXPORTABLE: &[&str] = &["catalog", "chart", "define", "schema", "table"];
    match &tokens[1..] {
        [] if cursor_has_gap_after(Some(export), cursor) => {
            Some(StructuralCursorState::FixedHeader(EXPORTABLE))
        }
        [partial]
            if partial.word().is_some()
                && !cursor_has_gap_after(Some(partial), cursor)
                && EXPORTABLE
                    .iter()
                    .any(|candidate| candidate.starts_with(partial.word().unwrap())) =>
        {
            Some(StructuralCursorState::FixedHeader(EXPORTABLE))
        }
        _ => None,
    }
}

fn structural_statement_start(syntax: &SyntaxAnalysis, cursor: usize) -> usize {
    significant_tokens(syntax, None)
        .into_iter()
        .filter(|token| token.span.range.end <= cursor)
        .filter(|token| {
            matches!(
                token.token,
                Some(Token::LBrace | Token::RBrace | Token::SemiColon)
            )
        })
        .map(|token| token.span.range.end)
        .next_back()
        .unwrap_or_default()
}

fn structural_statement_tokens(syntax: &SyntaxAnalysis, cursor: usize) -> Vec<SigToken<'_>> {
    let start = structural_statement_start(syntax, cursor);
    significant_tokens(syntax, None)
        .into_iter()
        .filter(|token| start <= token.span.range.start && token.span.range.start < cursor)
        .collect()
}

fn version_header_state(syntax: &SyntaxAnalysis, cursor: usize) -> Option<StructuralCursorState> {
    let text = syntax.parsed.tokens.text();
    if text[..cursor.min(text.len())].contains(';') {
        return None;
    }
    let tokens = significant_tokens(syntax, None)
        .into_iter()
        .filter(|token| token.span.range.start < cursor)
        .collect::<Vec<_>>();
    match tokens.as_slice() {
        [] => Some(StructuralCursorState::FixedHeader(&["avenger 1;"])),
        [keyword]
            if keyword
                .word()
                .is_some_and(|word| "avenger".starts_with(word)) =>
        {
            if keyword.word() == Some("avenger") && cursor_has_gap_after(Some(keyword), cursor) {
                Some(StructuralCursorState::FixedHeader(&["1"]))
            } else {
                Some(StructuralCursorState::FixedHeader(&["avenger 1;"]))
            }
        }
        [keyword, version]
            if keyword.word() == Some("avenger")
                && version.raw.parse::<u32>().ok() == Some(avenger_lang_core::LANGUAGE_MAJOR) =>
        {
            Some(StructuralCursorState::TokenInsertion(&[";"]))
        }
        _ => Some(StructuralCursorState::Nothing),
    }
}

fn import_cursor_state(syntax: &SyntaxAnalysis, cursor: usize) -> Option<StructuralCursorState> {
    let all = significant_tokens(syntax, None);
    let statement_start = all
        .iter()
        .rposition(|token| {
            token.span.range.start < cursor && matches!(token.token, Some(Token::SemiColon))
        })
        .map_or(0, |index| index + 1);
    let tokens = all[statement_start..]
        .iter()
        .filter(|token| token.span.range.start < cursor)
        .cloned()
        .collect::<Vec<_>>();
    let import = tokens
        .first()
        .filter(|token| token.word() == Some("import"))?;
    let clause = &tokens[1..];
    if clause.is_empty() {
        return cursor_has_gap_after(Some(import), cursor)
            .then_some(StructuralCursorState::ImportClause);
    }

    let clause_end = match clause.first().and_then(|token| token.token) {
        Some(Token::LBrace) => {
            let Some(close) = clause
                .iter()
                .position(|token| matches!(token.token, Some(Token::RBrace)))
            else {
                return Some(StructuralCursorState::Nothing);
            };
            close + 1
        }
        Some(Token::Mul) => {
            let as_index = clause.iter().position(|token| token.word() == Some("as"));
            let Some(as_index) = as_index else {
                return Some(StructuralCursorState::FixedHeader(&["as"]));
            };
            if clause.get(as_index + 1).and_then(SigToken::word).is_none() {
                return Some(StructuralCursorState::NameInvention);
            }
            as_index + 2
        }
        _ => return Some(StructuralCursorState::ImportClause),
    };

    let tail = &clause[clause_end..];
    if tail.is_empty() {
        return Some(StructuralCursorState::FixedHeader(&["from"]));
    }
    if tail.first().and_then(SigToken::word) != Some("from") {
        return Some(StructuralCursorState::FixedHeader(&["from"]));
    }
    if tail.len() == 1 {
        return cursor_has_gap_after(tail.first(), cursor)
            .then_some(StructuralCursorState::ImportSource);
    }
    let Some(source) = unquote(tail[1].raw) else {
        return Some(StructuralCursorState::Nothing);
    };
    if tail[1].span.range.start < cursor && cursor <= tail[1].span.range.end {
        // `import_prefix` owns completion while the source string is active.
        return Some(StructuralCursorState::Nothing);
    }
    if tail.len() == 2 {
        return Some(StructuralCursorState::ImportTail);
    }
    if tail[2].word() == Some("sha256") {
        if tail.len() == 3 {
            return Some(StructuralCursorState::FixedHeader(&["'<sha256>'"]));
        }
        if unquote(tail[3].raw).is_some() && tail.len() == 4 {
            return Some(StructuralCursorState::TokenInsertion(&[";"]));
        }
        return Some(StructuralCursorState::Nothing);
    }
    if matches!(tail[2].token, Some(Token::SemiColon)) {
        return Some(StructuralCursorState::Nothing);
    }
    let _ = source;
    Some(StructuralCursorState::ImportTail)
}

fn declaration_start_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    match tokens.as_slice() {
        [] => true,
        [token] => token.word().is_some(),
        _ => false,
    }
}

fn structural_noncode_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    syntax.parsed.tokens.tokens().iter().any(|token| {
        let span = token.span().range;
        if !(span.start < cursor && cursor <= span.end) {
            return false;
        }
        match token.kind() {
            LosslessTokenKind::Token(TokenClass::Comment(_)) => true,
            LosslessTokenKind::Token(TokenClass::String) => {
                let raw = syntax.parsed.tokens.raw(token);
                let closed = match raw.as_bytes() {
                    [b'\'', .., b'\''] | [b'"', .., b'"'] => true,
                    _ if raw.starts_with('$') => {
                        let Some(second) = raw[1..].find('$').map(|offset| offset + 1) else {
                            return true;
                        };
                        raw.len() >= 2 * (second + 1) && raw.ends_with(&raw[..=second])
                    }
                    _ => false,
                };
                cursor < span.end || !closed
            }
            _ => false,
        }
    })
}

fn cursor_has_gap_after(token: Option<&SigToken<'_>>, cursor: usize) -> bool {
    token.is_some_and(|token| token.span.range.end < cursor)
}

fn statement_source_after<'a>(
    syntax: &'a SyntaxAnalysis,
    tokens: &[SigToken<'_>],
    first: usize,
    cursor: usize,
) -> Option<&'a str> {
    let start = tokens.get(first)?.span.range.start;
    let text = syntax.parsed.tokens.text();
    text.get(start..cursor.min(text.len()))
}

fn contains_top_level_word_tokens(tokens: &[SigToken<'_>], expected: &str) -> bool {
    top_level_word_indices(tokens, expected).next().is_some()
}

fn top_level_word_indices<'a>(
    tokens: &'a [SigToken<'a>],
    expected: &'a str,
) -> impl DoubleEndedIterator<Item = usize> + 'a {
    let mut depth = 0usize;
    tokens
        .iter()
        .enumerate()
        .filter_map(move |(index, token)| match token.token {
            Some(Token::LParen | Token::LBracket) => {
                depth += 1;
                None
            }
            Some(Token::RParen | Token::RBracket) => {
                depth = depth.saturating_sub(1);
                None
            }
            Some(Token::Word(word)) if depth == 0 && word.value.eq_ignore_ascii_case(expected) => {
                Some(index)
            }
            _ => None,
        })
}

fn name_invention_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    let Some(as_index) = top_level_word_indices(&tokens, "as").next_back() else {
        return false;
    };
    let suffix = &tokens[as_index + 1..];
    suffix.is_empty()
        || (suffix.len() == 1
            && suffix[0].word().is_some()
            && !cursor_has_gap_after(suffix.last(), cursor))
}

fn param_initializer_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    tokens
        .first()
        .is_some_and(|token| token.word() == Some("param"))
        && cursor_has_gap_after(tokens.first(), cursor)
        && !contains_top_level_word_tokens(&tokens[1..], "as")
        && !struct_field_argument_is_name(&tokens)
}

fn physical_type_header_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    let Some(field) = tokens.first().filter(|token| token.word() == Some("field")) else {
        return false;
    };
    if !cursor_has_gap_after(Some(field), cursor)
        || tokens
            .iter()
            .any(|token| matches!(token.token, Some(Token::SemiColon)))
        || struct_field_argument_is_name(&tokens)
    {
        return false;
    }
    let Some(rest) = statement_source_after(syntax, &tokens, 1, cursor) else {
        return true;
    };
    split_header_type(rest.trim_start()).is_none()
}

fn action_target_context(syntax: &SyntaxAnalysis, cursor: usize) -> Option<StateActionVerb> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let verb = StateActionVerb::parse(tokens.first()?.word()?)?;
    (cursor_has_gap_after(tokens.first(), cursor) && tokens.len() <= 2).then_some(verb)
}

fn state_action_target(syntax: &SyntaxAnalysis, cursor: usize) -> Option<String> {
    let tokens = structural_statement_tokens(syntax, cursor);
    if let Some(action) = tokens
        .first()
        .filter(|token| token.word().and_then(StateActionVerb::parse).is_some())
    {
        cursor_has_gap_after(Some(action), cursor).then_some(())?;
        return state_action_target_path(&tokens);
    }
    let (_, target, _) = enclosing_state_action(syntax, cursor)?;
    Some(target)
}

fn event_actions_started(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let Some(event) = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                &node.kind,
                TolerantSyntaxNodeKind::Declaration { keyword, .. } if keyword == "on"
            ) && node.span.range.start <= cursor
                && cursor <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())
    else {
        return false;
    };
    if syntax.parsed.nodes.iter().any(|node| {
        node.id != event.id
            && matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. })
            && event.span.range.start <= node.span.range.start
            && node.span.range.start <= cursor
            && cursor <= node.span.range.end
    }) {
        return false;
    }
    syntax.parsed.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            TolerantSyntaxNodeKind::Declaration { keyword, .. }
                if StateActionVerb::parse(keyword).is_some()
        ) && event.span.range.start <= node.span.range.start
            && node.span.range.end <= cursor
    })
}

fn enclosing_state_action(
    syntax: &SyntaxAnalysis,
    cursor: usize,
) -> Option<(StateActionVerb, String, Vec<String>)> {
    let node = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                &node.kind,
                TolerantSyntaxNodeKind::Declaration { keyword, .. }
                    if StateActionVerb::parse(keyword).is_some()
            ) && node.span.range.start <= cursor
                && cursor <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())?;
    if syntax.parsed.nodes.iter().any(|candidate| {
        candidate.id != node.id
            && matches!(candidate.kind, TolerantSyntaxNodeKind::Declaration { .. })
            && node.span.range.start <= candidate.span.range.start
            && candidate.span.range.start <= cursor
            && cursor <= candidate.span.range.end
    }) {
        return None;
    }
    let tokens = significant_tokens(syntax, Some(node.span));
    let verb = StateActionVerb::parse(tokens.first()?.word()?)?;
    let target = state_action_target_path(&tokens)?;
    let header = tokens
        .iter()
        .take_while(|token| !matches!(token.token, Some(Token::LBrace | Token::SemiColon)))
        .filter_map(SigToken::word)
        .map(str::to_owned)
        .collect();
    Some((verb, target, header))
}

fn state_action_target_path(tokens: &[SigToken<'_>]) -> Option<String> {
    let mut position = 1;
    let mut target = tokens.get(position)?.word()?.to_owned();
    position += 1;
    while position + 1 < tokens.len()
        && matches!(tokens[position].token, Some(Token::Period))
        && tokens[position + 1].word().is_some()
    {
        target.push('.');
        target.push_str(tokens[position + 1].word().unwrap());
        position += 2;
    }
    Some(target)
}

fn action_body_candidates(
    syntax: &SyntaxAnalysis,
    cursor: usize,
    target_kind: Option<IndexedValueKind>,
) -> Option<&'static [&'static str]> {
    let tokens = structural_statement_tokens(syntax, cursor);
    if tokens.len() > 1
        || tokens
            .iter()
            .any(|token| matches!(token.token, Some(Token::Colon)))
    {
        return None;
    }
    let (verb, _, header) = enclosing_state_action(syntax, cursor)?;
    let from_scene = header.windows(2).any(|words| words == ["from", "scene"]);
    let within = header.iter().any(|word| word == "within");
    match (target_kind, verb, from_scene, within) {
        (Some(IndexedValueKind::Table), StateActionVerb::Insert, _, _)
        | (Some(IndexedValueKind::Table), StateActionVerb::Replace, _, _)
        | (Some(IndexedValueKind::Table), StateActionVerb::Upsert, _, _)
        | (Some(IndexedValueKind::Table), StateActionVerb::Toggle, _, _) => Some(&["row"]),
        (Some(IndexedValueKind::Table), StateActionVerb::Patch, _, _) => Some(&["key", "fields"]),
        (Some(IndexedValueKind::Table), StateActionVerb::Delete, _, _) => Some(&["key"]),
        (Some(IndexedValueKind::Selection), _, true, false) => Some(&[
            "geometry",
            "policy",
            "marks",
            "fields",
            "unique_by",
            "max_hits",
            "sharing",
            "clause_id",
        ]),
        (Some(IndexedValueKind::Selection), _, true, true) => Some(&[
            "geometry",
            "policy",
            "marks",
            "fields",
            "unique_by",
            "max_hits",
            "clause_id",
        ]),
        (
            Some(IndexedValueKind::Selection),
            StateActionVerb::Replace | StateActionVerb::Upsert | StateActionVerb::Toggle,
            false,
            _,
        ) => Some(&["clause"]),
        (Some(IndexedValueKind::Selection), StateActionVerb::Delete, false, _) => Some(&["ids"]),
        _ => None,
    }
}

fn action_target_kind_allowed(verb: StateActionVerb, kind: IndexedValueKind) -> bool {
    match verb {
        StateActionVerb::Set => kind == IndexedValueKind::Scalar,
        StateActionVerb::Clear => {
            matches!(kind, IndexedValueKind::Table | IndexedValueKind::Selection)
        }
        StateActionVerb::Insert | StateActionVerb::Patch => kind == IndexedValueKind::Table,
        StateActionVerb::Replace
        | StateActionVerb::Upsert
        | StateActionVerb::Delete
        | StateActionVerb::Toggle => {
            matches!(kind, IndexedValueKind::Table | IndexedValueKind::Selection)
        }
    }
}

fn action_modifier_candidates(
    syntax: &SyntaxAnalysis,
    cursor: usize,
    target_kind: Option<IndexedValueKind>,
) -> Option<&'static [&'static str]> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let verb = StateActionVerb::parse(tokens.first()?.word()?)?;
    if !cursor_has_gap_after(tokens.first(), cursor) {
        return None;
    }
    let target = tokens.get(1)?;
    let cursor_target = target.word() == Some("cursor");
    let tail = tokens[2..]
        .iter()
        .map(SigToken::word)
        .collect::<Option<Vec<_>>>()?;
    if tail.is_empty() {
        if !cursor_has_gap_after(Some(target), cursor) {
            return None;
        }
        return action_initial_modifier_candidates(verb, target_kind, cursor_target);
    }
    match tail.as_slice() {
        [partial] if "at".starts_with(*partial) => Some(&["at"]),
        ["at"] if cursor_has_gap_after(tokens.last(), cursor) => Some(&["current", "start"]),
        ["at", partial] if "current".starts_with(*partial) || "start".starts_with(*partial) => {
            Some(&["current", "start"])
        }
        ["at", "current" | "start"] if cursor_has_gap_after(tokens.last(), cursor) => {
            action_post_route_candidates(verb, target_kind)
        }
        [partial]
            if target_kind != Some(IndexedValueKind::Selection)
                && "replacing".starts_with(*partial) =>
        {
            Some(&["replacing"])
        }
        ["replacing"] if cursor_has_gap_after(tokens.last(), cursor) => Some(&["scopes"]),
        ["replacing", partial] if "scopes".starts_with(*partial) => Some(&["scopes"]),
        ["replacing", "scopes"] if cursor_has_gap_after(tokens.last(), cursor) => {
            (verb == StateActionVerb::Set).then_some(&["to"] as &'static [&'static str])
        }
        [partial] if "from".starts_with(*partial) => Some(&["from"]),
        ["from"] if cursor_has_gap_after(tokens.last(), cursor) => Some(&["scene"]),
        ["from", partial] if "scene".starts_with(*partial) => Some(&["scene"]),
        ["from", "scene"] if cursor_has_gap_after(tokens.last(), cursor) => {
            (verb == StateActionVerb::Replace).then_some(&["within"] as &'static [&'static str])
        }
        [partial] if "within".starts_with(*partial) => Some(&["within"]),
        ["within"] if cursor_has_gap_after(tokens.last(), cursor) => {
            Some(&["shared", "free", "level"])
        }
        ["within", partial]
            if ["shared", "free", "level"]
                .iter()
                .any(|candidate| candidate.starts_with(*partial)) =>
        {
            Some(&["shared", "free", "level"])
        }
        [partial] if verb == StateActionVerb::Set && "to".starts_with(*partial) => Some(&["to"]),
        _ => None,
    }
}

fn action_initial_modifier_candidates(
    verb: StateActionVerb,
    target_kind: Option<IndexedValueKind>,
    cursor_target: bool,
) -> Option<&'static [&'static str]> {
    if cursor_target {
        return (verb == StateActionVerb::Set).then_some(&["to"]);
    }
    match (verb, target_kind) {
        (StateActionVerb::Set, _) => Some(&["at", "replacing", "to"]),
        (_, Some(IndexedValueKind::Table)) => Some(&["at", "replacing"]),
        (StateActionVerb::Clear | StateActionVerb::Delete, Some(IndexedValueKind::Selection)) => {
            Some(&["at", "within"])
        }
        (StateActionVerb::Replace, Some(IndexedValueKind::Selection)) => {
            Some(&["at", "from", "within"])
        }
        (StateActionVerb::Upsert | StateActionVerb::Toggle, Some(IndexedValueKind::Selection)) => {
            Some(&["at", "from"])
        }
        (StateActionVerb::Insert | StateActionVerb::Patch, _) => Some(&["at", "replacing"]),
        (StateActionVerb::Replace | StateActionVerb::Upsert | StateActionVerb::Toggle, None) => {
            Some(&["at", "replacing", "from", "within"])
        }
        (StateActionVerb::Clear | StateActionVerb::Delete, None) => {
            Some(&["at", "replacing", "within"])
        }
        _ => None,
    }
}

fn action_post_route_candidates(
    verb: StateActionVerb,
    target_kind: Option<IndexedValueKind>,
) -> Option<&'static [&'static str]> {
    match (verb, target_kind) {
        (StateActionVerb::Set, _) => Some(&["replacing", "to"]),
        (_, Some(IndexedValueKind::Table)) => Some(&["replacing"]),
        (StateActionVerb::Clear | StateActionVerb::Delete, Some(IndexedValueKind::Selection)) => {
            Some(&["within"])
        }
        (StateActionVerb::Replace, Some(IndexedValueKind::Selection)) => Some(&["from", "within"]),
        (StateActionVerb::Upsert | StateActionVerb::Toggle, Some(IndexedValueKind::Selection)) => {
            Some(&["from"])
        }
        _ => None,
    }
}

fn declaration_binder_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    let Some(keyword) = tokens.first().and_then(SigToken::word) else {
        return false;
    };
    if !cursor_has_gap_after(tokens.first(), cursor) || !cursor_has_gap_after(tokens.last(), cursor)
    {
        return false;
    }
    if matches!(keyword, "store" | "selection") {
        return tokens.len() == 1;
    }
    if keyword != "param" {
        return false;
    }
    if contains_top_level_word_tokens(&tokens[1..], "as") {
        return false;
    }
    let Some(rest) = statement_source_after(syntax, &tokens, 1, cursor) else {
        return false;
    };
    avenger_lang_compiler::normalize_sql_expression(rest.trim()).is_ok()
}

fn field_nullable_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    let Some(field) = tokens.first().filter(|token| token.word() == Some("field")) else {
        return false;
    };
    if !cursor_has_gap_after(Some(field), cursor) || struct_field_argument_is_name(&tokens) {
        return false;
    }
    let Some(rest) = statement_source_after(syntax, &tokens, 1, cursor) else {
        return false;
    };
    let Some((data_type, tail)) = split_header_type(rest.trim_start()) else {
        return false;
    };
    if !physical_type_is_valid(data_type) {
        return false;
    }
    let words = tail.split_whitespace().collect::<Vec<_>>();
    let gap = cursor_has_gap_after(tokens.last(), cursor);
    match words.as_slice() {
        [] => true,
        [word] if gap => *word != "nullable",
        [partial] => "nullable".starts_with(*partial),
        [name, partial] if *name != "nullable" && !gap => "nullable".starts_with(*partial),
        _ => false,
    }
}

fn field_name_invention_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    let Some(field) = tokens.first().filter(|token| token.word() == Some("field")) else {
        return false;
    };
    if !cursor_has_gap_after(Some(field), cursor) || struct_field_argument_is_name(&tokens) {
        return false;
    }
    let Some(rest) = statement_source_after(syntax, &tokens, 1, cursor) else {
        return false;
    };
    let Some((data_type, tail)) = split_header_type(rest.trim_start()) else {
        return false;
    };
    if !physical_type_is_valid(data_type) {
        return false;
    }
    let words = tail.split_whitespace().collect::<Vec<_>>();
    match words.as_slice() {
        ["nullable"] => cursor_has_gap_after(tokens.last(), cursor),
        [name] => !cursor_has_gap_after(tokens.last(), cursor) && !"nullable".starts_with(*name),
        _ => false,
    }
}

fn physical_type_is_valid(data_type: &str) -> bool {
    let probe = SourceFile::new(
        SourceId::new(u32::MAX),
        SourceOrigin::Memory("completion-field-type-probe.avenger".to_owned()),
        format!("avenger 1; chart cartesian {{ store as rows {{ field {data_type} value; }} }}"),
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

fn struct_field_name_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    struct_field_argument_is_name(&structural_statement_tokens(syntax, cursor))
}

fn struct_field_argument_is_name(tokens: &[SigToken<'_>]) -> bool {
    let mut stack = Vec::<(Option<&str>, usize)>::new();
    let mut previous_word = None;
    for token in tokens {
        match token.token {
            Some(Token::Word(word)) => previous_word = Some(word.value.as_str()),
            Some(Token::LParen) => {
                stack.push((previous_word.take(), 0));
            }
            Some(Token::Comma) => {
                if let Some((_, argument)) = stack.last_mut() {
                    *argument += 1;
                }
                previous_word = None;
            }
            Some(Token::RParen) => {
                stack.pop();
                previous_word = None;
            }
            _ => previous_word = None,
        }
    }
    stack
        .last()
        .is_some_and(|(function, argument)| *function == Some("field") && *argument >= 1)
}

fn fixed_header_candidates(
    syntax: &SyntaxAnalysis,
    cursor: usize,
) -> Option<&'static [&'static str]> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let keyword = tokens.first()?.word()?;
    if tokens.len() > 2
        || tokens.iter().any(|token| token.word().is_none())
        || (tokens.len() == 2 && cursor_has_gap_after(tokens.last(), cursor))
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
            "ref",
            "block",
            "channel",
        ]),
        "variable" => Some(&["row", "column", "item"]),
        _ => None,
    }
}

fn property_value_context(syntax: &SyntaxAnalysis, cursor: usize) -> Option<&str> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if node.span.range.start < cursor && cursor <= node.span.range.end =>
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

fn relation_path_prefix(syntax: &SyntaxAnalysis, cursor: usize) -> String {
    let text = syntax.parsed.tokens.text();
    let before = &text[..cursor.min(text.len())];
    let start = before
        .char_indices()
        .rev()
        .find_map(|(index, character)| {
            (!(character == '.' || character == '_' || character.is_alphanumeric()))
                .then_some(index + character.len_utf8())
        })
        .unwrap_or(0);
    before[start..].to_owned()
}

fn incomplete_property_value_context(syntax: &SyntaxAnalysis, cursor: usize) -> Option<String> {
    let tokens = structural_statement_tokens(syntax, cursor);
    let colons = tokens
        .iter()
        .filter(|token| matches!(token.token, Some(Token::Colon)))
        .count();
    if colons != 1 {
        return None;
    }
    tokens.first().and_then(SigToken::word).map(str::to_owned)
}

fn is_property_name_context(syntax: &SyntaxAnalysis, cursor: usize) -> bool {
    let tokens = structural_statement_tokens(syntax, cursor);
    tokens.len() <= 1
        && !tokens
            .iter()
            .any(|token| matches!(token.token, Some(Token::Colon)))
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

fn authored_declaration_properties(syntax: &SyntaxAnalysis, scope: SourceSpan) -> BTreeSet<String> {
    let declaration_owner = syntax
        .parsed
        .nodes
        .iter()
        .find(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::Declaration { .. })
                && node.span.range.start == scope.range.start
        })
        .map(|node| node.id);
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if node.parent == declaration_owner
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

fn channel_mode_docs(mode: &str) -> &'static str {
    match mode {
        "encoded" => {
            "Evaluates the SQL expression through the channel policy registered by the active mark and coordinate profile. Scale-bearing policies create/apply a scale and participate in domain inference; identity policies remain unscaled."
        }
        "direct" => {
            "Evaluates the SQL expression directly in the channel's output space. It bypasses scale creation, domain inference, and scale application; the expression may still vary by row."
        }
        _ => "Unknown channel evaluation mode.",
    }
}

fn channel_property_has_head_mode(
    syntax: &SyntaxAnalysis,
    channel_property: &avenger_lang_core::syntax::TolerantSyntaxNode,
) -> bool {
    use avenger_lang_core::syntax::TolerantChannelModeRole;

    syntax.parsed.nodes.iter().any(|node| {
        matches!(
            node.kind,
            TolerantSyntaxNodeKind::ChannelMode {
                role: TolerantChannelModeRole::Head,
                ..
            }
        ) && node.parent == Some(channel_property.id)
    })
}

pub(crate) fn channel_body_has_effective_encoded(
    syntax: &SyntaxAnalysis,
    channel_property: &avenger_lang_core::syntax::TolerantSyntaxNode,
) -> bool {
    use avenger_lang_core::syntax::TolerantChannelModeRole;

    let is_descendant = |candidate: &avenger_lang_core::syntax::TolerantSyntaxNode| {
        let mut parent = candidate.parent;
        while let Some(id) = parent {
            if id == channel_property.id {
                return true;
            }
            parent = syntax
                .parsed
                .nodes
                .iter()
                .find(|node| node.id == id)
                .and_then(|node| node.parent);
        }
        false
    };
    let mut head = None;
    let mut otherwise = None;
    let mut encoded_condition = false;
    for node in &syntax.parsed.nodes {
        let TolerantSyntaxNodeKind::ChannelMode { mode, role, .. } = &node.kind else {
            continue;
        };
        if !is_descendant(node) {
            continue;
        }
        match role {
            TolerantChannelModeRole::Head => head = Some(mode.as_str()),
            TolerantChannelModeRole::WhenBranch => encoded_condition |= mode == "encoded",
            TolerantChannelModeRole::OtherwiseBranch => otherwise = Some(mode.as_str()),
        }
    }
    encoded_condition || otherwise.or(head) == Some("encoded")
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
    let keyword = tokens
        .iter()
        .position(|token| token.word() == Some(symbol.keyword.as_str()))?;
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
    snippets: bool,
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
                complete_shape(shape, prefix, replacement, snippets, output);
            }
        }
        ValueShape::OneOrMany(shape) | ValueShape::Array(shape) => {
            if candidate_matches("[value]", prefix) {
                push_shape_scaffold(
                    "[value]",
                    if snippets { "[${1:value}]" } else { "[]" },
                    "array value",
                    replacement,
                    snippets,
                    output,
                );
            }
            if matches!(
                shape.as_ref(),
                ValueShape::Atom { .. } | ValueShape::Boolean
            ) {
                complete_shape(shape, prefix, replacement, snippets, output);
            }
        }
        ValueShape::Integer => push_shape_scaffold(
            "integer",
            if snippets { "${1:0}" } else { "0" },
            "integer literal",
            replacement,
            snippets,
            output,
        ),
        ValueShape::Number | ValueShape::RasterDimension => push_shape_scaffold(
            "number",
            if snippets { "${1:0.0}" } else { "0.0" },
            "numeric literal",
            replacement,
            snippets,
            output,
        ),
        ValueShape::String => push_shape_scaffold(
            "string",
            if snippets { "'${1:value}'" } else { "''" },
            "string literal",
            replacement,
            snippets,
            output,
        ),
        ValueShape::Identifier => push_shape_scaffold(
            "identifier",
            if snippets { "${1:name}" } else { "name" },
            "source identifier",
            replacement,
            snippets,
            output,
        ),
        ValueShape::Object(_)
        | ValueShape::Map(_)
        | ValueShape::ChannelMap
        | ValueShape::ChannelConfig
        | ValueShape::WidgetData
        | ValueShape::StateActionBlock => push_shape_scaffold(
            "block",
            if snippets { "{\n  $0\n}" } else { "{ }" },
            "structured value",
            replacement,
            snippets,
            output,
        ),
        ValueShape::MarkBlock => push_shape_scaffold(
            "mark block",
            if snippets {
                "{\n  mark ${1:kind} as ${2:name} {\n    $0\n  }\n}"
            } else {
                "{ }"
            },
            "local mark block",
            replacement,
            snippets,
            output,
        ),
        ValueShape::PatternChannel => push_shape_scaffold(
            "pattern",
            if snippets {
                "pattern {\n  $0\n}"
            } else {
                "pattern { }"
            },
            "pattern channel value",
            replacement,
            snippets,
            output,
        ),
        ValueShape::CoordinationScope => {
            for value in ["shared", "free"] {
                if candidate_matches(value, prefix) {
                    output.push(item(
                        value.to_owned(),
                        replacement,
                        value.to_owned(),
                        CompletionKind::EnumValue,
                        Some("coordination scope".to_owned()),
                        None,
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
            push_shape_scaffold(
                "level",
                if snippets {
                    "level(${1:0})"
                } else {
                    "level(0)"
                },
                "coordination level",
                replacement,
                snippets,
                output,
            );
        }
        ValueShape::FacetDataScope => {
            for value in ["filtered", "broadcast"] {
                if candidate_matches(value, prefix) {
                    output.push(item(
                        value.to_owned(),
                        replacement,
                        value.to_owned(),
                        CompletionKind::EnumValue,
                        Some("facet data scope".to_owned()),
                        None,
                        CompletionOrigin::AuthoringSchema,
                        false,
                        "00",
                    ));
                }
            }
            push_shape_scaffold(
                "level",
                if snippets {
                    "level(${1:0})"
                } else {
                    "level(0)"
                },
                "facet level",
                replacement,
                snippets,
                output,
            );
        }
        ValueShape::Any => {
            for (label, insert, detail) in [
                (
                    "boolean",
                    if snippets { "${1:true}" } else { "true" },
                    "boolean value",
                ),
                (
                    "number",
                    if snippets { "${1:0.0}" } else { "0.0" },
                    "numeric value",
                ),
                (
                    "string",
                    if snippets { "'${1:value}'" } else { "''" },
                    "string value",
                ),
                (
                    "block",
                    if snippets { "{\n  $0\n}" } else { "{ }" },
                    "structured value",
                ),
            ] {
                if candidate_matches(label, prefix) {
                    push_shape_scaffold(label, insert, detail, replacement, snippets, output);
                }
            }
        }
        ValueShape::SqlExpression
        | ValueShape::SqlProjection { .. }
        | ValueShape::SqlQuery
        | ValueShape::ConfiguredExpression(_)
        | ValueShape::ConfiguredReference { .. }
        | ValueShape::RasterDimensionChannel
        | ValueShape::ScalarBinding
        | ValueShape::TableBinding
        | ValueShape::SelectionBinding
        | ValueShape::TypedReference { .. } => {
            // These shapes route to SQL, channel, or reference providers that
            // own their semantic candidate sets. Keeping this arm explicit
            // makes a newly added ValueShape a compile-time completion audit.
        }
    }
}

fn push_shape_scaffold(
    label: &str,
    insert: &str,
    detail: &str,
    replacement: SourceSpan,
    snippets: bool,
    output: &mut Vec<CompletionItem>,
) {
    let mut candidate = item(
        label.to_owned(),
        replacement,
        insert.to_owned(),
        CompletionKind::Snippet,
        Some(detail.to_owned()),
        None,
        CompletionOrigin::AuthoringSchema,
        false,
        "20",
    );
    candidate.insert_text_format = if snippets {
        CompletionTextFormat::Snippet
    } else {
        CompletionTextFormat::PlainText
    };
    candidate.validity = CompletionValidity::Scaffold;
    output.push(candidate);
}

fn binding_kinds(shape: &ValueShape) -> Vec<IndexedValueKind> {
    let mut output = BTreeSet::new();
    collect_binding_kinds(shape, &mut output);
    output.into_iter().collect()
}

fn collect_binding_kinds(shape: &ValueShape, output: &mut BTreeSet<IndexedValueKind>) {
    match shape {
        ValueShape::ScalarBinding | ValueShape::SqlExpression => {
            output.insert(IndexedValueKind::Scalar);
        }
        ValueShape::TableBinding | ValueShape::SqlQuery => {
            output.insert(IndexedValueKind::Table);
        }
        ValueShape::SelectionBinding => {
            output.insert(IndexedValueKind::Selection);
        }
        ValueShape::Any => {
            output.extend([
                IndexedValueKind::Scalar,
                IndexedValueKind::Table,
                IndexedValueKind::Selection,
            ]);
        }
        ValueShape::Union(shapes) => {
            for shape in shapes {
                collect_binding_kinds(shape, output);
            }
        }
        ValueShape::OneOrMany(shape) | ValueShape::Array(shape) => {
            collect_binding_kinds(shape, output);
        }
        _ => {}
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
        ValueShape::SqlProjection { .. } => "SQL projection list",
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
        "param" => &[("sharing", "State-sharing policy.")],
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
        "domain_contribution" => &["infer", "exclude"],
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
    if before
        .split_whitespace()
        .last()
        .is_none_or(|word| word != "from")
    {
        return None;
    }
    let value = &import[quote_start + quote.len_utf8()..];
    (!value.contains(quote)).then_some(value)
}

fn relative_module_path(importer: &Path, target: &Path) -> Option<String> {
    let base = importer.parent()?;
    let base = base.components().collect::<Vec<_>>();
    let target = target.components().collect::<Vec<_>>();
    let common = base
        .iter()
        .zip(&target)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        return None;
    }
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &target[common..] {
        relative.push(component.as_os_str());
    }
    let mut relative = relative.to_string_lossy().replace('\\', "/");
    if !relative.starts_with("../") && relative != ".." {
        relative.insert_str(0, "./");
    }
    Some(relative)
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
    let semantic_kind = structural_semantic_kind(kind, origin);
    let semantic_identity = format!("{semantic_kind:?}:{origin:?}:{label}");
    CompletionItem {
        filter_text: Some(label.clone()),
        sort_key: format!("{bucket}:{}", label.to_ascii_lowercase()),
        match_text: label.clone(),
        label,
        replacement,
        insert_text,
        insert_text_format: CompletionTextFormat::PlainText,
        kind,
        semantic_kind,
        semantic_identity,
        qualification: CompletionQualification::Unqualified,
        data_type: None,
        nullable: None,
        source_stage: None,
        expected_type_compatible: None,
        confidence: 100,
        semantic_proximity: 50,
        usage_prevalence: 0,
        validity: CompletionValidity::Strict,
        detail,
        documentation,
        origin,
        deprecated,
    }
}

fn structural_semantic_kind(
    kind: CompletionKind,
    origin: CompletionOrigin,
) -> CompletionSemanticKind {
    match kind {
        CompletionKind::Keyword => CompletionSemanticKind::DslKeyword,
        CompletionKind::Declaration | CompletionKind::Snippet => {
            CompletionSemanticKind::Declaration
        }
        CompletionKind::Property => CompletionSemanticKind::Property,
        CompletionKind::EnumValue => CompletionSemanticKind::EnumValue,
        CompletionKind::Variable => CompletionSemanticKind::ScalarParam,
        CompletionKind::Field => {
            if origin == CompletionOrigin::DatasetSchema {
                CompletionSemanticKind::DataColumn
            } else {
                CompletionSemanticKind::StructField
            }
        }
        CompletionKind::Function => CompletionSemanticKind::ScalarFunction,
        CompletionKind::Type => CompletionSemanticKind::SqlType,
        CompletionKind::Module => CompletionSemanticKind::Relation,
        CompletionKind::Catalog => CompletionSemanticKind::Catalog,
        CompletionKind::Schema => CompletionSemanticKind::Schema,
        CompletionKind::Table => {
            if origin == CompletionOrigin::LexicalScope {
                CompletionSemanticKind::StoreParam
            } else {
                CompletionSemanticKind::Relation
            }
        }
    }
}

fn annotate_structural_usage_prevalence(
    items: &mut [CompletionItem],
    index: &WorkspaceSemanticIndex,
) {
    for item in items {
        let name = item.label.trim_start_matches('$');
        let count = index
            .documents
            .values()
            .map(|document| {
                document
                    .symbols
                    .iter()
                    .filter(|symbol| symbol.name.eq_ignore_ascii_case(name))
                    .count()
                    + document
                        .references
                        .iter()
                        .filter(|reference| reference.name.eq_ignore_ascii_case(name))
                        .count()
            })
            .sum::<usize>();
        item.usage_prevalence = u32::try_from(count).unwrap_or(u32::MAX);
    }
}

fn declaration_snippet(keyword: &str) -> String {
    match keyword {
        "param" => "param ${1:CAST(NULL AS DOUBLE)} as ${2:name};$0".to_owned(),
        "store" | "selection" => {
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
        "set" => "set ${1:target} to ${2:value};$0".to_owned(),
        "clear" => "clear ${1:target};$0".to_owned(),
        "insert" | "replace" | "upsert" | "patch" | "delete" | "toggle" => {
            format!("{keyword} ${{1:target}} {{\n  $0\n}}")
        }
        _ => keyword.to_owned(),
    }
}

fn source_declaration_label(semantic_keyword: &str) -> Option<&str> {
    match semantic_keyword {
        "store" => Some("store"),
        "selection" => Some("selection"),
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
        "clear" => Some("clear"),
        "insert" => Some("insert"),
        "replace" => Some("replace"),
        "upsert" => Some("upsert"),
        "patch" => Some("patch"),
        "delete" => Some("delete"),
        "toggle" => Some("toggle"),
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
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
        let text = "avenger 1; chart cartesian as chart { param CAST(1.0 AS DOUBLE) as width; mark symbol as points { size: $width; } }";
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
    fn semantic_index_preserves_exact_sql_island_sites_and_owners() {
        let text = r#"avenger 1;
define transform sample {
  param 1 + 2 as offset;
  output $offset + 1 as adjusted;
  expressions: "value" AS copied;
}
chart cartesian {
  sql: SELECT 1;
  values: [1 + 2, none, { enabled: true; }];
  mark symbol as points {
    x: encoded "value";
    opacity: $offset;
  }
  on click {
    set cursor to 'crosshair';
    set offset to $offset + 1;
  }
}"#;
        let (origin, syntax) = fixture(text);
        let index = build_document_index(&origin, &syntax);
        let sites = index
            .sql_islands
            .iter()
            .map(|island| island.site)
            .collect::<BTreeSet<_>>();
        assert_eq!(sites, SqlIslandSite::ALL.into_iter().collect());
        assert!(
            index
                .sql_islands
                .iter()
                .all(|island| island.authored_fingerprint.len() == 64)
        );
        assert_eq!(
            index
                .sql_islands
                .iter()
                .map(|island| &island.identity)
                .collect::<BTreeSet<_>>()
                .len(),
            index.sql_islands.len()
        );

        let channel = index
            .sql_islands
            .iter()
            .find(|island| island.site == SqlIslandSite::ChannelModePayload)
            .unwrap();
        assert_eq!(channel.declaration_keyword.as_deref(), Some("mark"));
        assert_eq!(channel.declaration_name.as_deref(), Some("points"));
        assert_eq!(channel.property_path, ["x"]);
        assert_eq!(channel.channel_mode.as_deref(), Some("encoded"));

        let array = index
            .sql_islands
            .iter()
            .find(|island| island.site == SqlIslandSite::ArrayElement)
            .unwrap();
        assert_eq!(&text[array.span.range.as_range()], "1 + 2");
        assert_eq!(array.property_path, ["values"]);

        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = crate::sql_intelligence::SqlCompletionCache::default();
        for island in &index.sql_islands {
            let request = PositionRequest {
                source: origin.clone(),
                byte_offset: island.span.range.end,
                source_revision: syntax.revision.clone(),
            };
            let completion = crate::sql_intelligence::complete_sql(
                &request,
                &syntax,
                compiler.language_host().authoring_schema(),
                &WorkspaceSemanticIndex {
                    documents: BTreeMap::from([(origin.clone(), index.clone())]),
                    ..WorkspaceSemanticIndex::default()
                },
                &BTreeMap::new(),
                &BTreeMap::new(),
                &CompletionInvocation::Invoked,
                true,
                &cache,
                &AnalysisCancellation::default(),
            )
            .unwrap_or_else(|| panic!("site {:?} was not routed to SQL", island.site));
            assert_eq!(completion.debug.site, island.site);
            assert!(completion.debug.repair_attempts <= 6);
            assert!(
                completion
                    .items
                    .iter()
                    .all(|item| item.replacement.range.end <= text.len())
            );
        }
    }

    #[test]
    fn index_preserves_unified_header_names_and_semantic_categories() {
        let text = r#"avenger 1; chart cartesian {
          store as rows {}
          selection as picked {}
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
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
    fn authored_name_binders_do_not_dump_semantic_candidates() {
        for marked in [
            "avenger 1; chart cartesian as |",
            "avenger 1; chart cartesian { mark symbol as | }",
            "avenger 1; chart cartesian { param CAST(1 AS DOUBLE) as wi| }",
            "avenger 1; chart cartesian { store as rows { field int64 nullable | } }",
            "avenger 1; chart cartesian { store as rows { field int64 ident| } }",
        ] {
            assert!(completion_labels(marked).is_empty(), "{marked}");
        }
    }

    #[test]
    fn declaration_and_field_headers_follow_token_boundaries() {
        let declaration = completion_labels("avenger 1; chart cartesian {\n  mark symbol\n  |\n}");
        assert_eq!(declaration, ["as", "body"]);

        let field_type =
            completion_labels("avenger 1; chart cartesian { store as rows {\n  field utf|\n} }");
        assert!(field_type.contains(&"utf8".to_owned()), "{field_type:?}");

        let nullable = completion_labels(
            "avenger 1; chart cartesian { store as rows {\n  field int64\n  |\n} }",
        );
        assert_eq!(nullable, ["nullable"]);
    }

    #[test]
    fn structural_completion_is_silent_in_strings_comments_and_unknown_statements() {
        for source in [
            "avenger 1; chart cartesian { -- |\n }",
            "avenger 1; chart cartesian { 'unterminated| }",
            "avenger 1; chart cartesian { nonsense extra | }",
        ] {
            let labels = completion_labels(source);
            assert!(labels.is_empty(), "{source}: {labels:?}");
        }
    }

    #[test]
    fn every_value_shape_has_an_explicit_structural_completion_strategy() {
        use avenger_chart_schema::{EnumValueSchema, ProjectionExpressionMode, ProjectionPolicy};

        let shapes = vec![
            ValueShape::Boolean,
            ValueShape::Integer,
            ValueShape::Number,
            ValueShape::String,
            ValueShape::Identifier,
            ValueShape::Atom {
                values: vec![EnumValueSchema {
                    value: "sample".to_owned(),
                    docs: "sample enum".to_owned(),
                }],
            },
            ValueShape::SqlExpression,
            ValueShape::SqlProjection {
                policy: ProjectionPolicy::Named,
                expression_mode: ProjectionExpressionMode::Scalar,
            },
            ValueShape::SqlQuery,
            ValueShape::ChannelConfig,
            ValueShape::ConfiguredExpression(BTreeMap::new()),
            ValueShape::ConfiguredReference {
                namespaces: BTreeSet::new(),
                properties: BTreeMap::new(),
            },
            ValueShape::PatternChannel,
            ValueShape::CoordinationScope,
            ValueShape::FacetDataScope,
            ValueShape::RasterDimension,
            ValueShape::RasterDimensionChannel,
            ValueShape::ScalarBinding,
            ValueShape::TableBinding,
            ValueShape::SelectionBinding,
            ValueShape::WidgetData,
            ValueShape::StateActionBlock,
            ValueShape::MarkBlock,
            ValueShape::TypedReference {
                namespaces: BTreeSet::new(),
            },
            ValueShape::Union(vec![ValueShape::Boolean, ValueShape::String]),
            ValueShape::OneOrMany(Box::new(ValueShape::Boolean)),
            ValueShape::Array(Box::new(ValueShape::Number)),
            ValueShape::Map(Box::new(ValueShape::String)),
            ValueShape::ChannelMap,
            ValueShape::Object(BTreeMap::new()),
            ValueShape::Any,
        ];
        assert_eq!(
            shapes.len(),
            31,
            "update the completion audit for ValueShape"
        );
        let replacement = SourceSpan {
            source: SourceId::new(0),
            range: ByteSpan::empty(0),
        };
        for shape in &shapes {
            let mut output = Vec::new();
            complete_shape(shape, "", replacement, true, &mut output);
            assert!(
                output.iter().all(|item| item.replacement == replacement),
                "{shape:?}"
            );
        }

        let mut any = Vec::new();
        complete_shape(&ValueShape::Any, "", replacement, true, &mut any);
        assert_eq!(
            any.iter()
                .map(|item| item.label.as_str())
                .collect::<Vec<_>>(),
            ["boolean", "number", "string", "block"]
        );
    }

    #[test]
    #[ignore = "developer aid for refreshing the structural exact-list corpus"]
    fn record_structural_exact_list_candidates() {
        for (name, source) in [
            ("empty", "|"),
            ("version", "avenger |"),
            ("version_terminator", "avenger 1|"),
            ("module", "avenger 1; |"),
            ("module_after_item", "avenger 1; chart cartesian {} |"),
            ("export", "avenger 1; export |"),
            ("export_chart_kind", "avenger 1; export chart |"),
            ("import", "avenger 1; import |"),
            ("import_star", "avenger 1; import * |"),
            ("import_namespace_binder", "avenger 1; import * as |"),
            ("import_namespace_from", "avenger 1; import * as acme |"),
            ("import_named_from", "avenger 1; import { foo } |"),
            ("import_source", "avenger 1; import { foo } from |"),
            (
                "import_tail",
                "avenger 1; import { foo } from './foo.avenger' |",
            ),
            (
                "import_hash",
                "avenger 1; import { foo } from './foo.avenger' sha256 |",
            ),
            (
                "import_hash_end",
                "avenger 1; import { foo } from './foo.avenger' sha256 'abc' |",
            ),
            ("chart_kind", "avenger 1; chart |"),
            ("chart_tail", "avenger 1; chart cartesian |"),
            ("chart_name", "avenger 1; chart cartesian as | {}"),
            ("chart_body", "avenger 1; chart cartesian { | }"),
            ("mark_kind", "avenger 1; chart cartesian { mark | }"),
            (
                "mark_body",
                "avenger 1; chart cartesian { mark symbol { | } }",
            ),
            (
                "channel_mode",
                "avenger 1; chart cartesian { mark symbol { x: | } }",
            ),
            (
                "boolean_value",
                "avenger 1; chart cartesian { mark symbol { visible: |; } }",
            ),
            ("param_header", "avenger 1; chart cartesian { param | }"),
            (
                "store_body",
                "avenger 1; chart cartesian { store as rows { | } }",
            ),
            (
                "field_type",
                "avenger 1; chart cartesian { store as rows { field | } }",
            ),
            (
                "field_nullable",
                "avenger 1; chart cartesian { store as rows { field int64 name | } }",
            ),
            (
                "set_target",
                "avenger 1; chart cartesian { param 1 as width; on click { set | } }",
            ),
            (
                "set_scalar_tail",
                "avenger 1; chart cartesian { param 1 as width; on click { set width | } }",
            ),
            ("comment", "avenger 1; chart cartesian { -- |\n }"),
            ("unknown", "avenger 1; chart cartesian { nonsense extra | }"),
        ] {
            eprintln!("{name}: {:?}", completion_labels(source));
        }
    }

    #[test]
    fn completion_replacement_never_splits_unicode() {
        let text = "avenger 1; chart cartesian as chart { param 1.0 as café; mark symbol { size: $caf; } }";
        let (_, syntax) = fixture(text);
        let cursor = text.find("$caf").unwrap() + "$caf".len();
        let span = replacement_span(&syntax, cursor);
        assert_eq!(&text[span.range.as_range()], "$caf");
    }

    #[test]
    fn state_action_analysis_preserves_complete_qualified_targets() {
        let text =
            "avenger 1; chart cartesian { on click { set controls.width at current to 2; } }";
        let (_, syntax) = fixture(text);
        let cursor = text.find(" at current").unwrap();
        assert_eq!(
            state_action_target(&syntax, cursor).as_deref(),
            Some("controls.width")
        );
        let (_, target, _) = enclosing_state_action(&syntax, cursor).unwrap();
        assert_eq!(target, "controls.width");
    }

    #[test]
    fn local_import_paths_are_relative_to_the_importing_module() {
        assert_eq!(
            relative_module_path(
                Path::new("/project/charts/main.avenger"),
                Path::new("/project/lib/marks.avenger")
            )
            .as_deref(),
            Some("../lib/marks.avenger")
        );
        assert_eq!(
            relative_module_path(
                Path::new("/project/charts/main.avenger"),
                Path::new("/project/charts/shared.avenger")
            )
            .as_deref(),
            Some("./shared.avenger")
        );
    }

    #[test]
    fn unified_header_and_action_completion_use_semantic_categories() {
        let initializers = completion_labels("avenger 1; chart cartesian { param | }");
        assert!(initializers.contains(&"CAST".to_owned()));
        assert!(initializers.contains(&"NULL".to_owned()));
        assert!(!initializers.contains(&"store".to_owned()));
        assert!(!initializers.contains(&"selection".to_owned()));
        assert!(!initializers.contains(&"float64".to_owned()));

        let fields = completion_labels("avenger 1; chart cartesian { field utf| }");
        assert!(fields.contains(&"utf8".to_owned()));

        let targets = completion_labels(
            "avenger 1; chart cartesian { param 1 as width; store as rows {} selection as picked {} on click { set | } }",
        );
        for target in ["width", "cursor"] {
            assert!(targets.contains(&target.to_owned()), "{targets:?}");
        }
        for invalid in ["rows", "picked"] {
            assert!(!targets.contains(&invalid.to_owned()), "{targets:?}");
        }

        let store_targets = completion_labels(
            "avenger 1; chart cartesian { store as rows {} selection as picked {} on click { insert | } }",
        );
        assert!(
            store_targets.contains(&"rows".to_owned()),
            "{store_targets:?}"
        );
        assert!(
            !store_targets.contains(&"picked".to_owned()),
            "{store_targets:?}"
        );

        let mutation_targets = completion_labels(
            "avenger 1; chart cartesian { store as rows {} selection as picked {} on click { replace | } }",
        );
        for target in ["rows", "picked"] {
            assert!(
                mutation_targets.contains(&target.to_owned()),
                "{mutation_targets:?}"
            );
        }

        let scalar_modifiers = completion_labels(
            "avenger 1; chart cartesian { param 1 as width; on click { set width | } }",
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
            "avenger 1; chart cartesian { selection as picked {} on click { clear picked | } }",
        );
        assert!(
            selection_modifiers.contains(&"at".to_owned()),
            "{selection_modifiers:?}"
        );
        assert!(
            selection_modifiers.contains(&"within".to_owned()),
            "{selection_modifiers:?}"
        );
        assert!(
            !selection_modifiers.contains(&"replacing".to_owned()),
            "{selection_modifiers:?}"
        );

        let action_heads = completion_labels("avenger 1; chart cartesian { on click { | } }");
        for verb in StateActionVerb::ALL {
            assert!(
                action_heads.contains(&verb.as_str().to_owned()),
                "{action_heads:?}"
            );
        }
        let widget_action_heads = completion_labels(
            "avenger 1; chart zerod { widget button as button { label: 'Run'; action: { | } } }",
        );
        for verb in StateActionVerb::ALL {
            assert!(
                widget_action_heads.contains(&verb.as_str().to_owned()),
                "{widget_action_heads:?}"
            );
        }
        let chart_body = completion_labels("avenger 1; chart cartesian { | }");
        for verb in StateActionVerb::ALL {
            assert!(
                !chart_body.contains(&verb.as_str().to_owned()),
                "{chart_body:?}"
            );
        }

        let store_body = completion_labels(
            "avenger 1; chart cartesian { store as rows {} on click { patch rows { | } } }",
        );
        assert_eq!(store_body, ["fields", "key"]);

        let scene_body = completion_labels(
            "avenger 1; chart cartesian { selection as picked {} on click { replace picked from scene within free { | } } }",
        );
        for property in [
            "clause_id",
            "fields",
            "geometry",
            "marks",
            "max_hits",
            "policy",
            "unique_by",
        ] {
            assert!(scene_body.contains(&property.to_owned()), "{scene_body:?}");
        }
        assert!(
            !scene_body.contains(&"sharing".to_owned()),
            "{scene_body:?}"
        );

        let bindings = completion_labels(
            "avenger 1; chart cartesian { param 1 as width; store as rows {} selection as picked {} mark symbol { size: $|; } }",
        );
        assert!(bindings.contains(&"$width".to_owned()));
        assert!(!bindings.contains(&"$rows".to_owned()));
        assert!(bindings.contains(&"$picked".to_owned()));

        let binder = completion_labels("avenger 1; chart cartesian { param 1 | }");
        assert!(binder.contains(&"as".to_owned()), "{binder:?}");
        assert!(!binder.contains(&"float64".to_owned()), "{binder:?}");
        for category in ["store", "selection"] {
            let binder =
                completion_labels(&format!("avenger 1; chart cartesian {{ {category} | }}"));
            assert!(binder.contains(&"as".to_owned()), "{category}: {binder:?}");
        }

        let field =
            completion_labels("avenger 1; chart cartesian { store as rows { field utf8 name | } }");
        assert!(field.contains(&"nullable".to_owned()), "{field:?}");

        let member = completion_labels(
            "avenger 1; chart cartesian { param CAST(NULL AS DOUBLE) as value { | } }",
        );
        assert!(member.contains(&"sharing".to_owned()), "{member:?}");
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
        assert!(
            !slot_shapes.contains(&"function".to_owned()),
            "{slot_shapes:?}"
        );

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
    fn channel_value_completion_offers_only_context_legal_modes() {
        let channel = completion_labels("avenger 1; chart cartesian { mark symbol { x: | } }");
        assert!(channel.contains(&"encoded".to_owned()), "{channel:?}");
        assert!(channel.contains(&"direct".to_owned()), "{channel:?}");
        assert!(channel.contains(&"none".to_owned()), "{channel:?}");

        let ordinary =
            completion_labels("avenger 1; chart cartesian { transform filter { predicate: | } }");
        assert!(!ordinary.contains(&"encoded".to_owned()), "{ordinary:?}");
        assert!(!ordinary.contains(&"direct".to_owned()), "{ordinary:?}");

        let branch = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'base' { when { | } } } }",
        );
        assert!(branch.contains(&"predicate".to_owned()), "{branch:?}");
        assert!(branch.contains(&"encoded".to_owned()), "{branch:?}");
        assert!(branch.contains(&"direct".to_owned()), "{branch:?}");

        let selected_branch = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'base' { when { predicate: true; direct: 'hit'; | } } } }",
        );
        assert!(
            !selected_branch.contains(&"encoded".to_owned()),
            "{selected_branch:?}"
        );
        assert!(
            !selected_branch.contains(&"direct".to_owned()),
            "{selected_branch:?}"
        );

        let otherwise = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'base' { otherwise: { | } } } }",
        );
        assert!(otherwise.contains(&"encoded".to_owned()), "{otherwise:?}");
        assert!(otherwise.contains(&"direct".to_owned()), "{otherwise:?}");

        let direct_body = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: direct 'base' { | } } }",
        );
        assert!(
            !direct_body.contains(&"scale".to_owned()),
            "{direct_body:?}"
        );
        assert!(
            !direct_body.contains(&"legend".to_owned()),
            "{direct_body:?}"
        );

        let mixed_body = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: direct 'base' { when { predicate: true; encoded: 'kind'; } | } } }",
        );
        assert!(mixed_body.contains(&"scale".to_owned()), "{mixed_body:?}");
    }

    #[test]
    fn declaration_completion_emits_only_canonical_source_starters() {
        let labels = completion_labels("avenger 1; chart cartesian { | }");
        assert!(labels.contains(&"store".to_owned()));
        assert!(labels.contains(&"selection".to_owned()));
        assert!(labels.contains(&"mark group".to_owned()));
        assert!(!labels.contains(&"param store".to_owned()));
        assert!(!labels.contains(&"param selection".to_owned()));
        assert!(!labels.contains(&"group".to_owned()));
        assert!(!labels.contains(&"dimension".to_owned()));
    }

    #[test]
    fn group_and_legend_overlay_completion_use_mark_semantics() {
        let mark_kinds = completion_labels("avenger 1; chart cartesian { mark | }");
        assert!(mark_kinds.contains(&"group".to_owned()), "{mark_kinds:?}");

        let channel_configs = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'x' { | } } }",
        );
        assert!(
            channel_configs.contains(&"legend".to_owned()),
            "{channel_configs:?}"
        );

        let legend_properties = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'x' { legend: { | } } } }",
        );
        assert!(
            legend_properties.contains(&"overlay".to_owned()),
            "{legend_properties:?}"
        );

        let overlay_declarations = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'x' { legend: { overlay: { | } } } } }",
        );
        assert_eq!(overlay_declarations, vec!["mark"]);

        let overlay_mark_kinds = completion_labels(
            "avenger 1; chart cartesian { mark symbol { fill: encoded 'x' { legend: { overlay: { mark | } } } } }",
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
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
    fn tolerant_named_imports_navigate_exported_transform_bindings() {
        let chart_text = "avenger 1; import { pass } from 'transforms.avenger'; chart cartesian as chart { transform pass {} }";
        let definition_text =
            "avenger 1; export define transform pass { transform filter { predicate: true; } }";
        let chart_origin = SourceOrigin::Memory("multi/chart.avenger".to_owned());
        let definition_origin = SourceOrigin::Memory("multi/transforms.avenger".to_owned());
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
        );
        let reference_start = chart_text.find("transform pass").unwrap() + "transform ".len();
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
        let completion_cache = crate::sql_intelligence::SqlCompletionCache::default();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
            &semantic_roots,
            &dataset_contexts,
            &completion_cache,
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
