use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use avenger_chart_schema::{
    KindSchema, NativeKindKey, NativeKindNamespace, NativeSchemaSnapshot, PropertySchema,
    ValueShape,
};
use avenger_lang_core::{
    ByteSpan, ResolvedDeclaration, ResolvedProject, ResolvedTarget, SourceOrigin, SourceSpan,
    allowed_child_declarations,
    ast::Visibility,
    sql::{LosslessTokenKind, TokenClass},
    syntax::{TolerantSyntaxNodeId, TolerantSyntaxNodeKind},
};
use sqlparser::tokenizer::Token;

use crate::{
    AnalysisCancellation, AnalysisGeneration, CompletionItem, CompletionKind, CompletionOrigin,
    CompletionResult, CompletionTextFormat, HoverResult, NavigationResult, NavigationTarget,
    PositionRequest, RootAnalysis, SymbolKind, SyntaxAnalysis,
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
        index.resolve_lexical_references();
        index
    }

    fn enrich_from_resolved(&mut self, project: &ResolvedProject) {
        let mut declarations = BTreeMap::new();
        for file in project.files.values() {
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
            for reference in &mut document.references {
                if let Some(bind) = self.public_bindings.get(&reference.name) {
                    reference.value_kind = bind.value_kind;
                    reference.target_identity = bind.target.as_ref().and_then(|target| {
                        identity_by_location
                            .get(&(target.origin.clone(), target.selection_span))
                            .cloned()
                    });
                    continue;
                }
                let leaf = reference.name.rsplit('.').next().unwrap_or(&reference.name);
                if let Some(candidates) = by_name.get(leaf)
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
        let header = declaration_header(syntax, node.span, keyword, name.as_deref());
        let parent = parent_declaration(&syntax.parsed.nodes, node.parent).and_then(|parent_id| {
            declaration_nodes
                .iter()
                .position(|candidate| candidate.id == parent_id)
        });
        let ordinal = output.symbols.len();
        let selection_span = header.name_span.unwrap_or_else(|| {
            header
                .keyword_span
                .unwrap_or(SourceSpan::empty(node.span.source, node.span.range.start))
        });
        let name = header
            .name
            .or_else(|| name.clone())
            .unwrap_or_else(|| keyword.clone());
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
            native_kind: header.native_kind,
            visibility: header.visibility,
            detail: Some(keyword.clone()),
            documentation: header.documentation,
        });
    }

    for node in &syntax.parsed.nodes {
        if let TolerantSyntaxNodeKind::Property { name } = &node.kind
            && let Some(span) = first_word_span(syntax, node.span, name)
        {
            output.property_names.insert(span, name.clone());
        }
    }

    let occupied = output
        .symbols
        .iter()
        .map(|symbol| symbol.selection_span)
        .chain(output.property_names.keys().copied())
        .collect::<Vec<_>>();
    output.references = scan_references(origin, syntax, &occupied);
    output
}

#[derive(Default)]
struct DeclarationHeader {
    name: Option<String>,
    native_kind: Option<String>,
    keyword_span: Option<SourceSpan>,
    name_span: Option<SourceSpan>,
    visibility: Visibility,
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
    let fallback = match keyword {
        "define" => header.get(keyword_index + 2),
        "param" | "store" | "selection" => header.get(keyword_index + 2),
        _ => header.get(keyword_index + 1),
    };
    let named = binder.or(fallback).filter(|token| token.word().is_some());
    output.name = named
        .and_then(SigToken::word)
        .map(str::to_owned)
        .or_else(|| fallback_name.map(str::to_owned));
    output.name_span = named.map(|token| token.span);
    output.native_kind = match keyword {
        "chart" | "plot" | "mark" | "transform" | "tool" | "widget" | "resource" | "catalog"
        | "schema" | "table" => header
            .get(keyword_index + 1)
            .and_then(SigToken::word)
            .map(str::to_owned),
        "define" => header
            .get(keyword_index + 1)
            .and_then(SigToken::word)
            .map(str::to_owned),
        _ => None,
    };
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
    project: &ResolvedProject,
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
            span: token.span(),
        })
        .collect()
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
        index += 1;
    }
    output
}

fn symbol_kind(keyword: &str) -> SymbolKind {
    match keyword.to_ascii_lowercase().as_str() {
        "chart" => SymbolKind::Chart,
        "define" => SymbolKind::Definition,
        "catalog" => SymbolKind::Catalog,
        "schema" => SymbolKind::Schema,
        "table" => SymbolKind::Table,
        "group" => SymbolKind::Group,
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
        "mark" | "group" => IndexedValueKind::Mark,
        "tool" => IndexedValueKind::Tool,
        "widget" => IndexedValueKind::Widget,
        "on" => IndexedValueKind::Event,
        "field" => IndexedValueKind::Field,
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
}

impl<'a> QueryContext<'a> {
    pub(crate) fn new(
        generation: AnalysisGeneration,
        project_root: &'a Path,
        known_sources: &'a [SourceOrigin],
        registry: &'a NativeSchemaSnapshot,
        syntax: &'a BTreeMap<SourceOrigin, SyntaxAnalysis>,
        index: &'a WorkspaceSemanticIndex,
    ) -> Self {
        Self {
            generation,
            project_root,
            known_sources,
            registry,
            syntax,
            index,
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
        let prefix = &text[replacement.range.as_range()];
        let mut items = Vec::new();

        if let Some(import_prefix) = import_prefix(text, cursor) {
            self.complete_imports(import_prefix, replacement, &mut items);
        } else if prefix.starts_with('$') {
            self.complete_bindings(prefix, replacement, &request.source, cursor, &mut items);
        } else if let Some((namespace, typed)) = declaration_kind_context(text, cursor) {
            self.complete_native_kinds(namespace, typed, replacement, &mut items);
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
            self.complete_properties(
                syntax,
                &request.source,
                cursor,
                prefix,
                replacement,
                &mut items,
            );
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
            is_incomplete: false,
            generation: self.generation,
            source_revision: request.source_revision.clone(),
        })
    }

    fn complete_native_kinds(
        &self,
        namespace: NativeKindNamespace,
        typed: &str,
        replacement: SourceSpan,
        output: &mut Vec<CompletionItem>,
    ) {
        for (key, schema) in &self.registry.entries {
            if key.namespace != namespace || !candidate_matches(&key.kind, typed) {
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
        let Some(schema) = schema_for_symbol(self.registry, owner, self.index) else {
            return;
        };
        let authored = authored_properties(syntax, owner.scope_span);
        for (name, property) in &schema.properties {
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
        let property = owner
            .and_then(|owner| schema_for_symbol(self.registry, owner, self.index))
            .and_then(|schema| property_schema(schema, property_name));
        if let Some(property) = property {
            complete_shape(property.shape, prefix, replacement, output);
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
                    IndexedValueKind::Scalar
                        | IndexedValueKind::Table
                        | IndexedValueKind::Selection
                ) || symbol.selection_span.range.start >= cursor
                    || !scope_visible(document, symbol, cursor)
                    || !candidate_matches(&symbol.name, typed)
                {
                    continue;
                }
                let label = format!("${}", symbol.name);
                output.push(item(
                    label.clone(),
                    replacement,
                    label,
                    CompletionKind::Variable,
                    symbol.detail.clone(),
                    symbol.documentation.clone(),
                    CompletionOrigin::LexicalScope,
                    false,
                    "00",
                ));
            }
        }
        for binding in self.index.public_bindings.values() {
            if !candidate_matches(&binding.path, typed) {
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
            vec!["import", "chart", "define", "catalog", "schema", "table"]
        };
        for keyword in keywords {
            if !candidate_matches(keyword, prefix) {
                continue;
            }
            let (insert_text, format) = if options.snippets {
                (declaration_snippet(keyword), CompletionTextFormat::Snippet)
            } else {
                (keyword.to_owned(), CompletionTextFormat::PlainText)
            };
            let mut candidate = item(
                keyword.to_owned(),
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
        for standard in ["std:marks", "std:tools", "std:transforms"] {
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
    }

    pub(crate) fn hover(
        &self,
        request: &PositionRequest,
        cancellation: &AnalysisCancellation,
    ) -> Result<Option<HoverResult>, AnalysisQueryError> {
        self.syntax(request, cancellation)?;
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
            (symbol.selection_span, markdown)
        } else if let Some(reference) = reference {
            let target = reference
                .target_identity
                .as_deref()
                .and_then(|identity| self.index.symbol_by_identity(identity));
            let detail = target
                .and_then(|symbol| symbol.detail.clone())
                .unwrap_or_else(|| format!("{:?}", reference.value_kind).to_ascii_lowercase());
            (
                reference.span,
                format!("```avenger\n${}\n```\n\n{detail}", reference.name),
            )
        } else if let Some((span, name)) = property {
            let owner = owner_symbol(self.index, &request.source, request.byte_offset);
            let schema =
                owner.and_then(|owner| schema_for_symbol(self.registry, owner, self.index));
            let property = schema.and_then(|schema| property_schema(schema, name));
            let docs = property
                .map(|property| property.docs)
                .unwrap_or("Avenger property");
            (*span, format!("`{name}`\n\n{docs}"))
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

fn owner_symbol<'a>(
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
    let owner = document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.scope_span.range.start <= cursor && cursor <= symbol.scope_span.range.end
        })
        .min_by_key(|symbol| symbol.scope_span.range.len());
    let Some(owner) = owner else {
        return candidate.parent.is_none();
    };
    let mut current = Some(owner);
    while let Some(symbol) = current {
        let ordinal = document
            .symbols
            .iter()
            .position(|item| item.identity == symbol.identity);
        if candidate.parent == ordinal || candidate.identity == symbol.identity {
            return true;
        }
        current = symbol
            .parent
            .and_then(|parent| document.symbols.get(parent));
    }
    candidate.parent.is_none()
}

fn schema_for_symbol<'a>(
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

fn property_value_context(syntax: &SyntaxAnalysis, cursor: usize) -> Option<&str> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if node.span.range.start <= cursor && cursor <= node.span.range.end =>
            {
                Some((node.span.range.len(), name.as_str()))
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

fn authored_properties(syntax: &SyntaxAnalysis, scope: SourceSpan) -> BTreeSet<String> {
    syntax
        .parsed
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            TolerantSyntaxNodeKind::Property { name }
                if scope.range.start <= node.span.range.start
                    && node.span.range.end <= scope.range.end =>
            {
                Some(name.clone())
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy)]
struct PropertyView<'a> {
    shape: &'a ValueShape,
    docs: &'a str,
}

fn property_schema<'a>(schema: &'a KindSchema, name: &str) -> Option<PropertyView<'a>> {
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
    let line = prefix.rsplit_once('\n').map_or(prefix, |(_, line)| line);
    let import = line.trim_start().strip_prefix("import")?.trim_start();
    let quote = import.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let value = &import[quote.len_utf8()..];
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
        "param" | "store" | "selection" => format!("{keyword} as ${{1:name}} {{\n  $0\n}}"),
        "mark" | "transform" | "tool" | "widget" | "resource" => {
            format!("{keyword} ${{1:kind}} as ${{2:name}} {{\n  $0\n}}")
        }
        "group" | "view" => format!("{keyword} as ${{1:name}} {{\n  $0\n}}"),
        _ => keyword.to_owned(),
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

    #[test]
    fn index_preserves_exact_binder_and_reference_spans() {
        let text = "avenger 1; chart cartesian as chart { param as width { type: float64; default: 1.0; } mark symbol as points { size: $width; } }";
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
    fn registry_completion_has_no_native_kind_switch() {
        let text = "avenger 1; chart cartesian as chart { mark sy }";
        let (origin, syntax) = fixture(text);
        let revision = syntax.revision.clone();
        let syntax = BTreeMap::from([(origin.clone(), syntax)]);
        let index = WorkspaceSemanticIndex::build(&syntax, &BTreeMap::new());
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let context = QueryContext::new(
            AnalysisGeneration::new(1),
            Path::new("/tmp"),
            &[],
            compiler.language_host().authoring_schema(),
            &syntax,
            &index,
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
        let text = "avenger 1; chart cartesian as chart { param as café { type: float64; default: 1.0; } mark symbol { size: $caf; } }";
        let (_, syntax) = fixture(text);
        let cursor = text.find("$caf").unwrap() + "$caf".len();
        let span = replacement_span(&syntax, cursor);
        assert_eq!(&text[span.range.as_range()], "$caf");
    }

    #[test]
    fn source_span_constructor_used_by_fixture_is_valid() {
        let source = SourceFile::new(SourceId::new(7), SourceOrigin::Memory("x".to_owned()), "x");
        assert_eq!(source.text(), "x");
    }
}
