//! Completion-specific recovery and semantic SQL scopes.
//!
//! The strict Avenger SQL frontend remains the language authority. This module
//! deliberately owns only the bounded, cursor-oriented recovery needed while
//! an author is editing an incomplete island. Stable relation schemas come
//! from `ModuleAnalysis`; expressions are validated with DataFusion's logical
//! expression planner and are never executed.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

use arrow::datatypes::DataType;
use avenger_chart_schema::{NativeKindNamespace, NativeSchemaSnapshot};
use avenger_lang_compiler::{
    AnalyzedDataset, DatasetStageKind, ModuleAnalysis, physical_type_to_arrow,
};
use avenger_lang_core::{
    ByteSpan, INTRINSIC_OPERATION_SIGNATURES, IntrinsicOperationContext, PhysicalType,
    SourceOrigin, SourceSpan,
    ast::{SqlExpression, SqlProjection, SqlQuery},
    contextual_access_signature,
    resolve::{
        DeclarationId, ResolvedDeclaration, ResolvedEventScope, ResolvedEventSurface,
        ResolvedKindBinding, ResolvedTarget,
    },
    sql::{LosslessTokenKind, TokenClass},
    syntax::{SqlIslandContext, SqlIslandRoot, TolerantSyntaxNodeKind},
};
use datafusion::{
    catalog::{CatalogProvider, MemoryCatalogProvider, MemorySchemaProvider, SchemaProvider},
    datasource::{TableProvider, empty::EmptyTable},
    prelude::SessionContext,
};
use datafusion::{common::DFSchema, execution::SessionStateBuilder, logical_expr::ExprSchemable};
use sha2::{Digest, Sha256};
use sqlparser::tokenizer::Token;

use crate::{
    AnalysisCancellation, CompletionItem, CompletionKind, CompletionOrigin, CompletionTextFormat,
    DatasetContext, DocumentSemanticIndex, IndexedValueKind, PositionRequest, RootAnalysis,
    SyntaxAnalysis, WorkspaceSemanticIndex,
};

const MAX_REPAIR_ATTEMPTS: usize = 4;

/// Semantic categories that may legally satisfy the cursor position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlExpectedRole {
    Clause,
    Relation,
    Catalog,
    Schema,
    Expression,
    QualifierMember,
    Function,
    Type,
    Binding,
    Alias,
}

/// The deliberately small recovery matrix used before consulting semantic
/// scopes. Names are stable for corpus baselines and metrics, not diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlRepairStrategy {
    None,
    QualifierMember,
    EmptyExpression,
    MissingRelation,
    TrailingComma,
    CloseDelimiters,
}

/// Test/telemetry view of one completion classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlCompletionDebug {
    pub island: SourceSpan,
    pub roles: BTreeSet<SqlExpectedRole>,
    pub repair: SqlRepairStrategy,
    pub repair_attempts: usize,
    pub token_count: usize,
    pub expression_planned: bool,
    pub repaired_parse: bool,
    pub synthetic_ranges: Vec<Range<usize>>,
}

/// Process-local proof counters. The latter two counters intentionally remain
/// zero: editor completion does not build physical plans or execute data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SqlCompletionMetrics {
    pub requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub logical_expression_plans: u64,
    pub logical_query_plans: u64,
    pub physical_plans: u64,
    pub executions: u64,
}

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static LOGICAL_EXPRESSION_PLANS: AtomicU64 = AtomicU64::new(0);
static LOGICAL_QUERY_PLANS: AtomicU64 = AtomicU64::new(0);

impl SqlCompletionMetrics {
    pub fn snapshot() -> Self {
        Self {
            requests: REQUESTS.load(Ordering::Relaxed),
            cache_hits: CACHE_HITS.load(Ordering::Relaxed),
            cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
            logical_expression_plans: LOGICAL_EXPRESSION_PLANS.load(Ordering::Relaxed),
            logical_query_plans: LOGICAL_QUERY_PLANS.load(Ordering::Relaxed),
            physical_plans: 0,
            executions: 0,
        }
    }
}

pub(crate) struct SqlCompletionOutput {
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
    #[allow(dead_code)]
    pub debug: SqlCompletionDebug,
}

#[derive(Clone, Debug)]
struct SqlToken<'a> {
    token: Option<&'a Token>,
    raw: &'a str,
    span: SourceSpan,
    depth: usize,
}

impl SqlToken<'_> {
    fn word(&self) -> Option<&str> {
        match self.token {
            Some(Token::Word(word)) => Some(word.value.as_str()),
            _ => None,
        }
    }

    fn is_word(&self, expected: &str) -> bool {
        self.word()
            .is_some_and(|word| word.eq_ignore_ascii_case(expected))
    }

    fn is_period(&self) -> bool {
        matches!(self.token, Some(Token::Period)) || self.raw == "."
    }

    fn is_comma(&self) -> bool {
        matches!(self.token, Some(Token::Comma)) || self.raw == ","
    }
}

#[derive(Clone, Debug)]
struct ColumnMetadata {
    name: String,
    qualifier: Option<String>,
    data_type: DataType,
    nullable: bool,
    stage: String,
    lineage: Option<String>,
    detail: Option<String>,
}

#[derive(Clone, Debug)]
struct RelationMetadata {
    path: Vec<String>,
    alias: Option<String>,
    columns: Vec<ColumnMetadata>,
    detail: String,
    scope_depth: usize,
}

impl RelationMetadata {
    fn visible_name(&self) -> &str {
        self.alias
            .as_deref()
            .or_else(|| self.path.last().map(String::as_str))
            .unwrap_or("")
    }
}

#[derive(Clone, Debug, Default)]
struct QueryScope {
    relations: Vec<RelationMetadata>,
    projection_aliases: Vec<ColumnMetadata>,
    ctes: Vec<RelationMetadata>,
    projection_aliases_visible: bool,
}

#[derive(Clone, Debug)]
struct CachedSqlAnalysis {
    roles: BTreeSet<SqlExpectedRole>,
    repair: SqlRepairStrategy,
    repair_attempts: usize,
    token_count: usize,
    catalog: Vec<RelationMetadata>,
    scope: QueryScope,
    expression_planned: bool,
    repaired_parse: bool,
    synthetic_ranges: Vec<Range<usize>>,
}

#[derive(Clone, Debug)]
struct RepairedSql {
    text: String,
    generated: Vec<Range<usize>>,
}

#[derive(Clone, Debug)]
pub(crate) struct ContextualTokenSpans {
    pub root: SourceSpan,
    pub root_is_builtin: bool,
    pub properties: Vec<SourceSpan>,
    pub field: Option<SourceSpan>,
}

impl RepairedSql {
    fn authored_offset(&self, repaired_offset: usize) -> Option<usize> {
        let mut generated_before = 0;
        for generated in &self.generated {
            if generated.contains(&repaired_offset) {
                return None;
            }
            if generated.end <= repaired_offset {
                generated_before += generated.len();
            }
        }
        Some(repaired_offset.saturating_sub(generated_before))
    }
}

struct RecoveryResult {
    strategy: SqlRepairStrategy,
    attempts: usize,
    repaired: RepairedSql,
    parsed: bool,
}

static SQL_ANALYSIS_CACHE: OnceLock<Mutex<BTreeMap<String, Arc<CachedSqlAnalysis>>>> =
    OnceLock::new();

fn sql_analysis_cache() -> &'static Mutex<BTreeMap<String, Arc<CachedSqlAnalysis>>> {
    SQL_ANALYSIS_CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub(crate) fn complete_sql(
    request: &PositionRequest,
    syntax: &SyntaxAnalysis,
    registry: &NativeSchemaSnapshot,
    semantic_index: &WorkspaceSemanticIndex,
    semantic_roots: &BTreeMap<String, RootAnalysis>,
    dataset_contexts: &BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    cancellation: &AnalysisCancellation,
) -> Option<SqlCompletionOutput> {
    cancellation.check().ok()?;
    let node = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. })
                && node.span.range.start <= request.byte_offset
                && request.byte_offset <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())?;
    let TolerantSyntaxNodeKind::SqlIsland { context } = node.kind else {
        return None;
    };

    REQUESTS.fetch_add(1, Ordering::Relaxed);
    let replacement = sql_replacement_span(syntax, node.span, request.byte_offset);
    let text = syntax.parsed.tokens.text();
    let prefix = &text[replacement.range.start..request.byte_offset.min(text.len())];
    let (project, dataset_context) = project_at(
        &request.source,
        request.byte_offset,
        semantic_roots,
        dataset_contexts,
    );
    let tokens = island_tokens(syntax, node.span);
    let document = semantic_index.documents.get(&request.source);
    let cache_key = sql_cache_key(request, node.span, project, dataset_context);
    let cached = sql_analysis_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&cache_key)
        .cloned();
    let cached = if let Some(cached) = cached {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        cached
    } else {
        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
        let roles = expected_roles(&tokens, request.byte_offset, context, prefix);
        let recovery = recover_sql(
            &text[node.span.range.as_range()],
            request.byte_offset.saturating_sub(node.span.range.start),
            context.root(),
            &tokens,
            request.byte_offset,
            &roles,
        );
        debug_assert!(
            recovery
                .repaired
                .generated
                .iter()
                .all(|generated| { recovery.repaired.authored_offset(generated.start).is_none() })
        );
        let mut catalog = project.map(relation_catalog).unwrap_or_default();
        catalog.extend(table_binding_relations(
            document,
            request.byte_offset,
            project,
        ));
        let active_depth = cursor_depth(&tokens, request.byte_offset);
        let mut scope = build_query_scope(
            &tokens,
            request.byte_offset,
            active_depth,
            &catalog,
            project,
            dataset_context,
        );
        if context.root() == SqlIslandRoot::Expression
            && let Some(datum) = event_datum_relation(project, &request.source, request.byte_offset)
        {
            scope.relations.push(datum);
        }
        if context.root() == SqlIslandRoot::Query {
            reconcile_query_output_with_datafusion(
                &text[node.span.range.as_range()],
                &catalog,
                &mut scope,
            );
        }
        let expression_planned = if matches!(
            context.root(),
            SqlIslandRoot::Expression | SqlIslandRoot::Projection
        ) {
            project
                .and_then(|project| input_dataset(project, dataset_context, request.byte_offset))
                .is_some_and(|dataset| {
                    if context.root() == SqlIslandRoot::Projection {
                        validate_projection(
                            &text[node.span.range.as_range()],
                            dataset,
                            recovery.strategy,
                            request.byte_offset.saturating_sub(node.span.range.start),
                        )
                    } else {
                        validate_expression(
                            &text[node.span.range.as_range()],
                            dataset,
                            recovery.strategy,
                            request.byte_offset.saturating_sub(node.span.range.start),
                        )
                    }
                })
        } else {
            false
        };
        let cached = Arc::new(CachedSqlAnalysis {
            roles,
            repair: recovery.strategy,
            repair_attempts: recovery.attempts,
            token_count: tokens.len(),
            catalog,
            scope,
            expression_planned,
            repaired_parse: recovery.parsed,
            synthetic_ranges: recovery.repaired.generated,
        });
        let mut cache = sql_analysis_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.len() >= 512
            && let Some(oldest) = cache.keys().next().cloned()
        {
            cache.remove(&oldest);
        }
        cache.insert(cache_key, Arc::clone(&cached));
        cached
    };
    let roles = &cached.roles;
    let catalog = &cached.catalog;
    let scope = &cached.scope;

    let mut items = Vec::new();
    let mut incomplete = project.is_none();
    let qualifier = qualifier_before(text, node.span, replacement.range.start);
    let quoted_qualifier = quoted_qualifier_before(text, node.span, replacement.range.start);
    let qualifier = quoted_qualifier
        .as_ref()
        .map(|(qualifier, _)| qualifier.clone())
        .or(qualifier);
    let member_replacement = quoted_qualifier
        .map(|(_, quote_start)| SourceSpan {
            source: replacement.source,
            range: ByteSpan {
                start: quote_start,
                end: replacement.range.end,
            },
        })
        .unwrap_or(replacement);

    if (roles.contains(&SqlExpectedRole::QualifierMember) || qualifier.is_some())
        && let Some(qualifier) = qualifier.as_deref()
    {
        let contextual = complete_contextual_qualifier(
            qualifier,
            prefix,
            member_replacement,
            syntax,
            project,
            &request.source,
            request.byte_offset,
            registry,
            &mut items,
        );
        let found = contextual.unwrap_or_else(|| {
            complete_qualifier(
                qualifier,
                prefix,
                member_replacement,
                roles,
                scope,
                catalog,
                document,
                project,
                request.byte_offset,
                &mut items,
            )
        });
        incomplete |= !found;
    }
    if roles.contains(&SqlExpectedRole::Relation)
        || roles.contains(&SqlExpectedRole::Catalog)
        || roles.contains(&SqlExpectedRole::Schema)
    {
        complete_relations(prefix, replacement, scope, catalog, &mut items);
        complete_table_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            project,
            &mut items,
        );
    }
    if roles.contains(&SqlExpectedRole::Expression) || roles.contains(&SqlExpectedRole::Function) {
        complete_contextual_roots(
            prefix,
            replacement,
            syntax,
            project,
            &request.source,
            request.byte_offset,
            registry,
            &mut items,
        );
        complete_columns(prefix, replacement, scope, &mut items);
        complete_scalar_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            &mut items,
        );
        complete_functions(
            prefix,
            replacement,
            project,
            enclosing_event(project, &request.source, request.byte_offset).is_some(),
            &mut items,
        );
    }
    if roles.contains(&SqlExpectedRole::Type) {
        complete_types(prefix, replacement, &mut items);
    }
    if roles.contains(&SqlExpectedRole::Binding) {
        complete_scalar_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            &mut items,
        );
        complete_table_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            project,
            &mut items,
        );
    }
    if roles.contains(&SqlExpectedRole::Alias) {
        complete_projection_alias(
            prefix,
            replacement,
            &tokens,
            request.byte_offset,
            &mut items,
        );
    }
    complete_temporal_qualifiers(
        prefix,
        replacement,
        document,
        request.byte_offset,
        &mut items,
    );
    complete_keywords(prefix, replacement, roles, &mut items);

    cancellation.check().ok()?;
    rank_and_deduplicate(&mut items, prefix);
    Some(SqlCompletionOutput {
        items,
        is_incomplete: incomplete,
        debug: SqlCompletionDebug {
            island: node.span,
            roles: roles.clone(),
            repair: cached.repair,
            repair_attempts: cached.repair_attempts,
            token_count: cached.token_count,
            expression_planned: cached.expression_planned,
            repaired_parse: cached.repaired_parse,
            synthetic_ranges: cached.synthetic_ranges.clone(),
        },
    })
}

pub(crate) fn contextual_hover(
    request: &PositionRequest,
    syntax: &SyntaxAnalysis,
    registry: &NativeSchemaSnapshot,
    semantic_roots: &BTreeMap<String, RootAnalysis>,
    dataset_contexts: &BTreeMap<SourceOrigin, Vec<DatasetContext>>,
) -> Option<(SourceSpan, String)> {
    let node = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(
                node.kind,
                TolerantSyntaxNodeKind::SqlIsland { context }
                    if context.root() == SqlIslandRoot::Expression
            ) && node.span.range.start <= request.byte_offset
                && request.byte_offset <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())?;
    let tokens = island_tokens(syntax, node.span);
    let (project, _) = project_at(
        &request.source,
        request.byte_offset,
        semantic_roots,
        dataset_contexts,
    );
    let reference = contextual_references_in_tokens(&tokens)
        .into_iter()
        .find(|reference| {
            contextual_reference_is_legal(reference, project, &request.source, request.byte_offset)
                && reference.full_span.range.start <= request.byte_offset
                && request.byte_offset <= reference.full_span.range.end
        })?;
    let detail = contextual_reference_detail(
        &reference,
        project,
        &request.source,
        request.byte_offset,
        registry,
    )?;
    let text = syntax.parsed.tokens.text();
    let spelling = &text[reference.full_span.range.as_range()];
    Some((
        reference.full_span,
        format!("```avenger\n{spelling}\n```\n\n{detail}"),
    ))
}

pub(crate) fn contextual_semantic_token_spans(
    analysis: &crate::WorkspaceAnalysis,
    origin: &SourceOrigin,
) -> Vec<ContextualTokenSpans> {
    let Some(syntax) = analysis.syntax.get(origin) else {
        return Vec::new();
    };
    let mut output = Vec::new();
    for node in &syntax.parsed.nodes {
        let TolerantSyntaxNodeKind::SqlIsland { context } = node.kind else {
            continue;
        };
        if context.root() != SqlIslandRoot::Expression {
            continue;
        }
        let cursor = node.span.range.start;
        let (project, _) = project_at(
            origin,
            cursor,
            &analysis.semantic_roots,
            &analysis.dataset_contexts,
        );
        output.extend(
            contextual_references_in_tokens(&island_tokens(syntax, node.span))
                .into_iter()
                .filter(|reference| {
                    contextual_reference_is_legal(reference, project, origin, cursor)
                })
                .map(|reference| reference.spans),
        );
    }
    output
}

#[derive(Clone, Debug)]
struct ContextualTokenReference {
    spans: ContextualTokenSpans,
    parts: Vec<String>,
    quoted: Vec<bool>,
    full_span: SourceSpan,
}

fn contextual_references_in_tokens(tokens: &[SqlToken<'_>]) -> Vec<ContextualTokenReference> {
    let mut output = Vec::new();
    for (start, root) in tokens.iter().enumerate() {
        let Some(Token::Word(root_word)) = root.token else {
            continue;
        };
        if root_word.quote_style.is_some() {
            continue;
        }
        let mut parts = vec![root_word.value.clone()];
        let mut quoted = vec![false];
        let mut properties = Vec::new();
        let mut field = None;
        let mut cursor = start;
        while cursor + 2 < tokens.len() && tokens[cursor + 1].is_period() {
            let Some(Token::Word(member)) = tokens[cursor + 2].token else {
                break;
            };
            parts.push(member.value.clone());
            quoted.push(member.quote_style == Some('"'));
            if member.quote_style == Some('"') {
                field = Some(tokens[cursor + 2].span);
            } else {
                properties.push(tokens[cursor + 2].span);
            }
            cursor += 2;
        }
        if parts.len() < 2 {
            continue;
        }
        let mut end = tokens[cursor].span.range.end;
        if parts.len() == 2
            && parts[0].eq_ignore_ascii_case("event")
            && parts[1].eq_ignore_ascii_case("facet")
            && cursor + 3 < tokens.len()
            && matches!(tokens[cursor + 1].token, Some(Token::LBracket))
            && matches!(tokens[cursor + 3].token, Some(Token::RBracket))
        {
            end = tokens[cursor + 3].span.range.end;
        }
        let root_is_builtin = matches!(
            parts[0].to_ascii_lowercase().as_str(),
            "datum" | "channel" | "event" | "item"
        );
        output.push(ContextualTokenReference {
            spans: ContextualTokenSpans {
                root: root.span,
                root_is_builtin,
                properties,
                field,
            },
            parts,
            quoted,
            full_span: SourceSpan {
                source: root.span.source,
                range: ByteSpan {
                    start: root.span.range.start,
                    end,
                },
            },
        });
    }
    output
}

fn contextual_reference_is_legal(
    reference: &ContextualTokenReference,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
) -> bool {
    let parts = reference
        .parts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let quoted = &reference.quoted;
    let event = enclosing_event(project, origin, cursor);
    let path = declaration_path_at(project, origin, cursor);
    let mark = path.iter().any(|declaration| declaration.keyword == "mark");
    let item = mark
        && path
            .iter()
            .any(|declaration| matches!(declaration.keyword.as_str(), "adjust" | "derive"));
    let between = event.is_some_and(|event| event.properties.contains_key("between"));
    let legend = event
        .and_then(|event| event.event_binding.as_ref())
        .is_some_and(|binding| matches!(binding.surface, ResolvedEventSurface::Legend { .. }));
    match parts.as_slice() {
        [root, _] if root.eq_ignore_ascii_case("datum") => {
            event.is_some() && quoted.get(1) == Some(&true)
        }
        [root, _] if root.eq_ignore_ascii_case("channel") => mark && quoted.get(1) == Some(&false),
        [root, coord, _]
            if root.eq_ignore_ascii_case("event") && coord.eq_ignore_ascii_case("coord") =>
        {
            event.is_some() && quoted.iter().all(|quoted| !quoted)
        }
        [root, start, coord, _]
            if root.eq_ignore_ascii_case("event")
                && start.eq_ignore_ascii_case("start")
                && coord.eq_ignore_ascii_case("coord") =>
        {
            between && quoted.iter().all(|quoted| !quoted)
        }
        [root, domain, _, boundary]
            if root.eq_ignore_ascii_case("event")
                && domain.eq_ignore_ascii_case("domain")
                && (boundary.eq_ignore_ascii_case("start")
                    || boundary.eq_ignore_ascii_case("end")) =>
        {
            event.is_some() && quoted.iter().all(|quoted| !quoted)
        }
        [root, path] if root.eq_ignore_ascii_case("event") && path.eq_ignore_ascii_case("path") => {
            between && quoted.iter().all(|quoted| !quoted)
        }
        [root, facet]
            if root.eq_ignore_ascii_case("event") && facet.eq_ignore_ascii_case("facet") =>
        {
            event.is_some() && quoted.iter().all(|quoted| !quoted)
        }
        [root, legend_member, value]
            if root.eq_ignore_ascii_case("event")
                && legend_member.eq_ignore_ascii_case("legend")
                && value.eq_ignore_ascii_case("value") =>
        {
            legend && quoted.iter().all(|quoted| !quoted)
        }
        [root, channel, _]
            if root.eq_ignore_ascii_case("item") && channel.eq_ignore_ascii_case("channel") =>
        {
            item && quoted.iter().all(|quoted| !quoted)
        }
        [root, data, _]
            if root.eq_ignore_ascii_case("item") && data.eq_ignore_ascii_case("data") =>
        {
            item && quoted.get(2) == Some(&true)
        }
        [root, bbox, edge]
            if root.eq_ignore_ascii_case("item")
                && bbox.eq_ignore_ascii_case("bbox")
                && ["top", "right", "bottom", "left"]
                    .iter()
                    .any(|candidate| edge.eq_ignore_ascii_case(candidate)) =>
        {
            item && quoted.iter().all(|quoted| !quoted)
        }
        [view, axis, pixels]
            if (axis.eq_ignore_ascii_case("x") || axis.eq_ignore_ascii_case("y"))
                && pixels.eq_ignore_ascii_case("pixels") =>
        {
            path.iter().any(|declaration| {
                declaration.keyword == "view"
                    && declaration
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(view))
            }) && quoted.iter().all(|quoted| !quoted)
        }
        [view, axis, domain, boundary]
            if (axis.eq_ignore_ascii_case("x") || axis.eq_ignore_ascii_case("y"))
                && domain.eq_ignore_ascii_case("domain")
                && (boundary.eq_ignore_ascii_case("start")
                    || boundary.eq_ignore_ascii_case("end")) =>
        {
            path.iter().any(|declaration| {
                declaration.keyword == "view"
                    && declaration
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(view))
            }) && quoted.iter().all(|quoted| !quoted)
        }
        _ => false,
    }
}

fn contextual_reference_detail(
    reference: &ContextualTokenReference,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    registry: &NativeSchemaSnapshot,
) -> Option<String> {
    let parts = reference
        .parts
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let path = declaration_path_at(project, origin, cursor);
    let mark = path
        .iter()
        .rev()
        .copied()
        .find(|declaration| declaration.keyword == "mark");
    match parts.as_slice() {
        [root, field] if root.eq_ignore_ascii_case("datum") => {
            let relation = event_datum_relation(project, origin, cursor)?;
            let column = relation
                .columns
                .iter()
                .find(|column| column.name == *field)?;
            let detail = column.detail.clone().unwrap_or_else(|| {
                format!(
                    "{} · {} · logical hit row",
                    column.data_type,
                    nullability(column.nullable)
                )
            });
            Some(format!(
                "{}\n\n{detail}",
                contextual_signature_docs("datum.\"<field>\"")?
            ))
        }
        [root, channel] if root.eq_ignore_ascii_case("channel") => {
            let channel = mark_channels(project, mark?, registry, false)
                .into_iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(channel))?;
            let type_detail = channel.data_type.as_ref().map(|data_type| {
                format!(
                    "`{data_type}` · {}.\n\n",
                    channel
                        .nullable
                        .map(nullability)
                        .unwrap_or("nullability unavailable")
                )
            });
            Some(format!(
                "{}\n\n{}{}",
                contextual_signature_docs("channel.<channel>")?,
                type_detail.as_deref().unwrap_or_default(),
                channel.docs,
            ))
        }
        [root, coord, channel]
            if root.eq_ignore_ascii_case("event") && coord.eq_ignore_ascii_case("coord") =>
        {
            let detail = fixed_contextual_signature_detail("event.coord.<channel>")?;
            Some(format!("Channel `{channel}`.\n\n{detail}"))
        }
        [root, start, coord, channel]
            if root.eq_ignore_ascii_case("event")
                && start.eq_ignore_ascii_case("start")
                && coord.eq_ignore_ascii_case("coord") =>
        {
            let detail = fixed_contextual_signature_detail("event.start.coord.<channel>")?;
            Some(format!("Channel `{channel}`.\n\n{detail}"))
        }
        [root, domain, channel, boundary]
            if root.eq_ignore_ascii_case("event") && domain.eq_ignore_ascii_case("domain") =>
        {
            let pattern = format!("event.domain.<channel>.{}", boundary.to_ascii_lowercase());
            let detail = fixed_contextual_signature_detail(&pattern)?;
            Some(format!("Channel `{channel}`.\n\n{detail}"))
        }
        [root, path] if root.eq_ignore_ascii_case("event") && path.eq_ignore_ascii_case("path") => {
            fixed_contextual_signature_detail("event.path")
        }
        [root, facet]
            if root.eq_ignore_ascii_case("event") && facet.eq_ignore_ascii_case("facet") =>
        {
            fixed_contextual_signature_detail("event.facet[n]")
        }
        [root, legend, value]
            if root.eq_ignore_ascii_case("event")
                && legend.eq_ignore_ascii_case("legend")
                && value.eq_ignore_ascii_case("value") =>
        {
            fixed_contextual_signature_detail("event.legend.value")
        }
        [root, channel_member, channel]
            if root.eq_ignore_ascii_case("item")
                && channel_member.eq_ignore_ascii_case("channel") =>
        {
            let channel = mark_channels(project, mark?, registry, true)
                .into_iter()
                .find(|candidate| candidate.name.eq_ignore_ascii_case(channel))?;
            Some(format!(
                "{}\n\n{} · {} · {}",
                contextual_signature_docs("item.channel.<channel>")?,
                channel.data_type.as_deref().unwrap_or("schema-derived"),
                channel
                    .nullable
                    .map(nullability)
                    .unwrap_or("nullability unavailable"),
                channel.docs,
            ))
        }
        [root, data, field]
            if root.eq_ignore_ascii_case("item") && data.eq_ignore_ascii_case("data") =>
        {
            let column = mark_input_columns(project, mark?)
                .into_iter()
                .find(|column| column.name == *field)?;
            Some(format!(
                "{}\n\n{} · {}",
                contextual_signature_docs("item.data.\"<field>\"")?,
                column.data_type,
                nullability(column.nullable),
            ))
        }
        [root, bbox, edge]
            if root.eq_ignore_ascii_case("item") && bbox.eq_ignore_ascii_case("bbox") =>
        {
            let detail = fixed_contextual_signature_detail("item.bbox.<edge>")?;
            Some(format!("Edge `{edge}`.\n\n{detail}"))
        }
        [_, axis, pixels] if pixels.eq_ignore_ascii_case("pixels") => {
            fixed_contextual_signature_detail(&format!(
                "<view>.{}.pixels",
                axis.to_ascii_lowercase()
            ))
        }
        [_, axis, domain, boundary] if domain.eq_ignore_ascii_case("domain") => {
            fixed_contextual_signature_detail(&format!(
                "<view>.{}.domain.{}",
                axis.to_ascii_lowercase(),
                boundary.to_ascii_lowercase()
            ))
        }
        _ => None,
    }
}

fn contextual_signature_docs(pattern: &str) -> Option<&'static str> {
    contextual_access_signature(pattern).map(|signature| signature.docs)
}

fn fixed_contextual_signature_detail(pattern: &str) -> Option<String> {
    let signature = contextual_access_signature(pattern)?;
    let data_type = signature.fixed_arrow_type?.as_str();
    Some(format!(
        "{}\n\n`{data_type}` · {}.",
        signature.docs,
        nullability(signature.nullable)
    ))
}

fn sql_cache_key(
    request: &PositionRequest,
    island: SourceSpan,
    project: Option<&ModuleAnalysis>,
    context: Option<&DatasetContext>,
) -> String {
    let mut hash = Sha256::new();
    hash.update(b"avenger-sql-completion-v1\0");
    hash.update(request.source.canonical_uri().as_bytes());
    hash.update(b"\0");
    hash.update(request.source_revision.as_str().as_bytes());
    hash.update(island.range.start.to_le_bytes());
    hash.update(island.range.end.to_le_bytes());
    hash.update(request.byte_offset.to_le_bytes());
    if let Some(project) = project {
        hash.update(project.module_fingerprint.as_str().as_bytes());
    }
    if let Some(context) = context {
        hash.update(context.stage.dataset.as_str().as_bytes());
        hash.update(context.stage.ordinal.to_le_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn project_at<'a>(
    origin: &SourceOrigin,
    cursor: usize,
    roots: &'a BTreeMap<String, RootAnalysis>,
    contexts: &'a BTreeMap<SourceOrigin, Vec<DatasetContext>>,
) -> (Option<&'a ModuleAnalysis>, Option<&'a DatasetContext>) {
    let context = contexts
        .iter()
        .find(|(candidate, _)| same_origin(candidate, origin))
        .map(|(_, contexts)| contexts)
        .and_then(|contexts| {
            contexts
                .iter()
                .filter(|context| {
                    context.span.range.start <= cursor && cursor <= context.span.range.end
                })
                .min_by_key(|context| context.span.range.len())
        });
    if let Some(context) = context {
        return (
            roots
                .get(&context.root_uri)
                .and_then(|root| root.result.as_ref().ok()),
            Some(context),
        );
    }
    let project = roots.values().find_map(|root| {
        let project = root.result.as_ref().ok()?;
        project
            .sources
            .iter()
            .any(|(_, source)| same_origin(&source.origin, origin))
            .then_some(project)
    });
    (project, None)
}

fn same_origin(left: &SourceOrigin, right: &SourceOrigin) -> bool {
    if left == right || left.canonical_uri() == right.canonical_uri() {
        return true;
    }
    match (left, right) {
        (SourceOrigin::File(left), SourceOrigin::File(right)) => {
            let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.clone());
            let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.clone());
            left == right
        }
        _ => false,
    }
}

fn island_tokens<'a>(syntax: &'a SyntaxAnalysis, island: SourceSpan) -> Vec<SqlToken<'a>> {
    let mut depth = 0_usize;
    syntax
        .parsed
        .tokens
        .tokens()
        .iter()
        .filter(|token| {
            island.range.start <= token.span().range.start
                && token.span().range.end <= island.range.end
                && !matches!(
                    token.kind(),
                    LosslessTokenKind::Token(TokenClass::Whitespace(_) | TokenClass::Comment(_))
                        | LosslessTokenKind::Eof
                )
        })
        .map(|token| {
            if matches!(token.token(), Some(Token::RParen | Token::RBracket)) {
                depth = depth.saturating_sub(1);
            }
            let output = SqlToken {
                token: token.token(),
                raw: syntax.parsed.tokens.raw(token),
                span: token.span(),
                depth,
            };
            if matches!(token.token(), Some(Token::LParen | Token::LBracket)) {
                depth = depth.saturating_add(1);
            }
            output
        })
        .collect()
}

fn cursor_depth(tokens: &[SqlToken<'_>], cursor: usize) -> usize {
    tokens
        .iter()
        .take_while(|token| token.span.range.end <= cursor)
        .last()
        .map_or(0, |token| {
            token.depth + usize::from(matches!(token.token, Some(Token::LParen | Token::LBracket)))
        })
}

fn sql_replacement_span(syntax: &SyntaxAnalysis, island: SourceSpan, cursor: usize) -> SourceSpan {
    let text = syntax.parsed.tokens.text();
    let cursor = cursor.min(island.range.end).min(text.len());
    let mut start = cursor;
    while start > island.range.start {
        let character = text[..start].chars().next_back().unwrap();
        if character == '_' || character == '$' || character == '@' || character.is_alphanumeric() {
            start -= character.len_utf8();
        } else {
            break;
        }
    }
    let mut end = cursor;
    while end < island.range.end.min(text.len()) {
        let character = text[end..].chars().next().unwrap();
        if character == '_' || character == '$' || character == '@' || character.is_alphanumeric() {
            end += character.len_utf8();
        } else {
            break;
        }
    }
    SourceSpan {
        source: island.source,
        range: ByteSpan { start, end },
    }
}

fn expected_roles(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    context: SqlIslandContext,
    prefix: &str,
) -> BTreeSet<SqlExpectedRole> {
    let before = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.span.range.end <= cursor)
        .map(|(index, _)| index)
        .next_back();
    let previous = before.and_then(|index| tokens.get(index));
    let structural_before = before.and_then(|index| {
        let token = &tokens[index];
        if !prefix.is_empty()
            && token.span.range.end == cursor
            && token.span.range.start < cursor
            && !token.is_period()
        {
            index.checked_sub(1)
        } else {
            Some(index)
        }
    });
    let mut roles = BTreeSet::new();
    if prefix.starts_with('$') {
        roles.insert(SqlExpectedRole::Binding);
    }
    if previous.is_some_and(SqlToken::is_period) {
        roles.insert(SqlExpectedRole::QualifierMember);
        roles.insert(SqlExpectedRole::Expression);
        if before.is_some_and(|index| qualifier_is_relation_path(tokens, index)) {
            roles.insert(SqlExpectedRole::Catalog);
            roles.insert(SqlExpectedRole::Schema);
            roles.insert(SqlExpectedRole::Relation);
        }
        return roles;
    }
    if type_position(tokens, structural_before) {
        roles.insert(SqlExpectedRole::Type);
        return roles;
    }
    if context.root() == SqlIslandRoot::Projection
        && structural_before
            .and_then(|index| tokens.get(index))
            .is_some_and(|token| token.is_word("as"))
    {
        roles.insert(SqlExpectedRole::Alias);
        return roles;
    }
    if relation_position(tokens, structural_before, cursor) {
        roles.insert(SqlExpectedRole::Relation);
        roles.insert(SqlExpectedRole::Catalog);
        roles.insert(SqlExpectedRole::Schema);
        roles.insert(SqlExpectedRole::Binding);
    } else {
        roles.insert(SqlExpectedRole::Expression);
        roles.insert(SqlExpectedRole::Function);
        if context.root() == SqlIslandRoot::Query {
            roles.insert(SqlExpectedRole::Clause);
        }
    }
    roles
}

fn qualifier_is_relation_path(tokens: &[SqlToken<'_>], period: usize) -> bool {
    let depth = tokens[period].depth;
    tokens[..period]
        .iter()
        .rev()
        .find(|token| token.depth == depth && is_clause_token(token))
        .is_some_and(|token| token.is_word("from") || token.is_word("join"))
}

fn relation_position(tokens: &[SqlToken<'_>], before: Option<usize>, cursor: usize) -> bool {
    let depth = cursor_depth(tokens, cursor);
    let Some(index) = before else {
        return false;
    };
    if tokens[index].is_word("from") || tokens[index].is_word("join") {
        return true;
    }
    if tokens[index].is_comma() {
        let prior_clause = tokens[..index]
            .iter()
            .rev()
            .find(|token| token.depth == depth && is_clause_token(token));
        return prior_clause.is_some_and(|token| token.is_word("from"));
    }
    false
}

fn type_position(tokens: &[SqlToken<'_>], before: Option<usize>) -> bool {
    let Some(index) = before else {
        return false;
    };
    if tokens[index].is_word("as") {
        return tokens[..index]
            .iter()
            .rposition(|token| matches!(token.token, Some(Token::LParen)))
            .and_then(|open| open.checked_sub(1))
            .and_then(|cast| tokens.get(cast))
            .is_some_and(|token| token.is_word("cast") || token.is_word("try_cast"));
    }
    tokens[index].raw == "::"
}

fn select_repair(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    roles: &BTreeSet<SqlExpectedRole>,
) -> (SqlRepairStrategy, usize) {
    let previous = tokens.iter().rfind(|token| token.span.range.end <= cursor);
    let candidates = [
        roles
            .contains(&SqlExpectedRole::QualifierMember)
            .then_some(SqlRepairStrategy::QualifierMember),
        previous
            .is_some_and(SqlToken::is_comma)
            .then_some(SqlRepairStrategy::TrailingComma),
        roles
            .contains(&SqlExpectedRole::Relation)
            .then_some(SqlRepairStrategy::MissingRelation),
        roles
            .contains(&SqlExpectedRole::Expression)
            .then_some(SqlRepairStrategy::EmptyExpression),
        (tokens
            .iter()
            .filter(|token| matches!(token.token, Some(Token::LParen)))
            .count()
            > tokens
                .iter()
                .filter(|token| matches!(token.token, Some(Token::RParen)))
                .count())
        .then_some(SqlRepairStrategy::CloseDelimiters),
    ]
    .into_iter()
    .flatten()
    .take(MAX_REPAIR_ATTEMPTS)
    .collect::<Vec<_>>();
    (
        candidates
            .first()
            .copied()
            .unwrap_or(SqlRepairStrategy::None),
        candidates.len(),
    )
}

fn recover_sql(
    authored: &str,
    cursor: usize,
    root: SqlIslandRoot,
    tokens: &[SqlToken<'_>],
    absolute_cursor: usize,
    roles: &BTreeSet<SqlExpectedRole>,
) -> RecoveryResult {
    let mut strategies = vec![SqlRepairStrategy::None];
    let preferred = select_repair(tokens, absolute_cursor, roles).0;
    if preferred != SqlRepairStrategy::None {
        strategies.push(preferred);
    }
    let previous = tokens
        .iter()
        .rfind(|token| token.span.range.end <= absolute_cursor);
    for strategy in [
        roles
            .contains(&SqlExpectedRole::QualifierMember)
            .then_some(SqlRepairStrategy::QualifierMember),
        previous
            .is_some_and(SqlToken::is_comma)
            .then_some(SqlRepairStrategy::TrailingComma),
        roles
            .contains(&SqlExpectedRole::Relation)
            .then_some(SqlRepairStrategy::MissingRelation),
        roles
            .contains(&SqlExpectedRole::Expression)
            .then_some(SqlRepairStrategy::EmptyExpression),
        Some(SqlRepairStrategy::CloseDelimiters),
    ]
    .into_iter()
    .flatten()
    {
        if !strategies.contains(&strategy) {
            strategies.push(strategy);
        }
    }
    strategies.truncate(MAX_REPAIR_ATTEMPTS);

    let mut fallback = None;
    for (attempt, strategy) in strategies.iter().copied().enumerate() {
        let repaired = build_repaired_sql(authored, cursor, strategy);
        let parsed = match root {
            SqlIslandRoot::Query => SqlQuery::parse(&repaired.text).is_ok(),
            SqlIslandRoot::Projection => SqlProjection::parse(&repaired.text).is_ok(),
            SqlIslandRoot::Expression => SqlExpression::parse(&repaired.text).is_ok(),
        };
        fallback = Some(RecoveryResult {
            strategy,
            attempts: attempt + 1,
            repaired,
            parsed,
        });
        if parsed {
            return fallback.unwrap();
        }
    }
    fallback.unwrap_or_else(|| RecoveryResult {
        strategy: SqlRepairStrategy::None,
        attempts: 0,
        repaired: build_repaired_sql(authored, cursor, SqlRepairStrategy::None),
        parsed: false,
    })
}

fn build_repaired_sql(authored: &str, cursor: usize, strategy: SqlRepairStrategy) -> RepairedSql {
    let cursor = cursor.min(authored.len());
    let marker = match strategy {
        SqlRepairStrategy::QualifierMember => "__avenger_cursor_member",
        SqlRepairStrategy::EmptyExpression | SqlRepairStrategy::TrailingComma => "NULL",
        SqlRepairStrategy::MissingRelation => "__avenger_cursor_relation",
        SqlRepairStrategy::CloseDelimiters | SqlRepairStrategy::None => "",
    };
    let mut text = String::with_capacity(authored.len() + marker.len() + 8);
    text.push_str(&authored[..cursor]);
    let mut generated = Vec::new();
    if !marker.is_empty() {
        generated.push(cursor..cursor + marker.len());
        text.push_str(marker);
    }
    text.push_str(&authored[cursor..]);
    if strategy == SqlRepairStrategy::CloseDelimiters {
        let missing = authored
            .matches('(')
            .count()
            .saturating_sub(authored.matches(')').count());
        if missing > 0 {
            let start = text.len();
            text.extend(std::iter::repeat_n(')', missing));
            generated.push(start..text.len());
        }
    }
    RepairedSql { text, generated }
}

fn relation_catalog(project: &ModuleAnalysis) -> Vec<RelationMetadata> {
    project
        .datasets
        .iter()
        .filter_map(|(stage, dataset)| {
            let path = dataset.qualified_path.clone().or_else(|| {
                dataset
                    .qualified_name
                    .as_ref()
                    .map(|name| vec![name.clone()])
            })?;
            let lineage = project.lineage.get(stage);
            Some(RelationMetadata {
                path,
                alias: None,
                columns: dataset
                    .columns
                    .iter()
                    .map(|column| ColumnMetadata {
                        name: column.name.clone(),
                        qualifier: column.qualifier.clone(),
                        data_type: column.data_type.clone(),
                        nullable: column.nullable,
                        stage: format!("{}#{}", stage.dataset.as_str(), stage.ordinal),
                        lineage: lineage.and_then(|lineage| {
                            lineage
                                .columns
                                .iter()
                                .find(|entry| entry.output_column == column.name)
                                .map(|entry| {
                                    entry
                                        .inputs
                                        .iter()
                                        .map(|(stage, name)| {
                                            format!(
                                                "{}#{}.{}",
                                                stage.dataset.as_str(),
                                                stage.ordinal,
                                                name
                                            )
                                        })
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                })
                        }),
                        detail: None,
                    })
                    .collect(),
                detail: format!(
                    "{:?} · {}#{}",
                    dataset.provenance.stage_kind,
                    stage.dataset.as_str(),
                    stage.ordinal
                ),
                scope_depth: 0,
            })
        })
        .collect()
}

fn enclosing_event<'a>(
    project: Option<&'a ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
) -> Option<&'a ResolvedDeclaration> {
    let resolved = project?.resolved_module_graph.as_deref()?;
    resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
        .filter_map(|root| find_enclosing_event(root, resolved, origin, cursor))
        .min_by_key(|event| event.span.range.len())
}

fn syntax_contains_declaration(
    syntax: &SyntaxAnalysis,
    cursor: usize,
    expected_keyword: &str,
) -> bool {
    syntax.parsed.nodes.iter().any(|node| {
        matches!(
            &node.kind,
            TolerantSyntaxNodeKind::Declaration { keyword, .. }
                if keyword.eq_ignore_ascii_case(expected_keyword)
        ) && node.span.range.start <= cursor
            && cursor <= node.span.range.end
    })
}

fn nearest_declaration<'a>(
    project: Option<&'a ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    expected_keyword: &str,
) -> Option<&'a ResolvedDeclaration> {
    let resolved = project?.resolved_module_graph.as_deref()?;
    let mut candidates = Vec::new();
    for root in resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
    {
        collect_declarations_by_keyword(
            root,
            resolved,
            origin,
            cursor,
            expected_keyword,
            &mut candidates,
        );
    }
    candidates.into_iter().max_by_key(|declaration| {
        resolved
            .expansion_source_map
            .authored_span(declaration.span)
            .range
            .start
    })
}

fn collect_declarations_by_keyword<'a>(
    declaration: &'a ResolvedDeclaration,
    resolved: &avenger_lang_core::ResolvedModuleGraph,
    origin: &SourceOrigin,
    cursor: usize,
    expected_keyword: &str,
    output: &mut Vec<&'a ResolvedDeclaration>,
) {
    let authored = resolved
        .expansion_source_map
        .authored_span(declaration.span);
    if let Some(source) = resolved.sources.get(authored.source)
        && same_origin(&source.origin, origin)
        && authored.range.start <= cursor
        && declaration.keyword.eq_ignore_ascii_case(expected_keyword)
    {
        output.push(declaration);
    }
    for child in &declaration.children {
        collect_declarations_by_keyword(child, resolved, origin, cursor, expected_keyword, output);
    }
}

fn declaration_path_at<'a>(
    project: Option<&'a ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
) -> Vec<&'a ResolvedDeclaration> {
    let Some(resolved) = project.and_then(|project| project.resolved_module_graph.as_deref())
    else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for root in resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
    {
        let mut path = Vec::new();
        collect_declaration_paths(root, resolved, origin, cursor, &mut path, &mut candidates);
    }
    candidates
        .into_iter()
        .min_by_key(|path| {
            path.last()
                .map(|declaration| {
                    resolved
                        .expansion_source_map
                        .authored_span(declaration.span)
                        .range
                        .len()
                })
                .unwrap_or(usize::MAX)
        })
        .unwrap_or_default()
}

fn collect_declaration_paths<'a>(
    declaration: &'a ResolvedDeclaration,
    resolved: &avenger_lang_core::ResolvedModuleGraph,
    origin: &SourceOrigin,
    cursor: usize,
    path: &mut Vec<&'a ResolvedDeclaration>,
    output: &mut Vec<Vec<&'a ResolvedDeclaration>>,
) {
    let authored = resolved
        .expansion_source_map
        .authored_span(declaration.span);
    let Some(source) = resolved.sources.get(authored.source) else {
        return;
    };
    if !same_origin(&source.origin, origin)
        || authored.range.start > cursor
        || cursor > authored.range.end
    {
        return;
    }
    path.push(declaration);
    output.push(path.clone());
    for child in &declaration.children {
        collect_declaration_paths(child, resolved, origin, cursor, path, output);
    }
    path.pop();
}

fn native_mark_schema<'a>(
    mark: &ResolvedDeclaration,
    registry: &'a NativeSchemaSnapshot,
) -> Option<&'a avenger_chart_schema::KindSchema> {
    let direct = match mark.kind_binding.as_ref() {
        Some(ResolvedKindBinding::Builtin(key)) => Some(key),
        Some(ResolvedKindBinding::Native { implementation, .. }) => Some(implementation),
        _ => None,
    }
    .filter(|key| key.namespace == NativeKindNamespace::Mark)
    .and_then(|key| registry.entries.get(key));
    direct.or_else(|| {
        let kind = mark.kind.as_deref()?;
        registry.entries.values().find(|schema| {
            schema.key.namespace == NativeKindNamespace::Mark
                && schema.key.kind.eq_ignore_ascii_case(kind)
                && schema.key.coordinate.as_deref() == mark.coordinate.as_deref()
        })
    })
}

fn mark_channels(
    project: Option<&ModuleAnalysis>,
    mark: &ResolvedDeclaration,
    registry: &NativeSchemaSnapshot,
    item_only: bool,
) -> Vec<ContextualChannel> {
    let Some(schema) = native_mark_schema(mark, registry) else {
        return Vec::new();
    };
    schema
        .channels
        .values()
        .filter(|channel| !item_only || channel.item_type.is_some())
        .map(|channel| ContextualChannel {
            name: channel.name.clone(),
            data_type: if item_only {
                channel.item_type.clone()
            } else {
                project
                    .and_then(|project| project.mark_channels.get(&mark.id))
                    .and_then(|channels| {
                        channels
                            .iter()
                            .find(|candidate| candidate.name.eq_ignore_ascii_case(&channel.name))
                    })
                    .map(|channel| channel.data_type.to_string())
            },
            nullable: if item_only {
                channel.item_type.as_ref().map(|_| true)
            } else {
                project
                    .and_then(|project| project.mark_channels.get(&mark.id))
                    .and_then(|channels| {
                        channels
                            .iter()
                            .find(|candidate| candidate.name.eq_ignore_ascii_case(&channel.name))
                    })
                    .map(|channel| channel.nullable)
            },
            docs: channel.docs.clone(),
        })
        .collect()
}

fn event_target_marks<'a>(
    project: &'a ModuleAnalysis,
    event: &ResolvedDeclaration,
) -> Vec<&'a ResolvedDeclaration> {
    let Some(resolved) = project.resolved_module_graph.as_deref() else {
        return Vec::new();
    };
    let Some(binding) = event.event_binding.as_ref() else {
        return Vec::new();
    };
    let scope_id = match &binding.scope {
        ResolvedEventScope::Plot(id) | ResolvedEventScope::Subplots { plot: id, .. } => id,
    };
    let Some(scope) = resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
        .find_map(|root| find_declaration(root, scope_id))
    else {
        return Vec::new();
    };
    let mut marks = Vec::new();
    if binding.targets.is_empty() {
        collect_primitive_marks(scope, &mut marks);
    } else {
        for target in &binding.targets {
            collect_target_primitive_marks(scope, target, &mut marks);
        }
    }
    marks.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
    marks.dedup_by(|left, right| left.id == right.id);
    marks
}

fn event_channels(
    project: Option<&ModuleAnalysis>,
    event: &ResolvedDeclaration,
    registry: &NativeSchemaSnapshot,
    item_only: bool,
) -> Vec<ContextualChannel> {
    let Some(project) = project else {
        return Vec::new();
    };
    let mut channels = BTreeMap::<String, ContextualChannel>::new();
    for mark in event_target_marks(project, event) {
        for mut channel in mark_channels(Some(project), mark, registry, item_only) {
            if !item_only {
                // Event coordinate and domain accessors expose the fixed
                // float64 event boundary, independent of the authored
                // channel expression's physical type.
                channel.data_type = Some("float64".to_owned());
                channel.nullable = Some(true);
            }
            channels
                .entry(channel.name.to_ascii_lowercase())
                .and_modify(|existing| {
                    if existing.data_type != channel.data_type {
                        existing.data_type = None;
                        existing.nullable = None;
                        existing.docs = "Target-dependent channel type.".to_owned();
                    } else if existing.nullable != channel.nullable {
                        existing.nullable = Some(true);
                    }
                })
                .or_insert(channel);
        }
    }
    channels.into_values().collect()
}

fn mark_input_columns(
    project: Option<&ModuleAnalysis>,
    mark: &ResolvedDeclaration,
) -> Vec<ColumnMetadata> {
    let Some(project) = project else {
        return Vec::new();
    };
    project
        .datasets
        .iter()
        .filter(|(_, dataset)| {
            dataset.provenance.stage_kind == DatasetStageKind::MarkInput
                && dataset.provenance.declaration_span == mark.span
        })
        .map(|(_, dataset)| dataset)
        .last()
        .map(|dataset| {
            dataset
                .columns
                .iter()
                .map(|column| ColumnMetadata {
                    name: column.name.clone(),
                    qualifier: Some("item.data".to_owned()),
                    data_type: column.data_type.clone(),
                    nullable: column.nullable,
                    stage: "item data".to_owned(),
                    lineage: None,
                    detail: Some(format!(
                        "{} · {} · logical source item row",
                        column.data_type,
                        nullability(column.nullable)
                    )),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn event_datum_relation(
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
) -> Option<RelationMetadata> {
    let project = project?;
    let event = enclosing_event(Some(project), origin, cursor)?;
    let marks = event_target_marks(project, event);
    if marks.is_empty() {
        return None;
    }

    let schemas = marks
        .iter()
        .filter_map(|mark| {
            project
                .datasets
                .iter()
                .filter(|(_, dataset)| {
                    dataset.provenance.stage_kind == DatasetStageKind::MarkInput
                        && dataset.provenance.declaration_span == mark.span
                })
                .map(|(_, dataset)| dataset)
                .last()
                .map(|dataset| (*mark, dataset))
        })
        .collect::<Vec<_>>();
    if schemas.is_empty() {
        return None;
    }

    let mut fields = BTreeMap::<String, Vec<(&ResolvedDeclaration, &ColumnMetadata)>>::new();
    let schema_columns = schemas
        .iter()
        .map(|(_, dataset)| {
            dataset
                .columns
                .iter()
                .map(|column| ColumnMetadata {
                    name: column.name.clone(),
                    qualifier: Some("datum".to_owned()),
                    data_type: column.data_type.clone(),
                    nullable: column.nullable,
                    stage: "event datum".to_owned(),
                    lineage: None,
                    detail: None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for ((mark, _), columns) in schemas.iter().zip(&schema_columns) {
        for column in columns {
            fields
                .entry(column.name.clone())
                .or_default()
                .push((*mark, column));
        }
    }

    let total = schemas.len();
    let columns = fields
        .into_iter()
        .map(|(name, candidates)| {
            let first_type = candidates[0].1.data_type.clone();
            let consistent = candidates
                .iter()
                .all(|(_, column)| column.data_type == first_type);
            let coverage = candidates.len();
            let mut types = candidates
                .iter()
                .map(|(_, column)| column.data_type.to_string())
                .collect::<Vec<_>>();
            types.sort();
            types.dedup();
            let targets = candidates
                .iter()
                .map(|(mark, _)| {
                    mark.public_path
                        .as_deref()
                        .or(mark.name.as_deref())
                        .or(mark.kind.as_deref())
                        .unwrap_or("anonymous mark")
                })
                .collect::<Vec<_>>()
                .join(", ");
            let nullable =
                coverage < total || candidates.iter().any(|(_, column)| column.nullable);
            ColumnMetadata {
                name,
                qualifier: Some("datum".to_owned()),
                data_type: if consistent {
                    first_type
                } else {
                    DataType::Null
                },
                nullable,
                stage: "event datum".to_owned(),
                lineage: None,
                detail: Some(if consistent {
                    format!(
                        "{} · {} · logical hit row · {coverage}/{total} targets ({targets})",
                        candidates[0].1.data_type,
                        nullability(nullable)
                    )
                } else {
                    format!(
                        "target-dependent ({}) · {} · logical hit row · {coverage}/{total} targets ({targets})",
                        types.join(" | "),
                        nullability(nullable)
                    )
                }),
            }
        })
        .collect();

    Some(RelationMetadata {
        path: vec!["datum".to_owned()],
        alias: Some("datum".to_owned()),
        columns,
        detail: "logical pre-scale row of the hit mark instance".to_owned(),
        scope_depth: 0,
    })
}

fn find_enclosing_event<'a>(
    declaration: &'a ResolvedDeclaration,
    project: &avenger_lang_core::ResolvedModuleGraph,
    origin: &SourceOrigin,
    cursor: usize,
) -> Option<&'a ResolvedDeclaration> {
    let authored = project.expansion_source_map.authored_span(declaration.span);
    let source = project.sources.get(authored.source)?;
    if !same_origin(&source.origin, origin)
        || authored.range.start > cursor
        || cursor > authored.range.end
    {
        return None;
    }
    declaration
        .children
        .iter()
        .filter_map(|child| find_enclosing_event(child, project, origin, cursor))
        .min_by_key(|event| event.span.range.len())
        .or_else(|| declaration.event_binding.as_ref().map(|_| declaration))
}

fn find_declaration<'a>(
    declaration: &'a ResolvedDeclaration,
    id: &DeclarationId,
) -> Option<&'a ResolvedDeclaration> {
    if &declaration.id == id {
        return Some(declaration);
    }
    declaration
        .children
        .iter()
        .find_map(|child| find_declaration(child, id))
}

fn collect_primitive_marks<'a>(
    declaration: &'a ResolvedDeclaration,
    output: &mut Vec<&'a ResolvedDeclaration>,
) {
    if declaration.keyword == "mark" && declaration.kind.as_deref() != Some("group") {
        output.push(declaration);
        return;
    }
    for child in &declaration.children {
        collect_primitive_marks(child, output);
    }
}

fn collect_target_primitive_marks<'a>(
    declaration: &'a ResolvedDeclaration,
    target: &ResolvedTarget,
    output: &mut Vec<&'a ResolvedDeclaration>,
) {
    let matches = match (target, declaration.runtime_target.as_ref()) {
        (ResolvedTarget::Mark(expected), Some(ResolvedTarget::Mark(actual))) => expected == actual,
        (
            ResolvedTarget::Part {
                declaration: expected,
                ..
            },
            _,
        ) => expected == &declaration.id,
        _ => false,
    };
    if matches {
        collect_primitive_marks(declaration, output);
        return;
    }
    for child in &declaration.children {
        collect_target_primitive_marks(child, target, output);
    }
}

fn build_query_scope(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    active_depth: usize,
    catalog: &[RelationMetadata],
    project: Option<&ModuleAnalysis>,
    context: Option<&DatasetContext>,
) -> QueryScope {
    let mut scope = QueryScope {
        ctes: collect_ctes(tokens, cursor, active_depth, catalog),
        ..QueryScope::default()
    };
    let mut available = catalog.to_vec();
    available.extend(scope.ctes.clone());

    for (index, token) in tokens.iter().enumerate() {
        if token.depth > active_depth || !(token.is_word("from") || token.is_word("join")) {
            continue;
        }
        if let Some(relation) = relation_after(tokens, index + 1, token.depth, &available) {
            scope.relations.push(relation);
        }
    }

    if let Some(project) = project
        && let Some(input) = input_dataset(project, context, cursor)
    {
        let stage = &input.stage;
        scope.relations.push(RelationMetadata {
            path: vec!["input".to_owned()],
            alias: Some("input".to_owned()),
            columns: input
                .columns
                .iter()
                .map(|column| ColumnMetadata {
                    name: column.name.clone(),
                    qualifier: Some("input".to_owned()),
                    data_type: column.data_type.clone(),
                    nullable: column.nullable,
                    stage: format!("{}#{}", stage.dataset.as_str(), stage.ordinal),
                    lineage: None,
                    detail: None,
                })
                .collect(),
            detail: "exact pipeline input".to_owned(),
            scope_depth: active_depth,
        });
    }
    deduplicate_relations(&mut scope.relations);
    scope.projection_aliases = projection_aliases(tokens, active_depth, &scope.relations);
    scope.projection_aliases_visible = projection_aliases_visible(tokens, cursor, active_depth);
    scope
}

/// Replace heuristic projection metadata with DataFusion's qualified output
/// schema whenever the current query is strict-valid and plans against the
/// published schema-only catalog. Recovery remains useful for incomplete
/// queries, but it never overrides a successful planner result.
fn reconcile_query_output_with_datafusion(
    authored: &str,
    catalog: &[RelationMetadata],
    scope: &mut QueryScope,
) {
    let Ok(query) = SqlQuery::parse(authored) else {
        return;
    };
    let context = SessionContext::new();
    for relation in catalog.iter().chain(scope.relations.iter()) {
        let _ = register_schema_only_relation(&context, relation);
    }
    for binding in query.bindings() {
        if binding.kind != avenger_lang_core::ast::BindingKind::Store {
            continue;
        }
        let Some(name) = binding
            .path
            .first()
            .map(|name| format!("${}", name.as_str()))
        else {
            continue;
        };
        let Some(relation) = catalog
            .iter()
            .chain(scope.relations.iter())
            .find(|relation| {
                relation
                    .path
                    .last()
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(&name))
            })
        else {
            continue;
        };
        let synthetic = RelationMetadata {
            path: vec![binding.synthetic_identifier.clone()],
            ..relation.clone()
        };
        let _ = register_schema_only_relation(&context, &synthetic);
    }

    let canonical = query.ast().to_string();
    let Ok(plan) = futures::executor::block_on(context.state().create_logical_plan(&canonical))
    else {
        return;
    };
    LOGICAL_QUERY_PLANS.fetch_add(1, Ordering::Relaxed);
    scope.projection_aliases = plan
        .schema()
        .iter()
        .map(|(qualifier, field)| ColumnMetadata {
            name: field.name().clone(),
            qualifier: qualifier.map(ToString::to_string),
            data_type: field.data_type().clone(),
            nullable: field.is_nullable(),
            stage: "DataFusion query output".to_owned(),
            lineage: scope
                .projection_aliases
                .iter()
                .find(|column| column.name.eq_ignore_ascii_case(field.name()))
                .and_then(|column| column.lineage.clone()),
            detail: None,
        })
        .collect();
}

fn register_schema_only_relation(
    context: &SessionContext,
    relation: &RelationMetadata,
) -> datafusion::common::Result<()> {
    use arrow::datatypes::{Field, Schema};

    if relation.path.is_empty() || relation.path.len() > 3 || relation.columns.is_empty() {
        return Ok(());
    }
    let schema = Arc::new(Schema::new(
        relation
            .columns
            .iter()
            .map(|column| Field::new(&column.name, column.data_type.clone(), column.nullable))
            .collect::<Vec<_>>(),
    ));
    let provider: Arc<dyn TableProvider> = Arc::new(EmptyTable::new(schema));
    match relation.path.as_slice() {
        [table] => {
            if context.table_exist(table)? {
                return Ok(());
            }
            context.register_table(table, provider)?;
        }
        [schema, table] => {
            let catalog_name = context
                .state()
                .config_options()
                .catalog
                .default_catalog
                .clone();
            register_in_schema(context, &catalog_name, schema, table, provider)?;
        }
        [catalog, schema, table] => {
            register_in_schema(context, catalog, schema, table, provider)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn register_in_schema(
    context: &SessionContext,
    catalog_name: &str,
    schema_name: &str,
    table_name: &str,
    provider: Arc<dyn TableProvider>,
) -> datafusion::common::Result<()> {
    let catalog: Arc<dyn CatalogProvider> = context.catalog(catalog_name).unwrap_or_else(|| {
        let catalog: Arc<dyn CatalogProvider> = Arc::new(MemoryCatalogProvider::new());
        context.register_catalog(catalog_name, Arc::clone(&catalog));
        catalog
    });
    let schema: Arc<dyn SchemaProvider> = if let Some(schema) = catalog.schema(schema_name) {
        schema
    } else {
        let schema: Arc<dyn SchemaProvider> = Arc::new(MemorySchemaProvider::new());
        catalog.register_schema(schema_name, Arc::clone(&schema))?;
        schema
    };
    if !schema.table_exist(table_name) {
        schema.register_table(table_name.to_owned(), provider)?;
    }
    Ok(())
}

fn projection_aliases_visible(tokens: &[SqlToken<'_>], cursor: usize, depth: usize) -> bool {
    tokens
        .iter()
        .filter(|token| token.span.range.end <= cursor && token.depth == depth)
        .rev()
        .find(|token| is_clause_token(token))
        .and_then(SqlToken::word)
        .is_some_and(|word| {
            ["order", "group", "having", "qualify"]
                .iter()
                .any(|candidate| word.eq_ignore_ascii_case(candidate))
        })
}

fn collect_ctes(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    depth: usize,
    catalog: &[RelationMetadata],
) -> Vec<RelationMetadata> {
    let mut output = Vec::new();
    let Some(with_index) = tokens
        .iter()
        .position(|token| token.depth <= depth && token.is_word("with"))
    else {
        return output;
    };
    let mut index = with_index + 1;
    while index < tokens.len() {
        let Some(name) = tokens.get(index).and_then(SqlToken::word) else {
            break;
        };
        if tokens[index].span.range.start > cursor {
            break;
        }
        let Some(as_index) = (index + 1..tokens.len()).find(|candidate| {
            tokens[*candidate].depth == tokens[index].depth && tokens[*candidate].is_word("as")
        }) else {
            break;
        };
        let Some(open) = tokens
            .get(as_index + 1)
            .filter(|token| matches!(token.token, Some(Token::LParen)))
        else {
            break;
        };
        let close = matching_close(tokens, as_index + 1).unwrap_or(tokens.len());
        let mut relations = catalog.to_vec();
        relations.extend(output.clone());
        let columns = projection_aliases(
            &tokens[as_index + 2..close.min(tokens.len())],
            open.depth + 1,
            &relations,
        );
        output.push(RelationMetadata {
            path: vec![name.to_owned()],
            alias: None,
            columns,
            detail: "common table expression".to_owned(),
            scope_depth: tokens[index].depth,
        });
        index = close.saturating_add(1);
        if !tokens.get(index).is_some_and(SqlToken::is_comma) {
            break;
        }
        index += 1;
    }
    output
}

fn matching_close(tokens: &[SqlToken<'_>], open: usize) -> Option<usize> {
    let depth = tokens.get(open)?.depth;
    tokens
        .iter()
        .enumerate()
        .skip(open + 1)
        .find(|(_, token)| token.depth == depth && matches!(token.token, Some(Token::RParen)))
        .map(|(index, _)| index)
}

fn relation_after(
    tokens: &[SqlToken<'_>],
    start: usize,
    depth: usize,
    available: &[RelationMetadata],
) -> Option<RelationMetadata> {
    let first = tokens.get(start)?;
    if matches!(first.token, Some(Token::LParen)) {
        let close = matching_close(tokens, start).unwrap_or(tokens.len());
        let alias = alias_after(tokens, close, depth);
        let columns = projection_aliases(
            &tokens[start + 1..close.min(tokens.len())],
            first.depth + 1,
            available,
        );
        return Some(RelationMetadata {
            path: vec![alias.clone().unwrap_or_else(|| "subquery".to_owned())],
            alias,
            columns,
            detail: "derived subquery".to_owned(),
            scope_depth: depth,
        });
    }
    let mut path = Vec::new();
    let mut index = start;
    while let Some(token) = tokens.get(index) {
        if token.depth != depth {
            break;
        }
        if let Some(word) = token.word() {
            if is_clause_word(word) || word.eq_ignore_ascii_case("as") {
                break;
            }
            path.push(word.to_owned());
            index += 1;
            if tokens.get(index).is_some_and(SqlToken::is_period) {
                index += 1;
                continue;
            }
            break;
        }
        if token.raw.starts_with('$') {
            path.push(token.raw.to_owned());
        }
        break;
    }
    if path.is_empty() {
        return None;
    }
    let alias = alias_after(tokens, index, depth);
    let mut relation = resolve_relation(available, &path).unwrap_or(RelationMetadata {
        path: path.clone(),
        alias: None,
        columns: Vec::new(),
        detail: "unresolved relation".to_owned(),
        scope_depth: depth,
    });
    relation.path = path;
    relation.alias = alias;
    relation.scope_depth = depth;
    Some(relation)
}

fn alias_after(tokens: &[SqlToken<'_>], mut index: usize, depth: usize) -> Option<String> {
    if tokens.get(index).is_some_and(|token| token.is_word("as")) {
        index += 1;
    }
    let token = tokens.get(index)?;
    if token.depth != depth {
        return None;
    }
    token
        .word()
        .filter(|word| !is_clause_word(word))
        .map(str::to_owned)
}

fn resolve_relation(available: &[RelationMetadata], path: &[String]) -> Option<RelationMetadata> {
    available
        .iter()
        .filter(|relation| {
            eq_path(&relation.path, path)
                || relation
                    .path
                    .last()
                    .zip(path.last())
                    .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        })
        .min_by_key(|relation| usize::from(!eq_path(&relation.path, path)))
        .cloned()
}

fn eq_path(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn projection_aliases(
    tokens: &[SqlToken<'_>],
    depth: usize,
    relations: &[RelationMetadata],
) -> Vec<ColumnMetadata> {
    let Some(select) = tokens
        .iter()
        .position(|token| token.depth == depth && token.is_word("select"))
    else {
        return Vec::new();
    };
    let end = tokens
        .iter()
        .enumerate()
        .skip(select + 1)
        .find(|(_, token)| {
            token.depth == depth
                && (token.is_word("from")
                    || token.is_word("where")
                    || token.is_word("group")
                    || token.is_word("order")
                    || token.is_word("union")
                    || token.is_word("limit"))
        })
        .map_or(tokens.len(), |(index, _)| index);
    let mut output = Vec::new();
    let mut item_start = select + 1;
    for item_end in (select + 1..=end).filter(|index| {
        *index == end
            || tokens
                .get(*index)
                .is_some_and(|token| token.depth == depth && token.is_comma())
    }) {
        let item = &tokens[item_start..item_end];
        if item.iter().any(|token| token.raw == "*") {
            if let Some(qualifier) = item
                .windows(2)
                .find(|pair| pair[1].is_period())
                .and_then(|pair| pair[0].word())
            {
                if let Some(relation) = relations
                    .iter()
                    .find(|relation| relation.visible_name().eq_ignore_ascii_case(qualifier))
                {
                    output.extend(relation.columns.clone());
                }
            } else {
                output.extend(
                    relations
                        .iter()
                        .flat_map(|relation| relation.columns.clone()),
                );
            }
        } else if let Some(as_index) = item.iter().rposition(|token| token.is_word("as")) {
            if let Some(alias) = item.get(as_index + 1).and_then(SqlToken::word) {
                output.push(ColumnMetadata {
                    name: alias.to_owned(),
                    qualifier: None,
                    data_type: DataType::Null,
                    nullable: true,
                    stage: "projection".to_owned(),
                    lineage: item.iter().find_map(SqlToken::word).map(str::to_owned),
                    detail: None,
                });
            }
        } else if let Some(name) = item.iter().rev().find_map(SqlToken::word) {
            let source = relations
                .iter()
                .flat_map(|relation| &relation.columns)
                .find(|column| column.name.eq_ignore_ascii_case(name));
            output.push(source.cloned().unwrap_or(ColumnMetadata {
                name: name.to_owned(),
                qualifier: None,
                data_type: DataType::Null,
                nullable: true,
                stage: "projection".to_owned(),
                lineage: None,
                detail: None,
            }));
        }
        item_start = item_end.saturating_add(1);
    }
    deduplicate_columns(&mut output);
    output
}

fn input_dataset<'a>(
    project: &'a ModuleAnalysis,
    context: Option<&DatasetContext>,
    cursor: usize,
) -> Option<&'a AnalyzedDataset> {
    let context = context?;
    let current = project.datasets.get(&context.stage)?;
    if matches!(
        context.stage_kind,
        avenger_lang_compiler::DatasetStageKind::Transform { .. }
    ) && let Some(upstream) = project
        .lineage
        .get(&context.stage)
        .and_then(|lineage| lineage.upstream_stages.last())
    {
        return project.datasets.get(upstream);
    }
    if matches!(
        context.stage_kind,
        avenger_lang_compiler::DatasetStageKind::DatasetSource
    ) {
        return project
            .datasets
            .stages_for(&context.stage.dataset)
            .filter(|dataset| {
                dataset.provenance.stage_span.range.end <= cursor
                    && dataset.provenance.declaration_span.range.start <= cursor
                    && cursor <= dataset.provenance.declaration_span.range.end
            })
            .max_by_key(|dataset| dataset.stage.ordinal)
            .or(Some(current));
    }
    Some(current)
}

fn qualifier_before(text: &str, island: SourceSpan, start: usize) -> Option<String> {
    if start <= island.range.start || !text[..start].ends_with('.') {
        return None;
    }
    let mut position = start - 1;
    let mut parts = Vec::new();
    loop {
        let end = position;
        while position > island.range.start {
            let character = text[..position].chars().next_back().unwrap();
            if character == '_' || character == '$' || character.is_alphanumeric() {
                position -= character.len_utf8();
            } else {
                break;
            }
        }
        if position == end {
            break;
        }
        parts.push(text[position..end].to_owned());
        if position <= island.range.start || !text[..position].ends_with('.') {
            break;
        }
        position -= 1;
    }
    parts.reverse();
    (!parts.is_empty()).then(|| parts.join("."))
}

fn quoted_qualifier_before(
    text: &str,
    island: SourceSpan,
    start: usize,
) -> Option<(String, usize)> {
    let quote = start.checked_sub(1)?;
    if quote < island.range.start || text.as_bytes().get(quote) != Some(&b'"') {
        return None;
    }
    let period = quote.checked_sub(1)?;
    if period < island.range.start || text.as_bytes().get(period) != Some(&b'.') {
        return None;
    }
    let mut position = period;
    while position > island.range.start {
        let character = text[..position].chars().next_back()?;
        if character == '_' || character == '$' || character.is_alphanumeric() {
            position -= character.len_utf8();
        } else {
            break;
        }
    }
    (position < period).then(|| (text[position..period].to_owned(), quote))
}

#[derive(Clone, Debug)]
struct ContextualChannel {
    name: String,
    data_type: Option<String>,
    nullable: Option<bool>,
    docs: String,
}

#[allow(clippy::too_many_arguments)]
fn complete_contextual_qualifier(
    qualifier: &str,
    prefix: &str,
    replacement: SourceSpan,
    syntax: &SyntaxAnalysis,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    registry: &NativeSchemaSnapshot,
    output: &mut Vec<CompletionItem>,
) -> Option<bool> {
    let parts = qualifier.split('.').collect::<Vec<_>>();
    let root = parts.first()?.to_ascii_lowercase();
    let event = enclosing_event(project, origin, cursor).or_else(|| {
        syntax_contains_declaration(syntax, cursor, "on")
            .then(|| nearest_declaration(project, origin, cursor, "on"))
            .flatten()
    });
    let mut path = declaration_path_at(project, origin, cursor);
    if !path.iter().any(|declaration| declaration.keyword == "mark")
        && syntax_contains_declaration(syntax, cursor, "mark")
        && let Some(mark) = nearest_declaration(project, origin, cursor, "mark")
    {
        path.push(mark);
    }
    if !path.iter().any(|declaration| declaration.keyword == "view")
        && syntax_contains_declaration(syntax, cursor, "view")
        && let Some(view) = nearest_declaration(project, origin, cursor, "view")
    {
        path.push(view);
    }
    let mark = path
        .iter()
        .rev()
        .copied()
        .find(|declaration| declaration.keyword == "mark");
    let item_frame = path
        .iter()
        .rev()
        .any(|declaration| matches!(declaration.keyword.as_str(), "adjust" | "derive"))
        || syntax_contains_declaration(syntax, cursor, "adjust")
        || syntax_contains_declaration(syntax, cursor, "derive");
    let item_mark = item_frame.then_some(mark).flatten();
    let between = event.is_some_and(|event| event.properties.contains_key("between"));
    let legend = event
        .and_then(|event| event.event_binding.as_ref())
        .is_some_and(|binding| matches!(binding.surface, ResolvedEventSurface::Legend { .. }));
    let handled = match (root.as_str(), parts.as_slice()) {
        ("datum", ["datum"]) if event.is_some() => {
            let relation = event_datum_relation(project, origin, cursor)?;
            complete_contextual_columns(prefix, replacement, &relation.columns, true, output);
            true
        }
        ("channel", ["channel"]) if mark.is_some() => {
            complete_contextual_channels(
                prefix,
                replacement,
                &mark_channels(project, mark.unwrap(), registry, false),
                output,
            );
            true
        }
        ("event", ["event"]) if event.is_some() => {
            let mut members = vec![
                ("coord", "Current coordinates by channel."),
                ("domain", "Event-time scale domains by channel."),
                ("facet", "One-based facet-path components."),
            ];
            if between {
                members.extend([
                    ("start", "Gesture-start event context."),
                    ("path", "Accumulated between-interaction path."),
                ]);
            }
            if legend {
                members.push(("legend", "Continuous legend-surface context."));
            }
            complete_fixed_members(prefix, replacement, &members, output);
            true
        }
        ("event", ["event", member]) if member.eq_ignore_ascii_case("coord") && event.is_some() => {
            complete_contextual_channels(
                prefix,
                replacement,
                &event_channels(project, event.unwrap(), registry, false),
                output,
            );
            true
        }
        ("event", ["event", member])
            if member.eq_ignore_ascii_case("domain") && event.is_some() =>
        {
            complete_contextual_channels(
                prefix,
                replacement,
                &event_channels(project, event.unwrap(), registry, false),
                output,
            );
            true
        }
        ("event", ["event", start]) if start.eq_ignore_ascii_case("start") && between => {
            complete_fixed_members(
                prefix,
                replacement,
                &[("coord", "Gesture-start coordinates by channel.")],
                output,
            );
            true
        }
        ("event", ["event", start, coord])
            if start.eq_ignore_ascii_case("start")
                && coord.eq_ignore_ascii_case("coord")
                && between =>
        {
            complete_contextual_channels(
                prefix,
                replacement,
                &event_channels(project, event.unwrap(), registry, false),
                output,
            );
            true
        }
        ("event", ["event", domain, channel])
            if domain.eq_ignore_ascii_case("domain") && event.is_some() =>
        {
            let valid = event_channels(project, event.unwrap(), registry, false)
                .iter()
                .any(|candidate| candidate.name.eq_ignore_ascii_case(channel));
            if valid {
                let start = fixed_contextual_signature_detail("event.domain.<channel>.start")
                    .unwrap_or_else(|| "Start of the event-time scale domain.".to_owned());
                let end = fixed_contextual_signature_detail("event.domain.<channel>.end")
                    .unwrap_or_else(|| "End of the event-time scale domain.".to_owned());
                complete_fixed_members(
                    prefix,
                    replacement,
                    &[("start", &start), ("end", &end)],
                    output,
                );
            }
            valid
        }
        ("event", ["event", legend_member])
            if legend_member.eq_ignore_ascii_case("legend") && legend =>
        {
            let value = fixed_contextual_signature_detail("event.legend.value")
                .unwrap_or_else(|| "Continuous legend-surface value.".to_owned());
            complete_fixed_members(prefix, replacement, &[("value", &value)], output);
            true
        }
        ("item", ["item"]) if item_mark.is_some() => {
            complete_fixed_members(
                prefix,
                replacement,
                &[
                    ("channel", "Evaluated source item channels."),
                    ("data", "Logical source-row fields."),
                    ("bbox", "Evaluated source-item bounding box."),
                ],
                output,
            );
            true
        }
        ("item", ["item", member])
            if member.eq_ignore_ascii_case("channel") && item_mark.is_some() =>
        {
            complete_contextual_channels(
                prefix,
                replacement,
                &mark_channels(project, item_mark.unwrap(), registry, true),
                output,
            );
            true
        }
        ("item", ["item", member])
            if member.eq_ignore_ascii_case("data") && item_mark.is_some() =>
        {
            let columns = mark_input_columns(project, item_mark.unwrap());
            complete_contextual_columns(prefix, replacement, &columns, true, output);
            !columns.is_empty()
        }
        ("item", ["item", member])
            if member.eq_ignore_ascii_case("bbox") && item_mark.is_some() =>
        {
            let edge = fixed_contextual_signature_detail("item.bbox.<edge>")
                .unwrap_or_else(|| "Evaluated source-item bounding-box edge.".to_owned());
            complete_fixed_members(
                prefix,
                replacement,
                &[
                    ("top", &edge),
                    ("right", &edge),
                    ("bottom", &edge),
                    ("left", &edge),
                ],
                output,
            );
            true
        }
        _ => {
            let view = path.iter().rev().copied().find(|declaration| {
                declaration.keyword == "view"
                    && declaration
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(parts[0]))
            });
            match (view, parts.as_slice()) {
                (Some(_), [_]) => {
                    complete_fixed_members(
                        prefix,
                        replacement,
                        &[("x", "Inline-view x axis."), ("y", "Inline-view y axis.")],
                        output,
                    );
                    true
                }
                (Some(_), [_, axis])
                    if axis.eq_ignore_ascii_case("x") || axis.eq_ignore_ascii_case("y") =>
                {
                    let pixels = fixed_contextual_signature_detail(&format!(
                        "<view>.{}.pixels",
                        axis.to_ascii_lowercase()
                    ))
                    .unwrap_or_else(|| "Inline-view pixel count.".to_owned());
                    complete_fixed_members(
                        prefix,
                        replacement,
                        &[("domain", "Inline-view scale domain."), ("pixels", &pixels)],
                        output,
                    );
                    true
                }
                (Some(_), [_, axis, domain])
                    if (axis.eq_ignore_ascii_case("x") || axis.eq_ignore_ascii_case("y"))
                        && domain.eq_ignore_ascii_case("domain") =>
                {
                    let start = fixed_contextual_signature_detail(&format!(
                        "<view>.{}.domain.start",
                        axis.to_ascii_lowercase()
                    ))
                    .unwrap_or_else(|| "Inline-view domain start.".to_owned());
                    let end = fixed_contextual_signature_detail(&format!(
                        "<view>.{}.domain.end",
                        axis.to_ascii_lowercase()
                    ))
                    .unwrap_or_else(|| "Inline-view domain end.".to_owned());
                    complete_fixed_members(
                        prefix,
                        replacement,
                        &[("start", &start), ("end", &end)],
                        output,
                    );
                    true
                }
                _ => return None,
            }
        }
    };
    Some(handled)
}

#[allow(clippy::too_many_arguments)]
fn complete_contextual_roots(
    prefix: &str,
    replacement: SourceSpan,
    syntax: &SyntaxAnalysis,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    registry: &NativeSchemaSnapshot,
    output: &mut Vec<CompletionItem>,
) {
    let mut path = declaration_path_at(project, origin, cursor);
    if !path.iter().any(|declaration| declaration.keyword == "mark")
        && syntax_contains_declaration(syntax, cursor, "mark")
        && let Some(mark) = nearest_declaration(project, origin, cursor, "mark")
    {
        path.push(mark);
    }
    if !path.iter().any(|declaration| declaration.keyword == "view")
        && syntax_contains_declaration(syntax, cursor, "view")
        && let Some(view) = nearest_declaration(project, origin, cursor, "view")
    {
        path.push(view);
    }
    let event = enclosing_event(project, origin, cursor).or_else(|| {
        syntax_contains_declaration(syntax, cursor, "on")
            .then(|| nearest_declaration(project, origin, cursor, "on"))
            .flatten()
    });
    let mark = path
        .iter()
        .rev()
        .copied()
        .find(|declaration| declaration.keyword == "mark");
    if mark.is_some() && !mark_channels(project, mark.unwrap(), registry, false).is_empty() {
        complete_fixed_members(
            prefix,
            replacement,
            &[("channel", "Channels on the current mark.")],
            output,
        );
    }
    if event.is_some() {
        complete_fixed_members(
            prefix,
            replacement,
            &[
                ("event", "Current routed event context."),
                ("datum", "Logical pre-scale row of the hit mark."),
            ],
            output,
        );
    }
    if mark.is_some()
        && (path
            .iter()
            .any(|declaration| matches!(declaration.keyword.as_str(), "adjust" | "derive"))
            || syntax_contains_declaration(syntax, cursor, "adjust")
            || syntax_contains_declaration(syntax, cursor, "derive"))
    {
        complete_fixed_members(
            prefix,
            replacement,
            &[("item", "Source item-frame context.")],
            output,
        );
    }
    for view in path
        .iter()
        .filter(|declaration| declaration.keyword == "view")
    {
        if let Some(name) = view.name.as_deref() {
            complete_fixed_members(
                prefix,
                replacement,
                &[(name, "Lexically scoped inline view.")],
                output,
            );
        }
    }
}

fn complete_fixed_members(
    prefix: &str,
    replacement: SourceSpan,
    members: &[(&str, &str)],
    output: &mut Vec<CompletionItem>,
) {
    for (name, detail) in members {
        if candidate_matches(name, prefix) {
            output.push(candidate(
                name,
                name,
                replacement,
                CompletionKind::Property,
                Some((*detail).to_owned()),
                CompletionOrigin::AuthoringSchema,
                "00",
            ));
        }
    }
}

fn complete_contextual_channels(
    prefix: &str,
    replacement: SourceSpan,
    channels: &[ContextualChannel],
    output: &mut Vec<CompletionItem>,
) {
    for channel in channels {
        if candidate_matches(&channel.name, prefix) {
            let detail = channel.data_type.as_ref().map_or_else(
                || channel.docs.clone(),
                |data_type| {
                    format!(
                        "{data_type} · {} · {}",
                        channel
                            .nullable
                            .map(nullability)
                            .unwrap_or("nullability unavailable"),
                        channel.docs
                    )
                },
            );
            output.push(candidate(
                &channel.name,
                &channel.name,
                replacement,
                CompletionKind::Property,
                Some(detail),
                CompletionOrigin::AuthoringSchema,
                "00",
            ));
        }
    }
}

fn complete_contextual_columns(
    prefix: &str,
    replacement: SourceSpan,
    columns: &[ColumnMetadata],
    quoted: bool,
    output: &mut Vec<CompletionItem>,
) {
    let typed = prefix.trim_start_matches('"').replace("\"\"", "\"");
    for column in columns {
        if candidate_matches(&column.name, &typed) {
            let insert = if quoted {
                format!("\"{}\"", column.name.replace('"', "\"\""))
            } else {
                column.name.clone()
            };
            output.push(column_candidate(column, &insert, replacement, "00"));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_qualifier(
    qualifier: &str,
    prefix: &str,
    replacement: SourceSpan,
    roles: &BTreeSet<SqlExpectedRole>,
    scope: &QueryScope,
    catalog: &[RelationMetadata],
    document: Option<&DocumentSemanticIndex>,
    project: Option<&ModuleAnalysis>,
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) -> bool {
    let parts = qualifier.split('.').collect::<Vec<_>>();
    if let Some(relation) = scope
        .relations
        .iter()
        .chain(scope.ctes.iter())
        .find(|relation| relation.visible_name().eq_ignore_ascii_case(parts[0]))
    {
        if parts.len() == 1 && !relation.columns.is_empty() {
            complete_column_members(prefix, replacement, relation, output);
            return true;
        }
        if let Some(fields) = resolve_struct_chain(&relation.columns, &parts[1..]) {
            complete_struct_fields(prefix, replacement, fields, qualifier, output);
            return true;
        }
    }
    if let Some(column) = scope
        .relations
        .iter()
        .flat_map(|relation| &relation.columns)
        .find(|column| column.name.eq_ignore_ascii_case(parts[0]))
        && let Some(fields) = struct_fields(&column.data_type)
    {
        complete_struct_fields(prefix, replacement, fields, qualifier, output);
        return true;
    }
    if let Some(fields) = binding_struct_fields(qualifier, document, project, cursor) {
        complete_physical_struct_fields(prefix, replacement, fields, qualifier, output);
        return true;
    }
    if roles.contains(&SqlExpectedRole::Relation)
        || roles.contains(&SqlExpectedRole::Catalog)
        || roles.contains(&SqlExpectedRole::Schema)
    {
        let prefix_path = qualifier.split('.').collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for relation in catalog {
            if relation.path.len() <= prefix_path.len()
                || !relation
                    .path
                    .iter()
                    .zip(&prefix_path)
                    .all(|(left, right)| left.eq_ignore_ascii_case(right))
            {
                continue;
            }
            let member = &relation.path[prefix_path.len()];
            if seen.insert(member.to_ascii_lowercase()) && candidate_matches(member, prefix) {
                let kind = if relation.path.len() == prefix_path.len() + 1 {
                    CompletionKind::Table
                } else if prefix_path.is_empty() {
                    CompletionKind::Catalog
                } else {
                    CompletionKind::Schema
                };
                output.push(candidate(
                    member,
                    member,
                    replacement,
                    kind,
                    Some(relation.detail.clone()),
                    CompletionOrigin::Catalog,
                    "10",
                ));
            }
        }
        return !seen.is_empty();
    }
    false
}

fn resolve_struct_chain<'a>(
    columns: &'a [ColumnMetadata],
    parts: &[&str],
) -> Option<&'a arrow::datatypes::Fields> {
    let first = parts.first()?;
    let mut data_type = &columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(first))?
        .data_type;
    for part in &parts[1..] {
        let fields = struct_fields(data_type)?;
        data_type = fields
            .iter()
            .find(|field| field.name().eq_ignore_ascii_case(part))?
            .data_type();
    }
    struct_fields(data_type)
}

fn struct_fields(data_type: &DataType) -> Option<&arrow::datatypes::Fields> {
    match data_type {
        DataType::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn complete_struct_fields(
    prefix: &str,
    replacement: SourceSpan,
    fields: &arrow::datatypes::Fields,
    qualifier: &str,
    output: &mut Vec<CompletionItem>,
) {
    for field in fields {
        if candidate_matches(field.name(), prefix) {
            output.push(candidate(
                field.name(),
                field.name(),
                replacement,
                CompletionKind::Field,
                Some(format!(
                    "{} · {} · struct field of {qualifier}",
                    field.data_type(),
                    nullability(field.is_nullable())
                )),
                CompletionOrigin::DatasetSchema,
                "00",
            ));
        }
    }
}

fn complete_column_members(
    prefix: &str,
    replacement: SourceSpan,
    relation: &RelationMetadata,
    output: &mut Vec<CompletionItem>,
) {
    let quoted_members = relation.visible_name().eq_ignore_ascii_case("datum")
        && relation.detail.starts_with("logical pre-scale");
    let typed = if quoted_members {
        prefix.trim_start_matches('"').replace("\"\"", "\"")
    } else {
        prefix.to_owned()
    };
    for column in &relation.columns {
        if candidate_matches(&column.name, &typed) {
            let insert = if quoted_members {
                format!("\"{}\"", column.name.replace('"', "\"\""))
            } else {
                column.name.clone()
            };
            output.push(column_candidate(column, &insert, replacement, "00"));
        }
    }
}

fn complete_relations(
    prefix: &str,
    replacement: SourceSpan,
    scope: &QueryScope,
    catalog: &[RelationMetadata],
    output: &mut Vec<CompletionItem>,
) {
    for relation in scope.ctes.iter().chain(catalog.iter()) {
        let label = relation.path.join(".");
        if candidate_matches(&label, prefix) {
            output.push(candidate(
                &label,
                &label,
                replacement,
                CompletionKind::Table,
                Some(relation.detail.clone()),
                if scope
                    .ctes
                    .iter()
                    .any(|cte| eq_path(&cte.path, &relation.path))
                {
                    CompletionOrigin::QueryScope
                } else {
                    CompletionOrigin::Catalog
                },
                if relation.path.len() == 1 { "00" } else { "10" },
            ));
        }
    }
}

fn complete_columns(
    prefix: &str,
    replacement: SourceSpan,
    scope: &QueryScope,
    output: &mut Vec<CompletionItem>,
) {
    let mut counts = BTreeMap::<String, usize>::new();
    for column in scope
        .relations
        .iter()
        .flat_map(|relation| &relation.columns)
    {
        *counts.entry(column.name.to_ascii_lowercase()).or_default() += 1;
    }
    for relation in &scope.relations {
        if relation.visible_name().eq_ignore_ascii_case("datum")
            && relation.detail.starts_with("logical pre-scale")
        {
            continue;
        }
        for column in &relation.columns {
            let ambiguous = counts
                .get(&column.name.to_ascii_lowercase())
                .copied()
                .unwrap_or_default()
                > 1;
            let insert = if ambiguous && !relation.visible_name().is_empty() {
                format!("{}.{}", relation.visible_name(), column.name)
            } else {
                column.name.clone()
            };
            if candidate_matches(&column.name, prefix) || candidate_matches(&insert, prefix) {
                output.push(column_candidate(
                    column,
                    &insert,
                    replacement,
                    if ambiguous { "10" } else { "00" },
                ));
            }
        }
    }
    if scope.projection_aliases_visible {
        for alias in &scope.projection_aliases {
            if candidate_matches(&alias.name, prefix) {
                output.push(column_candidate(alias, &alias.name, replacement, "10"));
            }
        }
    }
}

fn table_binding_relations(
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    project: Option<&ModuleAnalysis>,
) -> Vec<RelationMetadata> {
    let Some(document) = document else {
        return Vec::new();
    };
    let Some(resolved) = project.and_then(|project| project.resolved_module_graph.as_deref())
    else {
        return Vec::new();
    };
    document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.value_kind == IndexedValueKind::Table
                && symbol.selection_span.range.start < cursor
                && symbol_visible(document, symbol, cursor)
        })
        .filter_map(|symbol| {
            let store = resolved
                .stores
                .values()
                .find(|store| store.source_name == symbol.name)?;
            Some(RelationMetadata {
                path: vec![format!("${}", symbol.name)],
                alias: None,
                columns: store
                    .fields
                    .iter()
                    .map(|field| ColumnMetadata {
                        name: field.name.clone(),
                        qualifier: Some(format!("${}", symbol.name)),
                        data_type: physical_type_to_arrow(&field.data_type),
                        nullable: field.nullable,
                        stage: format!("store:${}", symbol.name),
                        lineage: None,
                        detail: None,
                    })
                    .collect(),
                detail: "table-valued store".to_owned(),
                scope_depth: 0,
            })
        })
        .collect()
}

fn binding_struct_fields<'a>(
    qualifier: &str,
    document: Option<&DocumentSemanticIndex>,
    project: Option<&'a ModuleAnalysis>,
    cursor: usize,
) -> Option<&'a [avenger_lang_core::PhysicalField]> {
    let mut parts = qualifier.trim_start_matches('$').split('.');
    let name = parts.next()?;
    let document = document?;
    if !document.symbols.iter().any(|symbol| {
        symbol.value_kind == IndexedValueKind::Scalar
            && symbol.name.eq_ignore_ascii_case(name)
            && symbol.selection_span.range.start < cursor
            && symbol_visible(document, symbol, cursor)
    }) {
        return None;
    }
    let resolved = project?.resolved_module_graph.as_deref()?;
    let param = resolved
        .params
        .values()
        .find(|param| param.source_name.eq_ignore_ascii_case(name))?;
    let mut data_type = &param.data_type;
    for part in parts {
        let PhysicalType::Struct(fields) = data_type else {
            return None;
        };
        data_type = &fields
            .iter()
            .find(|field| field.name.eq_ignore_ascii_case(part))?
            .data_type;
    }
    match data_type {
        PhysicalType::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn complete_physical_struct_fields(
    prefix: &str,
    replacement: SourceSpan,
    fields: &[avenger_lang_core::PhysicalField],
    qualifier: &str,
    output: &mut Vec<CompletionItem>,
) {
    for field in fields {
        if candidate_matches(&field.name, prefix) {
            output.push(candidate(
                &field.name,
                &field.name,
                replacement,
                CompletionKind::Field,
                Some(format!(
                    "{} · {} · struct field of {qualifier}",
                    field.data_type,
                    nullability(field.nullable)
                )),
                CompletionOrigin::LexicalScope,
                "00",
            ));
        }
    }
}

fn symbol_visible(
    document: &DocumentSemanticIndex,
    candidate: &crate::IndexedSymbol,
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

fn column_candidate(
    column: &ColumnMetadata,
    insert: &str,
    replacement: SourceSpan,
    bucket: &str,
) -> CompletionItem {
    let mut detail = column.detail.clone().unwrap_or_else(|| {
        format!(
            "{} · {} · stage {}",
            column.data_type,
            nullability(column.nullable),
            column.stage
        )
    });
    if let Some(qualifier) = &column.qualifier {
        detail.push_str(&format!(" · {qualifier}"));
    }
    if let Some(lineage) = &column.lineage {
        detail.push_str(&format!(" · from {lineage}"));
    }
    candidate(
        &column.name,
        insert,
        replacement,
        CompletionKind::Field,
        Some(detail),
        CompletionOrigin::DatasetSchema,
        bucket,
    )
}

fn complete_scalar_bindings(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) {
    let typed = prefix.trim_start_matches('$');
    let Some(document) = document else {
        return;
    };
    for symbol in &document.symbols {
        if symbol.value_kind != IndexedValueKind::Scalar
            || symbol.selection_span.range.start >= cursor
            || !symbol_visible(document, symbol, cursor)
            || !candidate_matches(&symbol.name, typed)
        {
            continue;
        }
        let label = format!("${}", symbol.name);
        let bucket = format!("00:{:020}", symbol.selection_span.range.start);
        output.push(candidate(
            &label,
            &label,
            replacement,
            CompletionKind::Variable,
            symbol.detail.clone(),
            CompletionOrigin::LexicalScope,
            &bucket,
        ));
    }
}

fn complete_table_bindings(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    project: Option<&ModuleAnalysis>,
    output: &mut Vec<CompletionItem>,
) {
    let typed = prefix.trim_start_matches('$');
    let Some(document) = document else {
        return;
    };
    for symbol in &document.symbols {
        if symbol.value_kind != IndexedValueKind::Table
            || symbol.selection_span.range.start >= cursor
            || !symbol_visible(document, symbol, cursor)
            || !candidate_matches(&symbol.name, typed)
        {
            continue;
        }
        let label = format!("${}", symbol.name);
        let bucket = format!("00:{:020}", symbol.selection_span.range.start);
        let detail = project
            .and_then(|project| project.resolved_module_graph.as_deref())
            .and_then(|resolved| {
                resolved
                    .stores
                    .values()
                    .find(|store| store.source_name == symbol.name)
            })
            .map(|store| {
                let fields = store
                    .fields
                    .iter()
                    .map(|field| format!("{}: {}", field.name, field.data_type))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("table-valued store {{ {fields} }}")
            })
            .or_else(|| symbol.detail.clone());
        output.push(candidate(
            &label,
            &label,
            replacement,
            CompletionKind::Table,
            detail,
            CompletionOrigin::LexicalScope,
            &bucket,
        ));
    }
}

fn complete_temporal_qualifiers(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) {
    let Some((base, typed)) = prefix
        .strip_prefix('$')
        .and_then(|value| value.split_once('@'))
    else {
        return;
    };
    let Some(document) = document else {
        return;
    };
    let in_handler = document.symbols.iter().any(|symbol| {
        symbol.keyword == "on"
            && symbol.scope_span.range.start <= cursor
            && cursor <= symbol.scope_span.range.end
    });
    if !in_handler
        || !document.symbols.iter().any(|symbol| {
            matches!(
                symbol.value_kind,
                IndexedValueKind::Scalar | IndexedValueKind::Table
            ) && symbol.name.eq_ignore_ascii_case(base)
                && symbol.selection_span.range.start < cursor
                && symbol_visible(document, symbol, cursor)
        })
    {
        return;
    }
    for temporal in ["start", "previous"] {
        if candidate_matches(temporal, typed) {
            let label = format!("${base}@{temporal}");
            output.push(candidate(
                &label,
                &label,
                replacement,
                CompletionKind::Variable,
                Some(format!("{base} at {temporal}")),
                CompletionOrigin::LexicalScope,
                "00",
            ));
        }
    }
}

fn complete_functions(
    prefix: &str,
    replacement: SourceSpan,
    project: Option<&ModuleAnalysis>,
    in_event: bool,
    output: &mut Vec<CompletionItem>,
) {
    let inventories = project.into_iter().flat_map(|project| {
        [
            ("scalar", project.functions.scalar.as_slice()),
            ("aggregate", project.functions.aggregate.as_slice()),
            ("window", project.functions.window.as_slice()),
        ]
    });
    for (kind, functions) in inventories {
        for function in functions {
            if candidate_matches(function, prefix) {
                output.push(candidate(
                    function,
                    &format!("{function}()"),
                    replacement,
                    CompletionKind::Function,
                    Some(format!("DataFusion {kind} function")),
                    CompletionOrigin::FunctionRegistry,
                    "10",
                ));
            }
        }
    }
    for operation in INTRINSIC_OPERATION_SIGNATURES.iter().filter(|signature| {
        in_event
            && signature
                .contexts
                .contains(&IntrinsicOperationContext::EventExpression)
    }) {
        if candidate_matches(operation.name, prefix) {
            output.push(candidate(
                operation.name,
                &format!("{}()", operation.name),
                replacement,
                CompletionKind::Function,
                Some(format!(
                    "{} Returns `{}`.",
                    operation.docs,
                    operation.result.as_str()
                )),
                CompletionOrigin::FunctionRegistry,
                "00",
            ));
        }
    }
}

fn complete_types(prefix: &str, replacement: SourceSpan, output: &mut Vec<CompletionItem>) {
    for data_type in PhysicalType::CONSTRUCTORS.iter().copied().chain([
        "BOOLEAN",
        "BIGINT",
        "DOUBLE",
        "VARCHAR",
        "DATE",
        "TIMESTAMP",
    ]) {
        if candidate_matches(data_type, prefix) {
            output.push(candidate(
                data_type,
                data_type,
                replacement,
                CompletionKind::Type,
                Some("DataFusion / Arrow type".to_owned()),
                CompletionOrigin::FunctionRegistry,
                "00",
            ));
        }
    }
}

fn complete_projection_alias(
    prefix: &str,
    replacement: SourceSpan,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) {
    let Some(as_index) = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.span.range.end <= cursor && token.is_word("as"))
        .map(|(index, _)| index)
        .next_back()
    else {
        return;
    };
    let depth = tokens[as_index].depth;
    let start = tokens[..as_index]
        .iter()
        .rposition(|token| token.depth == depth && token.is_comma())
        .map_or(0, |index| index + 1);
    let mut words = tokens[start..as_index]
        .iter()
        .filter(|token| token.depth >= depth)
        .filter_map(SqlToken::word)
        .filter(|word| {
            !matches!(
                word.to_ascii_lowercase().as_str(),
                "cast" | "try_cast" | "as" | "distinct" | "filter" | "over"
            )
        })
        .map(to_alias_fragment)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    words.dedup();
    let suggestion = words
        .into_iter()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("_");
    if suggestion.is_empty() || !candidate_matches(&suggestion, prefix) {
        return;
    }
    output.push(candidate(
        &suggestion,
        &suggestion,
        replacement,
        CompletionKind::Field,
        Some("projection output alias".to_owned()),
        CompletionOrigin::Syntax,
        "00",
    ));
}

fn to_alias_fragment(value: &str) -> String {
    let mut output = String::new();
    let mut prior_separator = false;
    for character in value.chars() {
        if character == '_' || character.is_alphanumeric() {
            output.extend(character.to_lowercase());
            prior_separator = false;
        } else if !prior_separator && !output.is_empty() {
            output.push('_');
            prior_separator = true;
        }
    }
    output.trim_matches('_').to_owned()
}

fn complete_keywords(
    prefix: &str,
    replacement: SourceSpan,
    roles: &BTreeSet<SqlExpectedRole>,
    output: &mut Vec<CompletionItem>,
) {
    let keywords: &[&str] = if roles.contains(&SqlExpectedRole::Relation) {
        &["SELECT", "LATERAL"]
    } else if roles.contains(&SqlExpectedRole::Type) {
        &["AS"]
    } else {
        &[
            "SELECT",
            "FROM",
            "WHERE",
            "GROUP BY",
            "ORDER BY",
            "HAVING",
            "LIMIT",
            "JOIN",
            "LEFT JOIN",
            "RIGHT JOIN",
            "FULL JOIN",
            "ON",
            "UNION ALL",
            "AS",
            "CASE",
            "WHEN",
            "THEN",
            "ELSE",
            "END",
            "OVER",
            "PARTITION BY",
        ]
    };
    for keyword in keywords {
        if candidate_matches(keyword, prefix) {
            output.push(candidate(
                keyword,
                keyword,
                replacement,
                CompletionKind::Keyword,
                Some("SQL keyword".to_owned()),
                CompletionOrigin::Syntax,
                "30",
            ));
        }
    }
}

fn validate_expression(
    authored: &str,
    dataset: &AnalyzedDataset,
    repair: SqlRepairStrategy,
    cursor: usize,
) -> bool {
    let repaired = build_repaired_sql(authored, cursor, repair);
    let Ok(schema) = DFSchema::try_from(dataset.schema.as_ref().clone()) else {
        return false;
    };
    let state = SessionStateBuilder::new().with_default_features().build();
    LOGICAL_EXPRESSION_PLANS.fetch_add(1, Ordering::Relaxed);
    state
        .create_logical_expr(&repaired.text, &schema)
        .and_then(|expression| expression.get_type(&schema))
        .is_ok()
}

fn validate_projection(
    authored: &str,
    dataset: &AnalyzedDataset,
    repair: SqlRepairStrategy,
    cursor: usize,
) -> bool {
    let repaired = build_repaired_sql(authored, cursor, repair);
    let Ok(projection) = SqlProjection::parse(&repaired.text) else {
        return false;
    };
    let Ok(schema) = DFSchema::try_from(dataset.schema.as_ref().clone()) else {
        return false;
    };
    let state = SessionStateBuilder::new().with_default_features().build();
    projection.items().iter().all(|item| {
        let expression = match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expression)
            | sqlparser::ast::SelectItem::ExprWithAlias {
                expr: expression, ..
            }
            | sqlparser::ast::SelectItem::ExprWithAliases {
                expr: expression, ..
            } => expression,
            sqlparser::ast::SelectItem::QualifiedWildcard(_, _)
            | sqlparser::ast::SelectItem::Wildcard(_) => return false,
        };
        LOGICAL_EXPRESSION_PLANS.fetch_add(1, Ordering::Relaxed);
        state
            .create_logical_expr(&expression.to_string(), &schema)
            .and_then(|expression| expression.get_type(&schema))
            .is_ok()
    })
}

fn deduplicate_relations(relations: &mut Vec<RelationMetadata>) {
    relations.sort_by(|left, right| {
        left.scope_depth
            .cmp(&right.scope_depth)
            .then_with(|| left.visible_name().cmp(right.visible_name()))
            .then_with(|| left.path.cmp(&right.path))
    });
    relations.dedup_by(|left, right| {
        left.visible_name()
            .eq_ignore_ascii_case(right.visible_name())
            && eq_path(&left.path, &right.path)
    });
}

fn deduplicate_columns(columns: &mut Vec<ColumnMetadata>) {
    columns.sort_by(|left, right| left.name.cmp(&right.name));
    columns.dedup_by(|left, right| left.name.eq_ignore_ascii_case(&right.name));
}

fn nullability(nullable: bool) -> &'static str {
    if nullable { "nullable" } else { "not null" }
}

fn is_clause_token(token: &SqlToken<'_>) -> bool {
    token.word().is_some_and(is_clause_word)
}

fn is_clause_word(word: &str) -> bool {
    [
        "select",
        "from",
        "where",
        "group",
        "order",
        "having",
        "limit",
        "join",
        "left",
        "right",
        "full",
        "inner",
        "cross",
        "on",
        "using",
        "union",
        "except",
        "intersect",
        "qualify",
        "window",
        "fetch",
        "offset",
    ]
    .iter()
    .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

fn candidate_matches(candidate: &str, typed: &str) -> bool {
    let typed = typed.trim_start_matches('$').to_ascii_lowercase();
    candidate.to_ascii_lowercase().contains(&typed)
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    label: &str,
    insert: &str,
    replacement: SourceSpan,
    kind: CompletionKind,
    detail: Option<String>,
    origin: CompletionOrigin,
    bucket: &str,
) -> CompletionItem {
    CompletionItem {
        label: label.to_owned(),
        replacement,
        insert_text: insert.to_owned(),
        insert_text_format: CompletionTextFormat::PlainText,
        kind,
        detail,
        documentation: None,
        filter_text: Some(label.to_owned()),
        sort_key: format!("{bucket}:{}", label.to_ascii_lowercase()),
        origin,
        deprecated: false,
    }
}

fn rank_and_deduplicate(items: &mut Vec<CompletionItem>, prefix: &str) {
    let typed = prefix.trim_start_matches('$').to_ascii_lowercase();
    for item in items.iter_mut() {
        let label = item.label.trim_start_matches('$').to_ascii_lowercase();
        let prefix_rank = if label == typed {
            "0"
        } else if label.starts_with(&typed) {
            "1"
        } else {
            "2"
        };
        item.sort_key = format!("{prefix_rank}:{}", item.sort_key);
    }
    items.sort_by(|left, right| {
        left.sort_key
            .cmp(&right.sort_key)
            .then_with(|| left.label.cmp(&right.label))
    });
    items
        .dedup_by(|left, right| left.label == right.label && left.insert_text == right.insert_text);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_lang_core::{SourceFile, SourceId};

    use super::*;
    use crate::{DocumentSnapshot, SourceRevision, analyze_syntax};

    fn syntax(text: &str) -> (SourceOrigin, SyntaxAnalysis) {
        let origin = SourceOrigin::Memory("sql.avenger".to_owned());
        let snapshot = DocumentSnapshot::new(
            origin.clone(),
            SourceRevision::from_text(text),
            Arc::<str>::from(text),
        );
        (origin, analyze_syntax(&snapshot))
    }

    #[test]
    fn whole_island_keeps_tokens_after_cursor_and_classifies_member() {
        let text = "avenger 1; chart cartesian as chart { table sql as t { sql: SELECT m. FROM vega.movies AS m; } }";
        let cursor = text.find("m.").unwrap() + 2;
        let (_, syntax) = syntax(text);
        let node = syntax
            .parsed
            .nodes
            .iter()
            .find(|node| matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. }))
            .unwrap();
        let tokens = island_tokens(&syntax, node.span);
        assert!(tokens.iter().any(|token| token.is_word("vega")));
        let replacement = sql_replacement_span(&syntax, node.span, cursor);
        let roles = expected_roles(&tokens, cursor, SqlIslandContext::QueryProperty, "");
        assert_eq!(replacement.range, ByteSpan::empty(cursor));
        assert!(roles.contains(&SqlExpectedRole::QualifierMember));
        assert_eq!(
            select_repair(&tokens, cursor, &roles).0,
            SqlRepairStrategy::QualifierMember
        );
        let authored = &text[node.span.range.as_range()];
        let recovery = recover_sql(
            authored,
            cursor - node.span.range.start,
            SqlIslandRoot::Query,
            &tokens,
            cursor,
            &roles,
        );
        assert_eq!(recovery.strategy, SqlRepairStrategy::QualifierMember);
        assert!(recovery.parsed);
        let generated = recovery.repaired.generated[0].clone();
        assert_eq!(recovery.repaired.authored_offset(generated.start), None);
        assert_eq!(
            recovery.repaired.authored_offset(generated.end),
            Some(cursor - node.span.range.start)
        );
    }

    #[test]
    fn scope_reads_relations_on_both_sides_of_cursor() {
        let source = SourceFile::new(
            SourceId::new(0),
            SourceOrigin::Memory("scope".to_owned()),
            "avenger 1; chart cartesian as chart { table sql as t { sql: SELECT m. FROM vega.movies AS m; } }",
        );
        let syntax = crate::analyze_syntax(&DocumentSnapshot::new(
            source.origin.clone(),
            SourceRevision::from_text(source.text()),
            source.text(),
        ));
        let node = syntax
            .parsed
            .nodes
            .iter()
            .find(|node| matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. }))
            .unwrap();
        let tokens = island_tokens(&syntax, node.span);
        let cursor = source.text().find("m.").unwrap() + 2;
        let scope = build_query_scope(&tokens, cursor, 0, &[], None, None);
        assert!(
            scope
                .relations
                .iter()
                .any(|relation| relation.visible_name() == "m")
        );
    }

    #[test]
    fn completion_metrics_never_claim_execution() {
        let before = SqlCompletionMetrics::snapshot();
        assert_eq!(before.physical_plans, 0);
        assert_eq!(before.executions, 0);
    }

    #[test]
    fn frozen_sql_corpus_has_bounded_recovery_and_one_cursor() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/sql-completion-corpus.json"))
                .unwrap();
        let cases = corpus["cases"].as_array().unwrap();
        assert!(cases.len() >= 30);
        for case in cases {
            let sql = case["sql"].as_str().unwrap();
            assert_eq!(sql.matches("⟦cursor⟧").count(), 1, "{}", case["name"]);
            let cursor = sql.find("⟦cursor⟧").unwrap();
            let sql = sql.replacen("⟦cursor⟧", "", 1);
            let text = if case["root"] == "query" {
                format!(
                    "avenger 1; chart cartesian as chart {{ table sql as t {{ sql: {sql}; }} }}"
                )
            } else {
                format!("avenger 1; chart cartesian as chart {{ mark symbol {{ x: {sql}; }} }}")
            };
            let absolute = text.find(&sql).unwrap() + cursor;
            let (_, syntax) = syntax(&text);
            let node = syntax
                .parsed
                .nodes
                .iter()
                .filter(|node| {
                    matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. })
                        && node.span.range.start <= absolute
                        && absolute <= node.span.range.end
                })
                .min_by_key(|node| node.span.range.len())
                .unwrap_or_else(|| panic!("missing SQL island for {}", case["name"]));
            let TolerantSyntaxNodeKind::SqlIsland { context } = node.kind else {
                unreachable!()
            };
            let tokens = island_tokens(&syntax, node.span);
            let replacement = sql_replacement_span(&syntax, node.span, absolute);
            let prefix = &text[replacement.range.start..absolute];
            let roles = expected_roles(&tokens, absolute, context, prefix);
            let (_, attempts) = select_repair(&tokens, absolute, &roles);
            assert!(!roles.is_empty(), "{}", case["name"]);
            assert!(attempts <= MAX_REPAIR_ATTEMPTS, "{}", case["name"]);
        }
    }
}
