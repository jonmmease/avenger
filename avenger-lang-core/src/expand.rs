//! Resolved-guided source expansion for imported definitions.

use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::{
    Diagnostic, SourceFile, SourceId, SourceLabel, SourceMap, SourceOrigin, SourceSpan,
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
}

#[derive(Clone)]
struct ExpansionContext {
    caller: ProjectFileId,
    chart: ProjectFileId,
    definition_name: String,
    instance_name: String,
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
            return self.instantiate_definition(owner, declaration, definition, context, origin);
        }

        let mut substituted = declaration.clone();
        substituted.props = self.expand_properties(owner, &declaration.props, context);
        self.pending_origins
            .entry(self.chart_owner(owner, context))
            .or_default()
            .push(origin);
        let body = self.expand_body(
            owner,
            &PropertyMap::default(),
            &declaration.children,
            context,
        );
        substituted.children = body.children;
        let _ = path;
        substituted
    }

    fn expand_body(
        &mut self,
        owner: &ProjectFileId,
        props: &PropertyMap,
        children: &[Decl],
        context: Option<&ExpansionContext>,
    ) -> Body {
        let mut body = Body {
            props: self.expand_properties(owner, props, context),
            children: Vec::new(),
        };
        for (index, child) in children.iter().enumerate() {
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
                    let selected_body =
                        self.expand_body(owner, &arm.props, &arm.children, Some(context));
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
                    Some(context),
                );
                for mut supplied in supplied_body.children {
                    if block.owner == context.caller && supplied.name.is_some() {
                        supplied.visibility = Visibility::Public;
                    }
                    body.children.push(supplied);
                }
                continue;
            }
            let child_origin = context.map_or_else(
                || PendingOrigin {
                    authored: self.declaration_span(owner, &[0, index]),
                    definition: None,
                    instantiation: None,
                },
                |context| PendingOrigin {
                    authored: self.declaration_span(owner, &[0, index]),
                    definition: Some(context.definition_span),
                    instantiation: Some(context.instantiation_span),
                },
            );
            let mut expanded =
                self.expand_declaration(owner, child, &[index], context, child_origin);
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
        let instance_name = instance
            .name
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("anonymous_{}", schema.source_name));
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
                    definition_name: schema.source_name.clone(),
                    instance_name: instance_name.clone(),
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
            };
            if slot.shape == "block" {
                blocks.insert(name.clone(), bound.clone());
            }
            slots.insert(name.clone(), bound);
        }
        let context = ExpansionContext {
            caller: caller.clone(),
            chart,
            definition_name: schema.source_name.clone(),
            instance_name,
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

        let interface = template
            .children
            .iter()
            .filter(|child| matches!(child.keyword.as_str(), "output" | "export"))
            .cloned()
            .collect::<Vec<_>>();
        let executable = template
            .children
            .iter()
            .filter(|child| {
                !matches!(
                    child.keyword.as_str(),
                    "slot" | "channel" | "output" | "export"
                )
            })
            .cloned()
            .collect::<Vec<_>>();
        let expanded = self.expand_body(
            &definition_id,
            &PropertyMap::default(),
            &interface.into_iter().chain(executable).collect::<Vec<_>>(),
            Some(&context),
        );
        wrapper.children = expanded.children;
        let mut expanded_instance = instance.clone();
        for part in &mut expanded_instance.children {
            if part.keyword.as_str() == "part" {
                part.props = self.expand_properties(caller, &part.props, outer_context);
            }
        }
        self.apply_part_overrides(&mut wrapper, &expanded_instance, schema);
        wrapper
    }

    fn apply_part_overrides(
        &mut self,
        wrapper: &mut Decl,
        instance: &Decl,
        schema: &DefinitionSchema,
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
            if let Some(declaration) =
                find_declaration_mut(&mut wrapper.children, &target.declaration_path)
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
    ) -> PropertyMap {
        let mut output = PropertyMap::default();
        for (property, value) in props.iter() {
            let property_name = context
                .map(|context| rename_channel(property.as_str(), &context.channels))
                .unwrap_or_else(|| property.to_string());
            output.set(
                name(&property_name),
                self.expand_value(owner, value, context),
            );
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
                    body: self.expand_body(owner, &body.props, &body.children, None),
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
            Value::Expr(expression) => substitute_sql(expression.canonical_sql(), context)
                .and_then(|sql| crate::ast::SqlExpression::parse(&sql))
                .map(|expression| Value::Expr(Box::new(expression)))
                .unwrap_or_else(|_| value.clone()),
            Value::Query(query) => substitute_sql(query.canonical_sql(), context)
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
                body: self.expand_body(owner, &body.props, &body.children, Some(context)),
            },
            Value::Binding { kind, path, time } => Value::Binding {
                kind: *kind,
                path: path.clone(),
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
                    value.clone()
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

    fn declaration_span(&self, owner: &ProjectFileId, path: &[usize]) -> SourceSpan {
        self.project
            .files
            .get(owner)
            .and_then(|file| declaration_span(file, path))
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

fn substitute_sql(sql: String, context: &ExpansionContext) -> Result<String, crate::ast::AstError> {
    let mut replacements = BTreeMap::new();
    for (name, slot) in &context.slots {
        if slot.shape == "block" || slot.shape == "ref" {
            continue;
        }
        replacements.insert(name.clone(), slot_sql(slot)?);
    }
    for (logical, physical) in &context.channels {
        replacements.insert(logical.clone(), physical.clone());
        replacements.insert(format!("{logical}2"), format!("{physical}2"));
    }
    Ok(rewrite_identifiers(&sql, |identifier| {
        if let Some(value) = replacements.get(identifier) {
            return Some(value.clone());
        }
        let prefix = format!("__{}_", context.definition_name);
        identifier
            .strip_prefix(&prefix)
            .map(|suffix| format!("__{}_{}", context.instance_name, suffix))
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
    mut replacement: impl FnMut(&str) -> Option<String>,
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
            if !preceded_by_binding && let Some(value) = replacement(&source[start..end]) {
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

fn declaration_span(file: &ProjectFile, path: &[usize]) -> Option<SourceSpan> {
    let declaration = declaration_at(file.parsed.ast.root.declarations(), path)?;
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
