//! Resolved-guided source expansion for imported definitions.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{
    Diagnostic, ExpansionOrImportFrame, SourceFile, SourceId, SourceLabel, SourceMap, SourceOrigin,
    SourceSpan,
    ast::{
        AstNodeRole, Body, Decl, File, ModuleItem, Name, PropertyMap, Value, Visibility,
        is_state_action_keyword,
    },
    module_graph::{ModuleId, ParsedModule, ParsedModuleGraph, SourceModuleId},
    print::print_file,
    resolve::{
        BindingCategory, DefinitionKind, DefinitionSchema, ModuleItemId, ResolvedKindBinding,
        ResolvedModuleGraph,
    },
    syntax::{SyntaxLimits, parse_file_with_limits},
};

const MARK_BLOCK_PATH_SEGMENT: usize = usize::MAX;
const SPLICED_BLOCK_PATH_SEGMENT: usize = usize::MAX - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExpansionLimits {
    pub max_declarations: usize,
    pub max_depth: usize,
    pub max_output_bytes_per_chart: usize,
    pub max_total_output_bytes: usize,
    pub syntax: SyntaxLimits,
}

impl Default for ExpansionLimits {
    fn default() -> Self {
        Self {
            max_declarations: 100_000,
            max_depth: 128,
            max_output_bytes_per_chart: 16 * 1024 * 1024,
            max_total_output_bytes: 64 * 1024 * 1024,
            syntax: SyntaxLimits {
                max_tokens: 2_000_000,
                max_declarations: 100_000,
                ..SyntaxLimits::default()
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpansionMapping {
    pub expanded: SourceSpan,
    pub authored: SourceSpan,
    pub definition: Option<SourceSpan>,
    pub instantiation: Option<SourceSpan>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExpansionSourceMap {
    pub mappings: Vec<ExpansionMapping>,
}

impl ExpansionSourceMap {
    /// Return the authored span corresponding to an expanded span, or the
    /// original span when no expansion mapping applies.
    pub fn authored_span(&self, span: SourceSpan) -> SourceSpan {
        self.mapping_for(span)
            .map_or(span, |mapping| mapping.authored)
    }

    /// Re-anchor a diagnostic emitted against canonical expanded source to the
    /// authored declaration and retain macro-style definition/instance context.
    pub fn remap_diagnostic(&self, diagnostic: &mut Diagnostic) {
        let Some(mapping) = self.mapping_for(diagnostic.primary.span) else {
            return;
        };
        diagnostic.primary.span = mapping.authored;
        for secondary in &mut diagnostic.secondary {
            if let Some(mapping) = self.mapping_for(secondary.span) {
                secondary.span = mapping.authored;
            }
        }
        if let Some(definition) = mapping.definition
            && definition != mapping.authored
        {
            push_trace(
                &mut diagnostic.trace,
                definition,
                "expanded from this definition",
            );
        }
        if let Some(instantiation) = mapping.instantiation {
            push_trace(
                &mut diagnostic.trace,
                instantiation,
                "while expanding this definition instance",
            );
        }
    }

    pub fn remap_diagnostics(&self, diagnostics: &mut [Diagnostic]) {
        for diagnostic in diagnostics {
            self.remap_diagnostic(diagnostic);
        }
    }

    fn mapping_for(&self, span: SourceSpan) -> Option<&ExpansionMapping> {
        self.mappings
            .iter()
            .filter(|mapping| {
                mapping.expanded.source == span.source
                    && mapping.expanded.range.start <= span.range.start
                    && mapping.expanded.range.end >= span.range.end
            })
            .min_by_key(|mapping| {
                mapping
                    .expanded
                    .range
                    .end
                    .saturating_sub(mapping.expanded.range.start)
            })
            .or_else(|| {
                self.mappings
                    .iter()
                    .filter(|mapping| {
                        mapping.expanded.source == span.source
                            && mapping.expanded.range.start <= span.range.start
                    })
                    .max_by_key(|mapping| mapping.expanded.range.start)
            })
    }
}

fn push_trace(trace: &mut Vec<ExpansionOrImportFrame>, span: SourceSpan, message: &str) {
    if !trace
        .iter()
        .any(|frame| frame.span == span && frame.message == message)
    {
        trace.push(ExpansionOrImportFrame {
            span,
            message: message.to_owned(),
        });
    }
}

fn import_clause_is_subset(
    retained: &crate::ast::ImportClause,
    original: &crate::ast::ImportClause,
) -> bool {
    match (retained, original) {
        (crate::ast::ImportClause::Named(retained), crate::ast::ImportClause::Named(original)) => {
            retained
                .iter()
                .all(|specifier| original.contains(specifier))
        }
        (
            crate::ast::ImportClause::Namespace(retained),
            crate::ast::ImportClause::Namespace(original),
        ) => retained == original,
        _ => false,
    }
}

#[derive(Clone, Debug)]
pub struct ExpandedModuleGraph {
    pub module_graph: ParsedModuleGraph,
    pub texts: BTreeMap<SourceModuleId, String>,
    pub source_map: ExpansionSourceMap,
}

#[derive(Clone, Debug)]
pub struct ExpansionFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

#[derive(Clone, Copy)]
struct PendingOrigin {
    authored: SourceSpan,
    definition: Option<SourceSpan>,
    instantiation: Option<SourceSpan>,
}

#[derive(Clone)]
struct BoundSlot {
    shape: String,
    value: Value,
    owner: SourceModuleId,
    exposes: Vec<String>,
}

#[derive(Clone)]
struct ExpansionContext {
    caller: SourceModuleId,
    chart: SourceModuleId,
    definition: ModuleItemId,
    instance_name: String,
    instance_identity: String,
    private_names: BTreeMap<String, String>,
    exposed_private_names: BTreeMap<String, String>,
    slots: BTreeMap<String, BoundSlot>,
    channels: BTreeMap<String, String>,
    blocks: BTreeMap<String, BoundSlot>,
    definition_span: SourceSpan,
    instantiation_span: SourceSpan,
}

pub fn expand_module_graph(
    module_graph: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
) -> Result<ExpandedModuleGraph, ExpansionFailure> {
    expand_module_graph_with_limits(module_graph, resolved, ExpansionLimits::default())
}

pub fn expand_module_graph_with_limits(
    module_graph: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    limits: ExpansionLimits,
) -> Result<ExpandedModuleGraph, ExpansionFailure> {
    let mut expander = Expander {
        module_graph,
        resolved,
        limits,
        diagnostics: Vec::new(),
        pending_origins: BTreeMap::new(),
        expanded_declarations: 0,
        expansion_depth: 0,
        total_output_bytes: 0,
    };
    expander.expand()
}

struct Expander<'a> {
    module_graph: &'a ParsedModuleGraph,
    resolved: &'a ResolvedModuleGraph,
    limits: ExpansionLimits,
    diagnostics: Vec<Diagnostic>,
    pending_origins: BTreeMap<SourceModuleId, Vec<PendingOrigin>>,
    expanded_declarations: usize,
    expansion_depth: usize,
    total_output_bytes: usize,
}

impl Expander<'_> {
    fn expand(&mut self) -> Result<ExpandedModuleGraph, ExpansionFailure> {
        let mut next_source = self
            .module_graph
            .sources
            .iter()
            .map(|(id, _)| id.get())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut sources = self.module_graph.sources.clone();
        let mut source_modules = BTreeMap::new();
        let mut texts = BTreeMap::new();

        for (module_id, module) in &self.module_graph.source_modules {
            if !module
                .parsed
                .ast
                .items
                .iter()
                .any(|item| item.declaration.keyword.as_str() == "chart")
            {
                source_modules.insert(module_id.clone(), module.clone());
                continue;
            }

            let items = module
                .parsed
                .ast
                .items
                .iter()
                .enumerate()
                .map(|(item_index, item)| ModuleItem {
                    exported: item.exported,
                    declaration: if item.declaration.keyword.as_str() == "chart" {
                        self.expand_declaration(
                            module_id,
                            &item.declaration,
                            &[item_index],
                            None,
                            PendingOrigin {
                                authored: declaration_span(module, &[item_index])
                                    .unwrap_or_else(|| root_span(module)),
                                definition: None,
                                instantiation: None,
                            },
                        )
                    } else {
                        self.retain_declaration_origins(module_id, module, item_index);
                        item.declaration.clone()
                    },
                })
                .collect();
            let ast = File {
                version: module.parsed.ast.version,
                imports: self.expanded_imports(module_id, &module.parsed.ast.imports),
                items,
            };
            let text = print_file(&ast);
            if text.len() > self.limits.max_output_bytes_per_chart {
                return Err(ExpansionFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-EXPAND-007",
                        "expanded module size limit exceeded",
                        SourceLabel::new(
                            root_span(module),
                            format!(
                                "expanded module is {} bytes; limit is {} bytes",
                                text.len(),
                                self.limits.max_output_bytes_per_chart
                            ),
                        ),
                    )],
                    sources,
                });
            }
            self.total_output_bytes =
                self.total_output_bytes
                    .checked_add(text.len())
                    .ok_or_else(|| ExpansionFailure {
                        diagnostics: vec![Diagnostic::error(
                            "AVENGER-EXPAND-008",
                            "expanded module-graph size limit exceeded",
                            SourceLabel::new(
                                root_span(module),
                                "expanded module-graph byte count overflowed",
                            ),
                        )],
                        sources: sources.clone(),
                    })?;
            if self.total_output_bytes > self.limits.max_total_output_bytes {
                return Err(ExpansionFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-EXPAND-008",
                        "expanded module-graph size limit exceeded",
                        SourceLabel::new(
                            root_span(module),
                            format!(
                                "expanded modules total {} bytes; limit is {} bytes",
                                self.total_output_bytes, self.limits.max_total_output_bytes
                            ),
                        ),
                    )],
                    sources,
                });
            }

            let source = SourceId::new(next_source);
            next_source = next_source.saturating_add(1);
            let origin =
                SourceOrigin::Memory(format!("<expanded:{}>", module.origin.canonical_uri()));
            let source_file = SourceFile::new(source, origin, text.clone());
            let parsed =
                parse_file_with_limits(&source_file, self.limits.syntax).map_err(|error| {
                    ExpansionFailure {
                        diagnostics: vec![error.diagnostic().clone()],
                        sources: sources.clone(),
                    }
                })?;
            sources
                .insert(source_file)
                .map_err(|error| ExpansionFailure {
                    diagnostics: vec![Diagnostic::error(
                        "AVENGER-EXPAND-002",
                        "expanded source identity collision",
                        SourceLabel::new(root_span(module), error.to_string()),
                    )],
                    sources: sources.clone(),
                })?;
            source_modules.insert(
                module_id.clone(),
                ParsedModule {
                    id: module_id.clone(),
                    source,
                    origin: module.origin.clone(),
                    content_version: format!("expanded:{}", module.content_version),
                    content_sha256: content_hash(&text),
                    parsed,
                },
            );
            texts.insert(module_id.clone(), text);
        }

        if !self.diagnostics.is_empty() {
            return Err(ExpansionFailure {
                diagnostics: std::mem::take(&mut self.diagnostics),
                sources,
            });
        }

        let source_ids = source_modules
            .iter()
            .map(|(id, module)| (id.clone(), module.source))
            .collect::<BTreeMap<_, _>>();
        let imports = self
            .module_graph
            .imports
            .iter()
            .filter_map(|edge| {
                let import = source_modules
                    .get(&edge.importer)?
                    .parsed
                    .ast
                    .imports
                    .iter()
                    .find(|import| {
                        import.source == edge.specifier
                            && import_clause_is_subset(&import.clause, &edge.clause)
                    })?;
                let mut edge = edge.clone();
                edge.clause = import.clause.clone();
                edge.importer_source = source_ids[&edge.importer];
                edge.imported_source = match &edge.imported {
                    ModuleId::Source(id) => Some(source_ids[id]),
                    ModuleId::Native(_) => None,
                };
                Some(edge)
            })
            .collect::<Vec<_>>();
        let mut source_map = ExpansionSourceMap::default();
        for (file_id, pending) in &self.pending_origins {
            let Some(file) = source_modules.get(file_id) else {
                continue;
            };
            let mut expanded_spans = file
                .parsed
                .source_map
                .iter()
                .filter_map(|(id, span)| {
                    matches!(
                        file.parsed.source_map.role(id),
                        Some(AstNodeRole::Declaration(_))
                    )
                    .then_some(span)
                })
                .collect::<Vec<_>>();
            expanded_spans
                .sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
            for (expanded, origin) in expanded_spans.into_iter().zip(pending) {
                source_map.mappings.push(ExpansionMapping {
                    expanded,
                    authored: origin.authored,
                    definition: origin.definition,
                    instantiation: origin.instantiation,
                });
            }
        }
        source_map.mappings.sort_by_key(|mapping| mapping.expanded);

        Ok(ExpandedModuleGraph {
            module_graph: ParsedModuleGraph {
                sources,
                source_modules,
                native_modules: self.module_graph.native_modules.clone(),
                imports,
                requested_modules: self.module_graph.requested_modules.clone(),
                ambient_data_modules: self.module_graph.ambient_data_modules.clone(),
                ambient_catalog: self.module_graph.ambient_catalog.clone(),
                fingerprint: self.module_graph.fingerprint.clone(),
            },
            texts,
            source_map,
        })
    }

    /// Record identity mappings for declarations copied unchanged into a
    /// canonicalized module. The expanded source map is assembled by pairing
    /// printed declarations with this ordered origin ledger, so omitting
    /// non-chart items would shift every subsequent chart mapping.
    fn retain_declaration_origins(
        &mut self,
        module_id: &SourceModuleId,
        module: &ParsedModule,
        item_index: usize,
    ) {
        let Some(item_span) = declaration_span(module, &[item_index]) else {
            return;
        };
        let mut spans = module
            .parsed
            .source_map
            .iter()
            .filter_map(|(id, span)| {
                (matches!(
                    module.parsed.source_map.role(id),
                    Some(AstNodeRole::Declaration(_))
                ) && span.source == item_span.source
                    && item_span.range.start <= span.range.start
                    && span.range.end <= item_span.range.end)
                    .then_some(span)
            })
            .collect::<Vec<_>>();
        spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
        self.pending_origins
            .entry(module_id.clone())
            .or_default()
            .extend(spans.into_iter().map(|authored| PendingOrigin {
                authored,
                definition: None,
                instantiation: None,
            }));
    }

    fn expanded_imports(
        &self,
        module: &SourceModuleId,
        imports: &[crate::ast::Import],
    ) -> Vec<crate::ast::Import> {
        let Some(environment) = self
            .resolved
            .source_modules
            .get(module)
            .map(|module| &module.local_bindings)
        else {
            return imports.to_vec();
        };
        imports
            .iter()
            .filter_map(|import| {
                let crate::ast::ImportClause::Named(specifiers) = &import.clause else {
                    return Some(import.clone());
                };
                let retained = specifiers
                    .iter()
                    .filter(|specifier| {
                        let imported_definition = [
                            avenger_chart_schema::NativeKindNamespace::Mark,
                            avenger_chart_schema::NativeKindNamespace::Tool,
                            avenger_chart_schema::NativeKindNamespace::Transform,
                        ]
                        .into_iter()
                        .find_map(|namespace| {
                            let category = BindingCategory::NativeKind(namespace);
                            let export = environment
                                .local
                                .get(&(category, specifier.local.to_string()))?;
                            let crate::module_graph::ModuleId::Source(source) = &export.module
                            else {
                                return None;
                            };
                            self.resolved
                                .definitions
                                .values()
                                .find(|definition| {
                                    definition.item.module == *source
                                        && definition.source_name == export.name
                                })
                                .map(|definition| definition.item.clone())
                        });
                        imported_definition.is_none_or(|definition| {
                            self.retained_item_uses_definition(module, &definition)
                        })
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                (!retained.is_empty()).then(|| {
                    let mut import = import.clone();
                    import.clause = crate::ast::ImportClause::Named(retained);
                    import
                })
            })
            .collect()
    }

    fn retained_item_uses_definition(
        &self,
        module: &SourceModuleId,
        definition: &ModuleItemId,
    ) -> bool {
        self.resolved
            .source_modules
            .get(module)
            .into_iter()
            .flat_map(|module| module.item_order.iter().zip(&module.roots))
            .filter(|(_, declaration)| declaration.keyword != "chart")
            .any(|(item, _)| {
                item == definition
                    || self
                        .resolved
                        .item_dependencies
                        .transitive_closures
                        .get(item)
                        .is_some_and(|closure| closure.contains(definition))
            })
    }

    fn expand_declaration(
        &mut self,
        owner: &SourceModuleId,
        declaration: &Decl,
        path: &[usize],
        context: Option<&ExpansionContext>,
        origin: PendingOrigin,
    ) -> Decl {
        if self.expanded_declarations >= self.limits.max_declarations {
            self.push_limit_diagnostic(
                "AVENGER-EXPAND-005",
                "expanded declaration limit exceeded",
                origin.authored,
                format!(
                    "expanded project exceeds {} declarations",
                    self.limits.max_declarations
                ),
            );
            return declaration.clone();
        }
        if self.expansion_depth >= self.limits.max_depth {
            self.push_limit_diagnostic(
                "AVENGER-EXPAND-006",
                "definition expansion depth limit exceeded",
                origin.authored,
                format!(
                    "definition expansion exceeds {} levels",
                    self.limits.max_depth
                ),
            );
            return declaration.clone();
        }
        self.expanded_declarations += 1;
        self.expansion_depth += 1;
        let expanded = self.expand_declaration_inner(owner, declaration, path, context, origin);
        self.expansion_depth -= 1;
        expanded
    }

    fn expand_declaration_inner(
        &mut self,
        owner: &SourceModuleId,
        declaration: &Decl,
        path: &[usize],
        context: Option<&ExpansionContext>,
        origin: PendingOrigin,
    ) -> Decl {
        if let Some(definition) = self.definition_binding(owner, declaration, path) {
            return self.instantiate_definition(
                owner,
                declaration,
                path,
                definition,
                context,
                origin,
            );
        }

        let mut substituted = declaration.clone();
        if declaration.keyword.as_str() == "export"
            && declaration.name.is_none()
            && let Some(source_alias) = declaration
                .props
                .get("source")
                .and_then(value_path)
                .and_then(|path| path.last().cloned())
        {
            substituted.name = Some(source_alias);
        }
        if let Some(context) = context
            && owner == &context.definition.module
            && let Some(source_name) = declaration.name.as_ref().map(Name::as_str)
            && (binds_private_name(declaration)
                || is_state_action_keyword(declaration.keyword.as_str()))
            && let Some(private_name) = context.private_names.get(source_name)
        {
            substituted.name = Some(name(private_name));
        }
        let resolved_properties = self
            .resolved_declaration(owner, path)
            .map(|declaration| declaration.properties.clone());
        let mut mark_block_ordinal = 0;
        substituted.props = self.expand_properties_at(
            owner,
            &declaration.props,
            context,
            resolved_properties.as_ref(),
            path,
            &mut mark_block_ordinal,
        );
        self.pending_origins
            .entry(self.chart_owner(owner, context))
            .or_default()
            .push(origin);
        let expands_private = context.is_some_and(|context| {
            owner == &context.definition.module
                && (declaration.visibility == Visibility::Private
                    || (declaration.visibility == Visibility::Default
                        && targetable_declaration(declaration)))
        });
        let body = self.expand_body(
            owner,
            &PropertyMap::default(),
            &declaration.children,
            path,
            None,
            expands_private,
            context,
        );
        substituted.children = body.children;
        substituted
    }

    fn push_limit_diagnostic(
        &mut self,
        code: &'static str,
        message: &'static str,
        span: SourceSpan,
        label: String,
    ) {
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == code)
        {
            return;
        }
        self.diagnostics.push(Diagnostic::error(
            code,
            message,
            SourceLabel::new(span, label),
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_body(
        &mut self,
        owner: &SourceModuleId,
        props: &PropertyMap,
        children: &[Decl],
        parent_path: &[usize],
        source_indices: Option<&[usize]>,
        inside_private: bool,
        context: Option<&ExpansionContext>,
    ) -> Body {
        let mut mark_block_ordinal = 0;
        self.expand_body_at(
            owner,
            props,
            children,
            parent_path,
            source_indices,
            inside_private,
            context,
            &mut mark_block_ordinal,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn expand_body_at(
        &mut self,
        owner: &SourceModuleId,
        props: &PropertyMap,
        children: &[Decl],
        parent_path: &[usize],
        source_indices: Option<&[usize]>,
        inside_private: bool,
        context: Option<&ExpansionContext>,
        mark_block_ordinal: &mut usize,
    ) -> Body {
        let mut body = Body {
            props: self.expand_properties_at(
                owner,
                props,
                context,
                None,
                parent_path,
                mark_block_ordinal,
            ),
            children: Vec::new(),
        };
        for (index, child) in children.iter().enumerate() {
            let source_index = source_indices.map_or(index, |indices| indices[index]);
            if child.keyword.as_str() == "match" {
                let Some(context) = context else {
                    continue;
                };
                let Some(name) = child.name.as_ref().map(Name::as_str) else {
                    continue;
                };
                let selected = context
                    .slots
                    .get(name)
                    .and_then(|slot| value_atom_name(&slot.value));
                if let Some(arm) = child
                    .children
                    .iter()
                    .find(|arm| arm.name.as_ref().map(Name::as_str) == selected)
                {
                    let mut arm_path = parent_path.to_vec();
                    arm_path.push(source_index);
                    let selected_body = self.expand_body(
                        owner,
                        &arm.props,
                        &arm.children,
                        &arm_path,
                        None,
                        inside_private,
                        Some(context),
                    );
                    for (name, value) in selected_body.props.iter() {
                        body.props.set(name.clone(), value.clone());
                    }
                    body.children.extend(selected_body.children);
                }
                continue;
            }
            if child.keyword.as_str() == "splice" {
                let Some(context) = context else {
                    continue;
                };
                let Some(name) = child.name.as_ref().map(Name::as_str) else {
                    continue;
                };
                let Some(block) = context.blocks.get(name) else {
                    continue;
                };
                let Value::Block { body: supplied, .. } = &block.value else {
                    continue;
                };
                let supplied_body = self.expand_body(
                    &block.owner,
                    &supplied.props,
                    &supplied.children,
                    &[SPLICED_BLOCK_PATH_SEGMENT],
                    None,
                    inside_private,
                    Some(&ExpansionContext {
                        exposed_private_names: block
                            .exposes
                            .iter()
                            .filter_map(|name| {
                                context
                                    .private_names
                                    .get(name)
                                    .cloned()
                                    .map(|private| (name.clone(), private))
                            })
                            .collect(),
                        ..context.clone()
                    }),
                );
                for mut supplied in supplied_body.children {
                    if block.owner == context.caller && supplied.name.is_some() && inside_private {
                        supplied.visibility = Visibility::Public;
                    }
                    body.children.push(supplied);
                }
                continue;
            }
            let mut child_path = parent_path.to_vec();
            child_path.push(source_index);
            let child_origin = context.map_or_else(
                || PendingOrigin {
                    authored: self.declaration_source_span(owner, child),
                    definition: None,
                    instantiation: None,
                },
                |context| PendingOrigin {
                    authored: self.declaration_source_span(owner, child),
                    definition: Some(context.definition_span),
                    instantiation: Some(context.instantiation_span),
                },
            );
            let mut expanded =
                self.expand_declaration(owner, child, &child_path, context, child_origin);
            if context.is_some()
                && expanded.name.is_some()
                && (targetable_declaration(&expanded) || expanded.keyword.as_str() == "transform")
                && expanded.visibility == Visibility::Default
            {
                expanded.visibility = Visibility::Private;
            }
            body.children.push(expanded);
        }
        body
    }

    fn instantiate_definition(
        &mut self,
        caller: &SourceModuleId,
        instance: &Decl,
        instance_path: &[usize],
        definition_id: ModuleItemId,
        outer_context: Option<&ExpansionContext>,
        origin: PendingOrigin,
    ) -> Decl {
        let Some(definition_file) = self.module_graph.source_modules.get(&definition_id.module)
        else {
            return instance.clone();
        };
        let Some(schema) = self.resolved.definitions.get(&definition_id) else {
            return instance.clone();
        };
        let Some(template_index) = self
            .resolved
            .source_modules
            .get(&definition_id.module)
            .and_then(|module| {
                module
                    .roots
                    .iter()
                    .position(|declaration| declaration.id == schema.declaration)
            })
        else {
            return instance.clone();
        };
        let template = &definition_file.parsed.ast.items[template_index].declaration;
        let definition_span = declaration_span(definition_file, &[template_index])
            .unwrap_or_else(|| root_span(definition_file));
        let instantiation_span = origin.authored;
        let source_instance_name = instance
            .name
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("anonymous_{}", schema.source_name));
        let instance_identity = expansion_identity(
            schema.local_seed.as_str(),
            caller.as_str(),
            &stable_expansion_path(self.module_graph.source_modules.get(caller), instance_path),
            outer_context.map_or("", |context| context.instance_name.as_str()),
        );
        let instance_name = format!("{source_instance_name}_{instance_identity}");
        let private_names = definition_private_names(template, &instance_identity);
        if let Some(generated) =
            generated_name_collision(self.module_graph, caller, &private_names, outer_context)
        {
            self.diagnostics.push(Diagnostic::error(
                "AVENGER-EXPAND-009",
                "compiler-generated name collides with source",
                SourceLabel::new(
                    instantiation_span,
                    format!(
                        "`{generated}` is reserved for this definition expansion; rename the authored binding"
                    ),
                ),
            ));
        }
        let chart = outer_context
            .map(|context| context.chart.clone())
            .unwrap_or_else(|| caller.clone());
        let channels = schema
            .channels
            .iter()
            .filter_map(|(name, channel)| {
                instance
                    .props
                    .get(name)
                    .map(|value| self.expand_value(caller, value, outer_context))
                    .as_ref()
                    .and_then(value_atom_name)
                    .map(str::to_owned)
                    .or_else(|| channel.physical_channel.clone())
                    .map(|physical| (name.clone(), physical))
            })
            .collect::<BTreeMap<_, _>>();
        let mut slots = BTreeMap::new();
        let mut blocks = BTreeMap::new();
        for name in &schema.slot_order {
            let slot = &schema.slots[name];
            let supplied = instance.props.get(name);
            let default = supplied
                .is_none()
                .then(|| slot_default(template, name))
                .flatten();
            let Some(raw_value) = supplied.or(default.as_ref()) else {
                continue;
            };
            let owner = if supplied.is_some() {
                caller.clone()
            } else {
                definition_id.module.clone()
            };
            let value = if slot.shape == "block" {
                raw_value.clone()
            } else if supplied.is_some() {
                self.expand_value(caller, raw_value, outer_context)
            } else {
                let default_context = ExpansionContext {
                    caller: caller.clone(),
                    chart: chart.clone(),
                    definition: definition_id.clone(),
                    instance_name: instance_name.clone(),
                    instance_identity: instance_identity.clone(),
                    private_names: private_names.clone(),
                    exposed_private_names: BTreeMap::new(),
                    slots: slots.clone(),
                    channels: channels.clone(),
                    blocks: blocks.clone(),
                    definition_span,
                    instantiation_span,
                };
                self.expand_value(&definition_id.module, raw_value, Some(&default_context))
            };
            let bound = BoundSlot {
                shape: slot.shape.clone(),
                value,
                owner,
                exposes: slot.exposes.clone(),
            };
            if slot.shape == "block" {
                blocks.insert(name.clone(), bound.clone());
            }
            slots.insert(name.clone(), bound);
        }
        let context = ExpansionContext {
            caller: caller.clone(),
            chart,
            definition: definition_id.clone(),
            instance_name,
            instance_identity,
            private_names,
            exposed_private_names: BTreeMap::new(),
            slots,
            channels,
            blocks,
            definition_span,
            instantiation_span,
        };
        self.pending_origins
            .entry(self.chart_owner(caller, Some(&context)))
            .or_default()
            .push(PendingOrigin {
                authored: instantiation_span,
                definition: Some(definition_span),
                instantiation: Some(instantiation_span),
            });

        let mut wrapper = match schema.kind {
            DefinitionKind::Mark => {
                let mut declaration = Decl::new(name("mark"));
                declaration.kind = Some(name("group").into());
                declaration
            }
            DefinitionKind::Tool => {
                let mut declaration = Decl::new(name("tool"));
                declaration.kind = Some(name("behavior").into());
                declaration
            }
            DefinitionKind::Transform => {
                let mut declaration = Decl::new(name("transform"));
                declaration.kind = Some(name("pipeline").into());
                declaration
            }
        };
        wrapper.name = instance.name.clone();
        wrapper.visibility = instance.visibility;
        if schema.kind != DefinitionKind::Transform {
            wrapper.props.set(
                name("component_kind"),
                Value::Atom(name(&schema.source_name)),
            );
        }
        for (property, value) in instance.props.iter() {
            let generic = match schema.kind {
                DefinitionKind::Mark => matches!(
                    property.as_str(),
                    "visible" | "details" | "zindex" | "facet_data_scope" | "geometry_space"
                ),
                DefinitionKind::Transform => property.as_str() == "scope",
                DefinitionKind::Tool => false,
            };
            if generic {
                wrapper.props.set(
                    property.clone(),
                    self.expand_value(caller, value, outer_context),
                );
            }
        }

        let ordered_children = template
            .children
            .iter()
            .enumerate()
            .filter(|child| !matches!(child.1.keyword.as_str(), "slot" | "channel"))
            .collect::<Vec<_>>();
        let mut ordered_children = ordered_children;
        ordered_children
            .sort_by_key(|(_, child)| !matches!(child.keyword.as_str(), "output" | "export"));
        let child_indices = ordered_children
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>();
        let children = ordered_children
            .into_iter()
            .map(|(_, child)| child.clone())
            .collect::<Vec<_>>();
        let expanded = self.expand_body(
            &definition_id.module,
            &PropertyMap::default(),
            &children,
            &[template_index],
            Some(&child_indices),
            false,
            Some(&context),
        );
        wrapper.children = expanded.children;
        if schema.kind == DefinitionKind::Transform {
            let dynamic_outputs = schema
                .slots
                .iter()
                .filter(|(_, slot)| slot.shape == "outputs")
                .filter_map(|(slot_name, _)| context.slots.get(slot_name))
                .filter_map(|slot| {
                    let Value::Projection(projection) = &slot.value else {
                        return None;
                    };
                    Some(projection)
                })
                .flat_map(|projection| projection.items())
                .filter_map(|item| {
                    let sqlparser::ast::SelectItem::ExprWithAlias { alias, .. } = item else {
                        return None;
                    };
                    Name::new(alias.value.clone()).ok()
                })
                .map(|alias| Decl {
                    keyword: name("output"),
                    name: Some(alias),
                    ..Decl::new(name("output"))
                })
                .collect::<Vec<_>>();
            wrapper.children.splice(0..0, dynamic_outputs);
        }
        let mut expanded_instance = instance.clone();
        for part in &mut expanded_instance.children {
            if part.keyword.as_str() == "part" {
                part.props = self.expand_properties(caller, &part.props, outer_context, None);
            }
        }
        self.apply_part_overrides(&mut wrapper, &expanded_instance, schema, &context);
        wrapper
    }

    fn apply_part_overrides(
        &mut self,
        wrapper: &mut Decl,
        instance: &Decl,
        schema: &DefinitionSchema,
        context: &ExpansionContext,
    ) {
        for part in instance
            .children
            .iter()
            .filter(|child| child.keyword.as_str() == "part")
        {
            let Some(alias) = part.name.as_ref().map(Name::as_str) else {
                continue;
            };
            let Some(target) = schema.parts.get(alias) else {
                continue;
            };
            let declaration_path = target
                .declaration_path
                .iter()
                .map(|component| {
                    context
                        .private_names
                        .get(component)
                        .cloned()
                        .unwrap_or_else(|| component.clone())
                })
                .collect::<Vec<_>>();
            if let Some(declaration) =
                find_declaration_mut(&mut wrapper.children, &declaration_path)
            {
                for (name, value) in part.props.iter() {
                    declaration.props.set(name.clone(), value.clone());
                }
            }
        }
    }

    fn expand_properties(
        &mut self,
        owner: &SourceModuleId,
        props: &PropertyMap,
        context: Option<&ExpansionContext>,
        resolved: Option<&BTreeMap<String, crate::resolve::ResolvedValue>>,
    ) -> PropertyMap {
        let mut mark_block_ordinal = 0;
        self.expand_properties_at(
            owner,
            props,
            context,
            resolved,
            &[],
            &mut mark_block_ordinal,
        )
    }

    fn expand_properties_at(
        &mut self,
        owner: &SourceModuleId,
        props: &PropertyMap,
        context: Option<&ExpansionContext>,
        resolved: Option<&BTreeMap<String, crate::resolve::ResolvedValue>>,
        declaration_path: &[usize],
        mark_block_ordinal: &mut usize,
    ) -> PropertyMap {
        let mut output = PropertyMap::default();
        for (property, value) in props.iter() {
            let property_name = context
                .map(|context| rename_channel(property.as_str(), &context.channels))
                .unwrap_or_else(|| property.to_string());
            let value = if property.as_str() == "source"
                && let Some(context) = context
                && owner == &context.definition.module
                && matches!(value, Value::Array(_))
            {
                rename_path_value(value, &context.private_names)
            } else if property.as_str() == "target"
                && let Some(context) = context
                && matches!(value, Value::Array(_))
            {
                rename_path_value(value, context_names_for_owner(owner, context))
            } else if let Some(context) = context
                && owner == &context.definition.module
                && matches!(value, Value::Atom(_) | Value::Array(_))
                && resolved
                    .and_then(|properties| properties.get(property.as_str()))
                    .is_some_and(|value| {
                        matches!(
                            value,
                            crate::resolve::ResolvedValue::Reference(_)
                                | crate::resolve::ResolvedValue::Binding(_)
                        )
                    })
            {
                rename_reference_value(value, &context.private_names)
            } else {
                self.expand_value_at(
                    owner,
                    property.as_str(),
                    value,
                    context,
                    declaration_path,
                    mark_block_ordinal,
                )
            };
            output.set(name(&property_name), value);
        }
        output
    }

    fn expand_value(
        &mut self,
        owner: &SourceModuleId,
        value: &Value,
        context: Option<&ExpansionContext>,
    ) -> Value {
        let mut mark_block_ordinal = 0;
        self.expand_value_at(owner, "", value, context, &[], &mut mark_block_ordinal)
    }

    fn expand_value_at(
        &mut self,
        owner: &SourceModuleId,
        property: &str,
        value: &Value,
        context: Option<&ExpansionContext>,
        declaration_path: &[usize],
        mark_block_ordinal: &mut usize,
    ) -> Value {
        if property == "overlay"
            && let Value::Block { head, body } = value
        {
            let ordinal = *mark_block_ordinal;
            *mark_block_ordinal += 1;
            let mut block_path = declaration_path.to_vec();
            block_path.extend([MARK_BLOCK_PATH_SEGMENT, ordinal]);
            return Value::Block {
                head: head.as_ref().map(|head| {
                    Box::new(self.expand_value_at(
                        owner,
                        "",
                        head,
                        context,
                        declaration_path,
                        mark_block_ordinal,
                    ))
                }),
                body: self.expand_body_at(
                    owner,
                    &body.props,
                    &body.children,
                    &block_path,
                    None,
                    false,
                    context,
                    mark_block_ordinal,
                ),
            };
        }
        let Some(context) = context else {
            return match value {
                Value::Block { head, body } => Value::Block {
                    head: head.clone(),
                    body: self.expand_body_at(
                        owner,
                        &body.props,
                        &body.children,
                        declaration_path,
                        None,
                        false,
                        None,
                        mark_block_ordinal,
                    ),
                },
                _ => value.clone(),
            };
        };
        match value {
            Value::Atom(atom) => context
                .slots
                .get(atom.as_str())
                .filter(|slot| slot.shape != "block")
                .map(|slot| slot.value.clone())
                .unwrap_or_else(|| {
                    Value::Atom(name(&rename_channel(atom.as_str(), &context.channels)))
                }),
            Value::Expr(expression) => substitute_sql_expression(expression, owner, context)
                .map(|expression| Value::Expr(Box::new(expression)))
                .unwrap_or_else(|_| value.clone()),
            Value::Projection(projection) => substitute_sql_projection(projection, owner, context)
                .map(|projection| Value::Projection(Box::new(projection)))
                .unwrap_or_else(|_| value.clone()),
            Value::Query(query) => substitute_sql_query(query, owner, context)
                .map(|query| Value::Query(Box::new(query)))
                .unwrap_or_else(|_| value.clone()),
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .flat_map(|value| {
                        if let Value::Atom(atom) = value
                            && let Some(slot) = context.slots.get(atom.as_str())
                            && slot.shape == "expr_list"
                            && let Value::Array(values) = &slot.value
                        {
                            return values.clone();
                        }
                        vec![self.expand_value_at(
                            owner,
                            "",
                            value,
                            Some(context),
                            declaration_path,
                            mark_block_ordinal,
                        )]
                    })
                    .collect(),
            ),
            Value::Block { head, body } => Value::Block {
                head: head.as_ref().map(|head| {
                    Box::new(self.expand_value_at(
                        owner,
                        "",
                        head,
                        Some(context),
                        declaration_path,
                        mark_block_ordinal,
                    ))
                }),
                body: self.expand_body_at(
                    owner,
                    &body.props,
                    &body.children,
                    declaration_path,
                    None,
                    false,
                    Some(context),
                    mark_block_ordinal,
                ),
            },
            Value::Binding { kind, path, time } => Value::Binding {
                kind: *kind,
                path: rename_path(path, context_names_for_owner(owner, context)),
                time: *time,
            },
            Value::Ref { kind, path } => {
                if let Some(slot) = path
                    .first()
                    .and_then(|name| context.slots.get(name.as_str()))
                    && slot.shape == "ref"
                    && let Some(bound) = value_path(&slot.value)
                {
                    Value::Ref {
                        kind: *kind,
                        path: bound,
                    }
                } else {
                    Value::Ref {
                        kind: *kind,
                        path: rename_path(path, context_names_for_owner(owner, context)),
                    }
                }
            }
            Value::Channel { mode, expression } => Value::Channel {
                mode: *mode,
                expression: Box::new(self.expand_value(owner, expression, Some(context))),
            },
            Value::Pattern(value) => {
                Value::Pattern(Box::new(self.expand_value(owner, value, Some(context))))
            }
            Value::Call { function, args } => Value::Call {
                function: function.clone(),
                args: args
                    .iter()
                    .map(|value| self.expand_value(owner, value, Some(context)))
                    .collect(),
            },
            _ => value.clone(),
        }
    }

    fn resolved_declaration(
        &self,
        owner: &SourceModuleId,
        path: &[usize],
    ) -> Option<&crate::resolve::ResolvedDeclaration> {
        let roots = &self.resolved.source_modules.get(owner)?.roots;
        resolved_declaration_at(roots, path)
    }

    /// Return the reusable definition bound to a declaration.
    ///
    /// Most declarations have a resolved-tree path. Declarations supplied
    /// through a definition's block slot deliberately do not: their body is
    /// validated only after it is spliced into the expanded chart. Resolve
    /// those declarations against the caller module's already-built binding
    /// environment instead of accidentally treating their local block index
    /// as a module-item path.
    fn definition_binding(
        &self,
        owner: &SourceModuleId,
        declaration: &Decl,
        path: &[usize],
    ) -> Option<ModuleItemId> {
        if let Some(ResolvedKindBinding::Definition(definition)) = self
            .resolved_declaration(owner, path)
            .and_then(|resolved| resolved.kind_binding.as_ref())
        {
            return Some(definition.clone());
        }

        let kind = declaration.kind.as_ref()?;
        let category = match declaration.keyword.as_str() {
            "mark" => BindingCategory::NativeKind(avenger_chart_schema::NativeKindNamespace::Mark),
            "tool" => BindingCategory::NativeKind(avenger_chart_schema::NativeKindNamespace::Tool),
            "transform" => {
                BindingCategory::NativeKind(avenger_chart_schema::NativeKindNamespace::Transform)
            }
            _ => return None,
        };
        let environment = &self.resolved.source_modules.get(owner)?.local_bindings;
        let export = match kind.segments() {
            [name] => environment.local.get(&(category, name.to_string()))?,
            [namespace, member] => {
                let module = environment.namespaces.get(namespace.as_str())?;
                let index = match module {
                    ModuleId::Source(module) => &self.resolved.source_modules.get(module)?.exports,
                    ModuleId::Native(_) => return None,
                };
                index
                    .exports
                    .get(member.as_str())
                    .filter(|export| export.category == category)?
            }
            _ => return None,
        };
        let ModuleId::Source(module) = &export.module else {
            return None;
        };
        self.resolved
            .definitions
            .iter()
            .find_map(|(id, definition)| {
                (id.module == *module
                    && definition.source_name == export.name
                    && matches!(
                        (definition.kind, category),
                        (
                            DefinitionKind::Mark,
                            BindingCategory::NativeKind(
                                avenger_chart_schema::NativeKindNamespace::Mark
                            )
                        ) | (
                            DefinitionKind::Tool,
                            BindingCategory::NativeKind(
                                avenger_chart_schema::NativeKindNamespace::Tool
                            )
                        ) | (
                            DefinitionKind::Transform,
                            BindingCategory::NativeKind(
                                avenger_chart_schema::NativeKindNamespace::Transform
                            )
                        )
                    ))
                .then(|| id.clone())
            })
    }

    fn declaration_source_span(&self, owner: &SourceModuleId, declaration: &Decl) -> SourceSpan {
        self.module_graph
            .source_modules
            .get(owner)
            .and_then(|file| declaration_node_span(file, declaration))
            .or_else(|| self.module_graph.source_modules.get(owner).map(root_span))
            .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0))
    }

    fn chart_owner(
        &self,
        owner: &SourceModuleId,
        context: Option<&ExpansionContext>,
    ) -> SourceModuleId {
        context.map_or_else(|| owner.clone(), |context| context.chart.clone())
    }
}

fn slot_default(template: &Decl, name: &str) -> Option<Value> {
    template
        .children
        .iter()
        .find(|child| {
            child.keyword.as_str() == "slot" && child.name.as_ref().map(Name::as_str) == Some(name)
        })
        .and_then(|slot| slot.props.get("default"))
        .cloned()
}

fn substitute_sql_expression(
    expression: &crate::ast::SqlExpression,
    owner: &SourceModuleId,
    context: &ExpansionContext,
) -> Result<crate::ast::SqlExpression, crate::ast::AstError> {
    let sql = substitute_sql_macros(expression.canonical_sql(), owner, context, false)?;
    let mut expression = crate::ast::SqlExpression::parse(&sql)?;
    rewrite_definition_private_columns(&mut expression, context);
    Ok(expression)
}

fn substitute_sql_query(
    query: &crate::ast::SqlQuery,
    owner: &SourceModuleId,
    context: &ExpansionContext,
) -> Result<crate::ast::SqlQuery, crate::ast::AstError> {
    let sql = substitute_sql_macros(query.canonical_sql(), owner, context, true)?;
    let mut query = crate::ast::SqlQuery::parse(&sql)?;
    rewrite_definition_private_columns(&mut query, context);
    Ok(query)
}

fn substitute_sql_projection(
    projection: &crate::ast::SqlProjection,
    owner: &SourceModuleId,
    context: &ExpansionContext,
) -> Result<crate::ast::SqlProjection, crate::ast::AstError> {
    let sql = substitute_sql_macros(projection.canonical_sql(), owner, context, false)?;
    let mut projection = crate::ast::SqlProjection::parse(&sql)?;
    rewrite_definition_private_columns(&mut projection, context);
    Ok(projection)
}

trait RewritesSqlIdentifiers {
    fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>);
}

impl RewritesSqlIdentifiers for crate::ast::SqlExpression {
    fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        Self::rewrite_identifiers(self, replacement);
    }
}

impl RewritesSqlIdentifiers for crate::ast::SqlQuery {
    fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        Self::rewrite_identifiers(self, replacement);
    }
}

impl RewritesSqlIdentifiers for crate::ast::SqlProjection {
    fn rewrite_identifiers(&mut self, replacement: impl FnMut(&str) -> Option<String>) {
        Self::rewrite_identifiers(self, replacement);
    }
}

fn rewrite_definition_private_columns(
    sql: &mut impl RewritesSqlIdentifiers,
    context: &ExpansionContext,
) {
    sql.rewrite_identifiers(|identifier| {
        identifier
            .strip_prefix("__private_")
            .filter(|suffix| !suffix.is_empty())
            .map(|suffix| format!("__av_col_{}_{}", context.instance_identity, suffix))
    });
}

fn substitute_sql_macros(
    sql: String,
    owner: &SourceModuleId,
    context: &ExpansionContext,
    quote_output_aliases: bool,
) -> Result<String, crate::ast::AstError> {
    let mut replacements = BTreeMap::new();
    for (name, slot) in &context.slots {
        if slot.shape == "block" || slot.shape == "ref" {
            continue;
        }
        replacements.insert(name.clone(), slot_sql(slot, quote_output_aliases)?);
    }
    let private_names = context_names_for_owner(owner, context);
    for (logical, physical) in &context.channels {
        replacements.insert(logical.clone(), physical.clone());
        replacements.insert(format!("{logical}2"), format!("{physical}2"));
    }
    Ok(rewrite_identifiers(&sql, |identifier, binding| {
        if binding {
            return private_names.get(identifier).cloned();
        }
        if !binding && let Some(value) = replacements.get(identifier) {
            return Some(value.clone());
        }
        if !binding && let Some(value) = private_names.get(identifier) {
            return Some(value.clone());
        }
        None
    }))
}

fn slot_sql(slot: &BoundSlot, quote_output_aliases: bool) -> Result<String, crate::ast::AstError> {
    if slot.shape == "outputs"
        && let Value::Projection(projection) = &slot.value
    {
        return Ok(projection
            .items()
            .iter()
            .filter_map(|item| {
                let sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } = item else {
                    return None;
                };
                let alias = if quote_output_aliases {
                    format!("\"{}\"", alias.value.replace('"', "\"\""))
                } else {
                    alias.value.clone()
                };
                Some(format!(
                    "{} AS {alias}",
                    crate::ast::restore_bindings(expr.to_string(), projection.bindings()),
                ))
            })
            .collect::<Vec<_>>()
            .join(", "));
    }
    if slot.shape == "expr_list" {
        let Value::Array(values) = &slot.value else {
            return Ok(crate::print::print_value(&slot.value));
        };
        return Ok(values
            .iter()
            .map(crate::print::print_value)
            .collect::<Vec<_>>()
            .join(", "));
    }
    if slot.shape == "enum"
        && let Value::Atom(value) = &slot.value
    {
        return Ok(format!("'{}'", value.as_str().replace('\'', "''")));
    }
    Ok(crate::print::print_value(&slot.value))
}

fn rewrite_identifiers(
    source: &str,
    mut replacement: impl FnMut(&str, bool) -> Option<String>,
) -> String {
    let chars = source.char_indices().collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    let mut index = 0;
    while index < chars.len() {
        let (offset, character) = chars[index];
        if matches!(character, '\'' | '"') {
            let quote = character;
            index += 1;
            while index < chars.len() {
                let (_, current) = chars[index];
                index += 1;
                if current == quote {
                    if index < chars.len() && chars[index].1 == quote {
                        index += 1;
                    } else {
                        break;
                    }
                }
            }
            continue;
        }
        if character.is_ascii_alphabetic() || character == '_' {
            let start = offset;
            index += 1;
            while index < chars.len()
                && (chars[index].1.is_ascii_alphanumeric() || chars[index].1 == '_')
            {
                index += 1;
            }
            let end = chars.get(index).map_or(source.len(), |(offset, _)| *offset);
            let preceded_by_binding = start > 0 && source.as_bytes()[start - 1] == b'$';
            if let Some(value) = replacement(&source[start..end], preceded_by_binding) {
                output.push_str(&source[cursor..start]);
                output.push_str(&value);
                cursor = end;
            }
            continue;
        }
        index += 1;
    }
    output.push_str(&source[cursor..]);
    output
}

fn rename_channel(name: &str, channels: &BTreeMap<String, String>) -> String {
    channels
        .get(name)
        .cloned()
        .or_else(|| {
            let logical = name.strip_suffix('2')?;
            channels.get(logical).map(|physical| format!("{physical}2"))
        })
        .unwrap_or_else(|| name.to_owned())
}

fn value_atom_name(value: &Value) -> Option<&str> {
    match value {
        Value::Atom(name) => Some(name.as_str()),
        _ => None,
    }
}

fn value_path(value: &Value) -> Option<Vec<Name>> {
    match value {
        Value::Atom(name) => Some(vec![name.clone()]),
        Value::Ref { path, .. } | Value::Binding { path, .. } => Some(path.clone()),
        Value::Array(values) => values
            .iter()
            .map(|value| match value {
                Value::Atom(name) => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

fn find_declaration_mut<'a>(children: &'a mut [Decl], path: &[String]) -> Option<&'a mut Decl> {
    let (first, rest) = path.split_first()?;
    let declaration = children
        .iter_mut()
        .find(|child| child.name.as_ref().map(Name::as_str) == Some(first))?;
    if rest.is_empty() {
        Some(declaration)
    } else {
        find_declaration_mut(&mut declaration.children, rest)
    }
}

fn targetable_declaration(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "mark" | "tool" | "param" | "store" | "selection" | "widget"
    )
}

fn binds_private_name(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "param"
            | "store"
            | "selection"
            | "dimension"
            | "mark"
            | "transform"
            | "view"
            | "widget"
            | "resource"
            | "variable"
            | "derive"
            | "tool"
            | "on"
            | "cell"
            | "plot"
            | "table"
    )
}

fn definition_private_names(template: &Decl, instance_identity: &str) -> BTreeMap<String, String> {
    let mut names = BTreeSet::new();
    declaration_preorder(&template.children, &mut |declaration| {
        if binds_private_name(declaration)
            && let Some(source_name) = declaration.name.as_ref().map(Name::as_str)
        {
            names.insert(source_name.to_owned());
        }
        true
    });
    names
        .into_iter()
        .map(|source_name| {
            let private = format!("__av_{instance_identity}_{source_name}");
            (source_name, private)
        })
        .collect()
}

fn generated_name_collision(
    module_graph: &ParsedModuleGraph,
    caller: &SourceModuleId,
    private_names: &BTreeMap<String, String>,
    outer_context: Option<&ExpansionContext>,
) -> Option<String> {
    let generated = private_names
        .values()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut collision = outer_context
        .into_iter()
        .flat_map(|context| context.private_names.values())
        .find(|name| generated.contains(name.as_str()))
        .cloned();
    if collision.is_some() {
        return collision;
    }
    let file = module_graph.source_modules.get(caller)?;
    collision = file
        .parsed
        .ast
        .items
        .iter()
        .find_map(|item| declaration_generated_name_collision(&item.declaration, &generated));
    collision
}

fn declaration_generated_name_collision(
    declaration: &Decl,
    generated: &BTreeSet<&str>,
) -> Option<String> {
    declaration
        .name
        .as_ref()
        .map(Name::as_str)
        .filter(|name| generated.contains(name))
        .map(str::to_owned)
        .or_else(|| {
            declaration
                .props
                .iter()
                .find_map(|(_, value)| value_generated_name_collision(value, generated))
        })
        .or_else(|| {
            declaration
                .children
                .iter()
                .find_map(|child| declaration_generated_name_collision(child, generated))
        })
}

fn value_generated_name_collision(value: &Value, generated: &BTreeSet<&str>) -> Option<String> {
    match value {
        Value::Array(values) | Value::Call { args: values, .. } => values
            .iter()
            .find_map(|value| value_generated_name_collision(value, generated)),
        Value::Block { head, body } => head
            .as_deref()
            .and_then(|head| value_generated_name_collision(head, generated))
            .or_else(|| {
                body.props
                    .iter()
                    .find_map(|(_, value)| value_generated_name_collision(value, generated))
            })
            .or_else(|| {
                body.children
                    .iter()
                    .find_map(|child| declaration_generated_name_collision(child, generated))
            }),
        Value::Channel {
            expression: value, ..
        }
        | Value::Pattern(value) => value_generated_name_collision(value, generated),
        _ => None,
    }
}

fn rename_path(path: &[Name], private_names: &BTreeMap<String, String>) -> Vec<Name> {
    path.iter()
        .map(|component| {
            private_names
                .get(component.as_str())
                .map_or_else(|| component.clone(), |renamed| name(renamed))
        })
        .collect()
}

fn context_names_for_owner<'a>(
    owner: &SourceModuleId,
    context: &'a ExpansionContext,
) -> &'a BTreeMap<String, String> {
    if owner == &context.definition.module {
        &context.private_names
    } else {
        &context.exposed_private_names
    }
}

fn rename_path_value(value: &Value, private_names: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Array(components) => Value::Array(
            components
                .iter()
                .map(|component| match component {
                    Value::Atom(component) => Value::Atom(
                        private_names
                            .get(component.as_str())
                            .map_or_else(|| component.clone(), |renamed| name(renamed)),
                    ),
                    _ => component.clone(),
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

fn rename_reference_value(value: &Value, private_names: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Atom(component) => Value::Atom(
            private_names
                .get(component.as_str())
                .map_or_else(|| component.clone(), |renamed| name(renamed)),
        ),
        Value::Array(_) => rename_path_value(value, private_names),
        Value::Binding { kind, path, time } => Value::Binding {
            kind: *kind,
            path: rename_path(path, private_names),
            time: *time,
        },
        Value::Ref { kind, path } => Value::Ref {
            kind: *kind,
            path: rename_path(path, private_names),
        },
        _ => value.clone(),
    }
}

fn expansion_identity(
    definition_seed: &str,
    caller: &str,
    stable_path: &str,
    outer_instance: &str,
) -> String {
    let mut hash = Sha256::new();
    for part in [definition_seed, caller, stable_path, outer_instance] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())[..12].to_owned()
}

fn stable_expansion_path(file: Option<&ParsedModule>, path: &[usize]) -> String {
    let Some(file) = file else {
        return path
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(".");
    };
    let mut components = Vec::with_capacity(path.len());
    for (depth, index) in path.iter().copied().enumerate() {
        let siblings = if depth == 0 {
            file.parsed
                .ast
                .items
                .iter()
                .map(|item| &item.declaration)
                .collect::<Vec<_>>()
        } else {
            module_declaration_at(&file.parsed.ast, &path[..depth])
                .map(|parent| parent.children.iter().collect())
                .unwrap_or_default()
        };
        let Some(declaration) = siblings.get(index) else {
            components.push(format!("missing:{index}"));
            continue;
        };
        let signature = format!(
            "{}:{}:{}",
            declaration.keyword,
            declaration.name.as_ref().map_or("", Name::as_str),
            declaration.kind.as_ref().map_or("", |kind| kind.as_str()),
        );
        let ordinal = siblings[..index]
            .iter()
            .filter(|candidate| {
                format!(
                    "{}:{}:{}",
                    candidate.keyword,
                    candidate.name.as_ref().map_or("", Name::as_str),
                    candidate.kind.as_ref().map_or("", |kind| kind.as_str()),
                ) == signature
            })
            .count();
        components.push(format!("{signature}#{ordinal}"));
    }
    components.join("/")
}

fn name(value: &str) -> Name {
    Name::new(value.to_owned()).expect("expansion generates valid Avenger names")
}

fn content_hash(text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(text.as_bytes());
    format!("sha256:{:x}", hash.finalize())
}

fn module_declaration_at<'a>(file: &'a File, path: &[usize]) -> Option<&'a Decl> {
    let (first, rest) = path.split_first()?;
    let mut declaration = &file.items.get(*first)?.declaration;
    for index in rest {
        declaration = declaration.children.get(*index)?;
    }
    Some(declaration)
}

fn resolved_declaration_at<'a>(
    roots: &'a [crate::resolve::ResolvedDeclaration],
    path: &[usize],
) -> Option<&'a crate::resolve::ResolvedDeclaration> {
    let (first, rest) = path.split_first()?;
    let mut declaration = roots.get(*first)?;
    let mut index = 0;
    while index < rest.len() {
        if rest[index] == MARK_BLOCK_PATH_SEGMENT {
            let block = *rest.get(index + 1)?;
            let child = *rest.get(index + 2)?;
            declaration = resolved_mark_blocks(declaration).get(block)?.get(child)?;
            index += 3;
        } else {
            declaration = declaration.children.get(rest[index])?;
            index += 1;
        }
    }
    Some(declaration)
}

fn resolved_mark_blocks(
    declaration: &crate::resolve::ResolvedDeclaration,
) -> Vec<&[crate::resolve::ResolvedDeclaration]> {
    fn collect<'a>(
        value: &'a crate::resolve::ResolvedValue,
        output: &mut Vec<&'a [crate::resolve::ResolvedDeclaration]>,
    ) {
        match value {
            crate::resolve::ResolvedValue::Object { properties, .. } => {
                for (name, value) in properties {
                    if name == "overlay"
                        && let crate::resolve::ResolvedValue::Object { children, .. } = value
                    {
                        output.push(children);
                    } else {
                        collect(value, output);
                    }
                }
            }
            crate::resolve::ResolvedValue::Array(values) => {
                for value in values {
                    collect(value, output);
                }
            }
            crate::resolve::ResolvedValue::Channel {
                expression: value, ..
            }
            | crate::resolve::ResolvedValue::Pattern(value) => collect(value, output),
            crate::resolve::ResolvedValue::ChannelValue(channel) => {
                collect(&channel.head.expression, output);
                if let Some(otherwise) = &channel.otherwise {
                    collect(&otherwise.expression, output);
                }
                for condition in &channel.conditions {
                    collect(&condition.predicate, output);
                    collect(&condition.branch.expression, output);
                }
                for value in channel.configuration.values() {
                    collect(value, output);
                }
            }
            crate::resolve::ResolvedValue::Call { args, .. } => {
                for value in args {
                    collect(value, output);
                }
            }
            _ => {}
        }
    }

    let mut blocks = Vec::new();
    for value in declaration.properties.values() {
        collect(value, &mut blocks);
    }
    blocks
}

fn declaration_span(file: &ParsedModule, path: &[usize]) -> Option<SourceSpan> {
    let declaration = module_declaration_at(&file.parsed.ast, path)?;
    declaration_node_span(file, declaration)
}

fn declaration_node_span(file: &ParsedModule, declaration: &Decl) -> Option<SourceSpan> {
    let mut spans = file
        .parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| match file.parsed.source_map.role(id) {
            Some(AstNodeRole::Declaration(keyword)) if *keyword == declaration.keyword => {
                Some(span)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    let mut ordinal = 0usize;
    module_declaration_preorder(&file.parsed.ast, &mut |candidate| {
        if std::ptr::eq(candidate, declaration) {
            return false;
        }
        if candidate.keyword == declaration.keyword {
            ordinal += 1;
        }
        true
    });
    spans.get(ordinal).copied()
}

fn module_declaration_preorder<'a>(file: &'a File, visit: &mut impl FnMut(&'a Decl) -> bool) {
    for item in &file.items {
        let declaration = &item.declaration;
        if !visit(declaration) {
            return;
        }
        declaration_preorder(&declaration.children, visit);
    }
}

fn declaration_preorder<'a>(declarations: &'a [Decl], visit: &mut impl FnMut(&'a Decl) -> bool) {
    for declaration in declarations {
        if !visit(declaration) {
            return;
        }
        declaration_preorder(&declaration.children, visit);
    }
}

fn root_span(file: &ParsedModule) -> SourceSpan {
    file.parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| {
            matches!(
                file.parsed.source_map.role(id),
                Some(AstNodeRole::Declaration(_))
            )
            .then_some(span)
        })
        .min_by_key(|span| span.range.start)
        .unwrap_or_else(|| SourceSpan::empty(file.source, 0))
}
