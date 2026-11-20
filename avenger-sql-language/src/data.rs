//! The requests on the statements of data scripts: the options of files and of tables, with their
//! values, the URIs of files, and the tables, views, and schemas that a script defines.

use avenger_sql::{
    catalog::same_path,
    data::{
        file_option, option_format, option_places, unsupported_option, Compression, Definition,
        DefinitionKind, FileOption, Files, Format, OptionPlace, OptionType, FILE_OPTIONS,
    },
    types::googlesql_name,
};
use avenger_sql_syntax::{
    ast::{self, AstNode, CreateStmt, Scope},
    Role, SyntaxKind, SyntaxNode, SyntaxToken, TextRange,
};

use crate::{node_at, Document, Offset};

/// Whether the analysis of a text of `role` reads the statement `stmt` of files or of a data
/// script, with its options: each such statement of a data script, and of a chart script only
/// its own tables of files, `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`. A
/// module reads none.
pub(crate) fn reads_statement(role: Role, stmt: &SyntaxNode) -> bool {
    use SyntaxKind::*;
    match (role, stmt.kind()) {
        (
            Role::Data,
            CREATE_EXTERNAL_TABLE_STMT
            | LOAD_DATA_STMT
            | CREATE_VIEW_STMT
            | CREATE_TABLE_STMT
            | CREATE_SCHEMA_STMT,
        ) => true,
        (Role::Script, CREATE_EXTERNAL_TABLE_STMT) => {
            (ast::CreateExternalTableStmt::cast(stmt.clone()))
                .is_some_and(|create| create.scope_kind() == Some(Scope::Temp))
        }
        (Role::Script, LOAD_DATA_STMT) => {
            ast::LoadDataStmt::cast(stmt.clone()).is_some_and(|load| load.scope().is_some())
        }
        _ => false,
    }
}

/// A list of the options of a statement of a data script: an `OPTIONS (...)` or a `FROM FILES
/// (...)`.
pub(crate) struct OptionList {
    node: SyntaxNode,
    /// The places of the options that the list takes.
    places: &'static [OptionPlace],
}

impl OptionList {
    /// The list `node`, when it is the `OPTIONS` or the `FROM FILES` of a statement that the
    /// analysis of a text of `role` reads, as [`reads_statement`] tells. An external table's
    /// `OPTIONS` takes the options of its files and of its table, a `FROM FILES` those of the
    /// files, and the other `OPTIONS` those of a table.
    pub(crate) fn cast(node: &SyntaxNode, role: Role) -> Option<OptionList> {
        if !reads_statement(role, &node.parent()?) {
            return None;
        }
        Some(OptionList {
            node: node.clone(),
            places: option_places(node)?,
        })
    }

    /// The list that holds `token` directly, as its `(` or a `,` between its entries.
    pub(crate) fn of_separator(token: &SyntaxToken, role: Role) -> Option<OptionList> {
        matches!(token.kind(), SyntaxKind::L_PAREN | SyntaxKind::COMMA)
            .then(|| OptionList::cast(&token.parent()?, role))
            .flatten()
    }

    /// Whether the entry at `offset` has its `=`, so that a name there replaces its name alone.
    pub(crate) fn has_value_at(&self, offset: Offset) -> bool {
        (self.entries())
            .find(|entry| entry.syntax().text_range().contains_inclusive(offset))
            .is_some_and(|entry| {
                (entry.syntax().children_with_tokens()).any(|child| child.kind() == SyntaxKind::EQ)
            })
    }

    fn entries(&self) -> impl Iterator<Item = ast::OptionEntry> {
        self.node.children().filter_map(ast::OptionEntry::cast)
    }

    /// The format that the list's `format` sets, when it is one that a data script reads.
    fn format(&self) -> Option<Format> {
        option_format(&self.node).map(|(format, _)| format)
    }

    /// The compression that the list's `compression` sets, when it is a string of one.
    fn compression(&self) -> Option<Compression> {
        let entry = (self.entries()).find(|entry| {
            entry_name(entry).is_some_and(|name| name.eq_ignore_ascii_case("compression"))
        })?;
        match entry.expr()? {
            ast::Expr::Literal(literal) => Compression::parse(&literal.string_value()?),
            _ => None,
        }
    }

    /// The options that the list takes at `offset`: those of its places that apply to its
    /// format, or to any format when it sets none, without those that its other entries set.
    /// `hive_partition_uri_prefix` goes with `WITH PARTITION COLUMNS` alone.
    pub(crate) fn options_at(&self, offset: Offset) -> Vec<&'static FileOption> {
        let set: Vec<String> = (self.entries())
            .filter(|entry| !entry.syntax().text_range().contains_inclusive(offset))
            .filter_map(|entry| entry_name(&entry))
            .collect();
        let format = self.format();
        let partitions = (self.node.parent())
            .and_then(ast::CreateExternalTableStmt::cast)
            .is_some_and(|create| create.partition_columns().is_some());
        (FILE_OPTIONS.iter())
            .filter(|option| self.places.contains(&option.place))
            .filter(|option| format.is_none_or(|format| option.formats.contains(&format)))
            .filter(|option| partitions || option.name != "hive_partition_uri_prefix")
            .filter(|option| {
                !set.iter()
                    .any(|name| name.eq_ignore_ascii_case(option.name))
            })
            .collect()
    }
}

/// The name of the option of `entry`.
fn entry_name(entry: &ast::OptionEntry) -> Option<String> {
    entry.name()?.text()
}

/// The value of an option of a statement that the analysis of a text of `role` reads that
/// follows `token`: the option, or `None` for an option that a data script doesn't take, when
/// `token` is the `=` of its entry, or the `[` or a `,` of the array of its value, where an
/// option takes no other values.
pub(crate) fn value_after(token: &SyntaxToken, role: Role) -> Option<Option<&'static FileOption>> {
    let entry = match token.kind() {
        SyntaxKind::EQ => token.parent(),
        SyntaxKind::L_BRACK | SyntaxKind::COMMA => (token.parent())
            .filter(|node| node.kind() == SyntaxKind::ARRAY_EXPR)
            .and_then(|array| array.parent()),
        _ => None,
    }
    .and_then(ast::OptionEntry::cast)?;
    OptionList::cast(&entry.syntax().parent()?, role)?;
    let option = entry_name(&entry).and_then(|name| file_option(&name));
    Some(option.filter(|_| token.kind() == SyntaxKind::EQ))
}

/// What a string literal of the options of a data script's statement is.
pub(crate) enum FileString {
    /// The value of an option, such as `format`.
    Value(&'static FileOption),
    /// A URI of `uris`, of the files of the format and of the compression that the list sets.
    Uri(Option<Format>, Option<Compression>),
}

/// What the string literal `token` is, in the options of a statement that the analysis of a text
/// of `role` reads.
pub(crate) fn file_string(token: &SyntaxToken, role: Role) -> Option<FileString> {
    let literal = token
        .parent()
        .filter(|node| node.kind() == SyntaxKind::LITERAL)?;
    let parent = literal.parent()?;
    let (entry, uri) = match parent.kind() {
        SyntaxKind::OPTION_ENTRY => (parent, false),
        SyntaxKind::ARRAY_EXPR => (parent.parent()?, true),
        _ => return None,
    };
    let entry = ast::OptionEntry::cast(entry)?;
    let list = OptionList::cast(&entry.syntax().parent()?, role)?;
    let option = file_option(&entry_name(&entry)?)?;
    match (uri, option.ty) {
        (true, OptionType::StringArray) => Some(FileString::Uri(list.format(), list.compression())),
        (false, _) => Some(FileString::Value(option)),
        (true, _) => None,
    }
}

/// The values that `option` takes and that this build reads: the formats and the compressions
/// whose cargo features it has, and the other values of a list.
pub(crate) fn values(option: &FileOption) -> Vec<&'static str> {
    (option.values.iter())
        .copied()
        .filter(|value| match option.name {
            "format" => Format::parse(value).is_some_and(Format::available),
            "compression" => Compression::parse(value).is_some_and(Compression::available),
            _ => true,
        })
        .collect()
}

/// A path that a URI completes to: a file, or a directory, which ends with `/`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct UriPath {
    pub(crate) path: String,
    pub(crate) directory: bool,
    /// Whether a directory of the files reads the file by its name: a file with the extension
    /// of the format, and of the compression when the list sets one, or of no compression when
    /// it sets none, as `.csv.gz` for CSV with `GZIP`. Every file matches without a format, and
    /// every directory matches. A URI of one file reads it whatever its name, so the others
    /// complete too, after these.
    pub(crate) matches: bool,
}

/// The paths that extend `typed` by one segment: each file of `paths` in the directory of
/// `typed` whose name starts with the rest of `typed`, and each directory there that holds such
/// a file, in order.
pub(crate) fn paths(
    paths: &[String],
    typed: &str,
    format: Option<Format>,
    compression: Option<Compression>,
) -> Vec<UriPath> {
    let directory = typed.rfind('/').map_or(0, |slash| slash + 1);
    let matches = |path: &str| {
        format.is_none_or(|format| {
            let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
            let extension = match compression {
                Some(compression) => format!(".{}.{}", format.extension(), compression.extension()),
                None => format!(".{}", format.extension()),
            };
            name.ends_with(&extension)
        })
    };
    let mut found: Vec<UriPath> = (paths.iter())
        .filter(|path| path.starts_with(typed))
        .map(|path| match path[directory..].find('/') {
            Some(slash) => UriPath {
                path: path[..directory + slash + 1].to_string(),
                directory: true,
                matches: true,
            },
            None => UriPath {
                path: path.clone(),
                directory: false,
                matches: matches(path),
            },
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The hover of the option named `name`: its type, its meaning, the formats that it applies to,
/// and the values that this build reads, or, for an option of BigQuery that a data script
/// rejects, the reason.
pub(crate) fn option_hover(name: &str) -> Option<String> {
    if let Some(option) = file_option(name) {
        let mut markdown = format!(
            "```googlesql\n{}: {}\n```\n\n{}",
            option.name, option.ty, option.meaning
        );
        let applies = match option.place {
            OptionPlace::Table => "An option of a table, a view, or a schema".to_string(),
            OptionPlace::Files if option.formats.len() == Format::ALL.len() => {
                "An option of the files of every format".to_string()
            }
            OptionPlace::Files => {
                let formats: Vec<&str> = (option.formats.iter()).map(|f| f.name()).collect();
                format!("An option of {} files", formats.join(" and "))
            }
        };
        markdown.push_str(&format!("\n\n{applies}"));
        if option.required {
            markdown.push_str(", which every statement that reads files sets");
        }
        markdown.push('.');
        let values = values(option);
        if !values.is_empty() {
            let values: Vec<String> = (values.iter()).map(|v| format!("'{v}'")).collect();
            markdown.push_str(&format!("\n\nValues: {}", values.join(", ")));
        }
        return Some(markdown);
    }
    let unsupported = unsupported_option(name)?;
    let mut markdown = format!(
        "```googlesql\n{}\n```\n\nAn option of BigQuery that avenger-sql doesn't support: {}.",
        unsupported.name, unsupported.reason
    );
    if !unsupported.accepts.is_empty() {
        let values: Vec<String> = (unsupported.accepts.iter())
            .map(ToString::to_string)
            .collect();
        markdown.push_str(&format!(
            "\n\nIt takes {}, which is what avenger-sql does without it.",
            values.join(" or ")
        ));
    }
    Some(markdown)
}

impl Document {
    /// The hover of the name of a data script at `offset` that the tree tells: an option of a
    /// statement's `OPTIONS` or `FROM FILES`, or the path that a statement defines, as a table, a
    /// view, or a schema, and in a table's path, its schema.
    pub(crate) fn data_hover(
        &self,
        root: &SyntaxNode,
        offset: Offset,
    ) -> Option<(TextRange, String)> {
        let definitions = self.definitions();
        let token = root.token_at_offset(offset).max_by_key(|token| {
            matches!(token.kind(), SyntaxKind::WORD | SyntaxKind::QUOTED_IDENT)
        })?;
        let name = (token.parent()).filter(|node| node.kind() == SyntaxKind::NAME);
        if let Some(entry) = name
            .as_ref()
            .and_then(|name| name.parent())
            .and_then(ast::OptionEntry::cast)
        {
            OptionList::cast(&entry.syntax().parent()?, self.role)?;
            let name = entry.name()?;
            let text = name.text()?;
            let mut markdown = option_hover(&text)?;
            // The errors of the entry, such as an option of another format, or of another place.
            // An option that avenger-sql doesn't support tells its reason above.
            if file_option(&text).is_some() {
                let range = entry.syntax().text_range();
                for diagnostic in self.diagnostics() {
                    if range.contains_range(diagnostic.range) {
                        markdown.push_str(&format!("\n\n**Error:** {}", diagnostic.message));
                    }
                }
            }
            return Some((name.syntax().text_range(), markdown));
        }
        let index = (definitions.iter())
            .rposition(|definition| definition.name_range.contains_inclusive(offset))?;
        let definition = &definitions[index];
        if matches!(definition.kind, DefinitionKind::Schema) {
            let markdown = self.schema_hover(definitions, &definition.names);
            return Some((definition.name_range, markdown));
        }
        // The schema of a table's path, as `finance` of `finance.orders`.
        let schema = (definition.names.len() == 2)
            .then(|| node_at(root, definition.name_range, SyntaxKind::PATH))
            .flatten()
            .and_then(ast::Path::cast)
            .and_then(|path| path.name_refs().next())
            .map(|name| name.syntax().text_range())
            .filter(|range| range.contains_inclusive(offset));
        if let Some(range) = schema {
            return Some((
                range,
                self.schema_hover(definitions, &definition.names[..1]),
            ));
        }
        Some((definition.name_range, table_hover(definitions, index)))
    }

    /// The hover of the table at the path `names`, read at `offset`: the definition of the data
    /// script before the read, or the table of the catalog, with the place that defines it and
    /// its description.
    pub(crate) fn table_read_hover(&self, names: &[String], offset: Offset) -> Option<String> {
        if let Some(index) = self.own_table(names, offset) {
            return Some(table_hover(self.definitions(), index));
        }
        let table = self.catalog.table(names)?;
        let mut markdown = format!("```googlesql\nTABLE {}\n```", names.join("."));
        if let Some(definition) = self.catalog.definition(names) {
            if let Some(description) = &definition.description {
                markdown.push_str(&format!("\n\n{description}"));
            }
            markdown.push_str(&format!("\n\nDefined in `{}`", definition.file));
        }
        markdown.push_str(&format!("\n\n{}", columns(&table.schema())));
        Some(markdown)
    }

    /// The definition of this script that a read of the table `names` at `offset` reads: the
    /// last definition of the table that ends before the read, by its index in the definitions
    /// of a data script or of a chart script's own tables of files. `None` for a read of a
    /// table of the catalog, and in a module.
    pub(crate) fn own_table(&self, names: &[String], offset: Offset) -> Option<usize> {
        (self.definitions().iter()).rposition(|definition| {
            definition.range.end() <= offset
                && !matches!(definition.kind, DefinitionKind::Schema)
                && same_path(&definition.names, names)
        })
    }

    /// The hover of the schema `names`: a schema that this script creates, or that the catalog
    /// holds, with its description.
    fn schema_hover(&self, definitions: &[Definition], names: &[String]) -> String {
        let mut markdown = format!(
            "```googlesql\nSCHEMA {}\n```\n\nA schema of tables, which BigQuery calls a dataset.",
            names.join(".")
        );
        let defined = (definitions.iter()).rfind(|definition| {
            matches!(definition.kind, DefinitionKind::Schema) && same_path(&definition.names, names)
        });
        match defined {
            Some(definition) => {
                if let Some(description) = &definition.description {
                    markdown.push_str(&format!("\n\n{description}"));
                }
            }
            None => {
                if let Some(definition) = self.catalog.definition(names) {
                    if let Some(description) = &definition.description {
                        markdown.push_str(&format!("\n\n{description}"));
                    }
                    markdown.push_str(&format!("\n\nDefined in `{}`", definition.file));
                }
            }
        }
        markdown
    }
}

/// The hover of the table of the definition at `index` of `definitions`, or of the table that it
/// appends to: what defines it, its description, the format and the URIs of the files of its
/// statements, and its columns.
fn table_hover(definitions: &[Definition], index: usize) -> String {
    let root = definitions[index].appends().unwrap_or(index);
    let definition = &definitions[root];
    let temp = match definition.temp {
        true => "TEMP ",
        false => "",
    };
    let what = match definition.kind {
        DefinitionKind::ExternalTable { .. } => "EXTERNAL TABLE",
        DefinitionKind::View { .. } => "VIEW",
        _ => "TABLE",
    };
    let mut markdown = format!(
        "```googlesql\n{temp}{what} {}\n```",
        definition.names.join(".")
    );
    if let Some(description) = &definition.description {
        markdown.push_str(&format!("\n\n{description}"));
    }
    let statements = (definitions.iter().enumerate())
        .filter(|(at, other)| *at == root || other.appends() == Some(root))
        .map(|(_, other)| other);
    for statement in statements {
        if let Some(files) = files(statement) {
            let uris: Vec<String> = (files.uris.iter()).map(|uri| format!("`{uri}`")).collect();
            let verb = match statement.kind {
                DefinitionKind::LoadData { .. } => "Loads",
                _ => "Reads",
            };
            markdown.push_str(&format!(
                "\n\n{verb} {} files: {}",
                files.format.name(),
                uris.join(", ")
            ));
        }
    }
    if let Some(schema) = &definition.schema {
        markdown.push_str(&format!("\n\n{}", columns(schema)));
    }
    markdown
}

/// The files that `definition` reads.
fn files(definition: &Definition) -> Option<&Files> {
    match &definition.kind {
        DefinitionKind::ExternalTable { files } | DefinitionKind::LoadData { files, .. } => {
            files.as_ref()
        }
        _ => None,
    }
}

/// A Markdown list of the columns of `schema` with their types.
fn columns(schema: &datafusion::arrow::datatypes::Schema) -> String {
    (schema.fields().iter())
        .map(|field| {
            format!(
                "- `{}`: {}",
                field.name(),
                googlesql_name(field.data_type())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
