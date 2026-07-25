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
use avenger_lang_compiler::{AnalyzedDataset, ModuleAnalysis, physical_type_to_arrow};
use avenger_lang_core::{
    ByteSpan, PhysicalType, SourceOrigin, SourceSpan,
    ast::{SqlExpression, SqlQuery},
    sql::{DOMAIN_RANGE_HELPERS, LosslessTokenKind, RESERVED_HELPER_NAMES, TokenClass},
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
        let tokens = island_tokens(syntax, node.span);
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
        if context.root() == SqlIslandRoot::Query {
            reconcile_query_output_with_datafusion(
                &text[node.span.range.as_range()],
                &catalog,
                &mut scope,
            );
        }
        let expression_planned = if context.root() == SqlIslandRoot::Expression {
            project
                .and_then(|project| input_dataset(project, dataset_context, request.byte_offset))
                .is_some_and(|dataset| {
                    validate_expression(
                        &text[node.span.range.as_range()],
                        dataset,
                        recovery.strategy,
                        request.byte_offset.saturating_sub(node.span.range.start),
                    )
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

    if roles.contains(&SqlExpectedRole::QualifierMember)
        && let Some(qualifier) = qualifier.as_deref()
    {
        let found = complete_qualifier(
            qualifier,
            prefix,
            replacement,
            roles,
            scope,
            catalog,
            document,
            project,
            request.byte_offset,
            &mut items,
        );
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
        complete_columns(prefix, replacement, scope, &mut items);
        complete_scalar_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            &mut items,
        );
        complete_functions(prefix, replacement, project, &mut items);
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
    for column in &relation.columns {
        if candidate_matches(&column.name, prefix) {
            output.push(column_candidate(column, &column.name, replacement, "00"));
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
    let mut detail = format!(
        "{} · {} · stage {}",
        column.data_type,
        nullability(column.nullable),
        column.stage
    );
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
    for helper in RESERVED_HELPER_NAMES.iter().chain(DOMAIN_RANGE_HELPERS) {
        if candidate_matches(helper, prefix) {
            output.push(candidate(
                helper,
                &format!("{helper}()"),
                replacement,
                CompletionKind::Function,
                Some("Avenger SQL helper".to_owned()),
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
