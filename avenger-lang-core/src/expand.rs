//! Resolved-guided source expansion for imported definitions.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{
    Diagnostic, ExpansionOrImportFrame, SourceFile, SourceId, SourceLabel, SourceMap, SourceOrigin,
    SourceSpan,
    ast::{AstNodeRole, Body, Decl, File, Name, PropertyMap, Root, Value, Visibility},
    print::print_file,
    project::{DefinitionKind, ParsedProject, ProjectFile, ProjectFileId, ProjectFileKind},
    resolve::{DefinitionSchema, ResolvedProject},
    syntax::parse_file,
};

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

#[derive(Clone, Debug)]
pub struct ExpandedProject {
    pub project: ParsedProject,
    pub texts: BTreeMap<ProjectFileId, String>,
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
    owner: ProjectFileId,
    exposes: Vec<String>,
}

#[derive(Clone)]
struct ExpansionContext {
    caller: ProjectFileId,
    chart: ProjectFileId,
    definition: ProjectFileId,
    definition_name: String,
    instance_name: String,
    private_names: BTreeMap<String, String>,
    exposed_private_names: BTreeMap<String, String>,
    slots: BTreeMap<String, BoundSlot>,
    channels: BTreeMap<String, String>,
    blocks: BTreeMap<String, BoundSlot>,
    definition_span: SourceSpan,
    instantiation_span: SourceSpan,
}

pub fn expand_project(
    project: &ParsedProject,
    resolved: &ResolvedProject,
) -> Result<ExpandedProject, ExpansionFailure> {
    let mut expander = Expander {
        project,
        resolved,
        diagnostics: Vec::new(),
        pending_origins: BTreeMap::new(),
    };
    expander.expand()
}

struct Expander<'a> {
    project: &'a ParsedProject,
    resolved: &'a ResolvedProject,
    diagnostics: Vec<Diagnostic>,
    pending_origins: BTreeMap<ProjectFileId, Vec<PendingOrigin>>,
}

impl Expander<'_> {
    fn expand(&mut self) -> Result<ExpandedProject, ExpansionFailure> {
        let mut next_source = self
            .project
            .sources
            .iter()
            .map(|(id, _)| id.get())
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let mut sources = self.project.sources.clone();
        let mut files = BTreeMap::new();
        let mut texts = BTreeMap::new();
        let definition_sources = self
            .project
            .files
            .values()
            .filter(|file| matches!(file.kind, ProjectFileKind::Definition(_)))
            .map(|file| file.source)
            .collect::<BTreeSet<_>>();

        for (file_id, file) in &self.project.files {
            match file.kind {
                ProjectFileKind::Definition(_) => {}
                ProjectFileKind::Data => {
                    files.insert(file_id.clone(), file.clone());
                }
                ProjectFileKind::Chart => {
                    let Root::Chart(root) = &file.parsed.ast.root else {
                        continue;
                    };
                    let imports = retained_imports(self.project, file, &definition_sources);
                    let expanded_root = self.expand_declaration(
                        file_id,
                        root,
                        &[0],
                        None,
                        PendingOrigin {
                            authored: declaration_span(file, &[0])
                                .unwrap_or_else(|| root_span(file)),
                            definition: None,
                            instantiation: None,
                        },
                    );
                    let ast = File {
                        version: file.parsed.ast.version,
                        name: file.parsed.ast.name.clone(),
                        imports,
                        root: Root::Chart(expanded_root),
                    };
                    let text = print_file(&ast);
                    let source = SourceId::new(next_source);
                    next_source = next_source.saturating_add(1);
                    let origin =
                        SourceOrigin::Memory(format!("<expanded:{}>", file.origin.canonical_uri()));
                    let source_file = SourceFile::new(source, origin, text.clone());
                    let parsed = parse_file(&source_file).map_err(|error| ExpansionFailure {
                        diagnostics: vec![error.diagnostic().clone()],
                        sources: sources.clone(),
                    })?;
                    sources
                        .insert(source_file)
                        .map_err(|error| ExpansionFailure {
                            diagnostics: vec![Diagnostic::error(
                                "AVENGER-EXPAND-002",
                                "expanded source identity collision",
                                SourceLabel::new(root_span(file), error.to_string()),
                            )],
                            sources: sources.clone(),
                        })?;
                    files.insert(
                        file_id.clone(),
                        ProjectFile {
                            id: file_id.clone(),
                            source,
                            origin: file.origin.clone(),
                            kind: ProjectFileKind::Chart,
                            content_version: format!("expanded:{}", file.content_version),
                            content_sha256: content_hash(&text),
                            parsed,
                        },
                    );
                    texts.insert(file_id.clone(), text);
                }
            }
        }

        if !self.diagnostics.is_empty() {
            return Err(ExpansionFailure {
                diagnostics: std::mem::take(&mut self.diagnostics),
                sources,
            });
        }

        let retained_sources = files
            .values()
            .map(|file| file.source)
            .collect::<BTreeSet<_>>();
        let imports = self
            .project
            .imports
            .iter()
            .filter(|edge| {
                !definition_sources.contains(&edge.imported)
                    && retained_sources.contains(&edge.imported)
            })
            .cloned()
            .collect::<Vec<_>>();
        let mut source_map = ExpansionSourceMap::default();
        for (file_id, pending) in &self.pending_origins {
            let Some(file) = files.get(file_id) else {
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
            expanded_spans.sort_by_key(|span| span.range.start);
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

        Ok(ExpandedProject {
            project: ParsedProject {
                sources,
                files,
                imports,
                chart_roots: self.project.chart_roots.clone(),
                ambient_data: self.project.ambient_data.clone(),
                ambient_catalog: self.project.ambient_catalog.clone(),
                fingerprint: self.project.fingerprint.clone(),
            },
            texts,
            source_map,
        })
    }

    fn expand_declaration(
        &mut self,
        owner: &ProjectFileId,
        declaration: &Decl,
        path: &[usize],
        context: Option<&ExpansionContext>,
        origin: PendingOrigin,
    ) -> Decl {
        if let Some(kind) = declaration.kind.as_ref().map(Name::as_str)
            && let Some(definition) = self.imported_definition(owner, kind)
        {
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
            && owner == &context.definition
            && let Some(source_name) = declaration.name.as_ref().map(Name::as_str)
            && (binds_private_name(declaration) || declaration.keyword.as_str() == "set")
            && let Some(private_name) = context.private_names.get(source_name)
        {
            substituted.name = Some(name(private_name));
        }
        let resolved_properties = self
            .resolved_declaration(owner, path)
            .map(|declaration| declaration.properties.clone());
        substituted.props = self.expand_properties(
            owner,
            &declaration.props,
            context,
            resolved_properties.as_ref(),
        );
        self.pending_origins
            .entry(self.chart_owner(owner, context))
            .or_default()
            .push(origin);
        let expands_private = context.is_some_and(|context| {
            owner == &context.definition
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

    fn expand_body(
        &mut self,
        owner: &ProjectFileId,
        props: &PropertyMap,
        children: &[Decl],
        parent_path: &[usize],
        source_indices: Option<&[usize]>,
        inside_private: bool,
        context: Option<&ExpansionContext>,
    ) -> Body {
        let mut body = Body {
            props: self.expand_properties(owner, props, context, None),
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
                    &[],
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
                && targetable_declaration(&expanded)
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
        caller: &ProjectFileId,
        instance: &Decl,
        instance_path: &[usize],
        definition_id: ProjectFileId,
        outer_context: Option<&ExpansionContext>,
        origin: PendingOrigin,
    ) -> Decl {
        let Some(definition_file) = self.project.files.get(&definition_id) else {
            return instance.clone();
        };
        let Root::Define(template) = &definition_file.parsed.ast.root else {
            return instance.clone();
        };
        let Some(schema) = self.resolved.definitions.get(&definition_id) else {
            return instance.clone();
        };
        let definition_span =
            declaration_span(definition_file, &[0]).unwrap_or_else(|| root_span(definition_file));
        let instantiation_span = origin.authored;
        let source_instance_name = instance
            .name
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("anonymous_{}", schema.source_name));
        let instance_identity = expansion_identity(
            schema.local_seed.as_str(),
            caller.as_str(),
            &stable_expansion_path(self.project.files.get(caller), instance_path),
            outer_context.map_or("", |context| context.instance_name.as_str()),
        );
        let instance_name = format!("{source_instance_name}_{instance_identity}");
        let private_names = definition_private_names(template, &instance_identity);
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
                definition_id.clone()
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
                    definition_name: schema.source_name.clone(),
                    instance_name: instance_name.clone(),
                    private_names: private_names.clone(),
                    exposed_private_names: BTreeMap::new(),
                    slots: slots.clone(),
                    channels: channels.clone(),
                    blocks: blocks.clone(),
                    definition_span,
                    instantiation_span,
                };
                self.expand_value(&definition_id, raw_value, Some(&default_context))
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
            definition_name: schema.source_name.clone(),
            instance_name,
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
            DefinitionKind::Mark => Decl::new(name("group")),
            DefinitionKind::Tool => {
                let mut declaration = Decl::new(name("tool"));
                declaration.kind = Some(name("behavior"));
                declaration
            }
            DefinitionKind::Transform => {
                let mut declaration = Decl::new(name("transform"));
                declaration.kind = Some(name("pipeline"));
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
                    "visible"
                        | "details"
                        | "zindex"
                        | "facet_data_scope"
                        | "geometry_space"
                        | "exclude_from_scale_domains"
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
            &definition_id,
            &PropertyMap::default(),
            &children,
            &[0],
            Some(&child_indices),
            false,
            Some(&context),
        );
        wrapper.children = expanded.children;
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
        owner: &ProjectFileId,
        props: &PropertyMap,
        context: Option<&ExpansionContext>,
        resolved: Option<&BTreeMap<String, crate::resolve::ResolvedValue>>,
    ) -> PropertyMap {
        let mut output = PropertyMap::default();
        for (property, value) in props.iter() {
            let property_name = context
                .map(|context| rename_channel(property.as_str(), &context.channels))
                .unwrap_or_else(|| property.to_string());
            let value = if property.as_str() == "source"
                && let Some(context) = context
                && owner == &context.definition
                && matches!(value, Value::Array(_))
            {
                rename_path_value(value, &context.private_names)
            } else if property.as_str() == "target"
                && let Some(context) = context
                && matches!(value, Value::Array(_))
            {
                rename_path_value(value, context_names_for_owner(owner, context))
            } else if let Some(context) = context
                && owner == &context.definition
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
                self.expand_value(owner, value, context)
            };
            output.set(name(&property_name), value);
        }
        output
    }

    fn expand_value(
        &mut self,
        owner: &ProjectFileId,
        value: &Value,
        context: Option<&ExpansionContext>,
    ) -> Value {
        let Some(context) = context else {
            return match value {
                Value::Block { head, body } => Value::Block {
                    head: head.clone(),
                    body: self.expand_body(
                        owner,
                        &body.props,
                        &body.children,
                        &[],
                        None,
                        false,
                        None,
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
            Value::Expr(expression) => substitute_sql(expression.canonical_sql(), owner, context)
                .and_then(|sql| crate::ast::SqlExpression::parse(&sql))
                .map(|expression| Value::Expr(Box::new(expression)))
                .unwrap_or_else(|_| value.clone()),
            Value::Query(query) => substitute_sql(query.canonical_sql(), owner, context)
                .and_then(|sql| crate::ast::SqlQuery::parse(&sql))
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
                        vec![self.expand_value(owner, value, Some(context))]
                    })
                    .collect(),
            ),
            Value::Block { head, body } => Value::Block {
                head: head
                    .as_ref()
                    .map(|head| Box::new(self.expand_value(owner, head, Some(context)))),
                body: self.expand_body(
                    owner,
                    &body.props,
                    &body.children,
                    &[],
                    None,
                    false,
                    Some(context),
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
            Value::Visual(value) => {
                Value::Visual(Box::new(self.expand_value(owner, value, Some(context))))
            }
            Value::Pattern(value) => {
                Value::Pattern(Box::new(self.expand_value(owner, value, Some(context))))
            }
            Value::Call { function, args } => {
                let function = context
                    .slots
                    .get(function.as_str())
                    .filter(|slot| slot.shape == "function")
                    .and_then(|slot| value_atom_name(&slot.value))
                    .map(name)
                    .unwrap_or_else(|| function.clone());
                Value::Call {
                    function,
                    args: args
                        .iter()
                        .map(|value| self.expand_value(owner, value, Some(context)))
                        .collect(),
                }
            }
            _ => value.clone(),
        }
    }

    fn imported_definition(&self, owner: &ProjectFileId, kind: &str) -> Option<ProjectFileId> {
        self.resolved
            .files
            .get(owner)?
            .imports
            .get(kind)
            .filter(|file| self.resolved.definitions.contains_key(*file))
            .cloned()
    }

    fn resolved_declaration(
        &self,
        owner: &ProjectFileId,
        path: &[usize],
    ) -> Option<&crate::resolve::ResolvedDeclaration> {
        let roots = &self.resolved.files.get(owner)?.roots;
        resolved_declaration_at(roots, path)
    }

    fn declaration_source_span(&self, owner: &ProjectFileId, declaration: &Decl) -> SourceSpan {
        self.project
            .files
            .get(owner)
            .and_then(|file| declaration_node_span(file, declaration))
            .or_else(|| self.project.files.get(owner).map(root_span))
            .unwrap_or_else(|| SourceSpan::empty(SourceId::new(0), 0))
    }

    fn chart_owner(
        &self,
        owner: &ProjectFileId,
        context: Option<&ExpansionContext>,
    ) -> ProjectFileId {
        if self
            .project
            .files
            .get(owner)
            .is_some_and(|file| matches!(file.kind, ProjectFileKind::Chart))
        {
            owner.clone()
        } else {
            context.map_or_else(|| owner.clone(), |context| context.chart.clone())
        }
    }
}

fn retained_imports(
    project: &ParsedProject,
    file: &ProjectFile,
    definition_sources: &BTreeSet<SourceId>,
) -> Vec<crate::ast::Import> {
    let mut edges = project
        .imports
        .iter()
        .filter(|edge| edge.importer == file.source)
        .collect::<Vec<_>>();
    edges.sort_by_key(|edge| edge.site.range.start);
    file.parsed
        .ast
        .imports
        .iter()
        .zip(edges)
        .filter(|(_, edge)| !definition_sources.contains(&edge.imported))
        .map(|(import, _)| import.clone())
        .collect()
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

fn substitute_sql(
    sql: String,
    owner: &ProjectFileId,
    context: &ExpansionContext,
) -> Result<String, crate::ast::AstError> {
    let mut replacements = BTreeMap::new();
    for (name, slot) in &context.slots {
        if slot.shape == "block" || slot.shape == "ref" {
            continue;
        }
        replacements.insert(name.clone(), slot_sql(slot)?);
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
        let prefix = format!("__{}_", context.definition_name);
        (!binding)
            .then(|| {
                identifier
                    .strip_prefix(&prefix)
                    .map(|suffix| format!("__{}_{}", context.instance_name, suffix))
            })
            .flatten()
    }))
}

fn slot_sql(slot: &BoundSlot) -> Result<String, crate::ast::AstError> {
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
        "mark" | "group" | "tool" | "param" | "store" | "selection" | "widget"
    )
}

fn binds_private_name(declaration: &Decl) -> bool {
    matches!(
        declaration.keyword.as_str(),
        "param"
            | "store"
            | "selection"
            | "dimension"
            | "group"
            | "overlay"
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
    owner: &ProjectFileId,
    context: &'a ExpansionContext,
) -> &'a BTreeMap<String, String> {
    if owner == &context.definition {
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

fn stable_expansion_path(file: Option<&ProjectFile>, path: &[usize]) -> String {
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
            file.parsed.ast.root.declarations()
        } else {
            declaration_at(file.parsed.ast.root.declarations(), &path[..depth])
                .map_or(&[][..], |parent| parent.children.as_slice())
        };
        let Some(declaration) = siblings.get(index) else {
            components.push(format!("missing:{index}"));
            continue;
        };
        let signature = format!(
            "{}:{}:{}",
            declaration.keyword,
            declaration.name.as_ref().map_or("", Name::as_str),
            declaration.kind.as_ref().map_or("", Name::as_str),
        );
        let ordinal = siblings[..index]
            .iter()
            .filter(|candidate| {
                format!(
                    "{}:{}:{}",
                    candidate.keyword,
                    candidate.name.as_ref().map_or("", Name::as_str),
                    candidate.kind.as_ref().map_or("", Name::as_str),
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

fn declaration_at<'a>(roots: &'a [Decl], path: &[usize]) -> Option<&'a Decl> {
    let (first, rest) = path.split_first()?;
    let mut declaration = roots.get(*first)?;
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
    for index in rest {
        declaration = declaration.children.get(*index)?;
    }
    Some(declaration)
}

fn declaration_span(file: &ProjectFile, path: &[usize]) -> Option<SourceSpan> {
    let declaration = declaration_at(file.parsed.ast.root.declarations(), path)?;
    declaration_node_span(file, declaration)
}

fn declaration_node_span(file: &ProjectFile, declaration: &Decl) -> Option<SourceSpan> {
    let mut spans = file
        .parsed
        .source_map
        .iter()
        .filter_map(|(id, span)| match file.parsed.source_map.role(id) {
            Some(AstNodeRole::Declaration(keyword)) if keyword == &declaration.keyword => {
                Some(span)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| (span.range.start, std::cmp::Reverse(span.range.end)));
    let mut ordinal = 0usize;
    declaration_preorder(file.parsed.ast.root.declarations(), &mut |candidate| {
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

fn declaration_preorder<'a>(declarations: &'a [Decl], visit: &mut impl FnMut(&'a Decl) -> bool) {
    for declaration in declarations {
        if !visit(declaration) {
            return;
        }
        declaration_preorder(&declaration.children, visit);
    }
}

fn root_span(file: &ProjectFile) -> SourceSpan {
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
