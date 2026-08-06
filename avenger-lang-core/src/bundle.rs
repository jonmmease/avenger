//! Resolved, deterministic source-module bundling.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::ControlFlow,
};

use avenger_chart_schema::NativeModuleId;
use sha2::{Digest, Sha256};
use sqlparser::ast::{Ident, ObjectName, VisitMut, VisitorMut};

use crate::{
    Diagnostic, SourceId, SourceLabel, SourceMap, SourceSpan,
    ast::{
        BindingTime, Decl, File, Import, ImportClause, ModuleItem, Name, QualifiedName, SqlBinding,
        SqlQuery, Value,
    },
    module_graph::{ModuleId, ParsedModule, ParsedModuleGraph, SourceModuleId},
    print::print_file,
    resolve::{
        ChartEntrypointId, ModuleItemId, ResolvedDeclaration, ResolvedKindBinding,
        ResolvedModuleGraph, ResolvedRelationId, ResolvedRelationReference, ResolvedRelationTarget,
        ResolvedValue,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleTarget {
    Chart(ChartEntrypointId),
    Module(SourceModuleId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundledSource {
    pub text: String,
    pub root_module: SourceModuleId,
    pub included_items: BTreeSet<ModuleItemId>,
}

#[derive(Clone, Debug)]
pub struct BundleFailure {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: SourceMap,
}

impl BundleFailure {
    fn new(
        parsed: &ParsedModuleGraph,
        span: SourceSpan,
        code: &'static str,
        message: &'static str,
        label: impl Into<String>,
    ) -> Self {
        Self {
            diagnostics: vec![Diagnostic::error(
                code,
                message,
                SourceLabel::new(span, label),
            )],
            sources: parsed.sources.clone(),
        }
    }
}

/// Flatten a semantically resolved source-module graph into one canonical
/// module. Source imports are eliminated; exact native requirements are
/// regenerated as namespace imports.
pub fn bundle_module_graph(
    parsed: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    target: BundleTarget,
) -> Result<BundledSource, BundleFailure> {
    let (root_module, root_items, included_items) = select_items(parsed, resolved, &target)?;
    let root = parsed.source_modules.get(&root_module).ok_or_else(|| {
        BundleFailure::new(
            parsed,
            SourceSpan::empty(SourceId::new(0), 0),
            "AVENGER-BUNDLE-001",
            "bundle root module is unavailable",
            format!("`{}` is not loaded", root_module.as_str()),
        )
    })?;

    let names = allocate_item_names(resolved, &root_module, &included_items)?;
    let native_modules = collect_native_modules(resolved, &included_items);
    let native_aliases = allocate_native_aliases(&native_modules, &names);
    let mut items = Vec::new();

    for item_id in ordered_items(resolved, &root_module, &included_items) {
        let (authored, semantic) = item_pair(parsed, resolved, &item_id).ok_or_else(|| {
            BundleFailure::new(
                parsed,
                root_span(root),
                "AVENGER-BUNDLE-002",
                "bundle item source is unavailable",
                format!(
                    "cannot locate `{}` in `{}`",
                    item_id.declaration.as_str(),
                    item_id.module.as_str()
                ),
            )
        })?;
        let mut declaration = authored.declaration.clone();
        if let Some(name) = names.get(&item_id) {
            declaration.name = Some(name.clone());
        }
        rewrite_declaration(
            &mut declaration,
            semantic,
            &names,
            &native_aliases,
            resolved,
        )?;
        items.push(ModuleItem {
            exported: item_id.module == root_module
                && root_items.contains(&item_id)
                && authored.exported,
            declaration,
        });
    }

    let imports = native_aliases
        .iter()
        .map(|(module, alias)| Import {
            source: module.as_str().to_owned(),
            sha256: None,
            clause: ImportClause::Namespace(alias.clone()),
        })
        .collect();
    let text = print_file(&File {
        version: root.parsed.ast.version,
        imports,
        items,
    });
    Ok(BundledSource {
        text,
        root_module,
        included_items,
    })
}

fn select_items(
    parsed: &ParsedModuleGraph,
    resolved: &ResolvedModuleGraph,
    target: &BundleTarget,
) -> Result<
    (
        SourceModuleId,
        BTreeSet<ModuleItemId>,
        BTreeSet<ModuleItemId>,
    ),
    BundleFailure,
> {
    match target {
        BundleTarget::Chart(id) => {
            let entrypoint = resolved.entrypoints.get(id).ok_or_else(|| {
                BundleFailure::new(
                    parsed,
                    SourceSpan::empty(SourceId::new(0), 0),
                    "AVENGER-BUNDLE-003",
                    "chart entrypoint is unavailable",
                    format!(
                        "the selected chart in `{}` was not resolved",
                        id.module.as_str()
                    ),
                )
            })?;
            Ok((
                id.module.clone(),
                BTreeSet::from([entrypoint.item.clone()]),
                entrypoint.reachable_items.clone(),
            ))
        }
        BundleTarget::Module(module) => {
            let resolved_module = resolved.source_modules.get(module).ok_or_else(|| {
                BundleFailure::new(
                    parsed,
                    SourceSpan::empty(SourceId::new(0), 0),
                    "AVENGER-BUNDLE-004",
                    "module interface is unavailable",
                    format!("`{}` was not resolved", module.as_str()),
                )
            })?;
            let root_items = resolved_module
                .item_order
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut included = root_items.clone();
            for item in &root_items {
                included.extend(
                    resolved
                        .item_dependencies
                        .transitive_closures
                        .get(item)
                        .into_iter()
                        .flatten()
                        .cloned(),
                );
            }
            Ok((module.clone(), root_items, included))
        }
    }
}

fn allocate_item_names(
    resolved: &ResolvedModuleGraph,
    root: &SourceModuleId,
    included: &BTreeSet<ModuleItemId>,
) -> Result<BTreeMap<ModuleItemId, Name>, BundleFailure> {
    let mut names = BTreeMap::new();
    let mut used = BTreeSet::new();
    for item in included.iter().filter(|item| &item.module == root) {
        if let Some(name) = resolved
            .items
            .get(item)
            .and_then(|item| item.source_name.as_deref())
        {
            used.insert(name.to_owned());
            names.insert(
                item.clone(),
                Name::new(name).expect("resolved source names are valid"),
            );
        }
    }
    for item in included.iter().filter(|item| &item.module != root) {
        let Some(source_name) = resolved
            .items
            .get(item)
            .and_then(|item| item.source_name.as_deref())
        else {
            continue;
        };
        let suffix = short_hash(&[
            "bundle-item",
            item.module.as_str(),
            item.declaration.as_str(),
        ]);
        let base = format!("__av_{suffix}_{source_name}");
        let mut candidate = base.clone();
        let mut ordinal = 2_u32;
        while used.contains(&candidate) {
            candidate = format!("{base}_{ordinal}");
            ordinal += 1;
        }
        used.insert(candidate.clone());
        names.insert(
            item.clone(),
            Name::new(candidate).expect("generated bundle names are valid"),
        );
    }
    Ok(names)
}

fn collect_native_modules(
    resolved: &ResolvedModuleGraph,
    included: &BTreeSet<ModuleItemId>,
) -> BTreeSet<NativeModuleId> {
    let mut modules = BTreeSet::new();
    for item in included {
        let Some((_, declaration)) = resolved_item(resolved, item) else {
            continue;
        };
        collect_declaration_native_modules(declaration, &mut modules);
    }
    modules
}

fn collect_declaration_native_modules(
    declaration: &ResolvedDeclaration,
    output: &mut BTreeSet<NativeModuleId>,
) {
    if let Some(ResolvedKindBinding::Native { export, .. }) = &declaration.kind_binding
        && let ModuleId::Native(module) = &export.module
    {
        output.insert(module.clone());
    }
    for value in declaration.properties.values() {
        collect_value_native_modules(value, output);
    }
    for child in &declaration.children {
        collect_declaration_native_modules(child, output);
    }
}

fn collect_value_native_modules(value: &ResolvedValue, output: &mut BTreeSet<NativeModuleId>) {
    match value {
        ResolvedValue::Object {
            properties,
            children,
            ..
        } => {
            for value in properties.values() {
                collect_value_native_modules(value, output);
            }
            for child in children {
                collect_declaration_native_modules(child, output);
            }
        }
        ResolvedValue::Array(values) => {
            for value in values {
                collect_value_native_modules(value, output);
            }
        }
        ResolvedValue::Channel {
            expression: value, ..
        }
        | ResolvedValue::Pattern(value) => {
            collect_value_native_modules(value, output);
        }
        ResolvedValue::ChannelValue(channel) => {
            collect_value_native_modules(&channel.head.expression, output);
            if let Some(otherwise) = &channel.otherwise {
                collect_value_native_modules(&otherwise.expression, output);
            }
            for condition in &channel.conditions {
                collect_value_native_modules(&condition.predicate, output);
                collect_value_native_modules(&condition.branch.expression, output);
            }
            for value in channel.configuration.values() {
                collect_value_native_modules(value, output);
            }
        }
        ResolvedValue::Call { args, .. } => {
            for value in args {
                collect_value_native_modules(value, output);
            }
        }
        _ => {}
    }
}

fn allocate_native_aliases(
    modules: &BTreeSet<NativeModuleId>,
    item_names: &BTreeMap<ModuleItemId, Name>,
) -> BTreeMap<NativeModuleId, Name> {
    let mut used = item_names
        .values()
        .map(ToString::to_string)
        .collect::<BTreeSet<_>>();
    modules
        .iter()
        .map(|module| {
            let base = format!("__av_native_{}", short_hash(&["native", module.as_str()]));
            let mut candidate = base.clone();
            let mut ordinal = 2_u32;
            while used.contains(&candidate) {
                candidate = format!("{base}_{ordinal}");
                ordinal += 1;
            }
            used.insert(candidate.clone());
            (
                module.clone(),
                Name::new(candidate).expect("generated native aliases are valid"),
            )
        })
        .collect()
}

fn ordered_items(
    resolved: &ResolvedModuleGraph,
    root: &SourceModuleId,
    included: &BTreeSet<ModuleItemId>,
) -> Vec<ModuleItemId> {
    let mut modules = resolved
        .source_modules
        .keys()
        .filter(|module| *module != root)
        .cloned()
        .collect::<Vec<_>>();
    modules.sort();
    modules.push(root.clone());
    let mut ordered = Vec::new();
    for module in modules {
        if let Some(file) = resolved.source_modules.get(&module) {
            ordered.extend(
                file.item_order
                    .iter()
                    .filter(|item| included.contains(*item))
                    .cloned(),
            );
        }
    }
    ordered
}

fn item_pair<'a>(
    parsed: &'a ParsedModuleGraph,
    resolved: &'a ResolvedModuleGraph,
    item: &ModuleItemId,
) -> Option<(&'a ModuleItem, &'a ResolvedDeclaration)> {
    let parsed_module = parsed.source_modules.get(&item.module)?;
    let resolved_module = resolved.source_modules.get(&item.module)?;
    let index = resolved_module
        .item_order
        .iter()
        .position(|candidate| candidate == item)?;
    Some((
        parsed_module.parsed.ast.items.get(index)?,
        resolved_module.roots.get(index)?,
    ))
}

fn resolved_item<'a>(
    resolved: &'a ResolvedModuleGraph,
    item: &ModuleItemId,
) -> Option<(
    &'a crate::resolve::ResolvedModuleItem,
    &'a ResolvedDeclaration,
)> {
    let metadata = resolved.items.get(item)?;
    let module = resolved.source_modules.get(&item.module)?;
    let index = module
        .item_order
        .iter()
        .position(|candidate| candidate == item)?;
    Some((metadata, module.roots.get(index)?))
}

fn rewrite_declaration(
    authored: &mut Decl,
    semantic: &ResolvedDeclaration,
    names: &BTreeMap<ModuleItemId, Name>,
    native_aliases: &BTreeMap<NativeModuleId, Name>,
    resolved: &ResolvedModuleGraph,
) -> Result<(), BundleFailure> {
    match &semantic.kind_binding {
        Some(ResolvedKindBinding::Definition(item)) => {
            if let Some(name) = names.get(item) {
                authored.kind = Some(QualifiedName::new(vec![name.clone()]).expect("one segment"));
            }
        }
        Some(ResolvedKindBinding::Native { export, .. }) => {
            if let ModuleId::Native(module) = &export.module
                && let Some(alias) = native_aliases.get(module)
            {
                authored.kind = Some(
                    QualifiedName::new(vec![
                        alias.clone(),
                        Name::new(&export.name).expect("resolved export names are valid"),
                    ])
                    .expect("two segments"),
                );
            }
        }
        _ => {}
    }

    let property_names = authored
        .props
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    for property in property_names {
        let Some(value) = authored.props.get(property.as_str()).cloned() else {
            continue;
        };
        let semantic_value = semantic.properties.get(property.as_str());
        let rewritten = rewrite_value(
            value,
            semantic_value,
            &semantic.relation_references,
            names,
            native_aliases,
            resolved,
        )?;
        authored.props.set(property, rewritten);
    }
    for (child, semantic_child) in authored.children.iter_mut().zip(&semantic.children) {
        rewrite_declaration(child, semantic_child, names, native_aliases, resolved)?;
    }
    Ok(())
}

fn rewrite_value(
    mut authored: Value,
    semantic: Option<&ResolvedValue>,
    declaration_relations: &[ResolvedRelationReference],
    names: &BTreeMap<ModuleItemId, Name>,
    native_aliases: &BTreeMap<NativeModuleId, Name>,
    resolved: &ResolvedModuleGraph,
) -> Result<Value, BundleFailure> {
    match (&mut authored, semantic) {
        (Value::Query(query), Some(ResolvedValue::Query(semantic_query))) => {
            **query = rewrite_query(query, &semantic_query.relations, names)?;
        }
        (Value::Relation(path), Some(ResolvedValue::Relation(reference))) => {
            if let Some(replacement) = relation_target_path(&reference.target, names) {
                *path = replacement
                    .into_iter()
                    .map(|part| Name::new(part).expect("resolved relation names are valid"))
                    .collect();
            }
        }
        (Value::Relation(path), _) => {
            let authored_path = path.iter().map(Name::as_str).collect::<Vec<_>>().join(".");
            if let Some(replacement) = declaration_relations.iter().find_map(|reference| {
                (reference.authored_path.join(".") == authored_path)
                    .then(|| relation_target_path(&reference.target, names))
                    .flatten()
            }) {
                *path = replacement
                    .into_iter()
                    .map(|part| Name::new(part).expect("resolved relation names are valid"))
                    .collect();
            }
        }
        (Value::Array(values), Some(ResolvedValue::Array(semantic_values))) => {
            for (value, semantic) in values.iter_mut().zip(semantic_values) {
                *value = rewrite_value(
                    value.clone(),
                    Some(semantic),
                    declaration_relations,
                    names,
                    native_aliases,
                    resolved,
                )?;
            }
        }
        (Value::Block { head, body }, Some(ResolvedValue::ChannelValue(channel))) => {
            if let Some(Value::Channel { mode, expression }) = head.as_deref_mut()
                && *mode == channel.head.mode
            {
                **expression = rewrite_value(
                    (**expression).clone(),
                    Some(&channel.head.expression),
                    declaration_relations,
                    names,
                    native_aliases,
                    resolved,
                )?;
            }
            let property_names = body
                .props
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            for property in property_names {
                let Some(value) = body.props.get(property.as_str()).cloned() else {
                    continue;
                };
                let rewritten = if property.as_str() == "otherwise" {
                    if let Some(branch) = &channel.otherwise {
                        rewrite_channel_branch_value(
                            value,
                            branch,
                            declaration_relations,
                            names,
                            native_aliases,
                            resolved,
                        )?
                    } else {
                        value
                    }
                } else {
                    rewrite_value(
                        value,
                        channel.configuration.get(property.as_str()),
                        declaration_relations,
                        names,
                        native_aliases,
                        resolved,
                    )?
                };
                body.props.set(property, rewritten);
            }
            for (child, condition) in body
                .children
                .iter_mut()
                .filter(|child| child.keyword.as_str() == "when")
                .zip(&channel.conditions)
            {
                if let Some(predicate) = child.props.get("predicate").cloned() {
                    child.props.set(
                        Name::new("predicate").expect("static name"),
                        rewrite_value(
                            predicate,
                            Some(&condition.predicate),
                            declaration_relations,
                            names,
                            native_aliases,
                            resolved,
                        )?,
                    );
                }
                let mode = condition.branch.mode.as_str();
                if let Some(value) = child.props.get(mode).cloned() {
                    child.props.set(
                        Name::new(mode).expect("static mode"),
                        rewrite_value(
                            value,
                            Some(&condition.branch.expression),
                            declaration_relations,
                            names,
                            native_aliases,
                            resolved,
                        )?,
                    );
                }
            }
        }
        (
            Value::Block { head, body },
            Some(ResolvedValue::Object {
                head: semantic_head,
                properties,
                children,
                ..
            }),
        ) => {
            if let (Some(head), Some(semantic_head)) = (head, semantic_head) {
                **head = rewrite_value(
                    (**head).clone(),
                    Some(semantic_head),
                    declaration_relations,
                    names,
                    native_aliases,
                    resolved,
                )?;
            }
            let property_names = body
                .props
                .iter()
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            for property in property_names {
                let Some(value) = body.props.get(property.as_str()).cloned() else {
                    continue;
                };
                body.props.set(
                    property.clone(),
                    rewrite_value(
                        value,
                        properties.get(property.as_str()),
                        declaration_relations,
                        names,
                        native_aliases,
                        resolved,
                    )?,
                );
            }
            for (child, semantic_child) in body.children.iter_mut().zip(children) {
                rewrite_declaration(child, semantic_child, names, native_aliases, resolved)?;
            }
        }
        (
            Value::Channel {
                mode,
                expression: value,
            },
            Some(ResolvedValue::Channel {
                mode: semantic_mode,
                expression: semantic,
            }),
        ) if mode == semantic_mode => {
            **value = rewrite_value(
                (**value).clone(),
                Some(semantic),
                declaration_relations,
                names,
                native_aliases,
                resolved,
            )?;
        }
        (
            Value::Channel {
                mode,
                expression: value,
            },
            Some(ResolvedValue::ChannelValue(channel)),
        ) if *mode == channel.head.mode => {
            **value = rewrite_value(
                (**value).clone(),
                Some(&channel.head.expression),
                declaration_relations,
                names,
                native_aliases,
                resolved,
            )?;
        }
        (Value::Pattern(value), Some(ResolvedValue::Pattern(semantic))) => {
            **value = rewrite_value(
                (**value).clone(),
                Some(semantic),
                declaration_relations,
                names,
                native_aliases,
                resolved,
            )?;
        }
        (
            Value::Call { args, .. },
            Some(ResolvedValue::Call {
                args: semantic_args,
                ..
            }),
        ) => {
            for (arg, semantic) in args.iter_mut().zip(semantic_args) {
                *arg = rewrite_value(
                    arg.clone(),
                    Some(semantic),
                    declaration_relations,
                    names,
                    native_aliases,
                    resolved,
                )?;
            }
        }
        _ => {}
    }
    Ok(authored)
}

fn rewrite_channel_branch_value(
    mut authored: Value,
    branch: &crate::resolve::ResolvedChannelBranch,
    declaration_relations: &[ResolvedRelationReference],
    names: &BTreeMap<ModuleItemId, Name>,
    native_aliases: &BTreeMap<NativeModuleId, Name>,
    resolved: &ResolvedModuleGraph,
) -> Result<Value, BundleFailure> {
    let Value::Block { body, .. } = &mut authored else {
        return Ok(authored);
    };
    let mode = branch.mode.as_str();
    if let Some(value) = body.props.get(mode).cloned() {
        body.props.set(
            Name::new(mode).expect("static mode"),
            rewrite_value(
                value,
                Some(&branch.expression),
                declaration_relations,
                names,
                native_aliases,
                resolved,
            )?,
        );
    }
    Ok(authored)
}

fn relation_target_path(
    target: &ResolvedRelationTarget,
    names: &BTreeMap<ModuleItemId, Name>,
) -> Option<Vec<String>> {
    let ResolvedRelationTarget::Relation(ResolvedRelationId {
        defining_item,
        nested_path,
    }) = target
    else {
        return None;
    };
    let mut output = vec![names.get(defining_item)?.to_string()];
    output.extend(nested_path.iter().cloned());
    Some(output)
}

fn rewrite_query(
    query: &SqlQuery,
    relations: &[ResolvedRelationReference],
    names: &BTreeMap<ModuleItemId, Name>,
) -> Result<SqlQuery, BundleFailure> {
    let replacements = relations
        .iter()
        .filter_map(|reference| {
            relation_target_path(&reference.target, names)
                .map(|target| (reference.authored_path.clone(), target))
        })
        .collect::<BTreeMap<_, _>>();
    let mut ast = query.ast().clone();
    let _ = ast.visit(&mut RelationRewriter { replacements });
    let sql = restore_bindings(ast.to_string(), query.bindings());
    SqlQuery::parse(&sql).map_err(|error| BundleFailure {
        diagnostics: vec![Diagnostic::error(
            "AVENGER-BUNDLE-005",
            "bundled SQL could not be reconstructed",
            SourceLabel::new(SourceSpan::empty(SourceId::new(0), 0), error.to_string()),
        )],
        sources: SourceMap::default(),
    })
}

struct RelationRewriter {
    replacements: BTreeMap<Vec<String>, Vec<String>>,
}

impl VisitorMut for RelationRewriter {
    type Break = ();

    fn pre_visit_relation(&mut self, relation: &mut ObjectName) -> ControlFlow<Self::Break> {
        let path = relation
            .0
            .iter()
            .map(|part| part.as_ident().map(|ident| ident.value.clone()))
            .collect::<Option<Vec<_>>>();
        if let Some(target) = path.and_then(|path| self.replacements.get(&path)) {
            *relation = ObjectName::from(
                target
                    .iter()
                    .map(|segment| Ident::with_quote('"', segment))
                    .collect::<Vec<_>>(),
            );
        }
        ControlFlow::Continue(())
    }
}

fn restore_bindings(mut sql: String, bindings: &[SqlBinding]) -> String {
    for binding in bindings {
        let mut surface = String::from("$");
        for (index, segment) in binding.path.iter().enumerate() {
            if index > 0 {
                surface.push('.');
            }
            surface.push_str(segment.as_str());
        }
        match binding.time {
            BindingTime::Current => {}
            BindingTime::Start => surface.push_str("@start"),
            BindingTime::Previous => surface.push_str("@previous"),
        }
        sql = sql.replace(&format!("\"{}\"", binding.synthetic_identifier), &surface);
    }
    sql
}

fn short_hash(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part.as_bytes());
    }
    format!("{:x}", hash.finalize())[..10].to_owned()
}

fn root_span(module: &ParsedModule) -> SourceSpan {
    module
        .parsed
        .module_syntax
        .items
        .first()
        .map(|item| item.span)
        .unwrap_or_else(|| SourceSpan::empty(module.source, 0))
}
