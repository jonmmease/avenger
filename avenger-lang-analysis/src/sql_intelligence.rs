//! Completion-specific recovery and semantic SQL scopes.
//!
//! The strict Avenger SQL frontend remains the language authority. This module
//! deliberately owns only the bounded, cursor-oriented recovery needed while
//! an author is editing an incomplete island. Stable relation schemas come
//! from `ModuleAnalysis`; expressions are validated with DataFusion's logical
//! expression planner and are never executed.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use arrow::datatypes::DataType;
use avenger_chart_schema::{
    NativeKindNamespace, NativeSchemaSnapshot, ProjectionExpressionMode, ValueShape,
};
use avenger_lang_compiler::{
    AnalyzedDataset, DatasetStageKind, FunctionCategory, ModuleAnalysis, physical_type_to_arrow,
};
use avenger_lang_core::{
    ByteSpan, INTRINSIC_OPERATION_SIGNATURES, IntrinsicOperationContext, PhysicalType, SourceFile,
    SourceId, SourceOrigin, SourceSpan,
    ast::{SqlExpression, SqlProjection, SqlQuery},
    contextual_access_signature,
    resolve::{
        DeclarationId, ResolvedDeclaration, ResolvedEventScope, ResolvedEventSurface,
        ResolvedKindBinding, ResolvedTarget,
    },
    sql::{LosslessTokenKind, LosslessTokenStream, TokenClass, tokenize_lossless},
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

use crate::completion_rank::{candidate_matches, rank_and_deduplicate};
use crate::{
    AnalysisCancellation, CompletionInvocation, CompletionItem, CompletionKind, CompletionOrigin,
    CompletionQualification, CompletionSemanticKind, CompletionTextFormat, CompletionValidity,
    DatasetContext, DocumentSemanticIndex, IndexedValueKind, PositionRequest, RootAnalysis,
    SyntaxAnalysis, WorkspaceSemanticIndex,
};

const MAX_REPAIR_ATTEMPTS: usize = 6;
const MAX_PREFIX_FALLBACK_TOKENS: usize = 32;

/// Exact syntactic/semantic intentions that may legally satisfy the cursor.
///
/// This is deliberately more precise than the former coarse role set: the
/// candidate layer can distinguish query starts, clause transitions, binders,
/// qualified members, and expression states without inferring intent from a
/// broad `Expression` or `Clause` bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlIntent {
    QueryStart,
    ClauseTransition,
    RelationPath,
    QuotedColumn,
    ExpressionOperand,
    ExpressionOperator,
    QualifiedMember,
    FunctionName,
    TypeName,
    Binding,
    NameBinder,
    CteName,
    CteBodyStart,
    WindowName,
    WildcardModifier,
    Nothing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum QueryClause {
    #[default]
    Start,
    With,
    Select,
    From,
    Join,
    On,
    Using,
    Where,
    GroupBy,
    Having,
    Window,
    Qualify,
    OrderBy,
    Limit,
    Offset,
    Fetch,
    SetOperation,
    Values,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CteHeaderState {
    Name,
    RecursiveKeyword,
    AfterName,
    AfterAs,
    BodyComplete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaseStage {
    Case,
    When,
    Then,
    Else,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParentScopePolicy {
    Correlated,
    IsolatedDerived,
    Lateral { boundary: usize },
}

impl QueryClause {
    const fn name(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::With => "with",
            Self::Select => "select",
            Self::From => "from",
            Self::Join => "join",
            Self::On => "on",
            Self::Using => "using",
            Self::Where => "where",
            Self::GroupBy => "group_by",
            Self::Having => "having",
            Self::Window => "window",
            Self::Qualify => "qualify",
            Self::OrderBy => "order_by",
            Self::Limit => "limit",
            Self::Offset => "offset",
            Self::Fetch => "fetch",
            Self::SetOperation => "set_operation",
            Self::Values => "values",
        }
    }
}

/// The deliberately small recovery matrix used before consulting semantic
/// scopes. Names are stable for corpus baselines and metrics, not diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlRepairStrategy {
    None,
    QualifierMember,
    EmptyExpression,
    MissingRelation,
    MissingType,
    QueryStart,
    TrailingComma,
    CloseDelimiters,
    ParsablePrefix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlCursorSentinelKind {
    Expression,
    QuotedMember,
    Relation,
    Type,
    QueryStart,
}

/// Test/telemetry view of one completion classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlCompletionDebug {
    pub island: SourceSpan,
    pub site: avenger_lang_core::syntax::SqlIslandSite,
    pub lexical_mode: SqlLexicalMode,
    pub cursor_path: Option<SqlCursorPathDebug>,
    pub intents: BTreeSet<SqlIntent>,
    pub repair: SqlRepairStrategy,
    pub sentinel: Option<SqlCursorSentinelKind>,
    pub repair_attempts: usize,
    pub token_count: usize,
    pub expression_planned: bool,
    pub repaired_parse: bool,
    pub synthetic_ranges: Vec<Range<usize>>,
    pub scope_relations: Vec<SqlScopeRelationDebug>,
    pub query_blocks: Vec<SqlQueryBlockDebug>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlCursorPathDebug {
    pub clause: String,
    pub nesting_depth: usize,
    pub item_index: usize,
    pub qualifier: Option<String>,
    pub replacement: SourceSpan,
    pub ast_path: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SqlItemParseStatus {
    Parsed,
    Cursor,
    Empty,
    Unparsed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlClauseItemDebug {
    pub clause: String,
    pub index: usize,
    pub span: SourceSpan,
    pub status: SqlItemParseStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlQueryBlockDebug {
    pub index: usize,
    pub parent: Option<usize>,
    pub depth: usize,
    pub span: SourceSpan,
    pub active: bool,
    pub items: Vec<SqlClauseItemDebug>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlScopeRelationDebug {
    pub name: String,
    pub columns: Vec<String>,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SqlLexicalMode {
    Code,
    DoubleQuotedIdentifier,
    SingleQuotedString,
    DollarQuotedString,
    LineComment,
    BlockComment,
    Binding,
    TemporalQualifier,
}

#[derive(Clone, Debug)]
struct SqlCursorContext {
    mode: SqlLexicalMode,
    replacement: SourceSpan,
    decoded_prefix: String,
    synthetic_closer: Option<String>,
}

/// Process-local proof counters. The latter two counters intentionally remain
/// zero: editor completion does not build physical plans or execute data.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SqlCompletionMetrics {
    pub requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub cache_evictions: u64,
    pub logical_expression_plans: u64,
    pub logical_query_plans: u64,
    pub invalid_candidates: u64,
    pub physical_plans: u64,
    pub scans: u64,
    pub collects: u64,
    pub executions: u64,
}

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static CACHE_HITS: AtomicU64 = AtomicU64::new(0);
static CACHE_MISSES: AtomicU64 = AtomicU64::new(0);
static CACHE_EVICTIONS: AtomicU64 = AtomicU64::new(0);
static LOGICAL_EXPRESSION_PLANS: AtomicU64 = AtomicU64::new(0);
static LOGICAL_QUERY_PLANS: AtomicU64 = AtomicU64::new(0);
static INVALID_CANDIDATES: AtomicU64 = AtomicU64::new(0);

impl SqlCompletionMetrics {
    pub fn snapshot() -> Self {
        Self {
            requests: REQUESTS.load(Ordering::Relaxed),
            cache_hits: CACHE_HITS.load(Ordering::Relaxed),
            cache_misses: CACHE_MISSES.load(Ordering::Relaxed),
            cache_evictions: CACHE_EVICTIONS.load(Ordering::Relaxed),
            logical_expression_plans: LOGICAL_EXPRESSION_PLANS.load(Ordering::Relaxed),
            logical_query_plans: LOGICAL_QUERY_PLANS.load(Ordering::Relaxed),
            invalid_candidates: INVALID_CANDIDATES.load(Ordering::Relaxed),
            physical_plans: 0,
            scans: 0,
            collects: 0,
            executions: 0,
        }
    }
}

pub(crate) struct SqlCompletionOutput {
    pub items: Vec<CompletionItem>,
    pub is_incomplete: bool,
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
        matches!(
            self.token,
            Some(Token::Word(word))
                if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(expected)
        )
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
    /// Number of duplicate source occurrences collapsed by USING/NATURAL
    /// joins for each output name. Raw source-column counts minus this value
    /// determine whether an unqualified output remains ambiguous.
    merged_column_reductions: BTreeMap<String, usize>,
}

#[derive(Clone, Debug)]
struct QuerySkeleton {
    blocks: Vec<SqlQueryBlockDebug>,
    active_block: usize,
}

#[derive(Clone, Debug)]
struct CachedSqlAnalysis {
    intents: BTreeSet<SqlIntent>,
    repair: SqlRepairStrategy,
    sentinel: Option<SqlCursorSentinelKind>,
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
    authored_start: usize,
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
        Some(
            self.authored_start
                .saturating_add(repaired_offset.saturating_sub(generated_before)),
        )
    }
}

struct RecoveryResult {
    strategy: SqlRepairStrategy,
    sentinel: Option<SqlCursorSentinelKind>,
    attempts: usize,
    repaired: RepairedSql,
    parsed: bool,
}

const SQL_CACHE_MAX_ENTRIES: usize = 256;
const SQL_CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Default)]
struct SqlCompletionCacheState {
    entries: BTreeMap<String, (Arc<CachedSqlAnalysis>, usize)>,
    recency: VecDeque<String>,
    bytes: usize,
}

/// Bounded, workspace-owned authored SQL analysis cache. The owning
/// `AnalysisService` shares it across published generations; independent
/// workspaces and tests never share completion state.
#[derive(Debug, Default)]
pub(crate) struct SqlCompletionCache {
    state: Mutex<SqlCompletionCacheState>,
}

impl SqlCompletionCache {
    fn get(&self, key: &str) -> Option<Arc<CachedSqlAnalysis>> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let value = state.entries.get(key)?.0.clone();
        state.recency.retain(|candidate| candidate != key);
        state.recency.push_back(key.to_owned());
        Some(value)
    }

    fn insert(&self, key: String, value: Arc<CachedSqlAnalysis>) {
        let estimated_bytes = estimate_cached_analysis_bytes(&key, &value);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((_, bytes)) = state.entries.remove(&key) {
            state.bytes = state.bytes.saturating_sub(bytes);
            state.recency.retain(|candidate| candidate != &key);
        }
        state.bytes = state.bytes.saturating_add(estimated_bytes);
        state.entries.insert(key.clone(), (value, estimated_bytes));
        state.recency.push_back(key);
        while state.entries.len() > SQL_CACHE_MAX_ENTRIES || state.bytes > SQL_CACHE_MAX_BYTES {
            let Some(oldest) = state.recency.pop_front() else {
                break;
            };
            if let Some((_, bytes)) = state.entries.remove(&oldest) {
                state.bytes = state.bytes.saturating_sub(bytes);
                CACHE_EVICTIONS.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[cfg(test)]
    fn len_and_bytes(&self) -> (usize, usize) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (state.entries.len(), state.bytes)
    }
}

fn estimate_cached_analysis_bytes(key: &str, value: &CachedSqlAnalysis) -> usize {
    let relation_bytes = value
        .catalog
        .iter()
        .chain(value.scope.relations.iter())
        .chain(value.scope.ctes.iter())
        .map(|relation| {
            relation.path.iter().map(String::len).sum::<usize>()
                + relation.alias.as_ref().map_or(0, String::len)
                + relation.detail.len()
                + relation
                    .columns
                    .iter()
                    .map(|column| column.name.len() + column.stage.len() + 128)
                    .sum::<usize>()
        })
        .sum::<usize>();
    key.len() + relation_bytes + value.token_count * 64 + 512
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn complete_sql(
    request: &PositionRequest,
    syntax: &SyntaxAnalysis,
    registry: &NativeSchemaSnapshot,
    semantic_index: &WorkspaceSemanticIndex,
    semantic_roots: &BTreeMap<String, RootAnalysis>,
    dataset_contexts: &BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    invocation: &CompletionInvocation,
    snippets: bool,
    cache: &SqlCompletionCache,
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
    let TolerantSyntaxNodeKind::SqlIsland { site, .. } = &node.kind else {
        return None;
    };
    let site = *site;
    let context = site.context();
    if site == avenger_lang_core::syntax::SqlIslandSite::PropertyValue
        && !property_value_accepts_sql(
            semantic_index,
            registry,
            &request.source,
            request.byte_offset,
            node.span,
        )
    {
        return None;
    }
    if site == avenger_lang_core::syntax::SqlIslandSite::PropertyValue
        && let Some(owner) =
            crate::intelligence::owner_symbol(semantic_index, &request.source, request.byte_offset)
        && owner.keyword == "slot"
        && owner.native_kind.as_deref() != Some("expr")
    {
        // Non-expression definition-slot defaults are structural values. In
        // particular, channel defaults enumerate physical channel identities
        // and must not be captured by the generic SQL property island.
        return None;
    }

    REQUESTS.fetch_add(1, Ordering::Relaxed);
    let text = syntax.parsed.tokens.text();
    let cursor_context = sql_cursor_context(text, node.span, request.byte_offset);
    let replacement = cursor_context.replacement;
    let prefix = cursor_context.decoded_prefix.as_str();
    if matches!(
        cursor_context.mode,
        SqlLexicalMode::SingleQuotedString
            | SqlLexicalMode::DollarQuotedString
            | SqlLexicalMode::LineComment
            | SqlLexicalMode::BlockComment
    ) {
        return Some(SqlCompletionOutput {
            items: Vec::new(),
            is_incomplete: false,
            debug: SqlCompletionDebug {
                island: node.span,
                site,
                lexical_mode: cursor_context.mode,
                cursor_path: None,
                intents: BTreeSet::from([SqlIntent::Nothing]),
                repair: SqlRepairStrategy::None,
                sentinel: None,
                repair_attempts: 0,
                token_count: 0,
                expression_planned: false,
                repaired_parse: false,
                synthetic_ranges: cursor_context
                    .synthetic_closer
                    .as_ref()
                    .map(|closer| {
                        request.byte_offset..request.byte_offset.saturating_add(closer.len())
                    })
                    .into_iter()
                    .collect(),
                scope_relations: Vec::new(),
                query_blocks: Vec::new(),
            },
        });
    }
    let current_owner_path = semantic_index
        .documents
        .get(&request.source)
        .and_then(|document| {
            document
                .sql_islands
                .iter()
                .find(|descriptor| descriptor.span == node.span)
        })
        .map(|descriptor| {
            descriptor
                .declaration_path
                .iter()
                .map(|owner| crate::DatasetContextOwner {
                    keyword: owner.keyword.clone(),
                    name: owner.name.clone(),
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (project, dataset_context) = project_at_owner(
        &request.source,
        request.byte_offset,
        semantic_roots,
        dataset_contexts,
        &current_owner_path,
    );
    let repaired_token_stream =
        patched_completion_token_stream(syntax, node.span, request.byte_offset, &cursor_context);
    let mut tokens = repaired_token_stream.as_ref().map_or_else(
        || island_tokens(syntax, node.span),
        |stream| {
            island_tokens_from_stream(
                stream,
                node.span,
                cursor_context
                    .synthetic_closer
                    .as_ref()
                    .map(|closer| (request.byte_offset, closer.len())),
            )
        },
    );
    if repaired_token_stream.is_some() {
        tokens.retain(|token| {
            token.span.range.start != cursor_context.replacement.range.start
                || token.span.range.end != request.byte_offset
        });
    }
    truncate_at_outer_delimiter(&mut tokens, site, request.byte_offset);
    let skeleton = build_query_skeleton(
        text,
        node.span,
        &tokens,
        request.byte_offset,
        context.root(),
    );
    cancellation.check().ok()?;
    let document = semantic_index.documents.get(&request.source);
    let projection_mode = projection_expression_mode(
        semantic_index,
        registry,
        &request.source,
        request.byte_offset,
        node.span,
    );
    let cache_key = sql_cache_key(request, node.span, project, dataset_context);
    let cached = cache.get(&cache_key);
    let cached = if let Some(cached) = cached {
        CACHE_HITS.fetch_add(1, Ordering::Relaxed);
        cached
    } else {
        CACHE_MISSES.fetch_add(1, Ordering::Relaxed);
        let intents = expected_intents(&tokens, request.byte_offset, context, prefix, &skeleton);
        let recovery = recover_sql(
            &text[node.span.range.as_range()],
            request.byte_offset.saturating_sub(node.span.range.start),
            context.root(),
            &tokens,
            request.byte_offset,
            &intents,
            cursor_context
                .replacement
                .range
                .start
                .saturating_sub(node.span.range.start)
                ..cursor_context
                    .replacement
                    .range
                    .end
                    .saturating_sub(node.span.range.start),
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
            site,
            node.span,
        ));
        cancellation.check().ok()?;
        // Parentheses used by functions, USING, casts, and expressions do not
        // introduce a SQL name scope. The skeleton's active query-block depth
        // does; using raw cursor depth here made sources disappear inside
        // ordinary parenthesized constructs.
        let active_depth = skeleton.blocks[skeleton.active_block].depth;
        let mut scope = build_query_scope(
            &tokens,
            request.byte_offset,
            active_depth,
            context.root(),
            &catalog,
            project,
            dataset_context,
        );
        cancellation.check().ok()?;
        if matches!(
            site,
            avenger_lang_core::syntax::SqlIslandSite::ParamInitializer
                | avenger_lang_core::syntax::SqlIslandSite::OutputSource
                | avenger_lang_core::syntax::SqlIslandSite::CursorActionRhs
                | avenger_lang_core::syntax::SqlIslandSite::StateActionRhs
        ) {
            scope
                .relations
                .retain(|relation| relation.detail != "exact pipeline input");
        }
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
                &tokens,
                request.byte_offset,
                node.span.range.start,
            );
        }
        cancellation.check().ok()?;
        let expression_planned = if !matches!(
            site,
            avenger_lang_core::syntax::SqlIslandSite::ParamInitializer
                | avenger_lang_core::syntax::SqlIslandSite::OutputSource
                | avenger_lang_core::syntax::SqlIslandSite::CursorActionRhs
                | avenger_lang_core::syntax::SqlIslandSite::StateActionRhs
        ) && matches!(
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
            intents,
            repair: recovery.strategy,
            sentinel: recovery.sentinel,
            repair_attempts: recovery.attempts,
            token_count: tokens.len(),
            catalog,
            scope,
            expression_planned,
            repaired_parse: recovery.parsed,
            synthetic_ranges: recovery.repaired.generated,
        });
        cache.insert(cache_key, Arc::clone(&cached));
        cached
    };
    cancellation.check().ok()?;
    let mut effective_intents = cached.intents.clone();
    if cursor_context.mode == SqlLexicalMode::DoubleQuotedIdentifier {
        let before_quote = tokens
            .iter()
            .enumerate()
            .filter(|(_, token)| token.span.range.end <= replacement.range.start)
            .map(|(index, _)| index)
            .next_back();
        if type_position(&tokens, before_quote) {
            effective_intents.clear();
            effective_intents.insert(SqlIntent::TypeName);
        } else if before_quote
            .and_then(|index| tokens.get(index))
            .is_some_and(|token| token.is_word("as"))
        {
            effective_intents.clear();
            effective_intents.insert(SqlIntent::NameBinder);
        } else if context.root() == SqlIslandRoot::Query
            && matches!(
                query_clause_at(&tokens, request.byte_offset),
                QueryClause::From | QueryClause::Join
            )
        {
            effective_intents.clear();
            effective_intents.insert(SqlIntent::RelationPath);
        } else {
            let qualified = before_quote
                .and_then(|index| tokens.get(index))
                .is_some_and(SqlToken::is_period);
            effective_intents.clear();
            effective_intents.insert(SqlIntent::QuotedColumn);
            if qualified {
                effective_intents.insert(SqlIntent::QualifiedMember);
            }
        }
    }
    let named_window_reference = context.root() == SqlIslandRoot::Query
        && named_window_reference_position(&tokens, request.byte_offset, prefix);
    let wildcard_modifier = context.root() == SqlIslandRoot::Query
        && wildcard_modifier_position(&tokens, request.byte_offset, prefix);
    if named_window_reference {
        effective_intents.clear();
        effective_intents.insert(SqlIntent::WindowName);
    } else if wildcard_modifier {
        effective_intents.clear();
        effective_intents.insert(SqlIntent::WildcardModifier);
    }
    let intents = &effective_intents;
    let catalog = &cached.catalog;
    let scope = &cached.scope;

    let mut items = Vec::new();
    let allow_selection_bindings = context.root() != SqlIslandRoot::Query
        && enclosing_event(project, &request.source, request.byte_offset).is_none()
        && !matches!(
            site,
            avenger_lang_core::syntax::SqlIslandSite::ParamInitializer
                | avenger_lang_core::syntax::SqlIslandSite::CursorActionRhs
                | avenger_lang_core::syntax::SqlIslandSite::StateActionRhs
        );
    let mut incomplete = project.is_none();
    let mut resolved_qualified_member = false;
    let qualifier = qualifier_before(text, node.span, replacement.range.start);
    let bare_relation_member = cursor_context.mode == SqlLexicalMode::Code
        && qualifier.as_deref().is_some_and(|qualifier| {
            scope
                .relations
                .iter()
                .any(|relation| relation.visible_name().eq_ignore_ascii_case(qualifier))
        });
    let cursor_path = sql_cursor_path_debug(
        &tokens,
        request.byte_offset,
        qualifier.clone(),
        replacement,
        &skeleton,
    );
    let member_replacement = replacement;
    if named_window_reference {
        complete_named_windows(
            prefix,
            replacement,
            &tokens,
            request.byte_offset,
            &mut items,
        );
    }

    if cursor_context.mode == SqlLexicalMode::DoubleQuotedIdentifier
        && intents.contains(&SqlIntent::QuotedColumn)
        && ((context.root() == SqlIslandRoot::Query
            && tokens
                .iter()
                .any(|token| token.is_word("from") || token.is_word("join")))
            || dataset_context.is_some())
        && scope
            .relations
            .iter()
            .all(|relation| relation.columns.is_empty())
    {
        incomplete = true;
    }

    if (intents.contains(&SqlIntent::QualifiedMember) || qualifier.is_some())
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
                intents,
                scope,
                catalog,
                document,
                project,
                &request.source,
                request.byte_offset,
                site,
                cursor_context.mode == SqlLexicalMode::DoubleQuotedIdentifier,
                &mut items,
            )
        });
        resolved_qualified_member = found;
        if cursor_context.mode == SqlLexicalMode::Code
            && context.root() == SqlIslandRoot::Query
            && query_clause_at(&tokens, request.byte_offset) == QueryClause::Select
            && scope
                .relations
                .iter()
                .any(|relation| relation.visible_name().eq_ignore_ascii_case(qualifier))
        {
            let mut wildcard = candidate(
                "*",
                "*",
                replacement,
                CompletionKind::Keyword,
                Some(format!("all columns from {qualifier}")),
                CompletionOrigin::QueryScope,
                "00",
            );
            wildcard.semantic_kind = CompletionSemanticKind::SqlOperator;
            wildcard.semantic_identity = format!("QualifiedWildcard:{qualifier}");
            items.push(wildcard);
        }
        incomplete |= !found;
    }
    if intents.contains(&SqlIntent::RelationPath) {
        complete_relations(
            prefix,
            replacement,
            scope,
            catalog,
            cursor_context.mode == SqlLexicalMode::DoubleQuotedIdentifier,
            &mut items,
        );
        complete_table_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            project,
            site,
            node.span,
            &mut items,
        );
        complete_table_functions(prefix, replacement, project, snippets, &mut items);
    }
    if intents.contains(&SqlIntent::ExpressionOperand)
        || intents.contains(&SqlIntent::QuotedColumn)
        || intents.contains(&SqlIntent::FunctionName)
    {
        complete_definition_slots(
            prefix,
            replacement,
            document,
            request.byte_offset,
            site,
            node.span,
            &mut items,
        );
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
        if qualifier.is_none() {
            complete_columns(prefix, replacement, scope, &mut items);
        }
        complete_scalar_bindings(
            prefix,
            replacement,
            document,
            request.byte_offset,
            project,
            site,
            node.span,
            allow_selection_bindings,
            &mut items,
        );
        complete_functions(
            prefix,
            replacement,
            project,
            enclosing_event(project, &request.source, request.byte_offset).is_some(),
            context,
            &tokens,
            request.byte_offset,
            projection_mode,
            snippets,
            &mut items,
        );
    }
    if intents.contains(&SqlIntent::TypeName) {
        complete_types(prefix, replacement, &mut items);
    }
    if intents.contains(&SqlIntent::Binding) {
        if intents.contains(&SqlIntent::RelationPath) {
            complete_table_bindings(
                prefix,
                replacement,
                document,
                request.byte_offset,
                project,
                site,
                node.span,
                &mut items,
            );
        } else {
            complete_scalar_bindings(
                prefix,
                replacement,
                document,
                request.byte_offset,
                project,
                site,
                node.span,
                allow_selection_bindings,
                &mut items,
            );
        }
    }
    if !intents.contains(&SqlIntent::NameBinder) && !intents.contains(&SqlIntent::CteName) {
        complete_temporal_qualifiers(
            prefix,
            replacement,
            document,
            request.byte_offset,
            &mut items,
        );
        complete_keywords(
            prefix,
            replacement,
            intents,
            context,
            &tokens,
            request.byte_offset,
            &mut items,
        );
    }
    if named_window_reference {
        items.retain(|item| item.semantic_kind == CompletionSemanticKind::WindowName);
    } else if wildcard_modifier {
        items.retain(|item| {
            item.semantic_kind == CompletionSemanticKind::SqlKeyword
                && matches!(
                    item.label.as_str(),
                    "EXCLUDE" | "EXCEPT" | "REPLACE" | "ILIKE"
                )
        });
    }

    let outer_binder = match site {
        avenger_lang_core::syntax::SqlIslandSite::ParamInitializer => {
            Some(("parameter binder", "ParamBinder:as"))
        }
        avenger_lang_core::syntax::SqlIslandSite::OutputSource => {
            Some(("output binder", "OutputBinder:as"))
        }
        _ => None,
    };
    let outer_binder_ready = outer_binder.is_some()
        && replacement.range.start >= node.span.range.start
        && SqlExpression::parse(
            text[node.span.range.start..replacement.range.start]
                .trim_end()
                .trim(),
        )
        .is_ok();
    if outer_binder_ready {
        items.clear();
        if candidate_matches("as", prefix) {
            let (detail, identity) = outer_binder.unwrap();
            let mut binder = candidate(
                "as",
                "as",
                replacement,
                CompletionKind::Keyword,
                Some(detail.to_owned()),
                CompletionOrigin::Syntax,
                "00",
            );
            binder.semantic_kind = CompletionSemanticKind::SqlKeyword;
            binder.semantic_identity = identity.to_owned();
            items.push(binder);
        }
    }

    cancellation.check().ok()?;
    retain_candidates_for_lexical_mode(&mut items, cursor_context.mode);
    if resolved_qualified_member
        && cursor_context.mode == SqlLexicalMode::Code
        && !bare_relation_member
    {
        items.retain(|item| {
            matches!(
                item.semantic_kind,
                CompletionSemanticKind::ContextualMember
                    | CompletionSemanticKind::StructField
                    | CompletionSemanticKind::Relation
                    | CompletionSemanticKind::Catalog
                    | CompletionSemanticKind::Schema
            )
        });
    }
    if context.root() == SqlIslandRoot::Query
        && matches!(
            skeleton.active_clause(&tokens, request.byte_offset),
            QueryClause::Limit | QueryClause::Offset | QueryClause::Fetch
        )
    {
        items.retain(|item| item.semantic_kind != CompletionSemanticKind::DataColumn);
    }
    if skeleton.active_clause(&tokens, request.byte_offset) == QueryClause::Using
        && cursor_context.mode != SqlLexicalMode::DoubleQuotedIdentifier
    {
        items.clear();
    } else if skeleton.active_clause(&tokens, request.byte_offset) == QueryClause::Using {
        let mut seen = BTreeSet::new();
        items.retain_mut(|item| {
            if item.semantic_kind != CompletionSemanticKind::DataColumn
                || !seen.insert(item.label.to_ascii_lowercase())
            {
                return false;
            }
            item.insert_text = quote_identifier(&item.label);
            item.filter_text = Some(item.insert_text.clone());
            item.qualification = CompletionQualification::Unqualified;
            item.semantic_identity = format!("UsingColumn:{}", item.label);
            if let Some(detail) = &mut item.detail {
                detail.push_str(" · merged USING column");
            }
            true
        });
    }
    if bare_relation_member {
        // Authored columns are always double quoted. A bare relation member can
        // therefore only be the qualified wildcard; fields become available
        // after the author opens a quoted identifier (`relation."`).
        items.retain(|item| {
            item.semantic_kind == CompletionSemanticKind::SqlOperator && item.insert_text == "*"
        });
    }
    if let Some(expected) = expected_completion_type(
        semantic_index,
        registry,
        &request.source,
        request.byte_offset,
        node.span,
    ) {
        for item in &mut items {
            if let Some(data_type) = item.data_type.as_deref() {
                item.expected_type_compatible = Some(expected.accepts(data_type));
            }
        }
    }
    let invalid_candidates = completion_invariant_violations(&items, cursor_context.mode);
    if invalid_candidates != 0 {
        INVALID_CANDIDATES.fetch_add(invalid_candidates as u64, Ordering::Relaxed);
    }
    debug_assert_eq!(invalid_candidates, 0);
    annotate_usage_prevalence(&mut items, &tokens, semantic_index);
    rank_and_deduplicate(&mut items, prefix, invocation);
    Some(SqlCompletionOutput {
        items,
        is_incomplete: incomplete,
        debug: SqlCompletionDebug {
            island: node.span,
            site,
            lexical_mode: cursor_context.mode,
            cursor_path: Some(cursor_path),
            intents: intents.clone(),
            repair: cached.repair,
            sentinel: cached.sentinel,
            repair_attempts: cached.repair_attempts,
            token_count: cached.token_count,
            expression_planned: cached.expression_planned,
            repaired_parse: cached.repaired_parse,
            synthetic_ranges: cached.synthetic_ranges.clone(),
            scope_relations: scope
                .relations
                .iter()
                .chain(scope.ctes.iter())
                .map(|relation| SqlScopeRelationDebug {
                    name: relation.visible_name().to_owned(),
                    columns: relation
                        .columns
                        .iter()
                        .map(|column| format!("{}: {}", column.name, column.data_type))
                        .collect(),
                    detail: relation.detail.clone(),
                })
                .collect(),
            query_blocks: skeleton.blocks.clone(),
        },
    })
}

fn property_value_accepts_sql(
    index: &WorkspaceSemanticIndex,
    registry: &NativeSchemaSnapshot,
    origin: &SourceOrigin,
    cursor: usize,
    island: SourceSpan,
) -> bool {
    let Some(descriptor) = index.documents.get(origin).and_then(|document| {
        document
            .sql_islands
            .iter()
            .find(|descriptor| descriptor.span == island)
    }) else {
        return true;
    };
    let Some(first) = descriptor.property_path.first() else {
        return true;
    };
    if matches!(
        first.as_str(),
        "data"
            | "table"
            | "target"
            | "scope"
            | "surface"
            | "between"
            | "sharing"
            | "empty"
            | "combine"
            | "mode"
            | "consume"
            | "settle_exact"
            | "domain_contribution"
            | "layout"
            | "theme"
            | "time"
            | "format"
            | "guide"
            | "id"
    ) {
        return false;
    }
    let Some(owner) = crate::intelligence::owner_symbol(index, origin, cursor) else {
        // Core SQL-bearing properties such as event `filter` have no native
        // registry owner and remain expression islands.
        return true;
    };
    let Some(schema) = crate::intelligence::schema_for_symbol(registry, owner, index) else {
        return true;
    };
    let Some(property) = crate::intelligence::property_schema(schema, first) else {
        return true;
    };
    let mut shape = property.shape;
    for name in descriptor.property_path.iter().skip(1) {
        shape = match shape {
            ValueShape::Object(properties) | ValueShape::ConfiguredExpression(properties) => {
                let Some(property) = properties.get(name) else {
                    return false;
                };
                &property.shape
            }
            ValueShape::ConfiguredReference { properties, .. } => {
                let Some(property) = properties.get(name) else {
                    return false;
                };
                &property.shape
            }
            ValueShape::Array(element)
            | ValueShape::OneOrMany(element)
            | ValueShape::Map(element) => element,
            _ => return false,
        };
    }
    shape_accepts_sql(shape)
}

fn shape_accepts_sql(shape: &ValueShape) -> bool {
    match shape {
        ValueShape::SqlExpression | ValueShape::ConfiguredExpression(_) => true,
        ValueShape::Union(shapes) => shapes.iter().any(shape_accepts_sql),
        _ => false,
    }
}

fn complete_definition_slots(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    site: avenger_lang_core::syntax::SqlIslandSite,
    island: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    let Some(document) = document else {
        return;
    };
    let is_output = site == avenger_lang_core::syntax::SqlIslandSite::OutputSource;
    let owner = document
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.declaration_span.range.start <= island.range.start
                && island.range.end <= symbol.declaration_span.range.end
        })
        .min_by_key(|symbol| symbol.declaration_span.range.len());
    let is_slot_default = site == avenger_lang_core::syntax::SqlIslandSite::PropertyValue
        && owner.is_some_and(|owner| owner.keyword == "slot");
    if !is_output && !is_slot_default {
        return;
    }
    for slot in document.symbols.iter().filter(|symbol| {
        symbol.keyword == "slot"
            && symbol.selection_span.range.start < cursor
            && symbol_visible(document, symbol, cursor)
            && candidate_matches(&symbol.name, prefix)
    }) {
        let mut item = candidate(
            &slot.name,
            &slot.name,
            replacement,
            CompletionKind::Variable,
            slot.detail
                .clone()
                .or_else(|| Some("definition slot".to_owned())),
            CompletionOrigin::LexicalScope,
            "00",
        );
        item.semantic_kind = CompletionSemanticKind::ContextualMember;
        item.semantic_identity = format!("DefinitionSlot:{}", slot.identity);
        output.push(item);
    }
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
                TolerantSyntaxNodeKind::SqlIsland { site, .. }
                    if site.context().root() == SqlIslandRoot::Expression
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn column_hover(
    request: &PositionRequest,
    syntax: &SyntaxAnalysis,
    registry: &NativeSchemaSnapshot,
    semantic_index: &WorkspaceSemanticIndex,
    semantic_roots: &BTreeMap<String, RootAnalysis>,
    dataset_contexts: &BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    cache: &SqlCompletionCache,
    cancellation: &AnalysisCancellation,
) -> Option<(SourceSpan, String)> {
    let node = syntax
        .parsed
        .nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. })
                && node.span.range.start <= request.byte_offset
                && request.byte_offset < node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())?;
    let token = island_tokens(syntax, node.span)
        .into_iter()
        .filter(|token| {
            matches!(
                token.token,
                Some(Token::Word(word)) if word.quote_style == Some('"')
            ) && token.span.range.start <= request.byte_offset
                && request.byte_offset < token.span.range.end
        })
        .min_by_key(|token| token.span.range.len())?;
    let Some(Token::Word(word)) = token.token else {
        return None;
    };

    // Ask the ordinary SQL completion pipeline about the complete identifier,
    // even when the pointer is over its opening quote or an early character.
    // This keeps hover aligned with the exact query scope and effective Arrow
    // schemas used for completion across every SQL-island shape.
    let mut completion_request = request.clone();
    completion_request.byte_offset = token.span.range.end.saturating_sub(1);
    let completion = complete_sql(
        &completion_request,
        syntax,
        registry,
        semantic_index,
        semantic_roots,
        dataset_contexts,
        &CompletionInvocation::Invoked,
        false,
        cache,
        cancellation,
    )?;
    let mut columns = completion
        .items
        .into_iter()
        .filter(|item| {
            item.semantic_kind == CompletionSemanticKind::DataColumn && item.label == word.value
        })
        .collect::<Vec<_>>();
    columns.dedup_by(|left, right| {
        left.insert_text == right.insert_text
            && left.data_type == right.data_type
            && left.nullable == right.nullable
            && left.source_stage == right.source_stage
    });
    if columns.is_empty() {
        return None;
    }

    let text = syntax.parsed.tokens.text();
    let spelling = &text[token.span.range.as_range()];
    let markdown = if columns.len() == 1 {
        let column = &columns[0];
        let mut markdown = format!(
            "```sql\n{spelling}\n```\n\nSQL column\n\n- Arrow type: `{}`\n- Nullable: `{}`",
            column.data_type.as_deref().unwrap_or("unknown"),
            column.nullable.unwrap_or(true),
        );
        if let Some(stage) = &column.source_stage {
            markdown.push_str(&format!("\n- Source stage: `{stage}`"));
        }
        markdown
    } else {
        let mut markdown =
            format!("```sql\n{spelling}\n```\n\nAmbiguous SQL column. Possible sources:");
        for column in columns {
            markdown.push_str(&format!(
                "\n\n- `{}` — `{}`, {}",
                column.insert_text,
                column.data_type.as_deref().unwrap_or("unknown"),
                nullability(column.nullable.unwrap_or(true)),
            ));
        }
        markdown
    };
    Some((token.span, markdown))
}

pub(crate) fn typed_boundary_hover(
    request: &PositionRequest,
    syntax: &SyntaxAnalysis,
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
                TolerantSyntaxNodeKind::SqlIsland { site, .. }
                    if site.context().root() == SqlIslandRoot::Expression
            ) && node.span.range.start <= request.byte_offset
                && request.byte_offset <= node.span.range.end
        })
        .min_by_key(|node| node.span.range.len())?;
    let (analysis, _) = project_at(
        &request.source,
        request.byte_offset,
        semantic_roots,
        dataset_contexts,
    );
    let analysis = analysis?;
    let project = analysis.resolved_module_graph.as_deref()?;
    let mut property_path = Vec::new();
    let mut parent = node.parent;
    while let Some(id) = parent {
        let candidate = &syntax.parsed.nodes[id.get() as usize];
        match &candidate.kind {
            TolerantSyntaxNodeKind::Property { name } => property_path.push(name.clone()),
            TolerantSyntaxNodeKind::Declaration { .. } => break,
            _ => {}
        }
        parent = candidate.parent;
    }
    property_path.reverse();
    if property_path.first().is_some_and(|name| name == "value") {
        property_path.remove(0);
    }
    let boundary = project
        .stores
        .values()
        .filter_map(|store| {
            let declaration = resolved_declaration(project, &store.declaration)?;
            if !declaration_contains_node(project, declaration, request, node.span) {
                return None;
            }
            let field_name = property_path.first()?;
            let field = store
                .fields
                .iter()
                .find(|field| &field.name == field_name)?;
            let destination = physical_type_to_arrow(typed_member_destination(
                &field.data_type,
                &property_path[1..],
            )?);
            Some((
                declaration.span.range.len(),
                destination,
                format!(
                    "store `${}` field `{}`",
                    store.source_name,
                    property_path.join(".")
                ),
            ))
        })
        .chain(resolved_declarations(project).filter_map(|declaration| {
            if !declaration_contains_node(project, declaration, request, node.span) {
                return None;
            }
            if declaration.keyword == "set" && declaration.kind.as_deref() == Some("cursor") {
                return Some((
                    declaration.span.range.len(),
                    DataType::Utf8,
                    "cursor assignment".to_owned(),
                ));
            }
            let lvalue = declaration.state_lvalue.as_ref()?;
            match &lvalue.target {
                ResolvedTarget::Param(id) => {
                    let param = project.params.get(id)?;
                    Some((
                        declaration.span.range.len(),
                        analysis.param_types.get(id)?.clone(),
                        format!("assignment to param `${}`", param.source_name),
                    ))
                }
                ResolvedTarget::Store(id) => {
                    let store = project.stores.get(id)?;
                    let field_name = property_path.first()?;
                    let field = store
                        .fields
                        .iter()
                        .find(|field| &field.name == field_name)?;
                    let destination = physical_type_to_arrow(typed_member_destination(
                        &field.data_type,
                        &property_path[1..],
                    )?);
                    Some((
                        declaration.span.range.len(),
                        destination,
                        format!(
                            "assignment to store `${}` field `{}`",
                            store.source_name,
                            property_path.join(".")
                        ),
                    ))
                }
                _ => None,
            }
        }))
        .min_by_key(|(span_len, _, _)| *span_len)?;
    let (_, destination, path) = boundary;
    let text = syntax.parsed.tokens.text();
    let authored = &text[node.span.range.as_range()];
    let source_type = infer_row_free_expression_type(authored)
        .map(|data_type| format!("`{data_type:?}`"))
        .unwrap_or_else(|| "context-dependent or incomplete".to_owned());
    Some((
        node.span,
        format!(
            "**Typed SQL boundary** — {path}\n\n- Inferred SQL source type: {source_type}\n- Destination Arrow type: `{destination:?}` (`{destination}`)\n- Conversion: strict DataFusion/Arrow `CAST`\n\nA failed cast is an error. Write an inner `TRY_CAST` when failure should produce a typed `NULL`."
        ),
    ))
}

fn declaration_contains_node(
    project: &avenger_lang_core::ResolvedModuleGraph,
    declaration: &ResolvedDeclaration,
    request: &PositionRequest,
    node_span: SourceSpan,
) -> bool {
    let authored = project.expansion_source_map.authored_span(declaration.span);
    project.sources.get(authored.source).is_some_and(|source| {
        same_origin(&source.origin, &request.source)
            && authored.range.start <= node_span.range.start
            && node_span.range.end <= authored.range.end
    })
}

fn resolved_declarations(
    project: &avenger_lang_core::ResolvedModuleGraph,
) -> impl Iterator<Item = &ResolvedDeclaration> {
    fn collect<'a>(
        declarations: &'a [ResolvedDeclaration],
        result: &mut Vec<&'a ResolvedDeclaration>,
    ) {
        for declaration in declarations {
            result.push(declaration);
            collect(&declaration.children, result);
        }
    }
    let mut result = Vec::new();
    for module in project.source_modules.values() {
        collect(&module.roots, &mut result);
    }
    result.into_iter()
}

fn resolved_declaration<'a>(
    project: &'a avenger_lang_core::ResolvedModuleGraph,
    id: &DeclarationId,
) -> Option<&'a ResolvedDeclaration> {
    fn find<'a>(
        declarations: &'a [ResolvedDeclaration],
        id: &DeclarationId,
    ) -> Option<&'a ResolvedDeclaration> {
        declarations.iter().find_map(|declaration| {
            (&declaration.id == id)
                .then_some(declaration)
                .or_else(|| find(&declaration.children, id))
        })
    }
    project
        .source_modules
        .values()
        .find_map(|module| find(&module.roots, id))
}

fn typed_member_destination<'a>(
    mut data_type: &'a PhysicalType,
    path: &[String],
) -> Option<&'a PhysicalType> {
    for name in path {
        data_type = match data_type {
            PhysicalType::Struct(fields) => {
                &fields.iter().find(|field| &field.name == name)?.data_type
            }
            PhysicalType::Map { value, .. } => value,
            _ => return None,
        };
    }
    Some(data_type)
}

fn infer_row_free_expression_type(authored: &str) -> Option<DataType> {
    let normalized = avenger_lang_compiler::normalize_sql_expression(authored).ok()?;
    let schema = DFSchema::empty();
    let state = SessionStateBuilder::new().with_default_features().build();
    let expression = state.create_logical_expr(&normalized, &schema).ok()?;
    expression.get_type(&schema).ok()
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
        let TolerantSyntaxNodeKind::SqlIsland { site, .. } = &node.kind else {
            continue;
        };
        if site.context().root() != SqlIslandRoot::Expression {
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
    hash.update(avenger_lang_compiler::SQL_SEMANTIC_PROFILE.as_bytes());
    hash.update(b"\0");
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
    project_at_owner(origin, cursor, roots, contexts, &[])
}

fn project_at_owner<'a>(
    origin: &SourceOrigin,
    cursor: usize,
    roots: &'a BTreeMap<String, RootAnalysis>,
    contexts: &'a BTreeMap<SourceOrigin, Vec<DatasetContext>>,
    current_owner_path: &[crate::DatasetContextOwner],
) -> (Option<&'a ModuleAnalysis>, Option<&'a DatasetContext>) {
    let source_contexts = contexts
        .iter()
        .find(|(candidate, _)| same_origin(candidate, origin))
        .map(|(_, contexts)| contexts);
    let context = source_contexts
        .and_then(|contexts| {
            (!current_owner_path.is_empty())
                .then(|| {
                    contexts
                        .iter()
                        .filter(|context| context.owner_path == current_owner_path)
                        .min_by_key(|context| {
                            (
                                context.span.range.len(),
                                std::cmp::Reverse(context.stage.ordinal),
                            )
                        })
                })
                .flatten()
        })
        .or_else(|| {
            source_contexts.and_then(|contexts| {
                contexts
                    .iter()
                    .filter(|context| {
                        context.span.range.start <= cursor && cursor <= context.span.range.end
                    })
                    .min_by_key(|context| context.span.range.len())
            })
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

fn projection_expression_mode(
    index: &WorkspaceSemanticIndex,
    registry: &NativeSchemaSnapshot,
    origin: &SourceOrigin,
    cursor: usize,
    island: SourceSpan,
) -> Option<ProjectionExpressionMode> {
    let document = index.documents.get(origin)?;
    let descriptor = document
        .sql_islands
        .iter()
        .find(|descriptor| descriptor.span == island)?;
    if descriptor.site.context().root() != SqlIslandRoot::Projection {
        return None;
    }
    let property = descriptor.property_path.last()?;
    let owner = crate::intelligence::owner_symbol(index, origin, cursor)?;
    let schema = crate::intelligence::schema_for_symbol(registry, owner, index)?;
    let property = crate::intelligence::property_schema(schema, property)?;
    match property.shape {
        ValueShape::SqlProjection {
            expression_mode, ..
        } => Some(*expression_mode),
        _ => None,
    }
}

#[derive(Clone, Debug)]
enum ExpectedCompletionType {
    Boolean,
    Integer,
    Number,
    String,
    Arrow(String),
}

impl ExpectedCompletionType {
    fn accepts(&self, data_type: &str) -> bool {
        let data_type = data_type.to_ascii_lowercase();
        match self {
            Self::Boolean => data_type == "boolean",
            Self::Integer => data_type.starts_with("int") || data_type.starts_with("uint"),
            Self::Number => {
                data_type.starts_with("int")
                    || data_type.starts_with("uint")
                    || data_type.starts_with("float")
                    || data_type.starts_with("decimal")
            }
            Self::String => {
                data_type == "utf8" || data_type == "largeutf8" || data_type == "utf8view"
            }
            Self::Arrow(expected) => expected.eq_ignore_ascii_case(&data_type),
        }
    }
}

fn expected_completion_type(
    index: &WorkspaceSemanticIndex,
    registry: &NativeSchemaSnapshot,
    origin: &SourceOrigin,
    cursor: usize,
    island: SourceSpan,
) -> Option<ExpectedCompletionType> {
    let document = index.documents.get(origin)?;
    let descriptor = document
        .sql_islands
        .iter()
        .find(|descriptor| descriptor.span == island)?;
    let first = descriptor.property_path.first()?;
    let owner = crate::intelligence::owner_symbol(index, origin, cursor)?;
    let schema = crate::intelligence::schema_for_symbol(registry, owner, index)?;
    if let Some(channel) = schema.channels.get(first)
        && descriptor.channel_mode.as_deref() == Some("direct")
    {
        return channel
            .item_type
            .as_ref()
            .map(|value| ExpectedCompletionType::Arrow(value.clone()));
    }
    let property = crate::intelligence::property_schema(schema, first)?;
    let mut shape = property.shape;
    for name in descriptor.property_path.iter().skip(1) {
        shape = match shape {
            ValueShape::Object(properties) | ValueShape::ConfiguredExpression(properties) => {
                &properties.get(name)?.shape
            }
            ValueShape::ConfiguredReference { properties, .. } => &properties.get(name)?.shape,
            ValueShape::Array(element)
            | ValueShape::OneOrMany(element)
            | ValueShape::Map(element) => element,
            _ => return None,
        };
    }
    if descriptor.site == avenger_lang_core::syntax::SqlIslandSite::ArrayElement {
        while let ValueShape::Array(element) | ValueShape::OneOrMany(element) = shape {
            shape = element;
        }
    }
    match shape {
        ValueShape::Boolean => Some(ExpectedCompletionType::Boolean),
        ValueShape::Integer => Some(ExpectedCompletionType::Integer),
        ValueShape::Number | ValueShape::RasterDimension => Some(ExpectedCompletionType::Number),
        ValueShape::String | ValueShape::Identifier => Some(ExpectedCompletionType::String),
        _ => None,
    }
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
    island_tokens_from_stream(&syntax.parsed.tokens, island, None)
}

fn patched_completion_token_stream(
    syntax: &SyntaxAnalysis,
    island: SourceSpan,
    cursor: usize,
    context: &SqlCursorContext,
) -> Option<LosslessTokenStream> {
    let closer = context.synthetic_closer.as_deref()?;
    if context.mode != SqlLexicalMode::DoubleQuotedIdentifier {
        return None;
    }
    let text = syntax.parsed.tokens.text();
    if cursor < island.range.start || cursor > island.range.end || cursor > text.len() {
        return None;
    }
    let mut patched = String::with_capacity(text.len() + closer.len());
    patched.push_str(&text[..cursor]);
    patched.push_str(closer);
    patched.push_str(&text[cursor..]);
    let source = SourceFile::new(
        syntax.parsed.tokens.source(),
        SourceOrigin::Memory("completion-patched-token-view".to_owned()),
        patched,
    );
    Some(tokenize_lossless(&source))
}

fn island_tokens_from_stream<'a>(
    stream: &'a LosslessTokenStream,
    island: SourceSpan,
    synthetic_insertion: Option<(usize, usize)>,
) -> Vec<SqlToken<'a>> {
    let mut depth = 0_usize;
    let patched_end = island
        .range
        .end
        .saturating_add(synthetic_insertion.map_or(0, |(_, length)| length));
    stream
        .tokens()
        .iter()
        .filter(|token| {
            island.range.start <= token.span().range.start
                && token.span().range.end <= patched_end
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
            let map_offset = |offset: usize| {
                synthetic_insertion.map_or(offset, |(synthetic, length)| {
                    offset.saturating_sub(offset.saturating_sub(synthetic).min(length))
                })
            };
            let output = SqlToken {
                token: token.token(),
                raw: stream.raw(token),
                span: SourceSpan {
                    source: island.source,
                    range: ByteSpan {
                        start: map_offset(token.span().range.start),
                        end: map_offset(token.span().range.end),
                    },
                },
                depth,
            };
            if matches!(token.token(), Some(Token::LParen | Token::LBracket)) {
                depth = depth.saturating_add(1);
            }
            output
        })
        .collect()
}

fn truncate_at_outer_delimiter(
    tokens: &mut Vec<SqlToken<'_>>,
    site: avenger_lang_core::syntax::SqlIslandSite,
    cursor: usize,
) {
    let delimiters = site.outer_delimiters();
    if let Some(index) = tokens.iter().position(|token| {
        token.depth == 0
            && token.span.range.start >= cursor
            && delimiters.iter().any(|delimiter| {
                if delimiter.eq_ignore_ascii_case("as") {
                    token.is_word("as")
                } else {
                    token.raw == *delimiter
                }
            })
    }) {
        tokens.truncate(index);
    }
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

fn sql_cursor_context(text: &str, island: SourceSpan, cursor: usize) -> SqlCursorContext {
    #[derive(Clone, Debug)]
    enum ScanState {
        Code,
        Single { start: usize },
        Double { start: usize },
        Dollar { delimiter: String },
        LineComment,
        BlockComment { depth: usize },
    }

    let cursor = cursor.min(island.range.end).min(text.len());
    let bytes = text.as_bytes();
    let mut state = ScanState::Code;
    let mut offset = island.range.start;
    while offset < cursor {
        match &mut state {
            ScanState::Code => {
                if bytes.get(offset..offset + 2) == Some(b"--") {
                    state = ScanState::LineComment;
                    offset += 2;
                } else if bytes.get(offset..offset + 2) == Some(b"/*") {
                    state = ScanState::BlockComment { depth: 1 };
                    offset += 2;
                } else if bytes[offset] == b'\'' {
                    state = ScanState::Single { start: offset };
                    offset += 1;
                } else if bytes[offset] == b'"' {
                    state = ScanState::Double { start: offset };
                    offset += 1;
                } else if bytes[offset] == b'$' {
                    if let Some((delimiter, end)) = dollar_quote_opener(text, offset, cursor) {
                        state = ScanState::Dollar { delimiter };
                        offset = end;
                    } else {
                        offset += 1;
                    }
                } else {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                }
            }
            ScanState::Single { .. } => {
                if bytes[offset] == b'\'' {
                    if bytes.get(offset + 1) == Some(&b'\'') && offset + 1 < cursor {
                        offset += 2;
                    } else {
                        state = ScanState::Code;
                        offset += 1;
                    }
                } else {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                }
            }
            ScanState::Double { .. } => {
                if bytes[offset] == b'"' {
                    if bytes.get(offset + 1) == Some(&b'"') && offset + 1 < cursor {
                        offset += 2;
                    } else {
                        state = ScanState::Code;
                        offset += 1;
                    }
                } else {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                }
            }
            ScanState::Dollar { delimiter } => {
                if text[offset..].starts_with(delimiter.as_str()) {
                    offset += delimiter.len();
                    state = ScanState::Code;
                } else {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                }
            }
            ScanState::LineComment => {
                if matches!(bytes[offset], b'\n' | b'\r') {
                    state = ScanState::Code;
                }
                offset += 1;
            }
            ScanState::BlockComment { depth } => {
                if bytes.get(offset..offset + 2) == Some(b"/*") {
                    *depth += 1;
                    offset += 2;
                } else if bytes.get(offset..offset + 2) == Some(b"*/") {
                    *depth -= 1;
                    offset += 2;
                    if *depth == 0 {
                        state = ScanState::Code;
                    }
                } else {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                }
            }
        }
    }

    match state {
        ScanState::Double { start } => {
            let end = quoted_identifier_end(text, cursor, island.range.end);
            let authored = &text[start + 1..cursor];
            SqlCursorContext {
                mode: SqlLexicalMode::DoubleQuotedIdentifier,
                replacement: SourceSpan {
                    source: island.source,
                    range: ByteSpan { start, end },
                },
                decoded_prefix: authored.replace("\"\"", "\""),
                synthetic_closer: (end == cursor).then(|| "\"".to_owned()),
            }
        }
        ScanState::Single { start } => {
            let end = single_quoted_end(text, cursor, island.range.end);
            SqlCursorContext {
                mode: SqlLexicalMode::SingleQuotedString,
                replacement: SourceSpan {
                    source: island.source,
                    range: ByteSpan { start, end },
                },
                decoded_prefix: String::new(),
                synthetic_closer: (end == cursor).then(|| "'".to_owned()),
            }
        }
        ScanState::Dollar { delimiter } => SqlCursorContext {
            mode: SqlLexicalMode::DollarQuotedString,
            replacement: SourceSpan::empty(island.source, cursor),
            decoded_prefix: String::new(),
            synthetic_closer: (!text[cursor..island.range.end.min(text.len())]
                .contains(&delimiter))
            .then_some(delimiter),
        },
        ScanState::LineComment => SqlCursorContext {
            mode: SqlLexicalMode::LineComment,
            replacement: SourceSpan::empty(island.source, cursor),
            decoded_prefix: String::new(),
            synthetic_closer: None,
        },
        ScanState::BlockComment { depth } => SqlCursorContext {
            mode: SqlLexicalMode::BlockComment,
            replacement: SourceSpan::empty(island.source, cursor),
            decoded_prefix: String::new(),
            synthetic_closer: (!text[cursor..island.range.end.min(text.len())].contains("*/"))
                .then(|| "*/".repeat(depth)),
        },
        ScanState::Code => {
            let replacement = sql_word_replacement_span(text, island, cursor);
            let authored = &text[replacement.range.start..cursor];
            let mode = if authored.starts_with('$') && authored.contains('@') {
                SqlLexicalMode::TemporalQualifier
            } else if authored.starts_with('$') {
                SqlLexicalMode::Binding
            } else {
                SqlLexicalMode::Code
            };
            SqlCursorContext {
                mode,
                replacement,
                decoded_prefix: authored.to_owned(),
                synthetic_closer: None,
            }
        }
    }
}

fn single_quoted_end(text: &str, cursor: usize, limit: usize) -> usize {
    let bytes = text.as_bytes();
    let mut end = cursor;
    while end < limit.min(text.len()) {
        if bytes[end] == b'\'' {
            if bytes.get(end + 1) == Some(&b'\'') && end + 1 < limit {
                end += 2;
            } else {
                return end + 1;
            }
        } else {
            end += text[end..].chars().next().map_or(1, char::len_utf8);
        }
    }
    cursor
}

fn sql_word_replacement_span(text: &str, island: SourceSpan, cursor: usize) -> SourceSpan {
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

fn dollar_quote_opener(text: &str, start: usize, limit: usize) -> Option<(String, usize)> {
    let bytes = text.as_bytes();
    if bytes.get(start) != Some(&b'$') {
        return None;
    }
    let mut end = start + 1;
    while end < limit {
        match bytes[end] {
            b'$' => {
                let delimiter = text[start..=end].to_owned();
                return Some((delimiter, end + 1));
            }
            byte if byte == b'_' || byte.is_ascii_alphanumeric() => end += 1,
            _ => return None,
        }
    }
    None
}

fn quoted_identifier_end(text: &str, cursor: usize, limit: usize) -> usize {
    let bytes = text.as_bytes();
    let mut end = cursor;
    while end < limit.min(text.len()) {
        if matches!(bytes[end], b'\n' | b'\r') {
            // Canonical authored column identifiers do not span lines. During
            // editing, treating a later quote in the following SQL/DSL as the
            // closer would swallow the suffix and destroy FROM-first scope.
            return cursor;
        }
        if bytes[end].is_ascii_whitespace() && quoted_suffix_starts_sql_clause(text, end, limit) {
            // An unterminated identifier before an intact same-line clause is
            // another common editor state. A quote in that clause (for
            // example EXCLUDE ("id")) belongs to the suffix, not to the
            // identifier currently being completed.
            return cursor;
        }
        if bytes[end] == b'"' {
            if bytes.get(end + 1) == Some(&b'"') && end + 1 < limit {
                end += 2;
            } else {
                return end + 1;
            }
        } else {
            end += text[end..].chars().next().map_or(1, char::len_utf8);
        }
    }
    cursor
}

fn quoted_suffix_starts_sql_clause(text: &str, whitespace: usize, limit: usize) -> bool {
    let limit = limit.min(text.len());
    let mut start = whitespace;
    while start < limit && text.as_bytes()[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = start;
    while end < limit
        && (text.as_bytes()[end].is_ascii_alphanumeric() || text.as_bytes()[end] == b'_')
    {
        end += 1;
    }
    if start == end || end >= limit || text.as_bytes()[end] == b'"' {
        return false;
    }
    let word = &text[start..end];
    is_clause_word(word)
        || ["as", "by", "window", "filter", "over"]
            .iter()
            .any(|candidate| word.eq_ignore_ascii_case(candidate))
}

fn expected_intents(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    context: SqlIslandContext,
    prefix: &str,
    skeleton: &QuerySkeleton,
) -> BTreeSet<SqlIntent> {
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
    let mut intents = BTreeSet::new();
    if previous.is_some_and(SqlToken::is_period) {
        intents.insert(SqlIntent::QualifiedMember);
        intents.insert(SqlIntent::ExpressionOperand);
        if before.is_some_and(|index| qualifier_is_relation_path(tokens, index)) {
            intents.insert(SqlIntent::RelationPath);
        }
        return intents;
    }
    if type_position(tokens, structural_before) {
        intents.insert(SqlIntent::TypeName);
        return intents;
    }
    if context.root() == SqlIslandRoot::Query
        && let Some(state) = cte_header_state(tokens, cursor, prefix)
    {
        intents.insert(match state {
            CteHeaderState::Name => SqlIntent::CteName,
            CteHeaderState::AfterAs => SqlIntent::CteBodyStart,
            CteHeaderState::RecursiveKeyword
            | CteHeaderState::AfterName
            | CteHeaderState::BodyComplete => SqlIntent::ClauseTransition,
        });
        return intents;
    }
    if structural_before
        .and_then(|index| tokens.get(index))
        .is_some_and(|token| token.is_word("as"))
    {
        intents.insert(SqlIntent::NameBinder);
        return intents;
    }
    if relation_position(tokens, structural_before, cursor) {
        intents.insert(SqlIntent::RelationPath);
        if prefix.starts_with('$') {
            intents.insert(SqlIntent::Binding);
        }
    } else {
        let clause = skeleton.active_clause(tokens, cursor);
        if context.root() == SqlIslandRoot::Query
            && matches!(
                clause,
                QueryClause::Start | QueryClause::With | QueryClause::SetOperation
            )
        {
            intents.insert(if clause == QueryClause::Start {
                SqlIntent::QueryStart
            } else {
                SqlIntent::ClauseTransition
            });
        } else if expression_operand_position(tokens, structural_before, clause) {
            intents.insert(SqlIntent::ExpressionOperand);
            intents.insert(SqlIntent::FunctionName);
            if prefix.starts_with('$') {
                intents.insert(SqlIntent::Binding);
            }
            if context.root() == SqlIslandRoot::Query {
                intents.insert(SqlIntent::ClauseTransition);
            }
        } else {
            intents.insert(SqlIntent::ExpressionOperator);
            if context.root() == SqlIslandRoot::Query {
                intents.insert(SqlIntent::ClauseTransition);
            }
        }
    }
    if intents.is_empty() {
        intents.insert(SqlIntent::Nothing);
    }
    intents
}

fn expression_operand_position(
    tokens: &[SqlToken<'_>],
    before: Option<usize>,
    clause: QueryClause,
) -> bool {
    let Some(index) = before else {
        return true;
    };
    let token = &tokens[index];
    if token.is_comma()
        || matches!(token.token, Some(Token::LParen | Token::LBracket))
        || matches!(
            token.raw,
            "+" | "-"
                | "*"
                | "/"
                | "%"
                | "="
                | "=="
                | "!="
                | "<>"
                | "<"
                | ">"
                | "<="
                | ">="
                | "||"
                | "::"
        )
    {
        return true;
    }
    if [
        "select", "where", "on", "using", "having", "qualify", "by", "when", "then", "else", "and",
        "or", "not", "between", "in", "like", "ilike", "rlike",
    ]
    .iter()
    .any(|word| token.is_word(word))
    {
        return true;
    }
    matches!(
        clause,
        QueryClause::Limit | QueryClause::Offset | QueryClause::Fetch
    ) && token.is_word(clause.name())
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
    intents: &BTreeSet<SqlIntent>,
) -> (SqlRepairStrategy, usize) {
    let previous = tokens.iter().rfind(|token| token.span.range.end <= cursor);
    let candidates = [
        intents
            .contains(&SqlIntent::QualifiedMember)
            .then_some(SqlRepairStrategy::QualifierMember),
        previous
            .is_some_and(SqlToken::is_comma)
            .then_some(SqlRepairStrategy::TrailingComma),
        intents
            .contains(&SqlIntent::RelationPath)
            .then_some(SqlRepairStrategy::MissingRelation),
        intents
            .contains(&SqlIntent::ExpressionOperand)
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
    intents: &BTreeSet<SqlIntent>,
    replacement: Range<usize>,
) -> RecoveryResult {
    let sentinel = cursor_sentinel(intents, root);
    let mut strategies = vec![SqlRepairStrategy::None];
    let preferred = select_repair(tokens, absolute_cursor, intents).0;
    if preferred != SqlRepairStrategy::None {
        strategies.push(preferred);
    }
    let previous = tokens
        .iter()
        .rfind(|token| token.span.range.end <= absolute_cursor);
    for strategy in [
        intents
            .contains(&SqlIntent::QualifiedMember)
            .then_some(SqlRepairStrategy::QualifierMember),
        previous
            .is_some_and(SqlToken::is_comma)
            .then_some(SqlRepairStrategy::TrailingComma),
        intents
            .contains(&SqlIntent::RelationPath)
            .then_some(SqlRepairStrategy::MissingRelation),
        intents
            .contains(&SqlIntent::TypeName)
            .then_some(SqlRepairStrategy::MissingType),
        (root == SqlIslandRoot::Query
            && intents.contains(&SqlIntent::QueryStart)
            && tokens
                .iter()
                .all(|token| token.span.range.start >= absolute_cursor))
        .then_some(SqlRepairStrategy::QueryStart),
        intents
            .contains(&SqlIntent::ExpressionOperand)
            .then_some(SqlRepairStrategy::EmptyExpression),
        Some(SqlRepairStrategy::CloseDelimiters),
        Some(SqlRepairStrategy::ParsablePrefix),
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
        let authored_start = if strategy == SqlRepairStrategy::ParsablePrefix {
            tokens
                .iter()
                .filter(|token| token.span.range.end <= absolute_cursor)
                .rev()
                .take(MAX_PREFIX_FALLBACK_TOKENS)
                .last()
                .map_or(0, |token| {
                    token
                        .span
                        .range
                        .start
                        .saturating_sub(absolute_cursor.saturating_sub(cursor))
                })
        } else {
            0
        };
        let repaired = build_repaired_sql(
            authored,
            replacement.clone(),
            strategy,
            sentinel,
            authored_start,
        );
        let parsed = match root {
            SqlIslandRoot::Query => SqlQuery::parse(&repaired.text).is_ok(),
            SqlIslandRoot::Projection => SqlProjection::parse(&repaired.text).is_ok(),
            SqlIslandRoot::Expression => SqlExpression::parse(&repaired.text).is_ok(),
        };
        fallback = Some(RecoveryResult {
            strategy,
            sentinel,
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
        sentinel,
        attempts: 0,
        repaired: build_repaired_sql(authored, replacement, SqlRepairStrategy::None, sentinel, 0),
        parsed: false,
    })
}

fn cursor_sentinel(
    intents: &BTreeSet<SqlIntent>,
    root: SqlIslandRoot,
) -> Option<SqlCursorSentinelKind> {
    if intents.contains(&SqlIntent::TypeName) {
        Some(SqlCursorSentinelKind::Type)
    } else if intents.contains(&SqlIntent::QualifiedMember) {
        Some(SqlCursorSentinelKind::QuotedMember)
    } else if intents.contains(&SqlIntent::RelationPath) {
        Some(SqlCursorSentinelKind::Relation)
    } else if root == SqlIslandRoot::Query && intents.contains(&SqlIntent::QueryStart) {
        Some(SqlCursorSentinelKind::QueryStart)
    } else if intents.contains(&SqlIntent::ExpressionOperand)
        || intents.contains(&SqlIntent::FunctionName)
        || intents.contains(&SqlIntent::ExpressionOperator)
    {
        Some(SqlCursorSentinelKind::Expression)
    } else {
        None
    }
}

fn sentinel_text(sentinel: Option<SqlCursorSentinelKind>) -> &'static str {
    match sentinel {
        Some(SqlCursorSentinelKind::Expression) => "NULL",
        Some(SqlCursorSentinelKind::QuotedMember) => "\"__avenger_cursor_member\"",
        Some(SqlCursorSentinelKind::Relation) => "__avenger_cursor_relation",
        Some(SqlCursorSentinelKind::Type) => "BIGINT",
        Some(SqlCursorSentinelKind::QueryStart) => "SELECT NULL",
        None => "",
    }
}

fn sentinel_for_repair(strategy: SqlRepairStrategy) -> Option<SqlCursorSentinelKind> {
    match strategy {
        SqlRepairStrategy::QualifierMember => Some(SqlCursorSentinelKind::QuotedMember),
        SqlRepairStrategy::MissingRelation => Some(SqlCursorSentinelKind::Relation),
        SqlRepairStrategy::MissingType => Some(SqlCursorSentinelKind::Type),
        SqlRepairStrategy::QueryStart => Some(SqlCursorSentinelKind::QueryStart),
        SqlRepairStrategy::EmptyExpression
        | SqlRepairStrategy::TrailingComma
        | SqlRepairStrategy::CloseDelimiters
        | SqlRepairStrategy::ParsablePrefix => Some(SqlCursorSentinelKind::Expression),
        SqlRepairStrategy::None => None,
    }
}

fn build_repaired_sql(
    authored: &str,
    replacement: Range<usize>,
    strategy: SqlRepairStrategy,
    sentinel: Option<SqlCursorSentinelKind>,
    authored_start: usize,
) -> RepairedSql {
    let authored_start = authored_start.min(authored.len());
    let start = replacement.start.clamp(authored_start, authored.len());
    let end = replacement.end.clamp(start, authored.len());
    let marker = match strategy {
        SqlRepairStrategy::QualifierMember
        | SqlRepairStrategy::EmptyExpression
        | SqlRepairStrategy::MissingRelation
        | SqlRepairStrategy::MissingType
        | SqlRepairStrategy::QueryStart
        | SqlRepairStrategy::TrailingComma
        | SqlRepairStrategy::CloseDelimiters
        | SqlRepairStrategy::ParsablePrefix => sentinel_text(sentinel),
        SqlRepairStrategy::None => "",
    };
    let keep_suffix = strategy != SqlRepairStrategy::ParsablePrefix;
    let mut text = String::with_capacity(authored.len() + marker.len() + 16);
    text.push_str(&authored[authored_start..start]);
    let mut generated = Vec::new();
    if !marker.is_empty() {
        let marker_start = text.len();
        generated.push(marker_start..marker_start + marker.len());
        text.push_str(marker);
    }
    if keep_suffix {
        text.push_str(&authored[end..]);
    }
    if matches!(
        strategy,
        SqlRepairStrategy::CloseDelimiters | SqlRepairStrategy::ParsablePrefix
    ) {
        let closers = minimum_sql_closers(&text);
        if !closers.is_empty() {
            let closer_start = text.len();
            text.push_str(&closers);
            generated.push(closer_start..text.len());
        }
    }
    RepairedSql {
        text,
        generated,
        authored_start,
    }
}

fn minimum_sql_closers(authored: &str) -> String {
    #[derive(Clone, Copy)]
    enum Delimiter {
        Paren,
        Bracket,
        Case,
    }

    let source = SourceFile::new(
        SourceId::new(u32::MAX),
        SourceOrigin::Memory("completion-minimum-closers.sql".to_owned()),
        authored.to_owned(),
    );
    let stream = tokenize_lossless(&source);
    let mut stack = Vec::new();
    for token in stream.tokens() {
        match token.token() {
            Some(Token::LParen) => stack.push(Delimiter::Paren),
            Some(Token::LBracket) => stack.push(Delimiter::Bracket),
            Some(Token::RParen) => {
                if matches!(stack.last(), Some(Delimiter::Paren)) {
                    stack.pop();
                }
            }
            Some(Token::RBracket) => {
                if matches!(stack.last(), Some(Delimiter::Bracket)) {
                    stack.pop();
                }
            }
            Some(Token::Word(word))
                if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("case") =>
            {
                stack.push(Delimiter::Case);
            }
            Some(Token::Word(word))
                if word.quote_style.is_none() && word.value.eq_ignore_ascii_case("end") =>
            {
                if let Some(case) = stack
                    .iter()
                    .rposition(|delimiter| matches!(delimiter, Delimiter::Case))
                {
                    stack.remove(case);
                }
            }
            _ => {}
        }
    }
    let mut output = String::new();
    for delimiter in stack.into_iter().rev() {
        match delimiter {
            Delimiter::Paren => output.push(')'),
            Delimiter::Bracket => output.push(']'),
            Delimiter::Case => output.push_str(" END"),
        }
    }
    output
}

fn relation_catalog(project: &ModuleAnalysis) -> Vec<RelationMetadata> {
    project
        .datasets
        .iter()
        .filter_map(|(stage, dataset)| {
            // Only authored catalog/schema/table paths belong in the public
            // relation namespace. Compiler-internal `chart:...` stage names
            // are implementation identities; pipeline sites expose their
            // effective relation as `input` instead.
            let path = dataset.qualified_path.clone()?;
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
    root: SqlIslandRoot,
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
    let implicit_input = project
        .and_then(|project| input_dataset(project, context, cursor))
        .map(|input| {
            let stage = &input.stage;
            RelationMetadata {
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
            }
        });
    if let Some(input) = &implicit_input {
        available.push(input.clone());
    }
    let clause = query_clause_at(tokens, cursor);
    let (arm_start, arm_end) = active_set_arm(tokens, cursor, active_depth);
    let parent_policy = parent_scope_policy(tokens, cursor, active_depth);

    for (index, token) in tokens.iter().enumerate() {
        if token.depth > active_depth
            || (token.depth == active_depth && !(arm_start..arm_end).contains(&index))
            || !token_in_cursor_scope(tokens, index, cursor)
            || (!matches!(clause, QueryClause::Select) && token.span.range.start > cursor)
            || !(token.is_word("from") || token.is_word("join"))
        {
            continue;
        }
        if token.depth < active_depth {
            match parent_policy {
                ParentScopePolicy::IsolatedDerived => continue,
                ParentScopePolicy::Lateral { boundary } if token.span.range.start >= boundary => {
                    continue;
                }
                ParentScopePolicy::Correlated | ParentScopePolicy::Lateral { .. } => {}
            }
        }
        if let Some(relation) = relation_after(tokens, index + 1, token.depth, &available) {
            scope.relations.push(relation);
        }
    }

    if root != SqlIslandRoot::Query
        && let Some(input) = implicit_input
    {
        scope.relations.push(input);
    }
    deduplicate_relations(&mut scope.relations);
    scope.projection_aliases = projection_aliases(tokens, active_depth, &scope.relations);
    scope.projection_aliases_visible = projection_aliases_visible(tokens, cursor, active_depth);
    scope.merged_column_reductions =
        joined_column_reductions(tokens, active_depth, arm_start, arm_end, &scope.relations);
    if clause == QueryClause::Using {
        let active = scope
            .relations
            .iter()
            .filter(|relation| relation.scope_depth == active_depth)
            .collect::<Vec<_>>();
        let intersection = active
            .split_last()
            .map_or_else(BTreeSet::new, |(newest, prior)| {
                newest
                    .columns
                    .iter()
                    .map(|column| column.name.to_ascii_lowercase())
                    .filter(|name| {
                        prior.iter().any(|relation| {
                            relation
                                .columns
                                .iter()
                                .any(|column| column.name.eq_ignore_ascii_case(name))
                        })
                    })
                    .collect::<BTreeSet<_>>()
            });
        for relation in &mut scope.relations {
            relation
                .columns
                .retain(|column| intersection.contains(&column.name.to_ascii_lowercase()));
        }
    }
    scope
}

fn parent_scope_policy(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    active_depth: usize,
) -> ParentScopePolicy {
    let Some(open) = active_open_paren(tokens, cursor, active_depth) else {
        return ParentScopePolicy::Correlated;
    };
    let open_token = &tokens[open];
    let previous = tokens[..open]
        .iter()
        .rev()
        .find(|token| token.depth == open_token.depth);
    if previous.is_some_and(|token| token.is_word("lateral")) {
        return ParentScopePolicy::Lateral {
            boundary: open_token.span.range.start,
        };
    }
    if previous
        .is_some_and(|token| token.is_word("from") || token.is_word("join") || token.is_comma())
    {
        ParentScopePolicy::IsolatedDerived
    } else {
        ParentScopePolicy::Correlated
    }
}

fn joined_column_reductions(
    tokens: &[SqlToken<'_>],
    depth: usize,
    arm_start: usize,
    arm_end: usize,
    relations: &[RelationMetadata],
) -> BTreeMap<String, usize> {
    let mut reductions = BTreeMap::<String, usize>::new();
    let arm_end = arm_end.min(tokens.len());
    let arm_start = arm_start.min(arm_end);
    let arm = &tokens[arm_start..arm_end];
    for (offset, token) in arm.iter().enumerate() {
        if token.depth != depth || !token.is_word("using") {
            continue;
        }
        let Some(open) = tokens
            .get(arm_start + offset + 1)
            .filter(|token| matches!(token.token, Some(Token::LParen)))
            .map(|_| arm_start + offset + 1)
        else {
            continue;
        };
        let close = matching_close(tokens, open).unwrap_or(arm_end);
        for name in tokens[open + 1..close.min(tokens.len())]
            .iter()
            .filter_map(SqlToken::word)
            .map(str::to_ascii_lowercase)
        {
            *reductions.entry(name).or_default() += 1;
        }
    }

    let active_relations = relations
        .iter()
        .filter(|relation| relation.scope_depth == depth)
        .collect::<Vec<_>>();
    let mut left_columns = active_relations
        .first()
        .map(|relation| {
            relation
                .columns
                .iter()
                .map(|column| column.name.to_ascii_lowercase())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let join_indices = arm
        .iter()
        .enumerate()
        .filter(|(_, token)| token.depth == depth && token.is_word("join"))
        .map(|(offset, _)| arm_start + offset)
        .collect::<Vec<_>>();
    for (ordinal, join) in join_indices.into_iter().enumerate() {
        let Some(right) = active_relations.get(ordinal + 1) else {
            break;
        };
        let boundary = tokens[..join]
            .iter()
            .rposition(|token| {
                token.depth == depth
                    && (token.is_word("from") || token.is_word("join") || token.is_comma())
            })
            .map_or(arm_start, |index| index + 1);
        let natural = tokens[boundary..join]
            .iter()
            .any(|token| token.depth == depth && token.is_word("natural"));
        let right_columns = right
            .columns
            .iter()
            .map(|column| column.name.to_ascii_lowercase())
            .collect::<BTreeSet<_>>();
        if natural {
            for name in left_columns.intersection(&right_columns) {
                *reductions.entry(name.clone()).or_default() += 1;
            }
        }
        left_columns.extend(right_columns);
    }
    reductions
}

fn active_set_arm(tokens: &[SqlToken<'_>], cursor: usize, depth: usize) -> (usize, usize) {
    let mut start = 0;
    let mut end = tokens.len();
    for (index, token) in tokens.iter().enumerate() {
        if token.depth != depth
            || !(token.is_word("union") || token.is_word("except") || token.is_word("intersect"))
        {
            continue;
        }
        if token.span.range.end <= cursor {
            start = index + 1;
        } else if token.span.range.start >= cursor {
            end = index;
            break;
        }
    }
    (start, end)
}

fn token_in_cursor_scope(tokens: &[SqlToken<'_>], index: usize, cursor: usize) -> bool {
    let Some(token) = tokens.get(index) else {
        return false;
    };
    if token.depth == 0 {
        return true;
    }
    tokens[..index]
        .iter()
        .enumerate()
        .rev()
        .any(|(open, candidate)| {
            candidate.depth + 1 == token.depth
                && matches!(candidate.token, Some(Token::LParen))
                && candidate.span.range.start < cursor
                && matching_close(tokens, open)
                    .is_none_or(|close| tokens[close].span.range.end >= cursor)
        })
}

/// Replace heuristic projection metadata with DataFusion's qualified output
/// schema whenever the current query is strict-valid and plans against the
/// published schema-only catalog. Recovery remains useful for incomplete
/// queries, but it never overrides a successful planner result.
fn reconcile_query_output_with_datafusion(
    authored: &str,
    catalog: &[RelationMetadata],
    scope: &mut QueryScope,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    island_start: usize,
) {
    let context = SessionContext::new();
    for relation in catalog.iter().chain(scope.relations.iter()) {
        let _ = register_schema_only_relation(&context, relation);
    }
    let mut probes = vec![authored];
    let (clause, clause_index) = query_clause_and_index(tokens, cursor);
    if matches!(
        clause,
        QueryClause::Where
            | QueryClause::Having
            | QueryClause::Qualify
            | QueryClause::OrderBy
            | QueryClause::Limit
            | QueryClause::Offset
    ) && let Some(index) = clause_index
        && let Some(end) = tokens[index].span.range.start.checked_sub(island_start)
        && end <= authored.len()
    {
        probes.push(authored[..end].trim_end());
    }

    for probe in probes {
        let Ok(query) = SqlQuery::parse(probe) else {
            continue;
        };
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

        let Ok(canonical) = avenger_lang_compiler::normalize_sql_query(&query.ast().to_string())
        else {
            continue;
        };
        let Ok(plan) = futures::executor::block_on(context.state().create_logical_plan(&canonical))
        else {
            continue;
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
        return;
    }
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
    let recursive = tokens
        .get(index)
        .is_some_and(|token| token.is_word("recursive"));
    if recursive {
        index += 1;
    }
    while index < tokens.len() {
        let Some(name) = tokens.get(index).and_then(SqlToken::word) else {
            break;
        };
        if tokens[index].span.range.start > cursor {
            break;
        }
        let mut explicit_names = Vec::new();
        let search_start = if tokens
            .get(index + 1)
            .is_some_and(|token| matches!(token.token, Some(Token::LParen)))
        {
            let names_open = index + 1;
            let Some(names_close) = matching_close(tokens, names_open) else {
                break;
            };
            explicit_names.extend(
                tokens[names_open + 1..names_close]
                    .iter()
                    .filter_map(SqlToken::word)
                    .map(str::to_owned),
            );
            names_close + 1
        } else {
            index + 1
        };
        let Some(as_index) = (search_start..tokens.len()).find(|candidate| {
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
        let cursor_in_body = open.span.range.start < cursor
            && (close >= tokens.len() || cursor <= tokens[close].span.range.end);
        if cursor_in_body && !recursive {
            // A non-recursive CTE is not in scope within its own body. Prior
            // CTEs remain visible, while later declarations never do.
            break;
        }
        let mut relations = catalog.to_vec();
        relations.extend(output.clone());
        let body = &tokens[as_index + 2..close.min(tokens.len())];
        let seed_columns = projection_aliases(body, open.depth + 1, &relations);
        if recursive {
            let cte_name = name.to_owned();
            let declared_columns = if explicit_names.is_empty() {
                seed_columns
                    .iter()
                    .cloned()
                    .map(|mut column| {
                        column.qualifier = Some(cte_name.clone());
                        column
                    })
                    .collect()
            } else {
                explicit_names
                    .iter()
                    .enumerate()
                    .map(|(ordinal, name)| {
                        let mut column =
                            seed_columns
                                .get(ordinal)
                                .cloned()
                                .unwrap_or_else(|| ColumnMetadata {
                                    name: name.clone(),
                                    qualifier: Some(cte_name.clone()),
                                    data_type: DataType::Null,
                                    nullable: true,
                                    stage: "recursive CTE declaration".to_owned(),
                                    lineage: None,
                                    detail: Some(
                                        "recursive CTE output type pending planning".to_owned(),
                                    ),
                                });
                        column.name.clone_from(name);
                        column.qualifier = Some(cte_name.clone());
                        column
                    })
                    .collect()
            };
            relations.push(RelationMetadata {
                path: vec![cte_name.clone()],
                alias: None,
                columns: declared_columns,
                detail: "recursive common table expression".to_owned(),
                scope_depth: tokens[index].depth,
            });
        }
        let inferred_columns = projection_aliases(body, open.depth + 1, &relations);
        let columns = if explicit_names.is_empty() {
            inferred_columns
        } else {
            explicit_names
                .iter()
                .enumerate()
                .map(|(ordinal, explicit_name)| {
                    let mut column =
                        inferred_columns
                            .get(ordinal)
                            .cloned()
                            .unwrap_or_else(|| ColumnMetadata {
                                name: explicit_name.clone(),
                                qualifier: Some(name.to_owned()),
                                data_type: DataType::Null,
                                nullable: true,
                                stage: "explicit CTE output declaration".to_owned(),
                                lineage: None,
                                detail: Some("CTE output type pending planning".to_owned()),
                            });
                    column.name.clone_from(explicit_name);
                    column.qualifier = Some(name.to_owned());
                    column
                })
                .collect()
        };
        output.push(RelationMetadata {
            path: vec![name.to_owned()],
            alias: None,
            columns,
            detail: "common table expression".to_owned(),
            scope_depth: tokens[index].depth,
        });
        if cursor_in_body {
            break;
        }
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
    let start = if tokens
        .get(start)
        .is_some_and(|token| token.is_word("lateral"))
    {
        start + 1
    } else {
        start
    };
    let first = tokens.get(start)?;
    if matches!(first.token, Some(Token::LParen)) {
        let close = matching_close(tokens, start).unwrap_or(tokens.len());
        let alias = alias_after(tokens, close.saturating_add(1), depth);
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
            index += 1;
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
            let mut expanded = Vec::new();
            if let Some(qualifier) = item
                .windows(2)
                .find(|pair| pair[1].is_period())
                .and_then(|pair| pair[0].word())
            {
                if let Some(relation) = relations
                    .iter()
                    .find(|relation| relation.visible_name().eq_ignore_ascii_case(qualifier))
                {
                    expanded.extend(relation.columns.clone());
                }
            } else {
                expanded.extend(
                    relations
                        .iter()
                        .flat_map(|relation| relation.columns.clone()),
                );
            }
            let excluded = wildcard_excluded_columns(item);
            expanded.retain(|column| !excluded.contains(&column.name.to_ascii_lowercase()));
            output.extend(expanded);
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

fn wildcard_excluded_columns(item: &[SqlToken<'_>]) -> BTreeSet<String> {
    let Some(start) = item
        .iter()
        .position(|token| token.is_word("exclude") || token.is_word("except"))
    else {
        return BTreeSet::new();
    };
    item[start + 1..]
        .iter()
        .take_while(|token| !token.is_word("replace") && !token.is_word("ilike"))
        .filter_map(SqlToken::word)
        .map(str::to_ascii_lowercase)
        .collect()
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
        let part = if text[..position].ends_with('"') {
            position -= 1;
            let quoted_end = position;
            let mut opening = None;
            while position > island.range.start {
                let character = text[..position].chars().next_back().unwrap();
                position -= character.len_utf8();
                if character == '"' {
                    if position > island.range.start && text[..position].ends_with('"') {
                        position -= 1;
                    } else {
                        opening = Some(position);
                        break;
                    }
                }
            }
            let opening = opening?;
            text[opening + 1..quoted_end].replace("\"\"", "\"")
        } else {
            while position > island.range.start {
                let character = text[..position].chars().next_back().unwrap();
                if character == '_' || character == '$' || character.is_alphanumeric() {
                    position -= character.len_utf8();
                } else {
                    break;
                }
            }
            text[position..end].to_owned()
        };
        if position == end {
            break;
        }
        parts.push(part);
        if position <= island.range.start || !text[..position].ends_with('.') {
            break;
        }
        position -= 1;
    }
    parts.reverse();
    (!parts.is_empty()).then(|| parts.join("."))
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
        complete_contextual_root_members(
            prefix,
            replacement,
            &[("channel", "Channels on the current mark.")],
            output,
        );
    }
    if event.is_some() {
        complete_contextual_root_members(
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
        complete_contextual_root_members(
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
            complete_contextual_root_members(
                prefix,
                replacement,
                &[(name, "Lexically scoped inline view.")],
                output,
            );
        }
    }
}

fn complete_contextual_root_members(
    prefix: &str,
    replacement: SourceSpan,
    members: &[(&str, &str)],
    output: &mut Vec<CompletionItem>,
) {
    let start = output.len();
    complete_fixed_members(prefix, replacement, members, output);
    for item in &mut output[start..] {
        item.semantic_kind = CompletionSemanticKind::ContextualRoot;
        item.semantic_identity = format!("ContextualRoot:{}", item.label);
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
            let mut item = candidate(
                name,
                name,
                replacement,
                CompletionKind::Property,
                Some((*detail).to_owned()),
                CompletionOrigin::AuthoringSchema,
                "00",
            );
            item.semantic_kind = CompletionSemanticKind::ContextualMember;
            item.semantic_identity = format!("ContextualMember:{name}");
            output.push(item);
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
            let mut item = candidate(
                &channel.name,
                &channel.name,
                replacement,
                CompletionKind::Property,
                Some(detail),
                CompletionOrigin::AuthoringSchema,
                "00",
            );
            item.semantic_kind = CompletionSemanticKind::ContextualMember;
            item.semantic_identity = format!("ContextualMember:{}", channel.name);
            item.data_type = channel.data_type.clone();
            item.nullable = channel.nullable;
            output.push(item);
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
    intents: &BTreeSet<SqlIntent>,
    scope: &QueryScope,
    catalog: &[RelationMetadata],
    document: Option<&DocumentSemanticIndex>,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    site: avenger_lang_core::syntax::SqlIslandSite,
    quoted_mode: bool,
    output: &mut Vec<CompletionItem>,
) -> bool {
    let parts = qualifier.split('.').collect::<Vec<_>>();
    if intents.contains(&SqlIntent::RelationPath) {
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
                let insert = if quoted_mode {
                    quote_identifier(member)
                } else {
                    member.clone()
                };
                let mut item = candidate(
                    member,
                    &insert,
                    replacement,
                    kind,
                    Some(relation.detail.clone()),
                    CompletionOrigin::Catalog,
                    "10",
                );
                if quoted_mode {
                    item.filter_text = Some(quote_identifier(member));
                }
                output.push(item);
            }
        }
        if !seen.is_empty() {
            return true;
        }
    }
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
    if !quoted_mode
        && (complete_resolved_output_handle_members(
            qualifier,
            prefix,
            replacement,
            project,
            origin,
            cursor,
            site,
            output,
        ) || complete_output_handle_members(
            qualifier,
            prefix,
            replacement,
            document,
            cursor,
            output,
        ))
    {
        return true;
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn complete_resolved_output_handle_members(
    qualifier: &str,
    prefix: &str,
    replacement: SourceSpan,
    project: Option<&ModuleAnalysis>,
    origin: &SourceOrigin,
    cursor: usize,
    site: avenger_lang_core::syntax::SqlIslandSite,
    output: &mut Vec<CompletionItem>,
) -> bool {
    if qualifier.contains('.') {
        return false;
    }
    let Some(resolved) = project.and_then(|project| project.resolved_module_graph.as_deref())
    else {
        return false;
    };
    let enclosing = declaration_path_at(project, origin, cursor)
        .into_iter()
        .next()
        .map(|declaration| {
            resolved
                .expansion_source_map
                .authored_span(declaration.span)
                .range
        });
    let mut candidates = Vec::new();
    for root in resolved
        .source_modules
        .values()
        .flat_map(|module| &module.roots)
    {
        collect_output_handle_declarations(
            root,
            resolved,
            origin,
            enclosing,
            qualifier,
            &mut candidates,
        );
    }
    let Some(declaration) = candidates.into_iter().max_by_key(|declaration| {
        resolved
            .expansion_source_map
            .authored_span(declaration.span)
            .range
            .start
    }) else {
        return false;
    };
    let declaration_visible = resolved
        .expansion_source_map
        .authored_span(declaration.span)
        .range
        .start
        < cursor;
    if !declaration_visible && site != avenger_lang_core::syntax::SqlIslandSite::OutputSource {
        // The qualifier is known, but ordinary expressions remain
        // stage-sequential. Treat the member position as resolved so generic
        // SQL keywords do not leak into the invalid future-alias access.
        return true;
    }
    for (name, handle) in &declaration.transform_outputs {
        if !candidate_matches(name, prefix) {
            continue;
        }
        let detail = match handle.shape {
            avenger_lang_core::ResolvedOutputShape::Expression => "transform output expression",
            avenger_lang_core::ResolvedOutputShape::RasterDimension => {
                "transform raster-dimension output"
            }
            avenger_lang_core::ResolvedOutputShape::Opaque => "opaque transform output",
        };
        let mut item = candidate(
            name,
            name,
            replacement,
            CompletionKind::Field,
            Some(format!("{detail} of `{qualifier}`")),
            CompletionOrigin::LexicalScope,
            "00",
        );
        item.semantic_kind = CompletionSemanticKind::ContextualMember;
        item.semantic_identity = format!(
            "OutputHandle:{}:{}:{}",
            handle.producer, handle.ordinal, handle.name
        );
        item.semantic_proximity = 100;
        output.push(item);
    }
    true
}

fn collect_output_handle_declarations<'a>(
    declaration: &'a ResolvedDeclaration,
    resolved: &avenger_lang_core::ResolvedModuleGraph,
    origin: &SourceOrigin,
    enclosing: Option<ByteSpan>,
    qualifier: &str,
    output: &mut Vec<&'a ResolvedDeclaration>,
) {
    let authored = resolved
        .expansion_source_map
        .authored_span(declaration.span);
    if let Some(source) = resolved.sources.get(authored.source)
        && same_origin(&source.origin, origin)
        && enclosing.is_none_or(|scope| {
            scope.start <= authored.range.start && authored.range.end <= scope.end
        })
        && declaration.keyword == "transform"
        && declaration
            .name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(qualifier))
        && !declaration.transform_outputs.is_empty()
    {
        output.push(declaration);
    }
    for child in &declaration.children {
        collect_output_handle_declarations(child, resolved, origin, enclosing, qualifier, output);
    }
}

fn complete_output_handle_members(
    qualifier: &str,
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) -> bool {
    let Some(document) = document else {
        return false;
    };
    let parts = qualifier.split('.').collect::<Vec<_>>();
    if parts.len() != 1 {
        return false;
    }
    let Some((parent_index, parent)) = document
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            symbol.keyword == "transform"
                && symbol.name.eq_ignore_ascii_case(parts[0])
                && symbol.selection_span.range.start < cursor
                && symbol_visible(document, symbol, cursor)
        })
        .max_by_key(|(_, symbol)| symbol.selection_span.range.start)
    else {
        return false;
    };
    for handle in document.symbols.iter().filter(|symbol| {
        symbol.parent == Some(parent_index)
            && symbol.value_kind == IndexedValueKind::Output
            && candidate_matches(&symbol.name, prefix)
    }) {
        let mut item = candidate(
            &handle.name,
            &handle.name,
            replacement,
            CompletionKind::Field,
            handle
                .detail
                .clone()
                .or_else(|| Some(format!("output handle of `{}`", parent.name))),
            CompletionOrigin::LexicalScope,
            "00",
        );
        item.semantic_kind = CompletionSemanticKind::ContextualMember;
        item.semantic_identity = format!("OutputHandle:{}:{}", parent.identity, handle.identity);
        item.documentation = handle.documentation.clone();
        item.semantic_proximity = 100;
        output.push(item);
    }
    true
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
            let mut item = candidate(
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
            );
            item.semantic_kind = CompletionSemanticKind::StructField;
            item.semantic_identity = format!("StructField:{qualifier}:{}", field.name());
            item.data_type = Some(field.data_type().to_string());
            item.nullable = Some(field.is_nullable());
            output.push(item);
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
            let insert = quote_identifier(&column.name);
            output.push(column_candidate(column, &insert, replacement, "00"));
        }
    }
}

fn complete_relations(
    prefix: &str,
    replacement: SourceSpan,
    scope: &QueryScope,
    catalog: &[RelationMetadata],
    quoted_mode: bool,
    output: &mut Vec<CompletionItem>,
) {
    if quoted_mode {
        let mut seen = BTreeSet::new();
        for relation in scope.ctes.iter().chain(catalog.iter()) {
            let Some(label) = relation.path.first() else {
                continue;
            };
            if !seen.insert(label.to_ascii_lowercase()) || !candidate_matches(label, prefix) {
                continue;
            }
            let kind = if relation.path.len() == 1 {
                CompletionKind::Table
            } else {
                CompletionKind::Catalog
            };
            let mut item = candidate(
                label,
                &quote_identifier(label),
                replacement,
                kind,
                Some(relation.detail.clone()),
                if relation.path.len() == 1 {
                    CompletionOrigin::QueryScope
                } else {
                    CompletionOrigin::Catalog
                },
                "00",
            );
            item.filter_text = Some(quote_identifier(label));
            output.push(item);
        }
        return;
    }
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
                .saturating_sub(
                    scope
                        .merged_column_reductions
                        .get(&column.name.to_ascii_lowercase())
                        .copied()
                        .unwrap_or_default(),
                )
                > 1;
            let insert = if ambiguous && !relation.visible_name().is_empty() {
                format!(
                    "{}.{}",
                    relation.visible_name(),
                    quote_identifier(&column.name)
                )
            } else {
                quote_identifier(&column.name)
            };
            if candidate_matches(&column.name, prefix) || candidate_matches(&insert, prefix) {
                let mut item = column_candidate(
                    column,
                    &insert,
                    replacement,
                    if ambiguous { "10" } else { "00" },
                );
                if ambiguous {
                    item.qualification = CompletionQualification::Ambiguous;
                }
                output.push(item);
            }
        }
    }
    if scope.projection_aliases_visible {
        for alias in &scope.projection_aliases {
            if candidate_matches(&alias.name, prefix) {
                output.push(column_candidate(
                    alias,
                    &quote_identifier(&alias.name),
                    replacement,
                    "10",
                ));
            }
        }
    }
}

fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn table_binding_relations(
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    project: Option<&ModuleAnalysis>,
    site: avenger_lang_core::syntax::SqlIslandSite,
    island: SourceSpan,
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
                && binding_symbol_visible(document, symbol, cursor, site, island)
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
) -> Option<&'a arrow::datatypes::Fields> {
    let mut parts = qualifier.trim_start_matches('$').split('.');
    let name = parts.next()?;
    let document = document?;
    let island = document
        .sql_islands
        .iter()
        .filter(|island| island.span.range.start <= cursor && cursor <= island.span.range.end)
        .min_by_key(|island| island.span.range.len());
    if !document.symbols.iter().any(|symbol| {
        symbol.value_kind == IndexedValueKind::Scalar
            && symbol.name.eq_ignore_ascii_case(name)
            && island.is_some_and(|island| {
                binding_symbol_visible(document, symbol, cursor, island.site, island.span)
            })
    }) {
        return None;
    }
    let resolved = project?.resolved_module_graph.as_deref()?;
    let param = resolved
        .params
        .values()
        .find(|param| param.source_name.eq_ignore_ascii_case(name))?;
    let mut data_type = project?.param_types.get(&param.id)?;
    for part in parts {
        let DataType::Struct(fields) = data_type else {
            return None;
        };
        data_type = fields
            .iter()
            .find(|field| field.name().eq_ignore_ascii_case(part))?
            .data_type();
    }
    match data_type {
        DataType::Struct(fields) => Some(fields),
        _ => None,
    }
}

fn complete_physical_struct_fields(
    prefix: &str,
    replacement: SourceSpan,
    fields: &arrow::datatypes::Fields,
    qualifier: &str,
    output: &mut Vec<CompletionItem>,
) {
    for field in fields {
        if candidate_matches(field.name(), prefix) {
            let mut item = candidate(
                field.name(),
                field.name(),
                replacement,
                CompletionKind::Field,
                Some(format!(
                    "{} · {} · struct field of {qualifier}",
                    field.data_type(),
                    nullability(field.is_nullable())
                )),
                CompletionOrigin::LexicalScope,
                "00",
            );
            item.semantic_kind = CompletionSemanticKind::StructField;
            item.semantic_identity = format!("StructField:{qualifier}:{}", field.name());
            item.data_type = Some(field.data_type().to_string());
            item.nullable = Some(field.is_nullable());
            output.push(item);
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

fn binding_symbol_visible(
    document: &DocumentSemanticIndex,
    candidate: &crate::IndexedSymbol,
    cursor: usize,
    site: avenger_lang_core::syntax::SqlIslandSite,
    island: SourceSpan,
) -> bool {
    if !symbol_visible(document, candidate, cursor) {
        return false;
    }
    if site != avenger_lang_core::syntax::SqlIslandSite::ParamInitializer {
        return true;
    }
    let Some(owner_index) = document
        .symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| {
            symbol.declaration_span.range.start <= island.range.start
                && island.range.end <= symbol.declaration_span.range.end
        })
        .min_by_key(|(_, symbol)| symbol.declaration_span.range.len())
        .map(|(index, _)| index)
    else {
        return true;
    };
    let owner = &document.symbols[owner_index];
    if candidate.identity == owner.identity {
        return false;
    }
    let mut ancestor = Some(owner_index);
    while let Some(index) = ancestor {
        let symbol = &document.symbols[index];
        if symbol.keyword == "table" {
            // Catalog-table param initializers are deliberately self-contained.
            return false;
        }
        ancestor = symbol.parent;
    }
    true
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
    let mut item = candidate(
        &column.name,
        insert,
        replacement,
        CompletionKind::Field,
        Some(detail),
        CompletionOrigin::DatasetSchema,
        bucket,
    );
    item.data_type = Some(column.data_type.to_string());
    item.nullable = Some(column.nullable);
    item.source_stage = Some(column.stage.clone());
    item.filter_text = Some(quote_identifier(&column.name));
    item.semantic_proximity = 100;
    if insert.contains(".\"") {
        item.qualification = CompletionQualification::Qualified;
    }
    item
}

#[allow(clippy::too_many_arguments)]
fn complete_scalar_bindings(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    project: Option<&ModuleAnalysis>,
    site: avenger_lang_core::syntax::SqlIslandSite,
    island: SourceSpan,
    allow_selections: bool,
    output: &mut Vec<CompletionItem>,
) {
    let typed = prefix.trim_start_matches('$');
    let Some(document) = document else {
        return;
    };
    for symbol in &document.symbols {
        if !(matches!(symbol.value_kind, IndexedValueKind::Scalar)
            || allow_selections && symbol.value_kind == IndexedValueKind::Selection)
        {
            continue;
        }
        if !binding_symbol_visible(document, symbol, cursor, site, island)
            || !candidate_matches(&symbol.name, typed)
        {
            continue;
        }
        let label = format!("${}", symbol.name);
        let bucket = format!("00:{:020}", symbol.selection_span.range.start);
        let mut item = candidate(
            &label,
            &label,
            replacement,
            CompletionKind::Variable,
            symbol.detail.clone(),
            CompletionOrigin::LexicalScope,
            &bucket,
        );
        if symbol.value_kind == IndexedValueKind::Selection {
            item.semantic_kind = CompletionSemanticKind::SelectionParam;
            item.detail = Some("Boolean current-row selection predicate".to_owned());
            item.data_type = Some("Boolean".to_owned());
            item.nullable = Some(true);
        } else if let Some(data_type) = project
            .and_then(|project| {
                project
                    .resolved_module_graph
                    .as_deref()
                    .map(|graph| (project, graph))
            })
            .and_then(|(project, graph)| {
                graph
                    .params
                    .values()
                    .find(|param| param.source_name.eq_ignore_ascii_case(&symbol.name))
                    .and_then(|param| project.param_types.get(&param.id))
            })
        {
            item.data_type = Some(data_type.to_string());
        }
        output.push(item);
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_table_bindings(
    prefix: &str,
    replacement: SourceSpan,
    document: Option<&DocumentSemanticIndex>,
    cursor: usize,
    project: Option<&ModuleAnalysis>,
    site: avenger_lang_core::syntax::SqlIslandSite,
    island: SourceSpan,
    output: &mut Vec<CompletionItem>,
) {
    let typed = prefix.trim_start_matches('$');
    let Some(document) = document else {
        return;
    };
    for symbol in &document.symbols {
        if symbol.value_kind != IndexedValueKind::Table
            || !binding_symbol_visible(document, symbol, cursor, site, island)
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
            symbol.value_kind == IndexedValueKind::Scalar
                && symbol.name.eq_ignore_ascii_case(base)
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

#[allow(clippy::too_many_arguments)]
fn complete_functions(
    prefix: &str,
    replacement: SourceSpan,
    project: Option<&ModuleAnalysis>,
    in_event: bool,
    context: SqlIslandContext,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    projection_mode: Option<ProjectionExpressionMode>,
    snippets: bool,
    output: &mut Vec<CompletionItem>,
) {
    let clause = query_clause_at(tokens, cursor);
    for function in project
        .into_iter()
        .flat_map(|project| &project.functions.functions)
        .filter(|function| function.category != FunctionCategory::Table)
        .filter(|function| {
            function_category_is_legal(function.category, context, clause, projection_mode)
        })
    {
        if candidate_matches(&function.name, prefix) {
            let category = match function.category {
                FunctionCategory::Scalar => "scalar",
                FunctionCategory::Aggregate => "aggregate",
                FunctionCategory::Window => "window",
                FunctionCategory::Table => unreachable!(),
            };
            let mut detail = format!("DataFusion {category} function");
            if let Some(signature) = &function.signature {
                detail.push_str(&format!(" · {signature}"));
            }
            if let Some(volatility) = &function.volatility {
                detail.push_str(&format!(" · {volatility}"));
            }
            let (insert, text_format) =
                function_insert(&function.name, &function.parameter_names, snippets);
            let mut item = candidate(
                &function.name,
                &insert,
                replacement,
                CompletionKind::Function,
                Some(detail),
                CompletionOrigin::FunctionRegistry,
                "10",
            );
            item.documentation = function.description.clone();
            item.insert_text_format = text_format;
            item.data_type = function.return_type.clone();
            item.semantic_kind = match function.category {
                FunctionCategory::Scalar => CompletionSemanticKind::ScalarFunction,
                FunctionCategory::Aggregate => CompletionSemanticKind::AggregateFunction,
                FunctionCategory::Window => CompletionSemanticKind::WindowFunction,
                FunctionCategory::Table => CompletionSemanticKind::TableFunction,
            };
            item.semantic_identity = format!("{:?}:{}", function.category, function.name);
            output.push(item);
        }
    }
    for operation in INTRINSIC_OPERATION_SIGNATURES.iter().filter(|signature| {
        in_event
            && signature
                .contexts
                .contains(&IntrinsicOperationContext::EventExpression)
    }) {
        if candidate_matches(operation.name, prefix) {
            let argument_names = operation
                .arguments
                .iter()
                .map(|argument| argument.as_str().replace(' ', "_"))
                .collect::<Vec<_>>();
            let (insert, text_format) = function_insert(operation.name, &argument_names, snippets);
            let mut item = candidate(
                operation.name,
                &insert,
                replacement,
                CompletionKind::Function,
                Some(format!(
                    "{} Returns `{}`.",
                    operation.docs,
                    operation.result.as_str()
                )),
                CompletionOrigin::FunctionRegistry,
                "00",
            );
            item.insert_text_format = text_format;
            item.documentation = Some(operation.docs.to_owned());
            item.data_type = operation.result.arrow_type().map(str::to_owned);
            output.push(item);
        }
    }
}

fn function_category_is_legal(
    category: FunctionCategory,
    context: SqlIslandContext,
    clause: QueryClause,
    projection_mode: Option<ProjectionExpressionMode>,
) -> bool {
    if context.root() == SqlIslandRoot::Query && clause == QueryClause::Using {
        return false;
    }
    match category {
        FunctionCategory::Scalar => true,
        FunctionCategory::Aggregate => match context.root() {
            SqlIslandRoot::Projection => matches!(
                projection_mode,
                Some(ProjectionExpressionMode::Aggregate | ProjectionExpressionMode::Window)
            ),
            SqlIslandRoot::Query => matches!(
                clause,
                QueryClause::Select
                    | QueryClause::Having
                    | QueryClause::Qualify
                    | QueryClause::OrderBy
            ),
            SqlIslandRoot::Expression => false,
        },
        FunctionCategory::Window => match context.root() {
            SqlIslandRoot::Projection => projection_mode == Some(ProjectionExpressionMode::Window),
            SqlIslandRoot::Query => matches!(
                clause,
                QueryClause::Select | QueryClause::Qualify | QueryClause::OrderBy
            ),
            SqlIslandRoot::Expression => false,
        },
        FunctionCategory::Table => false,
    }
}

fn complete_table_functions(
    prefix: &str,
    replacement: SourceSpan,
    project: Option<&ModuleAnalysis>,
    snippets: bool,
    output: &mut Vec<CompletionItem>,
) {
    for function in project
        .into_iter()
        .flat_map(|project| &project.functions.functions)
        .filter(|function| function.category == FunctionCategory::Table)
    {
        if !candidate_matches(&function.name, prefix) {
            continue;
        }
        let (insert, text_format) =
            function_insert(&function.name, &function.parameter_names, snippets);
        let mut item = candidate(
            &function.name,
            &insert,
            replacement,
            CompletionKind::Function,
            Some("DataFusion table function".to_owned()),
            CompletionOrigin::FunctionRegistry,
            "10",
        );
        item.semantic_kind = CompletionSemanticKind::TableFunction;
        item.semantic_identity = format!("TableFunction:{}", function.name);
        item.insert_text_format = text_format;
        item.documentation = function.description.clone();
        output.push(item);
    }
}

fn named_window_reference_position(tokens: &[SqlToken<'_>], cursor: usize, prefix: &str) -> bool {
    let last = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.span.range.end <= cursor)
        .map(|(index, _)| index)
        .next_back();
    let structural = last.and_then(|index| {
        (!prefix.is_empty()
            && tokens[index].span.range.end == cursor
            && tokens[index].span.range.start < cursor)
            .then(|| index.checked_sub(1))
            .flatten()
            .or(Some(index))
    });
    structural.is_some_and(|index| tokens[index].is_word("over"))
}

fn complete_named_windows(
    prefix: &str,
    replacement: SourceSpan,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) {
    let depth = cursor_depth(tokens, cursor);
    let Some(mut index) = tokens
        .iter()
        .enumerate()
        .find(|(_, token)| token.depth == depth && token.is_word("window"))
        .map(|(index, _)| index + 1)
    else {
        return;
    };
    while index < tokens.len() {
        let Some(name) = tokens
            .get(index)
            .filter(|token| token.depth == depth)
            .and_then(SqlToken::word)
        else {
            break;
        };
        let Some(as_index) = tokens[index + 1..]
            .iter()
            .position(|token| token.depth == depth && token.is_word("as"))
            .map(|offset| index + 1 + offset)
        else {
            break;
        };
        if candidate_matches(name, prefix) {
            let mut item = candidate(
                name,
                name,
                replacement,
                CompletionKind::Variable,
                Some("named SQL window".to_owned()),
                CompletionOrigin::QueryScope,
                "00",
            );
            item.semantic_kind = CompletionSemanticKind::WindowName;
            item.semantic_identity = format!("WindowName:{name}");
            output.push(item);
        }
        let Some(open) = tokens
            .get(as_index + 1)
            .filter(|token| token.depth == depth && matches!(token.token, Some(Token::LParen)))
            .map(|_| as_index + 1)
        else {
            break;
        };
        let Some(close) = matching_close(tokens, open) else {
            break;
        };
        index = close + 1;
        if tokens
            .get(index)
            .is_some_and(|token| token.depth == depth && token.is_comma())
        {
            index += 1;
        } else {
            break;
        }
    }
}

fn function_insert(
    name: &str,
    parameter_names: &[String],
    snippets: bool,
) -> (String, CompletionTextFormat) {
    if !snippets {
        return (format!("{name}()"), CompletionTextFormat::PlainText);
    }
    let arguments = parameter_names
        .iter()
        .enumerate()
        .map(|(index, parameter)| format!("${{{}:{parameter}}}", index + 1))
        .collect::<Vec<_>>()
        .join(", ");
    (
        format!("{name}({arguments})$0"),
        CompletionTextFormat::Snippet,
    )
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
    intents: &BTreeSet<SqlIntent>,
    context: SqlIslandContext,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    output: &mut Vec<CompletionItem>,
) {
    if context.root() == SqlIslandRoot::Query
        && let Some(state) = cte_header_state(tokens, cursor, prefix)
    {
        match state {
            CteHeaderState::Name => {}
            CteHeaderState::RecursiveKeyword => {
                push_sql_words(prefix, replacement, &["RECURSIVE"], false, output)
            }
            CteHeaderState::AfterName => {
                push_sql_words(prefix, replacement, &["AS"], false, output)
            }
            CteHeaderState::AfterAs => {
                if candidate_matches("(", prefix) {
                    let mut scaffold = candidate(
                        "(",
                        "(",
                        replacement,
                        CompletionKind::Keyword,
                        Some("open the CTE query body".to_owned()),
                        CompletionOrigin::Syntax,
                        "00",
                    );
                    scaffold.semantic_kind = CompletionSemanticKind::SqlOperator;
                    scaffold.semantic_identity = "CteBodyOpen".to_owned();
                    scaffold.validity = CompletionValidity::Scaffold;
                    output.push(scaffold);
                }
            }
            CteHeaderState::BodyComplete => {
                push_sql_words(
                    prefix,
                    replacement,
                    &["SELECT", "FROM", "VALUES"],
                    false,
                    output,
                );
            }
        }
        return;
    }
    if intents.contains(&SqlIntent::TypeName)
        || intents.contains(&SqlIntent::NameBinder)
        || intents.contains(&SqlIntent::CteName)
    {
        return;
    }
    if intents.contains(&SqlIntent::RelationPath) {
        push_sql_words(prefix, replacement, &["LATERAL"], false, output);
        return;
    }

    if context.root() != SqlIslandRoot::Query {
        if intents.contains(&SqlIntent::ExpressionOperand) {
            let mut keywords = vec!["CASE", "CAST", "NULL", "TRUE", "FALSE"];
            if tokens.iter().any(|token| token.is_word("case")) {
                keywords.extend(["WHEN", "THEN", "ELSE", "END"]);
            }
            push_sql_words(prefix, replacement, &keywords, false, output);
        }
        if intents.contains(&SqlIntent::ExpressionOperator) {
            if context.root() == SqlIslandRoot::Projection {
                push_sql_words(prefix, replacement, &["AS"], false, output);
            }
            push_sql_words(
                prefix,
                replacement,
                &["AND", "OR", "IS NULL", "IS NOT NULL"],
                true,
                output,
            );
        }
        return;
    }

    if wildcard_modifier_position(tokens, cursor, prefix) {
        push_sql_words(
            prefix,
            replacement,
            &["EXCLUDE", "EXCEPT", "REPLACE", "ILIKE"],
            false,
            output,
        );
        return;
    }
    if inside_window_specification(tokens, cursor) {
        push_sql_words(
            prefix,
            replacement,
            &["PARTITION BY", "ORDER BY", "ROWS", "RANGE", "GROUPS"],
            false,
            output,
        );
        return;
    }

    if intents.contains(&SqlIntent::ExpressionOperand) {
        let mut operands = vec!["CASE", "CAST", "NULL", "TRUE", "FALSE"];
        let clause = query_clause_at(tokens, cursor);
        let depth = cursor_depth(tokens, cursor);
        let has_payload = query_clause_and_index(tokens, cursor)
            .1
            .is_some_and(|index| {
                tokens[index + 1..].iter().any(|token| {
                    token.span.range.start < cursor
                        && token.depth == depth
                        && !token.is_comma()
                        && !token.is_period()
                })
            });
        if clause == QueryClause::Select && !has_payload {
            operands.extend(["DISTINCT", "ALL"]);
        }
        if tokens.iter().any(|token| token.is_word("case")) {
            operands.extend(["WHEN", "THEN", "ELSE", "END"]);
        }
        push_sql_words(prefix, replacement, &operands, false, output);
        return;
    }

    if let Some(keywords) = case_transition_keywords(tokens, cursor) {
        push_sql_words(prefix, replacement, keywords, false, output);
        return;
    }

    let (clause, clause_index) = query_clause_and_index(tokens, cursor);
    let depth = cursor_depth(tokens, cursor);
    let has_payload = clause_index.is_some_and(|index| {
        tokens[index + 1..].iter().any(|token| {
            token.span.range.start < cursor
                && token.depth == depth
                && !token.is_comma()
                && !token.is_period()
        })
    });
    let has_window = tokens.iter().any(|token| token.is_word("over"));
    let mut keywords = Vec::new();
    let mut operators = Vec::new();
    match clause {
        QueryClause::Start => keywords.extend(["SELECT", "FROM", "WITH", "VALUES"]),
        QueryClause::With => keywords.extend(["SELECT", "FROM", "VALUES"]),
        QueryClause::Select if !has_payload => {
            keywords.extend(["DISTINCT", "ALL", "CASE", "CAST", "NULL", "TRUE", "FALSE"])
        }
        QueryClause::Select => {
            keywords.extend(["AS", "FROM", "FILTER", "OVER"]);
        }
        QueryClause::From => {
            keywords.extend([
                "AS",
                "JOIN",
                "LEFT JOIN",
                "RIGHT JOIN",
                "FULL JOIN",
                "INNER JOIN",
                "CROSS JOIN",
                "WHERE",
                "GROUP BY",
                "HAVING",
                "WINDOW",
                "ORDER BY",
                "LIMIT",
                "OFFSET",
                "FETCH",
                "UNION ALL",
                "EXCEPT",
                "INTERSECT",
            ]);
            if has_window {
                keywords.push("QUALIFY");
            }
        }
        QueryClause::Join => keywords.extend(["AS", "ON", "USING"]),
        QueryClause::Using => {}
        QueryClause::On | QueryClause::Where if !has_payload => {
            keywords.extend(["CASE", "CAST", "NULL", "TRUE", "FALSE"])
        }
        QueryClause::On | QueryClause::Where => {
            operators.extend(["AND", "OR", "IS NULL", "IS NOT NULL"]);
            keywords.extend([
                "JOIN",
                "LEFT JOIN",
                "RIGHT JOIN",
                "FULL JOIN",
                "GROUP BY",
                "HAVING",
                "WINDOW",
                "ORDER BY",
                "LIMIT",
                "OFFSET",
                "FETCH",
            ]);
            if has_window {
                keywords.push("QUALIFY");
            }
        }
        QueryClause::GroupBy
        | QueryClause::Having
        | QueryClause::Qualify
        | QueryClause::OrderBy
            if !has_payload =>
        {
            keywords.extend(["CASE", "CAST", "NULL", "TRUE", "FALSE"])
        }
        QueryClause::GroupBy => {
            keywords.extend(["HAVING", "WINDOW", "ORDER BY", "LIMIT", "OFFSET", "FETCH"])
        }
        QueryClause::Having => {
            operators.extend(["AND", "OR", "IS NULL", "IS NOT NULL"]);
            keywords.extend(["WINDOW", "ORDER BY", "LIMIT", "OFFSET", "FETCH"]);
            if has_window {
                keywords.push("QUALIFY");
            }
        }
        QueryClause::Window => keywords.extend(["QUALIFY", "ORDER BY", "LIMIT", "OFFSET", "FETCH"]),
        QueryClause::Qualify => {
            operators.extend(["AND", "OR", "IS NULL", "IS NOT NULL"]);
            keywords.extend(["ORDER BY", "LIMIT", "OFFSET", "FETCH"]);
        }
        QueryClause::OrderBy => keywords.extend([
            "ASC",
            "DESC",
            "NULLS FIRST",
            "NULLS LAST",
            "LIMIT",
            "OFFSET",
            "FETCH",
        ]),
        QueryClause::Limit => keywords.extend(["OFFSET", "FETCH"]),
        QueryClause::Offset => keywords.push("FETCH"),
        QueryClause::Fetch => {}
        QueryClause::SetOperation => keywords.extend(["SELECT", "FROM", "VALUES"]),
        QueryClause::Values => keywords.extend(["ORDER BY", "LIMIT", "OFFSET", "FETCH"]),
    }
    push_sql_words(prefix, replacement, &keywords, false, output);
    push_sql_words(prefix, replacement, &operators, true, output);
}

fn wildcard_modifier_position(tokens: &[SqlToken<'_>], cursor: usize, prefix: &str) -> bool {
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.span.range.end <= cursor)
        .map(|(index, _)| index)
        .next_back()
        .and_then(|index| {
            (!prefix.is_empty()
                && tokens[index].span.range.end == cursor
                && tokens[index].span.range.start < cursor)
                .then(|| index.checked_sub(1))
                .flatten()
                .or(Some(index))
        })
        .is_some_and(|index| tokens[index].raw == "*")
}

fn push_sql_words(
    prefix: &str,
    replacement: SourceSpan,
    words: &[&str],
    operators: bool,
    output: &mut Vec<CompletionItem>,
) {
    for word in words {
        if !candidate_matches(word, prefix) {
            continue;
        }
        let mut item = candidate(
            word,
            word,
            replacement,
            CompletionKind::Keyword,
            Some(if operators {
                "SQL operator".to_owned()
            } else {
                "SQL keyword".to_owned()
            }),
            CompletionOrigin::Syntax,
            "30",
        );
        if operators {
            item.semantic_kind = CompletionSemanticKind::SqlOperator;
            item.semantic_identity = format!("SqlOperator:{word}");
        }
        output.push(item);
    }
}

impl QuerySkeleton {
    fn active_clause(&self, tokens: &[SqlToken<'_>], cursor: usize) -> QueryClause {
        let block = &self.blocks[self.active_block];
        query_clause_and_index_in_block(tokens, cursor, block.depth, block.span).0
    }
}

fn build_query_skeleton(
    text: &str,
    island: SourceSpan,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    root: SqlIslandRoot,
) -> QuerySkeleton {
    let mut blocks = vec![SqlQueryBlockDebug {
        index: 0,
        parent: None,
        depth: 0,
        span: island,
        active: false,
        items: Vec::new(),
    }];

    for (open, token) in tokens.iter().enumerate() {
        if !matches!(token.token, Some(Token::LParen)) {
            continue;
        }
        let depth = token.depth + 1;
        let Some(first) = tokens[open + 1..]
            .iter()
            .find(|candidate| candidate.depth == depth)
        else {
            continue;
        };
        if !["select", "from", "with", "values"]
            .iter()
            .any(|word| first.is_word(word))
        {
            continue;
        }
        let end = matching_close(tokens, open)
            .and_then(|close| tokens.get(close))
            .map_or(island.range.end, |close| close.span.range.end);
        let span = SourceSpan {
            source: island.source,
            range: ByteSpan {
                start: token.span.range.start,
                end,
            },
        };
        let parent = blocks
            .iter()
            .filter(|block| {
                block.span.range.start <= span.range.start
                    && span.range.end <= block.span.range.end
                    && block.depth < depth
            })
            .max_by_key(|block| block.depth)
            .map(|block| block.index)
            .or(Some(0));
        let index = blocks.len();
        blocks.push(SqlQueryBlockDebug {
            index,
            parent,
            depth,
            span,
            active: false,
            items: Vec::new(),
        });
    }

    let active_block = blocks
        .iter()
        .filter(|block| block.span.range.start <= cursor && cursor <= block.span.range.end)
        .max_by_key(|block| (block.depth, std::cmp::Reverse(block.span.range.len())))
        .map_or(0, |block| block.index);
    blocks[active_block].active = true;

    for block in &mut blocks {
        block.items = if block.index == 0 && root != SqlIslandRoot::Query {
            root_sql_items(text, island, cursor, root)
        } else {
            query_block_items(text, tokens, cursor, block.depth, block.span)
        };
    }

    QuerySkeleton {
        blocks,
        active_block,
    }
}

fn root_sql_items(
    text: &str,
    island: SourceSpan,
    cursor: usize,
    root: SqlIslandRoot,
) -> Vec<SqlClauseItemDebug> {
    let clause = match root {
        SqlIslandRoot::Projection => "projection",
        SqlIslandRoot::Expression => "expression",
        SqlIslandRoot::Query => "query",
    };
    let mut ranges = Vec::new();
    let mut start = island.range.start;
    let mut depth = 0usize;
    let mut quote = None;
    let mut offset = island.range.start;
    while offset < island.range.end.min(text.len()) {
        let character = text[offset..].chars().next().unwrap();
        if let Some(active) = quote {
            if character == active {
                let next = offset + character.len_utf8();
                if text[next..].starts_with(character) {
                    offset = next + character.len_utf8();
                    continue;
                }
                quote = None;
            }
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '(' | '[' => depth += 1,
                ')' | ']' => depth = depth.saturating_sub(1),
                ',' if root == SqlIslandRoot::Projection && depth == 0 => {
                    ranges.push(start..offset);
                    start = offset + 1;
                }
                _ => {}
            }
        }
        offset += character.len_utf8();
    }
    ranges.push(start..island.range.end);
    ranges
        .into_iter()
        .enumerate()
        .map(|(index, range)| {
            let trimmed = trim_source_range(text, range);
            SqlClauseItemDebug {
                clause: clause.to_owned(),
                index,
                span: SourceSpan {
                    source: island.source,
                    range: ByteSpan {
                        start: trimmed.start,
                        end: trimmed.end,
                    },
                },
                status: item_parse_status(text, trimmed, cursor, root, None),
            }
        })
        .collect()
}

fn query_block_items(
    text: &str,
    tokens: &[SqlToken<'_>],
    cursor: usize,
    depth: usize,
    block: SourceSpan,
) -> Vec<SqlClauseItemDebug> {
    let clauses = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| {
            token.depth == depth
                && block.range.start <= token.span.range.start
                && token.span.range.end <= block.range.end
        })
        .filter_map(|(index, _)| {
            clause_start_at(tokens, index, depth).map(|clause| (index, clause))
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    for (ordinal, (clause_index, clause)) in clauses.iter().copied().enumerate() {
        if clause == QueryClause::SetOperation {
            continue;
        }
        let mut payload = clause_index + 1;
        if matches!(clause, QueryClause::GroupBy | QueryClause::OrderBy)
            && tokens
                .get(payload)
                .is_some_and(|token| token.depth == depth && token.is_word("by"))
        {
            payload += 1;
        }
        let end_index = clauses
            .get(ordinal + 1)
            .map_or(tokens.len(), |(index, _)| *index);
        let payload_tokens = tokens[payload.min(tokens.len())..end_index.min(tokens.len())]
            .iter()
            .enumerate()
            .filter(|(_, token)| {
                block.range.start <= token.span.range.start
                    && token.span.range.end <= block.range.end
            })
            .collect::<Vec<_>>();
        let mut item_start = 0usize;
        let mut item_index = 0usize;
        for split in 0..=payload_tokens.len() {
            let is_end = split == payload_tokens.len();
            let is_separator = !is_end
                && payload_tokens[split].1.depth == depth
                && payload_tokens[split].1.is_comma();
            if !is_end && !is_separator {
                continue;
            }
            let segment = &payload_tokens[item_start..split];
            let fallback = if item_start == 0 {
                tokens
                    .get(payload.saturating_sub(1))
                    .map_or(block.range.start, |token| token.span.range.end)
            } else {
                payload_tokens[item_start.saturating_sub(1)]
                    .1
                    .span
                    .range
                    .end
            };
            let raw_range = segment
                .first()
                .zip(segment.last())
                .map_or(fallback..fallback, |(first, last)| {
                    first.1.span.range.start..last.1.span.range.end
                });
            let range = trim_source_range(text, raw_range);
            let span = SourceSpan {
                source: block.source,
                range: ByteSpan {
                    start: range.start,
                    end: range.end,
                },
            };
            output.push(SqlClauseItemDebug {
                clause: clause.name().to_owned(),
                index: item_index,
                span,
                status: item_parse_status(text, range, cursor, SqlIslandRoot::Query, Some(clause)),
            });
            item_index += 1;
            item_start = split + 1;
        }
    }
    output
}

fn trim_source_range(text: &str, mut range: Range<usize>) -> Range<usize> {
    range.start = range.start.min(text.len());
    range.end = range.end.min(text.len()).max(range.start);
    while range.start < range.end {
        let character = text[range.start..range.end].chars().next().unwrap();
        if !character.is_whitespace() {
            break;
        }
        range.start += character.len_utf8();
    }
    while range.end > range.start {
        let character = text[range.start..range.end].chars().next_back().unwrap();
        if !character.is_whitespace() {
            break;
        }
        range.end -= character.len_utf8();
    }
    range
}

fn item_parse_status(
    text: &str,
    range: Range<usize>,
    cursor: usize,
    root: SqlIslandRoot,
    clause: Option<QueryClause>,
) -> SqlItemParseStatus {
    if range.start <= cursor && cursor <= range.end {
        return SqlItemParseStatus::Cursor;
    }
    if range.is_empty() {
        return SqlItemParseStatus::Empty;
    }
    let authored = text[range].trim();
    let parsed = match root {
        SqlIslandRoot::Expression => SqlExpression::parse(authored).is_ok(),
        SqlIslandRoot::Projection => SqlProjection::parse(authored).is_ok(),
        SqlIslandRoot::Query => match clause.unwrap_or(QueryClause::Start) {
            QueryClause::Select => SqlProjection::parse(authored).is_ok(),
            QueryClause::From | QueryClause::Join => {
                SqlQuery::parse(&format!("SELECT * FROM {authored}")).is_ok()
            }
            QueryClause::Values => SqlQuery::parse(&format!("VALUES {authored}")).is_ok(),
            QueryClause::With => SqlQuery::parse(&format!("WITH {authored} SELECT 1")).is_ok(),
            QueryClause::Window => SqlQuery::parse(&format!("SELECT 1 WINDOW {authored}")).is_ok(),
            QueryClause::OrderBy => {
                SqlQuery::parse(&format!("SELECT 1 ORDER BY {authored}")).is_ok()
            }
            QueryClause::GroupBy => {
                SqlQuery::parse(&format!("SELECT 1 GROUP BY {authored}")).is_ok()
            }
            QueryClause::Limit | QueryClause::Offset | QueryClause::Fetch => {
                SqlExpression::parse(authored).is_ok()
            }
            QueryClause::Start | QueryClause::SetOperation => false,
            QueryClause::On
            | QueryClause::Using
            | QueryClause::Where
            | QueryClause::Having
            | QueryClause::Qualify => SqlExpression::parse(authored).is_ok(),
        },
    };
    if parsed {
        SqlItemParseStatus::Parsed
    } else {
        SqlItemParseStatus::Unparsed
    }
}

fn clause_start_at(tokens: &[SqlToken<'_>], index: usize, depth: usize) -> Option<QueryClause> {
    let token = tokens.get(index)?;
    if token.depth != depth {
        return None;
    }
    let next_is_by = tokens
        .get(index + 1)
        .is_some_and(|next| next.depth == depth && next.is_word("by"));
    if token.is_word("with") {
        Some(QueryClause::With)
    } else if token.is_word("select") {
        Some(QueryClause::Select)
    } else if token.is_word("from") {
        Some(QueryClause::From)
    } else if token.is_word("join") {
        Some(QueryClause::Join)
    } else if token.is_word("on") {
        Some(QueryClause::On)
    } else if token.is_word("using") {
        Some(QueryClause::Using)
    } else if token.is_word("where") {
        Some(QueryClause::Where)
    } else if token.is_word("group") && next_is_by {
        Some(QueryClause::GroupBy)
    } else if token.is_word("having") {
        Some(QueryClause::Having)
    } else if token.is_word("window") {
        Some(QueryClause::Window)
    } else if token.is_word("qualify") {
        Some(QueryClause::Qualify)
    } else if token.is_word("order") && next_is_by {
        Some(QueryClause::OrderBy)
    } else if token.is_word("limit") {
        Some(QueryClause::Limit)
    } else if token.is_word("offset") {
        Some(QueryClause::Offset)
    } else if token.is_word("fetch") {
        Some(QueryClause::Fetch)
    } else if token.is_word("values") {
        Some(QueryClause::Values)
    } else if token.is_word("union") || token.is_word("except") || token.is_word("intersect") {
        Some(QueryClause::SetOperation)
    } else {
        None
    }
}

fn query_clause_at(tokens: &[SqlToken<'_>], cursor: usize) -> QueryClause {
    query_clause_and_index(tokens, cursor).0
}

fn cte_header_state(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    prefix: &str,
) -> Option<CteHeaderState> {
    let (clause, with_index) = query_clause_and_index(tokens, cursor);
    if clause != QueryClause::With {
        return None;
    }
    let with_index = with_index?;
    let depth = tokens[with_index].depth;
    let mut indices = tokens
        .iter()
        .enumerate()
        .skip(with_index + 1)
        .filter(|(_, token)| token.depth == depth && token.span.range.start < cursor)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if indices
        .first()
        .is_some_and(|index| tokens[*index].is_word("recursive"))
    {
        indices.remove(0);
    } else if let Some(index) = indices.first().copied()
        && let Some(word) = tokens[index].word()
        && !prefix.is_empty()
        && tokens[index].span.range.end == cursor
        && "recursive".starts_with(&word.to_ascii_lowercase())
    {
        return Some(CteHeaderState::RecursiveKeyword);
    }

    if let Some(comma) = indices.iter().rposition(|index| tokens[*index].is_comma()) {
        indices.drain(..=comma);
    }
    let Some(name_index) = indices.first().copied() else {
        return Some(CteHeaderState::Name);
    };
    let name = &tokens[name_index];
    if name.word().is_none() {
        return Some(CteHeaderState::Name);
    }
    let Some(as_index) = indices
        .iter()
        .copied()
        .find(|index| tokens[*index].is_word("as"))
    else {
        return Some(if name.span.range.end < cursor {
            CteHeaderState::AfterName
        } else {
            CteHeaderState::Name
        });
    };
    if tokens[as_index].span.range.end == cursor && prefix.eq_ignore_ascii_case("as") {
        return Some(CteHeaderState::AfterName);
    }
    let Some(open_index) = indices
        .iter()
        .copied()
        .find(|index| *index > as_index && matches!(tokens[*index].token, Some(Token::LParen)))
    else {
        return Some(CteHeaderState::AfterAs);
    };
    matching_close(tokens, open_index)
        .filter(|close| tokens[*close].span.range.end <= cursor)
        .map(|_| CteHeaderState::BodyComplete)
}

fn sql_cursor_path_debug(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    qualifier: Option<String>,
    replacement: SourceSpan,
    skeleton: &QuerySkeleton,
) -> SqlCursorPathDebug {
    let block = &skeleton.blocks[skeleton.active_block];
    let depth = block.depth;
    let (clause, clause_index) = query_clause_and_index_in_block(tokens, cursor, depth, block.span);
    let item_index = clause_index.map_or(0, |start| {
        tokens[start + 1..]
            .iter()
            .take_while(|token| token.span.range.start < cursor)
            .filter(|token| token.depth == depth && token.is_comma())
            .count()
    });
    SqlCursorPathDebug {
        clause: clause.name().to_owned(),
        nesting_depth: depth,
        item_index,
        qualifier,
        replacement,
        ast_path: vec![
            format!("query_block[{}]", block.index),
            clause.name().to_owned(),
            format!("item[{item_index}]"),
        ],
    }
}

fn query_clause_and_index(tokens: &[SqlToken<'_>], cursor: usize) -> (QueryClause, Option<usize>) {
    let depth = cursor_depth(tokens, cursor);
    for candidate_depth in (0..=depth).rev() {
        if candidate_depth < depth && paren_introduces_query(tokens, cursor, depth) {
            break;
        }
        let mut clause = QueryClause::Start;
        let mut clause_index = None;
        for (index, token) in tokens.iter().enumerate() {
            if token.span.range.end > cursor || token.depth != candidate_depth {
                continue;
            }
            let candidate = clause_start_at(tokens, index, candidate_depth);
            if let Some(candidate) = candidate {
                clause = candidate;
                clause_index = Some(index);
            }
        }
        if clause_index.is_some() {
            return (clause, clause_index);
        }
    }
    (QueryClause::Start, None)
}

fn query_clause_and_index_in_block(
    tokens: &[SqlToken<'_>],
    cursor: usize,
    depth: usize,
    block: SourceSpan,
) -> (QueryClause, Option<usize>) {
    let mut clause = QueryClause::Start;
    let mut clause_index = None;
    for (index, token) in tokens.iter().enumerate() {
        if token.span.range.end > cursor
            || token.depth != depth
            || token.span.range.start < block.range.start
            || block.range.end < token.span.range.end
        {
            continue;
        }
        if let Some(candidate) = clause_start_at(tokens, index, depth) {
            clause = candidate;
            clause_index = Some(index);
        }
    }
    (clause, clause_index)
}

fn active_open_paren(tokens: &[SqlToken<'_>], cursor: usize, depth: usize) -> Option<usize> {
    if depth == 0 {
        return None;
    }
    tokens
        .iter()
        .enumerate()
        .rev()
        .find(|(index, token)| {
            token.span.range.start < cursor
                && token.depth + 1 == depth
                && matches!(token.token, Some(Token::LParen))
                && matching_close(tokens, *index)
                    .is_none_or(|close| tokens[close].span.range.end >= cursor)
        })
        .map(|(index, _)| index)
}

fn paren_introduces_query(tokens: &[SqlToken<'_>], cursor: usize, depth: usize) -> bool {
    let Some(open) = active_open_paren(tokens, cursor, depth) else {
        return false;
    };
    if tokens[open + 1..].iter().any(|token| {
        token.span.range.end <= cursor
            && token.depth == depth
            && (token.is_word("select")
                || token.is_word("from")
                || token.is_word("with")
                || token.is_word("values"))
    }) {
        return true;
    }
    tokens[..open]
        .iter()
        .rev()
        .find_map(SqlToken::word)
        .is_some_and(|word| {
            ["exists", "in", "from", "join"]
                .iter()
                .any(|candidate| word.eq_ignore_ascii_case(candidate))
        })
}

fn inside_window_specification(tokens: &[SqlToken<'_>], cursor: usize) -> bool {
    let depth = cursor_depth(tokens, cursor);
    let Some(open) = active_open_paren(tokens, cursor, depth) else {
        return false;
    };
    let words = tokens[..open]
        .iter()
        .rev()
        .filter_map(SqlToken::word)
        .take(3)
        .collect::<Vec<_>>();
    words
        .first()
        .is_some_and(|word| word.eq_ignore_ascii_case("over"))
        || (words
            .first()
            .is_some_and(|word| word.eq_ignore_ascii_case("as"))
            && words
                .get(2)
                .is_some_and(|word| word.eq_ignore_ascii_case("window")))
}

fn case_transition_keywords(
    tokens: &[SqlToken<'_>],
    cursor: usize,
) -> Option<&'static [&'static str]> {
    let depth = cursor_depth(tokens, cursor);
    let mut stack = Vec::<CaseStage>::new();
    for token in tokens
        .iter()
        .filter(|token| token.span.range.start < cursor && token.depth == depth)
    {
        if token.is_word("case") {
            stack.push(CaseStage::Case);
        } else if token.is_word("when") {
            if let Some(stage) = stack.last_mut() {
                *stage = CaseStage::When;
            }
        } else if token.is_word("then") {
            if let Some(stage) = stack.last_mut() {
                *stage = CaseStage::Then;
            }
        } else if token.is_word("else") {
            if let Some(stage) = stack.last_mut() {
                *stage = CaseStage::Else;
            }
        } else if token.is_word("end") {
            stack.pop();
        }
    }
    match stack.last()? {
        CaseStage::Case => Some(&["WHEN"]),
        CaseStage::When => Some(&["THEN"]),
        CaseStage::Then => Some(&["WHEN", "ELSE", "END"]),
        CaseStage::Else => Some(&["END"]),
    }
}

fn validate_expression(
    authored: &str,
    dataset: &AnalyzedDataset,
    repair: SqlRepairStrategy,
    cursor: usize,
) -> bool {
    let repaired = build_repaired_sql(
        authored,
        cursor..cursor,
        repair,
        sentinel_for_repair(repair),
        0,
    );
    let Ok(schema) = DFSchema::try_from(dataset.schema.as_ref().clone()) else {
        return false;
    };
    let Ok(normalized) = avenger_lang_compiler::normalize_sql_expression(&repaired.text) else {
        return false;
    };
    let state = SessionStateBuilder::new().with_default_features().build();
    LOGICAL_EXPRESSION_PLANS.fetch_add(1, Ordering::Relaxed);
    state
        .create_logical_expr(&normalized, &schema)
        .and_then(|expression| expression.get_type(&schema))
        .is_ok()
}

fn validate_projection(
    authored: &str,
    dataset: &AnalyzedDataset,
    repair: SqlRepairStrategy,
    cursor: usize,
) -> bool {
    let repaired = build_repaired_sql(
        authored,
        cursor..cursor,
        repair,
        sentinel_for_repair(repair),
        0,
    );
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
        let Ok(normalized) =
            avenger_lang_compiler::normalize_sql_expression(&expression.to_string())
        else {
            return false;
        };
        LOGICAL_EXPRESSION_PLANS.fetch_add(1, Ordering::Relaxed);
        state
            .create_logical_expr(&normalized, &schema)
            .and_then(|expression| expression.get_type(&schema))
            .is_ok()
    })
}

fn deduplicate_relations(relations: &mut Vec<RelationMetadata>) {
    // Stable depth ordering keeps the active block's source order (needed by
    // JOIN/USING/LATERAL) while still putting a nearer shadowing alias before
    // the same alias in a correlated parent.
    relations.sort_by_key(|relation| std::cmp::Reverse(relation.scope_depth));
    let mut seen = BTreeSet::new();
    relations.retain(|relation| {
        seen.insert((
            relation.visible_name().to_ascii_lowercase(),
            relation
                .path
                .iter()
                .map(|part| part.to_ascii_lowercase())
                .collect::<Vec<_>>(),
        ))
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
    let semantic_kind = sql_semantic_kind(kind, origin, detail.as_deref());
    CompletionItem {
        label: label.to_owned(),
        match_text: label.to_owned(),
        replacement,
        insert_text: insert.to_owned(),
        insert_text_format: CompletionTextFormat::PlainText,
        kind,
        semantic_kind,
        semantic_identity: format!("{semantic_kind:?}:{origin:?}:{label}:{insert}"),
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
        documentation: None,
        filter_text: Some(label.to_owned()),
        sort_key: format!("{bucket}:{}", label.to_ascii_lowercase()),
        origin,
        deprecated: false,
    }
}

fn sql_semantic_kind(
    kind: CompletionKind,
    origin: CompletionOrigin,
    detail: Option<&str>,
) -> CompletionSemanticKind {
    match kind {
        CompletionKind::Keyword => CompletionSemanticKind::SqlKeyword,
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
        CompletionKind::Function => {
            let detail = detail.unwrap_or_default();
            if detail.contains("aggregate") {
                CompletionSemanticKind::AggregateFunction
            } else if detail.contains("window") {
                CompletionSemanticKind::WindowFunction
            } else if detail.contains("table") {
                CompletionSemanticKind::TableFunction
            } else {
                CompletionSemanticKind::ScalarFunction
            }
        }
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

fn retain_candidates_for_lexical_mode(items: &mut Vec<CompletionItem>, mode: SqlLexicalMode) {
    items.retain(|item| match mode {
        SqlLexicalMode::DoubleQuotedIdentifier => matches!(
            item.semantic_kind,
            CompletionSemanticKind::DataColumn
                | CompletionSemanticKind::Relation
                | CompletionSemanticKind::Catalog
                | CompletionSemanticKind::Schema
        ),
        SqlLexicalMode::Binding | SqlLexicalMode::TemporalQualifier => matches!(
            item.semantic_kind,
            CompletionSemanticKind::ScalarParam
                | CompletionSemanticKind::StoreParam
                | CompletionSemanticKind::SelectionParam
                | CompletionSemanticKind::StructField
        ),
        SqlLexicalMode::Code => !matches!(
            item.semantic_kind,
            CompletionSemanticKind::DataColumn
                | CompletionSemanticKind::ScalarParam
                | CompletionSemanticKind::StoreParam
                | CompletionSemanticKind::SelectionParam
        ),
        SqlLexicalMode::SingleQuotedString
        | SqlLexicalMode::DollarQuotedString
        | SqlLexicalMode::LineComment
        | SqlLexicalMode::BlockComment => false,
    });
}

fn completion_invariants_hold(items: &[CompletionItem], mode: SqlLexicalMode) -> bool {
    items.iter().all(|item| match item.semantic_kind {
        CompletionSemanticKind::DataColumn => mode == SqlLexicalMode::DoubleQuotedIdentifier,
        CompletionSemanticKind::ScalarParam
        | CompletionSemanticKind::StoreParam
        | CompletionSemanticKind::SelectionParam => matches!(
            mode,
            SqlLexicalMode::Binding | SqlLexicalMode::TemporalQualifier
        ),
        _ if mode == SqlLexicalMode::DoubleQuotedIdentifier => matches!(
            item.semantic_kind,
            CompletionSemanticKind::Relation
                | CompletionSemanticKind::Catalog
                | CompletionSemanticKind::Schema
        ),
        _ => true,
    })
}

fn completion_invariant_violations(items: &[CompletionItem], mode: SqlLexicalMode) -> usize {
    items
        .iter()
        .filter(|item| !completion_invariants_hold(std::slice::from_ref(*item), mode))
        .count()
}

fn annotate_usage_prevalence(
    items: &mut [CompletionItem],
    tokens: &[SqlToken<'_>],
    index: &WorkspaceSemanticIndex,
) {
    for item in items {
        let name = item.label.trim_start_matches('$');
        let local = tokens
            .iter()
            .filter_map(SqlToken::word)
            .filter(|word| word.eq_ignore_ascii_case(name))
            .count();
        let project = index
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
        item.usage_prevalence = u32::try_from(local.saturating_add(project)).unwrap_or(u32::MAX);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use avenger_lang_compiler::{Compiler, ModuleFingerprint};
    use avenger_lang_core::{SourceFile, SourceId};
    use sqlparser::{
        ast::Spanned,
        parser::Parser,
        tokenizer::{Token, Tokenizer},
    };

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
        let (_, syntax_analysis) = syntax(text);
        let node = syntax_analysis
            .parsed
            .nodes
            .iter()
            .find(|node| matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. }))
            .unwrap();
        let tokens = island_tokens(&syntax_analysis, node.span);
        assert!(tokens.iter().any(|token| token.is_word("vega")));
        let replacement = sql_cursor_context(text, node.span, cursor).replacement;
        let skeleton = build_query_skeleton(text, node.span, &tokens, cursor, SqlIslandRoot::Query);
        let intents = expected_intents(
            &tokens,
            cursor,
            SqlIslandContext::QueryProperty,
            "",
            &skeleton,
        );
        assert_eq!(replacement.range, ByteSpan::empty(cursor));
        assert!(intents.contains(&SqlIntent::QualifiedMember));
        assert_eq!(
            select_repair(&tokens, cursor, &intents).0,
            SqlRepairStrategy::QualifierMember
        );
        let authored = &text[node.span.range.as_range()];
        let recovery = recover_sql(
            authored,
            cursor - node.span.range.start,
            SqlIslandRoot::Query,
            &tokens,
            cursor,
            &intents,
            cursor - node.span.range.start..cursor - node.span.range.start,
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
    fn recovery_uses_intent_specific_sentinels_and_minimum_closers() {
        assert_eq!(minimum_sql_closers("CASE WHEN (true"), ") END");
        assert_eq!(minimum_sql_closers("array[1, (2"), ")]");
        assert_eq!(minimum_sql_closers("'CASE ('"), "");

        let query_intents = BTreeSet::from([SqlIntent::QueryStart]);
        let query = recover_sql("", 0, SqlIslandRoot::Query, &[], 0, &query_intents, 0..0);
        assert_eq!(query.sentinel, Some(SqlCursorSentinelKind::QueryStart));
        assert_eq!(query.strategy, SqlRepairStrategy::QueryStart);
        assert!(query.parsed);
        assert!(query.attempts <= MAX_REPAIR_ATTEMPTS);

        let type_intents = BTreeSet::from([SqlIntent::TypeName]);
        let authored = "CAST(1 AS ";
        let typed = recover_sql(
            authored,
            authored.len(),
            SqlIslandRoot::Expression,
            &[],
            authored.len(),
            &type_intents,
            authored.len()..authored.len(),
        );
        assert_eq!(typed.sentinel, Some(SqlCursorSentinelKind::Type));
        assert!(typed.parsed);
        assert!(typed.attempts <= MAX_REPAIR_ATTEMPTS);
        assert!(typed.repaired.text.contains("BIGINT"));
        assert!(typed.repaired.text.ends_with(')'));
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
        let scope = build_query_scope(&tokens, cursor, 0, SqlIslandRoot::Query, &[], None, None);
        assert!(
            scope
                .relations
                .iter()
                .any(|relation| relation.visible_name() == "m")
        );
    }

    #[test]
    fn scope_recovers_derived_outputs_and_store_relations() {
        fn relation(path: &str, columns: &[&str]) -> RelationMetadata {
            RelationMetadata {
                path: vec![path.to_owned()],
                alias: None,
                columns: columns
                    .iter()
                    .map(|name| ColumnMetadata {
                        name: (*name).to_owned(),
                        qualifier: Some(path.to_owned()),
                        data_type: DataType::Utf8,
                        nullable: false,
                        stage: "test".to_owned(),
                        lineage: None,
                        detail: None,
                    })
                    .collect(),
                detail: "test relation".to_owned(),
                scope_depth: 0,
            }
        }

        let text = "avenger 1; chart cartesian as chart { table sql as t { sql: SELECT s.\"\" FROM (SELECT \"title\" AS movie_title FROM movies) AS s; } }";
        let cursor = text.find("s.\"\"").unwrap() + 3;
        let (_, syntax_analysis) = syntax(text);
        let node = syntax_analysis
            .parsed
            .nodes
            .iter()
            .find(|node| matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. }))
            .unwrap();
        let tokens = island_tokens(&syntax_analysis, node.span);
        let scope = build_query_scope(
            &tokens,
            cursor,
            cursor_depth(&tokens, cursor),
            SqlIslandRoot::Query,
            &[relation("movies", &["title"])],
            None,
            None,
        );
        let derived = scope
            .relations
            .iter()
            .find(|relation| relation.visible_name() == "s")
            .unwrap_or_else(|| panic!("derived relation\ntokens: {tokens:#?}\nscope: {scope:#?}"));
        assert_eq!(
            derived
                .columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            ["movie_title"]
        );

        let text = "avenger 1; chart cartesian as chart { table sql as t { sql: SELECT s.\"\" FROM $selected AS s; } }";
        let cursor = text.find("s.\"\"").unwrap() + 3;
        let (_, syntax) = syntax(text);
        let node = syntax
            .parsed
            .nodes
            .iter()
            .find(|node| matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. }))
            .unwrap();
        let tokens = island_tokens(&syntax, node.span);
        let scope = build_query_scope(
            &tokens,
            cursor,
            cursor_depth(&tokens, cursor),
            SqlIslandRoot::Query,
            &[relation("$selected", &["id", "label"])],
            None,
            None,
        );
        let store = scope
            .relations
            .iter()
            .find(|relation| relation.visible_name() == "s")
            .unwrap_or_else(|| panic!("store relation\ntokens: {tokens:#?}\nscope: {scope:#?}"));
        assert_eq!(store.columns.len(), 2);
    }

    #[test]
    fn datafusion_reconciliation_preserves_projection_alias_types() {
        let relation = RelationMetadata {
            path: vec!["vega".to_owned(), "movies".to_owned()],
            alias: None,
            columns: vec![ColumnMetadata {
                name: "rating".to_owned(),
                qualifier: Some("movies".to_owned()),
                data_type: DataType::Decimal128(2, 1),
                nullable: false,
                stage: "test".to_owned(),
                lineage: None,
                detail: None,
            }],
            detail: "test relation".to_owned(),
            scope_depth: 0,
        };
        let mut scope = QueryScope {
            relations: vec![relation.clone()],
            projection_aliases: vec![ColumnMetadata {
                name: "score".to_owned(),
                qualifier: None,
                data_type: DataType::Null,
                nullable: true,
                stage: "projection".to_owned(),
                lineage: Some("rating".to_owned()),
                detail: None,
            }],
            ..QueryScope::default()
        };
        reconcile_query_output_with_datafusion(
            "SELECT \"rating\" AS score FROM vega.movies ORDER BY \"score\"",
            &[relation],
            &mut scope,
            &[],
            0,
            0,
        );
        assert_eq!(scope.projection_aliases[0].name, "score");
        assert_eq!(
            scope.projection_aliases[0].data_type,
            DataType::Decimal128(2, 1)
        );
    }

    #[test]
    fn completion_metrics_never_claim_execution() {
        let before = SqlCompletionMetrics::snapshot();
        assert_eq!(before.physical_plans, 0);
        assert_eq!(before.scans, 0);
        assert_eq!(before.collects, 0);
        assert_eq!(before.executions, 0);
        assert_eq!(before.invalid_candidates, 0);
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
            let TolerantSyntaxNodeKind::SqlIsland { site, .. } = &node.kind else {
                unreachable!()
            };
            let context = site.context();
            let tokens = island_tokens(&syntax, node.span);
            let cursor_context = sql_cursor_context(&text, node.span, absolute);
            let prefix = cursor_context.decoded_prefix.as_str();
            let skeleton =
                build_query_skeleton(&text, node.span, &tokens, absolute, context.root());
            let intents = expected_intents(&tokens, absolute, context, prefix, &skeleton);
            let (_, attempts) = select_repair(&tokens, absolute, &intents);
            assert!(!intents.is_empty(), "{}", case["name"]);
            assert!(attempts <= MAX_REPAIR_ATTEMPTS, "{}", case["name"]);
        }
    }

    #[test]
    fn frozen_sql_corpus_completion_sweep_is_bounded_and_span_safe() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/sql-completion-corpus.json"))
                .unwrap();
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = SqlCompletionCache::default();
        for case in corpus["cases"].as_array().unwrap() {
            let marked = case["sql"].as_str().unwrap();
            let sql = marked.replacen("⟦cursor⟧", "", 1);
            let text = if case["root"] == "query" {
                format!(
                    "avenger 1; chart cartesian as chart {{ table sql as t {{ sql: {sql}; }} }}"
                )
            } else {
                format!(
                    "avenger 1; chart cartesian as chart {{ mark symbol {{ x: encoded {sql}; }} }}"
                )
            };
            let base = text.find(&sql).unwrap();
            let (origin, syntax) = syntax(&text);
            let semantic_index = WorkspaceSemanticIndex::build(
                &BTreeMap::from([(origin.clone(), syntax.clone())]),
                &BTreeMap::new(),
            );
            for relative in sql
                .char_indices()
                .map(|(offset, _)| offset)
                .chain(std::iter::once(sql.len()))
            {
                let request = PositionRequest {
                    source: origin.clone(),
                    byte_offset: base + relative,
                    source_revision: syntax.revision.clone(),
                };
                let output = complete_sql(
                    &request,
                    &syntax,
                    compiler.language_host().authoring_schema(),
                    &semantic_index,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &CompletionInvocation::Invoked,
                    true,
                    &cache,
                    &AnalysisCancellation::default(),
                )
                .unwrap_or_else(|| {
                    panic!(
                        "completion routing failed for {} at {relative}: {sql}",
                        case["name"]
                    )
                });
                assert!(
                    output.debug.repair_attempts <= MAX_REPAIR_ATTEMPTS,
                    "{} at {relative}",
                    case["name"]
                );
                assert!(output.items.iter().all(|item| {
                    item.replacement.range.start <= item.replacement.range.end
                        && item.replacement.range.end <= text.len()
                        && text.is_char_boundary(item.replacement.range.start)
                        && text.is_char_boundary(item.replacement.range.end)
                }));
            }
        }
    }

    #[test]
    fn sql_clause_item_and_delimiter_mutations_remain_bounded_and_span_safe() {
        let cases = [
            (
                "avenger 1; chart cartesian { table sql as result { sql: ",
                "WITH c AS (SELECT \"id\" FROM vega.movies) SELECT c.\"id\" FROM c JOIN vega.ratings AS r ON c.\"id\" = r.\"id\" WHERE c.\"id\" BETWEEN 1 AND 3 ORDER BY c.\"id\"",
                "; } }",
            ),
            (
                "avenger 1; chart cartesian { transform calculate { expressions: ",
                "\"id\" AS id, CASE WHEN \"rating\" > 0 THEN round(\"rating\") ELSE 0 END AS score",
                "; } }",
            ),
            (
                "avenger 1; chart cartesian { param 1 as minimum; mark symbol { x: encoded ",
                "CASE WHEN \"id\" BETWEEN 1 AND 3 THEN ($minimum + 1) ELSE 0 END",
                "; } }",
            ),
        ];
        let mutation_targets = [
            "(", ")", ",", "\"", ".", "SELECT", "FROM", "JOIN", "ON", "WHERE", "ORDER BY", "CASE",
            "END",
        ];
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = SqlCompletionCache::default();

        for (prefix, authored, suffix) in cases {
            for target in mutation_targets {
                for target_start in authored.match_indices(target).map(|(start, _)| start) {
                    for duplicate in [false, true] {
                        let mut mutated = authored.to_owned();
                        let cursor = if duplicate {
                            mutated.insert_str(target_start, target);
                            target_start + target.len()
                        } else {
                            mutated.replace_range(target_start..target_start + target.len(), "");
                            target_start
                                + mutated[target_start..]
                                    .char_indices()
                                    .find(|(_, character)| !character.is_whitespace())
                                    .map_or(0, |(offset, _)| offset)
                        };
                        let text = format!("{prefix}{mutated}{suffix}");
                        let absolute = prefix.len() + cursor;
                        let (origin, syntax) = syntax(&text);
                        let semantic_index = WorkspaceSemanticIndex::build(
                            &BTreeMap::from([(origin.clone(), syntax.clone())]),
                            &BTreeMap::new(),
                        );
                        let output = complete_sql(
                            &PositionRequest {
                                source: origin,
                                byte_offset: absolute,
                                source_revision: syntax.revision.clone(),
                            },
                            &syntax,
                            compiler.language_host().authoring_schema(),
                            &semantic_index,
                            &BTreeMap::new(),
                            &BTreeMap::new(),
                            &CompletionInvocation::Invoked,
                            true,
                            &cache,
                            &AnalysisCancellation::default(),
                        )
                        .unwrap_or_else(|| {
                            panic!(
                                "completion routing failed after {} {target:?} in {authored:?}",
                                if duplicate { "duplicating" } else { "deleting" }
                            )
                        });
                        assert!(
                            output.debug.repair_attempts <= MAX_REPAIR_ATTEMPTS,
                            "{target:?}: {mutated:?}"
                        );
                        assert!(
                            completion_invariants_hold(&output.items, output.debug.lexical_mode),
                            "{target:?}: {mutated:?}: {:#?}",
                            output.items
                        );
                        assert!(output.items.iter().all(|item| {
                            item.replacement.range.start <= item.replacement.range.end
                                && item.replacement.range.end <= text.len()
                                && text.is_char_boundary(item.replacement.range.start)
                                && text.is_char_boundary(item.replacement.range.end)
                        }));
                    }
                }
            }
        }
    }

    #[test]
    fn exact_sql_intents_distinguish_query_cursor_states() {
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = SqlCompletionCache::default();
        let complete = |marked: &str| {
            let cursor = marked.find('|').expect("cursor marker");
            let sql = marked.replacen('|', "", 1);
            let prefix = "avenger 1; chart cartesian { table sql as result { sql: ";
            let text = format!("{prefix}{sql}; }} }}");
            let absolute = prefix.len() + cursor;
            let (origin, syntax) = syntax(&text);
            let semantic_index = WorkspaceSemanticIndex::build(
                &BTreeMap::from([(origin.clone(), syntax.clone())]),
                &BTreeMap::new(),
            );
            complete_sql(
                &PositionRequest {
                    source: origin,
                    byte_offset: absolute,
                    source_revision: syntax.revision.clone(),
                },
                &syntax,
                compiler.language_host().authoring_schema(),
                &semantic_index,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &CompletionInvocation::Invoked,
                true,
                &cache,
                &AnalysisCancellation::default(),
            )
            .unwrap()
            .debug
            .intents
        };

        assert_eq!(complete("|"), BTreeSet::from([SqlIntent::QueryStart]));
        assert_eq!(
            complete("SELECT |"),
            BTreeSet::from([
                SqlIntent::ClauseTransition,
                SqlIntent::ExpressionOperand,
                SqlIntent::FunctionName,
            ])
        );
        assert_eq!(
            complete("SELECT 1 |"),
            BTreeSet::from([SqlIntent::ClauseTransition, SqlIntent::ExpressionOperator,])
        );
        assert_eq!(
            complete("SELECT * FROM |"),
            BTreeSet::from([SqlIntent::RelationPath])
        );
        assert_eq!(complete("WITH |"), BTreeSet::from([SqlIntent::CteName]));
        assert_eq!(
            complete("WITH current AS |"),
            BTreeSet::from([SqlIntent::CteBodyStart])
        );
        assert_eq!(
            complete("SELECT * FROM movies AS |"),
            BTreeSet::from([SqlIntent::NameBinder])
        );
        assert_eq!(
            complete("SELECT CAST(1 AS |)"),
            BTreeSet::from([SqlIntent::TypeName])
        );
        assert_eq!(
            complete("SELECT m.\"| FROM movies AS m"),
            BTreeSet::from([SqlIntent::QuotedColumn, SqlIntent::QualifiedMember])
        );
        assert_eq!(
            complete("SELECT * EX| FROM movies"),
            BTreeSet::from([SqlIntent::WildcardModifier])
        );
        assert_eq!(
            complete("SELECT sum(1) OVER | FROM movies WINDOW recent AS (ORDER BY \"id\")"),
            BTreeSet::from([SqlIntent::WindowName])
        );
    }

    #[test]
    fn all_nine_sql_sites_match_a_270_case_exact_cursor_matrix() {
        use avenger_lang_core::syntax::SqlIslandSite;

        fn wrap(site: SqlIslandSite, marked: &str) -> (String, usize) {
            let relative = marked.find('|').expect("cursor marker");
            let fragment = marked.replacen('|', "", 1);
            let (prefix, suffix) = match site {
                SqlIslandSite::QueryProperty => {
                    ("avenger 1; chart cartesian { table sql { sql: ", "; } }")
                }
                SqlIslandSite::ProjectionProperty => (
                    "avenger 1; chart cartesian { transform calculate { expressions: ",
                    "; } }",
                ),
                SqlIslandSite::ChannelModePayload => (
                    "avenger 1; chart cartesian { mark symbol { x: encoded ",
                    "; } }",
                ),
                SqlIslandSite::PropertyValue => (
                    "avenger 1; chart cartesian { transform filter { predicate: ",
                    "; } }",
                ),
                SqlIslandSite::CursorActionRhs => (
                    "avenger 1; chart cartesian { on click { set cursor = ",
                    "; } }",
                ),
                SqlIslandSite::StateActionRhs => (
                    "avenger 1; chart cartesian { param 1 as width; on click { set width = ",
                    "; } }",
                ),
                SqlIslandSite::ArrayElement => (
                    "avenger 1; chart cartesian { table inline { values: [",
                    "]; } }",
                ),
                SqlIslandSite::ParamInitializer => (
                    "avenger 1; chart cartesian { param ",
                    if fragment.is_empty() {
                        "as width; }"
                    } else {
                        " as width; }"
                    },
                ),
                SqlIslandSite::OutputSource => (
                    "avenger 1; define transform sample { output ",
                    if fragment.is_empty() {
                        "as result; }"
                    } else {
                        " as result; }"
                    },
                ),
            };
            (
                format!("{prefix}{fragment}{suffix}"),
                prefix.len() + relative,
            )
        }

        let query_cases = [
            "|",
            "S|",
            "SELECT |",
            "SELECT 1|",
            "SELECT 1 |",
            "SELECT \"|",
            "SELECT m.\"| FROM movies AS m",
            "SELECT * FROM |",
            "SELECT * FROM veg|",
            "SELECT * FROM vega.|",
            "FROM movies AS m SELECT m.\"|",
            "WITH |",
            "WITH c |",
            "WITH c AS |",
            "WITH c AS (SELECT 1) SELECT |",
            "SELECT * FROM movies JOIN |",
            "SELECT * FROM movies JOIN ratings ON |",
            "SELECT * FROM movies JOIN ratings USING (\"|)",
            "SELECT * FROM movies WHERE |",
            "SELECT * FROM movies GROUP BY |",
            "SELECT * FROM movies HAVING |",
            "SELECT row_number() OVER () FROM movies QUALIFY |",
            "SELECT * FROM movies ORDER BY |",
            "SELECT * FROM movies LIMIT |",
            "SELECT * FROM movies OFFSET |",
            "SELECT * FROM movies FETCH |",
            "SELECT * EX| FROM movies",
            "SELECT sum(1) OVER | FROM movies WINDOW w AS (ORDER BY \"id\")",
            "VALUES (|)",
            "SELECT 1 UNION ALL |",
        ];
        let projection_cases = [
            "|",
            "C|",
            "CAST(|",
            "CAST(1 AS |)",
            "\"|",
            "\"va|",
            "$|",
            "$wid|",
            "1|",
            "1 |",
            "1 + |",
            "CASE |",
            "CASE WHEN |",
            "CASE WHEN true THEN |",
            "CASE WHEN true THEN 1 ELSE | END",
            "round(|)",
            "coalesce(1, |)",
            "1 AS |",
            "1 AS output, |",
            "*|",
            "*, |",
            "\"a\", \"|",
            "(|)",
            "((1 + |))",
            "NULL|",
            "TRUE|",
            "NOT |",
            "-|",
            "1::|",
            "/* note|",
        ];
        let expression_cases = [
            "|",
            "C|",
            "CA|",
            "CAST(|",
            "CAST(1 AS |)",
            "\"|",
            "\"va|",
            "$|",
            "$wid|",
            "1|",
            "1 |",
            "1 + |",
            "CASE |",
            "CASE WHEN |",
            "CASE WHEN true THEN |",
            "CASE WHEN true THEN 1 ELSE | END",
            "round(|)",
            "coalesce(1, |)",
            "(|)",
            "((1 + |))",
            "NULL|",
            "TRUE|",
            "NOT |",
            "-|",
            "1::|",
            "'text|",
            "$$raw|",
            "/* note|",
            "1 -- note|\n",
            "event.|",
        ];

        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = SqlCompletionCache::default();
        let mut executed = 0usize;
        let mut exact_digests = BTreeMap::<SqlIslandSite, Sha256>::new();
        for site in SqlIslandSite::ALL {
            let cases: &[&str] = match site {
                SqlIslandSite::QueryProperty => &query_cases,
                SqlIslandSite::ProjectionProperty => &projection_cases,
                _ => &expression_cases,
            };
            assert_eq!(cases.len(), 30);
            for marked in cases {
                let (text, cursor) = wrap(site, marked);
                let (origin, syntax) = syntax(&text);
                let semantic_index = WorkspaceSemanticIndex::build(
                    &BTreeMap::from([(origin.clone(), syntax.clone())]),
                    &BTreeMap::new(),
                );
                let output = complete_sql(
                    &PositionRequest {
                        source: origin,
                        byte_offset: cursor,
                        source_revision: syntax.revision.clone(),
                    },
                    &syntax,
                    compiler.language_host().authoring_schema(),
                    &semantic_index,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    &CompletionInvocation::Invoked,
                    true,
                    &cache,
                    &AnalysisCancellation::default(),
                )
                .unwrap_or_else(|| panic!("site {site:?} did not route `{marked}` in `{text}`"));
                assert_eq!(output.debug.site, site, "{marked}");
                assert!(
                    output.debug.repair_attempts <= MAX_REPAIR_ATTEMPTS,
                    "{site:?}: {marked}"
                );
                assert!(
                    completion_invariants_hold(&output.items, output.debug.lexical_mode),
                    "{site:?}: {marked}: {:#?}",
                    output.items
                );
                assert!(output.items.iter().all(|item| {
                    item.replacement.range.start <= item.replacement.range.end
                        && item.replacement.range.end <= text.len()
                        && text.is_char_boundary(item.replacement.range.start)
                        && text.is_char_boundary(item.replacement.range.end)
                }));
                let mut identities = output
                    .items
                    .iter()
                    .map(|item| {
                        format!(
                            "{}:{:?}:{}:{:?}",
                            item.semantic_identity,
                            item.replacement,
                            item.insert_text,
                            item.insert_text_format
                        )
                    })
                    .collect::<Vec<_>>();
                identities.sort();
                identities.dedup();
                assert_eq!(identities.len(), output.items.len(), "{site:?}: {marked}");
                let digest = exact_digests.entry(site).or_default();
                digest.update(marked.as_bytes());
                digest.update(b"\0");
                digest.update(format!("{:#?}", output.debug).as_bytes());
                digest.update(b"\0");
                digest.update(format!("{:#?}", output.items).as_bytes());
                digest.update(b"\0");
                executed += 1;
            }
        }
        assert_eq!(executed, 270);
        let actual = SqlIslandSite::ALL
            .into_iter()
            .map(|site| {
                let digest = exact_digests.remove(&site).unwrap().finalize();
                (site, format!("{digest:x}"))
            })
            .collect::<Vec<_>>();
        let expected = [
            (
                SqlIslandSite::QueryProperty,
                "c1e469f5861ae789a777938c3bd13d6fc11f5330b98bb98d8002c83d71446842",
            ),
            (
                SqlIslandSite::ProjectionProperty,
                "95e8728ddef1a5f099c82dbcfeb35f0d49491cedb43f5145a779d1cb60d4c18c",
            ),
            (
                SqlIslandSite::ChannelModePayload,
                "9de1408435ee69d589dc1b9a6ab48c7a4745a875f526c30f37eb455d8aedc6f0",
            ),
            (
                SqlIslandSite::PropertyValue,
                "79da320f1bfdbd3dcfc1db870a0bf1d90f5a533ef78523214906765e7b601fe1",
            ),
            (
                SqlIslandSite::ArrayElement,
                "9e0329ccd92796ec26e04633820afdc3ef4a2076764421b3b46c0efba5d68c51",
            ),
            (
                SqlIslandSite::ParamInitializer,
                "b3ab22f82fab79c55697ffe41a6abde392094f5968b0d339c01dc7173a433a74",
            ),
            (
                SqlIslandSite::OutputSource,
                "cd908a13dceb5a4ee6cec46f85a99187d84c4ecacdb8c50b791b55e1c36ee3f0",
            ),
            (
                SqlIslandSite::CursorActionRhs,
                "2fc70d5f5793cd97be498823c83f466ba52dac8b382043ccdc91c4e83f7d4e6b",
            ),
            (
                SqlIslandSite::StateActionRhs,
                "bba30421ce2575a9fa42b21cb19e572a74c3262768625acf2ff6a307bbd5b974",
            ),
        ]
        .map(|(site, digest)| (site, digest.to_owned()))
        .to_vec();
        assert_eq!(actual, expected);
    }

    #[test]
    fn lexical_cursor_modes_and_replacement_ranges_are_quote_aware() {
        fn context(marked: &str) -> (String, usize, SqlCursorContext) {
            let cursor = marked.find('|').expect("cursor marker");
            let expression = marked.replacen('|', "", 1);
            let text = format!(
                "avenger 1; chart cartesian as chart {{ mark symbol {{ x: encoded {expression}\n; }} }}"
            );
            let absolute = text.find(&expression).unwrap() + cursor;
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
                .unwrap_or_else(|| panic!("missing SQL island for `{marked}` in `{text}`"));
            let context = sql_cursor_context(&text, node.span, absolute);
            (text, absolute, context)
        }

        for (marked, mode) in [
            ("round(|)", SqlLexicalMode::Code),
            ("'text|", SqlLexicalMode::SingleQuotedString),
            ("1 -- note|", SqlLexicalMode::LineComment),
            ("1 /* note|", SqlLexicalMode::BlockComment),
            ("$$raw|", SqlLexicalMode::DollarQuotedString),
            ("$tag$raw|", SqlLexicalMode::DollarQuotedString),
            ("$a$b|", SqlLexicalMode::DollarQuotedString),
            ("$$param|", SqlLexicalMode::DollarQuotedString),
            ("$point|", SqlLexicalMode::Binding),
            ("$point@sta|", SqlLexicalMode::TemporalQualifier),
        ] {
            let (_, _, context) = context(marked);
            assert_eq!(context.mode, mode, "{marked}");
        }

        assert_eq!(context("'text|").2.synthetic_closer.as_deref(), Some("'"));
        assert_eq!(context("$$raw|").2.synthetic_closer.as_deref(), Some("$$"));
        assert_eq!(
            context("$tag$raw|").2.synthetic_closer.as_deref(),
            Some("$tag$")
        );
        assert_eq!(
            context("1 /* note|").2.synthetic_closer.as_deref(),
            Some("*/")
        );
        assert_eq!(context("m.\"ra|").2.synthetic_closer.as_deref(), Some("\""));

        let (text, cursor, quoted) = context("m.\"ra|ting\"");
        assert_eq!(quoted.mode, SqlLexicalMode::DoubleQuotedIdentifier);
        assert_eq!(quoted.decoded_prefix, "ra");
        assert_eq!(&text[quoted.replacement.range.as_range()], "\"rating\"");
        assert!(quoted.replacement.range.start < cursor);
        assert!(cursor < quoted.replacement.range.end);
        assert_eq!(quoted.synthetic_closer, None);

        let (text, _, escaped) = context("\"a\"\"b|\"");
        assert_eq!(escaped.decoded_prefix, "a\"b");
        assert_eq!(&text[escaped.replacement.range.as_range()], "\"a\"\"b\"");

        let (text, cursor, before_clause) =
            context("d.\"| FROM (SELECT * EXCLUDE (\"id\") FROM movies) AS d");
        assert_eq!(before_clause.mode, SqlLexicalMode::DoubleQuotedIdentifier);
        assert_eq!(&text[before_clause.replacement.range.as_range()], "\"");
        assert_eq!(before_clause.replacement.range.end, cursor);
        assert_eq!(before_clause.synthetic_closer.as_deref(), Some("\""));

        let (_, cursor, member) = context("event.coord.|");
        assert_eq!(member.mode, SqlLexicalMode::Code);
        assert_eq!(member.replacement.range, ByteSpan::empty(cursor));
        assert_eq!(member.decoded_prefix, "");

        let text = "\"address\".city.";
        let island = SourceSpan {
            source: SourceId::new(0),
            range: ByteSpan {
                start: 0,
                end: text.len(),
            },
        };
        assert_eq!(
            qualifier_before(text, island, text.len()).as_deref(),
            Some("address.city")
        );
        let escaped = "\"a\"\"b\".";
        let island = SourceSpan {
            source: SourceId::new(0),
            range: ByteSpan {
                start: 0,
                end: escaped.len(),
            },
        };
        assert_eq!(
            qualifier_before(escaped, island, escaped.len()).as_deref(),
            Some("a\"b")
        );
    }

    #[test]
    fn workspace_sql_cache_is_lru_and_bounded_by_entries_and_bytes() {
        fn analysis() -> Arc<CachedSqlAnalysis> {
            Arc::new(CachedSqlAnalysis {
                intents: BTreeSet::from([SqlIntent::ExpressionOperand]),
                repair: SqlRepairStrategy::None,
                sentinel: None,
                repair_attempts: 1,
                token_count: 1,
                catalog: Vec::new(),
                scope: QueryScope::default(),
                expression_planned: false,
                repaired_parse: false,
                synthetic_ranges: Vec::new(),
            })
        }

        let cache = SqlCompletionCache::default();
        for index in 0..SQL_CACHE_MAX_ENTRIES {
            cache.insert(format!("key-{index:04}"), analysis());
        }
        assert!(cache.get("key-0000").is_some());
        cache.insert("newest".to_owned(), analysis());
        assert!(cache.get("key-0000").is_some(), "recent entry was evicted");
        assert!(cache.get("key-0001").is_none(), "oldest entry survived");
        let (entries, bytes) = cache.len_and_bytes();
        assert!(entries <= SQL_CACHE_MAX_ENTRIES);
        assert!(bytes <= SQL_CACHE_MAX_BYTES);
    }

    #[test]
    fn sql_cache_identity_tracks_authored_and_semantic_generations() {
        let source = SourceOrigin::Memory("cache.avenger".to_owned());
        let island = SourceSpan {
            source: SourceId::new(0),
            range: ByteSpan { start: 10, end: 20 },
        };
        let request = PositionRequest {
            source: source.clone(),
            byte_offset: 15,
            source_revision: SourceRevision::new("revision-a"),
        };
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let mut project = compiler.analyze_phase0_empty();
        project.module_fingerprint = ModuleFingerprint::new("generation-a");
        let original = sql_cache_key(&request, island, Some(&project), None);

        let mut revised = request.clone();
        revised.source_revision = SourceRevision::new("revision-b");
        assert_ne!(
            original,
            sql_cache_key(&revised, island, Some(&project), None)
        );

        let mut moved = request.clone();
        moved.byte_offset += 1;
        assert_ne!(
            original,
            sql_cache_key(&moved, island, Some(&project), None)
        );

        project.module_fingerprint = ModuleFingerprint::new("generation-b");
        assert_ne!(
            original,
            sql_cache_key(&request, island, Some(&project), None)
        );
    }

    #[test]
    fn workspace_sql_cache_is_safe_under_concurrent_requests() {
        fn analysis(index: usize) -> Arc<CachedSqlAnalysis> {
            Arc::new(CachedSqlAnalysis {
                intents: BTreeSet::from([SqlIntent::ExpressionOperand]),
                repair: SqlRepairStrategy::None,
                sentinel: None,
                repair_attempts: 1,
                token_count: index % 7,
                catalog: Vec::new(),
                scope: QueryScope::default(),
                expression_planned: false,
                repaired_parse: false,
                synthetic_ranges: Vec::new(),
            })
        }

        let cache = Arc::new(SqlCompletionCache::default());
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let cache = Arc::clone(&cache);
                scope.spawn(move || {
                    for index in 0..512 {
                        // Keep the shared working set below the cache's entry
                        // bound. Eviction itself is covered separately; this
                        // test isolates lock safety and concurrent visibility.
                        let key = format!("worker-{worker}-{}", index % 16);
                        cache.insert(key.clone(), analysis(index));
                        assert!(cache.get(&key).is_some());
                    }
                });
            }
        });
        for worker in 0..8 {
            for index in 0..16 {
                assert!(cache.get(&format!("worker-{worker}-{index}")).is_some());
            }
        }
        let (entries, bytes) = cache.len_and_bytes();
        assert!(entries <= SQL_CACHE_MAX_ENTRIES);
        assert!(bytes <= SQL_CACHE_MAX_BYTES);
    }

    #[test]
    fn cancelled_completion_does_not_publish_or_populate_cache() {
        let text = "avenger 1; chart cartesian { table sql { sql: SELECT 1; } }";
        let cursor = text.find("SELECT 1").unwrap() + "SELECT ".len();
        let (origin, syntax) = syntax(text);
        let semantic_index = WorkspaceSemanticIndex::build(
            &BTreeMap::from([(origin.clone(), syntax.clone())]),
            &BTreeMap::new(),
        );
        let compiler = Compiler::builder().project_root("/tmp").build().unwrap();
        let cache = SqlCompletionCache::default();
        let cancellation = AnalysisCancellation::default();
        cancellation.cancel();
        assert!(
            complete_sql(
                &PositionRequest {
                    source: origin,
                    byte_offset: cursor,
                    source_revision: syntax.revision.clone(),
                },
                &syntax,
                compiler.language_host().authoring_schema(),
                &semantic_index,
                &BTreeMap::new(),
                &BTreeMap::new(),
                &CompletionInvocation::Invoked,
                true,
                &cache,
                &cancellation,
            )
            .is_none()
        );
        assert_eq!(cache.len_and_bytes(), (0, 0));
    }

    #[test]
    fn sqlparser_token_slices_preserve_original_locations() {
        let dialect = avenger_lang_core::sql::AvengerSqlDialect;
        let sql = "SELECT\n  m.\"rating\" + 1\nFROM vega.movies AS m";
        let tokens = Tokenizer::new(&dialect, sql)
            .tokenize_with_location()
            .unwrap();
        let select = tokens
            .iter()
            .position(|token| matches!(&token.token, Token::Word(word) if word.value.eq_ignore_ascii_case("select")))
            .unwrap();
        let from = tokens
            .iter()
            .position(|token| matches!(&token.token, Token::Word(word) if word.value.eq_ignore_ascii_case("from")))
            .unwrap();
        let expression = Parser::new(&dialect)
            .with_tokens_with_locations(tokens[select + 1..from].to_vec())
            .parse_expr()
            .unwrap();
        let span = expression.span();
        assert_eq!(span.start.line, 2);
        assert_eq!(span.start.column, 3);
        assert_eq!(span.end.line, 2);
        assert!(span.end.column > span.start.column);

        let select_item_sql = "\n\nm.\"rating\" AS score";
        let select_item_tokens = Tokenizer::new(&dialect, select_item_sql)
            .tokenize_with_location()
            .unwrap();
        let select_item = Parser::new(&dialect)
            .with_tokens_with_locations(select_item_tokens)
            .parse_select_item()
            .unwrap();
        assert_eq!(select_item.span().start.line, 3);

        let relation_sql = "\nvega.movies AS m JOIN vega.ratings AS r ON m.\"id\" = r.\"id\"";
        let relation_tokens = Tokenizer::new(&dialect, relation_sql)
            .tokenize_with_location()
            .unwrap();
        let relation = Parser::new(&dialect)
            .with_tokens_with_locations(relation_tokens)
            .parse_table_and_joins()
            .unwrap();
        assert_eq!(relation.span().start.line, 2);

        let query_tokens = Tokenizer::new(&dialect, sql)
            .tokenize_with_location()
            .unwrap();
        let query = Parser::new(&dialect)
            .with_tokens_with_locations(query_tokens.clone())
            .parse_query()
            .unwrap();
        assert_eq!(query.span().start.line, 1);
        assert!(query.span().end.line >= 3);

        let locationless = Parser::new(&dialect)
            .with_tokens(query_tokens.into_iter().map(|token| token.token).collect())
            .parse_query()
            .unwrap();
        assert_eq!(locationless.span(), sqlparser::tokenizer::Span::empty());
    }

    #[test]
    fn quoted_column_insertions_round_trip_every_supported_identifier_shape() {
        let dialect = avenger_lang_core::sql::AvengerSqlDialect;
        for name in [
            "id",
            "SELECT",
            "two words",
            "a.b",
            "naïve",
            "東京",
            "a\"b",
            "$value",
            "@start",
        ] {
            let quoted = quote_identifier(name);
            let tokens = Tokenizer::new(&dialect, &quoted)
                .tokenize_with_location()
                .unwrap_or_else(|error| panic!("failed to tokenize {quoted:?}: {error}"));
            let word = tokens
                .iter()
                .find_map(|token| match &token.token {
                    Token::Word(word) => Some(word),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing quoted identifier token for {quoted:?}"));
            assert_eq!(word.quote_style, Some('"'), "{quoted:?}");
            assert_eq!(word.value, name, "{quoted:?}");

            Parser::new(&dialect)
                .try_with_sql(&format!("SELECT {quoted}"))
                .unwrap()
                .parse_query()
                .unwrap_or_else(|error| panic!("failed to parse {quoted:?}: {error}"));
        }
    }

    #[test]
    fn completion_owned_closers_make_unterminated_tokens_lexable_without_authored_edits() {
        let dialect = avenger_lang_core::sql::AvengerSqlDialect;
        for (authored, closer) in [
            ("m.\"rating", "\""),
            ("'multiline\ntext", "'"),
            ("/* nested /* note */", "*/"),
            ("$tag$multiline\ntext", "$tag$"),
        ] {
            assert!(
                Tokenizer::new(&dialect, authored)
                    .tokenize_with_location()
                    .is_err(),
                "fixture unexpectedly lexed without a closer: {authored:?}"
            );
            let patched = format!("{authored}{closer}");
            let tokens = Tokenizer::new(&dialect, &patched)
                .tokenize_with_location()
                .unwrap_or_else(|error| panic!("patched {authored:?}: {error}"));
            assert!(!tokens.is_empty());
            assert_eq!(patched.len() - closer.len(), authored.len());
        }
    }

    #[test]
    fn query_skeleton_retains_nested_blocks_and_independent_item_status() {
        let text = r#"avenger 1; chart cartesian as chart {
  table sql as t {
    sql: WITH c AS (SELECT "id" FROM vega.movies)
         SELECT "id", +, "title"
         FROM c
         WHERE EXISTS (SELECT 1 FROM vega.ratings AS r WHERE r."movie_id" = "id");
  }
}"#;
        let cursor = text.find("r.\"movie_id\"").unwrap() + "r.\"mo".len();
        let (_, syntax) = syntax(text);
        let node = syntax
            .parsed
            .nodes
            .iter()
            .filter(|node| {
                matches!(node.kind, TolerantSyntaxNodeKind::SqlIsland { .. })
                    && node.span.range.start <= cursor
                    && cursor <= node.span.range.end
            })
            .min_by_key(|node| node.span.range.len())
            .unwrap();
        let tokens = island_tokens(&syntax, node.span);
        let skeleton = build_query_skeleton(text, node.span, &tokens, cursor, SqlIslandRoot::Query);
        assert_eq!(skeleton.blocks.len(), 3, "{:#?}", skeleton.blocks);
        let active = &skeleton.blocks[skeleton.active_block];
        assert!(active.active);
        assert_eq!(active.parent, Some(0));
        assert_eq!(active.depth, 1);
        assert_eq!(skeleton.active_clause(&tokens, cursor), QueryClause::Where);

        let root = &skeleton.blocks[0];
        let select = root
            .items
            .iter()
            .filter(|item| item.clause == "select")
            .collect::<Vec<_>>();
        assert_eq!(select.len(), 3, "{select:#?}");
        assert_eq!(select[0].status, SqlItemParseStatus::Parsed);
        assert_eq!(select[1].status, SqlItemParseStatus::Unparsed);
        assert_eq!(select[2].status, SqlItemParseStatus::Parsed);
    }
}
