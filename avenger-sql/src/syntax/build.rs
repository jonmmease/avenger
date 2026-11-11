//! Generated syntax: queries and selects, which take the spans of the constructs that they lower.

use datafusion::sql::sqlparser::ast::{
    helpers::attached_token::AttachedToken, GroupByExpr, Query, Select, SelectFlavor, SelectItem,
    SetExpr, TableFactor, TableWithJoins, WildcardAdditionalOptions,
};

/// A query of `body` without operators.
pub(crate) fn query(body: Select) -> Query {
    Query {
        with: None,
        body: Box::new(SetExpr::Select(Box::new(body))),
        order_by: None,
        limit_clause: None,
        fetch: None,
        locks: Vec::new(),
        for_clause: None,
        settings: None,
        format_clause: None,
        pipe_operators: Vec::new(),
    }
}

/// The projection `*`.
pub(crate) fn wildcard() -> SelectItem {
    SelectItem::Wildcard(WildcardAdditionalOptions::default())
}

/// The derived table of `query`, without an alias, which keeps the qualifiers of `query` in
/// DataFusion 55.2.
pub(crate) fn derived(query: Query) -> TableFactor {
    TableFactor::Derived {
        lateral: false,
        subquery: Box::new(query),
        alias: None,
        sample: None,
    }
}

/// `SELECT <projection> FROM <from>`, or a `SELECT` without `FROM`.
pub(crate) fn select(projection: Vec<SelectItem>, from: Option<TableFactor>) -> Select {
    let from = from.map(|relation| TableWithJoins {
        relation,
        joins: Vec::new(),
    });
    Select {
        select_token: AttachedToken::empty(),
        optimizer_hints: Vec::new(),
        distinct: None,
        select_modifiers: None,
        top: None,
        top_before_distinct: false,
        projection,
        exclude: None,
        into: None,
        from: from.into_iter().collect(),
        lateral_views: Vec::new(),
        prewhere: None,
        selection: None,
        connect_by: Vec::new(),
        group_by: GroupByExpr::Expressions(Vec::new(), Vec::new()),
        cluster_by: Vec::new(),
        distribute_by: Vec::new(),
        sort_by: Vec::new(),
        having: None,
        named_window: Vec::new(),
        qualify: None,
        window_before_qualify: false,
        value_table_mode: None,
        flavor: SelectFlavor::Standard,
    }
}
