//! The refinement of the syntax highlighter's ranges by the semantic model.

use avenger_sql_syntax::{Highlight, HighlightRange, NameRole, TextRange};

use super::model::{Definition, Model};

impl Model {
    /// Gives each range of `ranges` the class that the pass resolved, where the syntax alone
    /// cannot tell it: a name that refers to a definition takes the role of that definition, so
    /// a script variable that the syntax takes for a column is a `Variable`, and a function is a
    /// scalar, an aggregate, a window, or a table function, a mark, a constructor, or a
    /// transform. A string literal that a parameter takes as one value of a set is an
    /// `EnumValue`.
    ///
    /// `ranges` are the ranges of [`avenger_sql_syntax::highlight`] over the tree that this
    /// model's analysis read: a model of other text gives wrong classes, and this method does not
    /// check that. A range that the model has nothing on keeps its class, as does a name that a
    /// node defines, so the names after an error keep the classes of the syntax. A use of a table
    /// spans its whole path, so each name of the path takes its class.
    ///
    /// ```
    /// use avenger_sql::{analysis::Options, analyze, catalog::MemoryCatalog, library::Library};
    /// use avenger_sql_syntax::{highlight, parse, Highlight, NameRole};
    ///
    /// let text = "DECLARE lo INT64 DEFAULT 1;\nFROM UNNEST([1, 2, 3]) AS n |> WHERE n > lo";
    /// let parse = parse(text);
    /// let mut ranges = highlight(&parse.syntax());
    /// let analysis = analyze(
    ///     &parse.syntax(),
    ///     &MemoryCatalog::new(),
    ///     &Library::new(),
    ///     &Options::default(),
    /// );
    /// analysis.model.refine_highlights(&mut ranges);
    /// let last = ranges.last().unwrap();
    /// assert_eq!(&text[last.range], "lo");
    /// assert_eq!(last.highlight, Highlight::Name(NameRole::Variable));
    /// ```
    pub fn refine_highlights(&self, ranges: &mut [HighlightRange]) {
        let mut names: Vec<(TextRange, NameRole)> = (self.names.iter())
            .map(|name| (name.range, role(&name.definition)))
            .collect();
        names.sort_by_key(|(range, _)| range.start());
        let mut values: Vec<TextRange> = self.enum_values.iter().map(|value| value.range).collect();
        values.sort_by_key(|range| range.start());
        for range in ranges {
            match range.highlight {
                Highlight::Name(_) if !range.definition => {
                    if let Some(role) = holder(&names, range.range, |(at, _)| *at) {
                        range.highlight = Highlight::Name(role.1);
                    }
                }
                Highlight::String if holder(&values, range.range, |at| *at).is_some() => {
                    range.highlight = Highlight::EnumValue;
                }
                _ => {}
            }
        }
    }
}

/// The last item of `items`, sorted by the start of their ranges, whose range holds `range`.
fn holder<T>(items: &[T], range: TextRange, at: impl Fn(&T) -> TextRange) -> Option<&T> {
    let before = items.partition_point(|item| at(item).start() <= range.start());
    items[..before]
        .iter()
        .rev()
        .find(|item| at(item).contains_range(range))
}

/// The role of a name that refers to `definition`.
fn role(definition: &Definition) -> NameRole {
    match definition {
        Definition::Column(_) => NameRole::Column,
        Definition::RangeVariable { .. } => NameRole::RangeVariable,
        Definition::Field { .. } => NameRole::Field,
        Definition::Table { .. } => NameRole::Table,
        Definition::Query { .. } => NameRole::Query,
        Definition::Variable { .. } => NameRole::Variable,
        Definition::Module { .. } => NameRole::Module,
        Definition::Function { role, .. } => NameRole::Function(*role),
    }
}
