use std::any::TypeId;

use sqlparser::dialect::{Dialect, GenericDialect};

/// The single SQL dialect used by Avenger tokenization and SQL islands.
///
/// This deliberately pins the complete feature surface enabled by
/// `GenericDialect` in sqlparser 0.62. Keeping the list explicit makes a
/// sqlparser upgrade a language-compatibility review rather than silently
/// inheriting new or changed syntax.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AvengerSqlDialect;

impl AvengerSqlDialect {
    pub const fn new() -> Self {
        Self
    }
}

impl Dialect for AvengerSqlDialect {
    fn dialect(&self) -> TypeId {
        // Preserve Generic identity because sqlparser gates some behavior on
        // `dialect.is::<GenericDialect>()` rather than capability methods.
        TypeId::of::<GenericDialect>()
    }

    fn is_delimited_identifier_start(&self, ch: char) -> bool {
        ch == '"' || ch == '`'
    }

    fn is_identifier_start(&self, ch: char) -> bool {
        ch.is_alphabetic() || ch == '_' || ch == '#' || ch == '@'
    }

    fn is_identifier_part(&self, ch: char) -> bool {
        ch.is_alphabetic()
            || ch.is_ascii_digit()
            || ch == '@'
            || ch == '$'
            || ch == '#'
            || ch == '_'
    }

    fn requires_single_line_comment_whitespace(&self) -> bool {
        true
    }

    fn supports_unicode_string_literal(&self) -> bool {
        true
    }
    fn supports_partition_by_after_order_by(&self) -> bool {
        true
    }
    fn supports_array_join_syntax(&self) -> bool {
        true
    }
    fn supports_group_by_expr(&self) -> bool {
        true
    }
    fn supports_group_by_with_modifier(&self) -> bool {
        true
    }
    fn supports_left_associative_joins_without_parens(&self) -> bool {
        true
    }
    fn supports_connect_by(&self) -> bool {
        true
    }
    fn supports_match_recognize(&self) -> bool {
        true
    }
    fn supports_pipe_operator(&self) -> bool {
        true
    }
    fn supports_start_transaction_modifier(&self) -> bool {
        true
    }
    fn supports_window_function_null_treatment_arg(&self) -> bool {
        true
    }
    fn supports_dictionary_syntax(&self) -> bool {
        true
    }
    fn supports_window_clause_named_window_reference(&self) -> bool {
        true
    }
    fn supports_parenthesized_set_variables(&self) -> bool {
        true
    }
    fn supports_select_wildcard_except(&self) -> bool {
        true
    }
    fn support_map_literal_syntax(&self) -> bool {
        true
    }
    fn allow_extract_custom(&self) -> bool {
        true
    }
    fn allow_extract_single_quotes(&self) -> bool {
        true
    }
    fn supports_extract_comma_syntax(&self) -> bool {
        true
    }
    fn supports_create_view_comment_syntax(&self) -> bool {
        true
    }
    fn supports_parens_around_table_factor(&self) -> bool {
        true
    }
    fn supports_values_as_table_factor(&self) -> bool {
        true
    }
    fn supports_create_index_with_clause(&self) -> bool {
        true
    }
    fn supports_explain_with_utility_options(&self) -> bool {
        true
    }
    fn supports_limit_comma(&self) -> bool {
        true
    }
    fn supports_update_order_by(&self) -> bool {
        true
    }
    fn supports_from_first_select(&self) -> bool {
        true
    }
    fn supports_projection_trailing_commas(&self) -> bool {
        true
    }
    fn supports_asc_desc_in_column_definition(&self) -> bool {
        true
    }
    fn supports_try_convert(&self) -> bool {
        true
    }
    fn supports_bitwise_shift_operators(&self) -> bool {
        true
    }
    fn supports_comment_on(&self) -> bool {
        true
    }
    fn supports_load_extension(&self) -> bool {
        true
    }
    fn supports_named_fn_args_with_assignment_operator(&self) -> bool {
        true
    }
    fn supports_struct_literal(&self) -> bool {
        true
    }
    fn supports_empty_projections(&self) -> bool {
        true
    }
    fn supports_nested_comments(&self) -> bool {
        true
    }
    fn supports_multiline_comment_hints(&self) -> bool {
        true
    }
    fn supports_user_host_grantee(&self) -> bool {
        true
    }
    fn supports_string_escape_constant(&self) -> bool {
        true
    }
    fn supports_array_typedef_with_brackets(&self) -> bool {
        true
    }
    fn supports_match_against(&self) -> bool {
        true
    }
    fn supports_set_names(&self) -> bool {
        true
    }
    fn supports_comma_separated_set_assignments(&self) -> bool {
        true
    }
    fn supports_filter_during_aggregation(&self) -> bool {
        true
    }
    fn supports_select_wildcard_exclude(&self) -> bool {
        true
    }
    fn supports_data_type_signed_suffix(&self) -> bool {
        true
    }
    fn supports_interval_options(&self) -> bool {
        true
    }
    fn supports_quote_delimited_string(&self) -> bool {
        true
    }
    fn supports_select_wildcard_replace(&self) -> bool {
        true
    }
    fn supports_select_wildcard_ilike(&self) -> bool {
        true
    }
    fn supports_select_wildcard_rename(&self) -> bool {
        true
    }
    fn supports_optimize_table(&self) -> bool {
        true
    }
    fn supports_install(&self) -> bool {
        true
    }
    fn supports_detach(&self) -> bool {
        true
    }
    fn supports_prewhere(&self) -> bool {
        true
    }
    fn supports_with_fill(&self) -> bool {
        true
    }
    fn supports_limit_by(&self) -> bool {
        true
    }
    fn supports_interpolate(&self) -> bool {
        true
    }
    fn supports_settings(&self) -> bool {
        true
    }
    fn supports_select_format(&self) -> bool {
        true
    }
    fn supports_comment_optimizer_hint(&self) -> bool {
        true
    }
    fn supports_constraint_keyword_without_name(&self) -> bool {
        true
    }
    fn supports_key_column_option(&self) -> bool {
        true
    }
    fn supports_comma_separated_trim(&self) -> bool {
        true
    }
    fn supports_cte_without_as(&self) -> bool {
        true
    }
    fn supports_select_item_multi_column_alias(&self) -> bool {
        true
    }
    fn supports_xml_expressions(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use sqlparser::dialect::{Dialect, GenericDialect};

    use super::AvengerSqlDialect;

    #[test]
    fn avenger_dialect_preserves_generic_identity_and_pins_overrides() {
        let dialect = AvengerSqlDialect::new();
        let dialect: &dyn Dialect = &dialect;
        assert!(dialect.is::<GenericDialect>());
        assert!(dialect.requires_single_line_comment_whitespace());
        assert!(dialect.supports_nested_comments());
        assert!(dialect.supports_from_first_select());
    }

    #[test]
    fn avenger_dialect_pins_representative_generic_features() {
        let dialect = AvengerSqlDialect::new();
        assert!(dialect.supports_struct_literal());
        assert!(dialect.supports_projection_trailing_commas());
        assert!(dialect.supports_array_typedef_with_brackets());
        assert!(dialect.supports_string_escape_constant());
    }
}
